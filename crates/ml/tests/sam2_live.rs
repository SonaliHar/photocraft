//! Runs the real SAM 2 model. Opt-in: needs the ML assets (see `docs/ml.md`) and is skipped
//! otherwise. `cargo test -p photocraft-ml --test sam2_live -- --ignored --nocapture`
//! (set `PHOTOCRAFT_ML_DIR` if they are not in the default folder).

use std::time::Instant;

use photocraft_ml::{Assets, ModelSize, Prompt, Sam2};

/// A noisy grey-blue backdrop with a red disc at (300, 220) of radius 110 and a green square.
fn scene(w: usize, h: usize) -> (Vec<[f32; 3]>, impl Fn(usize, usize) -> bool) {
    let disc = |x: usize, y: usize| ((x as f32 - 300.0).powi(2) + (y as f32 - 220.0).powi(2)).sqrt() < 110.0;
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let n = (((x * 7 + y * 13) % 17) as f32) / 170.0;
            px.push(if disc(x, y) {
                [0.8 + n, 0.15, 0.1]
            } else if (480..600).contains(&x) && (120..260).contains(&y) {
                [0.1, 0.7 + n, 0.2]
            } else {
                [0.45 + n, 0.5 + n, 0.6]
            });
        }
    }
    (px, disc)
}

#[test]
#[ignore = "needs the SAM 2 ONNX models and ONNX Runtime"]
fn click_selects_the_disc() {
    let size = std::env::var("PHOTOCRAFT_SAM2_SIZE").ok().and_then(|s| ModelSize::parse(&s)).unwrap_or(ModelSize::Small);
    let assets = match Assets::locate(size) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let t = Instant::now();
    let sam = Sam2::load(&assets).expect("load");
    eprintln!("load {:?} on {:?}: {} ms", size, sam.backend(), t.elapsed().as_millis());

    let (w, h) = (640, 480);
    let (px, disc) = scene(w, h);
    let t = Instant::now();
    let emb = sam.encode(&px, w, h).expect("encode");
    eprintln!("encode: {} ms", t.elapsed().as_millis());
    let t = Instant::now();
    let masks = sam.decode(&emb, &[Prompt::Point { x: 300.0, y: 220.0, positive: true }]).expect("decode");
    eprintln!("decode: {} ms", t.elapsed().as_millis());

    let best = masks.iter().max_by(|a, b| a.iou.total_cmp(&b.iou)).expect("a mask");
    let m = best.to_bool(w, h);
    let (mut inter, mut union) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (m[y * w + x], disc(x, y));
            inter += (a && b) as usize;
            union += (a || b) as usize;
        }
    }
    let iou = inter as f32 / union as f32;
    eprintln!("candidate ious {:?}, disc IoU {iou:.3}", masks.iter().map(|m| m.iou).collect::<Vec<_>>());
    assert!(iou > 0.9, "disc IoU {iou}");

    let boxed = sam.decode(&emb, &[Prompt::Box { x0: 470.0, y0: 110.0, x1: 610.0, y1: 270.0 }]).expect("box");
    let b = boxed.iter().max_by(|a, b| a.iou.total_cmp(&b.iou)).expect("a mask").to_bool(w, h);
    assert!(b[190 * w + 540], "box prompt should select the green square");
    assert!(!b[220 * w + 300], "and not the disc");
}
