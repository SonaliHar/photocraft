//! Iterative isolated Normal execution; private accumulators publish atomically.
use super::*;
use photocraft_doc::BackedLayer;
struct Band {
    pixels: Vec<[f64; 4]>,
    lease: QuotaLease,
}
struct Frame {
    ordinal: Option<usize>,
    end: usize,
    band: Option<Band>,
}
fn region(ctx: &Context<'_>, r: Rect) -> Result<usize, E> {
    // Establish bounded canvas endpoints before width/height arithmetic.
    if r.is_empty() || !ctx.plan.bounds.contains_rect(&r) {
        return Err(length());
    }
    let pixels = u64::from(r.width()).checked_mul(u64::from(r.height())).ok_or_else(length)?;
    if r.height() > ctx.limits.max_rows || pixels > ctx.limits.max_pixels {
        return Err(length());
    }
    usize::try_from(pixels).map_err(|_| length())
}
fn allocate(ctx: &Context<'_>, r: Rect, c: &ReadControl<'_>) -> Result<Band, E> {
    let count = region(ctx, r)?;
    let cap = count.checked_mul(2).ok_or_else(length)?;
    let header = std::mem::size_of::<Band>().checked_add(std::mem::size_of::<OwnedNormalStackBand>()).and_then(|n| n.checked_add(128)).ok_or_else(length)?;
    let bytes = cap.checked_mul(std::mem::size_of::<[f64; 4]>()).and_then(|n| n.checked_add(header)).ok_or_else(length)?;
    c.check(1).map_err(E::Backing)?;
    let lease = ctx.owner.quota().reserve(QuotaKind::Resident, bytes as u64).map_err(E::Quota)?;
    let mut band = Band { pixels: Vec::new(), lease };
    band.pixels.try_reserve_exact(count).map_err(|_| E::Allocation)?;
    if band.pixels.capacity() > cap {
        return Err(E::Allocation);
    }
    let actual = band.pixels.capacity().checked_mul(std::mem::size_of::<[f64; 4]>()).and_then(|n| n.checked_add(header)).ok_or_else(length)?;
    band.lease.reconcile_capacity(actual as u64).map_err(E::Quota)?;
    c.check((count as u64).checked_mul(4).ok_or_else(length)?).map_err(E::Backing)?;
    band.pixels.resize(count, [0.; 4]);
    Ok(band)
}
fn native<'a>(ctx: &'a Context<'_>, ordinal: usize) -> Result<&'a BackedLayer, E> {
    let row = ctx.index.row(ordinal).ok_or_else(length)?;
    let node = ctx.plan.nodes.get(ordinal).ok_or_else(length)?;
    if row.id != node.id {
        return Err(scope());
    }
    let LayerContent::BackedRaster(v) = &row.layer.content else { return Err(scope()) };
    let leaf = node.leaf.as_ref().ok_or_else(scope)?;
    if v.descriptor() != leaf.descriptor || v.format() != ctx.plan.format || !v.quota().shares_ledger(ctx.owner.quota()) {
        return Err(scope());
    }
    Ok(v)
}
fn tile_rect(t: photocraft_geom::TileCoord) -> Result<Rect, E> {
    let side = photocraft_geom::TILE_SIZE;
    let x = t.tx.checked_mul(side).ok_or_else(length)?;
    let y = t.ty.checked_mul(side).ok_or_else(length)?;
    Ok(Rect::new(x, y, x.checked_add(side).ok_or_else(length)?, y.checked_add(side).ok_or_else(length)?))
}
fn walk(ctx: &Context<'_>, ordinal: usize, r: Rect, mut output: Option<&mut [[f64; 4]]>, c: &ReadControl<'_>) -> Result<(), E> {
    let native = native(ctx, ordinal)?;
    let leaf = ctx.plan.nodes.get(ordinal).and_then(|n| n.leaf.as_ref()).ok_or_else(scope)?;
    let view = native.checked_view();
    let format = ctx.plan.format;
    let bpp = format.bytes_per_pixel();
    let mut seen = 0usize;
    // Real runtime pin/raw/fixed-loop storage independent of preparation leases.
    c.check(1).map_err(E::Backing)?;
    let _pin_fee = ctx
        .owner
        .quota()
        .reserve(QuotaKind::Resident, (std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() + 64 + 128) as u64)
        .map_err(E::Quota)?;
    for coordinate in r.tiles() {
        c.check(0).map_err(E::Backing)?;
        let overlap = tile_rect(coordinate)?.intersect(&r);
        let pin = view.pin_tile(coordinate, c).map_err(E::View)?;
        for y in overlap.y0..overlap.y1 {
            for x in overlap.x0..overlap.x1 {
                c.check(64).map_err(E::Backing)?;
                let mut raw = [0u8; 32];
                let bytes = raw.get_mut(..bpp).ok_or_else(length)?;
                pin.read_pixel_native_into(x, y, bytes, c).map_err(E::View)?;
                unit(bytes, format)?;
                let value = if let Some(point) = &leaf.point {
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
                    let row = usize::try_from(i64::from(y) - i64::from(r.y0)).map_err(|_| length())?;
                    let col = usize::try_from(i64::from(x) - i64::from(r.x0)).map_err(|_| length())?;
                    let at = row.checked_mul(r.width() as usize).and_then(|n| n.checked_add(col)).ok_or_else(length)?;
                    *pixels.get_mut(at).ok_or_else(length)? = value;
                }
                seen = seen.checked_add(1).ok_or_else(length)?;
            }
        }
    }
    if seen != region(ctx, r)? {
        return Err(length());
    }
    c.check(0).map_err(E::Backing)
}
fn source(ctx: &Context<'_>, ordinal: usize, r: Rect, c: &ReadControl<'_>) -> Result<Band, E> {
    walk(ctx, ordinal, r, None, c)?;
    let mut band = allocate(ctx, r, c)?;
    walk(ctx, ordinal, r, Some(band.pixels.as_mut_slice()), c)?;
    Ok(band)
}
fn fold(ctx: &Context<'_>, target: &mut Band, source: &Band, node: &Node<QuotaLease>, c: &ReadControl<'_>) -> Result<(), E> {
    if target.pixels.len() != source.pixels.len() {
        return Err(length());
    }
    c.check(1).map_err(E::Backing)?;
    // Complete source subtree and enabled leaf proof has already run.
    if !node.visible || node.opacity == 0. {
        return Ok(());
    }
    // Independent fixed Normal/transfer locals; no Band header or retained
    // preparation reservation is reused as runtime scratch credit.
    let _normal_scratch = ctx.owner.quota().reserve(QuotaKind::Resident, 512).map_err(E::Quota)?;
    for (dst, src) in target.pixels.iter_mut().zip(&source.pixels) {
        c.check(2048).map_err(E::Backing)?;
        let next = ctx.plan.working.normal(*dst, *src, node.opacity, GimpOperator::Union).map_err(E::Color)?;
        if next.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&next[3]) {
            return Err(scope());
        }
        *dst = next;
    }
    c.check(0).map_err(E::Backing)
}
fn close(ctx: &Context<'_>, frames: &mut [Option<Frame>; 33], depth: &mut usize, c: &ReadControl<'_>) -> Result<(), E> {
    c.check(32).map_err(E::Backing)?;
    if *depth == 0 {
        return Err(length());
    }
    let top = frames.get_mut(*depth).ok_or_else(length)?.take().ok_or_else(scope)?;
    *depth = depth.checked_sub(1).ok_or_else(length)?;
    let node = ctx.plan.nodes.get(top.ordinal.ok_or_else(scope)?).ok_or_else(length)?;
    let parent = frames.get_mut(*depth).and_then(Option::as_mut).ok_or_else(scope)?;
    if node.parent != parent.ordinal {
        return Err(scope());
    }
    if let Some(source) = top.band {
        let target = parent.band.as_mut().ok_or_else(scope)?;
        fold(ctx, target, &source, node, c)?;
    }
    Ok(())
}
fn execute(ctx: &Context<'_>, r: Rect, c: &ReadControl<'_>) -> Result<Band, E> {
    // ALL group chains, including empty ones, fit33 frames: virtual root+32.
    // This fee precedes the actual fixed frame array, which outlives every band.
    c.check(1).map_err(E::Backing)?;
    let _frames_fee = ctx.owner.quota().reserve(QuotaKind::Resident, (std::mem::size_of::<[Option<Frame>; 33]>() + 128) as u64).map_err(E::Quota)?;
    let mut frames: [Option<Frame>; 33] = std::array::from_fn(|_| None);
    let mut depth = 0usize;
    frames[0] = Some(Frame { ordinal: None, end: ctx.plan.nodes.len(), band: Some(allocate(ctx, r, c)?) });
    for pos in 0..=ctx.plan.nodes.len() {
        c.check(32).map_err(E::Backing)?;
        while depth != 0 && frames.get(depth).and_then(Option::as_ref).is_some_and(|f| f.end == pos) {
            close(ctx, &mut frames, &mut depth, c)?;
        }
        if pos == ctx.plan.nodes.len() {
            break;
        }
        let node = ctx.plan.nodes.get(pos).ok_or_else(length)?;
        let parent = frames.get(depth).and_then(Option::as_ref).ok_or_else(scope)?;
        if node.parent != parent.ordinal || node.subtree_end <= pos || node.subtree_end > ctx.plan.nodes.len() {
            return Err(scope());
        }
        if node.leaf.is_none() {
            depth = depth.checked_add(1).ok_or_else(length)?;
            if depth > 32 {
                return Err(scope());
            }
            let band = if node.bounds.is_empty() { None } else { Some(allocate(ctx, r, c)?) };
            *frames.get_mut(depth).ok_or_else(length)? = Some(Frame { ordinal: Some(pos), end: node.subtree_end, band });
        } else {
            let source = source(ctx, pos, r, c)?;
            let parent = frames.get_mut(depth).and_then(Option::as_mut).ok_or_else(scope)?;
            let target = parent.band.as_mut().ok_or_else(scope)?;
            fold(ctx, target, &source, node, c)?;
        }
    }
    if depth != 0 {
        return Err(scope());
    }
    let root = frames[0].take().ok_or_else(scope)?;
    c.check(0).map_err(E::Backing)?;
    root.band.ok_or_else(scope)
}
fn quantum(ctx: &Context<'_>) -> Result<u64, E> {
    let live = ctx.plan.max_depth.checked_add(2).ok_or_else(length)?;
    let live = u64::try_from(live).map_err(|_| length())?;
    Ok((ctx.limits.max_pixels / live).max(1))
}
fn chunks(ctx: &Context<'_>, r: Rect, c: &ReadControl<'_>, mut visit: impl FnMut(Rect) -> Result<(), E>) -> Result<(), E> {
    if r.is_empty() || !ctx.plan.bounds.contains_rect(&r) {
        return Err(length());
    }
    let quantum = quantum(ctx)?;
    let mut x = r.x0;
    while x < r.x1 {
        c.check(1).map_err(E::Backing)?;
        let width = u64::try_from(i64::from(r.x1) - i64::from(x)).map_err(|_| length())?.min(quantum);
        let width = u32::try_from(width).map_err(|_| length())?;
        if width == 0 {
            return Err(length());
        }
        let rows = (quantum / u64::from(width)).min(u64::from(ctx.limits.max_rows));
        if rows == 0 {
            return Err(length());
        }
        let mut y = r.y0;
        while y < r.y1 {
            c.check(1).map_err(E::Backing)?;
            let h = u64::try_from(i64::from(r.y1) - i64::from(y)).map_err(|_| length())?.min(rows);
            let x1 = x.checked_add(i32::try_from(width).map_err(|_| length())?).ok_or_else(length)?;
            let y1 = y.checked_add(i32::try_from(h).map_err(|_| length())?).ok_or_else(length)?;
            visit(Rect::new(x, y, x1, y1))?;
            y = y1;
        }
        x = x.checked_add(i32::try_from(width).map_err(|_| length())?).ok_or_else(length)?;
    }
    Ok(())
}
pub(super) fn render(ctx: &Context<'_>, r: Rect, c: &ReadControl<'_>) -> Result<OwnedNormalStackBand, E> {
    c.check(1).map_err(E::Backing)?;
    region(ctx, r)?;
    let mut output = allocate(ctx, r, c)?;
    chunks(ctx, r, c, |chunk| {
        let band = execute(ctx, chunk, c)?;
        for y in chunk.y0..chunk.y1 {
            for x in chunk.x0..chunk.x1 {
                c.check(32).map_err(E::Backing)?;
                let from = usize::try_from(i64::from(y) - i64::from(chunk.y0))
                    .map_err(|_| length())?
                    .checked_mul(chunk.width() as usize)
                    .and_then(|n| n.checked_add(usize::try_from(i64::from(x) - i64::from(chunk.x0)).ok()?))
                    .ok_or_else(length)?;
                let to = usize::try_from(i64::from(y) - i64::from(r.y0))
                    .map_err(|_| length())?
                    .checked_mul(r.width() as usize)
                    .and_then(|n| n.checked_add(usize::try_from(i64::from(x) - i64::from(r.x0)).ok()?))
                    .ok_or_else(length)?;
                *output.pixels.get_mut(to).ok_or_else(length)? = *band.pixels.get(from).ok_or_else(length)?;
            }
        }
        Ok(())
    })?;
    c.check(0).map_err(E::Backing)?;
    Ok(OwnedNormalStackBand::from_accounted_pixels(r, output.pixels, output.lease))
}
pub(super) fn validate(ctx: &Context<'_>, c: &ReadControl<'_>) -> Result<(), E> {
    for (ordinal, node) in ctx.plan.nodes.iter().enumerate() {
        if node.leaf.is_some() {
            chunks(ctx, ctx.plan.bounds, c, |r| walk(ctx, ordinal, r, None, c))?;
        }
    }
    chunks(ctx, ctx.plan.bounds, c, |r| {
        drop(execute(ctx, r, c)?);
        Ok(())
    })?;
    c.check(0).map_err(E::Backing)
}
