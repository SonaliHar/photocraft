//! Bounded Clear policy over the shared paid native-region transaction.
use crate::{EngineError, Result, Session};
#[cfg(test)]
use photocraft_color::SampleType;
#[cfg(test)]
use photocraft_doc::LayerContent;
use photocraft_raster::backing::ReadControl;
#[cfg(test)]
use photocraft_raster::{Surface, backing::BackingError};
use serde_json::Value;
const ID: &str = "edit.clear";
pub(crate) const REGISTERED_WORK_LIMIT: u64 = 1 << 32;
fn bad(s: &str) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: s.into() }
}
fn decimal(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    if s.is_empty() || s.len() > 20 || s.len() > 1 && s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

pub(crate) fn validate_request(session: &Session, p: &Value) -> Result<()> {
    if !p.is_null() {
        let o = p.as_object().filter(|o| o.len() <= 4).ok_or_else(|| bad("bounded Clear parameters required"))?;
        if o.keys().any(|k| !matches!(k.as_str(), "layer" | "target" | "documentId" | "expectedRevision")) {
            return Err(bad("unsupported accounted Clear parameter"));
        }
    }
    if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
        return Err(bad("accounted Clear supports native layer pixels only"));
    }
    if p.get("layer").and_then(Value::as_str).is_some_and(|v| v.len() > 4096) {
        return Err(bad("Clear layer name is too long"));
    }
    let st = session.active().ok_or(EngineError::NoDocument)?;
    if !crate::accounted_native_pixels::programme_candidate(session)
        && (st.doc.accounted().is_none_or(|o| o.is_readonly_threshold_alpha_storage() || o.is_readonly_threshold_storage()) || st.doc.xcf_state.is_none())
    {
        return Err(bad("accounted Clear requires the plain imported native owner"));
    }
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(d), Some(r)) if decimal(d) == Some(st.doc.id.0) && decimal(r) == Some(st.revision) => {}
        _ => return Err(bad("captured Clear document or revision changed")),
    }
    Ok(())
}
pub(crate) struct Prepared<'a>(crate::accounted_native_pixels::Prepared<'a>);
impl<'a> Prepared<'a> {
    pub(crate) fn prepare(s: &Session, p: &'a Value, c: &ReadControl<'_>) -> Result<Self> {
        Ok(Self(crate::accounted_native_pixels::Prepared::prepare_clear(s, p, c)?))
    }
    pub(crate) fn execute(self, s: &mut Session, c: &ReadControl<'_>) -> Result<Value> {
        self.0.execute(s, c)
    }
}
#[cfg(test)]
#[path = "accounted_clear_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
