//! Controlled selection-only legacy Gray transforms. No wide numeric interpolation admission.
use crate::{
    transform::{Homography, Interp, bicubic},
    wide_geometry::GeometryLimits,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample, write_sample};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
const CELL: f64 = 4.0;
const MAX_CELLS: usize = 512;
const MAX_SS: f64 = 8.0;
fn poll(c: &Interrupt<'_>) -> Result<(), RasterInputError> {
    c.check().map_err(|_| RasterInputError::Cancelled)
}
fn vector<T: Clone>(n: usize, v: T, c: &Interrupt<'_>) -> Result<Vec<T>, RasterInputError> {
    if n.checked_mul(std::mem::size_of::<T>()).is_none_or(|b| b as u64 > MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| RasterInputError::AllocationFailed)?;
    if out.capacity() != n {
        return Err(RasterInputError::AllocationFailed);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(4096).min(n), v.clone());
    }
    Ok(out)
}
fn read(s: &Surface, r: Rect, c: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
    let n = usize::try_from(r.width())
        .ok()
        .and_then(|w| usize::try_from(r.height()).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|v| v.checked_mul(s.channels()))
        .ok_or(RasterInputError::LimitExceeded)?;
    let mut out = vector(n, 0f32, c)?;
    let stride = s.format().bytes_per_pixel();
    let mut raw = [0u8; 32];
    let mut i = 0;
    for y in r.y0..r.y1 {
        poll(c)?;
        for x in r.x0..r.x1 {
            if (x - r.x0) % 256 == 0 {
                poll(c)?;
            }
            let bytes = raw.get_mut(..stride).ok_or(RasterInputError::LengthMismatch)?;
            s.try_read_pixel_bytes(x, y, bytes)?;
            for channel in 0..s.channels() {
                let value = read_sample(bytes, s.format().sample, channel);
                if !value.is_finite() {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                *out.get_mut(i).ok_or(RasterInputError::LengthMismatch)? = value;
                i += 1;
            }
        }
    }
    Ok(out)
}
fn write(s: &mut Surface, r: Rect, values: &[f32], c: &Interrupt<'_>) -> Result<(), RasterInputError> {
    let bytes = checked_region_byte_len(r, s.format())?;
    let mut raw = vector(bytes, 0u8, c)?;
    if values.len() != bytes / s.format().sample.bytes() {
        return Err(RasterInputError::LengthMismatch);
    }
    for (i, &v) in values.iter().enumerate() {
        if i % 1024 == 0 {
            poll(c)?;
        }
        write_sample(&mut raw, s.format().sample, i, v);
    }
    s.try_write_interleaved_with(r, &raw, c)
}
fn tile_storage(r: Rect, fmt: PixelFormat) -> Result<u64, RasterInputError> {
    if r.is_empty() {
        return Ok(0);
    }
    let cols = i64::from(r.x1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(r.x0).div_euclid(i64::from(TILE_SIZE)) + 1;
    let rows = i64::from(r.y1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(r.y0).div_euclid(i64::from(TILE_SIZE)) + 1;
    u64::try_from(cols)
        .ok()
        .and_then(|x| u64::try_from(rows).ok().and_then(|y| x.checked_mul(y)))
        .and_then(|n| n.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * (fmt.bytes_per_pixel() as u64) + 4096))
        .ok_or(RasterInputError::LimitExceeded)
}
fn prepare(source: &Surface, limits: GeometryLimits, c: &Interrupt<'_>) -> Result<(Surface, f32, Rect), RasterInputError> {
    poll(c)?;
    let fmt = source.format();
    let retained = (source.tile_count() as u64)
        .checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * (fmt.bytes_per_pixel() as u64) + 4096)
        .ok_or(RasterInputError::LimitExceeded)?;
    if retained > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    if fmt.mode != ColorMode::Grayscale || fmt.alpha || !matches!(fmt.sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    source.try_validate_tile_origins_with(c)?;
    let r = source.try_content_bounds_with_interrupt(c)?;
    let bytes = checked_region_byte_len(r, PixelFormat::new(fmt.mode, fmt.sample, true))? as u64;
    let count = u64::from(r.width()).checked_mul(u64::from(r.height())).ok_or(RasterInputError::LimitExceeded)?;
    let staged = tile_storage(r, PixelFormat::new(fmt.mode, fmt.sample, true))?;
    if bytes.checked_mul(16).and_then(|n| n.checked_add(staged)).is_none_or(|b| b > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES))
        || count.checked_mul(64).is_none_or(|w| w > limits.work_units)
    {
        return Err(RasterInputError::LimitExceeded);
    }
    let default = source.default_pixel().first().copied().ok_or(RasterInputError::LengthMismatch)?;
    if !default.is_finite() {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let mut tmp = Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
    for y in r.y0..r.y1 {
        poll(c)?;
        let row = Rect::new(r.x0, y, r.x1, y + 1);
        let values = read(source, row, c)?;
        let mut paired = vector(values.len().checked_mul(2).ok_or(RasterInputError::LimitExceeded)?, 0f32, c)?;
        for (pair, &v) in paired.as_chunks_mut::<2>().0.iter_mut().zip(&values) {
            pair[0] = v;
            pair[1] = 1.0;
        }
        write(&mut tmp, row, &paired, c)?;
    }
    Ok((tmp, default, r))
}
fn flatten(source: &Surface, w: &Surface, default: f32, old: Rect, limits: GeometryLimits, c: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    let b = w.try_content_bounds_with_interrupt(c)?;
    let capacity = checked_region_byte_len(b, source.format())? as u64;
    if capacity.checked_mul(4).is_none_or(|v| v > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES)) {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut raw = [0u8; 4];
    write_sample(&mut raw, source.format().sample, 0, default);
    let mut out = Surface::try_with_default_bytes(source.format(), &raw[..source.format().bytes_per_pixel()])?;
    // The original vacated rectangle is written separately, never as a dense union.
    for y in old.y0..old.y1 {
        poll(c)?;
        let row = Rect::new(old.x0, y, old.x1, y + 1);
        let values = vector(old.width() as usize, default, c)?;
        write(&mut out, row, &values, c)?;
    }
    for y in b.y0..b.y1 {
        poll(c)?;
        let row = Rect::new(b.x0, y, b.x1, y + 1);
        let values = read(w, row, c)?;
        let mut flat = vector(b.width() as usize, 0f32, c)?;
        for (dst, p) in flat.iter_mut().zip(values.as_chunks::<2>().0.iter()) {
            *dst = p[0] * p[1] + default * (1.0 - p[1]);
        }
        write(&mut out, row, &flat, c)?;
    }
    out.try_prune_with(c)?;
    poll(c)?;
    Ok(out)
}
fn admit(
    source: &Surface,
    r: Rect,
    points: impl IntoIterator<Item = (f64, f64)>,
    triangles: u64,
    limits: GeometryLimits,
    c: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    poll(c)?;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut count = 0u64;
    for (x, y) in points {
        poll(c)?;
        if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
            return Err(RasterInputError::InvalidBounds);
        }
        lo[0] = lo[0].min(x);
        lo[1] = lo[1].min(y);
        hi[0] = hi[0].max(x);
        hi[1] = hi[1].max(y);
        count += 1;
    }
    if r.is_empty() {
        return Ok(());
    }
    let dst = Rect::new(lo[0].floor() as i32 - 1, lo[1].floor() as i32 - 1, hi[0].ceil() as i32 + 1, hi[1].ceil() as i32 + 1);
    let fmt = PixelFormat::new(source.format().mode, source.format().sample, true);
    let src = checked_region_byte_len(r, fmt)? as u64;
    let target = checked_region_byte_len(dst, fmt)? as u64;
    let pixels = u64::from(dst.width()).checked_mul(u64::from(dst.height())).ok_or(RasterInputError::LimitExceeded)?;
    let visits = (dst.tiles().count() as u64)
        .checked_mul(triangles)
        .and_then(|n| n.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64)))
        .ok_or(RasterInputError::LimitExceeded)?;
    let work = visits
        .checked_add(pixels.checked_mul(2048).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(src.checked_mul(64)?))
        .and_then(|n| n.checked_add(count.checked_mul(64)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    let retained = (source.tile_count() as u64)
        .checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * (source.format().bytes_per_pixel() as u64) + 4096)
        .ok_or(RasterInputError::LimitExceeded)?;
    let padded = tile_storage(r, fmt)?
        .checked_mul(2)
        .and_then(|n| n.checked_add(tile_storage(dst, fmt).ok()?.checked_mul(4)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    let resident = src
        .checked_mul(32)
        .and_then(|n| n.checked_add(retained))
        .and_then(|n| n.checked_add(padded))
        .and_then(|n| n.checked_add(target.checked_mul(16)?))
        .and_then(|n| n.checked_add(count.checked_mul(64)?))
        .and_then(|n| n.checked_add(triangles.checked_mul(128)?))
        .and_then(|n| n.checked_add(16 << 20))
        .ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units || resident > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}
pub fn try_warp_gray_homography_with_limits(
    source: &Surface,
    h: &Homography,
    interp: Interp,
    limits: GeometryLimits,
    c: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let r = source.try_content_bounds_with_interrupt(c)?;
    if h.0.iter().any(|v| !v.is_finite()) || h.inverse().is_none() {
        return Err(RasterInputError::InvalidBounds);
    }
    let mut sign = 0.0;
    for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)] {
        let d = h.0[6] * f64::from(x) + h.0[7] * f64::from(y) + h.0[8];
        if !d.is_finite() || d.abs() < 1e-12 || (sign != 0.0 && d.signum() != sign) {
            return Err(RasterInputError::InvalidBounds);
        }
        sign = d.signum();
    }
    admit(source, r, [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].into_iter().map(|(x, y)| h.apply(f64::from(x), f64::from(y))), 0, limits, c)?;
    let (tmp, default, r) = prepare(source, limits, c)?;
    let warped = crate::transform::try_warp_surface_legacy_with_limits(&tmp, r, h, interp, limits, c)?;
    flatten(source, &warped, default, r, limits, c)
}
pub fn try_warp_gray_mesh_with_limits(
    source: &Surface,
    f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync),
    interp: Interp,
    limits: GeometryLimits,
    c: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let r = source.try_content_bounds_with_interrupt(c)?;
    let sw = f64::from(r.width());
    let sh = f64::from(r.height());
    let nx = ((sw / CELL).ceil() as usize).clamp(1, MAX_CELLS);
    let ny = ((sh / CELL).ceil() as usize).clamp(1, MAX_CELLS);
    admit(
        source,
        r,
        (0..=ny).flat_map(|j| (0..=nx).map(move |i| f(f64::from(r.x0) + sw * i as f64 / nx as f64, f64::from(r.y0) + sh * j as f64 / ny as f64))),
        (nx * ny * 2) as u64,
        limits,
        c,
    )?;
    let (tmp, default, r) = prepare(source, limits, c)?;
    let warped = mesh(&tmp, r, f, interp, limits, c)?;
    flatten(source, &warped, default, r, limits, c)
}
fn mesh(
    src: &Surface,
    src_rect: Rect,
    f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync),
    interp: Interp,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let fmt = src.format();
    let mut out = Surface::new(fmt);
    if src_rect.is_empty() {
        return Ok(out);
    }
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    let nx = ((sw / CELL).ceil() as usize).clamp(1, MAX_CELLS);
    let ny = ((sh / CELL).ceil() as usize).clamp(1, MAX_CELLS);
    let vc = (nx + 1) * (ny + 1);
    let tc = nx * ny * 2;
    if (vc as u64).checked_mul(32).and_then(|n| n.checked_add((tc as u64) * 24)).is_none_or(|n| n > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES)) {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut verts = Vec::new();
    verts.try_reserve_exact(vc).map_err(|_| RasterInputError::AllocationFailed)?;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for j in 0..=ny {
        poll(control)?;
        for i in 0..=nx {
            let sx = f64::from(src_rect.x0) + sw * i as f64 / nx as f64;
            let sy = f64::from(src_rect.y0) + sh * j as f64 / ny as f64;
            let (dx, dy) = f(sx, sy);
            if [sx, sy, dx, dy].iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0) {
                return Err(RasterInputError::InvalidBounds);
            }
            lo[0] = lo[0].min(dx);
            lo[1] = lo[1].min(dy);
            hi[0] = hi[0].max(dx);
            hi[1] = hi[1].max(dy);
            verts.push(([dx, dy], [sx, sy]));
        }
    }
    let dst = Rect::new(lo[0].floor() as i32 - 1, lo[1].floor() as i32 - 1, hi[0].ceil() as i32 + 1, hi[1].ceil() as i32 + 1);
    let bytes = checked_region_byte_len(dst, fmt)? as u64;
    let source_bytes = checked_region_byte_len(src_rect, fmt)? as u64;
    let tiles = dst.tiles().count() as u64;
    let pixels = u64::from(dst.width()).checked_mul(u64::from(dst.height())).ok_or(RasterInputError::LimitExceeded)?;
    // Serial tiles retain original declaration order; precharge every candidate
    // triangle visit and the maximum 8x8 bilinear subsample footprint.
    let work = tiles
        .checked_mul(tc as u64)
        .and_then(|v| v.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64)))
        .and_then(|v| v.checked_add(pixels.checked_mul(1024)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    let scratch = source_bytes
        .checked_mul(8)
        .and_then(|v| v.checked_add((TILE_SIZE as u64) * (TILE_SIZE as u64) * 64))
        .and_then(|v| v.checked_add((tc as u64) * 64))
        .ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units
        || bytes.checked_mul(4).and_then(|v| v.checked_add(scratch)).is_none_or(|v| v > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES))
    {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut tris = Vec::new();
    tris.try_reserve_exact(tc).map_err(|_| RasterInputError::AllocationFailed)?;
    for j in 0..ny {
        poll(control)?;
        for i in 0..nx {
            let a = j * (nx + 1) + i;
            tris.push([a, a + 1, a + nx + 2]);
            tris.push([a, a + nx + 2, a + nx + 1]);
        }
    }
    let n = fmt.channels();
    let a = n - 1;
    for position in dst.tiles() {
        poll(control)?;
        let tile = position.rect().intersect(&dst);
        let t = &tile;
        let w = t.width() as usize;
        let h = t.height() as usize;
        let values = (|| -> Result<Option<Vec<f32>>, RasterInputError> {
            // Subsamples along one destination axis, from the source step `j` it makes: one where
            // the warp keeps or enlarges the source, else about one per source pixel the output pixel
            // spans (an area average, as the transform path gets from its pre-reduction; a single
            // bicubic sample aliases thin lines when shrinking). The subsamples are bilinear: the
            // box does the filtering, and it is ~4x cheaper than bicubic at no loss.
            let count = |j: [f64; 2]| if interp == Interp::Nearest { 1 } else { j[0].hypot(j[1]).round().clamp(1.0, MAX_SS) as usize };
            // Source position per destination pixel (NaN = not covered) and the triangle it came
            // from. Per triangle: its source step per destination pixel along x and y (`[dx_u, dx_v,
            // dy_u, dy_v]`, the inverse map's Jacobian, constant over a triangle; zero when
            // point-sampled) and the subsamples along each.
            let mut uv = vector(w * h, [f64::NAN; 2], control)?;
            let mut owner = vector(w * h, 0u32, control)?;
            let mut jac = Vec::new();
            jac.try_reserve_exact(tris.len()).map_err(|_| RasterInputError::AllocationFailed)?;
            for (k, tri) in tris.iter().enumerate() {
                poll(control)?;
                let [i0, i1, i2] = *tri;
                let (p0, s0) = *verts.get(i0).ok_or(RasterInputError::LengthMismatch)?;
                let (p1, s1) = *verts.get(i1).ok_or(RasterInputError::LengthMismatch)?;
                let (p2, s2) = *verts.get(i2).ok_or(RasterInputError::LengthMismatch)?;
                let det = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
                if det.abs() < 1e-12 {
                    jac.push(([0.0; 4], 1, 1));
                    continue;
                }
                let (d1, d2) = ([s1[0] - s0[0], s1[1] - s0[1]], [s2[0] - s0[0], s2[1] - s0[1]]);
                let (l1x, l1y, l2x, l2y) = ((p2[1] - p0[1]) / det, (p0[0] - p2[0]) / det, (p0[1] - p1[1]) / det, (p1[0] - p0[0]) / det);
                // Capped, so a sliver triangle can't ask for an unbounded footprint.
                let cap = |j: [f64; 2]| {
                    let l = j[0].hypot(j[1]);
                    if l > MAX_SS { [j[0] * MAX_SS / l, j[1] * MAX_SS / l] } else { j }
                };
                let jx = cap([l1x * d1[0] + l2x * d2[0], l1x * d1[1] + l2x * d2[1]]);
                let jy = cap([l1y * d1[0] + l2y * d2[0], l1y * d1[1] + l2y * d2[1]]);
                let (nx, ny) = (count(jx), count(jy));
                jac.push(if nx * ny == 1 { ([0.0; 4], 1, 1) } else { ([jx[0], jx[1], jy[0], jy[1]], nx, ny) });
                let bx0 = (p0[0].min(p1[0]).min(p2[0]) - 0.5).floor().max(f64::from(t.x0)) as i32;
                let by0 = (p0[1].min(p1[1]).min(p2[1]) - 0.5).floor().max(f64::from(t.y0)) as i32;
                let bx1 = (p0[0].max(p1[0]).max(p2[0]) + 0.5).ceil().min(f64::from(t.x1)) as i32;
                let by1 = (p0[1].max(p1[1]).max(p2[1]) + 0.5).ceil().min(f64::from(t.y1)) as i32;
                let eps = -1e-9;
                for y in by0..by1 {
                    poll(control)?;
                    let py = f64::from(y) + 0.5;
                    for x in bx0..bx1 {
                        let px = f64::from(x) + 0.5;
                        let l1 = ((px - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (py - p0[1])) / det;
                        let l2 = ((p1[0] - p0[0]) * (py - p0[1]) - (px - p0[0]) * (p1[1] - p0[1])) / det;
                        let l0 = 1.0 - l1 - l2;
                        if l0 < eps || l1 < eps || l2 < eps {
                            continue;
                        }
                        let o = (y - t.y0) as usize * w + (x - t.x0) as usize;
                        *uv.get_mut(o).ok_or(RasterInputError::LengthMismatch)? = [l0 * s0[0] + l1 * s1[0] + l2 * s2[0], l0 * s0[1] + l1 * s1[1] + l2 * s2[1]];
                        *owner.get_mut(o).ok_or(RasterInputError::LengthMismatch)? = u32::try_from(k).map_err(|_| RasterInputError::LimitExceeded)?;
                    }
                }
            }
            // Source footprint of the covered pixels, including the spread of their subsamples.
            let (mut fx0, mut fy0, mut fx1, mut fy1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (index, (q, &k)) in uv.iter().zip(&owner).enumerate() {
                if index % 256 == 0 {
                    poll(control)?;
                }
                if q[0].is_nan() {
                    continue;
                }
                let j = jac.get(k as usize).ok_or(RasterInputError::LengthMismatch)?.0;
                let (ex, ey) = (0.5 * (j[0].abs() + j[2].abs()), 0.5 * (j[1].abs() + j[3].abs()));
                fx0 = fx0.min(q[0] - ex);
                fy0 = fy0.min(q[1] - ey);
                fx1 = fx1.max(q[0] + ex);
                fy1 = fy1.max(q[1] + ey);
            }
            if fx0 > fx1 {
                return Ok(None);
            }
            let foot = Rect::new(fx0.floor() as i32 - 3, fy0.floor() as i32 - 3, fx1.ceil() as i32 + 3, fy1.ceil() as i32 + 3).intersect(&src_rect);
            if foot.is_empty() || !src.has_tiles_in(foot) {
                return Ok(None);
            }
            let mut px = read(src, foot, control)?;
            for p in px.chunks_exact_mut(n) {
                let al = *p.get(a).ok_or(RasterInputError::LengthMismatch)?;
                for v in p.get_mut(..a).ok_or(RasterInputError::LengthMismatch)? {
                    *v *= al;
                }
            }
            let fw = foot.width() as usize;
            let at = |x: i32, y: i32, c: usize| -> Result<f64, RasterInputError> {
                if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                    Ok(0.0)
                } else {
                    let offset = ((y - foot.y0) as usize)
                        .checked_mul(fw)
                        .and_then(|v| v.checked_add((x - foot.x0) as usize))
                        .and_then(|v| v.checked_mul(n))
                        .and_then(|v| v.checked_add(c))
                        .ok_or(RasterInputError::LimitExceeded)?;
                    Ok(f64::from(*px.get(offset).ok_or(RasterInputError::LengthMismatch)?))
                }
            };
            let mut outp = vector(w * h * n, 0.0f32, control)?;
            let mut any = false;
            let mut acc = [0.0f64; 8];
            let mut one = [0.0f64; 8];
            // One premultiplied sample at source position (u, v) (pixel centres at integers).
            let point = |interp: Interp, u: f64, v: f64, acc: &mut [f64; 8]| -> Result<(), RasterInputError> {
                acc.get_mut(..n).ok_or(RasterInputError::LengthMismatch)?.fill(0.0);
                match interp {
                    Interp::Nearest => {
                        let (ix, iy) = ((u + 0.5).floor() as i32, (v + 0.5).floor() as i32);
                        for (c, s) in acc.iter_mut().enumerate().take(n) {
                            *s = at(ix, iy, c)?;
                        }
                    }
                    Interp::Bilinear => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (fx, fy) = (u - f64::from(ix), v - f64::from(iy));
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += at(ix + dx, iy + dy, c)? * wx * wy;
                                }
                            }
                        }
                    }
                    Interp::Bicubic => {
                        // Snap to the pixel grid when within float noise, so identity is exact.
                        let snap = |t: f64| if (t - t.round()).abs() < 1e-7 { t.round() } else { t };
                        let (u, v) = (snap(u), snap(v));
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (wx, wy) = (bicubic(u - f64::from(ix)), bicubic(v - f64::from(iy)));
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                if k == 0.0 {
                                    continue;
                                }
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += at(ix - 1 + i as i32, iy - 1 + j as i32, c)? * k;
                                }
                            }
                        }
                    }
                }
                Ok(())
            };
            for (o, (q, &k)) in uv.iter().zip(&owner).enumerate() {
                if o % 128 == 0 {
                    poll(control)?;
                }
                if q[0].is_nan() {
                    continue;
                }
                let (j, nx, ny) = *jac.get(k as usize).ok_or(RasterInputError::LengthMismatch)?;
                if nx * ny == 1 {
                    point(interp, q[0] - 0.5, q[1] - 0.5, &mut acc)?;
                } else {
                    acc.get_mut(..n).ok_or(RasterInputError::LengthMismatch)?.fill(0.0);
                    for sy in 0..ny {
                        let t = (sy as f64 + 0.5) / ny as f64 - 0.5;
                        for sx in 0..nx {
                            let s = (sx as f64 + 0.5) / nx as f64 - 0.5;
                            point(Interp::Bilinear, q[0] + s * j[0] + t * j[2] - 0.5, q[1] + s * j[1] + t * j[3] - 0.5, &mut one)?;
                            for (c, v) in acc.iter_mut().zip(&one).take(n) {
                                *c += v;
                            }
                        }
                    }
                    let k = 1.0 / (nx * ny) as f64;
                    for c in acc.iter_mut().take(n) {
                        *c *= k;
                    }
                }
                let al = acc.get(a).ok_or(RasterInputError::LengthMismatch)?.clamp(0.0, 1.0);
                // Float noise (a vertex a hair off the pixel grid) must not leave invisible
                // colour behind in transparent pixels.
                if al <= 1e-6 {
                    continue;
                }
                any = true;
                let b = o * n;
                for c in 0..a {
                    *outp.get_mut(b + c).ok_or(RasterInputError::LengthMismatch)? =
                        (*acc.get(c).ok_or(RasterInputError::LengthMismatch)? / al).clamp(0.0, 1.0) as f32;
                }
                *outp.get_mut(b + a).ok_or(RasterInputError::LengthMismatch)? = al as f32;
            }
            Ok(any.then_some(outp))
        })()?;
        if let Some(values) = values {
            write(&mut out, tile, &values, control)?;
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
    fn homography_matches_upstream_gray_default_and_distant_regions() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, false), &[0.25]);
            source.fill_rect(Rect::new(-2, 1, 3, 4), &[0.75]);
            for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                let h = Homography([1.0, 0.0, 300.0, 0.0, 1.0, -10.0, 0.0, 0.0, 1.0]);
                let (tmp, default, r) = prepare(&source, GeometryLimits::default(), &Interrupt::NONE).unwrap();
                let old = crate::transform::try_warp_surface(&tmp, r, &h, interp).unwrap();
                let expected = flatten(&source, &old, default, r, GeometryLimits::default(), &Interrupt::NONE).unwrap();
                let got = try_warp_gray_homography_with_limits(&source, &h, interp, GeometryLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(got, expected);
            }
        }
    }
    #[test]
    fn mesh_preserves_upstream_triangle_order_and_default_flattening() {
        let mut source = Surface::with_default(PixelFormat::GRAY8, &[0.25]);
        source.fill_rect(Rect::new(1, 2, 7, 6), &[0.75]);
        for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
            let map = |x: f64, y: f64| (x + 0.1 * (y - 2.0) * (y - 2.0), y + 0.2 * x);
            let expected = crate::warp::try_warp_mesh_gray(&source, &map, interp).unwrap();
            let got = try_warp_gray_mesh_with_limits(&source, &map, interp, GeometryLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(got, expected);
        }
    }
    #[test]
    fn cancellation_and_tiny_admission_leave_source_bytes_unchanged() {
        let mut source = Surface::new(PixelFormat::GRAY8);
        source.fill_rect(Rect::new(-3, 1, 5, 8), &[0.5]);
        let before = source.clone();
        let ctl = Interrupt::cancel_only(&|| true);
        assert_eq!(
            try_warp_gray_mesh_with_limits(&source, &|x, y| (x + 2.0, y), Interp::Bilinear, GeometryLimits::default(), &ctl),
            Err(RasterInputError::Cancelled)
        );
        assert_eq!(
            try_warp_gray_homography_with_limits(
                &source,
                &Homography::IDENTITY,
                Interp::Nearest,
                GeometryLimits { resident_bytes: 1, work_units: 1 },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        );
        assert_eq!(source, before);
    }
}
