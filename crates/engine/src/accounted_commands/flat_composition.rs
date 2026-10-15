//! Paid adapter to the existing explicit CURRENT composition wire.
//! Original-source gates remain independent of this actual-class transaction.
use super::*;
use crate::{
    DocumentSnapshot, accounted_session::assigned_fence::AssignedFence, accounted_view::AccountedView, channel_cmds::ChannelView,
    document_history::DocumentHistory,
};
use photocraft_doc::accounted_document::{AccountedDocument, FlatCompositeCompositionPatch, accounted_layer_access};
pub(super) const REPLY_ENVELOPE: u64 = 8192;
const ID: &str = "xcf.composition.set";
pub(super) struct Request<'a> {
    params: &'a Value,
    patch: FlatCompositeCompositionPatch,
}
impl<'a> Request<'a> {
    pub(super) fn parse(s: &Session, p: &'a Value, c: &ReadControl<'_>) -> Result<Self> {
        c.check(1)?;
        let owner = s.active().and_then(|s| s.doc.accounted()).filter(|o| o.is_readonly_flat_composite_storage()).ok_or(BackingError::Unsupported)?;
        let fields = p.as_object().filter(|m| m.len() <= 8).ok_or_else(|| bad(ID, "expected bounded explicit composition fields"))?;
        // The shared parser rejects every unknown key and every unsupported value.
        // Fixed valid strings are <=20bytes; prepay bounded upstream parse/error work
        // and actual temporary formatting before calling it, never afterward.
        c.check(fields.len() as u64 + 512)?;
        let _parse = owner.quota().reserve(QuotaKind::Index, 2048).map_err(|_| BackingError::Quota)?;
        let patch = crate::xcf_composition_cmds::paid_flat_composition_patch(s, p)?;
        Ok(Self { params: p, patch })
    }
    pub(super) fn prepare(self, s: &Session, c: &ReadControl<'_>) -> Result<Prepared<'a>> {
        let state = s.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        owner.validate_readonly_flat_composite_storage(c)?;
        let fence = AssignedFence::prepare::<Prepared<'_>>(s, c)?;
        locks(owner, self.patch.layer, c)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        imported_metadata::channel_check(&state.channel_view, state.doc.mode.color_channels())?;
        let count = state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;
        let envelope = count.checked_mul(2).and_then(|n| n.checked_add(std::mem::size_of::<ChannelView>() + 128)).ok_or(BackingError::Length)?;
        let mut channel_lease = owner.quota().reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
        let channel = ChannelView {
            target: state.channel_view.target,
            color_hidden: imported_metadata::booleans(&state.channel_view.color_hidden, c)?,
            alpha_visible: imported_metadata::booleans(&state.channel_view.alpha_visible, c)?,
            quick_mask_hidden: state.channel_view.quick_mask_hidden,
            layer_mask: None,
            editing_mask: None,
        };
        let actual = channel
            .color_hidden
            .capacity()
            .checked_add(channel.alpha_visible.capacity())
            .and_then(|n| n.checked_add(std::mem::size_of::<ChannelView>() + 128))
            .ok_or(BackingError::Length)?;
        channel_lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        validate(owner, c)?;
        let staged = owner.try_with_flat_composite_composition_patch(self.patch, c)?;
        validate(&staged, c)?;
        let changed = !staged.shares_model(owner);
        let mut history = state.history.clone();
        let snapshot = if changed {
            history.max_states = s.prefs().performance.history_states.max(1) as usize;
            history.max_bytes = s.prefs().performance.history_budget_bytes();
            history.set_current_layers_borrowed(view.target(), c)?;
            history.try_commit_prepared_accounted(owner, staged, "Set GIMP Composition Policy", view.target(), c)?
        } else {
            state.doc.clone()
        };
        let next_view = view.try_replace(snapshot.accounted().ok_or(BackingError::Integrity)?, state.active_layer, view.selected(), state.layer_anchor, c)?;
        let reply_lease = owner.quota().reserve(QuotaKind::Index, REPLY_ENVELOPE).map_err(|_| BackingError::Quota)?;
        c.check(REPLY_ENVELOPE)?;
        // Upstream returns updated:true even for a state-preserving raw no-op.
        let reply = serde_json::json!({"layerId":self.patch.layer.0.to_string(),"sourceMode":self.patch.source_mode,"updated":true});
        c.check(1)?;
        Ok(Prepared {
            request: self,
            captured: state.doc.clone(),
            snapshot,
            history,
            view: view.clone(),
            next_view,
            revision: state.revision,
            view_revision: state.view_revision,
            active: state.active_layer,
            anchor: state.layer_anchor,
            channel,
            changed,
            reply,
            fence,
            _channel_lease: channel_lease,
            _reply_lease: reply_lease,
        })
    }
}
fn locks(owner: &AccountedDocument, id: LayerId, c: &ReadControl<'_>) -> Result<()> {
    let a = accounted_layer_access(owner.document(), id, c)?.ok_or(EngineError::NoLayer(id))?;
    crate::xcf_item_locks::EffectiveItemLocks { native: a.locks, imported: a.imported_locks }.require(crate::xcf_item_locks::EditKind::Content)
}
fn validate(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
    crate::accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(owner, c)
}
pub(super) struct Prepared<'a> {
    request: Request<'a>,
    captured: DocumentSnapshot,
    snapshot: DocumentSnapshot,
    history: DocumentHistory,
    view: AccountedView,
    next_view: AccountedView,
    revision: u64,
    view_revision: u64,
    active: Option<LayerId>,
    anchor: Option<LayerId>,
    channel: ChannelView,
    changed: bool,
    reply: Value,
    // Every heap-bearing payload above is destroyed before all of its fees.
    fence: AssignedFence,
    _channel_lease: QuotaLease,
    _reply_lease: QuotaLease,
}
impl Prepared<'_> {
    pub(super) fn execute(self, s: &mut Session, c: &ReadControl<'_>) -> Result<Value> {
        c.check(512)?;
        let owner = s.active().and_then(|s| s.doc.accounted()).ok_or(BackingError::Unsupported)?;
        let _parse = owner.quota().reserve(QuotaKind::Index, 2048).map_err(|_| BackingError::Quota)?;
        let repeated = crate::xcf_composition_cmds::paid_flat_composition_patch(s, self.request.params)?;
        // Exact borrowed wire is immutable; repeat captured-target/revision headroom
        // and parser admission before final state/fence/locks checks.
        if repeated.layer != self.request.patch.layer
            || repeated.source_mode != self.request.patch.source_mode
            || repeated.blend_space != self.request.patch.blend_space
            || repeated.composite_space != self.request.patch.composite_space
            || repeated.operator != self.request.patch.operator
        {
            return Err(BackingError::Integrity.into());
        }
        self.fence.check(s, c)?;
        let state = s.active().ok_or(EngineError::NoDocument)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        imported_metadata::channel_check(&state.channel_view, state.doc.mode.color_channels())?;
        c.check((self.channel.color_hidden.len() + self.channel.alpha_visible.len() + 1) as u64)?;
        if !state.doc.shares_state(&self.captured)
            || state.revision != self.revision
            || state.view_revision != self.view_revision
            || !view.shares_state(&self.view)
            || state.active_layer != self.active
            || state.layer_anchor != self.anchor
            || !state.selected_layers.is_empty()
            || state.channel_view != self.channel
        {
            return Err(EngineError::Other("the prepared composition edit targets older Session state".into()));
        }
        locks(state.doc.accounted().ok_or(BackingError::Integrity)?, self.request.patch.layer, c)?;
        let revision = state.revision.checked_add(u64::from(self.changed)).ok_or(BackingError::Length)?;
        c.check(1)?;
        if self.changed {
            let state = s.active_mut().ok_or(EngineError::NoDocument)?;
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
