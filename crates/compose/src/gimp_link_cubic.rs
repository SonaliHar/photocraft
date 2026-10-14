//! Bounded ICC-aware balanced cubic linked projective sampling.
//! Reconstruction family: Mitchell/Netravali BC, B=1/2, C=1/4; native packet
//! observations qualify the selected family. No foreign implementation copied.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{PixelFormat, Rect, xcf_items::TransformState};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
#[path = "gimp_link_cubic_controlled.rs"]
mod controlled;
pub use controlled::{Limits, Requirements};
fn refused() -> CompositionError {
    CompositionError::Unsupported("unqualified cubic linked transform")
}
pub(crate) fn cubic_weight(value: f64) -> f64 {
    let x = value.abs();
    if x < 1.0 {
        ((6.0 * x - 10.5) * x * x + 5.0) / 6.0
    } else if x < 2.0 {
        (((-2.0 * x + 10.5) * x - 18.0) * x + 10.0) / 6.0
    } else {
        0.0
    }
}
pub fn qualify_geometry(width: u32, height: u32, state: &TransformState, bounds: Rect) -> Result<()> {
    crate::gimp_link_nearest::mapping_with_interpolation(width, height, state, bounds, 2).map(|_| ())
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
    transform_with_profile_and_limits(bytes, width, height, format, state, bounds, profile, Limits::default(), control)
}
/// Current-profile/geometry/native-output requirements; no new source authority.
#[allow(clippy::too_many_arguments)]
pub fn requirements(
    bytes: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
    state: &TransformState,
    bounds: Rect,
    profile: &photocraft_cms::Profile,
    control: &Interrupt<'_>,
) -> Result<Requirements> {
    controlled::requirements(bytes, width, height, format, profile, state, bounds, Limits::default(), control)
}
/// Already resolved profile variant. Cold parsing remains the legacy outer boundary.
#[allow(clippy::too_many_arguments)]
pub fn transform_with_profile_and_limits(
    bytes: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
    state: &TransformState,
    bounds: Rect,
    profile: &photocraft_cms::Profile,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    let admitted = controlled::requirements(bytes, width, height, format, profile, state, bounds, limits, control)?;
    controlled::admit(admitted, limits)?;
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    if format != PixelFormat::RGBA8
        || u64::from(width) * u64::from(height) > 4_000_000
        || u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(4)).and_then(|n| usize::try_from(n).ok()) != Some(bytes.len())
    {
        return Err(refused());
    }
    let (inverse, ox, oy) = crate::gimp_link_nearest::mapping_with_interpolation(width, height, state, bounds, 2)?;
    let tables = crate::gimp_link_linear::TransferTables::new(profile, control)?;
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    let mut surface = Surface::try_with_default_bytes(format, &[0; 4])?;
    for tile in bounds.tiles() {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let patch = tile.rect().intersect(&bounds);
        let patch_bytes = photocraft_raster::checked_region_byte_len(patch, format)?;
        let mut row = Vec::new();
        row.try_reserve_exact(patch_bytes).map_err(|_| RasterInputError::AllocationFailed)?;
        let envelope = (patch_bytes as u64).checked_mul(2).and_then(|n| n.checked_add(64)).ok_or_else(refused)?;
        if row.capacity() as u64 > envelope || envelope > admitted.scratch_bytes {
            return Err(RasterInputError::LimitExceeded.into());
        }
        while row.len() < patch_bytes {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
            let end = row.len().checked_add((patch_bytes - row.len()).min(64 << 10)).ok_or_else(refused)?;
            row.resize(end, 0);
        }
        for y in patch.y0..patch.y1 {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
            for (column, x) in (patch.x0..patch.x1).enumerate() {
                let (sx, sy) = inverse.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
                let (sx, sy) = (sx - ox - 0.5, sy - oy - 0.5);
                if !sx.is_finite() || !sy.is_finite() {
                    return Err(refused());
                }
                // The four-tap support cannot touch the bounded input beyond this halo.
                if sx < -2.0 || sy < -2.0 || sx >= f64::from(width) + 1.0 || sy >= f64::from(height) + 1.0 {
                    continue;
                }
                let (ix, iy) = (sx.floor() as i64, sy.floor() as i64);

                let mut accumulated = [0.0_f64; 4];
                for dy in -1..=2 {
                    for dx in -1..=2 {
                        let weight = cubic_weight(sx - (ix + dx) as f64) * cubic_weight(sy - (iy + dy) as f64);
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
                            // Babl's public association floor preserves hidden RGB;
                            // actual native alpha is independently accumulated below.
                            *accumulated.get_mut(index).ok_or_else(refused)? += value * alpha.max(1.0 / 65536.0) * weight;
                        }
                        *accumulated.get_mut(3).ok_or_else(refused)? += alpha * weight;
                    }
                }
                let alpha = *accumulated.get(3).ok_or_else(refused)?;
                let divisor = if alpha.abs() < 1.0 / 65536.0 { 1.0 / 65536.0 } else { alpha };
                let mut unassociated = [0.0; 3];
                for (index, value) in unassociated.iter_mut().enumerate() {
                    *value = accumulated.get(index).ok_or_else(refused)? / divisor;
                }
                let at = usize::try_from(y - patch.y0)
                    .ok()
                    .and_then(|n| n.checked_mul(patch.width() as usize))
                    .and_then(|n| n.checked_add(column))
                    .and_then(|n| n.checked_mul(4))
                    .ok_or_else(refused)?;
                for (index, value) in unassociated.iter().enumerate() {
                    *row.get_mut(at.checked_add(index).ok_or_else(refused)?).ok_or_else(refused)? = tables.encode(index, *value)?;
                }
                if !alpha.is_finite() {
                    return Err(refused());
                }
                *row.get_mut(at.checked_add(3).ok_or_else(refused)?).ok_or_else(refused)? = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8;
            }
        }
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        surface.try_write_interleaved_with(patch, &row, control)?;
    }
    surface.try_prune_with(control)?;
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::too_many_arguments)]
    fn legacy_row_reference(
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
            || u64::from(width) * u64::from(height) > 4_000_000
            || u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(4)).and_then(|n| usize::try_from(n).ok()) != Some(bytes.len())
        {
            return Err(refused());
        }
        let (inverse, ox, oy) = crate::gimp_link_nearest::mapping_with_interpolation(width, height, state, bounds, 2)?;
        let tables = crate::gimp_link_linear::TransferTables::new(profile, control)?;
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
                // The four-tap support cannot touch the bounded input beyond this halo.
                if sx < -2.0 || sy < -2.0 || sx >= f64::from(width) + 1.0 || sy >= f64::from(height) + 1.0 {
                    continue;
                }
                let (ix, iy) = (sx.floor() as i64, sy.floor() as i64);

                let mut accumulated = [0.0_f64; 4];
                for dy in -1..=2 {
                    for dx in -1..=2 {
                        let weight = cubic_weight(sx - (ix + dx) as f64) * cubic_weight(sy - (iy + dy) as f64);
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
                            // Babl's public association floor preserves hidden RGB;
                            // actual native alpha is independently accumulated below.
                            *accumulated.get_mut(index).ok_or_else(refused)? += value * alpha.max(1.0 / 65536.0) * weight;
                        }
                        *accumulated.get_mut(3).ok_or_else(refused)? += alpha * weight;
                    }
                }
                let alpha = *accumulated.get(3).ok_or_else(refused)?;
                let divisor = if alpha.abs() < 1.0 / 65536.0 { 1.0 / 65536.0 } else { alpha };
                let mut unassociated = [0.0; 3];
                for (index, value) in unassociated.iter_mut().enumerate() {
                    *value = accumulated.get(index).ok_or_else(refused)? / divisor;
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

    #[test]
    fn cubic_family_is_normalized_and_hostile_requests_are_atomic() {
        for offset in [0.0, 0.125, 0.5, 0.875] {
            let sum: f64 = (-1..=2).map(|i| cubic_weight(offset - f64::from(i))).sum();
            assert!((sum - 1.0).abs() < 1e-12);
        }
        let state =
            TransformState { matrix_f64_bits: [1.0f64, 0.0, 0.25, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0].map(f64::to_bits), pre_offset: [0, 0], interpolation: 2 };
        let profile = photocraft_cms::builtin::srgb().to_bytes();
        let bounds = Rect::new(0, 0, 3, 1);
        let opaque = [255, 0, 0, 255, 0, 0, 255, 255];
        assert!(transform(&opaque, 2, 1, PixelFormat::RGBA8, &state, bounds, &profile, &Interrupt::NONE).is_ok());
        assert!(transform(&opaque, 2, 1, PixelFormat::RGBA8, &state, bounds, &profile, &Interrupt::cancel_only(&|| true)).is_err());
        let mut mixed = opaque;
        mixed[3] = 0;
        assert!(transform(&mixed, 2, 1, PixelFormat::RGBA8, &state, bounds, &profile, &Interrupt::NONE).is_ok());
        assert!(transform(&opaque, 2, 1, PixelFormat::RGBA8, &state, Rect::new(0, 0, 2, 1), &profile, &Interrupt::NONE).is_err());
        let mut wrong = state.clone();
        wrong.interpolation = 3;
        assert!(transform(&opaque, 2, 1, PixelFormat::RGBA8, &wrong, bounds, &profile, &Interrupt::NONE).is_err());
    }
    // Unrun: compares actual prior row implementation across tile boundaries,
    // hidden RGB and partial alpha, retaining unchanged tone-curve quantization.
    #[test]
    fn tiled_cubic_matches_prior_rows_and_refuses_lower_budgets() {
        let profile = photocraft_cms::builtin::srgb();
        let mut bytes = Vec::new();
        for y in 0..3 {
            for x in 0..5 {
                bytes.extend_from_slice(&[(20 + x * 35) as u8, (30 + y * 65) as u8, (210 - x * 17) as u8, [0, 64, 128, 192, 255][x]]);
            }
        }
        let state =
            TransformState { matrix_f64_bits: [1.0f64, 0.0, 0.25, 0.0, 1.0, 0.375, 0.0, 0.0, 1.0].map(f64::to_bits), interpolation: 2, pre_offset: [254, -1] };
        let bounds = Rect::new(254, -1, 260, 3);
        let expected = legacy_row_reference(&bytes, 5, 3, PixelFormat::RGBA8, &state, bounds, &profile, &Interrupt::NONE).unwrap();
        let req = requirements(&bytes, 5, 3, PixelFormat::RGBA8, &state, bounds, &profile, &Interrupt::NONE).unwrap();
        let actual =
            transform_with_profile_and_limits(&bytes, 5, 3, PixelFormat::RGBA8, &state, bounds, &profile, Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
        assert!(
            transform_with_profile_and_limits(
                &bytes,
                5,
                3,
                PixelFormat::RGBA8,
                &state,
                bounds,
                &profile,
                Limits { resident_bytes: req.peak_resident_bytes - 1, work_units: req.work_units },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(
            transform_with_profile_and_limits(
                &bytes,
                5,
                3,
                PixelFormat::RGBA8,
                &state,
                bounds,
                &profile,
                Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units - 1 },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(
            transform_with_profile_and_limits(&bytes, 5, 3, PixelFormat::RGBA8, &state, bounds, &profile, Limits::default(), &Interrupt::cancel_only(&|| true))
                .is_err()
        );
    }
}
