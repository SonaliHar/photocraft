//! Narrow current filter insertion from an explicit qualified owner-local template.
//! Original source records, caches and provenance are untouched. No operation,
//! clipping, composition space or literal color default is fabricated.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerContent, LayerId,
    item_identity::{FilterId, ItemRef},
    xcf_filters::FilterNode,
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

const INSERT: &str = "xcf.filter.insert";
const KEYS: &[&str] = &["layerId", "filterId", "templateFilterId", "index", "operation", "arguments", "enabled"];

/// Closed scalar metadata candidate only; no renderer/cache validation per frame.
pub fn template_candidate(node: &FilterNode) -> bool {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    if node.id.0 == 0
        || node.mask.is_some()
        || !node.auxiliary.is_empty()
        || node.arguments.len() > 64
        || node.operation.name.is_empty()
        || node.operation.name.len() > 256
        || node.operation.stored_version.is_empty()
        || node.operation.stored_version.len() > 64
        || node.operation.schema_epoch.is_empty()
        || node.operation.schema_epoch.len() > 256
        || !crate::xcf_filter_cmds::scalar_edit_candidate(node)
    {
        return false;
    }
    let mut bytes = node.operation.name.len() + node.operation.stored_version.len() + node.operation.schema_epoch.len() + 256;
    for (index, argument) in node.arguments.iter().enumerate() {
        if argument.name.is_empty()
            || argument.name.len() > 128
            || argument.name.contains('\0')
            || node.arguments.iter().take(index).any(|old| old.name == argument.name)
        {
            return false;
        }
        bytes = match bytes.checked_add(argument.name.len() + 128) {
            Some(bytes) => bytes,
            None => return false,
        };
        match &argument.value {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) if !f32::from_bits(*bits).is_finite() => return false,
            FilterValue::Number(ExactNumber::Float64Bits(bits)) if !f64::from_bits(*bits).is_finite() => return false,
            FilterValue::Number(_) | FilterValue::Bool(_) => {},
            FilterValue::Enum { domain, .. } if domain.len() <= 256 && !domain.contains('\0') => bytes = match bytes.checked_add(domain.len()) { Some(bytes) => bytes, None => return false },
            FilterValue::Color { components, profile: None, .. } if components.len() == 4 && components.iter().all(|component| matches!(component, ExactNumber::Float32Bits(bits) if f32::from_bits(*bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(*bits)))) => bytes = match bytes.checked_add(512) { Some(bytes) => bytes, None => return false },
            _ => return false,
        }
        if bytes > 12 << 10 {
            return false;
        }
    }
    true
}

/// Borrowed current stack metadata. Actual owner/rendering qualification belongs
/// to the registered command; this supplies no authority to activate a template.
pub fn current_templates(document: &Document, layer: LayerId) -> Option<&photocraft_doc::xcf_filters::FilterStack> {
    if layer.0 == 0 {
        return None;
    }
    let state = document.xcf_state.as_ref()?;
    if state.filter_stacks.len() > 10_000 || state.items.len() > 10_000 {
        return None;
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Layer(layer));
    let item = items.next()?;
    if items.next().is_some() || item.raster_bounds.is_none() {
        return None;
    }
    let mut stacks = state.filter_stacks.iter().filter(|stack| stack.owner == ItemRef::Layer(layer));
    let stack = stacks.next()?;
    if stacks.next().is_some() || stack.revision == u64::MAX || stack.nodes.len() >= 1024 || !stack.nodes.iter().any(template_candidate) {
        return None;
    }
    Some(stack)
}

/// First unused current canonical FilterId in a bounded deterministic range.
/// Original-provenance IDs in other namespaces are never inferred as collisions.
pub fn unused_filter_id(document: &Document) -> Option<FilterId> {
    let state = document.xcf_state.as_ref()?;
    if state.filter_stacks.len() > 10_000 {
        return None;
    }
    let mut used = [false; 10_002];
    let mut visits = 0usize;
    for stack in &state.filter_stacks {
        visits = visits.checked_add(stack.nodes.len()).filter(|visits| *visits <= 10_000)?;
        for node in &stack.nodes {
            if node.id.0 == 0 {
                return None;
            }
            if let Ok(index) = usize::try_from(node.id.0)
                && let Some(slot) = used.get_mut(index)
            {
                *slot = true;
            }
        }
    }
    used.iter().enumerate().skip(1).find(|(_, used)| !**used).and_then(|(index, _)| u64::try_from(index).ok()).map(FilterId)
}
/// Lossless bounded inverse of the flat scalar decoder, for an explicit action.
/// Never called while discovering templates in a UI frame.
pub fn template_wire(node: &FilterNode) -> std::result::Result<(Value, Value), String> {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    fn number(number: &ExactNumber) -> std::result::Result<Value, String> {
        Ok(match number {
            ExactNumber::Float32Bits(bits) => json!({"Float32Bits":decimal_text(*bits)?}),
            ExactNumber::Float64Bits(bits) => json!({"Float64Bits":decimal_text(*bits)?}),
            ExactNumber::Signed { width, value } => json!({"Signed":{"width":width,"value":decimal_text(*value)?}}),
            ExactNumber::Unsigned { width, value } => json!({"Unsigned":{"width":width,"value":decimal_text(*value)?}}),
        })
    }
    if !template_candidate(node) {
        return Err("xcf.filter.insert.ui.unsupported-template".into());
    }
    let operation = json!({"name":node.operation.name,"stored_version":node.operation.stored_version,"schema_epoch":node.operation.schema_epoch});
    let mut arguments = Vec::new();
    arguments.try_reserve_exact(node.arguments.len()).map_err(|_| "xcf.filter.insert.ui.allocation".to_owned())?;
    for argument in &node.arguments {
        let value = match &argument.value {
            FilterValue::Number(value) => json!({"Number":number(value)?}),
            FilterValue::Bool(value) => json!({"Bool":value}),
            FilterValue::Enum { domain, value } => json!({"Enum":{"domain":domain,"value":decimal_text(*value)?}}),
            FilterValue::Color { components, encoding, profile: None } => {
                let mut values = Vec::new();
                values.try_reserve_exact(components.len()).map_err(|_| "xcf.filter.insert.ui.allocation".to_owned())?;
                for value in components {
                    values.push(number(value)?);
                }
                json!({"Color":{"components":values,"encoding":encoding,"profile":null}})
            }
            _ => return Err("xcf.filter.insert.ui.unsupported-template".into()),
        };
        arguments.push(json!({"name":argument.name,"value":value}));
    }
    let arguments = Value::Array(arguments);
    crate::xcf_filter_wire::preflight_filter(&operation, &arguments)?;
    Ok((operation, arguments))
}
fn decimal_text(value: impl std::fmt::Display) -> std::result::Result<String, String> {
    use std::fmt::Write;
    let mut text = String::new();
    text.try_reserve_exact(21).map_err(|_| "xcf.filter.insert.ui.allocation".to_owned())?;
    write!(&mut text, "{value}").map_err(|_| "xcf.filter.insert.ui.allocation".to_owned())?;
    Ok(text)
}
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: INSERT.into(), msg: message.into() }
}
fn decimal(params: &Value, name: &str) -> Result<u64> {
    let text = params.get(name).and_then(Value::as_str).ok_or_else(|| bad(format!("{name} requires a canonical nonzero decimal string")))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) || text.starts_with('0') {
        return Err(bad("noncanonical filter/layer identity"));
    }
    text.parse::<u64>().map_err(|_| bad("filter/layer identity overflow"))
}
fn envelope(params: &Value) -> Result<()> {
    let fields = params.as_object().ok_or_else(|| bad("expected insert object"))?;
    if fields.len() != KEYS.len() || fields.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(bad("unknown or missing insert field"));
    }
    for name in ["layerId", "filterId", "templateFilterId"] {
        decimal(params, name)?;
    }
    params.get("index").and_then(Value::as_u64).filter(|index| *index < 1024).ok_or_else(|| bad("index requires a bounded owner-local integer"))?;
    params.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad("enabled requires a Boolean"))?;
    crate::xcf_filter_wire::preflight_filter(
        params.get("operation").ok_or_else(|| bad("missing operation"))?,
        params.get("arguments").ok_or_else(|| bad("missing arguments"))?,
    )
    .map_err(bad)
}
fn context(session: &Session) -> Result<()> {
    if session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
    {
        return Err(bad("insert requires synchronous uncoalesced context without jobs"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.floating.is_some() || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
        return Err(bad("insert refuses any existing floating selection/attachment"));
    }
    if state.revision == u64::MAX || state.doc.accounted().is_some() {
        return Err(bad("insert requires resident editable storage and revision headroom"));
    }
    Ok(())
}
/// Required early dispatcher seam, before normalization/job setup/params clone.
pub fn dispatch_preflight(session: &Session, params: &Value) -> Result<()> {
    context(session)?;
    envelope(params)
}

/// Explicit measured payload/structural capacity slice, not a whole-document RSS
/// promise: legacy text/effect/vector/private map overhead remains unmeasured.
pub(crate) fn headroom(doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    let mut limits = photocraft_io::NativeSaveLimits::default().preparation;
    limits.resident_bytes = limits.resident_bytes.min(128 << 20);
    limits.object_bytes = limits.object_bytes.min(128 << 20);
    limits.index_bytes = limits.index_bytes.min(4 << 20);
    limits.manifest_bytes = limits.manifest_bytes.min(16 << 20);
    photocraft_io::retained_native_input_bytes(doc, limits, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(format!("insert retained payload/capacity headroom: {error}")) })?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing imported filter state"))?;
    struct Counter<'a, 'b> {
        bytes: usize,
        calls: usize,
        control: &'a Interrupt<'b>,
    }
    impl std::io::Write for Counter<'_, '_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.calls = self.calls.checked_add(1).ok_or_else(|| std::io::Error::other("metadata work overflow"))?;
            self.bytes = self.bytes.checked_add(bytes.len()).ok_or_else(|| std::io::Error::other("metadata size overflow"))?;
            if self.control.cancelled() || self.calls > 1_000_000 || self.bytes > 16 << 20 {
                return Err(std::io::Error::other("insert metadata work/size/cancellation limit"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.control.cancelled() { Err(std::io::Error::other("cancelled")) } else { Ok(()) }
        }
    }
    let mut counter = Counter { bytes: 0, calls: 0, control };
    serde_json::to_writer(&mut counter, state)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(format!("insert metadata headroom: {error}")) })?;
    control.check().map_err(|_| EngineError::Cancelled)
}

fn insert(session: &mut Session, params: &Value) -> Result<Value> {
    insert_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn insert_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    dispatch_preflight(session, params)?;
    let owner = LayerId(decimal(params, "layerId")?);
    let id = FilterId(decimal(params, "filterId")?);
    let template_id = FilterId(decimal(params, "templateFilterId")?);
    let index = params.get("index").and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok()).ok_or_else(|| bad("index overflow"))?;
    let enabled = params.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad("enabled requires a Boolean"))?;
    let operation = crate::xcf_filter_wire::decode_operation(params.get("operation").ok_or_else(|| bad("missing operation"))?).map_err(bad)?;
    let arguments = crate::xcf_filter_wire::decode_arguments(params.get("arguments").ok_or_else(|| bad("missing arguments"))?).map_err(bad)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let document_id = active.doc.id;
    let revision = active.revision;
    let snapshot = active.doc.downgrade();
    active.doc.validate_resident().map_err(|error| bad(format!("insert requires resident storage: {error}")))?;
    let document: &Document = &active.doc;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing imported filter state"))?;
    if state.filter_stacks.len() > 10_000 {
        return Err(bad("filter stack census limit"));
    }
    let mut visits = 0usize;
    for stack in &state.filter_stacks {
        control.check().map_err(|_| EngineError::Cancelled)?;
        visits = visits.checked_add(stack.nodes.len()).filter(|count| *count <= 10_000).ok_or_else(|| bad("filter identity census limit"))?;
        if stack.nodes.len() > 1024 || stack.nodes.iter().any(|node| node.id == id) {
            return Err(bad("new filterId in use or owner stack exceeds limit"));
        }
    }
    document.validate_xcf_state_with(control).map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(error.to_string()) })?;
    headroom(document, control)?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing imported filter state"))?;
    let stack =
        state.filter_stacks.iter().find(|stack| stack.owner == ItemRef::Layer(owner)).ok_or_else(|| bad("existing owner-local filter stack required"))?;
    if stack.nodes.len() >= 1024 || index > stack.nodes.len() || stack.revision == u64::MAX {
        return Err(bad("stack index/count/revision headroom refused"));
    }
    let template =
        stack.nodes.iter().find(|node| node.id == template_id).ok_or_else(|| bad("templateFilterId must be in the selected owner's current stack"))?;
    if template.operation != operation
        || template.mask.is_some()
        || !template.auxiliary.is_empty()
        || !crate::xcf_filter_cmds::scalar_edit_candidate(template)
        || !crate::xcf_filter_cmds::template_arguments_compatible(template, &arguments)
    {
        return Err(bad("insert requires exact qualified unmasked/no-aux scalar template/configuration"));
    }
    if arguments.iter().any(|argument| matches!(&argument.value, photocraft_doc::xcf_filters::FilterValue::Color { components, profile, .. } if components.len() != 4 || profile.is_some() || components.iter().any(|component| !matches!(component, photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))))) {
        return Err(bad("insert color literals require the existing four-Float32 no-resource template domain"));
    }
    let locks = crate::xcf_item_locks::effective_layer_locks(document, owner).map_err(|error| bad(error.to_string()))?;
    if locks.imported.content
        || locks.imported.visibility
        || locks.native.all
        || locks.native.pixels
        || !document.layer(owner).is_some_and(|layer| matches!(layer.content, LayerContent::Raster(_)))
    {
        return Err(bad("insert owner/inherited locks or non-raster owner refused"));
    }
    let node = FilterNode {
        id,
        operation,
        source_display_name: None,
        source_icon: None,
        arguments,
        enabled,
        opacity_f64_bits: template.opacity_f64_bits,
        source_clipping: template.source_clipping,
        source_region: template.source_region,
        mask: None,
        auxiliary: Vec::new(),
        composition: template.composition.clone(),
    };
    crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(document, control)?;
    // Even a disabled template must be qualified as an operative source pipeline.
    {
        let mut qualified = document.clone();
        let template = qualified
            .xcf_state
            .as_mut()
            .and_then(|state| state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)))
            .and_then(|stack| stack.nodes.iter_mut().find(|node| node.id == template_id))
            .ok_or_else(|| bad("missing staged template"))?;
        template.enabled = true;
        crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&qualified, control)?;
    }
    let response = json!({"layerId":owner.0.to_string(),"filterId":id.0.to_string(),"templateFilterId":template_id.0.to_string(),"index":index,"updated":true});
    control.check().map_err(|_| EngineError::Cancelled)?;
    context(session)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let original = snapshot.upgrade().ok_or_else(|| bad("stale insert target"))?;
    if active.doc.id != document_id || active.revision != revision || !original.shares_state(&active.doc) {
        return Err(bad("stale insert target"));
    }
    session.edit_with_interrupt("Insert Imported Filter from Template", control, move |doc, _| {
        let stack = doc
            .xcf_state
            .as_mut()
            .and_then(|state| state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)))
            .ok_or_else(|| bad("missing staged filter owner"))?;
        if index > stack.nodes.len() || stack.nodes.len() >= 1024 || stack.nodes.iter().any(|old| old.id == id) {
            return Err(bad("stale staged index/filter identity"));
        }
        stack.nodes.try_reserve(1).map_err(|_| bad("filter insertion allocation refused"))?;
        stack.nodes.insert(index, node);
        stack.revision = stack.revision.checked_add(1).ok_or_else(|| bad("filter stack revision overflow"))?;
        headroom(doc, control)?;
        if !enabled {
            let mut qualified = doc.clone();
            let inserted = qualified
                .xcf_state
                .as_mut()
                .and_then(|state| state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)))
                .and_then(|stack| stack.nodes.iter_mut().find(|node| node.id == id))
                .ok_or_else(|| bad("missing staged inserted filter"))?;
            inserted.enabled = true;
            crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(&qualified, control)?;
        }
        crate::xcf_filter_cmds::validate_current_pipeline_with_interrupt(doc, control)?;
        control.check().map_err(|_| EngineError::Cancelled)
    })?;
    Ok(response)
}

fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session).map_err(|error| error.to_string())?;
    session
        .active()
        .and_then(|state| state.doc.xcf_state.as_ref())
        .filter(|state| !state.filter_stacks.is_empty())
        .map(|_| ())
        .ok_or_else(|| "no current imported filter stack".into())
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: INSERT,
        label: "Insert Imported Filter from Template",
        menu: &[],
        shortcut: None,
        params: "{layerId,filterId,templateFilterId:canonical decimal strings,index:owner-local integer,operation:exact OperationIdentity,arguments:complete bounded typed scalar values,enabled:bool}; qualified same-owner unmasked/no-aux template only",
        enabled,
        run: insert,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        ColorMode, Layer, SampleEncoding, SampleType, Size, XcfState,
        import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
        item_identity::ItemRef,
        xcf_filters::{ExactNumber, FilterArgument, FilterStack, FilterValue, OperationIdentity},
    };

    fn fixture() -> Document {
        let mut doc = Document::new("Own template insertion", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        doc.sample_encoding = Some(SampleEncoding::Linear);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let mut layer = Layer::raster("Own finite input", doc.pixel_format());
        layer.surface_mut().unwrap().try_write_region_f64(doc.bounds(), &[0.25, 0.5, 0.75, 1.0]).unwrap();
        let owner = layer.id;
        doc.layers.push(layer);
        let mut state = XcfState::default();
        state.provenance.format_version = 22;
        state.reconcile_layers(&doc).unwrap();
        let item = state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap();
        item.raster_bounds = Some(doc.bounds());
        item.composition = Some(GimpComposition {
            compatibility_epoch: "own-current-template-policy".into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 1,
            stored_composite_space: 1,
            stored_operator: 1,
            pass_through: false,
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        });
        state.filter_stacks.push(FilterStack {
            owner: ItemRef::Layer(owner),
            revision: 0,
            nodes: vec![FilterNode {
                id: FilterId(999_999),
                operation: OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: "xcf-22-stock-point-replace".into() },
                source_display_name: Some("Original own exposure".into()),
                source_icon: Some("own-icon".into()),
                arguments: vec![
                    FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.0_f32.to_bits())) },
                    FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.0_f32.to_bits())) },
                ],
                enabled: true,
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: Vec::new(),
                composition: Some(GimpComposition {
                    compatibility_epoch: "xcf-22-stock-point-replace".into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -261651887,
                    stored_composite_space: -261651887,
                    stored_operator: -261651887,
                    pass_through: false,
                    resolved: None,
                }),
            }],
        });
        doc.xcf_state = Some(state);
        doc
    }
    fn params(doc: &Document) -> Value {
        let stack = &doc.xcf_state.as_ref().unwrap().filter_stacks[0];
        json!({"layerId":stack.owner.raw().to_string(),"filterId":"999998","templateFilterId":"999999","index":1,
            "operation":stack.nodes[0].operation,"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.5_f32.to_bits().to_string()}}},{"name":"black-level","value":{"Number":{"Float32Bits":"0"}}}],"enabled":true})
    }

    #[test]
    fn registered_insert_preserves_template_source_and_has_one_exact_native_undo_redo() {
        let before = fixture();
        let template = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
        let before_native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(before.clone(), None).unwrap();
        session.execute(INSERT, params(&before)).unwrap();
        let after = (*session.active().unwrap().doc).clone();
        let state = after.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks[0].nodes[0], template);
        assert_eq!(state.filter_stacks[0].nodes[1].source_display_name, None);
        assert_eq!(state.filter_stacks[0].nodes[1].source_icon, None);
        assert_eq!(state.filter_stacks[0].revision, 1);
        assert_eq!(after.layers, before.layers);
        assert_eq!(state.provenance, before.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(state.source_items, before.xcf_state.as_ref().unwrap().source_items);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        let native = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), after);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), before_native);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, after);
    }

    #[test]
    fn refusal_preserves_document_and_real_redo_without_consuming_template() {
        let before = fixture();
        let mut session = Session::new();
        session.add_document(before.clone(), None).unwrap();
        session
            .edit("Seed real redo", |doc, _| {
                doc.name = "Seeded successor".into();
                Ok(())
            })
            .unwrap();
        let successor = (*session.active().unwrap().doc).clone();
        session.execute("edit.undo", json!({})).unwrap();
        let mut request = params(&before);
        request["filterId"] = json!("999999");
        assert!(session.execute(INSERT, request).is_err());
        assert_eq!(*session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, successor);
    }

    #[test]
    fn insertion_wire_preserves_double_words_and_unused_ids_are_bounded_deterministically() {
        let mut document = fixture();
        let state = document.xcf_state.as_mut().unwrap();
        let template = &mut state.filter_stacks[0].nodes[0];
        template.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits((-0.0_f64).to_bits()));
        let (operation, arguments) = template_wire(template).unwrap();
        assert_eq!(arguments[0]["value"]["Number"]["Float64Bits"], "9223372036854775808");
        assert_eq!(crate::xcf_filter_wire::decode_operation(&operation).unwrap(), template.operation);
        assert_eq!(crate::xcf_filter_wire::decode_arguments(&arguments).unwrap(), template.arguments);
        assert_eq!(unused_filter_id(&document), Some(FilterId(1)));
        let state = document.xcf_state.as_mut().unwrap();
        let mut first = state.filter_stacks[0].nodes[0].clone();
        first.id = FilterId(1);
        state.filter_stacks[0].nodes.push(first);
        assert_eq!(unused_filter_id(&document), Some(FilterId(2)));
        assert!(current_templates(&document, document.layers[0].id).is_some());
    }
    #[test]
    fn later_template_metadata_is_reachable_without_relaxing_candidate_or_stack_limits() {
        let mut document = fixture();
        let owner = document.layers[0].id;
        let original = document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
        let stack = &mut document.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.nodes.clear();
        for index in 0..65 {
            let mut node = original.clone();
            node.id = FilterId(1000 + index);
            if index < 64 {
                node.operation.name = "own:unsupported-template".into();
            }
            stack.nodes.push(node);
        }
        let shown = current_templates(&document, owner).unwrap();
        assert_eq!(shown.nodes.len(), 65);
        assert!(!shown.nodes.iter().take(64).any(template_candidate));
        assert!(template_candidate(&shown.nodes[64]));
        document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[64].operation.name = "own:unsupported-template".into();
        assert!(current_templates(&document, owner).is_none());
        let stack = &mut document.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.nodes.clear();
        for index in 0..1024 {
            let mut node = original.clone();
            node.id = FilterId(1000 + index);
            stack.nodes.push(node);
        }
        assert!(current_templates(&document, owner).is_none());
    }
}
