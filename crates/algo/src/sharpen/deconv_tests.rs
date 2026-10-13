use photocraft_color::ColorMode;

use super::*;

fn ctx(r: Rect, alpha: bool) -> Ctx {
    Ctx { bounds: r, mode: ColorMode::Grayscale, alpha }
}

/// Single-channel scene of hard edges: a bright box, a thin bar and a dot on a dark ground.
fn scene(w: usize, h: usize) -> Vec<f32> {
    let mut v = vec![0.1f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let box_ = (w / 8..w / 3).contains(&x) && (h / 4..h / 2).contains(&y);
            let bar = (w / 2..w / 2 + 2).contains(&x) && (h / 8..h - h / 8).contains(&y);
            let dot = (x as i32 - (3 * w / 4) as i32).pow(2) + (y as i32 - (h / 2) as i32).pow(2) < 9;
            if box_ || bar || dot {
                v[y * w + x] = 0.9;
            }
        }
    }
    v
}

/// `dst = src ⊛ taps` over a `w × h` plane, repeating the edge pixels: the direct (spatial)
/// reference the FFT path is checked against.
fn convolve(src: &[f32], w: usize, h: usize, taps: &[Tap], flip: bool, dst: &mut [f32]) {
    let sign = if flip { -1 } else { 1 };
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for &(dx, dy, k) in taps {
                let xx = (x as i32 + sign * dx).clamp(0, w as i32 - 1) as usize;
                let yy = (y as i32 + sign * dy).clamp(0, h as i32 - 1) as usize;
                acc += k * src[yy * w + xx];
            }
            dst[y * w + x] = acc;
        }
    }
}

/// Richardson–Lucy by direct convolution.
fn spatial_rl(obs: &[f32], w: usize, h: usize, taps: &[Tap], iters: usize) -> Vec<f32> {
    let mut est = obs.to_vec();
    let (mut blur, mut corr) = (vec![0.0; w * h], vec![0.0; w * h]);
    for _ in 0..iters {
        convolve(&est, w, h, taps, false, &mut blur);
        for (b, o) in blur.iter_mut().zip(obs) {
            *b = (o / b.max(EPS)).min(MAX_RATIO);
        }
        convolve(&blur, w, h, taps, true, &mut corr);
        for (e, k) in est.iter_mut().zip(&corr) {
            *e *= k.max(0.0);
        }
    }
    est
}

fn blurred(plane: &[f32], w: usize, h: usize, taps: &[Tap]) -> Vec<f32> {
    let mut out = vec![0.0; w * h];
    convolve(plane, w, h, taps, false, &mut out);
    out
}

fn mse(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / a.len() as f32
}

fn image(data: Vec<f32>, w: usize, h: usize, ch: usize) -> Image {
    Image { rect: Rect::new(0, 0, w as i32, h as i32), ch, data }
}

fn check_recovers(remove: SharpenRemove, radius: f32, angle: f32) {
    let (w, h) = (96, 64);
    let truth = scene(w, h);
    let taps = kernel(remove, radius, angle);
    let soft = blurred(&truth, w, h, &taps);
    let img = image(soft.clone(), w, h, 1);
    let sharp = deconvolve(&img, img.rect, &ctx(img.rect, false), &taps, iterations(false));
    let accurate = deconvolve(&img, img.rect, &ctx(img.rect, false), &taps, iterations(true));
    let (before, after, best) = (mse(&soft, &truth), mse(&sharp, &truth), mse(&accurate, &truth));
    assert!(after < before * 0.6, "{remove:?}: error {before} -> {after}");
    assert!(best < after, "{remove:?}: more iterations should recover more ({after} vs {best})");
}

#[test]
fn lens_deconvolution_recovers_a_defocused_scene() {
    check_recovers(SharpenRemove::Lens, 3.0, 0.0);
}

#[test]
fn motion_deconvolution_recovers_a_smeared_scene() {
    check_recovers(SharpenRemove::Motion, 4.0, 30.0);
    check_recovers(SharpenRemove::Motion, 5.0, 0.0);
}

#[test]
fn kernels_are_normalised_and_survive_hostile_radii() {
    for remove in [SharpenRemove::Lens, SharpenRemove::Motion] {
        for radius in [0.0, 0.1, 1.0, 7.3, 64.0, 1e9, f32::NAN, f32::INFINITY, -5.0] {
            let taps = kernel(remove, radius, f32::NAN);
            let sum: f32 = taps.iter().map(|t| t.2).sum();
            assert!((sum - 1.0).abs() < 1e-4, "{remove:?} {radius}: sum {sum}");
            assert!(taps.iter().all(|t| t.2.is_finite() && t.0.abs() <= 66 && t.1.abs() <= 66));
        }
    }
}

#[test]
fn a_flat_image_is_unchanged() {
    let (w, h) = (20, 16);
    let img = image(vec![0.4; w * h], w, h, 1);
    let taps = kernel(SharpenRemove::Lens, 2.0, 0.0);
    let out = deconvolve(&img, img.rect, &ctx(img.rect, false), &taps, 8);
    assert!(out.iter().all(|v| (v - 0.4).abs() < 1e-4));
}

#[test]
fn alpha_is_untouched_and_transparent_pixels_stay_clear() {
    let (w, h) = (24, 24);
    let plane = scene(w, h);
    let mut data = Vec::new();
    for (i, v) in plane.iter().enumerate() {
        data.extend_from_slice(&[*v, if i % w < 6 { 0.0 } else { 1.0 }]);
    }
    let img = image(data.clone(), w, h, 2);
    let taps = kernel(SharpenRemove::Lens, 1.5, 0.0);
    let out = deconvolve(&img, img.rect, &ctx(img.rect, true), &taps, 8);
    for (o, i) in out.as_chunks::<2>().0.iter().zip(data.as_chunks::<2>().0) {
        assert_eq!(o[1], i[1]);
        assert!(o[0].is_finite());
        if i[1] == 0.0 {
            assert_eq!(o[0], 0.0);
        }
    }
}

#[test]
fn a_tile_matches_the_whole_layer() {
    let (w, h) = (220usize, 40usize);
    let whole = image(blurred(&scene(w, h), w, h, &kernel(SharpenRemove::Lens, 2.0, 0.0)), w, h, 1);
    let taps = kernel(SharpenRemove::Lens, 2.0, 0.0);
    let iters = iterations(false);
    let full = deconvolve(&whole, whole.rect, &ctx(whole.rect, false), &taps, iters);
    let out = Rect::new(100, 0, 120, h as i32);
    let halo = reach(2.0, false);
    let window = Rect::new(out.x0 - halo, 0, out.x1 + halo, h as i32).intersect(&whole.rect);
    let sub = Image { rect: window, ch: 1, data: whole.crop(window) };
    let tile = deconvolve(&sub, out, &ctx(whole.rect, false), &taps, iters);
    for y in 0..h {
        for x in 0..20 {
            let (a, b) = (tile[y * 20 + x], full[y * w + 100 + x]);
            assert!((a - b).abs() < 1e-4, "({x},{y}): {a} vs {b}");
        }
    }
}

#[test]
fn the_fft_path_matches_direct_convolution_away_from_the_edges() {
    let (w, h) = (200usize, 120usize);
    for (remove, radius, angle) in [(SharpenRemove::Lens, 2.0, 0.0), (SharpenRemove::Motion, 3.0, 40.0)] {
        let taps = kernel(remove, radius, angle);
        let obs = blurred(&scene(w, h), w, h, &taps);
        let direct = spatial_rl(&obs, w, h, &taps, 8);
        let img = image(obs, w, h, 1);
        let fast = deconvolve(&img, img.rect, &ctx(img.rect, false), &taps, 8);
        let margin = 2 * 8 * (radius.ceil() as usize + 1);
        let mut worst = 0.0f32;
        for y in margin..h - margin {
            for x in margin..w - margin {
                worst = worst.max((direct[y * w + x] - fast[y * w + x]).abs());
            }
        }
        assert!(worst < 2e-3, "{remove:?}: worst difference {worst}");
    }
}

#[test]
fn the_full_radius_range_runs_and_still_sharpens() {
    let (w, h) = (160usize, 120usize);
    let truth = scene(w, h);
    for radius in [20.0, 40.0, 64.0] {
        let taps = kernel(SharpenRemove::Lens, radius, 0.0);
        let soft = blurred(&truth, w, h, &taps);
        let img = image(soft.clone(), w, h, 1);
        let out = deconvolve(&img, img.rect, &ctx(img.rect, false), &taps, 8);
        assert!(out.iter().all(|v| v.is_finite() && *v >= 0.0), "radius {radius}");
        assert!(mse(&out, &truth) < mse(&soft, &truth), "radius {radius}: no improvement");
    }
}

#[test]
fn blocks_agree_with_one_pass() {
    // A layer wider than one block: the seams must not show.
    let (w, h) = (2600usize, 40usize);
    let taps = kernel(SharpenRemove::Lens, 2.0, 0.0);
    let obs = blurred(&scene(w, h), w, h, &taps);
    let img = image(obs.clone(), w, h, 1);
    let blocked = deconvolve(&img, img.rect, &ctx(img.rect, false), &taps, 8);
    let direct = spatial_rl(&obs, w, h, &taps, 8);
    let margin = 60;
    let worst =
        (0..h).flat_map(|y| (margin..w - margin).map(move |x| (x, y))).map(|(x, y)| (blocked[y * w + x] - direct[y * w + x]).abs()).fold(0.0f32, f32::max);
    assert!(worst < 5e-3, "worst block-seam difference {worst}");
}
