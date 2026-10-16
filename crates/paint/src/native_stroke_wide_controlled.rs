//! Additive native binary64 Round producer; no document or edit authority.
use super::*;
/// Exact native working color. Profile conversion and actual document authority
/// remain with the caller; this fixed bounded value performs no CMS work.
#[derive(Clone, Copy, Debug)]
pub struct NativeWideColor {
    format: PixelFormat,
    values: [f64; 5],
}
impl NativeWideColor {
    pub fn new(format: PixelFormat, values: &[f64]) -> NResult<Self> {
        if !matches!(format.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
            || !matches!(format.sample, SampleType::U32 | SampleType::F64)
            || format.channels() > 5
            || values.len() != format.channels()
            || values.iter().any(|v| !v.is_finite())
            || alpha_index(&format).is_some_and(|a| values.get(a).is_none_or(|v| !(0.0..=1.0).contains(v)))
        {
            return invalid("native wide color shape/domain");
        }
        let mut stored = [0.; 5];
        stored.get_mut(..values.len()).ok_or(BackingError::Length)?.copy_from_slice(values);
        Ok(Self { format, values: stored })
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
}
// Covered payloads precede all retained preparation receipts.
pub struct PreparedNativeWideStroke<'s> {
    source: CheckedRasterView<'s>,
    ctx: BrushContext,
    cov: CoverageMap,
    color: NativeWideColor,
    bounds: Rect,
    lock_alpha: bool,
    limits: NativeStrokeLimits,
    quota: BackingQuota,
    prepared_bytes: u64,
    selection: Option<selection::CheckedNativeSelection<'s>>,
    _coverage: QuotaLease,
    _generation: QuotaLease,
}
#[allow(clippy::too_many_arguments)]
pub fn prepare_clipped_native_wide_stroke<'s>(
    source: CheckedRasterView<'s>,
    clip: Rect,
    brush: &BrushSettings,
    points: &[StrokePoint],
    color: NativeWideColor,
    lock_transparency: bool,
    zoom: f32,
    limits: NativeStrokeLimits,
    q: &BackingQuota,
    c: &ReadControl<'_>,
) -> NResult<PreparedNativeWideStroke<'s>> {
    c.check(1)?;
    accepted(brush)?;
    let fmt = source.format();
    if color.format != fmt {
        return invalid("native wide color format mismatch");
    }
    if !matches!(fmt.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
        || !matches!(fmt.sample, SampleType::U32 | SampleType::F64)
        || fmt.channels() > 8
        || !zoom.is_finite()
        || zoom <= 0.0
    {
        return invalid("native brush format/zoom");
    }
    check_rect(clip)?;
    if clip.intersect(&source.logical_bounds()) != clip {
        return invalid("clip outside native operative bounds");
    }
    let (steps, primary, gen_bytes) = generation(brush, points, c)?;
    let gen_bytes = sum(gen_bytes, std::mem::size_of::<PreparedNativeWideStroke<'_>>() as u64 + 1024)?;
    let generation_fee = reserve(q, gen_bytes, limits.resident, c)?;
    let failure = Cell::new(None);
    let forward = |n| {
        c.check(n).map_err(|e| {
            failure.set(Some(e));
            StrokeError::Limit
        })
    };
    let mut budget =
        Budget { control: c.interrupt, limits: StrokeLimits { resident_bytes: limits.resident, work_units: limits.work }, work: 0, forward: Some(&forward) };
    c.check(product(steps as u64, 256)?)?;
    let mut numeric = numeric_brush(brush);
    numeric.color = [0., 0., 0., 1.];
    numeric.background = [0., 0., 0., 1.];
    numeric.color_cmyk = None;
    let result = generate(&numeric, points, zoom, steps, primary, 0, &mut budget);
    let (primary, dual) = result.map_err(|e| failure.take().map_or(NativeStrokeError::Stroke(e), NativeStrokeError::Backing))?;
    drop(dual);
    let ctx = BrushContext { brush: numeric, tip: None, dual_tip: None, texture: None };
    let mut tiles = 0u64;
    let mut maximum = 0usize;
    let mut total_pixels = 0u64;
    let mut bounds = Rect::EMPTY;
    for dab in &primary {
        c.check(1)?;
        finite(&[dab.radius, dab.alpha, dab.opacity, dab.angle, dab.roundness, dab.proj_scale, dab.proj_angle, dab.depth])?;
        finite(&dab.color)?;
        let rect = ctx.dab_rect(dab, false).intersect(&clip);
        check_rect(rect)?;
        maximum = maximum.max(area(rect)?);
        total_pixels = sum(total_pixels, area(rect)? as u64)?;
        tiles = sum(tiles, cov_tiles(rect)?)?;
        bounds = bounds.union(&rect)
    }
    let bytes = product(product(bounds.width() as u64, bounds.height() as u64)?, fmt.bytes_per_pixel() as u64)?;
    if bytes > limits.native_bytes as u64 {
        return invalid("native stroke region exceeds bounded contiguous patch");
    }
    let coverage_bytes = sum(sum(product(tiles, (COV_TILE * COV_TILE * 4) as u64 + 2048)?, product(maximum as u64, 16)?)?, 1 << 20)?;
    if sum(gen_bytes, coverage_bytes)? > limits.resident {
        return Err(BackingError::Quota.into());
    }
    let coverage_fee = reserve(q, coverage_bytes, limits.resident, c)?;
    c.check(sum(product(tiles, (COV_TILE * COV_TILE * 4) as u64)?, product(tiles, 2048)?)?)?;
    let mut cov = CoverageMap::new(0);
    cov.tiles.try_reserve(usize::try_from(tiles).map_err(|_| BackingError::Length)?).map_err(|_| BackingError::Allocation)?;
    if (cov.tiles.capacity() as u64).checked_mul(128).is_none_or(|n| n > tiles.saturating_mul(2048).saturating_add(4096)) {
        return Err(BackingError::Allocation.into());
    }
    c.check(product(total_pixels, 256)?)?;
    let result = clipped_raster(&ctx, &primary, clip, &mut cov, maximum, &mut budget);
    result.map_err(|e| failure.take().map_or(NativeStrokeError::Stroke(e), NativeStrokeError::Backing))?;
    drop(primary);
    c.check(1)?;
    Ok(PreparedNativeWideStroke {
        source,
        ctx,
        cov,
        color,
        bounds,
        lock_alpha: lock_transparency,
        limits,
        quota: q.clone(),
        prepared_bytes: sum(gen_bytes, coverage_bytes)?,
        selection: None,
        _coverage: coverage_fee,
        _generation: generation_fee,
    })
}
impl PreparedNativeWideStroke<'_> {
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    pub fn format(&self) -> PixelFormat {
        self.source.format()
    }
    pub fn render(self, c: &ReadControl<'_>) -> NResult<NativeStrokePatch> {
        let q = &self.quota;
        c.check(1)?;
        let fmt = self.source.format();
        let n = fmt.channels();
        let bpp = fmt.bytes_per_pixel();
        let bytes =
            usize::try_from(product(product(self.bounds.width() as u64, self.bounds.height() as u64)?, bpp as u64)?).map_err(|_| BackingError::Length)?;
        if bytes > self.limits.native_bytes {
            return invalid("native wide patch cap");
        }
        let fixed = (std::mem::size_of::<NativeStrokePatch>() as u64)
            .checked_add(std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() as u64)
            .and_then(|n| n.checked_add(1024))
            .ok_or(BackingError::Length)?;
        let runtime_bytes = sum(sum(product(bytes as u64, 2)?, fixed)?, self.selection.as_ref().map_or(0, selection::CheckedNativeSelection::runtime_bytes))?;
        if sum(self.prepared_bytes, runtime_bytes)? > self.limits.resident {
            return Err(BackingError::Quota.into());
        }
        let fee = reserve(q, runtime_bytes, self.limits.resident, c)?;
        c.check(product(bytes as u64, 2)?)?;
        let mut input = vector(bytes, c)?;
        input.resize(bytes, 0);
        let mut output = vector(bytes, c)?;
        output.resize(bytes, 0);
        for coord in self.bounds.tiles() {
            let pin = self.source.pin_tile(coord, c)?;
            let part = coord.rect().intersect(&self.bounds);
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let at = ((y - self.bounds.y0) as usize * self.bounds.width() as usize + (x - self.bounds.x0) as usize) * bpp;
                    pin.read_pixel_native_into(x, y, input.get_mut(at..at + bpp).ok_or(BackingError::Length)?, c)?;
                }
            }
        }
        for (a, b) in output.chunks_mut(4096).zip(input.chunks(4096)) {
            c.check(b.len() as u64)?;
            a.copy_from_slice(b)
        }
        let brush = &self.ctx.brush;
        let paint = self.color.values.get(..n).ok_or(BackingError::Length)?;
        for (&(tx, ty), tile) in &self.cov.tiles {
            let rect = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&self.bounds);
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    c.check(product(256, (n as u64).checked_add(1).ok_or(BackingError::Length)?)?)?;
                    let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                    let coverage = *tile.cov.get(ti).ok_or(BackingError::Length)?;
                    if coverage <= 0.0 {
                        continue;
                    }
                    // Keep the existing geometry/opacity association in F32, then promote once.
                    let k = (self.ctx.stroke_mask_wet(coverage, None, None, x, y) * brush.opacity.clamp(0.0, 1.0)).min(1.0);
                    if !k.is_finite() {
                        return invalid("nonfinite native wide coverage");
                    }
                    if k <= 0.0 {
                        continue;
                    }
                    let selected_weight = if let Some(selection) = &self.selection {
                        c.check(128)?;
                        let weight = f64::from(k) * selection.wide(x, y, c)?;
                        if !weight.is_finite() { return invalid("nonfinite selected native wide coverage"); }
                        if weight <= 0.0 { continue; }
                        Some(weight)
                    } else { None };
                    let at = ((y - self.bounds.y0) as usize * self.bounds.width() as usize + (x - self.bounds.x0) as usize) * bpp;
                    let out = output.get_mut(at..at + bpp).ok_or(BackingError::Length)?;
                    if brush.erase {
                        // Existing background/no-alpha native erasing contributes no operation.
                        if !fmt.alpha {
                            continue;
                        }
                        if let Some(weight) = selected_weight {
                            crate::wide_erase::try_erase_native_pixel(out, fmt, weight, self.lock_alpha, crate::wide_erase::EraseColorPolicy::Preserve)
                        } else {
                            crate::wide_erase::try_erase_native_pixel(out, fmt, f64::from(k), self.lock_alpha, crate::wide_erase::EraseColorPolicy::Preserve)
                        }.map_err(|_| NativeStrokeError::Invalid("native wide erase arithmetic"))?;
                    } else {
                        if let Some(weight) = selected_weight {
                            crate::wide::try_normal_native_pixel(out, fmt, paint, weight, self.lock_alpha)
                        } else {
                            crate::wide::try_normal_native_pixel(out, fmt, paint, f64::from(k), self.lock_alpha)
                        }.map_err(|_| NativeStrokeError::Invalid("native wide normal arithmetic"))?;
                    }
                }
            }
        }
        let mut changed = false;
        for (a, b) in input.chunks(4096).zip(output.chunks(4096)) {
            c.check(a.len() as u64)?;
            changed |= a != b
        }
        drop(input);
        c.check(1)?;
        Ok(NativeStrokePatch { bytes: output, bounds: self.bounds, format: fmt, changed, _fee: fee })
    }
}

#[cfg(test)]
#[path="native_stroke_wide_tests.rs"]
mod tests;

/// Additive exact native selection path; no selection words pass through F32.
#[allow(clippy::too_many_arguments)]
pub fn prepare_clipped_native_wide_stroke_selected<'s>(source:CheckedRasterView<'s>,clip:Rect,brush:&BrushSettings,points:&[StrokePoint],color:NativeWideColor,selection:&'s Surface,lock_transparency:bool,zoom:f32,limits:NativeStrokeLimits,q:&BackingQuota,c:&ReadControl<'_>)->NResult<PreparedNativeWideStroke<'s>>{
 let selected=selection::CheckedNativeSelection::new(selection,source,q,limits.resident,c)?;
 let inner_limits=NativeStrokeLimits{resident:limits.resident.checked_sub(selected.retained_bytes()).ok_or(BackingError::Quota)?,..limits};
 let mut prepared=prepare_clipped_native_wide_stroke(source,clip,brush,points,color,lock_transparency,zoom,inner_limits,q,c)?;
 prepared.limits=limits;
 prepared.prepared_bytes=sum(prepared.prepared_bytes,selected.retained_bytes())?;
 if prepared.prepared_bytes>limits.resident{return Err(BackingError::Quota.into())}
 prepared.selection=Some(selected);Ok(prepared)
}
