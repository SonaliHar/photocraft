//! Sealed flat-root native reader and transactional Normal/Union stack.
use super::*;
use photocraft_doc::BackedLayer;
struct Band {
    pixels: Vec<[f64; 4]>,
    lease: QuotaLease,
}
fn region(rect: Rect, ctx: &Context<'_>) -> Result<usize, E> {
    let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(length)?;
    if rect.is_empty() || !ctx.plan.bounds.contains_rect(&rect) || rect.height() > ctx.limits.max_rows || count > ctx.limits.max_pixels {
        return Err(length());
    }
    usize::try_from(count).map_err(|_| length())
}
fn allocate(ctx: &Context<'_>, rect: Rect, c: &ReadControl<'_>) -> Result<Band, E> {
    let count = region(rect, ctx)?;
    let capacity = count.checked_mul(2).ok_or_else(length)?;
    let header = std::mem::size_of::<Band>().checked_add(std::mem::size_of::<OwnedNormalStackBand>()).and_then(|v| v.checked_add(128)).ok_or_else(length)?;
    let bytes = capacity.checked_mul(std::mem::size_of::<[f64; 4]>()).and_then(|v| v.checked_add(header)).ok_or_else(length)?;
    c.check(1).map_err(E::Backing)?;
    let lease = ctx.owner.quota().reserve(QuotaKind::Resident, u64::try_from(bytes).map_err(|_| length())?).map_err(E::Quota)?;
    let mut result = Band { pixels: Vec::new(), lease };
    result.pixels.try_reserve_exact(count).map_err(|_| E::Allocation)?;
    if result.pixels.capacity() > capacity {
        return Err(E::Allocation);
    }
    let actual = result.pixels.capacity().checked_mul(std::mem::size_of::<[f64; 4]>()).and_then(|v| v.checked_add(header)).ok_or_else(length)?;
    result.lease.reconcile_capacity(u64::try_from(actual).map_err(|_| length())?).map_err(E::Quota)?;
    Ok(result)
}
fn layer<'a>(ctx: &'a Context<'_>, entry: &Entry<QuotaLease>) -> Result<&'a BackedLayer, E> {
    let actual = ctx.owner.document().layers.get(entry.index).ok_or_else(length)?;
    if actual.id != entry.id {
        return Err(scope());
    }
    let LayerContent::BackedRaster(backed) = &actual.content else { return Err(scope()) };
    if backed.descriptor() != entry.descriptor || backed.format() != ctx.plan.format || !backed.quota().shares_ledger(ctx.owner.quota()) {
        return Err(scope());
    }
    Ok(backed)
}
fn tile_rect(coord: photocraft_geom::TileCoord) -> Result<Rect, E> {
    let side = photocraft_geom::TILE_SIZE;
    let x = coord.tx.checked_mul(side).ok_or_else(length)?;
    let y = coord.ty.checked_mul(side).ok_or_else(length)?;
    Ok(Rect::new(x, y, x.checked_add(side).ok_or_else(length)?, y.checked_add(side).ok_or_else(length)?))
}
fn walk(ctx: &Context<'_>, entry: &Entry<QuotaLease>, rect: Rect, mut output: Option<&mut Vec<[f64; 4]>>, c: &ReadControl<'_>) -> Result<(), E> {
    let native = layer(ctx, entry)?;
    let view = native.checked_view();
    let format = native.format();
    let bpp = format.bytes_per_pixel();
    let mut seen = 0usize;
    for coordinate in rect.tiles() {
        c.check(0).map_err(E::Backing)?;
        let overlap = tile_rect(coordinate)?.intersect(&rect);
        let pin = view.pin_tile(coordinate, c).map_err(E::View)?;
        for y in overlap.y0..overlap.y1 {
            for x in overlap.x0..overlap.x1 {
                c.check(8).map_err(E::Backing)?;
                let mut raw = [0u8; 32];
                let bytes = raw.get_mut(..bpp).ok_or_else(length)?;
                pin.read_pixel_native_into(x, y, bytes, c).map_err(E::View)?;
                unit(bytes, format)?;
                let value = if let Some(point) = &entry.point {
                    point.evaluate(ctx.owner.quota(), bytes, (x, y), c)?
                } else {
                    [
                        try_read_sample_f64(bytes, format.sample, 0).map_err(|_| length())?,
                        try_read_sample_f64(bytes, format.sample, 1).map_err(|_| length())?,
                        try_read_sample_f64(bytes, format.sample, 2).map_err(|_| length())?,
                        try_read_sample_f64(bytes, format.sample, 3).map_err(|_| length())?,
                    ]
                };
                if value.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&value[3]) {
                    return Err(scope());
                }
                if let Some(pixels) = &mut output {
                    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| length())?;
                    let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| length())?;
                    let at = row.checked_mul(rect.width() as usize).and_then(|n| n.checked_add(col)).ok_or_else(length)?;
                    *pixels.get_mut(at).ok_or_else(length)? = value;
                }
                seen = seen.checked_add(1).ok_or_else(length)?;
            }
        }
    }
    if seen != region(rect, ctx)? {
        return Err(length());
    }
    c.check(0).map_err(E::Backing)
}
fn source(ctx: &Context<'_>, entry: &Entry<QuotaLease>, rect: Rect, c: &ReadControl<'_>) -> Result<Band, E> {
    // Complete preflight before allocating published leaf pixels. The immutable
    // actual provider is read again for the output pass on the same control.
    walk(ctx, entry, rect, None, c)?;
    let count = region(rect, ctx)?;
    let mut band = allocate(ctx, rect, c)?;
    c.check((count as u64).checked_mul(4).ok_or_else(length)?).map_err(E::Backing)?;
    band.pixels.resize(count, [0.; 4]);
    walk(ctx, entry, rect, Some(&mut band.pixels), c)?;
    Ok(band)
}
pub(super) fn render(ctx: &Context<'_>, rect: Rect, c: &ReadControl<'_>) -> Result<OwnedNormalStackBand, E> {
    c.check(1).map_err(E::Backing)?;
    let count = region(rect, ctx)?;
    let mut prior = allocate(ctx, rect, c)?;
    c.check((count as u64).checked_mul(4).ok_or_else(length)?).map_err(E::Backing)?;
    prior.pixels.resize(count, [0.; 4]);
    // Document roots are bottom-first. Never copy the first visible root: Normal
    // over explicit transparent zero applies its real opacity/generated alpha.
    for entry in &ctx.plan.entries {
        let source = source(ctx, entry, rect, c)?; // hidden/zero roots still fullproof
        let opacity = if entry.visible { entry.opacity } else { 0. };
        if opacity == 0. {
            continue;
        }
        let mut next = allocate(ctx, rect, c)?;
        for (index, pixel) in source.pixels.iter().enumerate() {
            c.check(2048).map_err(E::Backing)?;
            let backdrop = *prior.pixels.get(index).ok_or_else(length)?;
            let value = ctx.plan.working.normal(backdrop, *pixel, opacity, GimpOperator::Union).map_err(E::Color)?;
            if value.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&value[3]) {
                return Err(scope());
            }
            next.pixels.push(value);
        }
        if next.pixels.len() != count {
            return Err(length());
        }
        c.check(0).map_err(E::Backing)?;
        prior = next;
        // source drops after the next private accumulator is complete. All
        // three bands have actual live same-q fees through this transition.
    }
    c.check(0).map_err(E::Backing)?;
    let Band { pixels, lease } = prior;
    Ok(OwnedNormalStackBand::from_accounted_pixels(rect, pixels, lease))
}
fn chunks(ctx: &Context<'_>, mut visit: impl FnMut(Rect) -> Result<(), E>) -> Result<(), E> {
    for coordinate in ctx.plan.bounds.tiles() {
        let tile = tile_rect(coordinate)?.intersect(&ctx.plan.bounds);
        let mut x = tile.x0;
        while x < tile.x1 {
            let width = u64::from(tile.width()).min(ctx.limits.max_pixels).min(u64::try_from(i64::from(tile.x1) - i64::from(x)).map_err(|_| length())?);
            let width = u32::try_from(width).map_err(|_| length())?;
            if width == 0 {
                return Err(length());
            }
            let rows = u32::try_from((ctx.limits.max_pixels / u64::from(width)).min(u64::from(ctx.limits.max_rows))).map_err(|_| length())?;
            if rows == 0 {
                return Err(length());
            }
            let mut y = tile.y0;
            while y < tile.y1 {
                let h = rows.min(u32::try_from(i64::from(tile.y1) - i64::from(y)).map_err(|_| length())?);
                let x1 = x.checked_add(i32::try_from(width).map_err(|_| length())?).ok_or_else(length)?;
                let y1 = y.checked_add(i32::try_from(h).map_err(|_| length())?).ok_or_else(length)?;
                visit(Rect::new(x, y, x1, y1))?;
                y = y1;
            }
            x = x.checked_add(i32::try_from(width).map_err(|_| length())?).ok_or_else(length)?;
        }
    }
    Ok(())
}
pub(super) fn validate(ctx: &Context<'_>, c: &ReadControl<'_>) -> Result<(), E> {
    for entry in &ctx.plan.entries {
        chunks(ctx, |rect| walk(ctx, entry, rect, None, c))?;
    }
    // Composition's own converted/intermediate finite domain is independently
    // checked, not inferred from unit native input or leaf output validation.
    chunks(ctx, |rect| {
        drop(render(ctx, rect, c)?);
        Ok(())
    })?;
    c.check(0).map_err(E::Backing)
}
