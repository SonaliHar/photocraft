//! Current opaque literal-color edits in already qualified regenerated vectors.
//! Paths, source caches, immutable provenance and all other style attributes are
//! preserved. The existing vector consumer owns encoding/profile qualification.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerId,
    item_identity::{ItemRef, PathId},
    xcf_filters::{FilterArgument, FilterValue},
    xcf_items::EditableSource,
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

const FILL: &str = "xcf.vector.fill.color.set";
const STROKE: &str = "xcf.vector.stroke.color.set";

/// Bounded candidate metadata for a UI frame, not rendering/enablement proof.
/// Does not validate cache hashes, inspect pixels or access resource bytes/paths.
/// The registered edit always performs the authoritative current renderer checks.
pub fn current_color(document: &Document, layer: LayerId, stroke: bool) -> Option<(PathId, [f32; 4])> {
    if layer.0 == 0
        || document.mode != photocraft_doc::ColorMode::Rgb
        || document.depth != photocraft_doc::SampleType::U8
        || document.sample_encoding != Some(photocraft_doc::SampleEncoding::NonlinearProfile)
        || !document.source_color_context.as_ref().is_some_and(|context| context.source_precision == 150 && context.original_icc.is_none())
        || document.icc_profile.is_some()
            && !matches!(document.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. })
    {
        return None;
    }
    let state = document.xcf_state.as_ref()?;
    if state.provenance.format != "xcf"
        || state.provenance.format_version != 24
        || state.source_items.len() > 10_000
        || document.paths.len() > 10_000
        || !peek_layer_exists(document, layer)
    {
        return None;
    }
    let mut matches = state.source_items.iter().filter(|item| item.owner == ItemRef::Layer(layer));
    let source = matches.next()?;
    if matches.next().is_some() || source.transform.is_some() {
        return None;
    }
    let EditableSource::Vector { path, source_modified: false, configuration } = &source.source else { return None };
    if path.0 == 0
        || document.paths.iter().filter(|candidate| candidate.id == *path).count() != 1
        || !peek_fields(configuration, &["source-path-tattoo", "filled", "stroked", "fill", "stroke"])
    {
        return None;
    }
    let filled = peek_field(configuration, "filled")?;
    let stroked = peek_field(configuration, "stroked")?;
    if filled != &FilterValue::Bool(!stroke) || stroked != &FilterValue::Bool(stroke) {
        return None;
    }
    let fill = peek_config(configuration, "fill")?;
    let stroke_style = peek_config(configuration, "stroke")?;
    if !peek_fields(stroke_style, &["fill", "width", "cap-style", "join-style", "miter-limit", "dashes"]) {
        return None;
    }
    let stroke_fill = peek_config(stroke_style, "fill")?;
    let selected = if stroke { stroke_fill } else { fill };
    let literal = peek_literal(selected)?;
    // Both styles must retain closed literal descriptors, including the inactive
    // style. Geometry, exact width/AA combinations and pattern authority remain
    // renderer-owned and deliberately are not inferred from this metadata peek.
    peek_literal(if stroke { fill } else { stroke_fill })?;
    Some((*path, literal))
}

fn peek_field<'a>(values: &'a [FilterArgument], name: &str) -> Option<&'a FilterValue> {
    values.iter().find(|argument| argument.name == name).map(|argument| &argument.value)
}
fn peek_fields(values: &[FilterArgument], names: &[&str]) -> bool {
    values.len() == names.len() && names.iter().all(|name| values.iter().filter(|argument| argument.name == *name).count() == 1)
}
fn peek_config<'a>(values: &'a [FilterArgument], name: &str) -> Option<&'a [FilterArgument]> {
    match peek_field(values, name)? {
        FilterValue::Config(values) => Some(values),
        _ => None,
    }
}
fn peek_literal(values: &[FilterArgument]) -> Option<[f32; 4]> {
    if !peek_fields(values, &["style", "antialias", "color", "pattern"])
        || !matches!(peek_field(values, "style")?, FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value: 0 }))
        || !matches!(peek_field(values, "antialias")?, FilterValue::Bool(_))
        || !matches!(peek_field(values, "pattern")?, FilterValue::Resource(id) if id.0 != 0)
    {
        return None;
    }
    let color = peek_config(values, "color")?;
    if !peek_fields(color, &["encoding", "native-samples-hex"])
        || !matches!(peek_field(color, "encoding")?, FilterValue::String(encoding) if encoding == "R'G'B'A float")
    {
        return None;
    }
    let FilterValue::String(hex) = peek_field(color, "native-samples-hex")? else { return None };
    if hex.len() != 32 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut raw = [0u8; 16];
    for (byte, pair) in raw.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    let mut rgba = [0.0; 4];
    for (component, bytes) in rgba.iter_mut().zip(raw.as_chunks::<4>().0) {
        *component = f32::from_le_bytes(*bytes);
        if !component.is_finite() || !(0.0..=1.0).contains(component) {
            return None;
        }
    }
    if rgba.last() != Some(&1.0) {
        return None;
    }
    Some(rgba)
}
fn peek_layer_exists(document: &Document, target: LayerId) -> bool {
    if document.layers.len() > 10_000 {
        return false;
    }
    let mut frames: [Option<std::slice::Iter<'_, photocraft_doc::Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    let Some(first) = frames.first_mut() else { return false };
    *first = Some(document.layers.iter());
    let (mut depth, mut visits, mut found) = (0usize, 0usize, false);
    loop {
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                return found;
            }
            depth -= 1;
            continue;
        };
        visits = match visits.checked_add(1).filter(|visits| *visits <= 10_000) {
            Some(visits) => visits,
            None => return false,
        };
        if layer.id == target {
            if found || !matches!(layer.content, photocraft_doc::LayerContent::Raster(_)) {
                return false;
            }
            found = true;
        }
        if let photocraft_doc::LayerContent::Group(group) = &layer.content {
            depth = match depth.checked_add(1).filter(|depth| *depth <= photocraft_doc::MAX_GROUP_DEPTH) {
                Some(depth) => depth,
                None => return false,
            };
            let Some(frame) = frames.get_mut(depth) else { return false };
            *frame = Some(group.children.iter());
        }
    }
}
fn bad(cmd: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn identity(cmd: &str, params: &Value, name: &str) -> Result<u64> {
    let text = params.get(name).and_then(Value::as_str).ok_or_else(|| bad(cmd, "identity requires canonical nonzero decimal string"))?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad(cmd, "noncanonical layer/path identity"));
    }
    text.parse().map_err(|_| bad(cmd, "identity overflow"))
}
fn colors(cmd: &str, params: &Value) -> Result<[u32; 4]> {
    let values = params
        .get("rgbaF32Bits")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 4)
        .ok_or_else(|| bad(cmd, "rgbaF32Bits requires four lowercase hexadecimal8 strings"))?;
    let mut bits = [0; 4];
    for (index, (value, bits)) in values.iter().zip(&mut bits).enumerate() {
        let text = value
            .as_str()
            .filter(|text| text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
            .ok_or_else(|| bad(cmd, "color bits require lowercase hexadecimal8"))?;
        *bits = u32::from_str_radix(text, 16).map_err(|_| bad(cmd, "invalid color bits"))?;
        let sample = f32::from_bits(*bits);
        if !sample.is_finite() || !(0.0..=1.0).contains(&sample) || index == 3 && sample != 1.0 {
            return Err(bad(cmd, "only qualified finite unit opaque literal colors are editable"));
        }
    }
    Ok(bits)
}
fn envelope(cmd: &str, params: &Value) -> Result<()> {
    if !matches!(cmd, FILL | STROKE) {
        return Err(bad(cmd, "not a vector color command"));
    }
    let fields = params.as_object().ok_or_else(|| bad(cmd, "expected parameter object"))?;
    if fields.len() != 3 || fields.keys().any(|key| !matches!(key.as_str(), "layerId" | "pathId" | "rgbaF32Bits")) {
        return Err(bad(cmd, "exact layerId/pathId/rgbaF32Bits parameters required"));
    }
    identity(cmd, params, "layerId")?;
    identity(cmd, params, "pathId")?;
    colors(cmd, params)?;
    Ok(())
}
fn context(session: &Session, cmd: &str) -> Result<()> {
    if session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
    {
        return Err(bad(cmd, "vector color edit requires synchronous uncoalesced context without jobs"));
    }
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
        return Err(bad(cmd, "vector color edit refuses any floating selection/attachment"));
    }
    if active.revision == u64::MAX || active.doc.accounted().is_some() {
        return Err(bad(cmd, "vector color edit requires resident storage and revision headroom"));
    }
    Ok(())
}
/// Root calls this before float normalization, job setup and parameter cloning.
pub fn dispatch_preflight(session: &Session, cmd: &str, params: &Value) -> Result<()> {
    context(session, cmd)?;
    envelope(cmd, params)
}

fn configuration<'a>(cmd: &str, values: &'a [FilterArgument], name: &str) -> Result<&'a [FilterArgument]> {
    match &values.iter().find(|argument| argument.name == name).ok_or_else(|| bad(cmd, "missing qualified style field"))?.value {
        FilterValue::Config(values) => Ok(values),
        _ => Err(bad(cmd, "invalid qualified style field")),
    }
}
fn sample_hex<'a>(cmd: &str, values: &'a [FilterArgument]) -> Result<&'a str> {
    let fill = if cmd == STROKE { configuration(cmd, configuration(cmd, values, "stroke")?, "fill")? } else { configuration(cmd, values, "fill")? };
    let color = configuration(cmd, fill, "color")?;
    match &color.iter().find(|argument| argument.name == "native-samples-hex").ok_or_else(|| bad(cmd, "missing literal samples"))?.value {
        FilterValue::String(samples) => Ok(samples),
        _ => Err(bad(cmd, "invalid literal samples")),
    }
}
fn nested_mut<'a>(cmd: &str, values: &'a mut [FilterArgument], name: &str) -> Result<&'a mut Vec<FilterArgument>> {
    match &mut values.iter_mut().find(|argument| argument.name == name).ok_or_else(|| bad(cmd, "missing staged style field"))?.value {
        FilterValue::Config(values) => Ok(values),
        _ => Err(bad(cmd, "invalid staged style field")),
    }
}
fn set_hex(doc: &mut Document, index: usize, cmd: &str, replacement: String) -> Result<()> {
    let item = doc.xcf_state.as_mut().and_then(|state| state.source_items.get_mut(index)).ok_or_else(|| bad(cmd, "missing staged vector source"))?;
    let EditableSource::Vector { configuration, source_modified: false, .. } = &mut item.source else {
        return Err(bad(cmd, "staged vector is source-modified"));
    };
    let fill = if cmd == STROKE { nested_mut(cmd, nested_mut(cmd, configuration, "stroke")?, "fill")? } else { nested_mut(cmd, configuration, "fill")? };
    let color = nested_mut(cmd, fill, "color")?;
    let sample = &mut color.iter_mut().find(|argument| argument.name == "native-samples-hex").ok_or_else(|| bad(cmd, "missing staged literal samples"))?.value;
    if !matches!(sample, FilterValue::String(_)) {
        return Err(bad(cmd, "invalid staged literal sample type"));
    }
    *sample = FilterValue::String(replacement);
    Ok(())
}
fn source_index(doc: &Document, layer: LayerId, path: PathId, cmd: &str, control: &Interrupt<'_>) -> Result<usize> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad(cmd, "missing vector state"))?;
    if state.source_items.len() > 10_000 || state.items.len() > 10_000 || doc.paths.len() > 10_000 {
        return Err(bad(cmd, "vector relationship census limit"));
    }
    doc.validate_xcf_state_with(control).map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, error.to_string()) })?;
    if doc.paths.iter().filter(|entry| entry.id == path).count() != 1 {
        return Err(bad(cmd, "unknown canonical path identity"));
    }
    let mut matches = state.source_items.iter().enumerate().filter(|(_, item)| item.owner == ItemRef::Layer(layer));
    let (index, item) = matches.next().ok_or_else(|| bad(cmd, "unknown vector owner"))?;
    if matches.next().is_some() {
        return Err(bad(cmd, "duplicate vector owner"));
    }
    let EditableSource::Vector { path: bound, source_modified: false, configuration } = &item.source else {
        return Err(bad(cmd, "current vector source is not safely regenerable"));
    };
    if *bound != path {
        return Err(bad(cmd, "path identity does not belong to vector owner"));
    }
    let filled = configuration.iter().find(|argument| argument.name == "filled").map(|argument| &argument.value);
    let stroked = configuration.iter().find(|argument| argument.name == "stroked").map(|argument| &argument.value);
    if (cmd == FILL && (filled != Some(&FilterValue::Bool(true)) || stroked != Some(&FilterValue::Bool(false))))
        || (cmd == STROKE && (filled != Some(&FilterValue::Bool(false)) || stroked != Some(&FilterValue::Bool(true))))
    {
        return Err(bad(cmd, "only the operative fill-only or stroke-only literal is editable"));
    }
    let locks = crate::xcf_item_locks::effective_layer_locks(doc, layer).map_err(|error| bad(cmd, error.to_string()))?;
    if locks.imported.content || locks.native.all || locks.native.pixels {
        return Err(bad(cmd, "vector owner or inherited content is locked"));
    }
    photocraft_compose::gimp_vector_items::qualify_source(doc, item, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, format!("unqualified current vector source: {error}")) })?;
    Ok(index)
}
fn literal_hex(cmd: &str, bits: [u32; 4]) -> Result<String> {
    let mut hex = String::new();
    hex.try_reserve_exact(32).map_err(|_| bad(cmd, "literal allocation refused"))?;
    for byte in bits.into_iter().flat_map(u32::to_le_bytes) {
        for nibble in [byte >> 4, byte & 15] {
            hex.push(char::from(*b"0123456789abcdef".get(usize::from(nibble)).ok_or_else(|| bad(cmd, "literal encoding failed"))?));
        }
    }
    Ok(hex)
}
pub fn set_with_interrupt(session: &mut Session, cmd: &str, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    dispatch_preflight(session, cmd, params)?;
    let layer = LayerId(identity(cmd, params, "layerId")?);
    let path = PathId(identity(cmd, params, "pathId")?);
    let bits = colors(cmd, params)?;
    let replacement = literal_hex(cmd, bits)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    active.doc.validate_resident().map_err(|error| bad(cmd, error.to_string()))?;
    let document_id = active.doc.id;
    let revision = active.revision;
    let snapshot = active.doc.downgrade();
    let before: &Document = &active.doc;
    before.validate_xcf_state_with(control).map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, error.to_string()) })?;
    crate::xcf_filter_insert_cmds::headroom(before, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, format!("vector staging headroom: {error}")) })?;
    let index = source_index(before, layer, path, cmd, control)?;
    let item = before.xcf_state.as_ref().and_then(|state| state.source_items.get(index)).ok_or_else(|| bad(cmd, "missing current vector source"))?;
    let EditableSource::Vector { configuration, .. } = &item.source else { return Err(bad(cmd, "missing vector configuration")) };
    let changed = !sample_hex(cmd, configuration)?.eq_ignore_ascii_case(&replacement);
    crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(before, control)?;
    if changed {
        let mut candidate = before.clone();
        set_hex(&mut candidate, index, cmd, replacement.clone())?;
        source_index(&candidate, layer, path, cmd, control)?;
        crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&candidate, control)?;
    }
    let response = json!({"layerId":layer.0.to_string(),"pathId":path.0.to_string(),"rgbaF32Bits":bits.map(|bits| format!("{bits:08x}")),"updated":changed});
    control.check().map_err(|_| EngineError::Cancelled)?;
    context(session, cmd)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let original = snapshot.upgrade().ok_or_else(|| bad(cmd, "stale vector style target"))?;
    if active.doc.id != document_id || active.revision != revision || !original.shares_state(&active.doc) {
        return Err(bad(cmd, "stale vector style target"));
    }
    if changed {
        session.edit_with_interrupt(
            if cmd == FILL { "Edit Imported Vector Fill Color" } else { "Edit Imported Vector Stroke Color" },
            control,
            move |doc, _| {
                let index = source_index(doc, layer, path, cmd, control)?;
                set_hex(doc, index, cmd, replacement)?;
                source_index(doc, layer, path, cmd, control)?;
                crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(doc, control)?;
                control.check().map_err(|_| EngineError::Cancelled)
            },
        )?;
    }
    Ok(response)
}
fn set_fill(session: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(session, FILL, params, &Interrupt::NONE)
}
fn set_stroke(session: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(session, STROKE, params, &Interrupt::NONE)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session, FILL).map_err(|error| error.to_string())?;
    session
        .active()
        .and_then(|state| state.doc.xcf_state.as_ref())
        .filter(|state| state.source_items.iter().any(|item| matches!(item.source, EditableSource::Vector { source_modified: false, .. })))
        .map(|_| ())
        .ok_or_else(|| "no regenerated current vector source".into())
}
pub fn specs() -> Vec<CommandSpec> {
    [(FILL, "Edit Imported Vector Fill Color", set_fill as fn(&mut Session, &Value) -> Result<Value>), (STROKE, "Edit Imported Vector Stroke Color", set_stroke)].into_iter().map(|(id, label, run)| CommandSpec { id, label, menu: &[], shortcut: None, params: "{layerId,pathId:canonical decimal strings,rgbaF32Bits:[four lowercase hexadecimal8 strings]}; operative opaque literal of already qualified fill-only/stroke-only regenerated source", enabled, run, journal: true }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_bits_are_lossless_little_endian_and_closed_opaque_domain() {
        let params = json!({"layerId":"1","pathId":"2","rgbaF32Bits":["80000000","3f000000","3f400000","3f800000"]});
        envelope(FILL, &params).unwrap();
        let bits = colors(FILL, &params).unwrap();
        assert_eq!(bits, [(-0.0_f32).to_bits(), 0.5_f32.to_bits(), 0.75_f32.to_bits(), 1.0_f32.to_bits()]);
        assert_eq!(literal_hex(FILL, bits).unwrap(), "000000800000003f0000403f0000803f");
        for value in ["7fc00000", "7f800000", "3f800001", "BF000000", "0"] {
            let mut hostile = params.clone();
            hostile["rgbaF32Bits"][0] = json!(value);
            assert!(envelope(FILL, &hostile).is_err());
        }
        let mut translucent = params.clone();
        translucent["rgbaF32Bits"][3] = json!("3f000000");
        assert!(envelope(STROKE, &translucent).is_err());
    }

    #[test]
    fn direct_inherited_context_refuses_without_clearing_or_decoding_sources() {
        let mut session = Session::new();
        session.coalesce_request = Some("retained".into());
        assert!(dispatch_preflight(&session, FILL, &Value::Null).unwrap_err().to_string().contains("uncoalesced"));
        assert_eq!(session.coalesce_request.as_deref(), Some("retained"));
        assert!(session.documents().is_empty());
        assert!(session.journal.is_empty());
    }

    #[test]
    #[ignore = "requires retained qualified vector source; command/native/Undo continuation draft"]
    fn registered_active_stroke_color_preserves_source_cache_path_and_native_undo() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("reviewed retained corpus"));
        let source = photocraft_io::import("source.xcf", &std::fs::read(root.join("vector-stroke-evidence/source.xcf")).unwrap()).unwrap().document;
        let item = source
            .xcf_state
            .as_ref()
            .unwrap()
            .source_items
            .iter()
            .find(|item| matches!(item.source, EditableSource::Vector { source_modified: false, .. }))
            .unwrap();
        let ItemRef::Layer(owner) = item.owner else { panic!("qualified vector layer required") };
        let EditableSource::Vector { path, configuration, .. } = &item.source else { panic!("vector path required") };
        let path = *path;
        let red = [1.0_f32.to_bits(), 0, 0, 1.0_f32.to_bits()];
        let chosen = if sample_hex(STROKE, configuration).unwrap().eq_ignore_ascii_case(&literal_hex(STROKE, red).unwrap()) {
            [0, 0, 1.0_f32.to_bits(), 1.0_f32.to_bits()]
        } else {
            red
        };
        let params = json!({"layerId":owner.0.to_string(),"pathId":path.0.to_string(),"rgbaF32Bits":chosen.map(|bits| format!("{bits:08x}"))});
        let original_cache = item.cache.clone();
        let original_native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let original_paths = source.paths.clone();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute(STROKE, params.clone()).unwrap();
        let after = (*session.active().unwrap().doc).clone();
        assert_eq!(after.paths, original_paths);
        assert_eq!(after.xcf_state.as_ref().unwrap().source_items.iter().find(|item| item.owner == ItemRef::Layer(owner)).unwrap().cache, original_cache);
        assert_eq!(after.layers, source.layers);
        assert_eq!(after.xcf_state.as_ref().unwrap().provenance, source.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        assert_eq!(session.execute(STROKE, params).unwrap()["updated"], false);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        assert_eq!(*session.active().unwrap().doc, after);
        let native = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), after);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), original_native);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, after);
    }
}
