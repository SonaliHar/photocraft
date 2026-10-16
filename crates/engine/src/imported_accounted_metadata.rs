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
    rename: bool,
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
    Ok(Request { target, name, visible, opacity, coalesce, only_visibility, rename:false })
}
/// Borrowed legacy rename witness, dispatched only for the actual mixed class.
pub(super) fn parse_rename<'a>(session:&Session,params:&'a Value,c:&ReadControl<'_>)->Result<Request<'a>>{
 let cmd="layer.renameLayer";
 let p=params.as_object().filter(|p|p.len()<=4&&p.keys().all(|k|matches!(k.as_str(),"layer"|"layerId"|"name"|"documentId"|"expectedRevision"))).ok_or_else(||bad(cmd,"unknown/too many rename fields"))?;
 let raw=p.get("name").and_then(Value::as_str).filter(|v|v.len()<=1<<20).ok_or_else(||bad(cmd,"bounded name required"))?;
 c.check(raw.len().checked_mul(3).ok_or(BackingError::Length)? as u64)?;
 if raw.contains('\0'){return Err(bad(cmd,"name must not contain NUL"))}
 let name=raw.trim();if name.is_empty(){return Err(bad(cmd,"nonempty name required"))}
 let state=session.active().ok_or(EngineError::NoDocument)?;
 if !state.doc.accounted().is_some_and(|o|o.is_readonly_filtered_flat_point_storage() || (o.is_readonly_masked_filtered_flat_point_storage() || o.is_readonly_normal_hierarchy_storage()))||session.command_is_background()||session.command_has_pending_job()||session.has_jobs()||session.coalesce_request.is_some()||session.color_restrict.is_some()||state.coalesce.is_some()||state.floating.is_some()||state.doc.xcf_state.as_ref().is_some_and(|x|x.floating.is_some())||state.revision==u64::MAX{return Err(bad(cmd,"synchronous idle uncoalesced rename required"))}
 let decimal=|v:&Value,zero:bool|->Result<u64>{let text=v.as_str().filter(|v|!v.is_empty()&&v.len()<=20&&v.bytes().all(|b|b.is_ascii_digit())&&!(v.len()>1&&v.starts_with('0'))).ok_or_else(||bad(cmd,"canonical captured identity required"))?;let id=text.parse().map_err(|_|bad(cmd,"identity overflow"))?;if id==0&&!zero{return Err(bad(cmd,"zero identity"))}Ok(id)};
 match(p.get("documentId"),p.get("expectedRevision")){(None,None)=>{},(Some(doc),Some(rev))=>{if decimal(doc,false)?!=state.doc.id.0||decimal(rev,true)?!=state.revision{return Err(bad(cmd,"stale captured target"))}},_=>return Err(bad(cmd,"paired captured identity/revision required"))}
 let target=match(p.get("layer"),p.get("layerId")){(Some(_),Some(_))=>return Err(bad(cmd,"one layer selector only")),(Some(v),None)=>Target::Id(LayerId(v.as_u64().ok_or_else(||bad(cmd,"integer layer required"))?)),(None,Some(v))=>Target::Id(LayerId(decimal(v,false)?)),(None,None)=>Target::Active};
 Ok(Request{target,name:Some(name),visible:None,opacity:None,coalesce:None,only_visibility:false,rename:true})
}
pub(super) fn channel_check(channel: &ChannelView, colors: usize) -> Result<()> {
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
pub(super) fn booleans(values: &[bool], control: &ReadControl<'_>) -> Result<Vec<bool>> {
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
pub(super) fn same_name(left: &str, right: &str, c: &ReadControl<'_>) -> Result<bool> {
    c.check(1)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (a, b) in left.as_bytes().chunks(32768).zip(right.as_bytes().chunks(32768)) {
        c.check(a.len().checked_add(b.len()).ok_or(BackingError::Length)? as u64)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
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
    rename: bool,
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
        let policy = MetadataPolicy::for_owner(owner);
        if crate::accounted_session::metadata_gesture::point_metadata_class(owner) && !matches!(policy,MetadataPolicy::ObservedBox) { return Err(BackingError::Unsupported.into()); }
        if matches!(policy,MetadataPolicy::FlatComposite | MetadataPolicy::ObservedBox | MetadataPolicy::FilteredFlat | MetadataPolicy::MaskedFilteredFlat | MetadataPolicy::NormalHierarchy) {policy.validate_storage(owner,control)?;}
        let id = match self.target {
            Target::Active => state.active_layer.ok_or(EngineError::NoDocument)?,
            Target::Id(id) => id,
            Target::Name(name) => {
                let mut found = None;
                if owner.is_readonly_normal_hierarchy_storage(){
                    let index=owner.checked_readonly_normal_hierarchy_current_index(control)?;
                    for row in index.rows().iter().rev(){if same_name(&row.layer.name,name,control)?{found=Some(row.layer.id);break;}}
                } else if matches!(MetadataPolicy::for_owner(owner), MetadataPolicy::IsolatedNormalGroup | MetadataPolicy::GaussianGroup) {
                    if owner.is_readonly_gaussian_group_storage() { owner.validate_gaussian_group_storage(control)?; }
                    // Reverse depth-first order is upper child, lower child,
                    // group, backdrop. The hint bounds this traversal to four
                    // borrowed layers; full storage validation follows below.
                    for (index, layer) in state.doc.layers.iter().enumerate().rev() {
                        if index == 1
                            && let photocraft_doc::LayerContent::Group(group) = &layer.content
                        {
                            for child in group.children.iter().rev() {
                                if same_name(&child.name, name, control)? {
                                    found = Some(child.id);
                                    break;
                                }
                            }
                        }
                        if found.is_some() {
                            break;
                        }
                        if same_name(&layer.name, name, control)? {
                            found = Some(layer.id);
                            break;
                        }
                    }
                } else {
                    // Preserve the original flat topmost-name policy.
                    for layer in state.doc.layers.iter().rev() {
                        control.check((name.len() + 1) as u64)?;
                        if if matches!(policy,MetadataPolicy::FlatComposite | MetadataPolicy::ObservedBox | MetadataPolicy::FilteredFlat | MetadataPolicy::MaskedFilteredFlat | MetadataPolicy::NormalHierarchy) {same_name(&layer.name,name,control)?} else {layer.name == name} {
                            found = Some(layer.id);
                            break;
                        }
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
            MetadataPolicy::ObservedBox | MetadataPolicy::FlatComposite | MetadataPolicy::FilteredFlat | MetadataPolicy::MaskedFilteredFlat | MetadataPolicy::NormalHierarchy | MetadataPolicy::AssignedGeometry | MetadataPolicy::IsolatedNormalGroup | MetadataPolicy::GaussianGroup => Some(AssignedFence::prepare::<Prepared<'_>>(session, control)?),
        };
        check_locks(owner, edit, self.rename, control)?;
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
            rename:self.rename,
            assigned,
            _lease: lease,
        })
    }
}
fn check_locks(owner: &photocraft_doc::accounted_document::AccountedDocument, edit: AccountedEdit<'_>, rename:bool, control: &ReadControl<'_>) -> Result<()> {
    let access = if owner.is_readonly_filtered_flat_point_storage() || (owner.is_readonly_masked_filtered_flat_point_storage() || owner.is_readonly_normal_hierarchy_storage()) {owner.accounted_layer_access(edit.layer_id(),control)?} else {accounted_layer_access(owner.document(), edit.layer_id(), control)?}.ok_or(EngineError::NoLayer(edit.layer_id()))?;
    if rename && (access.locks.all || access.locks.pixels || access.imported_locks.content) {return Err(EngineError::Other("the imported layer is content-locked".into()));}
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
        check_locks(owner, self.edit, self.rename, control)?;
        let result = Value::Null;
        control.check(1)?;
        session.edit_accounted_metadata_with_coalesce(
            if self.rename { "Rename Layer" } else if self.only_visibility { "Layer Visibility" } else { "Layer Properties" },
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
