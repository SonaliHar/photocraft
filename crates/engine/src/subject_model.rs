//! Select Subject and Remove Background with a local learned model.
//!
//! The classical heuristic (`photocraft_algo::segment::subject`) stays as the fallback: it needs
//! no download and works offline, but it is not a semantic segmenter. The model
//! (`photocraft-ml`, IS-Net general use) is fetched into the user's cache folder the first time
//! it is wanted:
//!
//! * `engine: "model"` always uses it (downloading if needed) and reports failures.
//! * `engine: "classic"` never does.
//! * `engine: "auto"` (the default) uses it when it is already on disk, or when the app turned
//!   on [`set_auto_download`] (the desktop app does, so the first click fetches it with a
//!   progress bar); otherwise, and when the model fails, it falls back to the classical
//!   algorithm with a note.
//!
//! `PHOTOCRAFT_ML=off` disables the model altogether.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use photocraft_algo::segment::{self, matte, subject};
use photocraft_algo::selection::Region;
use photocraft_doc::{Document, LayerId};
use photocraft_ml::{MlError, models, subject as model};
use serde_json::Value;

use crate::jobs::JobCtx;
use crate::smartselect_cmds::with_doc_sampler;
use crate::{EngineError, Result};

/// Pixels of the image handed to the model (it squeezes it to 1024 x 1024 anyway).
const MODEL_INPUT_PX: usize = 1_400_000;

static AUTO_DOWNLOAD: AtomicBool = AtomicBool::new(false);

/// Let `engine: "auto"` download the model on first use. Called once by interactive apps; tests,
/// the CLI and scripts keep the offline default.
pub fn set_auto_download(on: bool) {
    AUTO_DOWNLOAD.store(on, Ordering::Relaxed);
}

/// Which segmenter to use (the `engine` parameter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Classic,
    Auto,
    Model,
}

impl Engine {
    pub fn from_params(cmd: &str, p: &Value) -> Result<Self> {
        match p.get("engine").and_then(Value::as_str) {
            None | Some("auto") => Ok(Engine::Auto),
            Some("classic") => Ok(Engine::Classic),
            Some("model") => Ok(Engine::Model),
            Some(other) => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("engine must be auto|classic|model, not `{other}`") }),
        }
    }
}

/// What a search found.
pub struct Found {
    pub region: Option<Region>,
    /// `"model"` or `"classic"`.
    pub engine: &'static str,
    /// Why the model was not used, when it was wanted.
    pub note: Option<String>,
}

fn disabled_by_env() -> bool {
    std::env::var("PHOTOCRAFT_ML").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"))
}

fn convert(e: MlError) -> EngineError {
    match e {
        MlError::Cancelled => EngineError::Cancelled,
        MlError::Msg(m) => EngineError::Other(m),
    }
}

/// The model file: on disk, or downloaded when the caller allows it. `Ok(None)` = not available
/// and not to be fetched now.
fn model_path(engine: Engine, ctx: &JobCtx) -> Result<Option<PathBuf>> {
    if let Some(p) = models::present(&models::SUBJECT) {
        return Ok(Some(p));
    }
    if engine != Engine::Model && !AUTO_DOWNLOAD.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let mb = models::SUBJECT.bytes / (1024 * 1024);
    ctx.progress(0.0, &format!("Downloading the Select Subject model ({mb} MB, once)"));
    let path = models::ensure(&models::SUBJECT, &|f| ctx.progress(0.7 * f, ""), &|| !ctx.cancelled()).map_err(convert)?;
    Ok(Some(path))
}

fn model_region(doc: &Document, layer: Option<LayerId>, all_layers: bool, engine: Engine, ctx: &JobCtx) -> Result<Option<Option<Region>>> {
    let Some(path) = model_path(engine, ctx)? else { return Ok(None) };
    ctx.progress(0.72, "Finding the subject");
    let region = with_doc_sampler(doc, layer, all_layers, |smp, d| -> Result<Option<Region>> {
        let canvas = d.bounds();
        if canvas.width() < 4 || canvas.height() < 4 {
            return Ok(None);
        }
        let step = segment::scale_for(canvas, MODEL_INPUT_PX);
        let img = smp.rgb_scaled(canvas, step);
        ctx.check()?;
        let m = model::matte(&path, &img.px, img.w, img.h, &|| !ctx.cancelled()).map_err(convert)?;
        Ok(matte::region_from_matte(canvas, &m.alpha, m.side, m.side))
    })?;
    ctx.progress(0.95, "");
    Ok(Some(region))
}

/// Find the subject of `layer` (or of the composite). Runs on the job's worker thread.
pub fn find(doc: &Document, layer: Option<LayerId>, all_layers: bool, engine: Engine, ctx: &JobCtx) -> Result<Found> {
    let mut note = None;
    if engine != Engine::Classic && !disabled_by_env() {
        match model_region(doc, layer, all_layers, engine, ctx) {
            Ok(Some(region)) => return Ok(Found { region, engine: "model", note: None }),
            Ok(None) => {}
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(e) if engine == Engine::Model => return Err(e),
            Err(e) => note = Some(format!("The Select Subject model could not be used ({e}); the classic selection was used instead")),
        }
    }
    ctx.progress(0.5, "Finding the subject");
    let region = with_doc_sampler(doc, layer, all_layers, |smp, d| subject::select_subject(smp, d.bounds()));
    ctx.check()?;
    Ok(Found { region, engine: "classic", note })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// With the model on disk (`PHOTOCRAFT_MODEL_DIR` or the cache) Select Subject finds a bright
    /// disc on a dark, textured background; without it the test has nothing to check.
    #[test]
    fn model_selects_a_disc() {
        if !models::is_present(&models::SUBJECT) {
            eprintln!("SKIPPED: no model installed (set PHOTOCRAFT_MODEL_DIR to a folder with isnet-general-use.onnx)");
            return;
        }
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 192, "height": 160})).unwrap();
        s.edit("paint", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            let n = surf.channels();
            let mut data = Vec::new();
            for y in 0..160 {
                for x in 0..192 {
                    let (dx, dy) = (x as f32 - 96.0, y as f32 - 80.0);
                    let c = if dx * dx + dy * dy < 45.0 * 45.0 { [0.95, 0.35, 0.2] } else { [0.08 + 0.02 * ((x / 8 + y / 8) % 2) as f32, 0.09, 0.12] };
                    data.extend_from_slice(&c[..3]);
                    if n == 4 {
                        data.push(1.0);
                    }
                }
            }
            surf.write_region(photocraft_geom::Rect::new(0, 0, 192, 160), &data);
            Ok(())
        })
        .unwrap();
        let r = s.execute("select.subject", json!({"engine": "model"})).unwrap();
        assert_eq!(r["engine"], "model", "{r}");
        let sel = s.active().unwrap().doc.selection.clone().unwrap();
        assert!(sel.rgba(96, 80)[0] > 0.9, "centre selected");
        assert!(sel.rgba(5, 5)[0] < 0.1, "corner not selected");
    }

    #[test]
    fn classic_engine_never_touches_the_model() {
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let r = s.execute("select.subject", json!({"engine": "classic"})).unwrap();
        assert!(r.get("engine").and_then(Value::as_str) == Some("classic") || r["changed"] == false, "{r}");
    }

    #[test]
    fn engine_parameter() {
        assert_eq!(Engine::from_params("x", &json!({})).unwrap(), Engine::Auto);
        assert_eq!(Engine::from_params("x", &json!({"engine": "classic"})).unwrap(), Engine::Classic);
        assert_eq!(Engine::from_params("x", &json!({"engine": "model"})).unwrap(), Engine::Model);
        assert!(Engine::from_params("x", &json!({"engine": "gpu"})).is_err());
    }
}
