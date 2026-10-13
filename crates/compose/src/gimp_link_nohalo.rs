//! Bounded ICC-aware modern NoHalo linked affine sampling.
//! Primary mathematical source: Racette thesis NoHalo face split equations4.6-4.8
//! followed by locally bounded bicubic Hermite definition12.4.1. Modern saved
//! native observations qualify only the explicit non-downsampling geometry slice.
//! <https://www.collectionscanada.gc.ca/obj/thesescanada/vol2/002/MR87720.PDF>
//! No foreign implementation source inspected or copied.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{PixelFormat, Rect, xcf_items::TransformState};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
fn refused() -> CompositionError {
    CompositionError::Unsupported("unqualified NoHalo linked transform")
}
fn minmod(a: f64, b: f64) -> f64 {
    if a > 0.0 && b > 0.0 {
        a.min(b)
    } else if a < 0.0 && b < 0.0 {
        a.max(b)
    } else {
        0.0
    }
}
// Clean-room thesis equations4.6-4.8 and LBB definition12.4.1.
// Two-density face split followed by bounded bicubic Hermite, not level1 bilinear.
fn reconstruct(sample: impl Fn(i64, i64) -> Result<[f64; 4]>, x: f64, y: f64) -> Result<[f64; 4]> {
    let bx = x.floor() as i64 - 2;
    let by = y.floor() as i64 - 2;
    let mut original = [[[0.0; 4]; 6]; 6];
    for (j, row) in original.iter_mut().enumerate() {
        for (i, pixel) in row.iter_mut().enumerate() {
            *pixel = sample(bx + i as i64, by + j as i64)?;
            if pixel.iter().any(|v| !v.is_finite()) {
                return Err(refused());
            }
        }
    }
    let read = |i: i64, j: i64, channel: usize| -> Result<f64> {
        let ix = usize::try_from(i - bx).map_err(|_| refused())?;
        let iy = usize::try_from(j - by).map_err(|_| refused())?;
        original.get(iy).and_then(|r| r.get(ix)).and_then(|p| p.get(channel)).copied().ok_or_else(refused)
    };
    let horizontal = |i, j, c| -> Result<f64> { Ok(minmod(read(i + 1, j, c)? - read(i, j, c)?, read(i, j, c)? - read(i - 1, j, c)?)) };
    let vertical = |i, j, c| -> Result<f64> { Ok(minmod(read(i, j + 1, c)? - read(i, j, c)?, read(i, j, c)? - read(i, j - 1, c)?)) };
    let ix = (2.0 * x).floor() as i64;
    let iy = (2.0 * y).floor() as i64;
    let mut half = [[[0.0; 4]; 4]; 4];
    for (j, row) in half.iter_mut().enumerate() {
        for (i, pixel) in row.iter_mut().enumerate() {
            let hx = ix + i as i64 - 1;
            let hy = iy + j as i64 - 1;
            let (a, b) = (hx.div_euclid(2), hy.div_euclid(2));
            for (channel, value) in pixel.iter_mut().enumerate() {
                *value = match (hx.rem_euclid(2), hy.rem_euclid(2)) {
                    (0, 0) => read(a, b, channel)?,
                    (1, 0) => (read(a, b, channel)? + read(a + 1, b, channel)?) / 2.0 + (horizontal(a, b, channel)? - horizontal(a + 1, b, channel)?) / 4.0,
                    (0, 1) => (read(a, b, channel)? + read(a, b + 1, channel)?) / 2.0 + (vertical(a, b, channel)? - vertical(a, b + 1, channel)?) / 4.0,
                    (1, 1) => {
                        (read(a, b, channel)? + read(a + 1, b, channel)? + read(a, b + 1, channel)? + read(a + 1, b + 1, channel)?) / 4.0
                            + (horizontal(a, b, channel)? - horizontal(a + 1, b, channel)? + horizontal(a, b + 1, channel)?
                                - horizontal(a + 1, b + 1, channel)?)
                                / 8.0
                            + (vertical(a, b, channel)? + vertical(a + 1, b, channel)? - vertical(a, b + 1, channel)? - vertical(a + 1, b + 1, channel)?) / 8.0
                    }
                    _ => return Err(refused()),
                };
            }
        }
    }
    let read =
        |i: usize, j: usize, channel: usize| -> Result<f64> { half.get(j).and_then(|r| r.get(i)).and_then(|p| p.get(channel)).copied().ok_or_else(refused) };
    let basis = |t: f64| ([2.0 * t * t * t - 3.0 * t * t + 1.0, -2.0 * t * t * t + 3.0 * t * t], [t * t * t - 2.0 * t * t + t, t * t * t - t * t]);
    let (hx, gx) = basis(2.0 * x - ix as f64);
    let (hy, gy) = basis(2.0 * y - iy as f64);
    let mut output = [0.0; 4];
    for (channel, output) in output.iter_mut().enumerate() {
        for cy in 0..2 {
            for cx in 0..2 {
                let (i, j) = (cx + 1, cy + 1);
                let z = read(i, j, channel)?;
                let mut lower = z;
                let mut upper = z;
                for yy in j - 1..=j + 1 {
                    for xx in i - 1..=i + 1 {
                        let v = read(xx, yy, channel)?;
                        lower = lower.min(v);
                        upper = upper.max(v);
                    }
                }
                let bound = 3.0 * (z - lower).min(upper - z);
                if !bound.is_finite() || bound < 0.0 {
                    return Err(refused());
                }
                // Round the positive slope limit inward. The product 3*d can round
                // above its exact value; a saturated slope then violates the LBB
                // constraint and can make the cross interval empty. Two predecessor
                // values add a conservative margin for subsequent sum/product rounding.
                // This tightens the thesis bound; a genuinely empty interval still
                // refuses below. Zero remains exactly zero, including subnormals.
                let bound = if bound == 0.0 { 0.0 } else { bound.next_down().max(0.0).next_down().max(0.0) };
                let dx = ((read(i + 1, j, channel)? - read(i - 1, j, channel)?) / 2.0).clamp(-bound, bound);
                let dy = ((read(i, j + 1, channel)? - read(i, j - 1, channel)?) / 2.0).clamp(-bound, bound);
                let minimum = (3.0 * (dx + dy).abs() - 9.0 * (z - lower)).max(3.0 * (dx - dy).abs() - 9.0 * (upper - z));
                let maximum = (-3.0 * (dx + dy).abs() + 9.0 * (upper - z)).min(-3.0 * (dx - dy).abs() + 9.0 * (z - lower));
                // Hostile/nonfinite data never reaches f64::clamp's panic precondition.
                if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum || !bound.is_finite() || bound < 0.0 {
                    return Err(CompositionError::Unsupported("NoHalo cross-derivative interval not qualified"));
                }
                let cross = ((read(i + 1, j + 1, channel)? + read(i - 1, j - 1, channel)? - read(i + 1, j - 1, channel)? - read(i - 1, j + 1, channel)?) / 4.0)
                    .clamp(minimum, maximum);
                *output += hx.get(cx).ok_or_else(refused)? * hy.get(cy).ok_or_else(refused)? * z
                    + gx.get(cx).ok_or_else(refused)? * hy.get(cy).ok_or_else(refused)? * dx
                    + hx.get(cx).ok_or_else(refused)? * gy.get(cy).ok_or_else(refused)? * dy
                    + gx.get(cx).ok_or_else(refused)? * gy.get(cy).ok_or_else(refused)? * cross;
            }
        }
    }
    if output.iter().any(|v| !v.is_finite()) {
        return Err(refused());
    }
    Ok(output)
}

pub fn qualify_geometry(width: u32, height: u32, state: &TransformState, bounds: Rect) -> Result<()> {
    let [a, b, tx, c, d, ty, p, q, r] = state.matrix_f64_bits.map(f64::from_bits);
    if ![a, b, tx, c, d, ty, p, q, r].iter().all(|v| v.is_finite())
        || b != 0.0
        || c != 0.0
        || p != 0.0
        || q != 0.0
        || r != 1.0
        || !(1.0..=8.0).contains(&a)
        || !(1.0..=8.0).contains(&d)
        || u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).is_none_or(|n| n > 262_144)
    {
        return Err(refused());
    }
    crate::gimp_link_nearest::mapping_with_interpolation(width, height, state, bounds, 3).map(|_| ())
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
        || u64::from(width) * u64::from(height) > 4_000_000
        || u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(4)).and_then(|n| usize::try_from(n).ok()) != Some(bytes.len())
    {
        return Err(refused());
    }
    qualify_geometry(width, height, state, bounds)?;
    let (inverse, ox, oy) = crate::gimp_link_nearest::mapping_with_interpolation(width, height, state, bounds, 3)?;
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
            // No bounded subdivision stencil can touch native input beyond this halo.
            if sx < -3.0 || sy < -3.0 || sx >= f64::from(width) + 2.0 || sy >= f64::from(height) + 2.0 {
                continue;
            }
            let accumulated = reconstruct(
                |tx, ty| {
                    if tx < 0 || ty < 0 || tx >= i64::from(width) || ty >= i64::from(height) {
                        return Ok([0.0; 4]);
                    }
                    let at = usize::try_from(ty)
                        .ok()
                        .and_then(|y| y.checked_mul(width as usize))
                        .and_then(|n| usize::try_from(tx).ok().and_then(|x| n.checked_add(x)))
                        .and_then(|n| n.checked_mul(4))
                        .ok_or_else(refused)?;
                    let pixel = bytes.get(at..at.checked_add(4).ok_or_else(refused)?).ok_or_else(refused)?;
                    let alpha = f64::from(*pixel.get(3).ok_or_else(refused)?) / 255.0;
                    let mut associated = [0.0; 4];
                    for (channel, value) in associated.iter_mut().take(3).enumerate() {
                        let sample = usize::from(*pixel.get(channel).ok_or_else(refused)?);
                        *value = *tables.decoded.get(channel).and_then(|row| row.get(sample)).ok_or_else(refused)? * alpha.max(1.0 / 65536.0);
                    }
                    *associated.get_mut(3).ok_or_else(refused)? = alpha;
                    Ok(associated)
                },
                sx,
                sy,
            )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mathematical_affine_invariant_and_hostile_geometry() {
        for (x, y) in [(0.0, 0.0), (0.25, 0.75), (-0.7, 1.25), (1.4, -2.1)] {
            let actual = reconstruct(|i, j| Ok([2.0 * i as f64 + 3.0 * j as f64 + 1.0; 4]), x, y).unwrap();
            assert!(actual.iter().all(|v| (*v - (2.0 * x + 3.0 * y + 1.0)).abs() < 1e-12));
        }
        let mut state =
            TransformState { matrix_f64_bits: [1.0f64, 0.0, 0.25, 0.0, 1.0, 0.375, 0.0, 0.0, 1.0].map(f64::to_bits), pre_offset: [0, 0], interpolation: 3 };
        assert!(qualify_geometry(2, 2, &state, Rect::new(0, 0, 3, 3)).is_ok());
        state.matrix_f64_bits[0] = 0.5f64.to_bits();
        assert!(qualify_geometry(2, 2, &state, Rect::new(0, 0, 2, 3)).is_err());
        assert!(reconstruct(|_, _| Ok([f64::NAN; 4]), 0.5, 0.5).is_err());
    }
    #[test]
    fn bounded_hermite_preserves_impulse_extrema_and_transpose_symmetry() {
        let sample = |i, j| if i == 0 && j == 0 { 1.0 } else { 0.0 };
        for yi in -16..=16 {
            for xi in -16..=16 {
                let (x, y) = (f64::from(xi) / 8.0, f64::from(yi) / 8.0);
                let actual = reconstruct(|i, j| Ok([sample(i, j); 4]), x, y).unwrap();
                let transposed = reconstruct(|i, j| Ok([sample(j, i); 4]), y, x).unwrap();
                for (a, b) in actual.iter().zip(transposed) {
                    assert!(*a >= -1e-12 && *a <= 1.0 + 1e-12);
                    assert!((*a - b).abs() < 1e-12);
                }
            }
        }
    }
    #[test]
    fn clipped_slope_rounding_retains_a_nonempty_cross_interval() {
        // Own native input converted through the analytical nonlinear sRGB equation.
        // Captured exact binary64 stencil isolates the LBB arithmetic from profile conversion.
        let stencil: [[u64; 6]; 6] = [
            [0u64, 0u64, 0u64, 0u64, 0u64, 0u64],
            [0u64, 0u64, 0u64, 0u64, 0u64, 0u64],
            [0u64, 0u64, 0u64, 0u64, 0u64, 0u64],
            [0u64, 0u64, 0u64, 4527602940340807593u64, 4595351557438966454u64, 4597398491945863827u64],
            [0u64, 0u64, 0u64, 4598816904301465074u64, 4599831002624350891u64, 4590180129416474015u64],
            [0u64, 0u64, 0u64, 4591560718056835836u64, 4595351557438966454u64, 4597398491945863827u64],
        ];
        let actual = reconstruct(
            |i, j| {
                let value = stencil.get((j + 3) as usize).and_then(|row| row.get((i + 3) as usize)).copied().unwrap();
                Ok([f64::from_bits(value); 4])
            },
            -0.33333333333333326,
            -0.75,
        );
        assert!(actual.is_ok(), "{actual:?}");
    }
}
