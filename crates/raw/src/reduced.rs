//! A reduced-resolution development, for showing a photo while it is being browsed.
//!
//! A Camera Raw dialog shows a picture of a few thousand pixels at most, not 45 million. Demosaicing the whole sensor only to
//! average it down again costs seconds; here every output pixel is the plain average of each colour's samples inside a block of
//! the sensor ([`develop_reduced`]): one pass over the data, no interpolation, and so no demosaicing artefacts. The colour
//! handling is the full development's own (white balance, the camera look or calibration, exposure, tone, gamma, orientation):
//! the same code, not a copy of it, so the two cannot drift apart.

use crate::develop::{DevelopOptions, Developed, RowNormalizer, Scale, color_setup, finish};
use crate::error::{RawError, Result};
use crate::par;
use crate::sensor::Sensor;

/// A reduced development and the size of the full one it stands in for.
#[derive(Debug, Clone)]
pub struct ReducedDeveloped {
    pub developed: Developed,
    /// Width and height of the full-resolution development (after orientation) this is a reduction of.
    pub full_width: u32,
    pub full_height: u32,
}

/// The block size (an even number of sensor pixels, so every block holds the same colours) that leaves the longer side at
/// least `target_long_edge` pixels.
fn block_size(long_side: usize, target_long_edge: usize) -> usize {
    (2 * (long_side / (2 * target_long_edge.max(1)))).max(2)
}

/// Develops `s` at a fraction of its size: the longer side comes out at least `target_long_edge` pixels where the sensor allows.
///
/// Only Bayer sensors at least twice `target_long_edge` long are reduced; anything else is refused with [`RawError`], and the
/// caller develops it in full.
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
    // Only where it pays: a photo under twice the target is cheap to develop in full, and halving it would leave less than asked.
    if c.width.max(c.height) < 2 * target_long_edge.max(1) {
        return Err(RawError::unsupported("small enough to develop in full"));
    }
    let f = block_size(c.width.max(c.height), target_long_edge);
    let (bw, bh) = (c.width / f, c.height / f);
    if bw == 0 || bh == 0 {
        return Err(RawError::malformed("the sensor is too small to reduce"));
    }
    opts.limits.check(bw as u64, bh as u64, 12 + 12 + 6)?;
    let mut warnings = s.warnings.clone();
    let sc = Scale::new(s);
    let setup = color_setup(s, opts, &sc, &mut warnings)?;

    // The colours of the 2×2 cell at the crop's origin; every block starts on one (f is even).
    let phase = cfa.phase(c.x, c.y);
    let pattern = phase.iter().map(|c| ['R', 'G', 'B'][usize::from(*c).min(2)]).collect::<String>();
    // Each of the four cells appears f²/4 times in a block.
    let mut count = [0.0f32; 3];
    for ch in phase {
        count[usize::from(ch).min(2)] += (f * f / 4) as f32;
    }
    let inverse = count.map(|n| if n > 0.0 { 1.0 / n } else { 0.0 });
    let normalizer = RowNormalizer::new(s, &sc, setup.mult, cfa);
    let mut rgb = vec![0.0f32; bw * bh * 3];
    // One output row per task: it averages f source rows.
    par::chunks_mut(&mut rgb, bw * 3, |by, out| {
        let mut line = vec![0.0f32; bw * f];
        let mut acc = vec![[0.0f32; 3]; bw];
        for dy in 0..f {
            normalizer.fill(c.y + by * f + dy, &mut line);
            let (even, odd) = (usize::from(phase[(dy % 2) * 2]).min(2), usize::from(phase[(dy % 2) * 2 + 1]).min(2));
            for (cell, block) in acc.iter_mut().zip(line.chunks_exact(f)) {
                let (mut a, mut b) = (0.0f32, 0.0f32);
                for pair in block.as_chunks::<2>().0 {
                    a += pair[0];
                    b += pair[1];
                }
                cell[even] += a;
                cell[odd] += b;
            }
        }
        for (px, cell) in out.as_chunks_mut::<3>().0.iter_mut().zip(&acc) {
            for k in 0..3 {
                px[k] = cell[k] * inverse[k];
            }
        }
    });

    let (full_width, full_height) = if opts.orient && (5..=8).contains(&s.orientation) { (c.height, c.width) } else { (c.width, c.height) };
    let developed = finish(s, opts, rgb, (bw, bh), &setup, Some(pattern), warnings);
    Ok(ReducedDeveloped { developed, full_width: full_width as u32, full_height: full_height as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WhiteBalance, develop_sensor};

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
    fn blocks_are_even_and_keep_the_target() {
        assert_eq!(block_size(8256, 1800), 4);
        assert_eq!(block_size(6000, 1800), 2);
        assert_eq!(block_size(3000, 1800), 2);
        assert_eq!(block_size(100, 1000), 2);
        assert_eq!(block_size(5000, 0), 5000);
        for long in [4000, 5472, 6000, 8256, 9504] {
            let f = block_size(long, 1800);
            assert!(f.is_multiple_of(2) && (long / f >= 1800 || f == 2), "{long}: {f}");
        }
    }

    #[test]
    fn it_agrees_with_the_full_development_averaged_down() {
        let (w, h) = (480, 320);
        let s = sensor(w, h);
        let opts = DevelopOptions { white_balance: WhiteBalance::Multipliers([1.4, 1.0, 1.2]), ..Default::default() };
        let full = develop_sensor(&s, &opts).unwrap();
        let reduced = develop_reduced(&s, &opts, 100).unwrap();
        let f = block_size(w, 100);
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
        // 250 x 170 with 4-pixel blocks: the right and bottom remainders are dropped, never read past.
        let s = sensor(250, 170);
        let reduced = develop_reduced(&s, &DevelopOptions::default(), 60).unwrap();
        let f = block_size(250, 60);
        assert_eq!((reduced.developed.width as usize, reduced.developed.height as usize), (250 / f, 170 / f));
    }
}
