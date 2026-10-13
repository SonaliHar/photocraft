//! Scoped current-vector style edits; immutable imported caches remain evidence only.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerId,
    item_identity::{ItemRef, PathId},
    xcf_filters::{ExactNumber, FilterValue},
    xcf_items::EditableSource,
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

const SET: &str = "xcf.vector.stroke.width.set";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: SET.into(), msg: message.into() }
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn identity(p: &Value, key: &str) -> Result<u64> {
    let s = p.get(key).and_then(Value::as_str).ok_or_else(|| bad(format!("{key} requires an exact decimal-string U64")))?;
    if s.is_empty() || s.len() > 20 || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("identity must be canonical nonzero decimal U64"));
    }
    s.parse().map_err(|_| bad("identity exceeds U64"))
}
fn width(p: &Value) -> Result<u32> {
    let s = p.get("widthF32Bits").and_then(Value::as_str).ok_or_else(|| bad("widthF32Bits requires eight lowercase hexadecimal digits"))?;
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(bad("widthF32Bits requires eight lowercase hexadecimal digits"));
    }
    let bits = u32::from_str_radix(s, 16).map_err(|_| bad("invalid width bits"))?;
    if ![1f32.to_bits(), 2f32.to_bits(), 3f32.to_bits(), 4f32.to_bits(), 5f32.to_bits(), 6f32.to_bits(), 7f32.to_bits(), 8f32.to_bits(), 9f32.to_bits()]
        .contains(&bits)
    {
        return Err(bad("only measured width values are editable within their independently qualified AA/non-AA stroke styles"));
    }
    Ok(bits)
}
fn source_index(doc: &Document, layer: LayerId, path: PathId, ctl: &Interrupt<'_>) -> Result<usize> {
    doc.validate_xcf_state_with(ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    if doc.paths.iter().filter(|p| p.id == path).count() != 1 || doc.layer(layer).is_none() {
        return Err(bad("unknown layer or path identity"));
    }
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing imported vector state"))?;
    let mut matches = state.source_items.iter().enumerate().filter(|(_, i)| i.owner == ItemRef::Layer(layer));
    let (index, item) = matches.next().ok_or_else(|| bad("unknown vector owner"))?;
    if matches.next().is_some() {
        return Err(bad("duplicate vector owner"));
    }
    let EditableSource::Vector { path: bound, source_modified: false, configuration } = &item.source else {
        return Err(bad("owner is not an editable regenerated vector"));
    };
    if *bound != path {
        return Err(bad("path identity does not belong to vector owner"));
    }
    if !configuration.iter().any(|a| a.name == "filled" && a.value == FilterValue::Bool(false))
        || !configuration.iter().any(|a| a.name == "stroked" && a.value == FilterValue::Bool(true))
    {
        return Err(bad("only qualified stroke-only sources are editable"));
    }
    let locks = state.effective_locks(item.owner).map_err(|e| bad(e.to_string()))?;
    let ordinary = doc.effective_locks(layer);
    if locks.content || locks.position || ordinary.all || ordinary.pixels || ordinary.position {
        return Err(bad("vector owner or inherited content/position is locked"));
    }
    photocraft_compose::gimp_vector_items::qualify_source(doc, item, ctl)
        .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(format!("unqualified current vector source: {e}")) })?;
    Ok(index)
}
fn width_slot(doc: &mut Document, index: usize) -> Result<&mut FilterValue> {
    let source = doc.xcf_state.as_mut().and_then(|s| s.source_items.get_mut(index)).ok_or_else(|| bad("missing staged vector source"))?;
    let EditableSource::Vector { configuration, .. } = &mut source.source else { return Err(bad("missing staged vector configuration")) };
    let FilterValue::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").ok_or_else(|| bad("missing stroke configuration"))?.value
    else {
        return Err(bad("invalid stroke configuration"));
    };
    Ok(&mut stroke.iter_mut().find(|a| a.name == "width").ok_or_else(|| bad("missing stroke width"))?.value)
}
/// Synchronous command seam with cooperative cancellation, including the final commit boundary.
pub fn set_width_with_interrupt(s: &mut Session, p: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    let object = p.as_object().ok_or_else(|| bad("parameters must be an object"))?;
    if object.len() != 3 || object.keys().any(|k| !matches!(k.as_str(), "layerId" | "pathId" | "widthF32Bits")) {
        return Err(bad("exact layerId, pathId and widthF32Bits parameters are required"));
    }
    let layer = LayerId(identity(p, "layerId")?);
    let path = PathId(identity(p, "pathId")?);
    let bits = width(p)?;
    let before = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let index = source_index(before, layer, path, ctl)?;
    let mut proposed = (**before).clone();
    let slot = width_slot(&mut proposed, index)?;
    let replacement = FilterValue::Number(ExactNumber::Float32Bits(bits));
    let changed = *slot != replacement;
    *slot = replacement;
    crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&proposed, ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { e })?;
    ctl.progress(1.0);
    if ctl.cancelled() {
        return Err(EngineError::Cancelled);
    }
    if changed {
        s.edit_with_interrupt("Edit GIMP Vector Stroke Width", ctl, |doc, _| {
            let index = source_index(doc, layer, path, ctl)?;
            *width_slot(doc, index)? = FilterValue::Number(ExactNumber::Float32Bits(bits));
            if ctl.cancelled() {
                return Err(EngineError::Cancelled);
            }
            Ok(())
        })?;
    }
    Ok(json!({"layerId":layer.0.to_string(),"pathId":path.0.to_string(),"widthF32Bits":format!("{bits:08x}"),"updated":changed}))
}
fn set(s: &mut Session, p: &Value) -> Result<Value> {
    set_width_with_interrupt(s, p, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: SET,
        label: "Edit GIMP Vector Stroke Width",
        menu: &[],
        shortcut: None,
        params: r#"{"layerId":"nonzero decimal U64","pathId":"nonzero decimal U64","widthF32Bits":"3f800000|40000000|40400000|40800000|40a00000|40c00000|40e00000|41000000|41100000"} → checked measured AA/non-AA width within its qualified stroke-only vector style; original caches and resources remain immutable."#,
        enabled,
        run: set,
        journal: true,
    }]
}
