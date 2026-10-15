//! Paid adapter for the existing exact isolated Gaussian-group activation lane.
//! Source policy, raw tuples, parameters and native payloads remain retained.
use super::*;
use crate::{DocumentSnapshot, accounted_view::AccountedView, channel_cmds::ChannelView, document_history::DocumentHistory, source_policy::SourceIdentity};
use photocraft_doc::{
    accounted_document::{AccountedDocument, GaussianGroupEnabledPatch, accounted_layer_access},
    item_identity::{FilterId, ItemRef},
};

pub(super) const REPLY_ENVELOPE: u64 = 8192;
#[derive(Clone, Copy)]
struct Fields {
    filter: FilterId,
    enabled: bool,
}
// Exact existing activation wire for both IDs; no operation/source authority.
fn wire(id: &str, params: &Value) -> Option<Fields> {
    if !matches!(id, "xcf.filter.set" | "xcf.filter.enable") {
        return None;
    }
    let p = params.as_object()?;
    if p.len() != 2 {
        return None;
    }
    let text = p.get("filterId")?.as_str()?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|v| v.is_ascii_digit()) {
        return None;
    }
    let filter = FilterId(text.parse::<u64>().ok()?);
    Some(Fields { filter, enabled: p.get("enabled")?.as_bool()? })
}
pub(super) struct Request<'a> {
    id: &'static str,
    params: &'a Value,
    fields: Fields,
}
impl<'a> Request<'a> {
    pub(super) fn parse(session: &Session, id: &str, params: &'a Value, ctl: &ReadControl<'_>) -> Result<Self> {
        ctl.check(1)?;
        let id = match id {
            "xcf.filter.set" => "xcf.filter.set",
            "xcf.filter.enable" => "xcf.filter.enable",
            _ => return Err(BackingError::Unsupported.into()),
        };
        let fields = wire(id, params).ok_or_else(|| bad(id, "accounted isolated Gaussian group supports only exact filter activation"))?;
        // The closed shape has two fixed keys, twenty identity digits and one
        // boolean. Price its
        // repeated upstream wire/identity checks before invoking those helpers.
        ctl.check(512)?;
        crate::xcf_filter_cmds::pre_dispatch_set_change(session, id, params, false)?;
        // Preserve the upstream canonical identity parser as the command authority.
        if crate::xcf_filter_cmds::id(id, params)? != fields.filter {
            return Err(BackingError::Integrity.into());
        }
        Ok(Self { id, params, fields })
    }
    pub(super) fn prepare(self, session: &Session, ctl: &ReadControl<'_>) -> Result<Prepared<'a>> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        owner.validate_gaussian_group_storage(ctl)?;
        ctl.check(1)?;
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        let xcf = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let stack = xcf.filter_stacks.first().ok_or(BackingError::Integrity)?;
        let node = stack
            .nodes
            .first()
            .filter(|n| n.id == self.fields.filter)
            .ok_or_else(|| bad(self.id, "filterId does not identify the current isolated Gaussian group filter"))?;
        let ItemRef::Layer(layer) = stack.owner else { return Err(BackingError::Unsupported.into()) };
        let visibility_changed = self.fields.enabled != node.enabled;
        locks(owner, layer, visibility_changed, ctl)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        channel_shape(&state.channel_view)?;
        if matches!(state.channel_view.target,crate::channel_cmds::ChannelTarget::Color(i) if i>=state.doc.mode.color_channels()) {
            return Err(BackingError::Bounds.into());
        }
        if state.path.as_ref().is_some_and(|v| v.len() > 4096)
            || state.source.as_ref().is_some_and(|s| s.label.len() > 4096 || s.path.as_ref().is_some_and(|v| v.len() > 4096))
        {
            return Err(BackingError::Length.into());
        }
        let strings = state
            .path
            .as_ref()
            .map_or(0, String::len)
            .checked_add(state.source.as_ref().map_or(0, |s| s.label.len() + s.path.as_ref().map_or(0, String::len)))
            .ok_or(BackingError::Length)?;
        let count = state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;
        let envelope = strings
            .checked_add(count)
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(std::mem::size_of::<Prepared<'_>>() + 512))
            .ok_or(BackingError::Length)?;
        let mut lease = owner.quota().reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
        ctl.check(strings.checked_add(count).ok_or(BackingError::Length)? as u64)?;
        let path = state.path.as_deref().map(copy).transpose()?;
        let source = state
            .source
            .as_ref()
            .map(|s| Ok::<_, EngineError>(SourceIdentity { label: copy(&s.label)?, path: s.path.as_deref().map(copy).transpose()?, origin: s.origin }))
            .transpose()?;
        let channel = ChannelView {
            target: state.channel_view.target,
            color_hidden: flags(&state.channel_view.color_hidden, ctl)?,
            alpha_visible: flags(&state.channel_view.alpha_visible, ctl)?,
            quick_mask_hidden: state.channel_view.quick_mask_hidden,
            layer_mask: None,
            editing_mask: None,
        };
        let actual = path
            .as_ref()
            .map_or(0, String::capacity)
            .checked_add(source.as_ref().map_or(0, |s| s.label.capacity() + s.path.as_ref().map_or(0, String::capacity)))
            .and_then(|v| v.checked_add(channel.color_hidden.capacity() + channel.alpha_visible.capacity()))
            .and_then(|v| v.checked_add(std::mem::size_of::<Prepared<'_>>() + 512))
            .ok_or(BackingError::Length)?;
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        validate_recipe(owner, ctl)?;
        let staged = owner.try_with_gaussian_group_enabled(
            GaussianGroupEnabledPatch { layer, filter: self.fields.filter, expected_stack_revision: stack.revision, enabled: self.fields.enabled },
            ctl,
        )?;
        validate_recipe(&staged, ctl)?;
        let updated = !staged.shares_model(owner);
        let mut history = state.history.clone();
        let snapshot = if updated {
            history.max_states = session.prefs().performance.history_states.max(1) as usize;
            history.max_bytes = session.prefs().performance.history_budget_bytes();
            history.set_current_layers_borrowed(view.target(), ctl)?;
            history.try_commit_prepared_accounted(owner, staged, "Set GIMP Group Filter Enabled", view.target(), ctl)?
        } else {
            state.doc.clone()
        };
        let next_view = view.try_replace(snapshot.accounted().ok_or(BackingError::Integrity)?, state.active_layer, view.selected(), state.layer_anchor, ctl)?;
        let reply_lease = owner.quota().reserve(QuotaKind::Index, REPLY_ENVELOPE).map_err(|_| BackingError::Quota)?;
        ctl.check(REPLY_ENVELOPE)?;
        // Canonical wire identity is at most twenty bytes. Retain that exact
        // bounded string; JSON contains only two fixed keys and scalar cells.
        let identity = copy(self.params.get("filterId").and_then(Value::as_str).ok_or(BackingError::Integrity)?)?;
        let reply = serde_json::json!({"filterId":identity,"updated":updated});
        ctl.check(1)?;
        Ok(Prepared {
            request: self,
            captured: state.doc.clone(),
            snapshot,
            history,
            old_history: state.history.clone(),
            view: view.clone(),
            next_view,
            revision: state.revision,
            view_revision: state.view_revision,
            active: state.active_layer,
            anchor: state.layer_anchor,
            channel,
            path,
            source,
            read_only: state.source_read_only,
            history_states: session.prefs().performance.history_states,
            history_bytes: session.prefs().performance.history_budget_bytes(),
            layer,
            visibility_changed,
            updated,
            reply,
            _reply_lease: reply_lease,
            _lease: lease,
        })
    }
}

fn copy(v: &str) -> Result<String> {
    let mut s = String::new();
    s.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if s.capacity() > v.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    s.push_str(v);
    Ok(s)
}
fn flags(v: &[bool], ctl: &ReadControl<'_>) -> Result<Vec<bool>> {
    let mut out = Vec::new();
    out.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if out.capacity() > v.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for b in v {
        ctl.check(1)?;
        out.push(*b);
    }
    Ok(out)
}
fn channel_shape(v: &ChannelView) -> Result<()> {
    if v.color_hidden.len() > 32 || v.alpha_visible.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    if v.layer_mask.is_some()
        || v.editing_mask.is_some()
        || !matches!(v.target, crate::channel_cmds::ChannelTarget::Composite | crate::channel_cmds::ChannelTarget::Color(_))
    {
        return Err(BackingError::Unsupported.into());
    }
    Ok(())
}
fn locks(owner: &AccountedDocument, layer: LayerId, visibility: bool, ctl: &ReadControl<'_>) -> Result<()> {
    let a = accounted_layer_access(owner.document(), layer, ctl)?.ok_or(EngineError::NoLayer(layer))?;
    if a.locks.all || a.locks.pixels || a.imported_locks.content || visibility && a.imported_locks.visibility {
        return Err(EngineError::Other("the filter owner is locked".into()));
    }
    Ok(())
}
fn validate_recipe(owner: &AccountedDocument, ctl: &ReadControl<'_>) -> Result<()> {
    crate::accounted_xcf_open::gaussian_group::validate_current_isolated_gaussian_group_storage(owner, ctl)
}
pub(super) struct Prepared<'a> {
    request: Request<'a>,
    captured: DocumentSnapshot,
    snapshot: DocumentSnapshot,
    history: DocumentHistory,
    old_history: DocumentHistory,
    view: AccountedView,
    next_view: AccountedView,
    revision: u64,
    view_revision: u64,
    active: Option<LayerId>,
    anchor: Option<LayerId>,
    channel: ChannelView,
    path: Option<String>,
    source: Option<SourceIdentity>,
    read_only: bool,
    history_states: u32,
    history_bytes: usize,
    layer: LayerId,
    visibility_changed: bool,
    updated: bool,
    reply: Value,
    _reply_lease: QuotaLease,
    _lease: QuotaLease,
}
impl Prepared<'_> {
    pub(super) fn execute(self, session: &mut Session, ctl: &ReadControl<'_>) -> Result<Value> {
        ctl.check(512)?;
        crate::xcf_filter_cmds::pre_dispatch_set_change(session, self.request.id, self.request.params, false)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        channel_shape(&state.channel_view)?;
        ctl.check((self.channel.color_hidden.len() + self.channel.alpha_visible.len() + 1) as u64)?;
        for text in self.path.iter().chain(self.source.iter().flat_map(|s| std::iter::once(&s.label).chain(s.path.iter()))) {
            ctl.check(text.len() as u64)?;
        }
        if !state.doc.shares_state(&self.captured)
            || state.revision != self.revision
            || state.view_revision != self.view_revision
            || !view.shares_state(&self.view)
            || state.active_layer != self.active
            || state.layer_anchor != self.anchor
            || !state.selected_layers.is_empty()
            || state.channel_view != self.channel
            || state.path != self.path
            || state.source != self.source
            || state.source_read_only != self.read_only
            || !state.history.shares_accounted_history(&self.old_history)
            || state.history.max_states != self.old_history.max_states
            || state.history.max_bytes != self.old_history.max_bytes
            || session.prefs().performance.history_states != self.history_states
            || session.prefs().performance.history_budget_bytes() != self.history_bytes
        {
            return Err(EngineError::Other("the prepared isolated Gaussian group edit targets older Session state".into()));
        }
        locks(state.doc.accounted().ok_or(BackingError::Integrity)?, self.layer, self.visibility_changed, ctl)?;
        let revision = state.revision.checked_add(u64::from(self.updated)).ok_or(BackingError::Length)?;
        ctl.check(1)?;
        if self.updated {
            let state = session.active_mut().ok_or(EngineError::NoDocument)?;
            state.doc = self.snapshot;
            state.history = self.history;
            state.accounted_view = Some(self.next_view);
            state.revision = revision;
            state.last_damage = None;
            state.coalesce = None;
            state._accounted_metadata_gesture = None;
        }
        Ok(self.reply)
    }
}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "gaussian_group_tests.rs"]
mod tests;
