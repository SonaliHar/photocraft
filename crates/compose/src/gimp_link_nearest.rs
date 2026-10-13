//! Native-byte nearest linked transforms. Samples never pass through a normalized color.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::transform::Homography;
use photocraft_doc::{PixelFormat, Rect, xcf_items::TransformState};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
fn refused() -> CompositionError {
    CompositionError::Unsupported("unqualified nearest linked transform")
}
/// Verify stored nearest geometry and drawable placement without allocating pixels.
pub fn qualify_geometry(width: u32, height: u32, state: &TransformState, bounds: Rect) -> Result<()> {
    mapping_with_interpolation(width, height, state, bounds, 0).map(|_| ())
}
pub(crate) fn mapping_with_interpolation(width: u32, height: u32, state: &TransformState, bounds: Rect, interpolation: i32) -> Result<(Homography, f64, f64)> {
    if state.interpolation != interpolation
        || width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || bounds.is_empty()
        || bounds.width() > 8192
        || bounds.height() > 8192
        || u64::from(bounds.width()) * u64::from(bounds.height()) > 4_000_000
    {
        return Err(refused());
    }
    let matrix = state.matrix_f64_bits.map(f64::from_bits);
    if matrix.iter().any(|v| !v.is_finite()) {
        return Err(refused());
    }
    let forward = Homography(matrix);
    let inverse = forward.inverse().ok_or_else(refused)?;
    if inverse.0.iter().any(|v| !v.is_finite()) {
        return Err(refused());
    }
    let [ox, oy] = state.pre_offset.map(f64::from);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut denominator_sign = None;
    for (x, y) in [(ox, oy), (ox + f64::from(width), oy), (ox, oy + f64::from(height)), (ox + f64::from(width), oy + f64::from(height))] {
        let denominator = matrix.get(6).ok_or_else(refused)? * x + matrix.get(7).ok_or_else(refused)? * y + matrix.get(8).ok_or_else(refused)?;
        if !denominator.is_finite() || denominator == 0.0 || denominator_sign.is_some_and(|sign| sign != denominator.is_sign_positive()) {
            return Err(refused());
        }
        denominator_sign = Some(denominator.is_sign_positive());
        let (x, y) = forward.apply(x, y);
        if !x.is_finite() || !y.is_finite() {
            return Err(refused());
        }
        min = [min.first().copied().ok_or_else(refused)?.min(x), min.get(1).copied().ok_or_else(refused)?.min(y)];
        max = [max.first().copied().ok_or_else(refused)?.max(x), max.get(1).copied().ok_or_else(refused)?.max(y)];
    }
    let natural_x0 = min.first().copied().ok_or_else(refused)?.floor();
    let natural_y0 = min.get(1).copied().ok_or_else(refused)?.floor();
    let natural_x1 = max.first().copied().ok_or_else(refused)?.ceil();
    let natural_y1 = max.get(1).copied().ok_or_else(refused)?.ceil();
    let coordinate_limit = f64::from(1u32 << 29);
    if [natural_x0, natural_y0, natural_x1, natural_y1, f64::from(bounds.x0), f64::from(bounds.y0), f64::from(bounds.x1), f64::from(bounds.y1)]
        .iter()
        .any(|value| value.abs() > coordinate_limit)
        || natural_x1 - natural_x0 != f64::from(bounds.width())
        || natural_y1 - natural_y0 != f64::from(bounds.height())
    {
        return Err(refused());
    }
    // GIMP's independently observed layer translation changes drawable placement,
    // retaining the original projective matrix/pre-offset and transformed extents.
    let dx = f64::from(bounds.x0) - natural_x0;
    let dy = f64::from(bounds.y0) - natural_y0;
    let placed_inverse = inverse.mul(&Homography([1.0, 0.0, -dx, 0.0, 1.0, -dy, 0.0, 0.0, 1.0]));
    if placed_inverse.0.iter().any(|value| !value.is_finite()) {
        return Err(refused());
    }
    Ok((placed_inverse, ox, oy))
}
/// Inverse projective mapping at pixel centers, with transparent out-of-source pixels.
/// The caller supplies independently qualified current-color source bytes. Only
/// interpolation zero is admitted; this is not an approximation of other kernels.
pub fn transform(bytes: &[u8], width: u32, height: u32, format: PixelFormat, state: &TransformState, bounds: Rect, control: &Interrupt<'_>) -> Result<Surface> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    let bpp = format.bytes_per_pixel();
    let source_size = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(refused)?;
    let output_pixels = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(refused)?;
    if state.interpolation != 0
        || !format.alpha
        || width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || source_size > 64 << 20
        || usize::try_from(source_size).ok() != Some(bytes.len())
        || bounds.is_empty()
        || bounds.width() > 8192
        || bounds.height() > 8192
        || output_pixels > 4_000_000
        || output_pixels.checked_mul(bpp as u64).is_none_or(|n| n > 64 << 20)
    {
        return Err(refused());
    }
    let (inverse, ox, oy) = mapping_with_interpolation(width, height, state, bounds, 0)?;
    let row_size = usize::try_from(bounds.width()).ok().and_then(|n| n.checked_mul(bpp)).ok_or_else(refused)?;
    let mut row = Vec::new();
    row.try_reserve_exact(row_size).map_err(|_| RasterInputError::AllocationFailed)?;
    row.resize(row_size, 0);
    let mut output = Surface::try_with_default_bytes(format, [0u8; 40].get(..bpp).ok_or_else(refused)?)?;
    for y in bounds.y0..bounds.y1 {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        row.fill(0);
        for (column, x) in (bounds.x0..bounds.x1).enumerate() {
            if column.is_multiple_of(4096) {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
            }
            let (sx, sy) = inverse.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
            let (sx, sy) = (sx - ox, sy - oy);
            if !sx.is_finite() || !sy.is_finite() {
                return Err(refused());
            }
            if sx < 0.0 || sy < 0.0 || sx >= f64::from(width) || sy >= f64::from(height) {
                continue;
            }
            // Bounds above prove these finite floors fit the bounded source descriptor.
            let start = (sy.floor() as usize)
                .checked_mul(width as usize)
                .and_then(|n| n.checked_add(sx.floor() as usize))
                .and_then(|n| n.checked_mul(bpp))
                .ok_or_else(refused)?;
            let end = start.checked_add(bpp).ok_or_else(refused)?;
            let dest = column.checked_mul(bpp).ok_or_else(refused)?;
            let dest_end = dest.checked_add(bpp).ok_or_else(refused)?;
            row.get_mut(dest..dest_end).ok_or_else(refused)?.copy_from_slice(bytes.get(start..end).ok_or_else(refused)?);
        }
        let end_y = y.checked_add(1).ok_or_else(refused)?;
        output.try_write_interleaved_with(Rect::new(bounds.x0, y, bounds.x1, end_y), &row, control)?;
    }
    output.prune();
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, SampleType};
    #[test]
    fn native_identity_preserves_f64_payloads_and_cancel_invalid_geometry_is_fallible() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let bytes =
            [(-0.0f64).to_bits(), 0x7ff8_0000_0000_0042u64, 0.5f64.to_bits(), 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let mut state = TransformState { matrix_f64_bits: Homography::IDENTITY.0.map(f64::to_bits), interpolation: 0, pre_offset: [3, -2] };
        let bounds = Rect::new(3, -2, 5, -1);
        let output = transform(&bytes, 2, 1, format, &state, bounds, &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(bounds).unwrap(), bytes);
        assert!(transform(&bytes, 2, 1, format, &state, bounds, &Interrupt::cancel_only(&|| true)).is_err());
        assert!(transform(&bytes, 2, 1, format, &state, Rect::new(3, -2, 6, -1), &Interrupt::NONE).is_err());
        state.matrix_f64_bits[8] = f64::NAN.to_bits();
        assert!(transform(&bytes, 2, 1, format, &state, bounds, &Interrupt::NONE).is_err());
        state.matrix_f64_bits = [0; 9];
        assert!(transform(&bytes, 2, 1, format, &state, bounds, &Interrupt::NONE).is_err());
    }
}
