//! From a model's soft matte to a selection.
//!
//! Learned salient-object models (see `photocraft-ml`) answer with a low-resolution alpha map.
//! [`region_from_matte`] stretches it over the canvas with bilinear interpolation, tightens the
//! soft ramp a little so the result reads as a selection rather than a haze (hair and fur still
//! keep partial coverage), and crops it to the pixels it touches.

use photocraft_geom::Rect;

use super::Region;

/// Alpha below this is background, above `1 - CUT` is fully selected; between is a smooth ramp.
const CUT: f32 = 0.12;

/// Pixels with at least this coverage count towards the bounding box (a faint haze doesn't).
const MIN_COVER: u8 = 3;

/// Stretch `matte` (`mw x mh`, row-major, `0..=1`) over `canvas` and return it as a [`Region`].
/// `None` when the matte is empty or malformed, or selects nothing.
pub fn region_from_matte(canvas: Rect, matte: &[f32], mw: usize, mh: usize) -> Option<Region> {
    let (w, h) = (usize::try_from(canvas.width()).ok()?, usize::try_from(canvas.height()).ok()?);
    if w == 0 || h == 0 || mw == 0 || mh == 0 || matte.len() != mw.checked_mul(mh)? {
        return None;
    }
    // Per-column source index and weight, shared by every row.
    let cols: Vec<(usize, usize, f32)> = (0..w)
        .map(|x| {
            let f = ((x as f32 + 0.5) * mw as f32 / w as f32 - 0.5).clamp(0.0, (mw - 1) as f32);
            let x0 = f.floor() as usize;
            (x0, (x0 + 1).min(mw - 1), f - x0 as f32)
        })
        .collect();
    let ramp = |v: f32| {
        let t = ((v - CUT) / (1.0 - 2.0 * CUT)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mut full = vec![0u8; w.checked_mul(h)?];
    let (mut x_min, mut y_min, mut x_max, mut y_max) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (y, row) in full.chunks_exact_mut(w).enumerate() {
        let f = ((y as f32 + 0.5) * mh as f32 / h as f32 - 0.5).clamp(0.0, (mh - 1) as f32);
        let y0 = f.floor() as usize;
        let (y1, ty) = ((y0 + 1).min(mh - 1), f - y0 as f32);
        let (r0, r1) = (matte.get(y0 * mw..(y0 + 1) * mw)?, matte.get(y1 * mw..(y1 + 1) * mw)?);
        for (x, out) in row.iter_mut().enumerate() {
            let &(x0, x1, tx) = cols.get(x)?;
            let top = r0[x0] + (r0[x1] - r0[x0]) * tx;
            let bot = r1[x0] + (r1[x1] - r1[x0]) * tx;
            let v = ramp(top + (bot - top) * ty);
            let c = (v * 255.0).round() as u8;
            *out = c;
            if c >= MIN_COVER {
                x_min = x_min.min(x);
                x_max = x_max.max(x);
                y_min = y_min.min(y);
                y_max = y_max.max(y);
            }
        }
    }
    if x_min == usize::MAX {
        return None;
    }
    let bw = x_max - x_min + 1;
    let mut mask = Vec::with_capacity(bw * (y_max - y_min + 1));
    for y in y_min..=y_max {
        mask.extend_from_slice(full.get(y * w + x_min..y * w + x_min + bw)?);
    }
    let bbox = Rect::new(canvas.x0 + x_min as i32, canvas.y0 + y_min as i32, canvas.x0 + x_max as i32 + 1, canvas.y0 + y_max as i32 + 1);
    Some(Region { bbox, mask })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc_matte(n: usize) -> Vec<f32> {
        (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as f32 - n as f32 / 2.0, (i / n) as f32 - n as f32 / 2.0);
                let d = (x * x + y * y).sqrt() / (n as f32 * 0.25);
                (1.5 - d).clamp(0.0, 1.0)
            })
            .collect()
    }

    #[test]
    fn disc_matte_becomes_a_centred_region() {
        let canvas = Rect::new(10, 20, 210, 220);
        let r = region_from_matte(canvas, &disc_matte(64), 64, 64).unwrap();
        assert!(r.bbox.x0 > canvas.x0 + 20 && r.bbox.x1 < canvas.x1 - 20, "{:?}", r.bbox);
        assert_eq!(r.mask.len(), (r.bbox.width() * r.bbox.height()) as usize);
        assert!(r.at(110, 120) > 0.99);
        assert!(r.at(11, 21) == 0.0);
    }

    #[test]
    fn empty_or_malformed_mattes_select_nothing() {
        let canvas = Rect::new(0, 0, 40, 40);
        assert!(region_from_matte(canvas, &[0.0; 16], 4, 4).is_none());
        assert!(region_from_matte(canvas, &[1.0; 15], 4, 4).is_none());
        assert!(region_from_matte(canvas, &[], 0, 0).is_none());
        assert!(region_from_matte(Rect::new(0, 0, 0, 0), &[1.0; 4], 2, 2).is_none());
    }

    #[test]
    fn full_matte_selects_the_whole_canvas() {
        let canvas = Rect::new(-5, -5, 15, 25);
        let r = region_from_matte(canvas, &[1.0; 9], 3, 3).unwrap();
        assert_eq!(r.bbox, canvas);
        assert!(r.mask.iter().all(|v| *v == 255));
    }
}
