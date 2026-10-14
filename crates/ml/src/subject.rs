//! Salient-object matting with a local ONNX model (IS-Net general use).
//!
//! [`matte`] takes an RGB image of any size, squeezes it to the model's 1024 x 1024 input, and
//! returns the model's soft alpha at that size. Turning it into a selection at the document's
//! resolution is the caller's job (`photocraft-algo`).

use std::path::Path;

use crate::{MlError, Result};

/// Side of the model's square input and output.
pub const SIDE: usize = 1024;

/// The model's soft alpha, `SIDE x SIDE`, row-major, `0..=1`.
pub struct Matte {
    pub side: usize,
    pub alpha: Vec<f32>,
}

#[cfg(not(target_arch = "wasm32"))]
mod runtime {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, PoisonError};

    use tract_onnx::prelude::*;

    use super::SIDE;
    use crate::{MlError, Result};

    pub(super) type Plan = Arc<TypedRunnableModel>;

    /// The optimised model, kept between calls: loading and optimising costs about a second.
    static LOADED: Mutex<Option<(PathBuf, Plan)>> = Mutex::new(None);

    pub(super) fn load(model: &Path) -> Result<Plan> {
        let mut slot = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((p, plan)) = slot.as_ref()
            && p == model
        {
            return Ok(Arc::clone(plan));
        }
        configure_threads();
        let fail = |e: TractError| MlError::msg(format!("cannot load the model {}: {e}", model.display()));
        let plan = tract_onnx::onnx()
            .model_for_path(model)
            .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, SIDE, SIDE]).into()))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(fail)?;
        *slot = Some((model.to_path_buf(), Arc::clone(&plan)));
        Ok(plan)
    }

    fn configure_threads() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let n = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 12);
            tract_linalg::multithread::set_default_executor(tract_linalg::multithread::Executor::multithread(n));
        });
    }

    /// Run `plan` on a `SIDE x SIDE` NCHW tensor given as a closure over `(channel, y, x)`.
    pub(super) fn run(plan: &Plan, at: impl Fn(usize, usize, usize) -> f32) -> Result<Vec<f32>> {
        let input: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, SIDE, SIDE), |(_, c, y, x)| at(c, y, x)).into();
        let outputs = plan.run(tvec!(input.into())).map_err(|e| MlError::msg(format!("the model failed: {e}")))?;
        let first = outputs.first().ok_or_else(|| MlError::msg("the model returned nothing"))?;
        let view = first.to_plain_array_view::<f32>().map_err(|e| MlError::msg(format!("unexpected model output: {e}")))?;
        Ok(view.iter().copied().collect())
    }
}

#[cfg(target_arch = "wasm32")]
mod runtime {
    use std::path::Path;

    use crate::{MlError, Result};

    pub(super) struct Plan;

    pub(super) fn load(_: &Path) -> Result<Plan> {
        Err(MlError::msg("the Select Subject model is not available in the web build"))
    }

    pub(super) fn run(_: &Plan, _: impl Fn(usize, usize, usize) -> f32) -> Result<Vec<f32>> {
        Err(MlError::msg("the Select Subject model is not available in the web build"))
    }
}

/// Bilinear resample of `src` (`w x h`, RGB `0..=1`) to `SIDE x SIDE`.
fn squeeze(src: &[[f32; 3]], w: usize, h: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(SIDE * SIDE);
    let (sx, sy) = (w as f32 / SIDE as f32, h as f32 / SIDE as f32);
    for y in 0..SIDE {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..SIDE {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let at = |xx: usize, yy: usize| src.get(yy * w + xx).copied().unwrap_or([0.0; 3]);
            let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            let mut p = [0.0f32; 3];
            for k in 0..3 {
                let top = a[k] + (b[k] - a[k]) * tx;
                let bot = c[k] + (d[k] - c[k]) * tx;
                p[k] = top + (bot - top) * ty;
            }
            out.push(p);
        }
    }
    out
}

/// Run the model on `rgb` (`w x h`, straight RGB `0..=1`). `keep_going` is checked around the
/// (single, multi-second) inference call.
pub fn matte(model: &Path, rgb: &[[f32; 3]], w: usize, h: usize, keep_going: &dyn Fn() -> bool) -> Result<Matte> {
    if w == 0 || h == 0 || rgb.len() != w * h {
        return Err(MlError::msg("the image to analyse is empty"));
    }
    let plan = runtime::load(model)?;
    if !keep_going() {
        return Err(MlError::Cancelled);
    }
    let sq = squeeze(rgb, w, h);
    // rembg's IS-Net pre-processing: scale by the brightest sample, then centre on 0.5.
    let top = sq.iter().flat_map(|p| p.iter().copied()).fold(1e-6f32, f32::max);
    let mut alpha = runtime::run(&plan, |c, y, x| sq.get(y * SIDE + x).map_or(0.0, |p| p[c] / top - 0.5))?;
    if !keep_going() {
        return Err(MlError::Cancelled);
    }
    if alpha.len() != SIDE * SIDE {
        return Err(MlError::msg(format!("unexpected model output size {}", alpha.len())));
    }
    let (lo, hi) = alpha.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return Err(MlError::msg("the model found nothing"));
    }
    let span = hi - lo;
    for v in &mut alpha {
        *v = ((*v - lo) / span).clamp(0.0, 1.0);
    }
    Ok(Matte { side: SIDE, alpha })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squeeze_keeps_a_flat_colour() {
        let src = vec![[0.25, 0.5, 0.75]; 7 * 5];
        let out = squeeze(&src, 7, 5);
        assert_eq!(out.len(), SIDE * SIDE);
        assert!(out.iter().all(|p| (p[0] - 0.25).abs() < 1e-5 && (p[2] - 0.75).abs() < 1e-5));
    }

    #[test]
    fn rejects_mismatched_input() {
        let r = matte(Path::new("missing.onnx"), &[[0.0; 3]; 3], 2, 2, &|| true);
        assert!(r.is_err());
    }

    /// With the model present (`PHOTOCRAFT_MODEL_DIR` or the cache), a bright disc on black comes
    /// out mostly selected in the middle and not at the corners.
    #[test]
    fn real_model_finds_a_disc() {
        let Some(model) = crate::models::present(&crate::models::SUBJECT) else {
            eprintln!("SKIPPED: no model installed (set PHOTOCRAFT_MODEL_DIR to a folder with isnet-general-use.onnx)");
            return;
        };
        let (w, h) = (256usize, 256usize);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 - 128.0, (i / w) as f32 - 128.0);
                if x * x + y * y < 70.0 * 70.0 { [0.9, 0.3, 0.2] } else { [0.1, 0.1, 0.12] }
            })
            .collect();
        let m = matte(&model, &rgb, w, h, &|| true).unwrap();
        let at = |x: usize, y: usize| m.alpha[y * SIDE + x];
        assert!(at(SIDE / 2, SIDE / 2) > 0.5, "centre {}", at(SIDE / 2, SIDE / 2));
        assert!(at(8, 8) < 0.2, "corner {}", at(8, 8));
    }
}
