//! Controlled ordinary presentation warp. Exact legacy f32 source association,
//! f64 sampler accumulation and f32/native output quantization, not raw nearest.
use super::{Homography, Interp, bicubic};
use crate::{
    resample::{Resample, try_resize_surface_with_interrupt},
    wide_geometry::GeometryLimits,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn samples(rect: Rect, n: usize) -> Result<usize, RasterInputError> {
    usize::try_from(rect.width())
        .ok()
        .and_then(|w| usize::try_from(rect.height()).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|v| v.checked_mul(n))
        .ok_or(RasterInputError::LimitExceeded)
}
fn vector<T: Clone>(length: usize, value: T, control: &Interrupt<'_>) -> Result<Vec<T>, RasterInputError> {
    if length.checked_mul(std::mem::size_of::<T>()).is_none_or(|v| v as u64 > MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    if values.capacity() != length {
        return Err(RasterInputError::AllocationFailed);
    }
    while values.len() < length {
        poll(control)?;
        values.resize(values.len().saturating_add(8192).min(length), value.clone());
    }
    Ok(values)
}
fn checked_destination(rect: Rect, h: &Homography) -> Result<(Rect, Homography), RasterInputError> {
    if h.0.iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidBounds);
    }
    let inv = h.inverse().filter(|v| v.0.iter().all(|v| v.is_finite())).ok_or(RasterInputError::InvalidBounds)?;
    let corners = [(rect.x0, rect.y0), (rect.x1, rect.y0), (rect.x1, rect.y1), (rect.x0, rect.y1)];
    let mut lo = [f64::MAX; 2];
    let mut hi = [f64::MIN; 2];
    let mut sign = 0.0;
    for (x, y) in corners {
        if i64::from(x).abs() > 1_000_000 || i64::from(y).abs() > 1_000_000 {
            return Err(RasterInputError::InvalidBounds);
        }
        let d = h.0[6] * f64::from(x) + h.0[7] * f64::from(y) + h.0[8];
        if !d.is_finite() || d.abs() < 1e-12 || (sign != 0.0 && d.signum() != sign) {
            return Err(RasterInputError::InvalidBounds);
        }
        sign = d.signum();
        let (u, v) = h.apply(f64::from(x), f64::from(y));
        if !u.is_finite() || !v.is_finite() || u.abs() > 1_000_000.0 || v.abs() > 1_000_000.0 {
            return Err(RasterInputError::InvalidBounds);
        }
        lo[0] = lo[0].min(u);
        lo[1] = lo[1].min(v);
        hi[0] = hi[0].max(u);
        hi[1] = hi[1].max(v);
    }
    let dst = Rect::new(lo[0].floor() as i32 - 1, lo[1].floor() as i32 - 1, hi[0].ceil() as i32 + 1, hi[1].ceil() as i32 + 1);
    sign = 0.0;
    for (x, y) in [(dst.x0, dst.y0), (dst.x1, dst.y0), (dst.x1, dst.y1), (dst.x0, dst.y1)] {
        let d = inv.0[6] * f64::from(x) + inv.0[7] * f64::from(y) + inv.0[8];
        if !d.is_finite() || d.abs() < 1e-12 || (sign != 0.0 && d.signum() != sign) {
            return Err(RasterInputError::InvalidBounds);
        }
        sign = d.signum();
    }
    Ok((dst, inv))
}
// Keep source, destination, sampler, matrix, limits and original control explicit
// in this preflight; regrouping them obscures the bounded-work admission contract.
#[allow(clippy::too_many_arguments)]
fn charge(
    src: &Surface,
    bounds: Rect,
    dst: Rect,
    format: photocraft_color::PixelFormat,
    interp: Interp,
    h: &Homography,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<u64, RasterInputError> {
    let resident_cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    checked_region_byte_len(bounds, src.format())?;
    checked_region_byte_len(dst, format)?;
    let inv = h.inverse().ok_or(RasterInputError::InvalidBounds)?;
    let mut retained = src.default_bytes().len() as u64;
    let mut source_visits = 0u64;
    for (_, tile) in src.tiles() {
        poll(control)?;
        retained = retained.checked_add(tile.bytes().len() as u64 + 512).ok_or(RasterInputError::LimitExceeded)?;
        source_visits =
            source_visits.checked_add((TILE_SIZE as u64) * (TILE_SIZE as u64) * (format.channels() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    }
    let span = |a: i32, b: i32| {
        u64::try_from(i64::from((b - 1).div_euclid(TILE_SIZE)) - i64::from(a.div_euclid(TILE_SIZE)) + 1).map_err(|_| RasterInputError::InvalidBounds)
    };
    let tiles = span(dst.x0, dst.x1)?.checked_mul(span(dst.y0, dst.y1)?).ok_or(RasterInputError::LimitExceeded)?;
    let output = tiles.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * (format.bytes_per_pixel() as u64) + 512).ok_or(RasterInputError::LimitExceeded)?;
    let taps = match interp {
        Interp::Nearest => 1,
        Interp::Bilinear => 4,
        Interp::Bicubic => 16,
    };
    let mut work = (samples(dst, format.channels())? as u64)
        .checked_mul(taps)
        .and_then(|v| v.checked_add(source_visits.checked_mul(8)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut scratch = 0u64;
    for tile in dst.tiles() {
        poll(control)?;
        let t = tile.rect().intersect(&dst);
        let points = [(t.x0, t.y0), (t.x1, t.y0), (t.x1, t.y1), (t.x0, t.y1)].map(|(x, y)| inv.apply(f64::from(x), f64::from(y)));
        if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite() || x.abs() > f64::from(i32::MAX - 4) || y.abs() > f64::from(i32::MAX - 4)) {
            return Err(RasterInputError::InvalidBounds);
        }
        let foot = Rect::new(
            (points.iter().map(|p| p.0).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3),
            (points.iter().map(|p| p.1).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3),
            (points.iter().map(|p| p.0).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3),
            (points.iter().map(|p| p.1).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3),
        )
        .intersect(&bounds);
        let read = samples(foot, format.channels())? as u64;
        work = work
            .checked_add(read.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?)
            .filter(|v| *v <= limits.work_units)
            .ok_or(RasterInputError::LimitExceeded)?;
        let peak = read
            .checked_mul(12)
            .and_then(|v| v.checked_add((samples(t, format.channels()).ok()? as u64).checked_mul(8)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        scratch = scratch.max(peak);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let group = (rayon::current_num_threads().max(1).saturating_mul(2) as u64).min(tiles);
    #[cfg(target_arch = "wasm32")]
    let group = 1u64.min(tiles);
    // Original, alpha-prepared and reduced owners can coexist. Staging tile
    // windows/output/native encoding and fixed group metadata are additional.
    let peak = retained
        .checked_mul(3)
        .and_then(|v| v.checked_add(output.checked_mul(3)?))
        .and_then(|v| v.checked_add(scratch.checked_mul(group)?))
        .and_then(|v| v.checked_add(tiles.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    if peak > resident_cap {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(work)
}
/// Same supported ordinary sampler semantics as try_warp_surface, with a real token.
pub fn try_warp_surface_legacy_with_limits(
    source: &Surface,
    bounds: Rect,
    mapping: &Homography,
    interp: Interp,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    poll(control)?;
    if !matches!(source.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
        || !matches!(source.format().mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Cmyk | ColorMode::Lab)
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    source.try_validate_tile_origins_with(control)?;
    let mut fmt = source.format();
    fmt.alpha = true;
    if mapping.0.iter().any(|v| !v.is_finite()) || mapping.inverse().is_none() {
        return Err(RasterInputError::InvalidBounds);
    }
    if bounds.is_empty() {
        if limits.resident_bytes < fmt.bytes_per_pixel() as u64 {
            return Err(RasterInputError::LimitExceeded);
        }
        return Surface::try_with_default_bytes(fmt, &vector(fmt.bytes_per_pixel(), 0u8, control)?);
    }
    let (initial_dst, _) = checked_destination(bounds, mapping)?;
    let initial_work = charge(source, bounds, initial_dst, fmt, interp, mapping, limits, control)?;
    let prepared = source.try_convert_with(fmt, control)?;
    let mut src = &prepared;
    let mut h = *mapping;
    let mut src_rect = bounds;
    let reduced;
    let (cx, cy) = ((f64::from(bounds.x0) + f64::from(bounds.x1)) / 2.0, (f64::from(bounds.y0) + f64::from(bounds.y1)) / 2.0);
    let scale = h.local_scale(cx, cy);
    // Match the ordinary sampler: Bilinear samples directly; only Bicubic
    // prefilters, at the actual local scale with its existing 1/64 floor.
    if interp == Interp::Bicubic && scale < 0.999 && scale > 0.0 {
        let f = scale.max(1.0 / 64.0);
        reduced = try_resize_surface_with_interrupt(src, f, f, Resample::Bicubic, control)?;
        src = &reduced;
        h = h.mul(&Homography([1.0 / f, 0.0, 0.0, 0.0, 1.0 / f, 0.0, 0.0, 0.0, 1.0]));
        src_rect = crate::resample::scaled_rect(bounds, f, f);
    }
    let (dst, inv) = checked_destination(src_rect, &h)?;
    let remaining = GeometryLimits {
        resident_bytes: limits.resident_bytes,
        work_units: limits.work_units.checked_sub(initial_work).ok_or(RasterInputError::LimitExceeded)?,
    };
    charge(src, src_rect, dst, fmt, interp, &h, remaining, control)?;
    let n = fmt.channels();
    let a = n.checked_sub(1).ok_or(RasterInputError::UnsupportedConversion)?;
    if n > 8 {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let mut out = Surface::try_with_default_bytes(fmt, &vector(fmt.bytes_per_pixel(), 0u8, control)?)?;
    let src_ref = src;
    let work = |t: &Rect| -> Result<Option<(Rect, Vec<f32>)>, RasterInputError> {
        poll(control)?;
        // Source footprint of this tile (inverse-mapped corners), padded for the filter.
        let tc = [(t.x0, t.y0), (t.x1, t.y0), (t.x1, t.y1), (t.x0, t.y1)].map(|(x, y)| inv.apply(x as f64, y as f64));
        let fx0 = (tc.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
        let fy0 = (tc.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
        let fx1 = (tc.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
        let fy1 = (tc.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
        let foot = Rect::new(fx0, fy0, fx1, fy1).intersect(&src_rect);
        if foot.is_empty() || !src_ref.has_tiles_in(foot) {
            return Ok(None);
        }
        // Premultiplied source window.
        let raw = src_ref.try_read_interleaved_with(foot, control)?;
        let count = samples(foot, n)?;
        let mut px = vector(count, 0.0f32, control)?;
        for (index, value) in px.iter_mut().enumerate() {
            if index % 1024 == 0 {
                poll(control)?;
            }
            *value = photocraft_color::read_sample(&raw, fmt.sample, index);
            if !value.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
        }
        drop(raw);
        for (index, p) in px.chunks_exact_mut(n).enumerate() {
            if index % 1024 == 0 {
                poll(control)?;
            }
            let al = p[a];
            for v in &mut p[..a] {
                *v *= al;
            }
        }
        let fw = foot.width() as usize;
        let at = |x: i32, y: i32, c: usize| -> Result<f64, RasterInputError> {
            if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                Ok(0.0)
            } else {
                px.get(((y - foot.y0) as usize * fw + (x - foot.x0) as usize) * n + c).copied().map(f64::from).ok_or(RasterInputError::LengthMismatch)
            }
        };
        let w = t.width() as usize;
        let mut outp = vector(samples(*t, n)?, 0.0f32, control)?;
        let mut any = false;
        let mut acc = [0.0f64; 8];
        for y in t.y0..t.y1 {
            poll(control)?;
            for x in t.x0..t.x1 {
                poll(control)?;
                let (u, v) = inv.apply(x as f64 + 0.5, y as f64 + 0.5);
                let (u, v) = (u - 0.5, v - 0.5);
                if !u.is_finite() || !v.is_finite() {
                    return Err(RasterInputError::InvalidBounds);
                }
                if u < src_rect.x0 as f64 - 1.0 || v < src_rect.y0 as f64 - 1.0 || u > src_rect.x1 as f64 || v > src_rect.y1 as f64 {
                    continue;
                }
                acc.get_mut(..n).ok_or(RasterInputError::UnsupportedConversion)?.fill(0.0);
                match interp {
                    Interp::Nearest => {
                        let (ix, iy) = ((u + 0.5).floor() as i32, (v + 0.5).floor() as i32);
                        for (c, s) in acc.iter_mut().enumerate().take(n) {
                            *s = at(ix, iy, c)?;
                        }
                    }
                    Interp::Bilinear => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (fx, fy) = (u - ix as f64, v - iy as f64);
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += at(ix + dx, iy + dy, c)? * wx * wy;
                                }
                            }
                        }
                    }
                    Interp::Bicubic => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (wx, wy) = (bicubic(u - ix as f64), bicubic(v - iy as f64));
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += at(ix - 1 + i as i32, iy - 1 + j as i32, c)? * k;
                                }
                            }
                        }
                    }
                }
                let al = acc.get(a).copied().ok_or(RasterInputError::LengthMismatch)?.clamp(0.0, 1.0);
                if acc.get(..n).ok_or(RasterInputError::LengthMismatch)?.iter().any(|v| !v.is_finite()) {
                    return Err(RasterInputError::InvalidSample);
                }
                if al <= 0.0 {
                    continue;
                }
                any = true;
                let o = ((y - t.y0) as usize * w + (x - t.x0) as usize) * n;
                for c in 0..a {
                    // Match the ordinary sampler: integer encoding clips on
                    // write, while finite float colours retain signed/HDR data.
                    let value = (acc.get(c).copied().ok_or(RasterInputError::LengthMismatch)? / al) as f32;
                    if !value.is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    *outp.get_mut(o + c).ok_or(RasterInputError::LengthMismatch)? = value;
                }
                *outp.get_mut(o + a).ok_or(RasterInputError::LengthMismatch)? = al as f32;
            }
        }
        Ok(any.then_some((*t, outp)))
    };
    // Preserve independent native tile evaluation while bounding each private group.
    #[cfg(not(target_arch = "wasm32"))]
    let group = rayon::current_num_threads().max(1).saturating_mul(2);
    #[cfg(target_arch = "wasm32")]
    let group = 1;
    let mut coordinates = dst.tiles();
    loop {
        poll(control)?;
        let mut tiles = Vec::new();
        tiles.try_reserve_exact(group).map_err(|_| RasterInputError::AllocationFailed)?;
        if tiles.capacity() != group {
            return Err(RasterInputError::AllocationFailed);
        }
        for _ in 0..group {
            if let Some(tile) = coordinates.next() {
                tiles.push(tile.rect().intersect(&dst));
            } else {
                break;
            }
        }
        if tiles.is_empty() {
            break;
        }
        let mut slots = vector(tiles.len(), None, control)?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            slots.par_iter_mut().zip(tiles.par_iter()).for_each(|(slot, tile)| *slot = Some(work(tile)));
        }
        #[cfg(target_arch = "wasm32")]
        for (slot, tile) in slots.iter_mut().zip(&tiles) {
            *slot = Some(work(tile));
        }
        for slot in slots {
            poll(control)?;
            if let Some((r, values)) = slot.ok_or(RasterInputError::LengthMismatch)?? {
                let mut raw = vector(checked_region_byte_len(r, fmt)?, 0u8, control)?;
                for (index, &value) in values.iter().enumerate() {
                    if index % 1024 == 0 {
                        poll(control)?;
                    }
                    photocraft_color::write_sample(&mut raw, fmt.sample, index, value);
                }
                out.try_write_interleaved_with(r, &raw, control)?;
            }
        }
    }
    out.try_prune_with(control)?;
    poll(control)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_float_warp_preserves_ordinary_signed_hdr_colour() {
        let bounds = Rect::new(2, 3, 6, 7);
        let mut source = Surface::new(photocraft_color::PixelFormat::RGBA32F);
        source.fill_rect(bounds, &[2.0, -0.25, 0.5, 0.75]);
        let before = source.clone();
        let mapping = Homography([1.0, 0.0, 7.0, 0.0, 1.0, -3.0, 0.0, 0.0, 1.0]);
        for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
            let expected = super::super::try_warp_surface(&source, bounds, &mapping, interp).unwrap();
            let actual = try_warp_surface_legacy_with_limits(&source, bounds, &mapping, interp, GeometryLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(actual.try_read_interleaved(expected.content_bounds()).unwrap(), expected.try_read_interleaved(expected.content_bounds()).unwrap());
            assert_eq!(actual.pixel(10, 2), vec![2.0, -0.25, 0.5, 0.75]);
        }
        assert_eq!(source, before);
    }
    #[test]
    fn controlled_ordinary_warp_matches_old_all_interpolations_and_depths() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = photocraft_color::PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut source = Surface::new(format);
            let bounds = Rect::new(-3, 2, 9, 10);
            let mut values = Vec::new();
            for y in bounds.y0..bounds.y1 {
                for x in bounds.x0..bounds.x1 {
                    values.extend([((x + 3) % 5) as f32 / 4.0, (y - 2) as f32 / 8.0, 0.75, if x % 3 == 0 { 0.0 } else { 0.8 }]);
                }
            }
            source.write_region(bounds, &values);
            let before = source.clone();
            for mapping in [
                Homography::IDENTITY,
                Homography([0.2, 0.0, 2.0, 0.0, 0.2, -1.0, 0.0, 0.0, 1.0]),
                Homography([0.93, -0.2, 1.0, 0.2, 0.93, -2.0, 0.0, 0.0, 1.0]),
            ] {
                for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                    let old = super::super::try_warp_surface(&source, bounds, &mapping, interp).unwrap();
                    let controlled =
                        try_warp_surface_legacy_with_limits(&source, bounds, &mapping, interp, GeometryLimits::default(), &Interrupt::NONE).unwrap();
                    assert_eq!(controlled, old, "{sample:?} {interp:?} {mapping:?}");
                }
            }
            assert_eq!(source, before);
        }
    }
    #[test]
    fn ordinary_nearest_retains_legacy_zero_alpha_color_clearing_not_raw_copy() {
        let mut source = Surface::new(photocraft_color::PixelFormat::RGBA8);
        let bounds = Rect::new(0, 0, 2, 1);
        source.try_write_interleaved(bounds, &[41, 83, 127, 0, 211, 173, 139, 255]).unwrap();
        let out =
            try_warp_surface_legacy_with_limits(&source, bounds, &Homography::IDENTITY, Interp::Nearest, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(out.try_read_interleaved(bounds).unwrap(), [0, 0, 0, 0, 211, 173, 139, 255]);
    }
    #[test]
    fn cancellation_and_lower_work_budget_refuse_without_input_mutation() {
        let mut source = Surface::new(photocraft_color::PixelFormat::RGBA8);
        let bounds = Rect::new(0, 0, 2, 1);
        source.try_write_interleaved(bounds, &[1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        let before = source.clone();
        let stop = || true;
        assert_eq!(
            try_warp_surface_legacy_with_limits(
                &source,
                bounds,
                &Homography::IDENTITY,
                Interp::Bicubic,
                GeometryLimits::default(),
                &Interrupt::cancel_only(&stop)
            ),
            Err(RasterInputError::Cancelled)
        );
        assert_eq!(
            try_warp_surface_legacy_with_limits(
                &source,
                bounds,
                &Homography::IDENTITY,
                Interp::Bicubic,
                GeometryLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 0 },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        );
        assert_eq!(source, before);
    }
}
