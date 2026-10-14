//! Exact ordinary raster bevel preparation with caller-charged checked pattern textures.
use super::{BEVEL_H_EPS, Bevel, BevelGeom, BevelPaint, BevelTechnique, GlobalLight, INSIDE_EPS, Map, bevel_geom, box_geom};
use photocraft_raster::{Interrupt, RasterInputError as E};
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub(super) fn validate(b: &Bevel) -> Result<(), E> {
    let g = bevel_geom(b);
    super::controlled_contour::requirements(&b.gloss_contour, None, false)?;
    if let Some(contour) = &b.contour {
        super::controlled_contour::requirements(&contour.contour, None, true)?;
    }
    if let Some(t) = &b.texture {
        if !t.scale.is_finite() || !t.depth.is_finite() || !t.phase.0.is_finite() || !t.phase.1.is_finite() {
            return Err(E::UnsupportedConversion);
        }
        let names = t.id.len().checked_add(t.name.len()).ok_or(E::LimitExceeded)?;
        if (names as u64).checked_mul(4).is_none_or(|n| n > 8_000_000_000) {
            return Err(E::LimitExceeded);
        }
    }
    if !b.size.is_finite()
        || !b.depth.is_finite()
        || !b.soften.is_finite()
        || !b.angle.is_finite()
        || !b.altitude.is_finite()
        || !g.width.is_finite()
        || !g.depth.is_finite()
        || !g.chisel_soft.is_finite()
    {
        return Err(E::UnsupportedConversion);
    }
    Ok(())
}
pub(super) fn validate_light(b: &Bevel, light: &GlobalLight) -> Result<(), E> {
    let (angle, altitude) = if b.use_global_light { (light.angle, light.altitude) } else { (b.angle, b.altitude) };
    if !angle.is_finite() || !altitude.is_finite() {
        return Err(E::UnsupportedConversion);
    }
    Ok(())
}
pub(super) fn scratch_bytes(w: usize, h: usize, b: &Bevel) -> Result<usize, E> {
    validate(b)?;
    let g = bevel_geom(b);
    let (r, _, _) = box_geom(g.width);
    let (s, _, _) = box_geom(b.soften);
    let (ch, _, _) = box_geom(g.chisel_soft);
    let line = w
        .max(h)
        .checked_add(usize::try_from(r.max(s).max(ch)).map_err(|_| E::LimitExceeded)?.checked_mul(2).ok_or(E::LimitExceeded)?)
        .ok_or(E::LimitExceeded)?;
    // Worst-case sharp list + slots + immutable refinement points/normals:
    // 44 bytes/pixel. Second EDT with retained sharp data and first distance:
    // 49 bytes/pixel plus line scratch. Height, two shade pairs and mix peaks
    // are lower; reserve 64 bytes/pixel and fixed refinement stack/header space.
    let gloss = super::controlled_contour::requirements(&b.gloss_contour, None, false)?;
    let height = if let Some(contour) = &b.contour { super::controlled_contour::requirements(&contour.contour, None, true)?.peak_bytes } else { 0 };
    let contour_peak = gloss.peak_bytes.max(height);
    let base = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(if b.technique == BevelTechnique::Smooth { 32 } else { 64 }))
        .and_then(|n| line.checked_mul(24).and_then(|v| n.checked_add(v)))
        .and_then(|n| n.checked_add(contour_peak))
        .and_then(|n| n.checked_add(1024))
        .ok_or(E::LimitExceeded)?;
    if b.technique != BevelTechnique::Smooth {
        let edt = super::controlled_euclidean::scratch_bytes(w, h)?;
        let retained = w.checked_mul(h).and_then(|n| n.checked_mul(28)).ok_or(E::LimitExceeded)?;
        Ok(base.max(edt.checked_add(retained).ok_or(E::LimitExceeded)?))
    } else {
        Ok(base)
    }
}
pub(super) fn work_units(w: usize, h: usize, b: &Bevel) -> Result<u64, E> {
    validate(b)?;
    let g = bevel_geom(b);
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)? as u64;
    let (r, _, _) = box_geom(g.width);
    let (s, _, _) = box_geom(b.soften);
    let (ch, _, _) = box_geom(g.chisel_soft);
    let radius = u64::try_from(r)
        .map_err(|_| E::LimitExceeded)?
        .checked_add(if b.soften >= 1.0 { u64::try_from(s).map_err(|_| E::LimitExceeded)? } else { 0 })
        .and_then(|v| v.checked_add(if g.chisel_soft > 0.0 && g.paint == BevelPaint::Both { u64::try_from(ch).ok()? } else { 0 }))
        .ok_or(E::LimitExceeded)?;
    // Covers both exact EDT passes, radius-3 refinement, radius-2 foot searches,
    // coverage gradients, private initialization and the lighting/mix passes.
    let mut contour_work = super::controlled_contour::requirements(&b.gloss_contour, None, false)?.work_units;
    if let Some(contour) = &b.contour {
        contour_work = contour_work.checked_add(super::controlled_contour::requirements(&contour.contour, None, true)?.work_units).ok_or(E::LimitExceeded)?;
    }
    let work = n
        .checked_mul(if b.technique != BevelTechnique::Smooth {
            4096
        } else if g.paint == BevelPaint::Both {
            768
        } else {
            512
        })
        .and_then(|n| (w as u64).checked_add(h as u64).and_then(|v| v.checked_mul(radius)).and_then(|v| v.checked_mul(16)).and_then(|v| n.checked_add(v)))
        .and_then(|n| n.checked_add(contour_work))
        .and_then(|n| {
            if let Some(t) = &b.texture {
                (t.id.len() as u64)
                    .checked_add(t.name.len() as u64)
                    .and_then(|v| v.checked_mul(4))
                    .and_then(|v| (w as u64).checked_mul(h as u64).and_then(|p| p.checked_mul(128)).and_then(|p| p.checked_add(v)))
                    .and_then(|v| n.checked_add(v))
            } else {
                Some(n)
            }
        })
        .ok_or(E::LimitExceeded)?;
    if work > 8_000_000_000 {
        return Err(E::LimitExceeded);
    }
    Ok(work)
}
fn filled(n: usize, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    poll(c)?;
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() != n {
        return Err(E::LimitExceeded);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(4096).min(n), 0.0);
    }
    Ok(out)
}
fn get(m: &Map, x: i64, y: i64) -> Result<f32, E> {
    if x < 0 || y < 0 || x >= m.w as i64 || y >= m.h as i64 {
        return Ok(0.0);
    }
    m.v.get(y as usize * m.w + x as usize).copied().ok_or(E::LengthMismatch)
}
struct Lighting<'a> {
    vector: [f32; 3],
    elevation: f32,
    gloss: Option<&'a [f32]>,
}
fn shade(shape: &Map, height: &Map, reach_past: bool, light: &Lighting<'_>, depth: f32, outer: bool, c: &Interrupt<'_>) -> Result<(Map, Map), E> {
    let n = shape.w.checked_mul(shape.h).ok_or(E::LimitExceeded)?;
    let (mut hi, mut sh) = (filled(n, c)?, filled(n, c)?);
    for y in 0..shape.h {
        poll(c)?;
        for x in 0..shape.w {
            poll(c)?;
            let xi = x as i64;
            let yi = y as i64;
            let i = y * shape.w + x;
            let gx = (get(height, xi + 1, yi)? - get(height, xi - 1, yi)?) * 0.5 * depth;
            let gy = (get(height, xi, yi + 1)? - get(height, xi, yi - 1)?) * 0.5 * depth;
            let normal = [-gx, -gy, 1.0];
            let [a, b, z] = normal;
            let len = (a * a + b * b + 1.0).sqrt();
            let [lx, ly, lz] = light.vector;
            let value = (a * lx + b * ly + z * lz) / len;
            if !gx.is_finite() || !gy.is_finite() || !len.is_finite() || !value.is_finite() {
                return Err(E::InvalidSample);
            }
            let coverage = *shape.v.get(i).ok_or(E::LengthMismatch)?;
            let mut on = get(height, xi, yi)? > BEVEL_H_EPS;
            if !on && reach_past {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    poll(c)?;
                    if get(height, xi + dx, yi + dy)? > BEVEL_H_EPS {
                        on = true;
                        break;
                    }
                }
            }
            let region = if !outer { coverage } else { f32::from(coverage < 1.0 - INSIDE_EPS && on) };
            let k = value - light.elevation;
            if k > 0.0 {
                *hi.get_mut(i).ok_or(E::LengthMismatch)? = (k / (1.0 - light.elevation).max(1e-3)).clamp(0.0, 1.0) * region;
            } else {
                *sh.get_mut(i).ok_or(E::LengthMismatch)? = (-k / light.elevation.max(1e-3)).clamp(0.0, 1.0) * region;
            }
        }
    }
    let mut hi = Map { w: shape.w, h: shape.h, v: hi };
    let mut sh = Map { w: shape.w, h: shape.h, v: sh };
    super::controlled_contour::apply(&mut hi, light.gloss, c)?;
    super::controlled_contour::apply(&mut sh, light.gloss, c)?;
    Ok((hi, sh))
}
fn mix(a: Map, b: Map, shape: &Map, c: &Interrupt<'_>) -> Result<Map, E> {
    let mut values = filled(shape.v.len(), c)?;
    for (i, (((o, x), y), s)) in values.iter_mut().zip(&a.v).zip(&b.v).zip(&shape.v).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *o = if *s > INSIDE_EPS { *x } else { 0.0 } + y * (1.0 - s.min(1.0));
    }
    Ok(Map { w: a.w, h: a.h, v: values })
}
#[cfg(test)]
pub(super) fn build(shape: &Map, b: &Bevel, light: &GlobalLight, budget: usize, c: &Interrupt<'_>) -> Result<(Vec<Map>, BevelPaint), E> {
    build_with_texture(shape, b, light, None, budget, c)
}
pub(super) fn build_with_texture(
    shape: &Map,
    b: &Bevel,
    light: &GlobalLight,
    texture: Option<(&super::TextureCtx<'_>, &super::PreparedPatterns<'_>)>,
    budget: usize,
    c: &Interrupt<'_>,
) -> Result<(Vec<Map>, BevelPaint), E> {
    poll(c)?;
    validate(b)?;
    validate_light(b, light)?;
    let texture_tile = if let Some(t) = &b.texture {
        let (tex, patterns) = texture.ok_or(E::UnsupportedConversion)?;
        if !tex.anchor.0.is_finite() || !tex.anchor.1.is_finite() {
            return Err(E::UnsupportedConversion);
        }
        Some((tex, patterns.get_prepared_with_interrupt(&t.id, &t.name, c)?))
    } else {
        None
    };
    let n = shape.w.checked_mul(shape.h).ok_or(E::LimitExceeded)?;
    if n != shape.v.len() {
        return Err(E::LengthMismatch);
    }
    let peak = scratch_bytes(shape.w, shape.h, b)?;
    work_units(shape.w, shape.h, b)?;
    if peak > budget.min(64 << 20) {
        return Err(E::LimitExceeded);
    }
    for values in shape.v.chunks(4096) {
        poll(c)?;
        if values.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
    }
    let g = bevel_geom(b);
    let mut height = if b.technique == BevelTechnique::Smooth {
        super::checked_prepare::try_tent(shape, budget, g.width, c)?
    } else {
        chisel_height(shape, b, &g, budget, c)?
    };
    if let Some(contour) = &b.contour {
        let lut = super::controlled_contour::prepare(&contour.contour, None, true, budget, c)?;
        super::controlled_contour::apply(&mut height, lut.as_deref(), c)?;
    }
    if let Some((tex, Some(tile))) = texture_tile {
        let t = b.texture.as_ref().ok_or(E::UnsupportedConversion)?;
        let unit = if b.depth.abs() > 1e-6 { (g.depth / b.depth).abs().max(1e-3) } else { g.width.max(1.0) };
        let k = t.depth / unit;
        if !unit.is_finite() || !k.is_finite() {
            return Err(E::UnsupportedConversion);
        }
        let place = super::Placement::anchored(tex.anchor, t.link, t.phase, t.scale, 0.0);
        for y in 0..height.h {
            poll(c)?;
            for x in 0..height.w {
                if x.is_multiple_of(256) {
                    poll(c)?;
                }
                let wx = i64::from(tex.rect.x0).checked_add(i64::try_from(x).map_err(|_| E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
                let wy = i64::from(tex.rect.y0).checked_add(i64::try_from(y).map_err(|_| E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
                let (u, v) = place.map(wx as f64 + 0.5, wy as f64 + 0.5);
                if !u.is_finite() || !v.is_finite() {
                    return Err(E::InvalidSample);
                }
                let p = tile.sample(u, v);
                if p.iter().any(|v| !v.is_finite()) {
                    return Err(E::InvalidSample);
                }
                let l = (0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]) * p[3];
                let index = y.checked_mul(height.w).and_then(|n| n.checked_add(x)).ok_or(E::LimitExceeded)?;
                let value = height.v.get_mut(index).ok_or(E::LengthMismatch)?;
                *value += k * if t.invert { 1.0 - l } else { l };
                if !value.is_finite() {
                    return Err(E::InvalidSample);
                }
            }
        }
    }
    if b.soften >= 1.0 {
        let softened = super::checked_prepare::try_tent(&height, budget, b.soften, c)?;
        height = softened;
    }
    let (angle, altitude) = if b.use_global_light { (light.angle, light.altitude) } else { (b.angle, b.altitude) };
    if !angle.is_finite() || !altitude.is_finite() {
        return Err(E::UnsupportedConversion);
    }
    let (sa, ca) = angle.to_radians().sin_cos();
    let (se, ce) = altitude.to_radians().sin_cos();
    let gloss = super::controlled_contour::prepare(&b.gloss_contour, None, false, budget, c)?;
    let lighting = Lighting { vector: [ca * ce, -sa * ce, se], elevation: se, gloss: gloss.as_deref() };
    let mut maps = Vec::new();
    maps.try_reserve_exact(2).map_err(|_| E::AllocationFailed)?;
    if maps.capacity() != 2 {
        return Err(E::LimitExceeded);
    }
    let reach_past = g.paint == BevelPaint::Both || (g.paint == BevelPaint::Outer && b.technique != BevelTechnique::Smooth);
    match g.paint {
        BevelPaint::Inner | BevelPaint::Outer => {
            let (hi, sh) = shade(shape, &height, reach_past, &lighting, g.depth, g.paint == BevelPaint::Outer, c)?;
            maps.push(hi);
            maps.push(sh);
        }
        BevelPaint::Both => {
            let (hi, sh) = shade(shape, &height, reach_past, &lighting, g.depth, false, c)?;
            let (ho, so) = shade(shape, &height, reach_past, &lighting, if g.pillow { -g.depth } else { g.depth }, true, c)?;
            maps.push(mix(hi, ho, shape, c)?);
            maps.push(mix(sh, so, shape, c)?);
        }
    }
    for map in &maps {
        for values in map.v.chunks(4096) {
            poll(c)?;
            if values.iter().any(|v| !v.is_finite()) {
                return Err(E::InvalidSample);
            }
        }
    }
    poll(c)?;
    Ok((maps, g.paint))
}

// Serial row order matches the original row-flattened edge list. Every refinement
// reads immutable pre-refinement offsets/normals; no dependency on worker order.
fn owned<T: Copy>(n: usize, value: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() != n {
        return Err(E::LimitExceeded);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(4096).min(n), value);
    }
    Ok(out)
}
fn coverage(s: &Map, x: i64, y: i64, c: &Interrupt<'_>) -> Result<f32, E> {
    poll(c)?;
    let a = get(s, x, y)?;
    if a <= INSIDE_EPS {
        return Ok(a);
    }
    let mut mx = a;
    for dy in -1..=1 {
        for dx in -1..=1 {
            poll(c)?;
            mx = mx.max(get(s, x + dx, y + dy)?);
        }
    }
    Ok((a / mx).min(1.0))
}
fn sharp_edges(s: &Map, c: &Interrupt<'_>) -> Result<super::SharpEdges, E> {
    use super::{EdgePixel, SharpEdge, SharpEdges};
    let n = s.w.checked_mul(s.h).ok_or(E::LimitExceeded)?;
    if n != s.v.len() {
        return Err(E::LengthMismatch);
    }
    if n == 0 {
        return Ok(SharpEdges::empty(s.w, s.h));
    }
    if n >= u32::MAX as usize {
        return Err(E::LimitExceeded);
    }
    let wi = i64::try_from(s.w).map_err(|_| E::LimitExceeded)?;
    let hi = i64::try_from(s.h).map_err(|_| E::LimitExceeded)?;
    let cov_clamped = |x: i64, y: i64| coverage(s, x.max(0).min(wi - 1), y.max(0).min(hi - 1), c);
    let solid = |x: i64, y: i64| -> Result<bool, E> {
        poll(c)?;
        if x < 0 || y < 0 || x >= wi || y >= hi {
            return Ok(true);
        }
        Ok(get(s, x, y)? > INSIDE_EPS)
    };
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if pixels.capacity() != n {
        return Err(E::LimitExceeded);
    }
    for y in 0..hi {
        for x in 0..wi {
            poll(c)?;
            if !solid(x, y)? {
                continue;
            }
            let mut all = true;
            'neighbors: for dy in -1..=1 {
                for dx in -1..=1 {
                    if !solid(x + dx, y + dy)? {
                        all = false;
                        break 'neighbors;
                    }
                }
            }
            if all {
                continue;
            }
            let i = usize::try_from(y).map_err(|_| E::LimitExceeded)?.checked_mul(s.w).and_then(|n| n.checked_add(x as usize)).ok_or(E::LimitExceeded)?;
            let a = *s.v.get(i).ok_or(E::LengthMismatch)?;
            let cov = coverage(s, x, y, c)?;
            if a <= INSIDE_EPS || cov >= 1.0 - INSIDE_EPS {
                continue;
            }
            let (mut empty, mut full) = (false, false);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    poll(c)?;
                    let (nx, ny) = (x + dx, y + dy);
                    if (dx, dy) == (0, 0) || nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                        continue;
                    }
                    empty |= get(s, nx, ny)? <= INSIDE_EPS;
                    full |= coverage(s, nx, ny, c)? >= 1.0 - INSIDE_EPS;
                }
            }
            if !(empty && full) {
                continue;
            }
            let gx = (cov_clamped(x + 1, y - 1)? + 2.0 * cov_clamped(x + 1, y)? + cov_clamped(x + 1, y + 1)?)
                - (cov_clamped(x - 1, y - 1)? + 2.0 * cov_clamped(x - 1, y)? + cov_clamped(x - 1, y + 1)?);
            let gy = (cov_clamped(x - 1, y + 1)? + 2.0 * cov_clamped(x, y + 1)? + cov_clamped(x + 1, y + 1)?)
                - (cov_clamped(x - 1, y - 1)? + 2.0 * cov_clamped(x, y - 1)? + cov_clamped(x + 1, y - 1)?);
            let len = gx.hypot(gy);
            let (normal, norm) = if len.is_finite() && len > 1e-6 { ([gx / len, gy / len], [gx / len, gy / len]) } else { ([0.0, 0.0], [1.0, 0.0]) };
            pixels.push(EdgePixel {
                at: u32::try_from(i).map_err(|_| E::LimitExceeded)?,
                cov,
                edge: SharpEdge { offset: super::edge_offset(norm, cov), normal },
            });
        }
    }
    let mut slot = owned(n, 0u32, c)?;
    for (k, p) in pixels.iter().enumerate() {
        poll(c)?;
        *slot.get_mut(p.at as usize).ok_or(E::LengthMismatch)? = u32::try_from(k.checked_add(1).ok_or(E::LimitExceeded)?).map_err(|_| E::LimitExceeded)?;
    }
    refine(&mut pixels, &slot, s.w, s.h, c)?;
    Ok(SharpEdges { w: s.w, h: s.h, slot, pixels })
}
fn refine(pixels: &mut [super::EdgePixel], slot: &[u32], w: usize, h: usize, c: &Interrupt<'_>) -> Result<(), E> {
    use super::{EDGE_LINE_MAX_OFF, EDGE_LINE_MIN_SPREAD, NORMAL_RADIUS, SharpEdge};
    let wi = i64::try_from(w).map_err(|_| E::LimitExceeded)?;
    let hi = i64::try_from(h).map_err(|_| E::LimitExceeded)?;
    let mut points = owned(pixels.len(), [0.0f32; 2], c)?;
    let mut before = owned(pixels.len(), SharpEdge { offset: 0.0, normal: [0.0; 2] }, c)?;
    for ((point, old), p) in points.iter_mut().zip(&mut before).zip(pixels.iter()) {
        poll(c)?;
        *point = [(p.at as i64 % wi) as f32 - p.edge.offset * p.edge.normal[0], (p.at as i64 / wi) as f32 - p.edge.offset * p.edge.normal[1]];
        *old = p.edge;
    }
    for (k, pixel) in pixels.iter_mut().enumerate() {
        poll(c)?;
        let e = *before.get(k).ok_or(E::LengthMismatch)?;
        let (x, y) = (pixel.at as i64 % wi, pixel.at as i64 / wi);
        let (mut n, mut sum) = (0.0f32, [0.0f32; 2]);
        let mut near = [[0.0f32; 2]; 49];
        let mut count = 0usize;
        for dy in -NORMAL_RADIUS..=NORMAL_RADIUS {
            for dx in -NORMAL_RADIUS..=NORMAL_RADIUS {
                poll(c)?;
                let (nx, ny) = (x + dx, y + dy);
                if dx * dx + dy * dy > NORMAL_RADIUS * NORMAL_RADIUS || nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                    continue;
                }
                let i = (ny as usize).checked_mul(w).and_then(|n| n.checked_add(nx as usize)).ok_or(E::LimitExceeded)?;
                let Some(j) = slot.get(i).and_then(|&s| (s as usize).checked_sub(1)) else {
                    continue;
                };
                let (Some(o), Some(&p)) = (before.get(j), points.get(j)) else {
                    return Err(E::LengthMismatch);
                };
                if o.normal[0] * e.normal[0] + o.normal[1] * e.normal[1] > 0.9 {
                    *near.get_mut(count).ok_or(E::LimitExceeded)? = p;
                    count = count.checked_add(1).ok_or(E::LimitExceeded)?;
                    sum = [sum[0] + p[0], sum[1] + p[1]];
                    n += 1.0;
                }
            }
        }
        if count < 3 {
            continue;
        }
        let mean = [sum[0] / n, sum[1] / n];
        let (mut sxx, mut sxy, mut syy) = (0.0f32, 0.0f32, 0.0f32);
        for p in near.get(..count).ok_or(E::LengthMismatch)? {
            poll(c)?;
            let (dx, dy) = (p[0] - mean[0], p[1] - mean[1]);
            sxx += dx * dx;
            sxy += dx * dy;
            syy += dy * dy;
        }
        let half = 0.5 * (sxx + syy);
        let root = (0.25 * (sxx - syy) * (sxx - syy) + sxy * sxy).sqrt();
        let (along, across) = (((half + root) / n).max(0.0).sqrt(), ((half - root) / n).max(0.0).sqrt());
        if !(along > EDGE_LINE_MIN_SPREAD && across < EDGE_LINE_MAX_OFF) {
            continue;
        }
        let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let mut normal = [-angle.sin(), angle.cos()];
        if normal[0] * e.normal[0] + normal[1] * e.normal[1] < 0.0 {
            normal = [-normal[0], -normal[1]];
        }
        pixel.edge.normal = normal;
        pixel.edge.offset = super::edge_offset(normal, pixel.cov);
        if !pixel.edge.offset.is_finite() || normal.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
    }
    poll(c)
}
fn edge_distance(sharp: &super::SharpEdges, p: [usize; 2], q: [usize; 2], inside: bool, c: &Interrupt<'_>) -> Result<Option<f32>, E> {
    use super::FOOT_RADIUS;
    poll(c)?;
    let (px, py) = (p[0] as f32, p[1] as f32);
    let Some(e) = sharp.at(q[0], q[1]) else {
        return Ok(None);
    };
    let Some(mut best) = e.distance(px - q[0] as f32, py - q[1] as f32, inside) else {
        return Ok(None);
    };
    let signed = e.offset + (px - q[0] as f32) * e.normal[0] + (py - q[1] as f32) * e.normal[1];
    let (fx, fy) = ((px - signed * e.normal[0]).round(), (py - signed * e.normal[1]).round());
    if !fx.is_finite() || !fy.is_finite() {
        return Err(E::InvalidSample);
    }
    let (fx, fy) = (fx as i64, fy as i64);
    for sy in fy.saturating_sub(FOOT_RADIUS).max(0)..=fy.saturating_add(FOOT_RADIUS).min(sharp.h as i64 - 1) {
        for sx in fx.saturating_sub(FOOT_RADIUS).max(0)..=fx.saturating_add(FOOT_RADIUS).min(sharp.w as i64 - 1) {
            poll(c)?;
            if let Some(near) = sharp.at(sx as usize, sy as usize)
                && let Some(d) = near.distance(px - sx as f32, py - sy as f32, inside)
            {
                best = best.min(d);
            }
        }
    }
    if !best.is_finite() {
        return Err(E::InvalidSample);
    }
    Ok(Some(best))
}
fn bevel_distance(s: &Map, sharp: &super::SharpEdges, reach: f32, inner: bool, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let n = s.v.len();
    let mut seeds = owned(n, false, c)?;
    for (seed, a) in seeds.iter_mut().zip(&s.v) {
        poll(c)?;
        *seed = if inner { *a <= INSIDE_EPS } else { *a > INSIDE_EPS };
    }
    let (d, near) = super::controlled_euclidean::edt(&seeds, s.w, s.h, c)?;
    let mut out = filled(n, c)?;
    for (i, v) in out.iter_mut().enumerate() {
        poll(c)?;
        let (x, y) = (i % s.w, i / s.w);
        let distance = *d.get(i).ok_or(E::LengthMismatch)?;
        let q = *near.get(i).ok_or(E::LengthMismatch)? as usize;
        if inner {
            if *seeds.get(i).ok_or(E::LengthMismatch)? {
                *v = -0.5;
                continue;
            }
            if let Some(e) = sharp.get(i) {
                *v = e.offset;
                continue;
            }
            let plain = distance - 0.5;
            if distance > reach + super::EDGE_MARGIN || sharp.is_empty() {
                *v = plain;
                continue;
            }
            let (px, py) = (x as i64, y as i64);
            let (ox, oy) = ((q % s.w) as i64, (q / s.w) as i64);
            let mut closest = None;
            let mut best = i64::MAX;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    poll(c)?;
                    let (nx, ny) = (ox + dx, oy + dy);
                    if nx < 0 || ny < 0 || !sharp.has(nx as usize, ny as usize) {
                        continue;
                    }
                    let key = (nx - px) * (nx - px) + (ny - py) * (ny - py);
                    if key < best {
                        best = key;
                        closest = Some([nx as usize, ny as usize]);
                    }
                }
            }
            *v = match closest {
                Some(q) => edge_distance(sharp, [x, y], q, true, c)?.map_or(plain, |r| r.min(plain)),
                None => plain,
            };
        } else {
            if let Some(e) = sharp.get(i) {
                *v = -e.offset;
                continue;
            }
            if *seeds.get(i).ok_or(E::LengthMismatch)? {
                *v = -0.5;
                continue;
            }
            let Some(e) = sharp.get(q) else {
                *v = distance - (s.v.get(q).ok_or(E::LengthMismatch)?.min(1.0) - 0.5);
                continue;
            };
            let qp = [q % s.w, q / s.w];
            let measured = if distance <= reach + super::EDGE_MARGIN {
                edge_distance(sharp, [x, y], qp, false, c)?
            } else {
                e.distance(x as f32 - qp[0] as f32, y as f32 - qp[1] as f32, false)
            };
            *v = measured.unwrap_or(distance - e.offset);
        }
        if !v.is_finite() {
            return Err(E::InvalidSample);
        }
    }
    poll(c)?;
    Ok(out)
}
fn chisel_height(shape: &Map, b: &Bevel, g: &BevelGeom, budget: usize, c: &Interrupt<'_>) -> Result<Map, E> {
    if shape.v.is_empty() {
        return Ok(Map { w: shape.w, h: shape.h, v: Vec::new() });
    }
    let sharp = sharp_edges(shape, c)?;
    let size = b.size.max(1.0);
    let din = bevel_distance(shape, &sharp, size, true, c)?;
    let dout = bevel_distance(shape, &sharp, size, false, c)?;
    drop(sharp);
    let mut height = Map { w: shape.w, h: shape.h, v: filled(shape.v.len(), c)? };
    for ((v, di), dout) in height.v.iter_mut().zip(&din).zip(&dout) {
        poll(c)?;
        *v = super::bevel_chisel_h(g.paint, size, *di, *dout, g.chisel_soft > 0.0);
    }
    drop(din);
    drop(dout);
    if g.chisel_soft > 0.0 && g.paint == BevelPaint::Both {
        height = super::checked_prepare::try_tent(&height, budget, g.chisel_soft, c)?;
    }
    Ok(height)
}
#[cfg(test)]
mod tests {
    use super::super::Contour;
    use super::*;
    use photocraft_doc::{BevelContour, BevelStyle, BlendMode, Color, FxCommon};
    fn bevel(style: BevelStyle) -> Bevel {
        Bevel {
            enabled: true,
            style,
            technique: BevelTechnique::Smooth,
            depth: 1.0,
            up: true,
            size: 5.25,
            soften: 0.0,
            angle: 90.0,
            altitude: 30.0,
            use_global_light: false,
            gloss_contour: Contour::Linear,
            highlight: FxCommon::new(BlendMode::Screen, 0.75),
            highlight_color: Color::WHITE,
            shadow: FxCommon::new(BlendMode::Multiply, 0.75),
            shadow_color: Color::BLACK,
            contour: None,
            texture: None,
        }
    }
    #[test]
    fn smooth_all_styles_keep_legacy_height_lighting_and_mix_words() {
        for (w, h) in [(1, 1), (1, 19), (27, 1), (31, 23)] {
            let mut shape = Map::new(w, h, 0.0);
            for (i, v) in shape.v.iter_mut().enumerate() {
                *v = ((i * 73) % 101) as f32 / 100.0;
            }
            for style in [BevelStyle::InnerBevel, BevelStyle::OuterBevel, BevelStyle::Emboss, BevelStyle::PillowEmboss, BevelStyle::StrokeEmboss] {
                for soften in [0.0, 0.999, 1.0, 3.25] {
                    for up in [false, true] {
                        for global in [false, true] {
                            for contour in [None, Some(BevelContour { contour: Contour::Linear, range: 0.17, anti_alias: true })] {
                                let mut b = bevel(style);
                                b.soften = soften;
                                b.up = up;
                                b.use_global_light = global;
                                b.contour = contour;
                                if b.contour.is_some() {
                                    b.gloss_contour = Contour::Custom {
                                        name: String::new(),
                                        points: vec![
                                            photocraft_doc::adjust::CurvePoint { input: 0.0, output: 0.0 },
                                            photocraft_doc::adjust::CurvePoint { input: 0.45, output: 0.85 },
                                            photocraft_doc::adjust::CurvePoint { input: 1.0, output: 0.2 },
                                        ],
                                        corners: vec![false, true, false],
                                    };
                                }
                                let light = GlobalLight::default();
                                let tex =
                                    super::super::TextureCtx { rect: photocraft_geom::Rect::new(0, 0, w as i32, h as i32), patterns: &[], anchor: (0.0, 0.0) };
                                let patterns = super::super::PreparedPatterns::new(&[], super::super::PREPARED_PATTERN_BYTES);
                                let (expected, paint) = super::super::bevel_maps(&shape, &b, &light, &tex, &patterns);
                                let (got, actual_paint) = build(&shape, &b, &light, 64 << 20, &Interrupt::NONE).unwrap();
                                assert_eq!(actual_paint, paint);
                                for (a, b) in got.iter().zip(&expected) {
                                    assert_eq!(a.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), b.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sharp_refinement_and_chisel_fields_keep_legacy_words() {
        for diagonal in [false, true] {
            let mut shape = Map::new(37, 31, 0.0);
            for y in 0..shape.h {
                for x in 0..shape.w {
                    shape.v[y * shape.w + x] = if diagonal {
                        ((x as f32 + 0.375 * y as f32 - 12.37) * 0.8).clamp(0.0, 1.0)
                    } else {
                        if (8..29).contains(&x) && (7..24).contains(&y) { 0.75 } else { 0.0 }
                    };
                }
            }
            let old = super::super::sharp_edges(&shape);
            let got = sharp_edges(&shape, &Interrupt::NONE).unwrap();
            assert_eq!(old.slot, got.slot);
            assert_eq!(old.pixels.len(), got.pixels.len());
            for (a, b) in old.pixels.iter().zip(&got.pixels) {
                assert_eq!(a.at, b.at);
                assert_eq!(a.cov.to_bits(), b.cov.to_bits());
                assert_eq!(a.edge.offset.to_bits(), b.edge.offset.to_bits());
                assert_eq!(a.edge.normal.map(f32::to_bits), b.edge.normal.map(f32::to_bits));
            }
            for reach in [1.0, 7.25, 19.5] {
                for inner in [false, true] {
                    let expected =
                        if inner { super::super::bevel_dist_inside(&shape, &old, reach) } else { super::super::bevel_dist_outside(&shape, &old, reach) };
                    let actual = bevel_distance(&shape, &got, reach, inner, &Interrupt::NONE).unwrap();
                    assert_eq!(actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                }
            }
            for technique in [BevelTechnique::ChiselHard, BevelTechnique::ChiselSoft] {
                for style in [BevelStyle::InnerBevel, BevelStyle::OuterBevel, BevelStyle::Emboss, BevelStyle::PillowEmboss, BevelStyle::StrokeEmboss] {
                    for soften in [0.0, 1.0, 3.25] {
                        for up in [false, true] {
                            for contour in [
                                None,
                                Some(BevelContour::default()),
                                Some(BevelContour {
                                    contour: Contour::Custom {
                                        name: String::new(),
                                        points: vec![
                                            photocraft_doc::adjust::CurvePoint { input: 0.0, output: 0.0 },
                                            photocraft_doc::adjust::CurvePoint { input: 0.45, output: 0.85 },
                                            photocraft_doc::adjust::CurvePoint { input: 1.0, output: 0.2 },
                                        ],
                                        corners: vec![false, true, false],
                                    },
                                    range: 0.17,
                                    anti_alias: true,
                                }),
                            ] {
                                let mut b = bevel(style);
                                b.technique = technique;
                                b.soften = soften;
                                b.up = up;
                                b.contour = contour;
                                let light = GlobalLight::default();
                                let tex = super::super::TextureCtx { rect: photocraft_geom::Rect::new(0, 0, 37, 31), patterns: &[], anchor: (0.0, 0.0) };
                                let patterns = super::super::PreparedPatterns::new(&[], super::super::PREPARED_PATTERN_BYTES);
                                let (old, paint) = super::super::bevel_maps(&shape, &b, &light, &tex, &patterns);
                                let (actual, actual_paint) = build(&shape, &b, &light, 64 << 20, &Interrupt::NONE).unwrap();
                                assert_eq!(paint, actual_paint);
                                assert_eq!(old.len(), actual.len());
                                for (a, b) in old.iter().zip(&actual) {
                                    assert_eq!(a.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), b.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn checked_texture_keeps_pattern_placement_height_and_lighting_words() {
        let rect = photocraft_geom::Rect::new(-7, 11, 24, 34);
        let mut shape = Map::new(31, 23, 0.0);
        for (i, v) in shape.v.iter_mut().enumerate() {
            *v = ((i * 73) % 101) as f32 / 100.0;
        }
        for mode in [photocraft_doc::ColorMode::Rgb, photocraft_doc::ColorMode::Grayscale, photocraft_doc::ColorMode::Cmyk] {
            let mut source = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA32F);
            source.fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &[0.2, 0.7, 0.4, 0.75]);
            source.fill_rect(photocraft_geom::Rect::new(0, 2, 4, 4), &[0.9, 0.1, 0.6, 0.5]);
            let patterns = [photocraft_doc::Pattern::new("$$$/Patterns/test=Stripes", source, 4, 4)];
            let legacy = super::super::PreparedPatterns::new(&patterns, super::super::PREPARED_PATTERN_BYTES).in_mode(mode);
            let checked =
                super::super::PreparedPatterns::try_new_in_mode_with_interrupt(&patterns, super::super::PREPARED_PATTERN_BYTES, mode, &Interrupt::NONE)
                    .unwrap();
            checked.checked_requirements_with_interrupt(&Interrupt::NONE).unwrap();
            for technique in [BevelTechnique::Smooth, BevelTechnique::ChiselHard, BevelTechnique::ChiselSoft] {
                for link in [false, true] {
                    for invert in [false, true] {
                        for scale in [0.0, 0.5, 1.25] {
                            let mut b = bevel(BevelStyle::PillowEmboss);
                            b.technique = technique;
                            b.soften = 1.25;
                            b.contour = Some(BevelContour::default());
                            b.texture = Some(photocraft_doc::BevelTexture {
                                name: "stripes".into(),
                                id: String::new(),
                                scale,
                                depth: 0.7,
                                invert,
                                link,
                                phase: (1.25, -3.5),
                            });
                            let tex = super::super::TextureCtx { rect, patterns: &patterns, anchor: (-2.75, 17.5) };
                            let light = GlobalLight::default();
                            let (expected, paint) = super::super::bevel_maps(&shape, &b, &light, &tex, &legacy);
                            let (actual, actual_paint) = build_with_texture(&shape, &b, &light, Some((&tex, &checked)), 64 << 20, &Interrupt::NONE).unwrap();
                            assert_eq!(paint, actual_paint);
                            for (a, b) in actual.iter().zip(&expected) {
                                assert_eq!(a.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), b.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn private_failure_does_not_change_shape_or_admit_texture() {
        let shape = Map::new(64, 64, 0.5);
        let before = shape.v.clone();
        let mut b = bevel(BevelStyle::Emboss);
        assert!(matches!(build(&shape, &b, &GlobalLight::default(), 1, &Interrupt::NONE), Err(E::LimitExceeded)));
        b.texture =
            Some(photocraft_doc::BevelTexture { name: String::new(), id: String::new(), scale: 1.0, depth: 1.0, invert: false, link: true, phase: (0.0, 0.0) });
        assert!(matches!(build(&shape, &b, &GlobalLight::default(), 64 << 20, &Interrupt::NONE), Err(E::UnsupportedConversion)));
        b.texture = None;
        b.technique = BevelTechnique::ChiselHard;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ticks = AtomicUsize::new(0);
        let cancel = || ticks.fetch_add(1, Ordering::Relaxed) > 80;
        assert!(matches!(build(&shape, &b, &GlobalLight::default(), 64 << 20, &Interrupt::cancel_only(&cancel)), Err(E::Cancelled)));
        let mut light = GlobalLight::default();
        b.use_global_light = true;
        light.altitude = f32::NAN;
        assert!(matches!(build(&shape, &b, &light, 64 << 20, &Interrupt::NONE), Err(E::UnsupportedConversion)));
        assert_eq!(shape.v, before);
    }
}
