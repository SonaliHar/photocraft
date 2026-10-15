//! Independent discrete Gaussian convolution from the public normal distribution.
//! The caller supplies linear working-space samples and owns profile conversion.
//! Sampled FIR and independently derived Young/van Vliet kernels do not imply
//! qualification of every GEGL Auto/IIR variant or source color context.
use photocraft_color::try_write_sample_f64;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
mod box_blur;
mod finite;
mod median;
mod motion;
mod pixelize;
mod recursive_requirements;
pub use recursive_requirements::gaussian_young_recursive_requirements;
pub use box_blur::{box_blur_extent, box_blur_requirements, try_box_blur};
pub use finite::{GaussianFilter, gaussian_finite_requirements, try_gaussian_finite_clamp, try_gaussian_small_auto};
pub use median::{MedianNeighborhood, MedianParams, median_requirements, try_median_blur, try_median_blur_quantized_encoded};
pub use motion::{MotionBlurParams, motion_blur_extent, motion_blur_requirements, try_motion_blur_linear};
pub use pixelize::{PixelizeParams, pixelize_requirements, try_pixelize_square};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaussianAbyss {
    Clamp,
    None,
}

#[derive(Clone, Copy, Debug)]
pub struct GaussianParams {
    pub std_dev_x: f64,
    pub std_dev_y: f64,
    pub abyss: GaussianAbyss,
    /// Operation extent clipping; outer drawable-filter clipping is separate.
    pub clip_extent: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct GaussianLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}

/// Existing logical peak/work admission formulas for one neighborhood call.
/// This report allocates nothing and conveys neither working-space authority
/// nor a quota reservation or proof of actual allocator capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeighborhoodRequirements {
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}

// A legacy no-op clones a Surface before considering its numeric limits. Owned
// callers still need an admission envelope: include shared payload twice plus
// map/default/header bookkeeping, without asserting physical duplication or a
// reservation. Every metadata entry is polled; tile sample bytes are not read.
pub(super) fn clone_requirements(source: &Surface, interrupt: &Interrupt<'_>) -> Result<NeighborhoodRequirements, RasterInputError> {
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    let mut payload = source.default_bytes().len() as u64;
    for (_, tile) in source.tiles() {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        payload = payload.checked_add(tile.bytes().len() as u64).ok_or(RasterInputError::LimitExceeded)?;
    }
    let map = (source.tile_count() as u64).checked_mul(128).ok_or(RasterInputError::LimitExceeded)?;
    let metadata = map
        .checked_add((source.default_bytes().len() as u64).checked_mul(2).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|v| v.checked_add(std::mem::size_of::<Surface>() as u64 + 128))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(NeighborhoodRequirements {
        peak_resident_bytes: payload.checked_mul(2).and_then(|v| v.checked_add(metadata)).ok_or(RasterInputError::LimitExceeded)?,
        work_units: metadata,
    })
}

// The enclosing operation owns its aggregate transaction reservations and deadline;
// its Interrupt must expose the job's cancellation/deadline token. These sub-budgets
// bound this invocation, not other simultaneous graph nodes or history states.

impl Default for GaussianLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 } }
    }
}

/// Evaluate only the requested ROI, retaining exact source/default bytes elsewhere.
/// Alpha is convolved in premultiplied form then unassociated; zero-alpha output
/// has zero color samples. No clamping of finite HDR color channels is performed.
pub fn try_gaussian_fir(
    source: &Surface,
    output: Rect,
    input_extent: Rect,
    params: GaussianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    evaluate(source, output, input_extent, params, limits, interrupt, KernelKind::Sampled)
}

/// Independently derived symmetric impulse-response convolution of Young/van
/// Vliet (1995), equations 8c–11b, DOI 10.1016/0165-1684(95)00020-E.
/// https://repository.tudelft.nl/record/uuid:6fea1bc9-de9c-450f-8e34-a33c1129589a
/// Qualification against a GEGL Auto configuration is a separate oracle gate.
pub fn try_gaussian_young(
    source: &Surface,
    output: Rect,
    input_extent: Rect,
    params: GaussianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    evaluate(source, output, input_extent, params, limits, interrupt, KernelKind::Young)
}

#[derive(Clone, Copy)]
enum KernelKind {
    Sampled,
    Young,
}

#[allow(clippy::too_many_arguments)]
fn evaluate(
    source: &Surface,
    output: Rect,
    input_extent: Rect,
    params: GaussianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
    kind: KernelKind,
) -> Result<Surface, RasterInputError> {
    checked_region_byte_len(output, source.format())?;
    checked_region_byte_len(input_extent, source.format())?;
    if input_extent.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    for sigma in [params.std_dev_x, params.std_dev_y] {
        if !sigma.is_finite() || !(0.0..=1500.0).contains(&sigma) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    let output = if params.clip_extent { output.intersect(&input_extent) } else { output };
    if output.is_empty() {
        return Ok(source.clone());
    }
    if limits.work_units == 0 || limits.resident_bytes == 0 {
        return Err(RasterInputError::LimitExceeded);
    }
    if params.std_dev_x == 0.0 && params.std_dev_y == 0.0 {
        return Ok(source.clone());
    }
    if matches!(kind, KernelKind::Young) && [params.std_dev_x, params.std_dev_y].iter().any(|s| *s > 0.0 && *s < 0.5) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let nx = kernel_length(params.std_dev_x, kind)?;
    let ny = kernel_length(params.std_dev_y, kind)?;
    let kernel_bytes = (nx as u64).checked_add(ny as u64).and_then(|v| v.checked_mul(8)).ok_or(RasterInputError::LimitExceeded)?;
    let retained = source.resident_payload_bytes()?.checked_add(kernel_bytes).ok_or(RasterInputError::LimitExceeded)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    let temporary_kernel_bytes = if matches!(kind, KernelKind::Young) {
        (nx.max(ny) as u64).checked_mul(2).and_then(|v| v.checked_sub(1)).and_then(|v| v.checked_mul(8)).ok_or(RasterInputError::LimitExceeded)?
    } else {
        0
    };
    if retained.checked_add(temporary_kernel_bytes).ok_or(RasterInputError::LimitExceeded)? > cap {
        return Err(RasterInputError::LimitExceeded);
    }
    let ry = i32::try_from(ny / 2).map_err(|_| RasterInputError::LimitExceeded)?;
    let rx = i32::try_from(nx / 2).map_err(|_| RasterInputError::LimitExceeded)?;
    let channels = source.channels();
    // Conservatively debit impulse generation/normalization before doing it.
    let mut expected_work = (nx as u64).checked_add(ny as u64).and_then(|v| v.checked_mul(32)).ok_or(RasterInputError::LimitExceeded)?;
    for tile in output.tiles() {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let part = tile.rect().intersect(&output);
        let width = u64::from(part.width());
        let height = u64::from(part.height());
        let rows = height.checked_add((ry as u64).checked_mul(2).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        let horizontal = width.checked_mul(rows).and_then(|v| v.checked_mul(nx as u64)).ok_or(RasterInputError::LimitExceeded)?;
        let decode = width.checked_add((nx - 1) as u64).and_then(|v| v.checked_mul(rows)).ok_or(RasterInputError::LimitExceeded)?;
        let vertical = width.checked_mul(height).and_then(|v| v.checked_mul(ny as u64)).ok_or(RasterInputError::LimitExceeded)?;
        expected_work = expected_work
            .checked_add(
                horizontal
                    .checked_add(vertical)
                    .and_then(|v| v.checked_add(decode))
                    .and_then(|v| v.checked_mul(channels as u64))
                    .and_then(|v| v.checked_mul(8))
                    .ok_or(RasterInputError::LimitExceeded)?,
            )
            .ok_or(RasterInputError::LimitExceeded)?;
    }
    if expected_work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let kx = kernel(params.std_dev_x, kind, interrupt)?;
    let ky = kernel(params.std_dev_y, kind, interrupt)?;
    let mut out = source.clone();
    let mut poll_units = 0_usize;
    for tile in output.tiles() {
        let part = tile.rect().intersect(&output);
        let width = part.width() as usize;
        let height = part.height() as usize;
        let rows = height.checked_add((ry as usize).checked_mul(2).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        let samples = width.checked_mul(rows).and_then(|v| v.checked_mul(channels)).ok_or(RasterInputError::LimitExceeded)?;
        let row_pixels = width.checked_add(nx.checked_sub(1).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        let row_samples = row_pixels.checked_mul(channels).ok_or(RasterInputError::LimitExceeded)?;
        let raw_len = checked_region_byte_len(part, source.format())?;
        // Retained original + cumulative output payload + both scratch buffers + replacement tile.
        let scratch = (samples as u64)
            .checked_add(row_samples as u64)
            .and_then(|v| v.checked_mul(8))
            .and_then(|v| v.checked_add(raw_len as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        let tile_bytes = (256_u64 * 256).checked_mul(source.format().bytes_per_pixel() as u64).ok_or(RasterInputError::LimitExceeded)?;
        let peak = retained
            .checked_add(out.resident_payload_bytes()?)
            .and_then(|v| v.checked_add(scratch))
            .and_then(|v| v.checked_add(tile_bytes))
            .ok_or(RasterInputError::LimitExceeded)?;
        if peak > cap {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut tmp = allocate::<f64>(samples)?;
        // Decode each halo sample once per row. Tap order/arithmetic remain exact;
        // the bounded row cache avoids a tile lookup and decode for every tap.
        let mut row_values = allocate::<f64>(row_samples)?;
        for row in 0..rows {
            let y = i64::from(part.y0) + row as i64 - i64::from(ry);
            for (col, pixel) in row_values.chunks_exact_mut(channels).enumerate() {
                poll(&mut poll_units, channels, interrupt)?;
                let x = i64::from(part.x0) + col as i64 - i64::from(rx);
                read_premultiplied(source, input_extent, params.abyss, x, y, pixel)?;
            }
            for col in 0..width {
                let start = (row * width + col) * channels;
                let dst = tmp.get_mut(start..start + channels).ok_or(RasterInputError::LengthMismatch)?;
                for (tap, weight) in kx.iter().enumerate() {
                    poll(&mut poll_units, channels, interrupt)?;
                    let offset = (col + tap) * channels;
                    let pixel = row_values.get(offset..offset + channels).ok_or(RasterInputError::LengthMismatch)?;
                    for (dst, value) in dst.iter_mut().zip(pixel) {
                        *dst += *value * weight;
                    }
                }
            }
        }
        let mut raw = allocate::<u8>(raw_len)?;
        for row in 0..height {
            for col in 0..width {
                let mut values = [0.0_f64; 5];
                for (tap, weight) in ky.iter().enumerate() {
                    poll(&mut poll_units, channels, interrupt)?;
                    let start = ((row + tap) * width + col) * channels;
                    for (dst, src) in values.iter_mut().zip(tmp.get(start..start + channels).ok_or(RasterInputError::LengthMismatch)?) {
                        *dst += src * weight;
                    }
                }
                if source.format().alpha {
                    let alpha = values.get(channels - 1).copied().ok_or(RasterInputError::LengthMismatch)?;
                    for value in values.iter_mut().take(channels - 1) {
                        *value = if alpha > 0.0 { *value / alpha } else { 0.0 };
                    }
                }
                let start = (row * width + col) * source.format().bytes_per_pixel();
                let pixel = raw.get_mut(start..start + source.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
                for (channel, value) in values.iter().enumerate().take(channels) {
                    if !value.is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    try_write_sample_f64(pixel, source.format().sample, channel, *value).map_err(|_| RasterInputError::InvalidSample)?;
                }
            }
        }
        out.try_write_interleaved_with(part, &raw, interrupt)?;
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(out)
}

fn kernel(sigma: f64, kind: KernelKind, interrupt: &Interrupt<'_>) -> Result<Vec<f64>, RasterInputError> {
    if sigma == 0.0 {
        let mut values = allocate::<f64>(1)?;
        if let Some(value) = values.first_mut() {
            *value = 1.0;
        }
        return Ok(values);
    }
    if matches!(kind, KernelKind::Young) {
        return young_kernel(sigma, interrupt);
    }
    let radius = (sigma * 4.0).ceil() as i32;
    let length = kernel_length(sigma, kind)?;
    let mut values = allocate::<f64>(length)?;
    let mut sum = 0.0;
    for (index, value) in values.iter_mut().enumerate() {
        if index.is_multiple_of(4096) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let x = index as f64 - f64::from(radius);
        *value = (-0.5 * (x / sigma).powi(2)).exp();
        sum += *value;
    }
    if !sum.is_finite() || sum <= 0.0 {
        return Err(RasterInputError::InvalidSample);
    }
    for (index, value) in values.iter_mut().enumerate() {
        if index.is_multiple_of(4096) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        *value /= sum;
    }
    Ok(values)
}

fn kernel_length(sigma: f64, kind: KernelKind) -> Result<usize, RasterInputError> {
    let radius = if sigma == 0.0 {
        0
    } else {
        match kind {
            KernelKind::Sampled => (sigma * 4.0).ceil() as i32,
            KernelKind::Young => (sigma * 12.0).ceil() as i32 + 32,
        }
    };
    usize::try_from(radius).ok().and_then(|v| v.checked_mul(2)).and_then(|v| v.checked_add(1)).ok_or(RasterInputError::LimitExceeded)
}

fn young_kernel(sigma: f64, interrupt: &Interrupt<'_>) -> Result<Vec<f64>, RasterInputError> {
    let [gain, b1, b2, b3] = young_coefficients(sigma)?;
    let length = kernel_length(sigma, KernelKind::Young)?;
    let work_length = length.checked_mul(2).and_then(|v| v.checked_sub(1)).ok_or(RasterInputError::LimitExceeded)?;
    let center = work_length / 2;
    let mut work = allocate::<f64>(work_length)?;
    let mut history = [0.0; 3];
    for (index, value) in work.iter_mut().enumerate() {
        if index.is_multiple_of(512) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let output = if index == center { gain } else { 0.0 } + b1 * history[0] + b2 * history[1] + b3 * history[2];
        *value = output;
        history = [output, history[0], history[1]];
    }
    history.fill(0.0);
    for (index, value) in work.iter_mut().rev().enumerate() {
        if index.is_multiple_of(512) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let output = gain * *value + b1 * history[0] + b2 * history[1] + b3 * history[2];
        *value = output;
        history = [output, history[0], history[1]];
    }
    let radius = length / 2;
    let slice = work.get(center - radius..center + radius + 1).ok_or(RasterInputError::LengthMismatch)?;
    let mut sum = 0.0;
    for (index, value) in slice.iter().enumerate() {
        if index.is_multiple_of(4096) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        sum += value;
    }
    if !sum.is_finite() || sum <= 0.0 {
        return Err(RasterInputError::InvalidSample);
    }
    let mut values = allocate::<f64>(length)?;
    for (index, (value, src)) in values.iter_mut().zip(slice).enumerate() {
        if index.is_multiple_of(4096) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        *value = src / sum;
    }
    Ok(values)
}

fn young_coefficients(sigma: f64) -> Result<[f64; 4], RasterInputError> {
    let q = if sigma >= 2.5 { 0.98711 * sigma - 0.96330 } else { 3.97156 - 4.14554 * (1.0 - 0.26891 * sigma).sqrt() };
    let b0 = 1.57825 + 2.44413 * q + 1.4281 * q * q + 0.422205 * q * q * q;
    let b1 = (2.44413 * q + 2.85619 * q * q + 1.26661 * q * q * q) / b0;
    let b2 = -(1.4281 * q * q + 1.26661 * q * q * q) / b0;
    let b3 = 0.422205 * q * q * q / b0;
    let gain = 1.0 - b1 - b2 - b3;
    if !gain.is_finite() || gain <= 0.0 {
        return Err(RasterInputError::InvalidSample);
    }
    Ok([gain, b1, b2, b3])
}

/// Direct forward/backward Young recurrence, with guarded tile-local line stages.
/// Constant endpoint history follows the stationary solution y=x (gain+sum(b)=1).
/// A guard of twice the convolution radius isolates artificial tile endpoints;
/// this approximates infinite-tail recursion, while `try_gaussian_young` remains
/// the explicit finite normalized impulse-convolution alternative/oracle.
pub fn try_gaussian_young_recursive(
    source: &Surface,
    output: Rect,
    input_extent: Rect,
    params: GaussianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let output = recursive_requirements::validated_output(source, output, input_extent, params, interrupt)?;
    if limits.work_units == 0 || limits.resident_bytes == 0 {
        return Err(RasterInputError::LimitExceeded);
    }
    if output.is_empty() || params.std_dev_x == 0.0 && params.std_dev_y == 0.0 {
        return Ok(source.clone());
    }
    let gx = kernel_length(params.std_dev_x, KernelKind::Young)?.checked_sub(1).ok_or(RasterInputError::LimitExceeded)?;
    let gy = kernel_length(params.std_dev_y, KernelKind::Young)?.checked_sub(1).ok_or(RasterInputError::LimitExceeded)?;
    let channels = source.channels();
    let work = recursive_requirements::work(output, gx, gy, channels, interrupt)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut out = source.clone();
    let mut poll_units = 0;
    for tile in output.tiles() {
        let part = tile.rect().intersect(&output);
        let recursive_requirements::TileRequirements { width, height, line_width, rows, samples, line_samples, raw_len, scratch } =
            recursive_requirements::tile_requirements(source, part, gx, gy)?;
        let peak = recursive_requirements::peak(source.resident_payload_bytes()?, out.resident_payload_bytes()?, scratch, source.format().bytes_per_pixel())?;
        if peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut tmp = allocate::<f64>(samples)?;
        let mut line = allocate::<f64>(line_samples)?;
        for row in 0..rows {
            let y = i64::from(part.y0) + row as i64 - gy as i64;
            let row_values = line.get_mut(..line_width * channels).ok_or(RasterInputError::LengthMismatch)?;
            for (col, pixel) in row_values.chunks_exact_mut(channels).enumerate() {
                poll(&mut poll_units, channels, interrupt)?;
                read_premultiplied(source, input_extent, params.abyss, i64::from(part.x0) + col as i64 - gx as i64, y, pixel)?;
            }
            recursive_line(row_values, channels, params.std_dev_x, &mut poll_units, interrupt)?;
            let from = gx * channels;
            let count = width * channels;
            let dest = row * count;
            tmp.get_mut(dest..dest + count)
                .ok_or(RasterInputError::LengthMismatch)?
                .copy_from_slice(row_values.get(from..from + count).ok_or(RasterInputError::LengthMismatch)?);
        }
        for col in 0..width {
            let values = line.get_mut(..rows * channels).ok_or(RasterInputError::LengthMismatch)?;
            for (row, pixel) in values.chunks_exact_mut(channels).enumerate() {
                poll(&mut poll_units, channels, interrupt)?;
                let from = (row * width + col) * channels;
                pixel.copy_from_slice(tmp.get(from..from + channels).ok_or(RasterInputError::LengthMismatch)?);
            }
            recursive_line(values, channels, params.std_dev_y, &mut poll_units, interrupt)?;
            for row in 0..height {
                let from = (row + gy) * channels;
                let dest = (row * width + col) * channels;
                tmp.get_mut(dest..dest + channels)
                    .ok_or(RasterInputError::LengthMismatch)?
                    .copy_from_slice(values.get(from..from + channels).ok_or(RasterInputError::LengthMismatch)?);
            }
        }
        let mut raw = allocate::<u8>(raw_len)?;
        for (index, pixel) in raw.chunks_exact_mut(source.format().bytes_per_pixel()).enumerate() {
            poll(&mut poll_units, channels, interrupt)?;
            let values = tmp.get(index * channels..(index + 1) * channels).ok_or(RasterInputError::LengthMismatch)?;
            let alpha = if source.format().alpha { values.last().copied().ok_or(RasterInputError::LengthMismatch)? } else { 1.0 };
            for (channel, value) in values.iter().enumerate() {
                let value = if source.format().alpha && channel + 1 < channels { if alpha > 0.0 { *value / alpha } else { 0.0 } } else { *value };
                if !value.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                try_write_sample_f64(pixel, source.format().sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
            }
        }
        out.try_write_interleaved_with(part, &raw, interrupt)?;
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(out)
}

fn recursive_line(values: &mut [f64], channels: usize, sigma: f64, units: &mut usize, interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if sigma == 0.0 {
        return Ok(());
    }
    let [gain, b1, b2, b3] = young_coefficients(sigma)?;
    let mut left = [0.0; 5];
    let mut right = [0.0; 5];
    left.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(values.get(..channels).ok_or(RasterInputError::LengthMismatch)?);
    right
        .get_mut(..channels)
        .ok_or(RasterInputError::LengthMismatch)?
        .copy_from_slice(values.get(values.len().checked_sub(channels).ok_or(RasterInputError::LengthMismatch)?..).ok_or(RasterInputError::LengthMismatch)?);
    let mut history = [left; 3];
    for pixel in values.chunks_exact_mut(channels) {
        poll(units, channels * 8, interrupt)?;
        for (channel, value) in pixel.iter_mut().enumerate() {
            let a = history.first().and_then(|v| v.get(channel)).copied().ok_or(RasterInputError::LengthMismatch)?;
            let b = history.get(1).and_then(|v| v.get(channel)).copied().ok_or(RasterInputError::LengthMismatch)?;
            let c = history.get(2).and_then(|v| v.get(channel)).copied().ok_or(RasterInputError::LengthMismatch)?;
            *value = gain * *value + b1 * a + b2 * b + b3 * c;
            if !value.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
            if let Some(v) = history.get_mut(2).and_then(|v| v.get_mut(channel)) {
                *v = b;
            }
            if let Some(v) = history.get_mut(1).and_then(|v| v.get_mut(channel)) {
                *v = a;
            }
            if let Some(v) = history.first_mut().and_then(|v| v.get_mut(channel)) {
                *v = *value;
            }
        }
    }
    history = [right; 3];
    for pixel in values.chunks_exact_mut(channels).rev() {
        poll(units, channels * 8, interrupt)?;
        for (channel, value) in pixel.iter_mut().enumerate() {
            let a = history.first().and_then(|v| v.get(channel)).copied().ok_or(RasterInputError::LengthMismatch)?;
            let b = history.get(1).and_then(|v| v.get(channel)).copied().ok_or(RasterInputError::LengthMismatch)?;
            let c = history.get(2).and_then(|v| v.get(channel)).copied().ok_or(RasterInputError::LengthMismatch)?;
            *value = gain * *value + b1 * a + b2 * b + b3 * c;
            if !value.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
            if let Some(v) = history.get_mut(2).and_then(|v| v.get_mut(channel)) {
                *v = b;
            }
            if let Some(v) = history.get_mut(1).and_then(|v| v.get_mut(channel)) {
                *v = a;
            }
            if let Some(v) = history.first_mut().and_then(|v| v.get_mut(channel)) {
                *v = *value;
            }
        }
    }
    Ok(())
}

fn read_premultiplied(source: &Surface, extent: Rect, abyss: GaussianAbyss, x: i64, y: i64, output: &mut [f64]) -> Result<(), RasterInputError> {
    let outside = x < i64::from(extent.x0) || x >= i64::from(extent.x1) || y < i64::from(extent.y0) || y >= i64::from(extent.y1);
    if outside && abyss == GaussianAbyss::None {
        output.fill(0.0);
        return Ok(());
    }
    let x = x.clamp(i64::from(extent.x0), i64::from(extent.x1) - 1);
    let y = y.clamp(i64::from(extent.y0), i64::from(extent.y1) - 1);
    source.try_read_pixel_f64(
        i32::try_from(x).map_err(|_| RasterInputError::InvalidBounds)?,
        i32::try_from(y).map_err(|_| RasterInputError::InvalidBounds)?,
        output,
    )?;
    if output.iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidSample);
    }
    if source.format().alpha {
        let alpha = output.last().copied().ok_or(RasterInputError::LengthMismatch)?;
        if !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::InvalidSample);
        }
        let colors = output.len() - 1;
        for value in output.iter_mut().take(colors) {
            *value *= alpha;
        }
    }
    Ok(())
}

fn allocate<T: Default + Clone>(length: usize) -> Result<Vec<T>, RasterInputError> {
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    checked_allocation_capacity::<T>(values.capacity(), length)?;
    values.resize(length, T::default());
    Ok(values)
}

// Check the actual allocator result before initialization or input copies. The
// aggregate owned caller reserves twice the old logical peak separately; this
// bounded capacity check is not a reservation or a physical RSS assertion.
pub(super) fn checked_allocation_capacity<T>(actual: usize, requested: usize) -> Result<(), RasterInputError> {
    let maximum = requested.checked_mul(2).ok_or(RasterInputError::LimitExceeded)?;
    let bytes = actual.checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)?;
    let bytes = u64::try_from(bytes).map_err(|_| RasterInputError::LimitExceeded)?;
    if actual < requested || actual > maximum || bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}

fn poll(units: &mut usize, count: usize, interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
    *units += count;
    if *units >= 4096 {
        *units = 0;
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "neighborhood/requirements_tests.rs"]
mod requirements_tests;
