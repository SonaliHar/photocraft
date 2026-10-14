//! Same Object Finder smoothing/growth/seed recipe; no semantic segmenter admission.
use super::*;
fn box3(px: &[[f32; 3]], w: usize, h: usize, b: &Budget<'_, '_>) -> Result<Vec<[f32; 3]>, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if px.len() != n {
        return Err(E::LengthMismatch);
    }
    b.ensure_work((n as u64).checked_mul(10).ok_or(E::LimitExceeded)?)?;
    let mut out = b.reserve(n)?;
    for y in 0..h {
        for x in 0..w {
            let (mut acc, mut count) = ([0f32; 3], 0f32);
            for yy in y.saturating_sub(1)..y.saturating_add(2).min(h) {
                for xx in x.saturating_sub(1)..x.saturating_add(2).min(w) {
                    b.work(1)?;
                    let p = *px.get(index(xx, yy, w)?).ok_or(E::LengthMismatch)?;
                    acc = [acc[0] + p[0], acc[1] + p[1], acc[2] + p[2]];
                    count += 1.;
                }
            }
            out.push(acc.map(|v| v / count.max(1.)));
        }
    }
    Ok(out)
}
fn grow(lab: &[[f32; 3]], w: usize, h: usize, sx: usize, sy: usize, b: &Budget<'_, '_>) -> Result<Vec<bool>, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    b.ensure_work((n as u64).checked_mul(64).ok_or(E::LimitExceeded)?)?;
    let mut out = b.filled(n, false)?;
    let seed = *lab.get(index(sx, sy, w)?).ok_or(E::LengthMismatch)?;
    let (mut sum, mut count) = ([f64::from(seed[0]), f64::from(seed[1]), f64::from(seed[2])], 1f64);
    let mut stack = b.reserve(n)?;
    stack.push((sx, sy));
    *out.get_mut(index(sx, sy, w)?).ok_or(E::LengthMismatch)? = true;
    let d = |a: [f32; 3], c: [f32; 3]| ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2) + (a[2] - c[2]).powi(2)).sqrt();
    while let Some((x, y)) = stack.pop() {
        b.work(1)?;
        let c = *lab.get(index(x, y, w)?).ok_or(E::LengthMismatch)?;
        let mean = [(sum[0] / count) as f32, (sum[1] / count) as f32, (sum[2] / count) as f32];
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            b.work(16)?;
            if nx >= w || ny >= h {
                continue;
            }
            let i = index(nx, ny, w)?;
            if *out.get(i).ok_or(E::LengthMismatch)? {
                continue;
            }
            let q = *lab.get(i).ok_or(E::LengthMismatch)?;
            if d(q, c) <= 10. && d(q, mean) <= 32. {
                *out.get_mut(i).ok_or(E::LengthMismatch)? = true;
                sum = [sum[0] + f64::from(q[0]), sum[1] + f64::from(q[1]), sum[2] + f64::from(q[2])];
                count += 1.;
                stack.push((nx, ny));
            }
        }
    }
    Ok(out)
}
// One seed keeps precisely its 8-connected component, matching keep_seeded.
fn keep_seed(low: &[bool], w: usize, h: usize, seed: usize, b: &Budget<'_, '_>) -> Result<Vec<bool>, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if low.len() != n {
        return Err(E::LengthMismatch);
    }
    b.ensure_work((n as u64).checked_mul(12).ok_or(E::LimitExceeded)?)?;
    let mut out = b.filled(n, false)?;
    if !*low.get(seed).ok_or(E::LengthMismatch)? {
        return Ok(out);
    }
    let mut stack = b.reserve(n)?;
    stack.push(seed);
    *out.get_mut(seed).ok_or(E::LengthMismatch)? = true;
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                b.work(1)?;
                let nx = x as i64 + dx;
                let ny = y as i64 + dy;
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = index(nx as usize, ny as usize, w)?;
                if *low.get(j).ok_or(E::LengthMismatch)? && !*out.get(j).ok_or(E::LengthMismatch)? {
                    *out.get_mut(j).ok_or(E::LengthMismatch)? = true;
                    stack.push(j);
                }
            }
        }
    }
    Ok(out)
}
pub fn point_requirements(canvas: Rect, x: f32, y: f32, work_px: usize) -> Result<ObjectSelectionRequirements, E> {
    if !x.is_finite() || !y.is_finite() {
        return Err(E::InvalidSample);
    }
    if canvas.width() < 4 || canvas.height() < 4 || !canvas.contains(x.floor() as i32, y.floor() as i32) {
        return Ok(ObjectSelectionRequirements { window: Rect::EMPTY, step: 1, peak_resident_bytes: 0, work_units: 1 });
    }
    let mut r = requirements(canvas, canvas, work_px.max(1024))?;
    let n = (canvas.width() as usize)
        .div_ceil(r.step)
        .try_into()
        .ok()
        .and_then(|w: u64| w.checked_mul((canvas.height() as usize).div_ceil(r.step) as u64))
        .ok_or(E::LimitExceeded)?;
    r.peak_resident_bytes = r.peak_resident_bytes.checked_add(n.checked_mul(64).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    r.work_units = r.work_units.checked_add(n.checked_mul(256).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    Ok(r)
}
pub fn object_at_with_interrupt(
    sampler: &dyn Sampler,
    canvas: Rect,
    x: f32,
    y: f32,
    work_px: usize,
    limits: ObjectSelectionLimits,
    control: &Interrupt<'_>,
) -> Result<Option<Region>, E> {
    control.check().map_err(|_| E::Cancelled)?;
    let defaults = ObjectSelectionLimits::default();
    if limits.resident_bytes > defaults.resident_bytes || limits.work_units > defaults.work_units {
        return Err(E::LimitExceeded);
    }
    let req = point_requirements(canvas, x, y, work_px)?;
    if req.window.is_empty() {
        return Ok(None);
    }
    if req.peak_resident_bytes > limits.resident_bytes || req.work_units > limits.work_units {
        return Err(E::LimitExceeded);
    }
    let b = Budget { left: Cell::new(limits.work_units), control };
    let step = req.step;
    let img = scaled(sampler, canvas, step, &b)?;
    let (w, h) = (img.w, img.h);
    if w < 2 || h < 2 {
        return Ok(None);
    }
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    b.ensure_work((n as u64).checked_mul(128).ok_or(E::LimitExceeded)?)?;
    let first = box3(&img.px, w, h, &b)?;
    let second = box3(&first, w, h, &b)?;
    drop(first);
    let mut lab = b.reserve(n)?;
    for c in second {
        b.work(64)?;
        lab.push(crate::segment::slic::rgb_to_lab(c));
    }
    let sx = (((i64::from(x.floor() as i32) - i64::from(canvas.x0)) as usize) / step).min(w - 1);
    let sy = (((i64::from(y.floor() as i32) - i64::from(canvas.y0)) as usize) / step).min(h - 1);
    let grown = grow(&lab, w, h, sx, sy, &b)?;
    drop(lab);
    let count = grown.iter().try_fold(0usize, |n, v| {
        b.work(1)?;
        Ok::<_, E>(n + usize::from(*v))
    })?;
    if count.checked_mul(10).ok_or(E::LimitExceeded)? > n.checked_mul(6).ok_or(E::LimitExceeded)? || count < 4 {
        return Ok(None);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for (i, v) in grown.iter().enumerate() {
        b.work(1)?;
        if *v {
            let (gx, gy) = (i % w, i / w);
            x0 = x0.min(gx);
            y0 = y0.min(gy);
            x1 = x1.max(gx + 1);
            y1 = y1.max(gy + 1);
        }
    }
    let pad = ((x1 - x0).max(y1 - y0) / 6).max(2);
    let (bx0, by0, bx1, by1) = (x0.saturating_sub(pad), y0.saturating_sub(pad), (x1 + pad).min(w), (y1 + pad).min(h));
    let mut trimap = b.filled(n, BG)?;
    let mut any_bg = false;
    for yy in 0..h {
        for xx in 0..w {
            b.work(1)?;
            let i = index(xx, yy, w)?;
            let inside = xx >= bx0 && xx < bx1 && yy >= by0 && yy < by1;
            *trimap.get_mut(i).ok_or(E::LengthMismatch)? = if *grown.get(i).ok_or(E::LengthMismatch)? {
                PR_FG
            } else if inside {
                PR_BG
            } else {
                any_bg = true;
                BG
            };
        }
    }
    if !any_bg {
        for yy in 0..h {
            for xx in 0..w {
                b.work(1)?;
                if (xx == 0 || yy == 0 || xx + 1 == w || yy + 1 == h) && !(xx == sx && yy == sy) {
                    *trimap.get_mut(index(xx, yy, w)?).ok_or(E::LengthMismatch)? = BG;
                }
            }
        }
    }
    let seed = index(sx, sy, w)?;
    *trimap.get_mut(seed).ok_or(E::LengthMismatch)? = super::super::FG;
    drop(grown);
    let Some((fg, bg)) = iterations(&img, &mut trimap, 5, &b)? else { return Ok(None) };
    let mut low = b.reserve(n)?;
    for t in &trimap {
        b.work(1)?;
        low.push(is_fg(*t));
    }
    let low = keep_seed(&low, w, h, seed, &b)?;
    let cleanup_work = (n as u64).checked_mul(192).ok_or(E::LimitExceeded)?;
    b.work(cleanup_work)?;
    let low = crate::segment::clean_mask_with_interrupt(
        &low,
        w,
        h,
        0.1,
        0.01,
        crate::segment::CleanupLimits { resident_bytes: limits.resident_bytes, work_units: cleanup_work },
        control,
    )?;
    let found = low.iter().try_fold(false, |found, v| {
        b.work(1)?;
        Ok::<_, E>(found || *v)
    })?;
    if !found {
        return Ok(None);
    }
    let result = finish(sampler, canvas, step, &low, w, h, &fg, &bg, &b)?;
    b.work(1)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seed_component_matches_baseline() {
        let mask = [true, false, false, true, true, false, false, false, true];
        let b = Budget { left: Cell::new(100_000), control: &Interrupt::NONE };
        let mut seeds = vec![false; 9];
        seeds[0] = true;
        assert_eq!(keep_seed(&mask, 3, 3, 0, &b).unwrap(), crate::segment::keep_seeded(&mask, &seeds, 3, 3));
    }
    #[test]
    fn actual_point_disc_and_background_match_baseline() {
        let image = crate::segment::RgbImage::from_fn(80, 64, |x, y| {
            if (x as i32 - 40).pow(2) + (y as i32 - 32).pow(2) < 18 * 18 { [0.9, 0.1, 0.1] } else { [0.1, 0.3, 0.8] }
        });
        let source = crate::segment::ImageSampler { img: &image, origin: (0, 0) };
        let canvas = Rect::from_xywh(0, 0, 80, 64);
        for (x, y) in [(40., 32.), (1., 1.)] {
            assert_eq!(
                object_at_with_interrupt(&source, canvas, x, y, 60_000, ObjectSelectionLimits::default(), &Interrupt::NONE).unwrap(),
                super::super::super::try_object_at(&source, canvas, x, y, 60_000).unwrap()
            );
        }
    }
}
