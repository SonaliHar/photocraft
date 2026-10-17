//! Bounded current-model name/visibility/opacity adapter for the existing layer.setProps command.
//! A runtime storage class and a fresh complete operative recipe are mandatory;
//! neither persisted operation names nor this request grant source authority.
use super::*;
use crate::{DocumentSnapshot, accounted_view::AccountedView, channel_cmds::ChannelView, document_history::DocumentHistory, source_policy::SourceIdentity};
use photocraft_doc::accounted_document::{AccountedDocument, ReadonlyLayerNamePatch, ReadonlyLayerPropertiesPatch, accounted_layer_access};

pub(super) const REPLY_ENVELOPE: u64 = 8192;
#[derive(Clone, Copy)]
enum Target<'a> {
    Active,
    Id(LayerId),
    Name(&'a str),
}
pub(super) struct Request<'a> {
    target: Target<'a>,
    name: Option<&'a str>,
    visible: Option<bool>,
    opacity: Option<u32>,
    extended: bool,
    only_visibility: bool,
}
pub(super) fn applies(session: &Session) -> bool {
    session.active().and_then(|s| s.doc.accounted()).is_some_and(|o| {
        o.is_readonly_threshold_alpha_storage()
            || o.is_readonly_threshold_storage()
            || (o.is_readonly_point_storage() && !crate::accounted_session::metadata_gesture::observed_box_layer_metadata_candidate(o))
    })
}
impl<'a> Request<'a> {
    pub(super) fn parse(params: &'a Value, ctl: &ReadControl<'_>) -> Result<Self> {
        ctl.check(1)?;
        let p = params.as_object().ok_or_else(|| bad("layer.setProps", "readonly layer properties require an object"))?;
        if p.len() > 4 || p.keys().any(|k| !matches!(k.as_str(), "layer" | "name" | "visible" | "opacity")) {
            return Err(bad("layer.setProps", "readonly filter owners support name, visible and opacity without coalescing"));
        }
        let extended = p.contains_key("visible") || p.contains_key("opacity");
        // Old name-only accepted/refused shapes remain exact. New property
        // shapes ignore recognized wrong types only if some typed field remains.
        let name = p.get("name").and_then(Value::as_str);
        if !extended && name.is_none() {
            return Err(bad("layer.setProps", "name must be a bounded string"));
        }
        if let Some(name) = name {
            if name.len() > MAX_STRING {
                return Err(bad("layer.setProps", "name must be a bounded string"));
            }
            ctl.check(name.len() as u64)?;
            if name.contains('\0') {
                return Err(bad("layer.setProps", "name must not contain NUL"));
            }
        }
        let visible = p.get("visible").and_then(Value::as_bool);
        // Match upstream's finite JSON f64 -> f32 THEN clamp order, including
        // finite f64 overflow and signed zero. Never normalize the resulting bits.
        let opacity = p.get("opacity").and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| (v as f32).clamp(0.0, 1.0).to_bits());
        if name.is_none() && visible.is_none() && opacity.is_none() {
            return Err(bad("layer.setProps", "at least one supported typed property is required"));
        }
        let only_visibility = p.contains_key("visible") && p.keys().all(|k| matches!(k.as_str(), "layer" | "visible"));
        let target = match p.get("layer") {
            None | Some(Value::Null) => Target::Active,
            Some(Value::Number(n)) => {
                Target::Id(LayerId(n.as_u64().ok_or_else(|| bad("layer.setProps", "layer must be a non-negative integer identity or name"))?))
            }
            Some(Value::String(n)) if n.len() <= MAX_STRING => Target::Name(n),
            _ => return Err(bad("layer.setProps", "layer must be a bounded name, integer identity, or null")),
        };
        Ok(Self { target, name, visible, opacity, extended, only_visibility })
    }
    pub(super) fn prepare(self, session: &Session, ctl: &ReadControl<'_>) -> Result<Prepared<'a>> {
        idle(session)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        if owner.is_readonly_threshold_storage() {
            owner.validate_readonly_threshold_storage(ctl)?;
        } else if owner.is_readonly_point_storage() {
            owner.validate_readonly_point_storage(ctl)?;
        } else {
            owner.validate_readonly_threshold_alpha_storage(ctl)?;
        }
        let layer = match self.target {
            Target::Active => state.active_layer.ok_or(EngineError::NoDocument)?,
            Target::Id(id) => id,
            Target::Name(name) => {
                let mut found = None;
                for layer in owner.document().layers.iter().rev() {
                    ctl.check(name.len().checked_add(1).ok_or(BackingError::Length)? as u64)?;
                    if layer.name == name {
                        found = Some(layer.id);
                        break;
                    }
                }
                found.ok_or_else(|| bad("layer.setProps", "no layer has the requested name"))?
            }
        };
        let visibility_changed =
            self.visible.is_some_and(|visible| owner.document().layers.first().is_some_and(|current| current.id == layer && visible != current.visible));
        target_access(owner, layer, visibility_changed, ctl)?;
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
        let staged = if self.extended {
            owner.try_with_readonly_layer_properties(
                ReadonlyLayerPropertiesPatch { layer, name: self.name, visible: self.visible, opacity_f32_bits: self.opacity },
                ctl,
            )?
        } else {
            owner.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer, name: self.name.ok_or(BackingError::Integrity)? }, ctl)?
        };
        validate_recipe(&staged, ctl)?;
        let updated = !staged.shares_model(owner);
        let mut history = state.history.clone();
        let snapshot = if updated {
            history.set_accounted_state_limit(session.prefs().performance.history_states.max(1) as usize)?;
            history.set_current_layers_borrowed(view.target(), ctl)?;
            history.try_commit_prepared_accounted(
                owner,
                staged,
                if self.only_visibility { "Layer Visibility" } else { "Layer Properties" },
                view.target(),
                ctl,
            )?
        } else {
            state.doc.clone()
        };
        let next_view = view.try_replace(snapshot.accounted().ok_or(BackingError::Integrity)?, state.active_layer, view.selected(), state.layer_anchor, ctl)?;
        let reply_lease = owner.quota().reserve(QuotaKind::Index, REPLY_ENVELOPE).map_err(|_| BackingError::Quota)?;
        ctl.check(REPLY_ENVELOPE)?;
        // Upstream layer.setProps returns null. The retained journal owns the
        // conservative synchronous response allowance after publication.
        let reply = Value::Null;
        ctl.check(1)?;
        Ok(Prepared {
            _request: self,
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
fn idle(session: &Session) -> Result<()> {
    if session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
    {
        return Err(bad("layer.setProps", "readonly layer properties require an idle synchronous color context without coalescing"));
    }
    Ok(())
}
// Upstream checks only ACTUAL Layer.visible transitions against imported
// visibility locks. Canonical-only reconciliation does not create that change;
// content/all/pixel/position/alpha locks do not prohibit names or opacity.
fn target_access(owner: &AccountedDocument, layer: LayerId, visibility_changed: bool, ctl: &ReadControl<'_>) -> Result<()> {
    let access = accounted_layer_access(owner.document(), layer, ctl)?.ok_or(EngineError::NoLayer(layer))?;
    if visibility_changed && access.imported_locks.visibility {
        return Err(bad("layer.setProps", "the imported layer is visibility-locked"));
    }
    Ok(())
}
fn validate_recipe(owner: &AccountedDocument, ctl: &ReadControl<'_>) -> Result<()> {
    if owner.is_readonly_threshold_storage() {
        crate::accounted_native_open::validate_current_threshold_storage(owner, ctl)
    } else if owner.is_readonly_point_storage() {
        // The supplied basis is nonlinear; the existing controlled context
        // independently reconstructs/binds the actual stored-encoding profile.
        crate::accounted_native_open::validate_current_stock_point_storage(owner, ctl)
    } else {
        validate_alpha_recipe(owner, ctl)
    }
}
fn validate_alpha_recipe(owner: &AccountedDocument, ctl: &ReadControl<'_>) -> Result<()> {
    ctl.check(1)?;
    let profile = match owner.document().mode {
        photocraft_doc::ColorMode::Rgb => photocraft_cms::Builtin::Srgb,
        photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray,
        _ => return Err(BackingError::Unsupported.into()),
    }
    .profile();
    ctl.check(1)?;
    let prepared = photocraft_compose::backed_threshold_alpha_context::PreparedBackedThresholdAlphaContext::new(owner, profile, Default::default(), ctl)
        .map_err(|e| match e {
            photocraft_compose::backed_normal::BackedNormalError::Backing(e) => EngineError::from(e),
            e => EngineError::Other(format!("current Threshold Alpha recipe: {e}")),
        })?;
    prepared.validate_operative(ctl).map_err(|e| match e {
        photocraft_compose::backed_point_filter::BackedPointError::Backing(e) => EngineError::from(e),
        e => EngineError::Other(format!("current Threshold Alpha band: {e}")),
    })?;
    ctl.check(1)?;
    Ok(())
}
pub(super) struct Prepared<'a> {
    _request: Request<'a>,
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
        ctl.check(1)?;
        idle(session)?;
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
            return Err(EngineError::Other("the prepared readonly layer properties target older Session state".into()));
        }
        target_access(state.doc.accounted().ok_or(BackingError::Integrity)?, self.layer, self.visibility_changed, ctl)?;
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

#[cfg(test)]
#[path = "readonly_layer_name_tests.rs"]
mod tests;
