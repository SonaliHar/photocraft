//! Typed current assigned-geometry layer-mask enablement. No source or pixel grant.
use super::*;
use crate::{accounted_session::assigned_fence::AssignedFence, accounted_view::AccountedView, channel_cmds::ChannelView};
use photocraft_doc::{
    accounted_document::{AccountedDocument, accounted_layer_access},
    item_identity::{ItemRef, MaskId},
};

pub(super) const REPLY_ENVELOPE: u64 = 8192;
fn mask_error(code: &str) -> EngineError {
    EngineError::Other(format!("xcf.mask.enabled.{code}"))
}
#[derive(Clone, Copy)]
struct Relation {
    mask: MaskId,
    layer: LayerId,
    previous: bool,
}
#[derive(Clone, Copy)]
enum Target {
    Mask(MaskId),
    Layer(LayerId),
}
fn relation(owner: &AccountedDocument, target: Target, c: &ReadControl<'_>) -> Result<Relation> {
    owner.validate_assigned_analytic_geometry_storage(c)?;
    let doc = owner.document();
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut found = None;
    for item in &state.items {
        c.check(1)?;
        let (ItemRef::Mask(id), Some(metadata)) = (item.item, item.mask.as_ref()) else { continue };
        let ItemRef::Layer(layer) = metadata.owner else { return Err(mask_error("unsupported-owner")) };
        let matches = match target {
            Target::Mask(want) => id == want,
            Target::Layer(want) => layer == want,
        };
        if !matches {
            continue;
        }
        if found.is_some() {
            return Err(mask_error("ambiguous-owner"));
        }
        let access = accounted_layer_access(doc, layer, c)?.ok_or_else(|| mask_error("missing-owner"))?;
        let native = access.layer.mask.as_ref().ok_or_else(|| mask_error("missing-native-mask"))?;
        if native.enabled != metadata.enabled {
            return Err(mask_error("unsupported-mask-domain"));
        }
        // The admitted structure is flat and the mask has no parent. Mask
        // effective native locks are its actual owner's; imported content is
        // the union of that owner and this exact canonical mask item.
        if access.locks.all || access.imported_locks.content || item.locks.content {
            return Err(mask_error("locked"));
        }
        found = Some(Relation { mask: id, layer, previous: metadata.enabled });
    }
    found.ok_or_else(|| mask_error("missing-mask"))
}
fn channel_count(channel: &ChannelView) -> Result<usize> {
    if channel.color_hidden.len() > 32 || channel.alpha_visible.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    channel.color_hidden.len().checked_add(channel.alpha_visible.len()).ok_or_else(|| BackingError::Length.into())
}
fn flags(values: &[bool], c: &ReadControl<'_>) -> Result<Vec<bool>> {
    let mut out = Vec::new();
    out.try_reserve_exact(values.len()).map_err(|_| BackingError::Allocation)?;
    if out.capacity() > values.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for value in values {
        c.check(1)?;
        out.push(*value);
    }
    Ok(out)
}
pub(super) struct Request<'a> {
    params: &'a Value,
    mask: MaskId,
    enabled: bool,
}
impl<'a> Request<'a> {
    pub(super) fn parse(session: &Session, params: &'a Value, c: &ReadControl<'_>) -> Result<Self> {
        c.check(64)?;
        crate::xcf_mask_cmds::dispatch_preflight(session, params, false)?;
        let (mask, enabled) = crate::xcf_mask_cmds::envelope(params)?;
        let owner = session.active().and_then(|state| state.doc.accounted()).ok_or(BackingError::Unsupported)?;
        if !crate::assigned_geometry_metadata_candidate(owner) {
            return Err(BackingError::Unsupported.into());
        }
        Ok(Self { params, mask, enabled })
    }
    pub(super) fn prepare(self, session: &Session, c: &ReadControl<'_>) -> Result<Prepared<'a>> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        let found = relation(owner, Target::Mask(self.mask), c)?;
        owner.validate_assigned_analytic_mask_edit(AccountedEdit::MaskEnabled { layer: found.layer, enabled: self.enabled }, c)?;
        crate::accounted_xcf_open::analytic_geometry::validate_current_analytic_geometry_storage(owner, c)?;
        let fence = AssignedFence::prepare::<Prepared<'_>>(session, c)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        let count = channel_count(&state.channel_view)?;
        let envelope = count.checked_mul(2).and_then(|n| n.checked_add(std::mem::size_of::<ChannelView>() + 128)).ok_or(BackingError::Length)?;
        let mut channel_fee = owner.quota().reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
        let channel = ChannelView {
            target: state.channel_view.target,
            color_hidden: flags(&state.channel_view.color_hidden, c)?,
            alpha_visible: flags(&state.channel_view.alpha_visible, c)?,
            quick_mask_hidden: state.channel_view.quick_mask_hidden,
            layer_mask: state.channel_view.layer_mask,
            editing_mask: state.channel_view.editing_mask,
        };
        let actual = channel
            .color_hidden
            .capacity()
            .checked_add(channel.alpha_visible.capacity())
            .and_then(|n| n.checked_add(std::mem::size_of::<ChannelView>() + 128))
            .ok_or(BackingError::Length)?;
        if actual > envelope {
            return Err(BackingError::Allocation.into());
        }
        channel_fee.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        let reply_fee = owner.quota().reserve(QuotaKind::Index, REPLY_ENVELOPE).map_err(|_| BackingError::Quota)?;
        c.check(REPLY_ENVELOPE)?;
        let reply = serde_json::json!({"maskId":found.mask.0.to_string(),"layerId":found.layer.0.to_string(),"enabled":self.enabled,"changed":found.previous!=self.enabled});
        for key in ["maskId", "layerId"] {
            let Value::String(text) = reply.get(key).ok_or(BackingError::Integrity)? else { return Err(BackingError::Integrity.into()) };
            if text.capacity() > 40 {
                return Err(BackingError::Allocation.into());
            }
        }
        let prepared = Prepared {
            request: self,
            found,
            fence,
            view: view.clone(),
            active: state.active_layer,
            anchor: state.layer_anchor,
            channel,
            reply,
            _reply_fee: reply_fee,
            _channel_fee: channel_fee,
        };
        c.check(1)?;
        Ok(prepared)
    }
}
pub(super) struct Prepared<'a> {
    request: Request<'a>,
    found: Relation,
    fence: AssignedFence,
    view: AccountedView,
    active: Option<LayerId>,
    anchor: Option<LayerId>,
    channel: ChannelView,
    reply: Value,
    _reply_fee: QuotaLease,
    _channel_fee: QuotaLease,
}
impl Prepared<'_> {
    pub(super) fn execute(self, session: &mut Session, c: &ReadControl<'_>) -> Result<Value> {
        c.check(64)?;
        crate::xcf_mask_cmds::dispatch_preflight(session, self.request.params, false)?;
        self.fence.check(session, c)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        c.check(
            channel_count(&state.channel_view)?.checked_add(channel_count(&self.channel)?).and_then(|n| n.checked_add(1)).ok_or(BackingError::Length)? as u64
        )?;
        if !view.shares_state(&self.view)
            || state.active_layer != self.active
            || state.layer_anchor != self.anchor
            || !state.selected_layers.is_empty()
            || state.channel_view != self.channel
        {
            return Err(mask_error("stale-target"));
        }
        let found = relation(state.doc.accounted().ok_or(BackingError::Integrity)?, Target::Mask(self.found.mask), c)?;
        if found.layer != self.found.layer || found.previous != self.found.previous {
            return Err(mask_error("stale-target"));
        }
        session.edit_accounted_assigned_mask_enabled("Set XCF Layer Mask Enabled", found.layer, self.request.enabled, c)?;
        Ok(self.reply)
    }
}
impl Session {
    /// Current-only typed mask transaction; no coalescing or source permission.
    pub(crate) fn edit_accounted_assigned_mask_enabled(&mut self, label: &str, layer: LayerId, enabled: bool, c: &ReadControl<'_>) -> Result<bool> {
        c.check(1)?;
        if self.coalesce_request.is_some() {
            return Err(mask_error("coalescing"));
        }
        if self.command_is_background() || self.command_has_pending_job() || self.has_jobs() || self.color_restrict.is_some() {
            return Err(mask_error("context"));
        }
        let state = self.active_mut().ok_or(EngineError::NoDocument)?;
        if state.floating.is_some() {
            return Err(mask_error("floating"));
        }
        if state.coalesce.is_some() {
            return Err(mask_error("coalescing"));
        }
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        let edit = AccountedEdit::MaskEnabled { layer, enabled };
        owner.validate_assigned_analytic_mask_edit(edit, c)?;
        let _found = relation(owner, Target::Layer(layer), c)?;
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        crate::accounted_xcf_open::analytic_geometry::validate_current_analytic_geometry_storage(owner, c)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        channel_count(&state.channel_view)?;
        let mut history = state.history.clone();
        history.set_current_layers_borrowed(view.target(), c)?;
        let snapshot = history.try_commit_accounted(edit, label, view.target(), c)?;
        if snapshot.shares_state(&state.doc) {
            c.check(1)?;
            return Ok(false);
        }
        let current = snapshot.accounted().ok_or(BackingError::Integrity)?;
        crate::accounted_xcf_open::analytic_geometry::validate_current_analytic_geometry_storage(current, c)?;
        let next_view = view.try_replace(current, state.active_layer, view.selected(), state.layer_anchor, c)?;
        let revision = state.revision.checked_add(1).ok_or(BackingError::Length)?;
        c.check(1)?;
        state.doc = snapshot;
        state.history = history;
        state.accounted_view = Some(next_view);
        state.revision = revision;
        state.last_damage = None;
        state.coalesce = None;
        state._accounted_metadata_gesture = None;
        Ok(true)
    }
}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "assigned_mask_tests.rs"]
mod tests;
