//! SAM 2 encoder + decoder over ONNX Runtime (native). On wasm every entry point reports
//! [`Error::Unsupported`].
//!
//! The ONNX exports (vietanhdev/segment-anything-2-onnx-models, Apache-2.0) take the image
//! stretched to 1024×1024 and normalised with the ImageNet mean/std, and answer prompts in that
//! 1024 space with 256×256 mask logits covering the same stretched image.

// The wasm stub never reads the embedding or preprocesses.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::sync::Arc;

use crate::{Assets, Error, INPUT_SIZE, MASK_SIZE, Result};

/// What ran the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Cuda,
    Cpu,
}

/// One user prompt, in the image's own pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prompt {
    /// A click on the object (`positive`) or off it.
    Point { x: f32, y: f32, positive: bool },
    /// A rectangle around the object.
    Box { x0: f32, y0: f32, x1: f32, y1: f32 },
}

/// The encoder's result for one image: keep it and decode many prompts against it.
#[derive(Clone, Debug)]
pub struct Embedding {
    /// Size of the image that was encoded; prompts are given in these pixels.
    pub width: usize,
    pub height: usize,
    data: Arc<EmbeddingData>,
}

#[derive(Debug)]
struct EmbeddingData {
    image_embed: Vec<f32>,
    high_res_0: Vec<f32>,
    high_res_1: Vec<f32>,
}

/// One candidate mask: [`MASK_SIZE`]² logits (positive = object) and the model's own quality guess.
#[derive(Clone, Debug)]
pub struct SamMask {
    pub logits: Vec<f32>,
    pub iou: f32,
}

impl SamMask {
    /// Bilinear logit at `(u, v)` in `0..=1` of the encoded image.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let n = MASK_SIZE;
        let fx = (u * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let fy = (v * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: usize, y: usize| self.logits.get(y * n + x).copied().unwrap_or(-10.0);
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bot = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        top * (1.0 - ty) + bot * ty
    }

    /// Thresholds the logits at `w × h` (bilinear upsampling), row-major, `true` = object.
    pub fn to_bool(&self, w: usize, h: usize) -> Vec<bool> {
        let mut out = Vec::with_capacity(w.saturating_mul(h));
        for y in 0..h {
            let v = (y as f32 + 0.5) / h as f32;
            for x in 0..w {
                out.push(self.sample((x as f32 + 0.5) / w as f32, v) > 0.0);
            }
        }
        out
    }
}

/// Resizes straight RGB (`0..=1`, `w × h`) to `n × n` bilinearly and returns ImageNet-normalised
/// planar CHW floats.
fn preprocess(px: &[[f32; 3]], w: usize, h: usize, n: usize) -> Vec<f32> {
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let mut out = vec![0.0f32; 3 * n * n];
    let (sx, sy) = (w as f32 / n as f32, h as f32 / n as f32);
    for y in 0..n {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..n {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let at = |xx: usize, yy: usize| px.get(yy * w + xx).copied().unwrap_or([0.0; 3]);
            let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            for ch in 0..3 {
                let v = (a[ch] * (1.0 - tx) + b[ch] * tx) * (1.0 - ty) + (c[ch] * (1.0 - tx) + d[ch] * tx) * ty;
                out[ch * n * n + y * n + x] = (v.clamp(0.0, 1.0) - MEAN[ch]) / STD[ch];
            }
        }
    }
    out
}

/// Prompt tensors (`coords` as x, y pairs in 1024 space, one label per point).
fn prompt_tensors(prompts: &[Prompt], w: usize, h: usize) -> Result<(Vec<f32>, Vec<f32>)> {
    let (kx, ky) = (INPUT_SIZE as f32 / w as f32, INPUT_SIZE as f32 / h as f32);
    let (mut coords, mut labels) = (Vec::new(), Vec::new());
    let mut push = |x: f32, y: f32, l: f32| -> Result<()> {
        if !x.is_finite() || !y.is_finite() {
            return Err(Error::BadInput("prompt coordinates must be finite".into()));
        }
        coords.extend([x * kx, y * ky]);
        labels.push(l);
        Ok(())
    };
    for p in prompts {
        match *p {
            Prompt::Point { x, y, positive } => push(x, y, if positive { 1.0 } else { 0.0 })?,
            // SAM encodes a box as two corner points with labels 2 (top-left) and 3 (bottom-right).
            Prompt::Box { x0, y0, x1, y1 } => {
                push(x0.min(x1), y0.min(y1), 2.0)?;
                push(x0.max(x1), y0.max(y1), 3.0)?;
            }
        }
    }
    if labels.is_empty() {
        return Err(Error::BadInput("needs at least one prompt".into()));
    }
    Ok((coords, labels))
}

#[cfg(target_arch = "wasm32")]
pub struct Sam2;

#[cfg(target_arch = "wasm32")]
impl Sam2 {
    pub fn load(_: &Assets) -> Result<Self> {
        Err(Error::Unsupported)
    }
    pub fn backend(&self) -> Backend {
        Backend::Cpu
    }
    pub fn encode(&self, _: &[[f32; 3]], _: usize, _: usize) -> Result<Embedding> {
        Err(Error::Unsupported)
    }
    pub fn decode(&self, _: &Embedding, _: &[Prompt]) -> Result<Vec<SamMask>> {
        Err(Error::Unsupported)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Sam2;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::{Mutex, OnceLock, PoisonError};

    use ort::ep;
    use ort::environment::Environment;
    use ort::session::Session;
    use ort::session::builder::GraphOptimizationLevel;
    use ort::value::Tensor;

    use super::*;

    fn rt(e: impl std::fmt::Display) -> Error {
        Error::Runtime(e.to_string())
    }

    /// Loads every shared library in `dirs` so cuDNN and cuBLAS find their lazily loaded parts
    /// (engines, NVRTC) without a `PATH` entry. Libraries load in any order: a pass that fails
    /// on a missing dependency is retried once the others are in.
    fn preload_dir_libs(dirs: &[Option<&std::path::Path>]) {
        let ext = if cfg!(windows) { "dll" } else if cfg!(target_os = "macos") { "dylib" } else { "so" };
        let mut pending: Vec<std::path::PathBuf> = dirs
            .iter()
            .flatten()
            .filter_map(|d| std::fs::read_dir(d).ok())
            .flat_map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
            .collect();
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|p| ort::util::preload_dylib(p).is_err());
            if pending.len() == before {
                break;
            }
        }
    }

    /// One process-wide ONNX Runtime environment, loaded from the first assets used.
    fn environment(a: &Assets) -> Result<&'static Environment> {
        static ENV: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();
        ENV.get_or_init(|| {
            // CUDA/cuDNN are optional; without them the CUDA provider fails to register and the
            // sessions fall back to the CPU.
            preload_dir_libs(&[a.cuda_dir.as_deref(), a.cudnn_dir.as_deref()]);
            let builder = ort::init_from(&a.ort_lib).map_err(|e| format!("{}: {e}", a.ort_lib.display()))?;
            builder.with_name("photocraft").build().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| Error::Runtime(e.clone()))
    }

    fn session(env: &Environment, path: &std::path::Path, gpu: bool) -> Result<Session> {
        let b = Session::builder(env).map_err(rt)?.with_optimization_level(GraphOptimizationLevel::Level3).map_err(rt)?;
        let mut b = if gpu { b.with_execution_providers([ep::CUDA::default().build().error_on_failure()]).map_err(rt)? } else { b };
        b.commit_from_file(path).map_err(rt)
    }

    /// SAM 2 sessions for one model size. Cheap to share: calls lock the session they use.
    pub struct Sam2 {
        encoder: Mutex<Session>,
        decoder: Mutex<Session>,
        backend: Backend,
    }

    impl Sam2 {
        /// Loads the model, on the GPU when CUDA works and the CPU otherwise.
        pub fn load(a: &Assets) -> Result<Self> {
            let env = environment(a)?;
            let on_gpu = (|| Ok::<_, Error>((session(env, &a.encoder, true)?, session(env, &a.decoder, true)?)))();
            let (encoder, decoder, backend) = match on_gpu {
                Ok((e, d)) => (e, d, Backend::Cuda),
                Err(_) => (session(env, &a.encoder, false)?, session(env, &a.decoder, false)?, Backend::Cpu),
            };
            Ok(Self { encoder: Mutex::new(encoder), decoder: Mutex::new(decoder), backend })
        }

        pub fn backend(&self) -> Backend {
            self.backend
        }

        /// Encodes straight RGB samples in `0..=1` (`width × height`, row-major).
        pub fn encode(&self, px: &[[f32; 3]], width: usize, height: usize) -> Result<Embedding> {
            if width == 0 || height == 0 || px.len() != width.saturating_mul(height) {
                return Err(Error::BadInput("image size does not match its pixels".into()));
            }
            let input = preprocess(px, width, height, INPUT_SIZE);
            let n = INPUT_SIZE as i64;
            let tensor = Tensor::from_array(([1i64, 3, n, n], input)).map_err(rt)?;
            let mut enc = self.encoder.lock().unwrap_or_else(PoisonError::into_inner);
            let out = enc.run(ort::inputs!["image" => tensor]).map_err(rt)?;
            let get = |name: &str| -> Result<Vec<f32>> { Ok(out[name].try_extract_tensor::<f32>().map_err(rt)?.1.to_vec()) };
            Ok(Embedding {
                width,
                height,
                data: Arc::new(EmbeddingData { image_embed: get("image_embed")?, high_res_0: get("high_res_feats_0")?, high_res_1: get("high_res_feats_1")? }),
            })
        }

        /// Decodes `prompts` against `emb`: three candidate masks, coarse to fine.
        pub fn decode(&self, emb: &Embedding, prompts: &[Prompt]) -> Result<Vec<SamMask>> {
            let (coords, labels) = prompt_tensors(prompts, emb.width, emb.height)?;
            let np = labels.len() as i64;
            let d = &emb.data;
            let mut dec = self.decoder.lock().unwrap_or_else(PoisonError::into_inner);
            let m = MASK_SIZE as i64;
            let out = dec
                .run(ort::inputs![
                    "image_embed" => Tensor::from_array(([1i64, 256, 64, 64], d.image_embed.clone())).map_err(rt)?,
                    "high_res_feats_0" => Tensor::from_array(([1i64, 32, 256, 256], d.high_res_0.clone())).map_err(rt)?,
                    "high_res_feats_1" => Tensor::from_array(([1i64, 64, 128, 128], d.high_res_1.clone())).map_err(rt)?,
                    "point_coords" => Tensor::from_array(([1i64, np, 2], coords)).map_err(rt)?,
                    "point_labels" => Tensor::from_array(([1i64, np], labels)).map_err(rt)?,
                    "mask_input" => Tensor::from_array(([1i64, 1, m, m], vec![0.0f32; MASK_SIZE * MASK_SIZE])).map_err(rt)?,
                    "has_mask_input" => Tensor::from_array(([1i64], vec![0.0f32])).map_err(rt)?,
                ])
                .map_err(rt)?;
            let (shape, masks) = out["masks"].try_extract_tensor::<f32>().map_err(rt)?;
            let (_, iou) = out["iou_predictions"].try_extract_tensor::<f32>().map_err(rt)?;
            let per = MASK_SIZE * MASK_SIZE;
            let count = shape.get(1).copied().unwrap_or(0).max(0) as usize;
            if count == 0 || masks.len() < count * per || iou.len() < count {
                return Err(Error::Runtime("decoder returned an unexpected shape".into()));
            }
            Ok((0..count).map(|i| SamMask { logits: masks[i * per..(i + 1) * per].to_vec(), iou: iou[i] }).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_normalises_and_keeps_planes_apart() {
        let px = vec![[1.0, 0.0, 0.5]; 4];
        let t = preprocess(&px, 2, 2, 4);
        assert_eq!(t.len(), 3 * 16);
        assert!((t[0] - (1.0 - 0.485) / 0.229).abs() < 1e-5);
        assert!((t[16] - (0.0 - 0.456) / 0.224).abs() < 1e-5);
        assert!((t[32] - (0.5 - 0.406) / 0.225).abs() < 1e-5);
    }

    #[test]
    fn box_becomes_two_labelled_corners_in_input_space() {
        let (c, l) = prompt_tensors(&[Prompt::Box { x0: 300.0, y0: 100.0, x1: 100.0, y1: 50.0 }], 2048, 512).unwrap();
        assert_eq!(l, vec![2.0, 3.0]);
        assert_eq!(c, vec![50.0, 100.0, 150.0, 200.0]);
    }

    #[test]
    fn bad_prompts_are_errors() {
        assert!(prompt_tensors(&[], 10, 10).is_err());
        assert!(prompt_tensors(&[Prompt::Point { x: f32::NAN, y: 0.0, positive: true }], 10, 10).is_err());
    }

    #[test]
    fn mask_upsampling_follows_the_logits() {
        let mut logits = vec![-5.0; MASK_SIZE * MASK_SIZE];
        for y in 0..MASK_SIZE {
            for x in MASK_SIZE / 2..MASK_SIZE {
                logits[y * MASK_SIZE + x] = 5.0;
            }
        }
        let m = SamMask { logits, iou: 1.0 };
        let b = m.to_bool(40, 10);
        assert!(!b[0] && b[39] && !b[19] && b[21]);
    }
}
