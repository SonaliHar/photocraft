//! Imported plain-accounted `layer.setProps` adapter. This is the closed
//! name/visibility/opacity subset of the existing registered upstream command;
//! it grants neither native pixel editing nor source/render qualification.
use super::*;
use crate::accounted_session::assigned_fence::AssignedFence;
use crate::accounted_session::metadata_gesture::MetadataPolicy;
use crate::{
    accounted_view::AccountedView,
    channel_cmds::{ChannelTarget, ChannelView},
};
use photocraft_doc::accounted_document::accounted_layer_access;
#[derive(Clone, Copy)]
enum Target<'a> {
    Active,
    Id(LayerId),
    Name(&'a str),
}
#[derive(Clone, Copy)]
pub(super) struct Request<'a> {
    target: Target<'a>,
    name: Option<&'a str>,
    visible: Option<bool>,
    opacity: Option<f32>,
    coalesce: Option<&'a str>,
    only_visibility: bool,
}
fn invalid(message: &str) -> EngineError {
    bad("layer.setProps", message)
}
pub(super) fn applies(session: &Session) -> bool {
    session.active().is_some_and(|s| s.doc.accounted().is_some() && (s.doc.xcf_state.is_some() || s.doc.source_color_context.is_some()))
}
pub(super) fn parse(params: &Value) -> Result<Request<'_>> {
    let p = params.as_object().ok_or_else(|| invalid("imported layer properties require an object"))?;
    if p.len() > 5 {
        return Err(invalid("too many imported layer property fields"));
    }
    for key in p.keys() {
        if !matches!(key.as_str(), "layer" | "name" | "visible" | "opacity" | "coalesce") {
            return Err(EngineError::Other(
                "imported accounted properties support name, visible, and opacity only; other property edits are unsupported".into(),
            ));
        }
    }
    // Match the ordinary dispatcher's as_str(): absent, null and nonstrings
    // carry no key. Strings are opaque exact bytes, including empty/NUL keys.
    let coalesce = p.get("coalesce").and_then(Value::as_str);
    if coalesce.is_some_and(|key| key.len() > 128) {
        return Err(invalid("coalesce string exceeds 128 bytes"));
    }
    let target = match p.get("layer") {
        None | Some(Value::Null) => Target::Active,
        Some(Value::Number(v)) => Target::Id(LayerId(v.as_u64().ok_or_else(|| invalid("layer must be a non-negative integer identity or name"))?)),
        Some(Value::String(v)) if v.len() <= MAX_STRING => Target::Name(v),
        _ => return Err(invalid("layer must be a bounded name, integer identity, or null")),
    };
    // The original handler ignores recognized wrong-typed fields. Opacity uses
    // its exact cast-then-clamp order, including finite f64 that overflows f32.
    let name = p.get("name").and_then(Value::as_str);
    if name.is_some_and(|v| v.len() > MAX_STRING) {
        return Err(invalid("name must be bounded"));
    }
    let visible = p.get("visible").and_then(Value::as_bool);
    let opacity = p.get("opacity").and_then(Value::as_f64).map(|v| (v as f32).clamp(0.0, 1.0));
    let only_visibility = p.contains_key("visible") && p.keys().all(|k| matches!(k.as_str(), "layer" | "visible" | "coalesce"));
    Ok(Request { target, name, visible, opacity, coalesce, only_visibility })
}
fn channel_check(channel: &ChannelView, colors: usize) -> Result<()> {
    if channel.color_hidden.len() > 32 || channel.alpha_visible.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    if channel.layer_mask.is_some() || channel.editing_mask.is_some() || !matches!(channel.target, ChannelTarget::Composite | ChannelTarget::Color(_)) {
        return Err(EngineError::Other("imported accounted metadata requires a current layer color view, without a mask or saved-channel target".into()));
    }
    if matches!(channel.target,ChannelTarget::Color(index) if index>=colors) {
        return Err(BackingError::Bounds.into());
    }
    Ok(())
}
fn booleans(values: &[bool], control: &ReadControl<'_>) -> Result<Vec<bool>> {
    let mut result = Vec::new();
    result.try_reserve_exact(values.len()).map_err(|_| BackingError::Allocation)?;
    if result.capacity() > values.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for value in values {
        control.check(1)?;
        result.push(*value);
    }
    Ok(result)
}
// Owned channel containers precede their lease. The retained layer-view Arc is
// an actual paid owner, not a copied list or caller-asserted selection witness.
pub(super) struct Prepared<'a> {
    edit: AccountedEdit<'a>,
    coalesce: Option<&'a str>,
    view: AccountedView,
    active: Option<LayerId>,
    anchor: Option<LayerId>,
    channel: ChannelView,
    only_visibility: bool,
    assigned: Option<AssignedFence>,
    _lease: QuotaLease,
}
impl<'a> Request<'a> {
    pub(super) fn prepare(self, session: &Session, control: &ReadControl<'_>) -> Result<Prepared<'a>> {
        control.check(1)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        if state.doc.layers.len() > 4096 {
            return Err(BackingError::Length.into());
        }
        let id = match self.target {
            Target::Active => state.active_layer.ok_or(EngineError::NoDocument)?,
            Target::Id(id) => id,
            Target::Name(name) => {
                // The closed imported class is flat; reversed declaration order
                // is the same topmost-name policy as ordinary explicit_layer.
                let mut found = None;
                for layer in state.doc.layers.iter().rev() {
                    control.check((name.len() + 1) as u64)?;
                    if layer.name == name {
                        found = Some(layer.id);
                        break;
                    }
                }
                found.ok_or_else(|| EngineError::Other("no layer has the requested name".into()))?
            }
        };
        let edit = AccountedEdit::LayerMetadata { layer: id, name: self.name, visible: self.visible, opacity: self.opacity };
        owner.validate_imported_metadata_edit(edit, control)?;
        let policy = MetadataPolicy::for_owner(owner);
        // Current validation precedes copied channel/journal containers. The
        // eventual transaction repeats it under the same cumulative control.
        policy.validate_current(owner, control)?;
        let assigned = match policy {
            MetadataPolicy::Plain => None,
            MetadataPolicy::AssignedGeometry => Some(AssignedFence::prepare::<Prepared<'_>>(session, control)?),
        };
        check_locks(owner.document(), edit, control)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        channel_check(&state.channel_view, state.doc.mode.color_channels())?;
        let count = state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;
        let bytes = count.checked_mul(2).and_then(|n| n.checked_add(std::mem::size_of::<ChannelView>() + 128)).ok_or(BackingError::Length)?;
        let mut lease = owner.quota().reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
        let channel = ChannelView {
            target: state.channel_view.target,
            color_hidden: booleans(&state.channel_view.color_hidden, control)?,
            alpha_visible: booleans(&state.channel_view.alpha_visible, control)?,
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
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        control.check(1)?;
        Ok(Prepared {
            edit,
            coalesce: self.coalesce,
            view: view.clone(),
            active: state.active_layer,
            anchor: state.layer_anchor,
            channel,
            only_visibility: self.only_visibility,
            assigned,
            _lease: lease,
        })
    }
}
fn check_locks(doc: &photocraft_doc::Document, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<()> {
    let access = accounted_layer_access(doc, edit.layer_id(), control)?.ok_or(EngineError::NoLayer(edit.layer_id()))?;
    let AccountedEdit::LayerMetadata { visible, .. } = edit else { return Err(BackingError::Unsupported.into()) };
    // Content/Position/Alpha locks remain unchanged and are inspected through the
    // genuine access record. Existing name/opacity semantics do not prohibit
    // those properties under such locks. Only changed visibility uses its lock.
    if visible.is_some_and(|v| v != access.layer.visible) && access.imported_locks.visibility {
        return Err(EngineError::Other("the imported layer is visibility-locked".into()));
    }
    Ok(())
}
impl Prepared<'_> {
    pub(super) fn execute(self, session: &mut Session, control: &ReadControl<'_>) -> Result<Value> {
        control.check(1)?;
        if let Some(assigned) = &self.assigned {
            assigned.check(session, control)?;
        }
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Integrity)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if !view.shares_state(&self.view) || state.active_layer != self.active || state.layer_anchor != self.anchor || !state.selected_layers.is_empty() {
            return Err(EngineError::Other("the prepared metadata layer view changed".into()));
        }
        channel_check(&state.channel_view, state.doc.mode.color_channels())?;
        control.check((self.channel.color_hidden.len() + self.channel.alpha_visible.len() + 1) as u64)?;
        if state.channel_view != self.channel {
            return Err(EngineError::Other("the prepared metadata channel view changed".into()));
        }
        check_locks(owner.document(), self.edit, control)?;
        let result = Value::Null;
        control.check(1)?;
        session.edit_accounted_metadata_with_coalesce(
            if self.only_visibility { "Layer Visibility" } else { "Layer Properties" },
            self.edit,
            self.coalesce,
            control,
        )?;
        Ok(result)
    }
}
#[cfg(test)]
#[path = "imported_accounted_metadata_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "imported_accounted_metadata/assigned_tests.rs"]
mod assigned_tests;
