//! ICC transfer-correct, premultiplied-alpha bilinear linked projective sampling.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64};
use photocraft_doc::{PixelFormat, Rect, xcf_items::TransformState};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
fn refused() -> CompositionError {
    CompositionError::Unsupported("unqualified linear linked transform")
}
pub(crate) struct TransferTables {
    pub(crate) decoded: [[f64; 256]; 3],
    thresholds: [[f64; 255]; 3],
}
impl TransferTables {
    pub(crate) fn new(profile: &photocraft_cms::Profile, control: &Interrupt<'_>) -> Result<Self> {
        let mut result = Self { decoded: [[0.0; 256]; 3], thresholds: [[0.0; 255]; 3] };
        for sample in 0..256 {
            if sample % 32 == 0 {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
            }
            let mut decoded = [0.0; 3];
            convert_encoding_f64(profile, Encoding::NonlinearProfile, Encoding::Linear, &[sample as f64 / 255.0; 3], &mut decoded)?;
            for (channel, value) in decoded.into_iter().enumerate() {
                *result.decoded.get_mut(channel).and_then(|row| row.get_mut(sample)).ok_or_else(refused)? = value;
            }
            if sample < 255 {
                let mut threshold = [0.0; 3];
                convert_encoding_f64(profile, Encoding::NonlinearProfile, Encoding::Linear, &[(sample as f64 + 0.5) / 255.0; 3], &mut threshold)?;
                for (channel, value) in threshold.into_iter().enumerate() {
                    *result.thresholds.get_mut(channel).and_then(|row| row.get_mut(sample)).ok_or_else(refused)? = value;
                }
            }
        }
        // Midpoint quantization is equivalent to inverse transfer only for strictly
        // monotonic curves. Reject hostile/nonseparable interpretations instead.
        if result
            .decoded
            .iter()
            .any(|row| row.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_none_or(|(a, b)| !a.is_finite() || !b.is_finite() || a >= b)))
            || result
                .thresholds
                .iter()
                .any(|row| row.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_none_or(|(a, b)| !a.is_finite() || !b.is_finite() || a >= b)))
        {
            return Err(refused());
        }
        Ok(result)
    }
    pub(crate) fn encode(&self, channel: usize, linear: f64) -> Result<u8> {
        if !linear.is_finite() {
            return Err(refused());
        }
        let thresholds = self.thresholds.get(channel).ok_or_else(refused)?;
        u8::try_from(thresholds.partition_point(|threshold| *threshold <= linear)).map_err(|_| refused())
    }
}
pub fn qualify_geometry(width: u32, height: u32, state: &TransformState, bounds: Rect) -> Result<()> {
    crate::gimp_link_nearest::mapping_with_interpolation(width, height, state, bounds, 1).map(|_| ())
}
/// Native RGBA8 consumer; other storage precisions need independent quantization proof.
#[allow(clippy::too_many_arguments)]
pub fn transform(
    bytes: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
    state: &TransformState,
    bounds: Rect,
    profile: &[u8],
    control: &Interrupt<'_>,
) -> Result<Surface> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    let profile = photocraft_cms::xcf_color::parse_bounded_profile(profile, photocraft_cms::ColorSpace::Rgb)?;
    transform_with_profile(bytes, width, height, format, state, bounds, &profile, control)
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn transform_with_profile(
    bytes: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
    state: &TransformState,
    bounds: Rect,
    profile: &photocraft_cms::Profile,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    if format != PixelFormat::RGBA8
        || u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(4)).and_then(|n| usize::try_from(n).ok()) != Some(bytes.len())
    {
        return Err(refused());
    }
    let (inverse, ox, oy) = crate::gimp_link_nearest::mapping_with_interpolation(width, height, state, bounds, 1)?;
    let tables = TransferTables::new(profile, control)?;
    let row_size = usize::try_from(bounds.width()).ok().and_then(|n| n.checked_mul(4)).ok_or_else(refused)?;
    let mut row = Vec::new();
    row.try_reserve_exact(row_size).map_err(|_| RasterInputError::AllocationFailed)?;
    row.resize(row_size, 0);
    let mut surface = Surface::try_with_default_bytes(format, &[0; 4])?;
    for y in bounds.y0..bounds.y1 {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        row.fill(0);
        for (column, x) in (bounds.x0..bounds.x1).enumerate() {
            if column.is_multiple_of(256) {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
            }
            let (sx, sy) = inverse.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
            let (sx, sy) = (sx - ox - 0.5, sy - oy - 0.5);
            if !sx.is_finite() || !sy.is_finite() {
                return Err(refused());
            }
            // No tap can touch the bounded input when this center lies farther than one pixel outside.
            if sx < -1.0 || sy < -1.0 || sx >= f64::from(width) || sy >= f64::from(height) {
                continue;
            }
            let (ix, iy) = (sx.floor() as i64, sy.floor() as i64);
            let (fx, fy) = (sx - sx.floor(), sy - sy.floor());
            let mut accumulated = [0.0_f64; 4];
            for (dx, dy, weight) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
                let (tx, ty) = (ix + dx, iy + dy);
                if tx < 0 || ty < 0 || tx >= i64::from(width) || ty >= i64::from(height) {
                    continue;
                }
                let at = usize::try_from(ty)
                    .ok()
                    .and_then(|n| n.checked_mul(width as usize))
                    .and_then(|n| usize::try_from(tx).ok().and_then(|tx| n.checked_add(tx)))
                    .and_then(|n| n.checked_mul(4))
                    .ok_or_else(refused)?;
                let pixel = bytes.get(at..at.checked_add(4).ok_or_else(refused)?).ok_or_else(refused)?;
                let alpha = f64::from(*pixel.get(3).ok_or_else(refused)?) / 255.0;
                for index in 0..3 {
                    let sample = usize::from(*pixel.get(index).ok_or_else(refused)?);
                    let value = tables.decoded.get(index).and_then(|row| row.get(sample)).ok_or_else(refused)?;
                        // Public Babl association preserves hidden color by using
                        // the 2^-16 floor; stored alpha itself remains unchanged.
                        *accumulated.get_mut(index).ok_or_else(refused)? += value * alpha.max(1.0 / 65536.0) * weight;
                }
                *accumulated.get_mut(3).ok_or_else(refused)? += alpha * weight;
            }
            let alpha = *accumulated.get(3).ok_or_else(refused)?;
            let mut unassociated = [0.0; 3];
            for (index, value) in unassociated.iter_mut().enumerate() {
                *value = accumulated.get(index).ok_or_else(refused)? / alpha.max(1.0 / 65536.0);
            }
            let at = column.checked_mul(4).ok_or_else(refused)?;
            for (index, value) in unassociated.iter().enumerate() {
                *row.get_mut(at.checked_add(index).ok_or_else(refused)?).ok_or_else(refused)? = tables.encode(index, *value)?;
            }
            if !alpha.is_finite() {
                return Err(refused());
            }
            *row.get_mut(at.checked_add(3).ok_or_else(refused)?).ok_or_else(refused)? = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8;
        }
        surface.try_write_interleaved_with(Rect::new(bounds.x0, y, bounds.x1, y.checked_add(1).ok_or_else(refused)?), &row, control)?;
    }
    surface.prune();
    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linear_sampling_uses_profile_transfer_premultiplied_alpha_and_transparent_border() {
        let profile = photocraft_cms::builtin::srgb().to_bytes();
        let state =
            TransformState { matrix_f64_bits: [1.0f64, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0].map(f64::to_bits), pre_offset: [0, 0], interpolation: 1 };
        let bounds = Rect::new(0, 0, 3, 1);
        let raw = [255, 0, 0, 128, 0, 0, 255, 255];
        let output = transform(&raw, 2, 1, PixelFormat::RGBA8, &state, bounds, &profile, &Interrupt::NONE).unwrap();
        let actual = output.try_read_interleaved(bounds).unwrap();
        let expected = [255, 0, 0, 64, 156, 0, 213, 192, 0, 0, 255, 128];
        assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1), "{actual:?}");
        assert!(transform(&raw, 2, 1, PixelFormat::RGBA8, &state, bounds, &profile, &Interrupt::cancel_only(&|| true)).is_err());
        assert!(transform(&raw, 2, 1, PixelFormat::RGBA8, &state, bounds, b"invalid ICC", &Interrupt::NONE).is_err());
        let mut unsupported = state.clone();
        unsupported.interpolation = 2;
        assert!(transform(&raw, 2, 1, PixelFormat::RGBA8, &unsupported, bounds, &profile, &Interrupt::NONE).is_err());
        assert!(qualify_geometry(2, 1, &state, Rect::new(0, 0, 2, 1)).is_err());
    }
}
