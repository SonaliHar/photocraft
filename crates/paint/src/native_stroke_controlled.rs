//! Paid bounded native Round brush producer. No document, source or edit authority.
//! One absolute-coordinate generation; operative clipping precedes coverage allocation.
use super::*;
use photocraft_raster::{
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
    checked_view::{CheckedRasterView, RasterViewError},
};
use std::cell::Cell;
#[derive(Clone, Copy, Debug)]
pub struct NativeStrokeLimits {
    resident: u64,
    work: u64,
    native_bytes: usize,
}
impl NativeStrokeLimits {
    pub const STANDARD: Self = Self { resident: 256 << 20, work: MAX_WORK, native_bytes: 16 << 20 };
    pub fn try_lowered(resident: u64, work: u64, native_bytes: usize) -> std::result::Result<Self, NativeStrokeError> {
        if resident > Self::STANDARD.resident || work > MAX_WORK || native_bytes > 16 << 20 {
            return Err(NativeStrokeError::Invalid("native stroke limits"));
        }
        Ok(Self { resident, work, native_bytes })
    }
}
#[derive(Debug)]
pub enum NativeStrokeError {
    Backing(BackingError),
    View(RasterViewError),
    Stroke(StrokeError),
    Invalid(&'static str),
}
impl std::fmt::Display for NativeStrokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native stroke: {self:?}")
    }
}
impl std::error::Error for NativeStrokeError {}
impl From<BackingError> for NativeStrokeError {
    fn from(e: BackingError) -> Self {
        Self::Backing(e)
    }
}
impl From<RasterViewError> for NativeStrokeError {
    fn from(e: RasterViewError) -> Self {
        Self::View(e)
    }
}
impl From<StrokeError> for NativeStrokeError {
    fn from(e: StrokeError) -> Self {
        Self::Stroke(e)
    }
}
type NResult<T> = std::result::Result<T, NativeStrokeError>;
fn invalid<T>(m: &'static str) -> NResult<T> {
    Err(NativeStrokeError::Invalid(m))
}
fn sum(a: u64, b: u64) -> NResult<u64> {
    a.checked_add(b).ok_or(NativeStrokeError::Backing(BackingError::Length))
}
fn product(a: u64, b: u64) -> NResult<u64> {
    a.checked_mul(b).ok_or(NativeStrokeError::Backing(BackingError::Length))
}
fn reserve(q: &BackingQuota, bytes: u64, cap: u64, c: &ReadControl<'_>) -> NResult<QuotaLease> {
    c.check(1)?;
    if bytes > cap {
        return Err(BackingError::Quota.into());
    }
    q.reserve(QuotaKind::Resident, bytes).map_err(|_| BackingError::Quota.into())
}
fn vector<T>(n: usize, c: &ReadControl<'_>) -> NResult<Vec<T>> {
    c.check(1)?;
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| BackingError::Allocation)?;
    if v.capacity() > n {
        return Err(BackingError::Allocation.into());
    }
    Ok(v)
}
fn accepted(b: &BrushSettings) -> NResult<()> {
    validate_brush(b)?;
    if !matches!(b.tip, TipShape::Round)
        || b.square
        || b.scattering.enabled
        || b.texture.enabled
        || b.dual_brush.enabled
        || b.color_dynamics.enabled
        || b.wet_edges
        || b.noise
        || b.build_up
        || b.mode != BlendMode::Normal
    {
        return invalid("this native lane supports ordinary Round Normal/Clear/erase/pencil only");
    }
    Ok(())
}
/// Scalar owned settings for this closed producer; inactive bitmap payloads are
/// not cloned. This is neither source nor document-edit authority.
pub fn checked_round_brush(brush: &BrushSettings) -> NResult<BrushSettings> {
    accepted(brush)?;
    Ok(numeric_brush(brush))
}
// All payload fields precede their conservative admitted ownership envelopes.
pub struct PreparedNativeStroke<'s> {
    source: CheckedRasterView<'s>,
    ctx: BrushContext,
    cov: CoverageMap,
    bounds: Rect,
    lock_alpha: bool,
    limits: NativeStrokeLimits,
    quota: BackingQuota,
    prepared_bytes: u64,
    selection: Option<selection::CheckedNativeSelection<'s>>,
    _coverage: QuotaLease,
    _generation: QuotaLease,
}
pub struct NativeStrokePatch {
    bytes: Vec<u8>,
    bounds: Rect,
    format: PixelFormat,
    changed: bool,
    _fee: QuotaLease,
}
impl NativeStrokePatch {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn changed(&self) -> bool {
        self.changed
    }
}
fn generation(brush: &BrushSettings, points: &[StrokePoint], c: &ReadControl<'_>) -> NResult<(usize, usize, u64)> {
    if points.is_empty() || points.len() > MAX_POINTS {
        return invalid("bounded nonempty points required");
    }
    let mut distance = 0.0;
    let mut bounds = Rect::EMPTY;
    let mut previous: Option<&StrokePoint> = None;
    for p in points {
        c.check(1)?;
        if !p.x.is_finite() || !p.y.is_finite() || !p.time.is_finite() || p.x.abs() > f64::from(1 << 27) || p.y.abs() > f64::from(1 << 27) {
            return invalid("point coordinates/time");
        }
        finite(&[p.pressure, p.tilt_x, p.tilt_y, p.rotation, p.wheel])?;
        if let Some(old) = previous {
            distance += (p.x - old.x).hypot(p.y - old.y)
        }
        previous = Some(p);
        let x = p.x.floor() as i32;
        let y = p.y.floor() as i32;
        bounds = bounds.union(&Rect::new(x, y, x + 1, y + 1));
    }
    if brush.smoothing.amount > 0.0 {
        distance = f64::from(bounds.width()).hypot(f64::from(bounds.height())) * (points.len() as f64 * 64.0 + 1.0)
    }
    let smoothed =
        (points.len() as u64).checked_mul(if brush.smoothing.amount > 0.0 { 64 } else { 1 }).and_then(|v| v.checked_add(1)).ok_or(BackingError::Length)?;
    let travel = (distance * 4.0).ceil();
    if !travel.is_finite() || travel > MAX_DABS as f64 {
        return invalid("native dab count");
    }
    let steps = sum(sum(travel as u64, product(smoothed, 2)?)?, 2)?;
    if steps > MAX_DABS as u64 {
        return invalid("native dab count");
    }
    let bytes = sum(
        sum(product(steps, std::mem::size_of::<Dab>() as u64)?, product(steps, std::mem::size_of::<StepInput>() as u64)?)?,
        sum(product(points.len() as u64, std::mem::size_of::<StrokePoint>() as u64)?, 8192)?,
    )?;
    Ok((usize::try_from(steps).map_err(|_| BackingError::Length)?, usize::try_from(steps).map_err(|_| BackingError::Length)?, product(bytes, 2)?))
}
fn clipped_raster(ctx: &BrushContext, dabs: &[Dab], clip: Rect, cov: &mut CoverageMap, maximum: usize, b: &mut Budget<'_, '_>) -> Result<()> {
    let mut scratch = b.vector(maximum)?;
    let mut row = b.vector(maximum)?;
    for dab in dabs {
        b.poll()?;
        let rect = ctx.dab_rect(dab, false).intersect(&clip);
        if rect.is_empty() {
            continue;
        }
        check_rect(rect)?;
        scratch.clear();
        for y in rect.y0..rect.y1 {
            b.tick((rect.width() as u64).checked_mul(64).ok_or(StrokeError::Limit)?)?;
            ctx.rasterize_into(dab, false, Rect::new(rect.x0, y, rect.x1, y + 1), &mut row, None);
            if scratch.len().checked_add(row.len()).is_none_or(|n| n > scratch.capacity()) {
                return Err(StrokeError::Limit);
            }
            scratch.extend_from_slice(&row)
        }
        accumulate(cov, rect, &scratch, dab.opacity, None, None, b)?;
        cov.bounds = cov.bounds.union(&rect);
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub fn prepare_clipped_native_stroke<'s>(
    source: CheckedRasterView<'s>,
    clip: Rect,
    brush: &BrushSettings,
    points: &[StrokePoint],
    selection: Option<&'s Surface>,
    lock_transparency: bool,
    zoom: f32,
    limits: NativeStrokeLimits,
    q: &BackingQuota,
    c: &ReadControl<'_>,
) -> NResult<PreparedNativeStroke<'s>> {
    c.check(1)?;
    accepted(brush)?;
    let fmt = source.format();
    if selection.is_some() {
        return invalid("native stroke selection is not yet supported");
    }
    if !matches!(fmt.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
        || !matches!(fmt.sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
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
    let gen_bytes = sum(gen_bytes, std::mem::size_of::<PreparedNativeStroke<'_>>() as u64)?;
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
    let numeric = numeric_brush(brush);
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
    Ok(PreparedNativeStroke {
        source,
        ctx,
        cov,
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
impl PreparedNativeStroke<'_> {
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
            return invalid("native patch cap");
        }
        let runtime_bytes = sum(
            sum(product(bytes as u64, 2)?, std::mem::size_of::<NativeStrokePatch>() as u64 + 1024)?,
            self.selection.as_ref().map_or(0, selection::CheckedNativeSelection::runtime_bytes),
        )?;
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
                    pin.read_pixel_native_into(x, y, input.get_mut(at..at + bpp).ok_or(BackingError::Length)?, c)?
                }
            }
        }
        for (a, b) in output.chunks_mut(4096).zip(input.chunks(4096)) {
            c.check(b.len() as u64)?;
            a.copy_from_slice(b)
        }
        let brush = &self.ctx.brush;
        let mut src = [0.0f32; 8];
        from_rgba_into(&fmt, brush.color, &mut src);
        if let Some(a) = alpha_index(&fmt) {
            *src.get_mut(a).ok_or(BackingError::Length)? = brush.color[3]
        }
        for (&(tx, ty), tile) in &self.cov.tiles {
            let rect = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&self.bounds);
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    c.check(256)?;
                    let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                    let coverage = *tile.cov.get(ti).ok_or(BackingError::Length)?;
                    if coverage <= 0.0 {
                        continue;
                    }
                    let k = if let Some(selection) = &self.selection {
                        c.check(128)?;
                        let m = self.ctx.stroke_mask_wet(coverage, None, None, x, y);
                        let opacity = brush.opacity.clamp(0.0, 1.0);
                        let s = selection.narrow(x, y, c)?;
                        (m * opacity * s).min(1.0)
                    } else {
                        (self.ctx.stroke_mask_wet(coverage, None, None, x, y) * brush.opacity.clamp(0.0, 1.0)).min(1.0)
                    };
                    if !k.is_finite() {
                        return invalid("nonfinite native coverage");
                    }
                    if k <= 0.0 {
                        continue;
                    }
                    let at = ((y - self.bounds.y0) as usize * self.bounds.width() as usize + (x - self.bounds.x0) as usize) * bpp;
                    let before = input.get(at..at + bpp).ok_or(BackingError::Length)?;
                    let mut px = [0.0f32; 8];
                    for (i, v) in px.iter_mut().take(n).enumerate() {
                        *v = read_sample(before, fmt.sample, i)
                    }
                    if px.iter().take(n).any(|v| !v.is_finite()) || alpha_index(&fmt).is_some_and(|a| px.get(a).is_none_or(|v| !(0.0..=1.0).contains(v))) {
                        return invalid("unsupported native source arithmetic");
                    }
                    if brush.erase {
                        if let (Some(a), false) = (alpha_index(&fmt), self.lock_alpha) {
                            *px.get_mut(a).ok_or(BackingError::Length)? *= 1.0 - k
                        }
                    } else {
                        if self.lock_alpha && alpha_index(&fmt).is_some_and(|a| px.get(a).is_some_and(|v| *v <= 0.0)) {
                            continue;
                        }
                        over_native(&fmt, px.get_mut(..n).ok_or(BackingError::Length)?, src.get(..n).ok_or(BackingError::Length)?, k, self.lock_alpha)
                    }
                    if px.iter().take(n).any(|v| !v.is_finite()) || alpha_index(&fmt).is_some_and(|a| px.get(a).is_none_or(|v| !(0.0..=1.0).contains(v))) {
                        return invalid("nonfinite native brush arithmetic");
                    }
                    if fmt.sample == SampleType::U8 && !brush.aliased {
                        dither8(px.get_mut(..n).ok_or(BackingError::Length)?, x, y)
                    }
                    if px.iter().take(n).any(|v| !v.is_finite()) {
                        return invalid("nonfinite native brush result");
                    }
                    let out = output.get_mut(at..at + bpp).ok_or(BackingError::Length)?;
                    for (i, v) in px.iter().take(n).enumerate() {
                        write_sample(out, fmt.sample, i, *v)
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
#[path = "native_stroke_tests.rs"]
mod tests;

#[path = "native_stroke_wide_controlled.rs"]
mod wide_native;
pub use wide_native::{NativeWideColor, PreparedNativeWideStroke, prepare_clipped_native_wide_stroke, prepare_clipped_native_wide_stroke_selected};

#[path = "native_selection.rs"]
mod selection;

/// Additive selected producer. The caller retains the actual installed owner;
/// this borrowed selection proof supplies no document-edit authority.
#[allow(clippy::too_many_arguments)]
pub fn prepare_clipped_native_stroke_selected<'s>(
    source: CheckedRasterView<'s>,
    clip: Rect,
    brush: &BrushSettings,
    points: &[StrokePoint],
    selection: &'s Surface,
    lock_transparency: bool,
    zoom: f32,
    limits: NativeStrokeLimits,
    q: &BackingQuota,
    c: &ReadControl<'_>,
) -> NResult<PreparedNativeStroke<'s>> {
    let selected = selection::CheckedNativeSelection::new(selection, source, q, limits.resident, c)?;
    let inner_limits = NativeStrokeLimits { resident: limits.resident.checked_sub(selected.retained_bytes()).ok_or(BackingError::Quota)?, ..limits };
    let mut prepared = prepare_clipped_native_stroke(source, clip, brush, points, None, lock_transparency, zoom, inner_limits, q, c)?;
    prepared.limits = limits;
    prepared.prepared_bytes = sum(prepared.prepared_bytes, selected.retained_bytes())?;
    if prepared.prepared_bytes > limits.resident {
        return Err(BackingError::Quota.into());
    }
    prepared.selection = Some(selected);
    Ok(prepared)
}
