//! A reduced-resolution development, for showing a photo while it is being browsed.
//!
//! A Camera Raw dialog shows a picture of a few thousand pixels at most, not 45 million. Demosaicing the whole sensor only to
//! average it down again costs seconds. [`develop_reduced`] instead averages each colour's samples inside blocks of the sensor
//! into a small Bayer mosaic and hands that to [`develop_sensor`]: the development itself (white balance, the camera look or
//! calibration, exposure, tone, gamma, orientation) is the full one's, unchanged, so the two cannot drift apart.

use crate::develop::{DevelopOptions, Developed, develop_sensor};
use crate::error::{RawError, Result};
use crate::par;
use crate::sensor::{BlackLevels, Cfa, Rect, Sensor};

/// A reduced development and the size of the full one it stands in for.
#[derive(Debug, Clone)]
pub struct ReducedDeveloped {
    pub developed: Developed,
    /// Width and height of the full-resolution development (after orientation) this is a reduction of.
    pub full_width: u32,
    pub full_height: u32,
}

/// How many times smaller the reduced picture is: the largest whole factor that leaves the longer side at least
/// `target_long_edge` pixels.
fn factor(long_side: usize, target_long_edge: usize) -> usize {
    (long_side / target_long_edge.max(1)).max(1)
}

/// Develops `s` at a fraction of its size: the longer side comes out at least `target_long_edge` pixels.
///
/// Only plain Bayer sensors at least twice `target_long_edge` long are reduced (no black-level deltas, no lens-shading gain
/// maps: both are indexed by sensor position); anything else is refused with [`RawError`], and the caller develops it in full.
pub fn develop_reduced(s: &Sensor, opts: &DevelopOptions, target_long_edge: usize) -> Result<ReducedDeveloped> {
    let c = s.crop;
    if c.is_empty() || c.x + c.width > s.width || c.y + c.height > s.height || s.data.len() < s.width * s.height * s.samples {
        return Err(RawError::malformed("crop lies outside the sensor data"));
    }
    let (Some(cfa), 1) = (&s.cfa, s.samples) else {
        return Err(RawError::unsupported("a reduced development needs a one-sample colour filter array"));
    };
    if !cfa.is_bayer() {
        return Err(RawError::unsupported("a reduced development needs a Bayer sensor"));
    }
    let bl = &s.black;
    if !bl.delta_h.is_empty() || !bl.delta_v.is_empty() || !matches!(bl.rows, 1 | 2) || !matches!(bl.cols, 1 | 2) || !s.gain_maps.is_empty() {
        return Err(RawError::unsupported("black levels or gain maps that vary across the sensor"));
    }
    // Only where it pays: a photo under twice the target is cheap to develop in full.
    let k = factor(c.width.max(c.height), target_long_edge);
    if k < 2 {
        return Err(RawError::unsupported("small enough to develop in full"));
    }
    // A block of f × f sensor pixels becomes one 2 × 2 cell of the reduced mosaic; f is even, so every block starts on the
    // same colours and the same black levels.
    let f = 2 * k;
    let (bw, bh) = (c.width / f, c.height / f);
    if bw == 0 || bh == 0 {
        return Err(RawError::malformed("the sensor is too small to reduce"));
    }
    let (w, h) = (2 * bw, 2 * bh);
    opts.limits.check(w as u64, h as u64, 2)?;
    let lin = s.linearization.as_deref();
    let linear = |raw: u16| lin.map_or(raw, |l| l.get(usize::from(raw)).or_else(|| l.last()).copied().unwrap_or(raw));
    let samples = (k * k) as u64;
    let mut data = vec![0u16; w * h];
    // One row of blocks (two mosaic rows) per task.
    par::chunks_mut(&mut data, 2 * w, |by, out| {
        let mut sums = vec![0u64; 2 * w];
        for dy in 0..f {
            let start = (c.y + by * f + dy) * s.width + c.x;
            let src = &s.data[start..start + bw * f];
            let row = &mut sums[(dy % 2) * w..(dy % 2 + 1) * w];
            for (cell, block) in row.as_chunks_mut::<2>().0.iter_mut().zip(src.chunks_exact(f)) {
                for pair in block.as_chunks::<2>().0 {
                    cell[0] += u64::from(linear(pair[0]));
                    cell[1] += u64::from(linear(pair[1]));
                }
            }
        }
        for (o, sum) in out.iter_mut().zip(&sums) {
            *o = ((sum + samples / 2) / samples) as u16;
        }
    });

    let (ox, oy) = (c.x.saturating_sub(s.active.x), c.y.saturating_sub(s.active.y));
    let black = BlackLevels {
        rows: 2,
        cols: 2,
        values: vec![bl.at(ox, oy, 0, 1), bl.at(ox + 1, oy, 0, 1), bl.at(ox, oy + 1, 0, 1), bl.at(ox + 1, oy + 1, 0, 1)],
        delta_h: Vec::new(),
        delta_v: Vec::new(),
    };
    // Field by field (not `..s.clone()`, which would copy the full sensor data).
    let small = Sensor {
        format: s.format,
        make: s.make.clone(),
        model: s.model.clone(),
        width: w,
        height: h,
        samples: 1,
        data,
        cfa: Some(Cfa { width: 2, height: 2, colors: cfa.phase(c.x, c.y).to_vec(), origin_x: 0, origin_y: 0 }),
        linearization: None,
        black,
        white: s.white,
        active: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        color: s.color.clone(),
        camera_wb: s.camera_wb,
        orientation: s.orientation,
        baseline_exposure: s.baseline_exposure,
        gain_maps: Vec::new(),
        tone_curve: s.tone_curve.clone(),
        black_point: s.black_point,
        warnings: s.warnings.clone(),
    };
    let developed = develop_sensor(&small, opts)?;
    let (full_width, full_height) = if opts.orient && (5..=8).contains(&s.orientation) { (c.height, c.width) } else { (c.width, c.height) };
    Ok(ReducedDeveloped { developed, full_width: full_width as u32, full_height: full_height as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WhiteBalance;

    /// A smooth camera-space scene: gradients and a gentle swirl, so box averages and demosaiced pixels agree closely.
    fn smooth(w: usize, h: usize) -> Vec<[f32; 3]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                [0.15 + 0.55 * x, 0.2 + 0.5 * y, 0.15 + 0.3 * (0.5 + 0.5 * ((x + y) * 5.0).sin())]
            })
            .collect()
    }

    fn sensor(w: usize, h: usize) -> Sensor {
        let data = crate::testgen::mosaic(&smooth(w, h), w, [0, 1, 1, 2], 0, 4095);
        let bytes = crate::testgen::tiff_ep("NIKON CORPORATION", w, h, &data, [0, 1, 1, 2], 12, vec![]);
        crate::decode(&bytes, &crate::Limits::default()).unwrap()
    }

    /// Mean absolute difference, in 16-bit codes, between the reduced image and the full one averaged down by `f`.
    fn mean_difference(full: &Developed, reduced: &Developed, f: usize) -> f64 {
        let (fw, fh) = (full.width as usize, full.height as usize);
        let (rw, rh) = (reduced.width as usize, reduced.height as usize);
        let mut total = 0.0;
        for y in 0..rh {
            for x in 0..rw {
                for k in 0..3 {
                    let mut sum = 0.0;
                    for dy in 0..f {
                        for dx in 0..f {
                            sum += f64::from(full.rgb[(((y * f + dy).min(fh - 1)) * fw + (x * f + dx).min(fw - 1)) * 3 + k]);
                        }
                    }
                    total += (sum / (f * f) as f64 - f64::from(reduced.rgb[(y * rw + x) * 3 + k])).abs();
                }
            }
        }
        total / (rw * rh * 3) as f64
    }

    #[test]
    fn the_factor_keeps_the_target() {
        assert_eq!(factor(8256, 1800), 4);
        assert_eq!(factor(6000, 1800), 3);
        assert_eq!(factor(3000, 1800), 1);
        assert_eq!(factor(100, 1000), 1);
        assert_eq!(factor(5000, 0), 5000);
    }

    #[test]
    fn it_agrees_with_the_full_development_averaged_down() {
        let (w, h) = (480, 320);
        let s = sensor(w, h);
        let opts = DevelopOptions { white_balance: WhiteBalance::Multipliers([1.4, 1.0, 1.2]), ..Default::default() };
        let full = develop_sensor(&s, &opts).unwrap();
        let reduced = develop_reduced(&s, &opts, 100).unwrap();
        let f = factor(w, 100);
        assert_eq!((reduced.developed.width as usize, reduced.developed.height as usize), (w / f, h / f));
        assert_eq!((reduced.full_width, reduced.full_height), (full.width, full.height));
        let difference = mean_difference(&full, &reduced.developed, f);
        assert!(difference < 250.0, "mean difference {difference:.0} codes of 65535");
        assert_eq!(reduced.developed.info.wb_multipliers, full.info.wb_multipliers);
    }

    #[test]
    fn exposure_and_white_balance_still_act() {
        let s = sensor(240, 160);
        let mean =
            |d: &ReducedDeveloped, k: usize| d.developed.rgb.iter().skip(k).step_by(3).map(|v| f64::from(*v)).sum::<f64>() / (d.developed.rgb.len() / 3) as f64;
        let base = develop_reduced(&s, &DevelopOptions::default(), 60).unwrap();
        let brighter = develop_reduced(&s, &DevelopOptions { exposure: 1.0, ..Default::default() }, 60).unwrap();
        assert!(mean(&brighter, 1) > mean(&base, 1) * 1.2, "{} vs {}", mean(&brighter, 1), mean(&base, 1));
        let warm = develop_reduced(&s, &DevelopOptions { white_balance: WhiteBalance::Multipliers([1.6, 1.0, 0.65]), ..Default::default() }, 60).unwrap();
        let (before, after) = (mean(&base, 0) / mean(&base, 2), mean(&warm, 0) / mean(&warm, 2));
        assert!(after > before * 1.3, "red/blue {before:.3} -> {after:.3}");
    }

    #[test]
    fn orientation_swaps_the_full_size_too() {
        let mut s = sensor(240, 160);
        s.orientation = 6;
        let reduced = develop_reduced(&s, &DevelopOptions::default(), 60).unwrap();
        let (rw, rh) = (reduced.developed.width, reduced.developed.height);
        assert!(rh > rw, "the picture is turned upright: {rw}x{rh}");
        assert_eq!((reduced.full_width, reduced.full_height), (160, 240));
        let unturned = develop_reduced(&s, &DevelopOptions { orient: false, ..Default::default() }, 60).unwrap();
        assert_eq!((unturned.full_width, unturned.full_height), (240, 160));
    }

    #[test]
    fn what_cannot_be_reduced_is_refused_for_the_caller_to_develop_in_full() {
        let mut no_cfa = sensor(240, 160);
        no_cfa.cfa = None;
        assert!(develop_reduced(&no_cfa, &DevelopOptions::default(), 60).is_err());
        let mut linear = sensor(240, 160);
        linear.samples = 3;
        assert!(develop_reduced(&linear, &DevelopOptions::default(), 60).is_err());
        let mut tiny = sensor(240, 160);
        tiny.crop = crate::sensor::Rect::new(0, 0, 1, 1);
        assert!(develop_reduced(&tiny, &DevelopOptions::default(), 60).is_err());
        let mut outside = sensor(240, 160);
        outside.crop = crate::sensor::Rect::new(100, 100, 400, 400);
        assert!(develop_reduced(&outside, &DevelopOptions::default(), 60).is_err());
    }

    #[test]
    fn the_whole_crop_is_used_not_just_a_multiple_of_the_block() {
        // 250 x 170 with 8-pixel blocks: the right and bottom remainders are dropped, never read past.
        let s = sensor(250, 170);
        let reduced = develop_reduced(&s, &DevelopOptions::default(), 60).unwrap();
        let f = 2 * factor(250, 60);
        assert_eq!((reduced.developed.width as usize, reduced.developed.height as usize), (2 * (250 / f), 2 * (170 / f)));
    }
}
