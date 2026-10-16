//! Explicit, bounded command dispatch for retained accounted documents.
//!
//! This module stages the caller-equivalent journal before a typed operation.
//! Container envelopes are conservative logical allocation charges; opaque
//! serde map nodes and standard allocator overhead are not an RSS proof.
use crate::{EngineError, Result, Session};
use photocraft_doc::{LayerId, accounted_document::AccountedEdit};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use serde_json::Value;

mod job_journal;
#[path = "accounted_save_journal.rs"]
mod save_journal;
pub use save_journal::PreparedAccountedSaveReply;
pub(crate) use save_journal::PreparedSaveJournalWitness;
mod assigned_mask;
mod assigned_translate;
#[path = "imported_accounted_metadata.rs"]
mod imported_metadata;
mod imported_translate;
mod point;
mod filtered_flat;
mod hierarchy_expanded;
pub use filtered_flat::candidate as filtered_flat_filter_edit_candidate;
pub use filtered_flat::masked_candidate as masked_filtered_flat_filter_edit_candidate;
pub use filtered_flat::hierarchy_candidate as normal_hierarchy_filter_edit_candidate;
mod gaussian_group;
mod flat_composition;
mod readonly_layer_name;
mod threshold;
mod threshold_alpha;
#[cfg(test)]
mod threshold_alpha_journal_tests;
pub(crate) use job_journal::PreparedAccountedJobJournal;

const MAX_RECORDS: usize = 256;
const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_STRING: usize = 4096;
const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 4;

/// Early synchronous routing for the paid plain Move lane. This is only wire
/// context admission; actual class, locks, native/profile and view validation
/// remain in the controlled typed transaction.
pub fn imported_translation_dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    imported_translate::dispatch_preflight(session, params, background)
}

pub fn supports(id: &str) -> bool {
    matches!(
        id,
        "layer.select"
            | "layer.translate"
            | "layer.setProps"
            | "edit.clear"
            | "edit.fill"
            | "select.all"
            | "select.deselect"
            | "select.colorRange"
            | "edit.undo"
            | "edit.redo"
            | "history.selectState"
            | "xcf.filter.set"
            | "xcf.filter.enable"
            | "xcf.filter.move"
            | "xcf.filter.remove"
            | "xcf.mask.enabled.set"
            | "xcf.composition.set"
    )
}

/// Observed Box admits only its sealed CURRENT filter edit lane, viewing and
/// history navigation. Other mutations retain the original refusal.
pub(crate) fn observed_box_mutation_refused(session: &Session, id: &str) -> bool {
    supports(id)
        && !matches!(id, "layer.select" | "edit.undo" | "edit.redo" | "history.selectState")
        && session.active().and_then(|state| state.doc.accounted()).is_some_and(point::observed_box_readonly)
        && !(id == "layer.translate" && session.active().and_then(|s|s.doc.accounted()).is_some_and(crate::observed_box_translation_candidate))
        && !(id == "layer.setProps" && session.active().and_then(|s|s.doc.accounted()).is_some_and(crate::accounted_session::metadata_gesture::observed_box_layer_metadata_candidate))
        && !(matches!(id, "xcf.filter.set" | "xcf.filter.enable")
            && session.active().and_then(|state| state.doc.accounted()).is_some_and(point::observed_box_edit_candidate))
}

fn bad(id: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: id.into(), msg: message.into() }
}

/// Preserve the old global command set; rename is admitted only on this actual class.
pub(crate) fn supports_for_session(session:&Session,id:&str)->bool{
 supports(id) || id=="layer.setExpanded" && session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_normal_hierarchy_storage()) || id=="layer.renameLayer" && session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage() || (o.is_readonly_masked_filtered_flat_point_storage() || o.is_readonly_normal_hierarchy_storage()))
}
/// No editing or selection route is granted by readonly mixed-root storage.
pub(crate) fn filtered_flat_mutation_refused(session:&Session,id:&str)->bool{
    if matches!(id,"layer.select"|"layer.setExpanded") && session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_normal_hierarchy_storage()){return false}
    supports_for_session(session,id) && !matches!(id,"edit.undo"|"edit.redo"|"history.selectState"|"xcf.filter.set"|"xcf.filter.enable"|"xcf.filter.move"|"xcf.filter.remove"|"layer.setProps"|"layer.renameLayer")
        && session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage() || (o.is_readonly_masked_filtered_flat_point_storage() || o.is_readonly_normal_hierarchy_storage()))
}

enum Request<'a> {
    HierarchyExpanded(hierarchy_expanded::Request<'a>),
    PreparedHierarchyExpanded(hierarchy_expanded::Prepared<'a>),
    FlatComposition(flat_composition::Request<'a>),
    PreparedFlatComposition(flat_composition::Prepared<'a>),
    ImportedTranslate(imported_translate::Request<'a>),
    PreparedImportedTranslate(imported_translate::Prepared<'a>),
    AssignedMask(assigned_mask::Request<'a>),
    PreparedAssignedMask(assigned_mask::Prepared<'a>),
    ImportedMetadata(imported_metadata::Request<'a>),
    PreparedMetadata(imported_metadata::Prepared<'a>),
    ThresholdAlpha(threshold_alpha::Request<'a>),
    PreparedThresholdAlpha(threshold_alpha::Prepared<'a>),
    ReadonlyLayerName(readonly_layer_name::Request<'a>),
    PreparedReadonlyLayerName(readonly_layer_name::Prepared<'a>),
    GaussianGroup(gaussian_group::Request<'a>),
    PreparedGaussianGroup(gaussian_group::Prepared<'a>),
    FilteredFlat(filtered_flat::Request<'a>),
    PreparedFilteredFlat(filtered_flat::Prepared<'a>),
    Point(point::Request<'a>),
    PreparedPoint(point::Prepared<'a>),
    Threshold(threshold::Request<'a>),
    PreparedThreshold(threshold::Prepared<'a>),
    Clear(&'a Value),
    PreparedClear(crate::accounted_clear::Prepared<'a>),
    Fill(&'a Value),
    PreparedFill(crate::accounted_fill::Prepared<'a>),
    Select(LayerId),
    HierarchySelect(LayerId,hierarchy_expanded::SelectionWitness),
    Edit(AccountedEdit<'a>),
    ClearSelection,
    SelectAll(&'a Value),
    ColorRange(&'a Value),
    Undo,
    Redo,
    HistorySelect(usize),
}

fn history_selection_disabled(session: &Session) -> Result<bool> {
    let state = session.active().ok_or(EngineError::NoDocument)?;
    Ok(!state.history.can_undo() && !state.history.can_redo())
}
fn require_history_selection_enabled(session: &Session) -> Result<()> {
    if history_selection_disabled(session)? {
        return Err(EngineError::Disabled("history.selectState".into(), "the history holds only one state".into()));
    }
    Ok(())
}

fn request<'a>(session: &Session, id: &str, params: &'a Value, control: &ReadControl<'_>) -> Result<Request<'a>> {
    if observed_box_mutation_refused(session, id) || filtered_flat_mutation_refused(session,id) {
        return Err(BackingError::Unsupported.into());
    }
    if id == "xcf.composition.set" {
        return Ok(Request::FlatComposition(flat_composition::Request::parse(session,params,control)?));
    }
    if id == "layer.translate" {
        return Ok(Request::ImportedTranslate(imported_translate::Request::parse(session, params, control)?));
    }
    if id == "xcf.mask.enabled.set" {
        return Ok(Request::AssignedMask(assigned_mask::Request::parse(session, params, control)?));
    }
    if id == "edit.fill" {
        crate::accounted_fill::validate_request(session, params)?;
        return Ok(Request::Fill(params));
    }
    if id == "edit.clear" {
        crate::accounted_clear::validate_request(session, params)?;
        return Ok(Request::Clear(params));
    }
    if id=="layer.setExpanded" && session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_normal_hierarchy_storage()) {
        return Ok(Request::HierarchyExpanded(hierarchy_expanded::Request::parse(session,params,control)?));
    }
    if matches!(id,"xcf.filter.set"|"xcf.filter.enable"|"xcf.filter.move"|"xcf.filter.remove") && session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage() || (o.is_readonly_masked_filtered_flat_point_storage() || o.is_readonly_normal_hierarchy_storage())) {
        return Ok(Request::FilteredFlat(filtered_flat::Request::parse(session,id,params,control)?));
    }
    if matches!(id, "xcf.filter.set" | "xcf.filter.enable" | "xcf.filter.move" | "xcf.filter.remove") {
        if session.active().and_then(|state| state.doc.accounted()).is_some_and(point::observed_box_readonly) {
            return Ok(Request::Point(point::Request::parse_observed_box(session,id,params,control)?));
        }
        if session.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_assigned_point_storage()) {
            return Ok(Request::Point(point::Request::parse_assigned_enabled(session,id,params,control)?));
        }
        if session.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_gaussian_group_storage()) {
            return Ok(Request::GaussianGroup(gaussian_group::Request::parse(session,id,params,control)?));
        }
        if session.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_point_storage()) {
            return Ok(Request::Point(point::Request::parse(session, id, params, control)?));
        }
        if matches!(id, "xcf.filter.move" | "xcf.filter.remove") {
            return Err(BackingError::Unsupported.into());
        }
        if session.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_threshold_storage()) {
            return Ok(Request::Threshold(threshold::Request::parse(session, id, params, control)?));
        }
        return Ok(Request::ThresholdAlpha(threshold_alpha::Request::parse(session, id, params, control)?));
    }
    if id == "history.selectState" {
        require_history_selection_enabled(session)?;
        // Preserve upstream ignored fields. The shared bounded journal census
        // admits their actual retained payload before any journal copies.
        params.as_object().ok_or_else(|| bad(id, "parameters must be an object"))?;
        return Ok(Request::HistorySelect(crate::history_select_cmds::validate_request(session, params, control)?));
    }
    if id=="layer.renameLayer" && session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage() || (o.is_readonly_masked_filtered_flat_point_storage() || o.is_readonly_normal_hierarchy_storage())) {
        return Ok(Request::ImportedMetadata(imported_metadata::parse_rename(session,params,control)?));
    }
    if id == "layer.setProps" && readonly_layer_name::applies(session) {
        return Ok(Request::ReadonlyLayerName(readonly_layer_name::Request::parse(params, control)?));
    }
    if id == "layer.setProps" && imported_metadata::applies(session) {
        return Ok(Request::ImportedMetadata(imported_metadata::parse(params)?));
    }
    if id == "select.all" {
        crate::accounted_select_all::validate_request(session, params)?;
        return Ok(Request::SelectAll(params));
    }
    if id == "select.colorRange" {
        crate::accounted_color_range::validate_request(params)?;
        return Ok(Request::ColorRange(params));
    }
    if id == "select.deselect" {
        crate::selection_inverse_controlled::storage_dispatch_preflight(session, id, params, false)?;
        return Ok(Request::ClearSelection);
    }
    if params.is_null() && matches!(id, "edit.undo" | "edit.redo") {
        return Ok(if id == "edit.undo" { Request::Undo } else { Request::Redo });
    }
    let object = params.as_object().ok_or_else(|| bad(id, "parameters must be an object"))?;
    if object.len() > 4 {
        return Err(bad(id, "too many parameters"));
    }
    if let Some(value) = object.get("coalesce") {
        match value {
            Value::Null => {}
            Value::String(value) if value.len() <= 128 => return Err(EngineError::Other("accounted command coalescing is not yet supported".into())),
            _ => return Err(bad(id, "coalesce must be null or a bounded string")),
        }
    }
    match id {
        "edit.undo" | "edit.redo" => {
            if !object.is_empty() {
                return Err(bad(id, "history commands take no parameters"));
            }
            Ok(if id == "edit.undo" { Request::Undo } else { Request::Redo })
        }
        "layer.select" => {
            if object.keys().any(|key| !matches!(key.as_str(), "layer" | "mode")) {
                return Err(bad(id, "unsupported selection parameter"));
            }
            if let Some(mode) = object.get("mode") {
                let mode = mode.as_str().ok_or_else(|| bad(id, "mode must be a string"))?;
                if mode != "replace" {
                    return Err(EngineError::Other("accounted layer selection supports replace only".into()));
                }
            }
            let layer = object.get("layer").and_then(Value::as_u64).filter(|id| *id != 0).ok_or_else(|| bad(id, "layer must be a nonzero integer identity"))?;
            let layer=LayerId(layer);
            if session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_normal_hierarchy_storage()) {
                Ok(Request::HierarchySelect(layer,hierarchy_expanded::SelectionWitness::prepare(session,layer,control)?))
            }else{Ok(Request::Select(layer))}
        }
        "layer.setProps" => {
            if object.keys().any(|key| !matches!(key.as_str(), "layer" | "coalesce" | "name" | "visible" | "opacity")) {
                return Err(bad(id, "accounted properties support one name, visible, or opacity field"));
            }
            let properties = ["name", "visible", "opacity"].iter().filter(|key| object.contains_key(**key)).count();
            if properties != 1 {
                return Err(bad(id, "exactly one property is required"));
            }
            let layer = if let Some(value) = object.get("layer") {
                LayerId(value.as_u64().filter(|id| *id != 0).ok_or_else(|| bad(id, "layer must be a nonzero integer identity"))?)
            } else {
                session.active().and_then(|state| state.active_layer).ok_or(EngineError::NoDocument)?
            };
            if let Some(value) = object.get("name") {
                let name = value
                    .as_str()
                    .filter(|name| name.len() <= MAX_STRING && !name.contains('\0'))
                    .ok_or_else(|| bad(id, "name must be a bounded string without NUL"))?;
                return Ok(Request::Edit(AccountedEdit::LayerName { layer, name }));
            }
            if let Some(value) = object.get("visible") {
                let visible = value.as_bool().ok_or_else(|| bad(id, "visible must be a boolean"))?;
                return Ok(Request::Edit(AccountedEdit::LayerVisibility { layer, visible }));
            }
            let opacity = object
                .get("opacity")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                .ok_or_else(|| bad(id, "opacity must be finite and within 0..1"))?;
            Ok(Request::Edit(AccountedEdit::LayerOpacity { layer, opacity: opacity as f32 }))
        }
        _ => Err(BackingError::Unsupported.into()),
    }
}

fn global_refusal(session: &Session) -> Option<&'static str> {
    if session.actions.recording.is_some() || session.actions.playing != 0 || !session.actions.playback_stack.is_empty() {
        return Some("accounted command action recording and playback are not yet qualified");
    }
    if session.preset_store.as_ref().is_some_and(|store| !store.is_clean_for(session.tools.presets_rev, session.actions.rev)) {
        return Some("accounted commands require preset and action persistence to be synchronized");
    }
    if session.edit_state.fade.is_some() {
        return Some("accounted commands require no active Fade state");
    }
    let state = session.active()?;
    if state.doc.accounted().is_none() {
        return Some("this dispatch requires an accounted document");
    }
    if state.floating.is_some() {
        return Some("accounted commands require no active floating selection");
    }
    if !state.doc.slices.is_empty() {
        return Some("accounted command slice refresh is not yet qualified");
    }
    if state.channel_view.layer_mask.is_some() {
        return Some("accounted commands require the ordinary layer view");
    }
    None
}

pub fn disabled_reason(session: &Session, id: &str, params: &Value) -> Result<Option<&'static str>> {
    if !supports_for_session(session,id) {
        return Ok(Some("this command has no qualified accounted dispatch"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if let Some(reason) = global_refusal(session) {
        return Ok(Some(reason));
    }
    if id == "xcf.composition.set" && !state.doc.accounted().is_some_and(|owner|owner.is_readonly_flat_composite_storage()) {
        return Ok(Some("this accounted storage has no qualified explicit composition transaction"));
    }
    if id == "layer.translate" && !state.doc.accounted().is_some_and(imported_translate::candidate) {
        return Ok(Some("this accounted storage has no qualified plain Move transaction"));
    }
    if id == "xcf.mask.enabled.set" && !state.doc.accounted().is_some_and(crate::assigned_geometry_metadata_candidate) {
        return Ok(Some("this accounted storage has no qualified assigned layer mask"));
    }
    if id == "history.selectState" && history_selection_disabled(session)? {
        return Ok(Some("the history holds only one state"));
    }
    if observed_box_mutation_refused(session, id) {
        return Ok(Some("this observed Box geometry recipe is read-only"));
    }
    if matches!(id, "xcf.filter.move" | "xcf.filter.remove") && !state.doc.accounted().is_some_and(|owner| owner.is_readonly_point_storage() || (owner.is_readonly_filtered_flat_point_storage() || (owner.is_readonly_masked_filtered_flat_point_storage() || owner.is_readonly_normal_hierarchy_storage()))) {
        return Ok(Some("this accounted filter storage has no qualified stack edit recipe"));
    }
    if matches!(id, "xcf.filter.set" | "xcf.filter.enable")
        && !state
            .doc
            .accounted()
            .is_some_and(|owner| owner.is_readonly_threshold_alpha_storage() || owner.is_readonly_threshold_storage() || owner.is_readonly_point_storage() || owner.is_readonly_gaussian_group_storage() || (owner.is_readonly_filtered_flat_point_storage() || (owner.is_readonly_masked_filtered_flat_point_storage() || owner.is_readonly_normal_hierarchy_storage())) || point::assigned_enabled_candidate(owner))
    {
        return Ok(Some("this accounted filter storage has no qualified editing recipe"));
    }
    if id == "edit.clear"
        && (state.doc.xcf_state.is_none()
            || state.doc.accounted().is_none_or(|owner| owner.is_readonly_threshold_alpha_storage() || owner.is_readonly_threshold_storage()))
    {
        return Ok(Some("accounted Clear requires the plain imported native owner"));
    }
    if id == "edit.fill"
        && (state.doc.xcf_state.is_none()
            || state.doc.accounted().is_none_or(|owner| owner.is_readonly_threshold_alpha_storage() || owner.is_readonly_threshold_storage()))
    {
        return Ok(Some("accounted Fill requires the plain imported native owner"));
    }
    if id == "edit.fill" && !matches!(state.doc.depth, photocraft_doc::SampleType::U8 | photocraft_doc::SampleType::U16 | photocraft_doc::SampleType::F32) {
        return Ok(Some("accounted Fill requires U8/U16/F32 native pixels"));
    }
    // Null is the existing enabled-query shape, not a layer-property call.
    // Do not disable a menu because its eventual call parameters are absent.
    if !params.is_null() {
        if matches!(id,"xcf.filter.set"|"xcf.filter.enable") && state.doc.accounted().is_some_and(point::observed_box_readonly) {
            if !point::observed_box_wire_valid(id,params) { return Err(bad(id,"observed Box requires radius Signed32 0..1000 and/or enabled")); }
        } else if matches!(id,"xcf.filter.set"|"xcf.filter.enable") && state.doc.accounted().is_some_and(|owner|owner.is_readonly_assigned_point_storage()) {
            if !point::assigned_enabled_wire_valid(id,params) { return Err(bad(id,"assigned activation requires only filterId and enabled")); }
        } else {
            request(session, id, params, &crate::read_control_none())?;
        }
    }
    Ok(match id {
        "select.deselect" if state.doc.selection.is_none() => Some("no selection"),
        "select.all" | "select.deselect" if state.doc.selection_channel_id.is_some() => {
            // Menu eligibility is only a hint. The actual source/guard/model
            // census remains mandatory under the typed publication control.
            Some("accounted anonymous selection editing requires no saved selection owner")
        }
        "layer.setProps" if state.active_layer.is_none() => Some("no active layer"),
        "edit.undo" if !state.history.can_undo() => Some("nothing to undo"),
        "edit.redo" if !state.history.can_redo() => Some("nothing to redo"),
        _ => None,
    })
}

#[derive(Default)]
struct Census {
    payload: usize,
    nodes: usize,
    envelope: u64,
    // Only exact closed filter wires gain their own bounded depth: scalar5
    // Signed32 depth6, Color7, FREE256 config9 or ordered Levels/CB config11.
    // Unrelated values retain depth4; structured scans use the same control.
    depth_extra: usize,
}
impl Census {
    fn command_value(&mut self, id: &str, value: &Value, control: &ReadControl<'_>) -> Result<()> {
        self.depth_extra = threshold_alpha::journal_depth(id, value)
            .max(threshold::journal_depth(id, value))
            .max(point::journal_depth_controlled(id, value, control)?)
            .saturating_sub(MAX_DEPTH);
        let result = self.value(value, 0, control);
        self.depth_extra = 0;
        result
    }
    fn retained_command_value(&mut self, id: &str, value: &Value, control: &ReadControl<'_>) -> Result<()> {
        self.depth_extra = threshold_alpha::journal_depth(id, value)
            .max(threshold::journal_depth(id, value))
            .max(point::journal_depth_controlled(id, value, control)?)
            .saturating_sub(MAX_DEPTH);
        let result = self.retained_value(value, 0, control);
        self.depth_extra = 0;
        result
    }
    fn retained_string(&mut self, value: &String) -> Result<()> {
        if value.len() > MAX_STRING || value.capacity() > MAX_STRING.saturating_mul(2) {
            return Err(BackingError::Length.into());
        }
        self.add(value.capacity(), value.capacity().checked_add(32).ok_or(BackingError::Length)?)
    }
    fn retained_value(&mut self, value: &Value, depth: usize, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        self.nodes = self.nodes.checked_add(1).ok_or(BackingError::Length)?;
        if self.nodes > MAX_NODES || depth > MAX_DEPTH + self.depth_extra {
            return Err(BackingError::Length.into());
        }
        match value {
            Value::String(value) => self.retained_string(value),
            Value::Array(values) => {
                if values.capacity() > MAX_NODES.saturating_mul(2) {
                    return Err(BackingError::Length.into());
                }
                self.add(0, values.capacity().checked_mul(std::mem::size_of::<Value>()).ok_or(BackingError::Length)?)?;
                for value in values {
                    self.retained_value(value, depth + 1, control)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                if values.len() > 16 {
                    return Err(BackingError::Length.into());
                }
                let bytes = values.len().checked_mul(256).and_then(|n| n.checked_add(if values.is_empty() { 0 } else { 4096 })).ok_or(BackingError::Length)?;
                self.add(0, bytes)?;
                for (key, value) in values {
                    self.retained_string(key)?;
                    self.retained_value(value, depth + 1, control)?;
                }
                Ok(())
            }
            _ => self.add(8, std::mem::size_of::<Value>()),
        }
    }
    fn add(&mut self, payload: usize, envelope: usize) -> Result<()> {
        self.payload = self.payload.checked_add(payload).ok_or(BackingError::Length)?;
        self.envelope = self.envelope.checked_add(envelope as u64).ok_or(BackingError::Length)?;
        if self.payload > MAX_PAYLOAD {
            return Err(BackingError::Length.into());
        }
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<()> {
        if value.len() > MAX_STRING {
            return Err(BackingError::Length.into());
        }
        self.add(value.len(), value.len().checked_mul(2).and_then(|n| n.checked_add(32)).ok_or(BackingError::Length)?)
    }
    fn value(&mut self, value: &Value, depth: usize, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        self.nodes = self.nodes.checked_add(1).ok_or(BackingError::Length)?;
        if self.nodes > MAX_NODES || depth > MAX_DEPTH + self.depth_extra {
            return Err(BackingError::Length.into());
        }
        match value {
            Value::String(value) => self.string(value),
            Value::Array(values) => {
                self.add(0, values.len().checked_mul(std::mem::size_of::<Value>()).and_then(|n| n.checked_mul(2)).ok_or(BackingError::Length)?)?;
                for value in values {
                    self.value(value, depth + 1, control)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                if values.len() > 16 {
                    return Err(BackingError::Length.into());
                }
                // A nonempty BTree-backed serde map allocates a node sized
                // for multiple pairs even with one key. This fixed envelope
                // plus per-pair growth is a bounded logical charge, not an
                // introspection of opaque allocator nodes or physical RSS.
                let bytes = values.len().checked_mul(256).and_then(|n| n.checked_add(if values.is_empty() { 0 } else { 4096 })).ok_or(BackingError::Length)?;
                self.add(0, bytes)?;
                for (key, value) in values {
                    self.string(key)?;
                    self.value(value, depth + 1, control)?;
                }
                Ok(())
            }
            _ => self.add(8, std::mem::size_of::<Value>()),
        }
    }
}

fn copied_string(value: &str) -> Result<String> {
    let bound = value.len().checked_mul(2).ok_or(BackingError::Length)?;
    let mut result = String::new();
    result.try_reserve_exact(value.len()).map_err(|_| BackingError::Allocation)?;
    if result.capacity() > bound {
        return Err(BackingError::Allocation.into());
    }
    result.push_str(value);
    Ok(result)
}

fn copied_value(value: &Value, control: &ReadControl<'_>) -> Result<Value> {
    control.check(1)?;
    Ok(match value {
        Value::Null => Value::Null,
        Value::Bool(value) => Value::Bool(*value),
        Value::Number(value) => Value::Number(value.clone()),
        Value::String(value) => Value::String(copied_string(value)?),
        Value::Array(values) => {
            let mut result = Vec::new();
            result.try_reserve_exact(values.len()).map_err(|_| BackingError::Allocation)?;
            if result.capacity() > values.len().saturating_mul(2) {
                return Err(BackingError::Allocation.into());
            }
            for value in values {
                result.push(copied_value(value, control)?);
            }
            Value::Array(result)
        }
        Value::Object(values) => {
            let mut result = serde_json::Map::new();
            for (key, value) in values {
                result.insert(copied_string(key)?, copied_value(value, control)?);
            }
            Value::Object(result)
        }
    })
}

/// Owns the replacement journal until the caller publishes it with its lease.
struct AccountedCommandOutcome {
    journal: Vec<(String, Value)>,
    result: Value,
    lease: QuotaLease,
    overlap: QuotaLease,
}
impl AccountedCommandOutcome {
    fn publish(self, session: &mut Session) -> Value {
        session.publish_accounted_journal(self.journal, self.lease);
        // The previous Vec and all its values have been destroyed by the
        // nonfallible replacement before this temporary charge is refunded.
        drop(self.overlap);
        self.result
    }
}

/// Actual combined journal/response reservation, never caller-asserted payment.
/// The result's synchronous return boundary is covered while this command's
/// retained journal lease lives; this does not own arbitrary Values after Session.
pub(crate) struct HistoryResponseAllowance {
    lease: QuotaLease,
    quota: BackingQuota,
    target: usize,
    changed: bool,
    max_label: usize,
}
impl HistoryResponseAllowance {
    pub(crate) fn response(
        &self,
        owner: &photocraft_doc::accounted_document::AccountedDocument,
        target: usize,
        changed: bool,
        current: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<Value> {
        control.check(1)?;
        if !self.quota.shares_ledger(owner.quota())
            || target != self.target
            || changed != self.changed
            || current.is_some_and(|label| label.len() > self.max_label)
            || (!changed && current.is_some())
        {
            return Err(BackingError::Integrity.into());
        }
        let length = current.map_or(0, str::len);
        control.check((length as u64).checked_mul(2).and_then(|n| n.checked_add(256)).ok_or(BackingError::Length)?)?;
        let label = match current {
            Some(value) => {
                let mut owned = String::new();
                owned.try_reserve_exact(value.len()).map_err(|_| BackingError::Allocation)?;
                if owned.capacity() > value.len().checked_mul(2).ok_or(BackingError::Length)? {
                    return Err(BackingError::Allocation.into());
                }
                owned.push_str(value);
                Some(Value::String(owned))
            }
            None => None,
        };
        // Three fixed map nodes/key strings fit the prepaid 4096-byte logical
        // envelope. Standard serde map allocator internals are not RSS control.
        let mut object = serde_json::Map::new();
        object.insert("state".into(), Value::from(u64::try_from(target).map_err(|_| BackingError::Length)?));
        object.insert("changed".into(), Value::Bool(changed));
        if changed {
            object.insert("current".into(), label.unwrap_or(Value::Null));
        }
        control.check(1)?;
        Ok(Value::Object(object))
    }
}

enum CommandLease {
    Journal(QuotaLease),
    History(HistoryResponseAllowance),
}
impl CommandLease {
    fn into_lease(self) -> QuotaLease {
        match self {
            Self::Journal(lease) => lease,
            Self::History(response) => response.lease,
        }
    }
}

#[derive(Clone, Copy)]
enum JournalPolicy {
    Append,
    Retain,
}
pub struct PreparedAccountedCommand<'a> {
    journal_policy: JournalPolicy,
    request: Request<'a>,
    snapshot: crate::document_snapshot::DocumentSnapshot,
    revision: u64,
    view_revision: u64,
    history: Option<crate::document_history::DocumentHistory>,
    journal: Vec<(String, Value)>,
    lease: CommandLease,
    overlap: QuotaLease,
    quota: BackingQuota,
}
impl<'a> PreparedAccountedCommand<'a> {
    pub fn prepare(session: &Session, id: &str, params: &'a Value, control: &ReadControl<'_>) -> Result<Self> {
        control.check(1)?;
        let request = request(session, id, params, control)?;
        if let Some(reason) = global_refusal(session) {
            return Err(EngineError::Other(reason.into()));
        }
        let request = match request {
            Request::HierarchyExpanded(request)=>Request::PreparedHierarchyExpanded(request.prepare(session,control)?),
            Request::FlatComposition(request)=>Request::PreparedFlatComposition(request.prepare(session,control)?),
            Request::ImportedTranslate(request) => Request::PreparedImportedTranslate(request.prepare(session, control)?),
            Request::AssignedMask(request) => Request::PreparedAssignedMask(request.prepare(session, control)?),
            Request::ImportedMetadata(request) => Request::PreparedMetadata(request.prepare(session, control)?),
            Request::ThresholdAlpha(request) => Request::PreparedThresholdAlpha(request.prepare(session, control)?),
            Request::Threshold(request) => Request::PreparedThreshold(request.prepare(session, control)?),
            Request::FilteredFlat(request) => Request::PreparedFilteredFlat(request.prepare(session,control)?),
            Request::Point(request) => Request::PreparedPoint(request.prepare(session, control)?),
            Request::GaussianGroup(request) => Request::PreparedGaussianGroup(request.prepare(session, control)?),
            Request::ReadonlyLayerName(request) => Request::PreparedReadonlyLayerName(request.prepare(session, control)?),
            Request::Clear(params) => Request::PreparedClear(crate::accounted_clear::Prepared::prepare(session, params, control)?),
            Request::Fill(params) => Request::PreparedFill(crate::accounted_fill::Prepared::prepare(session, params, control)?),
            other => other,
        };
        let quota = session.active().and_then(|state| state.doc.quota()).ok_or(BackingError::Unsupported)?.clone();
        let response_plan = if let Request::HistorySelect(target) = &request {
            // At most 256 retained slots. Charge before each borrowed label visit;
            // no compatibility label Vec or model copy is constructed here.
            let state = session.active().ok_or(EngineError::NoDocument)?;
            let mut visits = 0usize;
            let mut max_label = 0usize;
            for label in state.history.entry_labels().chain(state.history.redo_labels()) {
                control.check(1)?;
                visits = visits.checked_add(1).ok_or(BackingError::Length)?;
                if visits > 256 || label.len() > MAX_STRING {
                    return Err(BackingError::Length.into());
                }
                max_label = max_label.max(label.len());
            }
            let bytes =
                max_label.checked_mul(2).and_then(|n| n.checked_add(4096 + std::mem::size_of::<HistoryResponseAllowance>())).ok_or(BackingError::Length)?;
            Some((*target, *target != state.history.past_len(), max_label, bytes as u64))
        } else {
            None
        };
        // Mask enablement preserves its upstream journal:false contract. Its
        // retained response still owns a real fee beside the unchanged journal.
        let journal_policy = if matches!(&request, Request::PreparedAssignedMask(_)|Request::PreparedHierarchyExpanded(_)) { JournalPolicy::Retain } else { JournalPolicy::Append };
        let count = session.journal.len().checked_add(usize::from(matches!(journal_policy, JournalPolicy::Append))).ok_or(BackingError::Length)?;
        if count > MAX_RECORDS {
            return Err(BackingError::Length.into());
        }
        if session.journal.capacity() > MAX_RECORDS.saturating_mul(2) {
            return Err(BackingError::Length.into());
        }
        let mut old = Census::default();
        old.add(
            0,
            session.journal.capacity().checked_mul(std::mem::size_of::<(String, Value)>()).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?,
        )?;
        for (label, value) in &session.journal {
            old.retained_string(label)?;
            old.retained_command_value(label, value, control)?;
        }
        let mut census = Census::default();
        census.add(
            0,
            count
                .checked_mul(std::mem::size_of::<(String, Value)>())
                .and_then(|n| n.checked_mul(2))
                .and_then(|n| n.checked_add(128))
                .ok_or(BackingError::Length)?,
        )?;
        for (label, value) in &session.journal {
            census.string(label)?;
            census.command_value(label, value, control)?;
        }
        if matches!(journal_policy, JournalPolicy::Append) {
            census.string(id)?;
            census.command_value(id, params, control)?;
        }
        if matches!(&request, Request::PreparedAssignedMask(_)) {
            census.add(0, usize::try_from(assigned_mask::REPLY_ENVELOPE).map_err(|_| BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedThresholdAlpha(_)) {
            // The staged adapter already owns its temporary reply fee. This
            // overlapping retained charge survives publication with the journal
            // and covers the synchronous returned reply after that stage drops.
            census.add(0, usize::try_from(threshold_alpha::REPLY_ENVELOPE).map_err(|_| BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedFlatComposition(_)) {
            census.add(0,usize::try_from(flat_composition::REPLY_ENVELOPE).map_err(|_|BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedHierarchyExpanded(_)) {
            census.add(0,usize::try_from(hierarchy_expanded::REPLY_ENVELOPE).map_err(|_|BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedGaussianGroup(_)) {
            census.add(0,usize::try_from(gaussian_group::REPLY_ENVELOPE).map_err(|_|BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedFilteredFlat(_)) {
            census.add(0,usize::try_from(filtered_flat::REPLY_ENVELOPE).map_err(|_|BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedPoint(_)) {
            census.add(0, usize::try_from(point::REPLY_ENVELOPE).map_err(|_| BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedThreshold(_)) {
            census.add(0, usize::try_from(threshold::REPLY_ENVELOPE).map_err(|_| BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedReadonlyLayerName(_)) {
            census.add(0, usize::try_from(readonly_layer_name::REPLY_ENVELOPE).map_err(|_| BackingError::Length)?)?;
        }
        if matches!(&request, Request::PreparedFill(_)) {
            // The temporary native transaction pays its prepared reply until
            // publication; the retained journal covers the returned value.
            census.add(0, usize::try_from(crate::accounted_fill::REPLY_ENVELOPE).map_err(|_| BackingError::Length)?)?;
        }
        // The copier polls Value visits but String copies do not debit bytes.
        // Prepay actual bounded payload and known Value/entry slot copies before
        // any destination allocation. Later equality admission is independent.
        let copy_work = census
            .nodes
            .checked_mul(std::mem::size_of::<Value>())
            .and_then(|n| count.checked_mul(std::mem::size_of::<(String, Value)>()).and_then(|v| n.checked_add(v)))
            .and_then(|n| n.checked_add(census.payload))
            .ok_or(BackingError::Length)?;
        control.check(u64::try_from(copy_work).map_err(|_| BackingError::Length)?)?;
        // Separate old-capacity overlap charge is intentionally conservative
        // when an older journal lease already charges the same ledger. It is
        // never retained after the old physical payload is destroyed.
        let overlap = quota.reserve(QuotaKind::Index, old.envelope).map_err(|_| BackingError::Quota)?;
        let envelope = census.envelope.checked_add(response_plan.map_or(0, |p| p.3)).ok_or(BackingError::Length)?;
        let lease = quota.reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
        let mut journal = Vec::new();
        journal.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
        if journal.capacity() > count.saturating_mul(2) {
            return Err(BackingError::Allocation.into());
        }
        for (label, value) in &session.journal {
            journal.push((copied_string(label)?, copied_value(value, control)?));
        }
        if matches!(journal_policy, JournalPolicy::Append) {
            journal.push((copied_string(id)?, copied_value(params, control)?));
        }
        control.check(1)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let lease = match response_plan {
            Some((target, changed, max_label, _)) => {
                CommandLease::History(HistoryResponseAllowance { lease, quota: quota.clone(), target, changed, max_label })
            }
            None => CommandLease::Journal(lease),
        };
        let history = if matches!(
            &request,
            Request::HistorySelect(_)
                | Request::HierarchySelect(_,_)
                | Request::PreparedImportedTranslate(_)
                | Request::PreparedFilteredFlat(_)
                | Request::PreparedPoint(_)
                | Request::PreparedGaussianGroup(_)
                | Request::PreparedFlatComposition(_)
                | Request::PreparedThresholdAlpha(_)
                | Request::PreparedThreshold(_)
                | Request::PreparedReadonlyLayerName(_)
                | Request::PreparedClear(_)
                | Request::PreparedFill(_)
                | Request::PreparedAssignedMask(_)
        ) {
            Some(state.history.clone())
        } else {
            None
        };
        Ok(Self {
            journal_policy,
            request,
            snapshot: state.doc.clone(),
            revision: state.revision,
            view_revision: state.view_revision,
            history,
            journal,
            lease,
            overlap,
            quota,
        })
    }
    pub fn execute(self, session: &mut Session, control: &ReadControl<'_>) -> Result<Value> {
        let outcome = self.execute_staged(session, control)?;
        Ok(outcome.publish(session))
    }

    fn execute_staged(self, session: &mut Session, control: &ReadControl<'_>) -> Result<AccountedCommandOutcome> {
        if let Some(reason) = global_refusal(session) {
            return Err(EngineError::Other(reason.into()));
        }
        if matches!(&self.request, Request::HistorySelect(_)) {
            require_history_selection_enabled(session)?;
        }
        let quota = session.active().and_then(|state| state.doc.quota()).ok_or(BackingError::Unsupported)?;
        if !quota.shares_ledger(&self.quota) {
            return Err(BackingError::Quota.into());
        }
        let state = session.active().ok_or(EngineError::NoDocument)?;
        if state.revision != self.revision || !state.doc.shares_state(&self.snapshot) {
            return Err(EngineError::Other("the prepared accounted command targets an older document state".into()));
        }
        if matches!(
            &self.request,
            Request::ClearSelection
                | Request::HierarchySelect(_,_)
                | Request::PreparedHierarchyExpanded(_)
                | Request::PreparedImportedTranslate(_)
                | Request::SelectAll(_)
                | Request::ColorRange(_)
                | Request::PreparedMetadata(_)
                | Request::PreparedAssignedMask(_)
                | Request::PreparedThresholdAlpha(_)
                | Request::PreparedReadonlyLayerName(_)
                | Request::PreparedThreshold(_)
                | Request::PreparedFilteredFlat(_)
                | Request::PreparedPoint(_)
                | Request::PreparedGaussianGroup(_)
                | Request::PreparedFlatComposition(_)
                | Request::PreparedClear(_)
                | Request::PreparedFill(_)
                | Request::HistorySelect(_)
        ) && state.view_revision != self.view_revision
        {
            return Err(EngineError::Other("the prepared accounted selection targets an older view".into()));
        }
        if let Some(history) = &self.history {
            control.check(1)?;
            if !state.history.shares_accounted_history(history)
                || state.history.max_states != history.max_states
                || state.history.max_bytes != history.max_bytes
            {
                return Err(EngineError::Other("the prepared history selection targets an older history".into()));
            }
        }
        let previous = match self.journal_policy {
            JournalPolicy::Append => self.journal.get(..self.journal.len().checked_sub(1).ok_or(BackingError::Integrity)?).ok_or(BackingError::Integrity)?,
            JournalPolicy::Retain => self.journal.as_slice(),
        };
        control.check(MAX_PAYLOAD as u64)?;
        if previous != session.journal.as_slice() {
            return Err(EngineError::Other("the command journal changed after preparation".into()));
        }
        // This poll precedes the transactional API publication. No fallible
        // bookkeeping runs after a successful edit.
        control.check(1)?;
        // A private enum owns the real combined reservation. Validate the
        // request/allowance relationship before invoking any publication body.
        if matches!(&self.request, Request::HistorySelect(_)) != matches!(&self.lease, CommandLease::History(_)) {
            return Err(BackingError::Integrity.into());
        }
        let result = match self.request {
            Request::PreparedHierarchyExpanded(prepared)=>prepared.execute(session,control)?,
            Request::HierarchyExpanded(_)=>return Err(BackingError::Integrity.into()),
            Request::PreparedFlatComposition(prepared)=>prepared.execute(session,control)?,
            Request::FlatComposition(_)=>return Err(BackingError::Integrity.into()),
            Request::PreparedImportedTranslate(prepared) => prepared.execute(session, control)?,
            Request::ImportedTranslate(_) => return Err(BackingError::Integrity.into()),
            Request::PreparedAssignedMask(prepared) => prepared.execute(session, control)?,
            Request::PreparedMetadata(prepared) => prepared.execute(session, control)?,
            Request::PreparedThresholdAlpha(prepared) => prepared.execute(session, control)?,
            Request::PreparedThreshold(prepared) => prepared.execute(session, control)?,
            Request::PreparedFilteredFlat(prepared) => prepared.execute(session,control)?,
            Request::PreparedPoint(prepared) => prepared.execute(session, control)?,
            Request::PreparedGaussianGroup(prepared) => prepared.execute(session, control)?,
            Request::PreparedReadonlyLayerName(prepared) => prepared.execute(session, control)?,
            Request::PreparedClear(prepared) => prepared.execute(session, control)?,
            Request::PreparedFill(prepared) => prepared.execute(session, control)?,
            Request::Clear(_) => return Err(BackingError::Integrity.into()),
            Request::Fill(_) => return Err(BackingError::Integrity.into()),
            Request::ThresholdAlpha(_) => return Err(BackingError::Integrity.into()),
            Request::Threshold(_) => return Err(BackingError::Integrity.into()),
            Request::FilteredFlat(_) => return Err(BackingError::Integrity.into()),
            Request::Point(_) => return Err(BackingError::Integrity.into()),
            Request::GaussianGroup(_) => return Err(BackingError::Integrity.into()),
            Request::ReadonlyLayerName(_) => return Err(BackingError::Integrity.into()),
            Request::AssignedMask(_) => return Err(BackingError::Integrity.into()),
            Request::ImportedMetadata(_) => return Err(BackingError::Integrity.into()),
            Request::HierarchySelect(layer,witness)=>{
                witness.validate(session,control)?;
                session.select_accounted_with_control(layer,control)?;
                Value::Null
            }
            Request::Select(layer) => {
                session.select_accounted_with_control(layer, control)?;
                Value::Null
            }
            Request::Edit(edit) => {
                let label = if matches!(edit, AccountedEdit::LayerVisibility { .. }) { "Layer Visibility" } else { "Layer Properties" };
                session.edit_accounted(label, edit, control)?;
                Value::Null
            }
            Request::ClearSelection => {
                let target = crate::accounted_selection_session::AccountedSelectionTarget::capture(session)?;
                session.replace_accounted_selection(target, "Deselect", None, control)?;
                Value::Null
            }
            Request::ColorRange(params) => crate::accounted_color_range::run(session, params, control)?,
            Request::SelectAll(params) => crate::accounted_select_all::run(session, params, control)?,
            Request::Undo => Value::Bool(session.undo_with_control(control)?),
            Request::Redo => Value::Bool(session.redo_with_control(control)?),
            Request::HistorySelect(target) => {
                let CommandLease::History(response) = &self.lease else { return Err(BackingError::Integrity.into()) };
                crate::history_select_cmds::run_accounted(session, target, control, response)?
            }
        };
        let lease = self.lease.into_lease();
        Ok(AccountedCommandOutcome { journal: self.journal, lease, overlap: self.overlap, result })
    }
}

pub fn dispatch(session: &mut Session, id: &str, params: &Value, control: &ReadControl<'_>) -> Result<Value> {
    PreparedAccountedCommand::prepare(session, id, params, control)?.execute(session, control)
}

impl Session {
    /// Paid selection replacement changes focus without adding a history entry.
    pub fn select_accounted_with_control(&mut self, id: LayerId, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        if self.active_job().is_some() {
            return Err(EngineError::Other("the document has an active background job".into()));
        }
        let state = self.active_mut().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        if owner.is_readonly_normal_hierarchy_storage(){
            if !state.history.current()?.shares_state(&state.doc){return Err(BackingError::Integrity.into())}
            crate::accounted_xcf_open::normal_hierarchy::validate_current_normal_hierarchy_storage(owner,control)?;
        }
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        let selection = view.try_replace(owner, Some(id), &[id], Some(id), control)?;
        let mut history = state.history.clone();
        history.set_current_layers_borrowed(selection.target(), control)?;
        let revision = state.revision.checked_add(1).ok_or(BackingError::Length)?;
        let clean = state.saved_revision == state.revision;
        control.check(1)?;
        state.accounted_view = Some(selection);
        state.history = history;
        state.active_layer = Some(id);
        state.layer_anchor = Some(id);
        state.revision = revision;
        if clean {
            state.saved_revision = revision;
        }
        state.last_damage = Some(photocraft_geom::Rect::EMPTY);
        crate::mask_view_cmds::fix(state);
        Ok(())
    }
}

#[cfg(test)]
#[path = "accounted_history_select_tests.rs"]
mod history_select_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_legacy_flat_composite_workflow_tests.rs"]
mod legacy_flat_composite_workflow_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_component_luma_workflow_tests.rs"]
mod component_luma_workflow_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_alpha_flat_workflow_tests.rs"]
mod alpha_flat_workflow_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_lab_flat_workflow_tests.rs"]
mod lab_flat_workflow_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="accounted_commands/exposure_loaded_tests.rs"]
mod exposure_loaded_tests;
