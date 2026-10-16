//! Paid CURRENT translation of a closed plain or linked assigned foreground.
//! Source pixels/locators remain immutable. This grants no new source family.
use super::*;
use crate::{
    accounted_session::{assigned_fence::AssignedFence, metadata_gesture::PaidMetadataGesture},
    accounted_view::AccountedView,
    channel_cmds::ChannelView,
};
use photocraft_doc::accounted_document::{AccountedDocument, accounted_layer_access};
#[path = "imported_translate_targets.rs"]
mod targets;
use targets::{TargetMode, Targets};
const ID: &str = "layer.translate";
fn invalid(message: &str) -> EngineError {
    bad(ID, message)
}
/// A routing hint only; the actual L1/current renderer gates remain mandatory.
pub(super) fn candidate(owner: &AccountedDocument) -> bool {
    crate::observed_box_translation_candidate(owner) || (!owner.is_readonly_assigned_point_storage()
        && !owner.is_readonly_gaussian_group_storage()
        && !owner.is_readonly_point_storage()
        && !owner.is_readonly_threshold_storage()
        && !owner.is_readonly_threshold_alpha_storage()
        && !owner.document().layers.is_empty()
        && owner.document().layers.len() <= 4096
        && owner.document().xcf_state.is_some()
        && owner.document().source_color_context.is_some()
        && owner.document().layers.first().is_some_and(|l| matches!(&l.content, photocraft_doc::LayerContent::BackedRaster(_))))
}
#[derive(Clone, Copy)]
enum MovePolicy {
    ObservedBox,
    RefusedPoint,
    FlatComposite,
    Plain,
    Assigned,
}
impl MovePolicy {
    fn for_owner(owner: &AccountedDocument) -> Self {
        if crate::observed_box_translation_candidate(owner) { Self::ObservedBox }
        else if owner.is_readonly_point_storage() || owner.is_readonly_assigned_point_storage() { Self::RefusedPoint }
        else if owner.is_readonly_flat_composite_storage() { Self::FlatComposite } else if super::assigned_translate::candidate(owner) { Self::Assigned } else { Self::Plain }
    }
    fn validate(self, owner: &AccountedDocument, layers: &[LayerId], dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<()> {
        match self {
            Self::RefusedPoint => Err(BackingError::Unsupported.into()),
            Self::ObservedBox => {
                let [layer] = layers else { return Err(BackingError::Unsupported.into()) };
                Ok(owner.validate_readonly_point_layer_translation(*layer,dx,dy,c)?)
            }
            Self::FlatComposite => Ok(owner.validate_flat_composite_layers_translation(layers, dx, dy, c)?),
            Self::Plain => Ok(owner.validate_imported_layers_translation(layers, dx, dy, c)?),
            Self::Assigned => {
                let [layer] = layers else { return Err(BackingError::Unsupported.into()) };
                super::assigned_translate::validate(owner, *layer, dx, dy, c)
            }
        }
    }
    fn stage(self, owner: &AccountedDocument, layers: &[LayerId], dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<AccountedDocument> {
        match self {
            Self::RefusedPoint => Err(BackingError::Unsupported.into()),
            Self::ObservedBox => {
                let [layer] = layers else { return Err(BackingError::Unsupported.into()) };
                Ok(owner.try_with_readonly_point_layer_translation(*layer,dx,dy,c)?)
            }
            Self::FlatComposite => Ok(owner.try_with_flat_composite_layers_translation(layers, dx, dy, c)?),
            Self::Plain => Ok(owner.try_with_imported_layers_translation(layers, dx, dy, c)?),
            Self::Assigned => {
                let [layer] = layers else { return Err(BackingError::Unsupported.into()) };
                super::assigned_translate::stage(owner, *layer, dx, dy, c)
            }
        }
    }
    fn current(self, owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
        match self {
            Self::RefusedPoint => Err(BackingError::Unsupported.into()),
            Self::ObservedBox => crate::accounted_native_open::validate_current_observed_box_storage(owner,c),
            Self::FlatComposite => crate::accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(owner, c),
            Self::Plain => current(owner, c),
            Self::Assigned => super::assigned_translate::current(owner, c),
        }
    }
}
fn wire(session: &Session, p: &Value) -> Result<(i32, i32)> {
    // Existing resident diagnostics format an unknown unit verbatim. Refuse
    // this bounded paid wire before that potentially attacker-sized copy.
    if p.get("units").and_then(Value::as_str).is_some_and(|unit| !matches!(unit, "px" | "pt")) {
        return Err(invalid("unknown units (px|pt)"));
    }
    let delta = crate::layer_multi_cmds::translation_delta(session, p)?;
    if p.is_null() {
        return Ok(delta);
    }
    let object = p.as_object().filter(|o| o.len() <= 6).ok_or_else(|| invalid("bounded Move parameters required"))?;
    if object.keys().any(|key| !matches!(key.as_str(), "layer" | "dx" | "dy" | "units" | "target" | "coalesce")) {
        return Err(invalid("unsupported owned Move field"));
    }
    if object.get("coalesce").and_then(Value::as_str).is_some_and(|s| s.len() > 128) {
        return Err(invalid("coalesce string exceeds 128 bytes"));
    }
    // The upstream handler returns before even malformed target resolution on
    // rounded zero deltas. This is not a class/renderer admission or an edit.
    if delta == (0, 0) {
        return Ok(delta);
    }
    let policy = session.active().and_then(|state| state.doc.accounted()).map(MovePolicy::for_owner).ok_or(BackingError::Unsupported)?;
    if p.get("target").and_then(Value::as_str).is_some_and(|s| s == "quickMask" || (s == "mask" && !matches!(policy, MovePolicy::Assigned)))
        || p.get("target").and_then(|v| v.get("channel")).and_then(Value::as_u64).is_some()
    {
        return Err(invalid("owned Move supports plain pixels or an assigned linked foreground only"));
    }
    if p.get("layer").and_then(Value::as_str).is_some_and(|s| s.len() > MAX_STRING) {
        return Err(invalid("Move layer name exceeds limit"));
    }
    Ok(delta)
}
pub(super) fn dispatch_preflight(session: &Session, p: &Value, background: bool) -> Result<()> {
    if background {
        return Err(invalid("owned Move requires synchronous dispatch"));
    }
    let owner = session.active().and_then(|s| s.doc.accounted()).ok_or(BackingError::Unsupported)?;
    if !candidate(owner) {
        return Err(BackingError::Unsupported.into());
    }
    if !p.is_null() && p.as_object().is_none_or(|o| o.len() > 6) {
        return Err(invalid("bounded Move parameters required"));
    }
    Ok(())
}
fn hidden_preflight(session: &Session, p: &Value, c: &ReadControl<'_>) -> Result<()> {
    if p.get("target").and_then(Value::as_str) == Some("quickMask") || p.get("target").and_then(|v| v.get("channel")).and_then(Value::as_u64).is_some() {
        return Ok(());
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    // Match the ordinary dispatch guard: names use active focus here, while
    // numeric explicit targets use that identity. The later target lookup still
    // validates the real named layer independently. Plain storage is flat.
    let id = p.get("layer").and_then(Value::as_u64).map(LayerId).or(state.active_layer);
    if let Some(id) = id {
        for layer in &state.doc.layers {
            c.check(1)?;
            if layer.id == id && !layer.visible {
                return Err(EngineError::Other(crate::hidden_target::HIDDEN.into()));
            }
        }
    }
    Ok(())
}
fn context(session: &Session) -> Result<()> {
    if session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() {
        return Err(invalid("Move context is unsupported"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.accounted().is_some_and(|o|o.is_readonly_filtered_flat_point_storage() || o.is_readonly_masked_filtered_flat_point_storage()) { return Err(BackingError::Unsupported.into()); }
    if state.floating.is_some() || state.coalesce.is_some() {
        return Err(invalid("Move requires no floating or resident coalescing state"));
    }
    Ok(())
}
fn target_check(owner: &AccountedDocument, layer: LayerId, c: &ReadControl<'_>) -> Result<()> {
    let access = accounted_layer_access(owner.document(), layer, c)?.ok_or(EngineError::NoLayer(layer))?;
    if (access.layer.mask.is_some() && !matches!(MovePolicy::for_owner(owner), MovePolicy::Assigned)) || access.layer.link_group.is_some() {
        return Err(invalid("Move masks and linked-layer expansion are not qualified"));
    }
    // Ordinary move_layers enforces native inherited position/all. Canonical
    // imported.position is preserved, not introduced as an extra command ban.
    if access.locks.all || access.locks.position {
        return Err(invalid("the imported layer is position-locked"));
    }
    Ok(())
}
fn view_check(state: &crate::DocState, c: &ReadControl<'_>) -> Result<()> {
    let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
    if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
        return Err(BackingError::Integrity.into());
    }
    c.check(view.selected().len() as u64)?;
    if view.selected().len() > 4096 {
        return Err(BackingError::Length.into());
    }
    let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
    if matches!(MovePolicy::for_owner(owner), MovePolicy::Assigned) && (view.selected().len() != 1 || state.active_layer != view.selected().first().copied()) {
        return Err(invalid("assigned Move requires one selected foreground"));
    }
    super::imported_metadata::channel_check(&state.channel_view, state.doc.mode.color_channels())?;
    Ok(())
}
pub(super) fn current_error(error: photocraft_compose::backed_normal::BackedNormalError, c: &ReadControl<'_>, context: &str) -> EngineError {
    match error {
        photocraft_compose::backed_normal::BackedNormalError::Backing(e) => e.into(),
        photocraft_compose::backed_normal::BackedNormalError::View(
            photocraft_raster::checked_view::RasterViewError::Backed(e)
            | photocraft_raster::checked_view::RasterViewError::Control(e)
            | photocraft_raster::checked_view::RasterViewError::Descriptor(e),
        ) => e.into(),
        photocraft_compose::backed_normal::BackedNormalError::Quota(_) => BackingError::Quota.into(),
        photocraft_compose::backed_normal::BackedNormalError::Allocation => BackingError::Allocation.into(),
        _ if c.interrupt.cancelled() => EngineError::Cancelled,
        other => EngineError::Other(format!("{context}: {other}")),
    }
}
fn current(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
    photocraft_compose::backed_plain_current::validate_current_plain_import_storage(owner, c)
        .map_err(|error| current_error(error, c, "owned plain Move current recipe"))
}
pub(super) struct Request<'a> {
    mode: TargetMode,
    dx: i32,
    dy: i32,
    key: Option<&'a str>,
}
impl<'a> Request<'a> {
    pub(super) fn parse(session: &Session, p: &'a Value, c: &ReadControl<'_>) -> Result<Self> {
        c.check(16)?;
        dispatch_preflight(session, p, false)?;
        hidden_preflight(session, p, c)?;
        let (dx, dy) = wire(session, p)?;
        let key = p.get("coalesce").and_then(Value::as_str);
        if (dx, dy) == (0, 0) {
            return Ok(Self { mode: TargetMode::Zero, dx, dy, key });
        }
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let mode = match p.get("layer") {
            None | Some(Value::Null) => TargetMode::Selected,
            Some(Value::Number(n)) => {
                TargetMode::Explicit(LayerId(n.as_u64().ok_or_else(|| invalid("layer must be a non-negative integer identity or name"))?))
            }
            Some(Value::String(name)) => {
                let mut found = None;
                for layer in state.doc.layers.iter().rev() {
                    if super::imported_metadata::same_name(&layer.name, name, c)? {
                        found = Some(layer.id);
                        break;
                    }
                }
                TargetMode::Explicit(found.ok_or_else(|| invalid("no layer has the requested name"))?)
            }
            _ => return Err(invalid("layer must be an identity, bounded name, or null")),
        };
        Ok(Self { mode, dx, dy, key })
    }
    pub(super) fn prepare(self, session: &Session, c: &ReadControl<'_>) -> Result<Prepared<'a>> {
        context(session)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        let targets = Targets::prepare(state, self.mode, c)?;
        if !targets.ids().is_empty() {
            let policy = MovePolicy::for_owner(owner);
            policy.validate(owner, targets.ids(), self.dx, self.dy, c)?;
            policy.current(owner, c)?;
        }
        let fence = AssignedFence::prepare::<Prepared<'_>>(session, c)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        let count = state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;
        if state.channel_view.color_hidden.len() > 32 || state.channel_view.alpha_visible.len() > 4096 {
            return Err(BackingError::Length.into());
        }
        let bytes = count.checked_mul(2).and_then(|n| n.checked_add(std::mem::size_of::<ChannelView>() + 128)).ok_or(BackingError::Length)?;
        let mut fee = owner.quota().reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
        let channel = ChannelView {
            color_hidden: super::imported_metadata::booleans(&state.channel_view.color_hidden, c)?,
            alpha_visible: super::imported_metadata::booleans(&state.channel_view.alpha_visible, c)?,
            target: state.channel_view.target,
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
        if actual > bytes {
            return Err(BackingError::Allocation.into());
        }
        fee.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        let result = Prepared { request: self, targets, fence, view: view.clone(), channel, _fee: fee };
        c.check(1)?;
        Ok(result)
    }
}
pub(super) struct Prepared<'a> {
    request: Request<'a>,
    targets: Targets,
    fence: AssignedFence,
    view: AccountedView,
    channel: ChannelView,
    _fee: QuotaLease,
}
impl Prepared<'_> {
    pub(super) fn execute(self, session: &mut Session, c: &ReadControl<'_>) -> Result<Value> {
        c.check(1)?;
        self.fence.check(session, c)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        c.check(
            self.channel.color_hidden.len().checked_add(self.channel.alpha_visible.len()).and_then(|n| n.checked_add(1)).ok_or(BackingError::Length)? as u64
        )?;
        if !view.shares_state(&self.view) || state.channel_view != self.channel {
            return Err(invalid("prepared Move view changed"));
        }
        session.translate_accounted_targets(self.targets.ids(), self.request.dx, self.request.dy, self.request.key, c)?;
        Ok(Value::Null)
    }
}
impl Session {
    /// Typed current-only plain or linked assigned-foreground Move. An opaque key shares the
    /// existing document gesture across metadata/Move commands, never grants
    /// source/locator/native pixel authority. No-op/error preserves key and redo.
    pub fn translate_accounted_imported_layer(&mut self, layer: LayerId, dx: i32, dy: i32, key: Option<&str>, c: &ReadControl<'_>) -> Result<()> {
        context(self)?;
        if key.is_some_and(|k| k.len() > 128) {
            return Err(BackingError::Length.into());
        }
        if dx == 0 && dy == 0 {
            c.check(1)?;
            return Ok(());
        }
        let state = self.active().ok_or(EngineError::NoDocument)?;
        let targets = Targets::prepare(state, TargetMode::Explicit(layer), c)?;
        self.translate_accounted_targets(targets.ids(), dx, dy, key, c)
    }
    /// Move the current genuine paid selection in document order as one edit.
    /// This does not accept caller IDs as proof of selected membership.
    pub fn translate_accounted_imported_selection(&mut self, dx: i32, dy: i32, key: Option<&str>, c: &ReadControl<'_>) -> Result<()> {
        context(self)?;
        if key.is_some_and(|k| k.len() > 128) {
            return Err(BackingError::Length.into());
        }
        if dx == 0 && dy == 0 {
            c.check(1)?;
            return Ok(());
        }
        let targets = Targets::prepare(self.active().ok_or(EngineError::NoDocument)?, TargetMode::Selected, c)?;
        self.translate_accounted_targets(targets.ids(), dx, dy, key, c)
    }
    fn translate_accounted_targets(&mut self, layers: &[LayerId], dx: i32, dy: i32, key: Option<&str>, c: &ReadControl<'_>) -> Result<()> {
        c.check(1)?;
        context(self)?;
        if key.is_some_and(|k| k.len() > 128) {
            return Err(BackingError::Length.into());
        }
        if dx == 0 && dy == 0 {
            c.check(1)?;
            return Ok(());
        }
        let state = self.active_mut().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        view_check(state, c)?;
        MovePolicy::for_owner(owner).validate(owner, layers, dx, dy, c)?;
        for layer in layers {
            c.check(1)?;
            target_check(owner, *layer, c)?;
        }
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        let policy = MovePolicy::for_owner(owner);
        policy.current(owner, c)?;
        let staged = policy.stage(owner, layers, dx, dy, c)?;
        if staged.shares_model(owner) {
            c.check(1)?;
            return Ok(());
        }
        policy.current(&staged, c)?;
        let matching = match (key, state._accounted_metadata_gesture.as_ref()) {
            (Some(key), Some(marker)) => marker.matches(key, state.doc.id, owner.quota(), c)?,
            _ => false,
        };
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        let mut history = state.history.clone();
        history.set_current_layers_borrowed(view.target(), c)?;
        let snapshot = if matching && history.can_undo() {
            let expected_history = history.clone();
            history.try_continue_prepared_accounted(&expected_history, owner, staged, view.target(), c)?
        } else {
            history.try_commit_prepared_accounted(owner, staged, "Move", view.target(), c)?
        };
        let current = snapshot.accounted().ok_or(BackingError::Integrity)?;
        let next_view = view.try_replace(current, state.active_layer, view.selected(), state.layer_anchor, c)?;
        let marker = match key {
            None => None,
            Some(_) if matching => state._accounted_metadata_gesture.clone(),
            Some(key) => Some(PaidMetadataGesture::prepare(key, state.doc.id, owner.quota(), c)?),
        };
        let revision = state.revision.checked_add(1).ok_or(BackingError::Length)?;
        c.check(1)?;
        state.doc = snapshot;
        state.history = history;
        state.accounted_view = Some(next_view);
        state.revision = revision;
        state.last_damage = None;
        state.coalesce = None;
        state._accounted_metadata_gesture = marker;
        Ok(())
    }
}

#[cfg(test)]
#[path = "imported_translate_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "imported_translate_bulk_tests.rs"]
mod bulk_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "imported_translate_flat_tests.rs"]
mod flat_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "imported_translate_observed_tests.rs"]
mod observed_tests;
