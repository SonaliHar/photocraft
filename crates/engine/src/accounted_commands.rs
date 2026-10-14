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
pub(crate) use job_journal::PreparedAccountedJobJournal;

const MAX_RECORDS: usize = 256;
const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_STRING: usize = 4096;
const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 4;

pub fn supports(id: &str) -> bool {
    matches!(id, "layer.select" | "layer.setProps" | "select.all" | "select.deselect" | "select.colorRange" | "edit.undo" | "edit.redo")
}

fn bad(id: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: id.into(), msg: message.into() }
}

#[derive(Clone, Copy)]
enum Request<'a> {
    Select(LayerId),
    Edit(AccountedEdit<'a>),
    ClearSelection,
    SelectAll(&'a Value),
    ColorRange(&'a Value),
    Undo,
    Redo,
}

fn request<'a>(session: &Session, id: &str, params: &'a Value) -> Result<Request<'a>> {
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
            Ok(Request::Select(LayerId(layer)))
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
    if !supports(id) {
        return Ok(Some("this command has no qualified accounted dispatch"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if let Some(reason) = global_refusal(session) {
        return Ok(Some(reason));
    }
    // Null is the existing enabled-query shape, not a layer-property call.
    // Do not disable a menu because its eventual call parameters are absent.
    if !params.is_null() {
        request(session, id, params)?;
    }
    Ok(match id {
        "select.deselect" if state.doc.selection.is_none() => Some("no selection"),
        "select.all" | "select.deselect" if state.doc.xcf_state.is_some() || state.doc.source_color_context.is_some() || state.doc.selection_channel_id.is_some() => {
            Some("accounted selection editing requires a plain native selection owner")
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
}
impl Census {
    fn retained_string(&mut self, value: &String) -> Result<()> {
        if value.len() > MAX_STRING || value.capacity() > MAX_STRING.saturating_mul(2) {
            return Err(BackingError::Length.into());
        }
        self.add(value.capacity(), value.capacity().checked_add(32).ok_or(BackingError::Length)?)
    }
    fn retained_value(&mut self, value: &Value, depth: usize, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        self.nodes = self.nodes.checked_add(1).ok_or(BackingError::Length)?;
        if self.nodes > MAX_NODES || depth > MAX_DEPTH {
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
        if self.nodes > MAX_NODES || depth > MAX_DEPTH {
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
    lease: QuotaLease,
    overlap: QuotaLease,
    result: Value,
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

pub struct PreparedAccountedCommand<'a> {
    request: Request<'a>,
    snapshot: crate::document_snapshot::DocumentSnapshot,
    revision: u64,
    view_revision: u64,
    journal: Vec<(String, Value)>,
    lease: QuotaLease,
    overlap: QuotaLease,
    quota: BackingQuota,
}
impl<'a> PreparedAccountedCommand<'a> {
    pub fn prepare(session: &Session, id: &str, params: &'a Value, control: &ReadControl<'_>) -> Result<Self> {
        control.check(1)?;
        let request = request(session, id, params)?;
        if let Some(reason) = global_refusal(session) {
            return Err(EngineError::Other(reason.into()));
        }
        let quota = session.active().and_then(|state| state.doc.quota()).ok_or(BackingError::Unsupported)?.clone();
        let count = session.journal.len().checked_add(1).ok_or(BackingError::Length)?;
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
            old.retained_value(value, 0, control)?;
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
            census.value(value, 0, control)?;
        }
        census.string(id)?;
        census.value(params, 0, control)?;
        // Separate old-capacity overlap charge is intentionally conservative
        // when an older journal lease already charges the same ledger. It is
        // never retained after the old physical payload is destroyed.
        let overlap = quota.reserve(QuotaKind::Index, old.envelope).map_err(|_| BackingError::Quota)?;
        let lease = quota.reserve(QuotaKind::Index, census.envelope).map_err(|_| BackingError::Quota)?;
        let mut journal = Vec::new();
        journal.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
        if journal.capacity() > count.saturating_mul(2) {
            return Err(BackingError::Allocation.into());
        }
        for (label, value) in &session.journal {
            journal.push((copied_string(label)?, copied_value(value, control)?));
        }
        journal.push((copied_string(id)?, copied_value(params, control)?));
        control.check(1)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        Ok(Self { request, snapshot: state.doc.clone(), revision: state.revision, view_revision: state.view_revision, journal, lease, overlap, quota })
    }
    pub fn execute(self, session: &mut Session, control: &ReadControl<'_>) -> Result<Value> {
        let outcome = self.execute_staged(session, control)?;
        Ok(outcome.publish(session))
    }

    fn execute_staged(self, session: &mut Session, control: &ReadControl<'_>) -> Result<AccountedCommandOutcome> {
        if let Some(reason) = global_refusal(session) {
            return Err(EngineError::Other(reason.into()));
        }
        let quota = session.active().and_then(|state| state.doc.quota()).ok_or(BackingError::Unsupported)?;
        if !quota.shares_ledger(&self.quota) {
            return Err(BackingError::Quota.into());
        }
        let state = session.active().ok_or(EngineError::NoDocument)?;
        if state.revision != self.revision || !state.doc.shares_state(&self.snapshot) {
            return Err(EngineError::Other("the prepared accounted command targets an older document state".into()));
        }
        if matches!(self.request, Request::ClearSelection | Request::SelectAll(_) | Request::ColorRange(_)) && state.view_revision != self.view_revision {
            return Err(EngineError::Other("the prepared accounted selection targets an older view".into()));
        }
        let previous = self.journal.get(..self.journal.len().checked_sub(1).ok_or(BackingError::Integrity)?).ok_or(BackingError::Integrity)?;
        control.check(MAX_PAYLOAD as u64)?;
        if previous != session.journal.as_slice() {
            return Err(EngineError::Other("the command journal changed after preparation".into()));
        }
        // This poll precedes the transactional API publication. No fallible
        // bookkeeping runs after a successful edit.
        control.check(1)?;
        let result = match self.request {
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
        };
        Ok(AccountedCommandOutcome { journal: self.journal, lease: self.lease, overlap: self.overlap, result })
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
