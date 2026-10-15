//! Ordinary solid Fill over the existing plain imported paid pixel owner.
//! Native-wide CMS, patterns, history and content-aware sources remain separate.
use crate::{EngineError, Result, Session};
use photocraft_color::{ColorMode, SampleType};
use photocraft_raster::backing::ReadControl;
use serde_json::Value;

// One fixed object entry, two short strings and map/value/header slack. The
// temporary transaction and enclosing journal both retain genuine allowances.
pub(crate) const REPLY_ENVELOPE: usize = 1024;
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "edit.fill".into(), msg: message.into() }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || text.len() > 1 && text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
pub(crate) fn validate_request(session: &Session, params: &Value) -> Result<()> {
    // Unknown fields and ordinary non-object/default forms remain ignored as
    // upstream Fill does. The parent journal bounds the complete JSON tree.
    if crate::channel_cmds::target_of(params) != crate::channel_cmds::Target::Pixels {
        return Err(bad("accounted Fill supports ordinary native layer pixels only"));
    }
    if params.get("layer").and_then(Value::as_str).is_some_and(|v| v.len() > 4096) {
        return Err(bad("Fill layer name is too long"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.accounted().is_none_or(|v| v.is_readonly_threshold_alpha_storage() || v.is_readonly_threshold_storage())
        || state.doc.xcf_state.is_none()
        || !matches!(state.doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(state.doc.depth, SampleType::U8 | SampleType::U16 | SampleType::F32)
    {
        return Err(bad("accounted Fill requires the plain imported U8/U16/F32 RGB/Gray owner"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(d), Some(r)) if decimal(d) == Some(state.doc.id.0) && decimal(r) == Some(state.revision) => {}
        _ => return Err(bad("captured Fill document or revision changed")),
    }
    Ok(())
}
pub(crate) struct Prepared<'a>(crate::accounted_native_pixels::Prepared<'a>);
impl<'a> Prepared<'a> {
    pub(crate) fn prepare(session: &Session, params: &'a Value, control: &ReadControl<'_>) -> Result<Self> {
        validate_request(session, params)?;
        Ok(Self(crate::accounted_native_pixels::Prepared::prepare_fill(session, params, control)?))
    }
    pub(crate) fn execute(self, session: &mut Session, control: &ReadControl<'_>) -> Result<Value> {
        self.0.execute(session, control)
    }
}

#[cfg(test)]
#[path = "accounted_fill_tests.rs"]
mod tests;
