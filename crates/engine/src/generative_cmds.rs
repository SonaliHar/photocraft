//! Edit … Generative Fill: fill the selection through the AI provider and put the result on a
//! new layer, as one undo step in a background job. Content-Aware Fill (edit_menu_cmds) reads the
//! sampling window and writes the result; only the fill step here differs, and it is a provider
//! call rather than PatchMatch. No network code lives in this module.

use std::time::Duration;

use photocraft_color::ColorMode;
use serde_json::Value;

use crate::ai::{self, AiError, AiRequest, AiTask};
use crate::commands::CommandSpec;
use crate::edit_menu_cmds::{self, FillWindow, FilledWindow};
use crate::{EngineError, Result, Session};

const CMD: &str = "edit.generativeFill";
const LABEL: &str = "Generative Fill";

/// How long to wait between provider polls on a job worker.
const POLL_EVERY: Duration = Duration::from_millis(100);

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    // The provider comes first: the menu item stays enabled without one so the user gets the
    // message instead of a greyed-out item.
    let provider = s.ai_provider().ok_or(AiError::NoProvider)?;
    let prompt = match p.get("prompt") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.trim().is_empty() => None,
        Some(Value::String(_)) => Some(()),
        Some(_) => return Err(bad("`prompt` must be a string")),
    };
    if prompt.is_some() {
        // Milestone 1 has one provider task, fill_masked_area, which takes no text.
        return Err(EngineError::Other("Prompts aren't supported yet; leave the prompt empty to remove the selection".into()));
    }
    let id = edit_menu_cmds::pixel_layer(s).map_err(EngineError::Other)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    if doc.mode != ColorMode::Rgb {
        return Err(EngineError::Other("Generative Fill works on RGB documents for now".into()));
    }
    // Same selection errors as Content-Aware Fill, so the two removal commands explain themselves
    // the same way.
    let sel = doc.selection.clone().ok_or_else(|| bad("no selection"))?;
    let hb = sel.content_bounds().intersect(&doc.bounds());
    if hb.is_empty() {
        return Err(bad("the selection is outside the canvas"));
    }
    let window = crate::fill_cmds::sampling_window(hb, doc.bounds()).intersect(&doc.bounds());
    let surf = doc.layer(id).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(id))?;
    let fmt = surf.format();
    // Photoshop's Sample All Layers checkbox is off unless the caller asks for it.
    let sample_all = p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(false);
    // Refuse a window too large to read before allocating for it, as Content-Aware Fill does.
    crate::fill_cmds::window_size(window)?;
    // A background job when started with Session::start: the provider call runs on a worker
    // against the document snapshot, and cancelling it discards the result (jobs::run never
    // reaches the apply step), so the document stays untouched.
    crate::jobs::run(
        s,
        LABEL,
        true,
        move |ctx| {
            ctx.progress(0.0, LABEL);
            let FillWindow { img, bounds, original, cover, n_hole, .. } = edit_menu_cmds::read_fill_window(&doc, id, &sel, window, sample_all, fmt, ctx)?;
            // The provider needs soft coverage over the whole window, so the feathered edge and
            // the antialiased selection boundary reach it; the apply step blends the same way.
            let coverage = crate::fill_cmds::read_window_mask(&sel, window, &|| ctx.cancelled()).ok_or(EngineError::Cancelled)?;
            let req = AiRequest {
                task: AiTask::FillMaskedArea,
                window,
                pixels: img,
                format: fmt,
                coverage,
                prompt: None,
                client_version: format!("photocraft/{}", crate::build_info::VERSION),
            };
            ctx.progress(0.1, "Waiting for Craft AI Manager");
            let result = ai::run_to_completion(provider.as_ref(), &req, ctx, POLL_EVERY).map_err(EngineError::from)?;
            // Provider pixels are outside our input: one NaN or inf would silently corrupt the
            // layer, so the whole result is rejected instead.
            if result.pixels.iter().any(|v| !v.is_finite()) {
                return Err(EngineError::Other("The AI result contains invalid pixel values; the document is unchanged".into()));
            }
            ctx.progress(1.0, LABEL);
            Ok(FilledWindow { filled: result.pixels, bounds, original, cover, n_hole })
        },
        move |s, done| {
            let name = s.active().map_or_else(|| LABEL.to_string(), |st| edit_menu_cmds::fill_layer_name(&st.doc, LABEL));
            edit_menu_cmds::apply_content_aware_fill(s, LABEL, id, "new", fmt, window, done, &name)
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Generative Fill…",
        menu: &["Edit"],
        shortcut: None,
        params: r##"{"prompt":string?,"sampleAllLayers":bool=false} … {layer,filled,window} (fills the selection through the AI provider onto a new "Generative Fill" layer)"##,
        enabled: edit_menu_cmds::can_fill,
        run,
        journal: true,
    }]
}

#[cfg(test)]
#[path = "generative_cmds/tests.rs"]
mod tests;
