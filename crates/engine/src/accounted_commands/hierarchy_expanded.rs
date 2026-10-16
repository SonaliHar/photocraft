//! Actual hierarchy disclosure with upstream no-history/no-journal semantics.
use super::*;
use crate::{DocumentSnapshot,accounted_view::AccountedView,channel_cmds::ChannelView,document_history::DocumentHistory,source_policy::SourceIdentity};
use photocraft_doc::accounted_document::AccountedDocument;
pub(super) const REPLY_ENVELOPE:u64=8192;
pub(super) struct Request<'a>{_params:&'a Value,layer:Option<LayerId>,expanded:bool,all:bool,groups:usize}
fn recipe(owner:&AccountedDocument,c:&ReadControl<'_>)->Result<()>{crate::accounted_xcf_open::normal_hierarchy::validate_current_normal_hierarchy_storage(owner,c)}
impl<'a> Request<'a>{
 pub(super) fn parse(session:&Session,params:&'a Value,c:&ReadControl<'_>)->Result<Self>{
  c.check(64)?;idle(session)?;
  let state=session.active().ok_or(EngineError::NoDocument)?;
  let owner=state.doc.accounted().filter(|o|o.is_readonly_normal_hierarchy_storage()).ok_or(BackingError::Unsupported)?;
  let boolean=|key:&str|match params.get(key){None|Some(Value::Null)=>Ok(None),Some(v)=>v.as_bool().map(Some).ok_or_else(||bad("layer.setExpanded","boolean required"))};
  let want=boolean("expanded")?;let all=boolean("all")?.unwrap_or(false);
  let layer=match params.get("layer"){
   None|Some(Value::Null) if all=>None,
   None|Some(Value::Null)=>Some(state.active_layer.ok_or_else(||bad("layer.setExpanded","no active layer"))?),
   Some(v)=>Some(LayerId(v.as_u64().ok_or_else(||bad("layer.setExpanded","layer must be an integer identity"))?)),
  };
  let index=owner.checked_readonly_normal_hierarchy_index(c)?;
  let mut any=false;let mut selected=None;let mut found_layer=false;let mut groups=0usize;
  for row in index.rows(){c.check(1)?;found_layer|=Some(row.layer.id)==layer;if let photocraft_doc::LayerContent::Group(g)=&row.layer.content{
    groups=groups.checked_add(1).ok_or(BackingError::Length)?;any|=g.expanded;
    if Some(row.layer.id)==layer{selected=Some(g.expanded)}
  }}
  let current=if let Some(id)=layer{selected.ok_or_else(||if found_layer{bad("layer.setExpanded",&format!("layer {} is not a group",id.0))}else{EngineError::NoLayer(id)})?}else{any};
  Ok(Self{_params:params,layer,expanded:want.unwrap_or(!current),all,groups:if all{groups}else{1}})
 }
 pub(super) fn prepare(self,session:&Session,ctl:&ReadControl<'_>)->Result<Prepared<'a>>{
  idle(session)?;let state=session.active().ok_or(EngineError::NoDocument)?;
  if state.coalesce.is_some()||state.floating.is_some()||state.doc.xcf_state.as_ref().is_some_and(|s|s.floating.is_some()){return Err(BackingError::Unsupported.into())}
  let owner=state.doc.accounted().ok_or(BackingError::Unsupported)?;
  if !state.history.current()?.shares_state(&state.doc){return Err(BackingError::Integrity.into())}
  recipe(owner,ctl)?;
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

  let staged=owner.try_with_readonly_normal_hierarchy_group_expanded(self.layer,self.expanded,self.all,ctl)?;
  recipe(&staged,ctl)?;
  let updated=!staged.shares_model(owner);
  let mut history=state.history.clone();
  let snapshot=history.try_replace_accounted_current(owner,staged,ctl)?;
  let next_view=view.try_replace(snapshot.accounted().ok_or(BackingError::Integrity)?,state.active_layer,view.selected(),state.layer_anchor,ctl)?;
  let reply_lease=owner.quota().reserve(QuotaKind::Index,REPLY_ENVELOPE).map_err(|_|BackingError::Quota)?;
  ctl.check(REPLY_ENVELOPE)?;
  let reply=serde_json::json!({"expanded":self.expanded,"groups":self.groups});
  Ok(Prepared{_request:self,captured:state.doc.clone(),snapshot,history,old_history:state.history.clone(),view:view.clone(),next_view,revision:state.revision,view_revision:state.view_revision,active:state.active_layer,anchor:state.layer_anchor,channel,path,source,read_only:state.source_read_only,history_states:session.prefs().performance.history_states,history_bytes:session.prefs().performance.history_budget_bytes(),updated,reply,_reply_lease:reply_lease,_lease:lease})
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
    if session.active().is_some_and(|s|s.coalesce.is_some()||s.floating.is_some()||s.doc.xcf_state.as_ref().is_some_and(|x|x.floating.is_some()))
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
    {
        return Err(bad("layer.setExpanded", "hierarchy disclosure require an idle synchronous color context without coalescing"));
    }
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
            return Err(EngineError::Other("the prepared hierarchy disclosure target older Session state".into()));
        }
        let owner=state.doc.accounted().ok_or(BackingError::Integrity)?;
        if !owner.is_readonly_normal_hierarchy_storage(){return Err(BackingError::Unsupported.into())}
        owner.validate_readonly_normal_hierarchy_group_expanded(self._request.layer,self._request.expanded,self._request.all,ctl)?;
        ctl.check(1)?;
        if self.updated {
            let state = session.active_mut().ok_or(EngineError::NoDocument)?;
            state.doc = self.snapshot;
            state.history = self.history;
            state.accounted_view = Some(self.next_view);
            state.last_damage = Some(photocraft_geom::Rect::EMPTY);
            state.coalesce = None;
            state._accounted_metadata_gesture = None;
        }
        Ok(self.reply)
    }
}

/// Paid captured-context fence for the newly admitted hierarchy focus route.
/// Publication still calls the unchanged upstream selection implementation.
pub(super) struct SelectionWitness{
 captured:DocumentSnapshot,old_history:DocumentHistory,view:AccountedView,
 revision:u64,view_revision:u64,active:Option<LayerId>,anchor:Option<LayerId>,
 channel:ChannelView,path:Option<String>,source:Option<SourceIdentity>,read_only:bool,
 history_states:u32,history_bytes:usize,_lease:QuotaLease,
}
impl SelectionWitness{
 pub(super) fn prepare(session:&Session,layer:LayerId,ctl:&ReadControl<'_>)->Result<Self>{
  idle(session)?;let state=session.active().ok_or(EngineError::NoDocument)?;
  let owner=state.doc.accounted().filter(|o|o.is_readonly_normal_hierarchy_storage()).ok_or(BackingError::Unsupported)?;
  if !state.history.current()?.shares_state(&state.doc){return Err(BackingError::Integrity.into())}
  recipe(owner,ctl)?;
  owner.accounted_layer_access(layer,ctl)?.ok_or(EngineError::NoLayer(layer))?;
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
            .and_then(|v| v.checked_add(std::mem::size_of::<SelectionWitness>() + 512))
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
            .and_then(|v| v.checked_add(std::mem::size_of::<SelectionWitness>() + 512))
            .ok_or(BackingError::Length)?;
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;

  Ok(Self{captured:state.doc.clone(),old_history:state.history.clone(),view:view.clone(),revision:state.revision,view_revision:state.view_revision,active:state.active_layer,anchor:state.layer_anchor,channel,path,source,read_only:state.source_read_only,history_states:session.prefs().performance.history_states,history_bytes:session.prefs().performance.history_budget_bytes(),_lease:lease})
 }
 pub(super) fn validate(&self,session:&Session,ctl:&ReadControl<'_>)->Result<()>{
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
            return Err(EngineError::Other("the prepared hierarchy disclosure target older Session state".into()));
        }

  ctl.check(1)?;Ok(())
 }
}
