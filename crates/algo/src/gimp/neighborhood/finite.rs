//! Finite backward initialization derived from Triggs, equations 3 and 8.
//! The independently observed stock variant uses the boundary continuation
//! [v_n,v_(n+1),v_(n+2)] as history before evaluating n once more.
//! This explicit alignment differs from the infinite-clamp evaluator.
//! Public mathematical source (paper 9, equations 3 and 8; no implementation):
//! https://thoth.inrialpes.fr/people/triggs/pubs/Triggs-hdr-papers.pdf
use super::{
    GaussianAbyss, GaussianLimits, GaussianParams, NeighborhoodRequirements, checked_allocation_capacity, clone_requirements, poll, read_premultiplied,
    young_coefficients,
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

/// Bounded observed small-Auto member: x=0 or stored f32 .8 (seven-tap FIR),
/// y=0 or stored f32 1.3 (finite Young/Triggs alignment), Clamp/clip=true.
/// Input extents above 256 in either axis are explicitly unsupported for this
/// initial full-line implementation. No other Auto sigma/operator is admitted.
pub fn try_gaussian_small_auto(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: GaussianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    if !params.std_dev_x.is_finite() || !params.std_dev_y.is_finite() {
        return Err(RasterInputError::InvalidSample);
    }
    if input.width() > 256
        || input.height() > 256
        || !(params.std_dev_x == 0. || params.std_dev_x == f64::from(0.8_f32))
        || !(params.std_dev_y == 0. || params.std_dev_y == f64::from(1.3_f32))
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    try_gaussian_finite_clamp(source, output, input, params, GaussianFilter::Auto, limits, interrupt)
}

/// Independently observed stock algorithm selection. Auto selects FIR below
/// sigma=1 and finite Young/Triggs IIR otherwise, separately for each axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaussianFilter {
    Auto,
    Fir,
    Iir,
}

struct FinitePlan {
    output: Rect,
    width: usize,
    height: usize,
    channels: usize,
    count: usize,
    line_count: usize,
    output_count: usize,
    requirements: NeighborhoodRequirements,
}
/// Allocation-free, cancellation-polled prospective kernel requirements.
/// Empty/clipped output or zero sigmas report a conservative source-clone
/// envelope; legacy constructors retain their pre-limit clone bypass. These
/// numbers are not quota receipts and do not grant any source or CMS policy.
pub fn gaussian_finite_requirements(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: GaussianParams,
    filter: GaussianFilter,
    interrupt: &Interrupt<'_>,
) -> Result<NeighborhoodRequirements, RasterInputError> {
    match plan(source, output, input, params, filter, interrupt)? {
        Some(plan) => Ok(plan.requirements),
        None => clone_requirements(source, interrupt),
    }
}
fn plan(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: GaussianParams,
    filter: GaussianFilter,
    interrupt: &Interrupt<'_>,
) -> Result<Option<FinitePlan>, RasterInputError> {
    checked_region_byte_len(input, source.format())?;
    checked_region_byte_len(output, source.format())?;
    source.try_validate_tile_origins_with(interrupt)?;
    if !params.std_dev_x.is_finite() || !params.std_dev_y.is_finite() {
        return Err(RasterInputError::InvalidSample);
    }
    if input.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    if !matches!(params.abyss, GaussianAbyss::Clamp)
        || !params.clip_extent
        || !(0.0..=15.0).contains(&params.std_dev_x)
        || !(0.0..=15.0).contains(&params.std_dev_y)
        || filter == GaussianFilter::Iir && [params.std_dev_x, params.std_dev_y].iter().any(|s| *s > 0. && *s < 0.5)
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let output = output.intersect(&input);
    if output.is_empty() || params.std_dev_x == 0. && params.std_dev_y == 0. {
        return Ok(None);
    }
    let width = input.width() as usize;
    let height = input.height() as usize;
    let channels = source.channels();
    let count = width.checked_mul(height).and_then(|n| n.checked_mul(channels)).ok_or(RasterInputError::LimitExceeded)?;
    let line_count = width.max(height).checked_mul(channels).ok_or(RasterInputError::LimitExceeded)?;
    let scratch = count
        .checked_add(line_count.checked_mul(2).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(1024))
        .ok_or(RasterInputError::LimitExceeded)?;
    let raw_count = checked_region_byte_len(output, source.format())?;
    let output_count =
        (output.width() as usize).checked_mul(output.height() as usize).and_then(|n| n.checked_mul(channels)).ok_or(RasterInputError::LimitExceeded)?;
    let tile_bytes = (256 * 256 * source.format().bytes_per_pixel()) as u64;
    let mut new_tiles = 0_u64;
    for (index, coordinate) in output.tiles().enumerate() {
        if index.is_multiple_of(256) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        if source.tile(coordinate).is_none() {
            new_tiles = new_tiles.checked_add(tile_bytes + 64).ok_or(RasterInputError::LimitExceeded)?;
        }
    }
    let peak = source
        .resident_payload_bytes()?
        .checked_mul(2)
        .and_then(|n| n.checked_add(new_tiles))
        .and_then(|n| n.checked_add((source.tile_count() as u64).saturating_mul(128)))
        .and_then(|n| n.checked_add(scratch as u64))
        .and_then(|n| n.checked_add(raw_count as u64))
        .and_then(|n| n.checked_add((output_count as u64) * 8))
        .and_then(|n| n.checked_add((256 * 256 * source.format().bytes_per_pixel()) as u64 * 4))
        .ok_or(RasterInputError::LimitExceeded)?;
    let work_per_sample = 64 + axis_work(params.std_dev_x, filter) + axis_work(params.std_dev_y, filter);
    let work = (count as u64).checked_mul(work_per_sample).ok_or(RasterInputError::LimitExceeded)?;
    Ok(Some(FinitePlan {
        output,
        width,
        height,
        channels,
        count,
        line_count,
        output_count,
        requirements: NeighborhoodRequirements { peak_resident_bytes: peak, work_units: work },
    }))
}

/// Finite, full-line Gaussian in caller-supplied linear working samples.
/// Qualified numerical basis: official GIMP 3.2.6/GEGL 0.4.72, Clamp,
/// clip=true, sigma 0..=15 (explicit IIR requires nonzero sigma >=0.5).
/// Larger sigmas and other abyss policies are explicit
/// unsupported until their independent numerical differences are resolved.
/// Memory/work are checked before any allocation; output is staged privately.
pub fn try_gaussian_finite_clamp(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: GaussianParams,
    filter: GaussianFilter,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let Some(plan) = plan(source, output, input, params, filter, interrupt)? else {
        return Ok(source.clone());
    };
    if plan.requirements.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || plan.requirements.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let FinitePlan { output, width, height, channels, count, line_count, output_count, .. } = plan;
    let mut data = allocate_interrupt(count, interrupt)?;
    let mut line = allocate_interrupt(line_count, interrupt)?;
    let mut copied = allocate_interrupt(line_count, interrupt)?;
    let mut units = 0;
    for (index, pixel) in data.chunks_exact_mut(channels).enumerate() {
        poll(&mut units, channels, interrupt)?;
        read_premultiplied(
            source,
            input,
            GaussianAbyss::Clamp,
            i64::from(input.x0) + (index % width) as i64,
            i64::from(input.y0) + (index / width) as i64,
            pixel,
        )?;
    }
    if params.std_dev_x != 0. {
        for row in data.chunks_exact_mut(width * channels) {
            filter_line(row, &mut copied, channels, params.std_dev_x, filter, &mut units, interrupt)?;
        }
    }
    if params.std_dev_y != 0. {
        for x in 0..width {
            let values = line.get_mut(..height * channels).ok_or(RasterInputError::LengthMismatch)?;
            for (y, pixel) in values.chunks_exact_mut(channels).enumerate() {
                pixel.copy_from_slice(data.get((y * width + x) * channels..(y * width + x + 1) * channels).ok_or(RasterInputError::LengthMismatch)?);
            }
            filter_line(values, &mut copied, channels, params.std_dev_y, filter, &mut units, interrupt)?;
            for (y, pixel) in values.chunks_exact(channels).enumerate() {
                data.get_mut((y * width + x) * channels..(y * width + x + 1) * channels).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(pixel);
            }
        }
    }
    let mut pixels = allocate_interrupt(output_count, interrupt)?;
    for (index, pixel) in pixels.chunks_exact_mut(channels).enumerate() {
        poll(&mut units, channels, interrupt)?;
        let x = (output.x0 - input.x0) as usize + index % output.width() as usize;
        let y = (output.y0 - input.y0) as usize + index / output.width() as usize;
        let values = data.get((y * width + x) * channels..(y * width + x + 1) * channels).ok_or(RasterInputError::LengthMismatch)?;
        let alpha = if source.format().alpha { values.last().copied().ok_or(RasterInputError::LengthMismatch)? } else { 1. };
        for (c, (value, sample)) in pixel.iter_mut().zip(values).enumerate() {
            *value = if source.format().alpha && c + 1 < channels { if alpha > 0. { *sample / alpha } else { 0. } } else { *sample };
        }
    }
    let mut result = source.clone();
    result.try_write_region_f64_with(output, &pixels, interrupt)?;
    Ok(result)
}

fn axis_work(sigma: f64, filter: GaussianFilter) -> u64 {
    if sigma == 0. {
        0
    } else if filter == GaussianFilter::Iir || filter == GaussianFilter::Auto && sigma >= 1. {
        32
    } else {
        ((3.25 * sigma).round().max(1.) as u64 * 2 + 1) * 4
    }
}

fn allocate_interrupt(count: usize, interrupt: &Interrupt<'_>) -> Result<Vec<f64>, RasterInputError> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| RasterInputError::LimitExceeded)?;
    checked_allocation_capacity::<f64>(values.capacity(), count)?;
    while values.len() < count {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        values.resize(values.len().saturating_add(4096).min(count), 0.);
    }
    Ok(values)
}

fn filter_line(
    values: &mut [f64],
    copied: &mut [f64],
    channels: usize,
    sigma: f64,
    filter: GaussianFilter,
    units: &mut usize,
    interrupt: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    if sigma == 0. {
        return Ok(());
    }
    if filter == GaussianFilter::Iir || filter == GaussianFilter::Auto && sigma >= 1. {
        return finite_line(values, channels, sigma, units, interrupt);
    }
    // Native impulse support across independent sigma/length bases establishes
    // round(3.25*sigma), min one. Coefficients remain the public normal density,
    // never fitted response coefficients or copied implementation.
    let radius = (3.25 * sigma).round().max(1.) as i64;
    let taps = usize::try_from(radius * 2 + 1).map_err(|_| RasterInputError::LimitExceeded)?;
    let mut weights = [0.; 99];
    let weights = weights.get_mut(..taps).ok_or(RasterInputError::LimitExceeded)?;
    for (i, w) in weights.iter_mut().enumerate() {
        *w = (-0.5 * ((i as f64 - radius as f64) / sigma).powi(2)).exp();
    }
    let total: f64 = weights.iter().sum();
    let original = copied.get_mut(..values.len()).ok_or(RasterInputError::LengthMismatch)?;
    original.copy_from_slice(values);
    let length = values.len() / channels;
    for (x, pixel) in values.chunks_exact_mut(channels).enumerate() {
        poll(units, channels * taps, interrupt)?;
        pixel.fill(0.);
        for (tap, weight) in weights.iter().enumerate() {
            let xx = (x as i64 + tap as i64 - radius).clamp(0, length as i64 - 1) as usize;
            let sample = original.get(xx * channels..(xx + 1) * channels).ok_or(RasterInputError::LengthMismatch)?;
            for (value, input) in pixel.iter_mut().zip(sample) {
                *value += input * weight / total;
            }
        }
    }
    Ok(())
}

fn finite_line(values: &mut [f64], channels: usize, sigma: f64, units: &mut usize, interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
    let [gain, a1, a2, a3] = young_coefficients(sigma)?;
    let denominator = (1. + a1 - a2 + a3) * (1. - a1 - a2 - a3) * (1. + a2 + (a1 - a3) * a3);
    if !denominator.is_finite() || denominator == 0. {
        return Err(RasterInputError::InvalidSample);
    }
    let matrix = [
        [1. - a2 - a3 * a3 - a3 * a1, (a3 + a1) * (a2 + a3 * a1), a3 * (a1 + a3 * a2)],
        [a1 + a3 * a2, -(a2 - 1.) * (a2 + a3 * a1), -a3 * (a3 * a1 + a3 * a3 + a2 - 1.)],
        [a3 * a1 + a2 + a1 * a1 - a2 * a2, a1 * a2 + a3 * a2 * a2 - a1 * a3 * a3 - a3 * a3 * a3 - a3 * a2 + a3, a3 * (a1 + a3 * a2)],
    ];
    let mut initial = [0.; 5];
    let mut terminal = [0.; 5];
    initial.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(values.get(..channels).ok_or(RasterInputError::LengthMismatch)?);
    terminal
        .get_mut(..channels)
        .ok_or(RasterInputError::LengthMismatch)?
        .copy_from_slice(values.get(values.len().checked_sub(channels).ok_or(RasterInputError::LengthMismatch)?..).ok_or(RasterInputError::LengthMismatch)?);
    let coefficients = [a1, a2, a3];
    let mut history = [initial; 3];
    for pixel in values.chunks_exact_mut(channels) {
        poll(units, channels * 8, interrupt)?;
        recurrence_step(pixel, &history, coefficients, gain)?;
        history.rotate_right(1);
        history.first_mut().ok_or(RasterInputError::LengthMismatch)?.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(pixel);
    }
    let mut backward = [[0.; 5]; 3];
    for (row, dest) in matrix.iter().zip(&mut backward) {
        for (c, value) in dest.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?.iter_mut().enumerate() {
            let right = terminal.get(c).copied().ok_or(RasterInputError::LengthMismatch)?;
            let mut sum = 0.;
            for (k, h) in row.iter().zip(&history) {
                sum += k * (h.get(c).copied().ok_or(RasterInputError::LengthMismatch)? - right);
            }
            *value = right + gain / denominator * sum;
        }
    }
    for pixel in values.chunks_exact_mut(channels).rev() {
        poll(units, channels * 8, interrupt)?;
        recurrence_step(pixel, &backward, coefficients, gain)?;
        backward.rotate_right(1);
        backward.first_mut().ok_or(RasterInputError::LengthMismatch)?.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(pixel);
    }
    Ok(())
}

fn recurrence_step(pixel: &mut [f64], history: &[[f64; 5]; 3], coefficients: [f64; 3], gain: f64) -> Result<(), RasterInputError> {
    for (c, value) in pixel.iter_mut().enumerate() {
        let mut sum = 0.;
        for (h, k) in history.iter().zip(coefficients) {
            sum += h.get(c).copied().ok_or(RasterInputError::LengthMismatch)? * k;
        }
        *value = gain * *value + sum;
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "finite_allocation_requirements_tests.rs"]
mod allocation_requirements_tests;
