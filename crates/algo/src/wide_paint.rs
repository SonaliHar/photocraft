//! Checked RGB/Gray U32/F64 gradient and precomputed bucket-region painting.
//! Colors/stops are native working samples: the caller must perform CMS conversion.
//! Normal source-over only; indexed assignment, dithering and other blends are separate.
use crate::paint::GradientShape;
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct PaintLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for PaintLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}

/// The first `format.channels()` values are native color(s), then alpha if present.
#[derive(Clone, Copy, Debug)]
pub struct NativeStop {
    pub position: f64,
    pub color: [f64; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct GradientOptions {
    pub from: (f64, f64),
    pub to: (f64, f64),
    pub shape: GradientShape,
    pub reverse: bool,
    pub opacity: f64,
    pub lock_transparency: bool,
}

/// Paint a checked native gradient over explicit bounds, retaining exact outside/default bytes.
/// Stops are ordered, positions are in0..=1; equal positions retain their ordered hard edge.
/// Empty/oversized stop sets, degenerate drag vectors and unsupported formats fail atomically.
#[allow(clippy::too_many_arguments)]
pub fn try_paint_gradient(
    target: &mut Surface,
    area: Rect,
    options: GradientOptions,
    stops: &[NativeStop],
    selection: Option<&Surface>,
    limits: PaintLimits,
    control: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    let format = target.format();
    validate_format(format)?;
    if stops.is_empty() || stops.len() > 4096 {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut previous = 0.0;
    for (index, stop) in stops.iter().enumerate() {
        if index.is_multiple_of(256) && control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        if !stop.position.is_finite() || !(0.0..=1.0).contains(&stop.position) || (index > 0 && stop.position < previous) {
            return Err(RasterInputError::InvalidSample);
        }
        validate_color(&stop.color, format)?;
        previous = stop.position;
    }
    let (dx, dy) = (options.to.0 - options.from.0, options.to.1 - options.from.1);
    let length = dx.hypot(dy);
    if ![options.from.0, options.from.1, options.to.0, options.to.1, length].iter().all(|v| v.is_finite()) || length == 0.0 {
        return Err(RasterInputError::InvalidSample);
    }
    let extra = (stops.len() as u64).checked_mul(std::mem::size_of::<NativeStop>() as u64).ok_or(RasterInputError::LimitExceeded)?;
    apply(
        target,
        area,
        options.opacity,
        selection,
        options.lock_transparency,
        limits,
        extra,
        control,
        |_, _| Ok(1.0),
        |x, y| {
            let (px, py) = (f64::from(x) + 0.5 - options.from.0, f64::from(y) + 0.5 - options.from.1);
            let (along, across) = (px * (dx / length) + py * (dy / length), -px * (dy / length) + py * (dx / length));
            let t = match options.shape {
                GradientShape::Linear => along / length,
                GradientShape::Radial => px.hypot(py) / length,
                GradientShape::Reflected => along.abs() / length,
                GradientShape::Diamond => (along.abs() + across.abs()) / length,
                GradientShape::Angle => (across.atan2(along) / std::f64::consts::TAU).rem_euclid(1.0),
            };
            if !t.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
            let t = t.clamp(0.0, 1.0);
            sample_stops(stops, if options.reverse { 1.0 - t } else { t }, format.channels())
        },
    )
}

/// Apply a previously sampled bucket region with exact f64 coverage and native foreground.
/// This does not qualify a tolerance/flood-region sampler or sample merged layers.
#[allow(clippy::too_many_arguments)]
pub fn try_paint_region(
    target: &mut Surface,
    area: Rect,
    coverage: &[f64],
    native_color: &[f64],
    opacity: f64,
    selection: Option<&Surface>,
    lock_transparency: bool,
    limits: PaintLimits,
    control: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    let format = target.format();
    validate_format(format)?;
    if native_color.len() != format.channels() {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut color = [0.0; 4];
    color.get_mut(..native_color.len()).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(native_color);
    validate_color(&color, format)?;
    let count = checked_region_byte_len(area, format)? / format.bytes_per_pixel();
    if count != coverage.len() {
        return Err(RasterInputError::LengthMismatch);
    }
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(RasterInputError::InvalidSample);
    }
    let extra = (coverage.len() as u64).checked_mul(8).ok_or(RasterInputError::LimitExceeded)?;
    check_budget(target, area, selection, limits, extra, control)?;
    for (index, value) in coverage.iter().enumerate() {
        if index.is_multiple_of(4096) && control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        if !value.is_finite() || !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    let width = usize::try_from(area.width()).map_err(|_| RasterInputError::InvalidBounds)?;
    apply(
        target,
        area,
        opacity,
        selection,
        lock_transparency,
        limits,
        extra,
        control,
        |x, y| {
            let index = usize::try_from(y - area.y0)
                .ok()
                .and_then(|y| y.checked_mul(width))
                .and_then(|i| usize::try_from(x - area.x0).ok().and_then(|x| i.checked_add(x)))
                .ok_or(RasterInputError::InvalidBounds)?;
            coverage.get(index).copied().ok_or(RasterInputError::LengthMismatch)
        },
        |_, _| Ok(color),
    )
}

/// Solid native fill without allocating a whole-canvas coverage buffer.
#[allow(clippy::too_many_arguments)]
pub fn try_paint_solid(
    target: &mut Surface,
    area: Rect,
    native_color: &[f64],
    opacity: f64,
    selection: Option<&Surface>,
    lock_transparency: bool,
    limits: PaintLimits,
    control: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    let format = target.format();
    validate_format(format)?;
    if native_color.len() != format.channels() {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut color = [0.0; 4];
    color.get_mut(..native_color.len()).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(native_color);
    validate_color(&color, format)?;
    apply(target, area, opacity, selection, lock_transparency, limits, 0, control, |_, _| Ok(1.0), |_, _| Ok(color))
}

/// Exact native-component bucket matching. No display quantization, tolerance or anti-aliasing.
#[derive(Clone, Copy, Debug)]
pub struct BucketOptions {
    pub seed: (i32, i32),
    pub contiguous: bool,
    pub opacity: f64,
    pub lock_transparency: bool,
}
#[allow(clippy::too_many_arguments)]
pub fn try_paint_bucket_exact(
    target: &mut Surface,
    area: Rect,
    options: BucketOptions,
    native_color: &[f64],
    selection: Option<&Surface>,
    limits: PaintLimits,
    control: &Interrupt<'_>,
) -> Result<bool, RasterInputError> {
    let format = target.format();
    validate_format(format)?;
    if native_color.len() != format.channels() {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut color = [0.0; 4];
    color.get_mut(..native_color.len()).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(native_color);
    validate_color(&color, format)?;
    if !options.opacity.is_finite() || !(0.0..=1.0).contains(&options.opacity) {
        return Err(RasterInputError::InvalidSample);
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    if !area.contains(options.seed.0, options.seed.1) {
        return Ok(false);
    }
    let count = checked_region_byte_len(area, format)? / format.bytes_per_pixel();
    let width = usize::try_from(area.width()).map_err(|_| RasterInputError::InvalidBounds)?;
    if width == 0 {
        return Err(RasterInputError::InvalidBounds);
    }
    let mut seed = [0.0; 4];
    target.try_read_pixel_f64(options.seed.0, options.seed.1, seed.get_mut(..format.channels()).ok_or(RasterInputError::LengthMismatch)?)?;
    if seed.iter().take(format.channels()).any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidSample);
    }
    check_budget(target, area, selection, limits, 0, control)?;
    // With no materialized tile intersecting this finite area, every sample is the
    // already-validated default. Both connected and global regions are the whole area.
    let mut materialized = false;
    for (index, (coordinate, _)) in target.tiles().enumerate() {
        if index.is_multiple_of(256) && control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let x = coordinate.tx.checked_mul(photocraft_geom::TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        let y = coordinate.ty.checked_mul(photocraft_geom::TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        let tile_rect = Rect::from_xywh(x, y, photocraft_geom::TILE_SIZE as u32, photocraft_geom::TILE_SIZE as u32);
        if !tile_rect.intersect(&area).is_empty() {
            materialized = true;
            break;
        }
    }
    if !materialized {
        try_paint_solid(target, area, native_color, options.opacity, selection, options.lock_transparency, limits, control)?;
        return Ok(true);
    }
    let per_pixel = if options.contiguous { 1 + std::mem::size_of::<usize>() as u64 } else { 1 };
    let extra = (count as u64).checked_mul(per_pixel).ok_or(RasterInputError::LimitExceeded)?;
    check_budget(target, area, selection, limits, extra, control)?;
    let work = (count as u64).checked_mul(format.channels() as u64 + 48).ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut marks = Vec::new();
    marks.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
    while marks.len() < count {
        if control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let next = marks.len().checked_add(4096).ok_or(RasterInputError::LimitExceeded)?.min(count);
        marks.resize(next, 0_u8);
    }
    let matches = |index| -> Result<bool, RasterInputError> {
        let (x, y) = bucket_coordinates(area, index, width)?;
        let mut pixel = [0.0; 4];
        target.try_read_pixel_f64(x, y, pixel.get_mut(..format.channels()).ok_or(RasterInputError::LengthMismatch)?)?;
        if pixel.iter().take(format.channels()).any(|v| !v.is_finite()) {
            return Err(RasterInputError::InvalidSample);
        }
        Ok(pixel.iter().zip(seed.iter()).take(format.channels()).all(|(a, b)| a == b))
    };
    if options.contiguous {
        let y = usize::try_from(i64::from(options.seed.1) - i64::from(area.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let x = usize::try_from(i64::from(options.seed.0) - i64::from(area.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let index = y.checked_mul(width).and_then(|v| v.checked_add(x)).ok_or(RasterInputError::InvalidBounds)?;
        let mut stack = Vec::new();
        stack.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
        *marks.get_mut(index).ok_or(RasterInputError::InvalidBounds)? = 2;
        stack.push(index);
        let mut visited = 0_usize;
        while let Some(index) = stack.pop() {
            if visited.is_multiple_of(256) && control.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            visited = visited.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
            let column = index % width;
            let neighbors = [
                if column > 0 { index.checked_sub(1) } else { None },
                if column + 1 < width { index.checked_add(1) } else { None },
                index.checked_sub(width),
                index.checked_add(width).filter(|v| *v < count),
            ];
            for neighbor in neighbors.into_iter().flatten() {
                let mark = marks.get_mut(neighbor).ok_or(RasterInputError::InvalidBounds)?;
                if *mark != 0 {
                    continue;
                }
                *mark = if matches(neighbor)? { 2 } else { 1 };
                if *mark == 2 {
                    stack.push(neighbor);
                }
            }
        }
    } else {
        for (index, mark) in marks.iter_mut().enumerate() {
            if index.is_multiple_of(256) && control.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            *mark = if matches(index)? { 2 } else { 1 };
        }
    }
    apply(
        target,
        area,
        options.opacity,
        selection,
        options.lock_transparency,
        limits,
        extra,
        control,
        |x, y| {
            let row = usize::try_from(i64::from(y) - i64::from(area.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
            let column = usize::try_from(i64::from(x) - i64::from(area.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
            let index = row.checked_mul(width).and_then(|v| v.checked_add(column)).ok_or(RasterInputError::InvalidBounds)?;
            Ok(if marks.get(index) == Some(&2) { 1.0 } else { 0.0 })
        },
        |_, _| Ok(color),
    )?;
    Ok(true)
}
fn bucket_coordinates(area: Rect, index: usize, width: usize) -> Result<(i32, i32), RasterInputError> {
    let x = i64::try_from(index % width)
        .ok()
        .and_then(|v| v.checked_add(i64::from(area.x0)))
        .and_then(|v| i32::try_from(v).ok())
        .ok_or(RasterInputError::InvalidBounds)?;
    let y = i64::try_from(index / width)
        .ok()
        .and_then(|v| v.checked_add(i64::from(area.y0)))
        .and_then(|v| i32::try_from(v).ok())
        .ok_or(RasterInputError::InvalidBounds)?;
    Ok((x, y))
}

fn validate_format(format: photocraft_color::PixelFormat) -> Result<(), RasterInputError> {
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::U32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    Ok(())
}
fn validate_color(color: &[f64; 4], format: photocraft_color::PixelFormat) -> Result<(), RasterInputError> {
    if color.iter().take(format.channels()).any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidSample);
    }
    if format.alpha && !color.get(format.mode.color_channels()).is_some_and(|a| (0.0..=1.0).contains(a)) {
        return Err(RasterInputError::InvalidSample);
    }
    Ok(())
}
fn sample_stops(stops: &[NativeStop], t: f64, channels: usize) -> Result<[f64; 4], RasterInputError> {
    let first = stops.first().ok_or(RasterInputError::LengthMismatch)?;
    if t <= first.position {
        return Ok(first.color);
    }
    let right = stops.partition_point(|stop| stop.position < t);
    let Some(b) = stops.get(right) else { return stops.last().map(|stop| stop.color).ok_or(RasterInputError::LengthMismatch) };
    let a = stops.get(right.checked_sub(1).ok_or(RasterInputError::LengthMismatch)?).ok_or(RasterInputError::LengthMismatch)?;
    let k = if b.position > a.position { (t - a.position) / (b.position - a.position) } else { 0.0 };
    let mut out = a.color;
    for (index, value) in out.iter_mut().enumerate().take(channels) {
        let b = b.color.get(index).copied().ok_or(RasterInputError::LengthMismatch)?;
        // A weighted sum avoids overflow in b-a for finite opposing HDR stops.
        *value = *value * (1.0 - k) + b * k;
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn check_budget(
    target: &Surface,
    area: Rect,
    selection: Option<&Surface>,
    limits: PaintLimits,
    extra: u64,
    control: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    target.try_validate_tile_origins_with(control)?;
    if let Some(selection) = selection {
        selection.try_validate_tile_origins_with(control)?;
    }
    let format = target.format();
    let bpp = format.bytes_per_pixel();
    let bytes = checked_region_byte_len(area, format)? as u64;
    let retained = target
        .resident_payload_bytes()?
        .checked_mul(2)
        .and_then(|v| v.checked_add(bytes))
        .and_then(|v| v.checked_add(extra))
        .and_then(|v| v.checked_add(256 * 256 * bpp as u64 * 3))
        .ok_or(RasterInputError::LimitExceeded)?;
    let peak = retained.checked_add(selection.map_or(Ok(0), Surface::resident_payload_bytes)?).ok_or(RasterInputError::LimitExceeded)?;
    // Per-pixel accounting includes at most12 binary stop comparisons plus native
    // channel reads/writes; no scan proportional to stop count occurs per pixel.
    let work = (bytes / bpp as u64).checked_mul(format.channels() as u64 + 32).and_then(|v| v.checked_add(extra)).ok_or(RasterInputError::LimitExceeded)?;
    if peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply(
    target: &mut Surface,
    area: Rect,
    opacity: f64,
    selection: Option<&Surface>,
    lock: bool,
    limits: PaintLimits,
    extra: u64,
    control: &Interrupt<'_>,
    coverage: impl Fn(i32, i32) -> Result<f64, RasterInputError>,
    color: impl Fn(i32, i32) -> Result<[f64; 4], RasterInputError>,
) -> Result<(), RasterInputError> {
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(RasterInputError::InvalidSample);
    }
    check_budget(target, area, selection, limits, extra, control)?;
    let format = target.format();
    let bpp = format.bytes_per_pixel();
    if opacity == 0.0 {
        return Ok(());
    }
    let mut staged = target.clone();
    for tile in area.tiles() {
        let part = tile.rect().intersect(&area);
        let width = usize::try_from(part.width()).map_err(|_| RasterInputError::InvalidBounds)?;
        let mut raw = target.try_read_interleaved_with(part, control)?;
        let mut changed = false;
        for (index, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) && control.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            let x = part.x0 + (index % width) as i32;
            let y = part.y0 + (index / width) as i32;
            let weight = coverage(x, y)? * opacity * crate::wide::selection_weight(selection, x, y)?;
            if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
                return Err(RasterInputError::InvalidSample);
            }
            if weight == 0.0 {
                continue;
            }
            let alpha = format.mode.color_channels();
            let src = color(x, y)?;
            validate_color(&src, format)?;
            let sa = weight * if format.alpha { *src.get(alpha).ok_or(RasterInputError::LengthMismatch)? } else { 1.0 };
            if sa == 0.0 {
                continue;
            }
            if lock && format.alpha {
                let a = try_read_sample_f64(pixel, format.sample, alpha).map_err(|_| RasterInputError::InvalidSample)?;
                if !a.is_finite() || !(0.0..=1.0).contains(&a) {
                    return Err(RasterInputError::InvalidSample);
                }
                if a == 0.0 {
                    continue;
                }
            }
            let mut dst = [0.0; 4];
            for (channel, value) in dst.iter_mut().enumerate().take(format.channels()) {
                *value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
                if !value.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
            }
            let da = if format.alpha { *dst.get(alpha).ok_or(RasterInputError::LengthMismatch)? } else { 1.0 };
            if !(0.0..=1.0).contains(&da) {
                return Err(RasterInputError::InvalidSample);
            }
            if lock && da == 0.0 {
                continue;
            }
            let oa = sa + da * (1.0 - sa);
            let mut original = [0_u8; 32];
            original.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(pixel);
            for channel in 0..alpha {
                let d = *dst.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                let s = *src.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                let value = if sa == 1.0 || da == 0.0 { s } else { (s * sa + d * da * (1.0 - sa)) / oa };
                if !value.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                try_write_sample_f64(pixel, format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
            }
            if format.alpha && !lock {
                try_write_sample_f64(pixel, format.sample, alpha, oa).map_err(|_| RasterInputError::InvalidSample)?;
            }
            changed |= pixel != original.get(..bpp).ok_or(RasterInputError::LengthMismatch)?;
        }
        if changed {
            staged.try_write_interleaved_with(part, &raw, control)?;
        }
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    *target = staged;
    Ok(())
}
