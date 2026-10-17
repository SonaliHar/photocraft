//! Explicit current GIMP composition policy controls; source evidence stays immutable.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{BlendMode, ImportCompositing, LayerContent, LayerId, SampleEncoding};
use photocraft_doc::{
    import_compositing::{GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::ItemRef,
};
use serde_json::{Value, json};
const INSPECT: &str = "xcf.composition.inspect";
const SET: &str = "xcf.composition.set";
const SET_KEYS: [&str; 8] = ["layerId", "sourceMode", "legacy", "blendSpace", "compositeSpace", "operator", "documentId", "expectedRevision"];
fn bad(cmd: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn params(cmd: &str, value: &Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or_else(|| bad(cmd, "expected an object"))?;
    if object.len() > keys.len() {
        return Err(bad(cmd, "too many composition parameters"));
    }
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(bad(cmd, "unknown composition parameter"));
    }
    Ok(())
}
fn id(cmd: &str, value: &Value) -> Result<LayerId> {
    let text = value.get("layerId").and_then(Value::as_str).ok_or_else(|| bad(cmd, "layerId must be a nonzero decimal string"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad(cmd, "invalid decimal layerId"));
    }
    let number = text.parse::<u64>().map_err(|_| bad(cmd, "layerId overflow"))?;
    if number == 0 || number.to_string() != text {
        return Err(bad(cmd, "layerId must use canonical nonzero decimal notation"));
    }
    Ok(LayerId(number))
}
fn captured_target(s: &Session, value: &Value) -> Result<()> {
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let decimal = |value: &Value, nonzero: bool| -> Result<u64> {
        let text = value
            .as_str()
            .filter(|text| {
                !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
            })
            .ok_or_else(|| bad(SET, "captured target requires canonical decimal strings"))?;
        let number = text.parse::<u64>().map_err(|_| bad(SET, "captured target overflow"))?;
        if nonzero && number == 0 {
            return Err(bad(SET, "documentId must be nonzero"));
        }
        Ok(number)
    };
    match (value.get("documentId"), value.get("expectedRevision")) {
        (None, None) => {}
        (Some(document), Some(revision)) if decimal(document, true)? == active.doc.id.0 && decimal(revision, false)? == active.revision => {}
        _ => return Err(bad(SET, "captured composition target changed")),
    }
    if active.revision == u64::MAX {
        return Err(bad(SET, "composition revision exhausted"));
    }
    Ok(())
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|state| state.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no imported composition state".into())
}
fn inspect(s: &mut Session, value: &Value) -> Result<Value> {
    params(INSPECT, value, &["layerId"])?;
    let id = id(INSPECT, value)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    doc.layer(id).ok_or_else(|| bad(INSPECT, "unknown layerId"))?;
    let item = doc
        .xcf_state
        .as_ref()
        .and_then(|state| state.items.iter().find(|item| item.item == ItemRef::Layer(id)))
        .ok_or_else(|| bad(INSPECT, "no canonical imported layer metadata"))?;
    Ok(json!({"layerId":id.0.to_string(),"composition":item.composition}))
}
fn space(value: &Value, key: &str) -> Result<(GimpSpace, i32, SampleEncoding)> {
    match value.get(key).and_then(Value::as_str) {
        Some("linear-rgb") => Ok((GimpSpace::LinearRgb, 1, SampleEncoding::Linear)),
        Some("profile-rgb") => Ok((GimpSpace::ProfileRgb, 2, SampleEncoding::NonlinearProfile)),
        Some("perceptual-rgb") => Ok((GimpSpace::PerceptualRgb, 4, SampleEncoding::Perceptual)),
        _ => Err(bad(SET, format!("unsupported or ambiguous {key}; specify linear-rgb, profile-rgb or perceptual-rgb"))),
    }
}
fn operator(value: &Value) -> Result<(GimpOperator, i32)> {
    match value.get("operator").and_then(Value::as_str) {
        Some("union") => Ok((GimpOperator::Union, 1)),
        Some("clip-to-backdrop") => Ok((GimpOperator::ClipToBackdrop, 2)),
        Some("clip-to-layer") => Ok((GimpOperator::ClipToLayer, 3)),
        Some("intersection") => Ok((GimpOperator::Intersection, 4)),
        _ => Err(bad(SET, "unsupported or ambiguous operator")),
    }
}
fn blend(mode: i32) -> Result<BlendMode> {
    Ok(match mode {
        28 => BlendMode::Normal,
        23 => BlendMode::Overlay,
        30 => BlendMode::Multiply,
        31 => BlendMode::Screen,
        32 => BlendMode::Difference,
        33 => BlendMode::LinearDodge,
        34 => BlendMode::Subtract,
        35 => BlendMode::Darken,
        36 => BlendMode::Lighten,
        41 => BlendMode::Divide,
        42 => BlendMode::ColorDodge,
        43 => BlendMode::ColorBurn,
        44 => BlendMode::HardLight,
        45 => BlendMode::SoftLight,
        48 => BlendMode::VividLight,
        49 => BlendMode::PinLight,
        50 => BlendMode::LinearLight,
        51 => BlendMode::HardMix,
        52 => BlendMode::Exclusion,
        53 => BlendMode::LinearBurn,
        _ => return Err(bad(SET, "unsupported mode: requires a qualified modern mode with an exact ordinary BlendMode equivalent")),
    })
}
/// Borrowed admission before dispatch can commit a transient floating preview.
pub(crate) fn dispatch_preflight(s: &Session, command: &str, value: &Value) -> Result<()> {
    if command != SET {
        return Ok(());
    }
    params(SET, value, &SET_KEYS)?;
    captured_target(s, value)?;
    id(SET, value)?;
    let mode =
        value.get("sourceMode").and_then(Value::as_i64).and_then(|mode| i32::try_from(mode).ok()).ok_or_else(|| bad(SET, "sourceMode must be an integer"))?;
    blend(mode)?;
    if value.get("legacy").and_then(Value::as_bool) != Some(false) {
        return Err(bad(SET, "only explicit legacy=false modern policies are qualified"));
    }
    space(value, "blendSpace")?;
    space(value, "compositeSpace")?;
    operator(value)?;
    if s.active().ok_or(EngineError::NoDocument)?.floating.is_some() {
        return Err(bad(SET, "finish or cancel the transient floating preview before editing composition"));
    }
    Ok(())
}
/// Borrowed parser seam for the paid readonly-flat adapter. The caller holds
/// its real prospective temporary fee before invoking these existing helpers.
pub(crate) fn paid_flat_composition_patch(s: &Session, value: &Value) -> Result<photocraft_doc::accounted_document::FlatCompositeCompositionPatch> {
    dispatch_preflight(s, SET, value)?;
    let layer = id(SET, value)?;
    let source_mode =
        value.get("sourceMode").and_then(Value::as_i64).and_then(|v| i32::try_from(v).ok()).ok_or_else(|| bad(SET, "sourceMode must be an integer"))?;
    blend(source_mode)?;
    let (blend_space, _, _) = space(value, "blendSpace")?;
    let (composite_space, _, _) = space(value, "compositeSpace")?;
    let (operator, _) = operator(value)?;
    Ok(photocraft_doc::accounted_document::FlatCompositeCompositionPatch { layer, source_mode, blend_space, composite_space, operator })
}

pub fn set_with_interrupt(s: &mut Session, value: &Value, control: &photocraft_raster::Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    dispatch_preflight(s, SET, value)?;
    params(SET, value, &SET_KEYS)?;
    let id = id(SET, value)?;
    let mode =
        value.get("sourceMode").and_then(Value::as_i64).and_then(|mode| i32::try_from(mode).ok()).ok_or_else(|| bad(SET, "sourceMode must be an integer"))?;
    let ordinary = blend(mode)?;
    if value.get("legacy").and_then(Value::as_bool) != Some(false) {
        return Err(bad(SET, "only explicit legacy=false modern policies are qualified"));
    }
    let (blend_space, stored_blend_space, blend_encoding) = space(value, "blendSpace")?;
    let (composite_space, stored_composite_space, composite_encoding) = space(value, "compositeSpace")?;
    let (operator, stored_operator) = operator(value)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    crate::xcf_item_locks::effective_layer_locks(doc, id)?.require(crate::xcf_item_locks::EditKind::Content)?;
    let footprint = s
        .active()
        .ok_or(EngineError::NoDocument)?
        .doc
        .resident_raster_footprint_with(control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, e.to_string()) })?;
    let retained_peak = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes)
        .filter(|bytes| *bytes <= photocraft_raster::MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(SET, "composition raster retention and clone exceed resident limit"))?;
    let render_limit =
        photocraft_raster::MAX_CHECKED_RASTER_BYTES.checked_sub(retained_peak).ok_or_else(|| bad(SET, "composition render budget underflow"))?.min(512 << 20);
    // The scoped scene ledger already charges its own borrowed source raster.
    // Retain headroom for the outer edit's sparse-map/default COW clone.
    let scene_limit = photocraft_raster::MAX_CHECKED_RASTER_BYTES
        .checked_sub(footprint.clone_metadata_bytes)
        .ok_or_else(|| bad(SET, "composition scene budget underflow"))?
        .min(512 << 20);
    s.edit_with_interrupt("Set GIMP Composition Policy", control, |doc, _| {
        doc.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, e.to_string()) })?;
        let state = doc.xcf_state.as_ref().ok_or_else(|| bad(SET, "missing imported composition state"))?;
        crate::xcf_item_locks::effective_layer_locks(doc, id)?.require(crate::xcf_item_locks::EditKind::Content)?;
        let layer = doc.layer(id).ok_or_else(|| bad(SET, "unknown layerId"))?;
        if !matches!(layer.content, LayerContent::Raster(_)) {
            return Err(bad(SET, "composition editing currently requires an ordinary raster owner"));
        }
        if state.floating.is_some()
            || state.editable_indexed.iter().any(|r| r.owner == id)
            || state.source_items.iter().any(|item| item.owner == ItemRef::Layer(id))
        {
            return Err(bad(SET, "floating, indexed or editable-source composition transitions remain unqualified"));
        }
        let item = doc
            .xcf_state
            .as_mut()
            .and_then(|state| state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)))
            .ok_or_else(|| bad(SET, "missing canonical layer metadata"))?;
        let policy = item.composition.as_mut().ok_or_else(|| bad(SET, "missing canonical composition policy"))?;
        policy.source_mode = mode;
        policy.legacy = false;
        policy.stored_blend_space = stored_blend_space;
        policy.stored_composite_space = stored_composite_space;
        policy.stored_operator = stored_operator;
        policy.pass_through = false;
        policy.resolved = Some(ResolvedGimpComposition { blend_space, composite_space, operator });
        let layer = doc.layer_mut(id).ok_or_else(|| bad(SET, "unknown layerId"))?;
        layer.blend = ordinary;
        layer.import_compositing = Some(ImportCompositing { blend_encoding, composite_encoding, pass_through: false });
        let bounds = doc.bounds();
        let pixels = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(|| bad(SET, "composition validation work overflow"))?;
        if pixels == 0 || pixels > 36_000_000 {
            return Err(bad(SET, "composition validation exceeds 36 MP work limit"));
        }
        let accounted = photocraft_compose::gimp_render::PreparedRenderer::requires_preparation_budget(doc);
        let prepared = if accounted {
            None
        } else {
            Some(
                photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control)
                    .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, format!("unqualified composition pipeline: {e}")) })?,
            )
        };
        let scene = if accounted {
            Some(
                photocraft_compose::gimp_render::scene_budget::prepare(
                    doc,
                    control,
                    photocraft_compose::gimp_render::scene_budget::SceneLimits { resident_bytes: scene_limit, work_units: 36_000_000 * 512 },
                )
                .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, format!("unqualified composition pipeline: {e}")) })?,
            )
        } else {
            None
        };
        // Small checked buffers bound peak memory while every affected canvas pixel is evaluated.
        let mut sampled = 0u64;
        let mut resident = 0u64;
        let mut poll = |work| {
            use photocraft_cms::WideWork;
            use photocraft_compose::gimp_composite::CompositionError;
            control.check().map_err(|_| CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))?;
            match work {
                WideWork::SampleUnits(units) => {
                    sampled = sampled.checked_add(units).ok_or(CompositionError::Invalid("composition work overflow"))?;
                    if sampled > 36_000_000 * 512 {
                        return Err(CompositionError::Invalid("composition command work limit"));
                    }
                }
                WideWork::ReserveBytes(bytes) => {
                    resident = resident.checked_add(bytes).ok_or(CompositionError::Invalid("composition resident overflow"))?;
                    if resident > render_limit {
                        return Err(CompositionError::Invalid("composition command resident limit"));
                    }
                }
                WideWork::ReleaseBytes(bytes) => {
                    resident = resident.checked_sub(bytes).ok_or(CompositionError::Invalid("composition resident accounting underflow"))?
                }
            }
            Ok(())
        };
        let mut y = bounds.y0;
        while y < bounds.y1 {
            control.check().map_err(|_| EngineError::Cancelled)?;
            let y1 = y.saturating_add(128).min(bounds.y1);
            let mut x = bounds.x0;
            while x < bounds.x1 {
                control.check().map_err(|_| EngineError::Cancelled)?;
                let x1 = x.saturating_add(128).min(bounds.x1);
                let tile = photocraft_doc::Rect::new(x, y, x1, y1);
                let released = if let Some(scene) = &scene {
                    let rendered = scene
                        .render(tile, control, &mut poll)
                        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, format!("unqualified composition samples: {e}")) })?;
                    let released = u64::try_from(rendered.buffer().px.len())
                        .ok()
                        .and_then(|pixels| pixels.checked_mul(32))
                        .ok_or_else(|| bad(SET, "composition output residency overflow"))?;
                    drop(rendered);
                    released
                } else {
                    let renderer = prepared.as_ref().ok_or_else(|| bad(SET, "missing composition renderer"))?;
                    let rendered = renderer
                        .render_with_interrupt(tile, &mut poll, control)
                        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, format!("unqualified composition samples: {e}")) })?;
                    // The renderer transfers this reservation to its returned buffer.
                    // Release it after dropping each staged tile, not at the end of the whole canvas.
                    let released = u64::try_from(rendered.px.len())
                        .ok()
                        .and_then(|pixels| pixels.checked_mul(32))
                        .ok_or_else(|| bad(SET, "composition output residency overflow"))?;
                    drop(rendered);
                    released
                };
                poll(photocraft_cms::WideWork::ReleaseBytes(released))
                    .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, e.to_string()) })?;
                x = x1;
            }
            y = y1;
        }
        Ok(())
    })?;
    Ok(json!({"layerId":id.0.to_string(),"sourceMode":mode,"updated":true}))
}
fn set(s: &mut Session, value: &Value) -> Result<Value> {
    set_with_interrupt(s, value, &photocraft_raster::Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: INSPECT,
            label: "Inspect GIMP Composition",
            menu: &[],
            shortcut: None,
            params: "{layerId:decimal string} -> {layerId,composition:stored and resolved policy|null}",
            enabled,
            run: inspect,
            journal: false,
        },
        CommandSpec {
            id: SET,
            label: "Set GIMP Composition",
            menu: &[],
            shortcut: None,
            params: "{layerId:decimal string,sourceMode:integer,legacy:false,blendSpace:linear-rgb|profile-rgb|perceptual-rgb,compositeSpace:...,operator:union|clip-to-backdrop|clip-to-layer|intersection,documentId?:canonical decimal string,expectedRevision?:canonical decimal string}; captured target fields must be supplied together",
            enabled,
            run: set,
            journal: true,
        },
    ]
}
