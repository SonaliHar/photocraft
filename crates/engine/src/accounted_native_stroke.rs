//! One bounded native Round stroke over an actual imported plain owner.
//! A source/operation name is never pixel-edit authority.
use crate::{DocumentSnapshot,EngineError,Result,Session,accounted_view::AccountedView,channel_cmds::ChannelView,document_history::DocumentHistory,source_policy::SourceIdentity};
use photocraft_doc::{LayerId,LayerContent,accounted_document::{AccountedDocument,AccountedEdit,accounted_layer_access}};
use photocraft_raster::backing::{BackingError,ReadControl,quota::{QuotaKind,QuotaLease}};
use photocraft_paint::native_stroke_controlled::{NativeStrokeError,NativeStrokeLimits,NativeStrokePatch,prepare_clipped_native_stroke};
use serde_json::Value;
const REPLY_ENVELOPE:u64=8192;
fn fail(id:&str,m:&str)->EngineError{EngineError::BadParams{cmd:id.into(),msg:m.into()}}
fn native(e:NativeStrokeError)->EngineError{match e{NativeStrokeError::Backing(e)=>e.into(),NativeStrokeError::View(photocraft_raster::checked_view::RasterViewError::Backed(e)|photocraft_raster::checked_view::RasterViewError::Control(e)|photocraft_raster::checked_view::RasterViewError::Descriptor(e))=>e.into(),NativeStrokeError::Stroke(photocraft_paint::StrokeError::Cancelled)=>EngineError::Cancelled,e=>EngineError::Other(e.to_string())}}
fn recipe_color(doc:&photocraft_doc::Document,brush:&mut photocraft_paint::BrushSettings,q:&photocraft_raster::backing::quota::BackingQuota,c:&ReadControl<'_>)->Result<()>{
 use photocraft_cms::{Builtin,Intent,Transform,control::Limits};
 c.check(1)?;if brush.erase{return Ok(())}
 let rgba=brush.color.map(f64::from);if rgba.iter().any(|v|!v.is_finite())||!(0.0..=1.0).contains(&rgba[3]){return Err(fail("paint.stroke","finite color and unit alpha required"))}
 let limits=Limits::default();let req=crate::color_cmds::document_profile_requirements_with_limits(doc,limits,c.interrupt)?;
 let source=Builtin::Srgb.profile();let cancelled=||c.interrupt.cancelled();let src=source.clone_requirements_with_control(limits,&cancelled).map_err(|e|EngineError::Other(e.to_string()))?;
 let reserve=req.result_retained_bytes_upper_bound.checked_mul(16).and_then(|n|n.checked_add(src.retained_bytes.checked_mul(16)?)).and_then(|n|n.checked_add(8<<20)).ok_or(BackingError::Length)?;
 let peak=req.peak_resident_bytes.checked_add(reserve).ok_or(BackingError::Length)?;if peak>limits.resident_bytes{return Err(BackingError::Quota.into())}
 let work=(reserve as u64).checked_mul(32).and_then(|n|n.checked_add(16<<20)).ok_or(BackingError::Length)?;c.check(req.work_units.checked_mul(3).and_then(|n|n.checked_add(work)).ok_or(BackingError::Length)?)?;
 let _fee=q.reserve(QuotaKind::Resident,peak as u64).map_err(|_|BackingError::Quota)?;
 let profile=crate::color_cmds::try_document_profile_with_limits(doc,limits,c.interrupt)?;
 if profile.color_space!=crate::color_cmds::mode_space(doc.mode){return Err(fail("paint.stroke","current brush profile/model mismatch"))}
 let exact=Transform::requirements_with_control(source,&profile,photocraft_cms::transform::TransformOptions{intent:Intent::RelativeColorimetric,bpc:true,..Default::default()},Limits{resident_bytes:reserve,work_units:work/3},&cancelled).map_err(|e|if c.interrupt.cancelled(){EngineError::Cancelled}else{EngineError::Other(e.to_string())})?;
 if exact.peak_resident_bytes>reserve||exact.work_units.checked_mul(3).is_none_or(|n|n>work){return Err(BackingError::Quota.into())}
 let transform=Transform::new_with_control(source,&profile,Intent::RelativeColorimetric,true,Limits{resident_bytes:reserve,work_units:work/3},&cancelled).map_err(|e|if c.interrupt.cancelled(){EngineError::Cancelled}else{EngineError::Other(e.to_string())})?;
 let mut out=[0.0f64;3];let count=doc.mode.color_channels();c.check(128)?;transform.try_eval_f64(&rgba[..3],out.get_mut(..count).ok_or(BackingError::Length)?).map_err(|e|EngineError::Other(e.to_string()))?;c.check(1)?;
 let rgb=if count==1{[out[0];3]}else{out};let rgb=rgb.map(|v|v as f32);if rgb.iter().any(|v|!v.is_finite()){return Err(fail("paint.stroke","converted brush color is not finite F32"))}brush.color=[rgb[0],rgb[1],rgb[2],brush.color[3]];Ok(())
}
fn idle(s:&Session,id:&str)->Result<()>{let st=s.active().ok_or(EngineError::NoDocument)?;if s.has_jobs()||s.command_has_pending_job()||s.command_is_background()||s.coalesce_request.is_some()||s.color_restrict.is_some()||st.floating.is_some()||st.coalesce.is_some()||st.symmetry.is_some(){return Err(fail(id,"native stroke requires an idle synchronous pixel context"))}if st.channel_view.target!=crate::channel_cmds::ChannelTarget::Composite||st.channel_view.layer_mask.is_some()||st.channel_view.editing_mask.is_some()||st.channel_view.color_hidden.iter().any(|v|*v)||st.channel_view.alpha_visible.iter().any(|v|*v)||st.channel_view.color_hidden.len()>32||st.channel_view.alpha_visible.len()>4096{return Err(fail(id,"native stroke requires all ordinary color channels"))}Ok(())}
fn copy(v:&str)->Result<String>{let mut s=String::new();s.try_reserve_exact(v.len()).map_err(|_|BackingError::Allocation)?;if s.capacity()>v.len().checked_mul(2).ok_or(BackingError::Length)?{return Err(BackingError::Allocation.into())}s.push_str(v);Ok(s)}
fn flags(v:&[bool],c:&ReadControl<'_>)->Result<Vec<bool>>{let mut x=Vec::new();x.try_reserve_exact(v.len()).map_err(|_|BackingError::Allocation)?;if x.capacity()>v.len().checked_mul(2).ok_or(BackingError::Length)?{return Err(BackingError::Allocation.into())}for b in v{c.check(1)?;x.push(*b)}Ok(x)}
fn locks(owner:&AccountedDocument,layer:LayerId,erase:bool,c:&ReadControl<'_>)->Result<bool>{let a=accounted_layer_access(owner.document(),layer,c)?.ok_or(EngineError::NoLayer(layer))?;if a.locks.all||a.locks.pixels||a.imported_locks.content||erase&&owner.document().is_background(layer){return Err(fail("paint.stroke","native brush target is locked or an unsupported Background erase"))}Ok(a.locks.transparency||a.imported_locks.alpha)}
pub(crate) struct Request<'a>{id:&'static str,params:&'a Value}
impl<'a> Request<'a>{
 pub(crate) fn parse(s:&Session,id:&str,p:&'a Value,c:&ReadControl<'_>)->Result<Self>{c.check(1)?;idle(s,id)?;if !p.is_object(){return Err(fail(id,"stroke parameters must be an object"))}let id=match id{"paint.stroke"=>"paint.stroke","paint.pencil"=>"paint.pencil",_=>return Err(BackingError::Unsupported.into())};s.active().and_then(|st|st.doc.accounted()).filter(|o|o.is_legacy_storage()).ok_or(BackingError::Unsupported)?.validate_selection_replacement(c)?;Ok(Self{id,params:p})}
 pub(crate) fn prepare(self,s:&Session,c:&ReadControl<'_>)->Result<Prepared<'a>>{
  idle(s,self.id)?;let state=s.active().ok_or(EngineError::NoDocument)?;let owner=state.doc.accounted().ok_or(BackingError::Unsupported)?;owner.validate_selection_replacement(c)?;
  if owner.document().selection.is_some()||owner.document().selection_channel_id.is_some(){return Err(fail(self.id,"native brush selection is not yet supported"))}
  if !state.history.current()?.shares_state(&state.doc){return Err(BackingError::Integrity.into())}
  c.check(64<<20)?;crate::brush_cmds::stroke_dispatch_preflight(s,self.id,self.params,false)?;
  let layer=match self.params.get("layer"){Some(v)=>LayerId(v.as_u64().filter(|v|*v!=0).ok_or_else(||fail(self.id,"nonzero layer identity required"))?),None=>state.active_layer.ok_or(EngineError::NoDocument)?};
  if self.params.get("target").is_some_and(|v|v.as_str()!=Some("pixels")){return Err(fail(self.id,"native brush requires pixel target"))}
  let access=accounted_layer_access(owner.document(),layer,c)?.ok_or(EngineError::NoLayer(layer))?;let LayerContent::BackedRaster(backed)=&access.layer.content else{return Err(BackingError::Unsupported.into())};
  let mut recipe=crate::brush_cmds::accounted_round_recipe(s,self.id,self.params,owner.quota(),c)?;let alpha_locked=locks(owner,layer,recipe.brush.erase,c)?;recipe_color(owner.document(),&mut recipe.brush,owner.quota(),c)?;
  let view=state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;if state.active_layer!=view.target().active||state.layer_anchor!=view.anchor()||!state.selected_layers.is_empty(){return Err(BackingError::Integrity.into())}
  let strings=state.path.as_ref().map_or(0,String::len).checked_add(state.source.as_ref().map_or(0,|x|x.label.len()+x.path.as_ref().map_or(0,String::len))).ok_or(BackingError::Length)?;if strings>12288{return Err(BackingError::Length.into())}
  let count=state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;let bytes=strings.checked_add(count).and_then(|n|n.checked_mul(2)).and_then(|n|n.checked_add(std::mem::size_of::<Prepared<'_>>()+1024)).ok_or(BackingError::Length)?;
  let mut lease=owner.quota().reserve(QuotaKind::Index,bytes as u64).map_err(|_|BackingError::Quota)?;c.check(strings.checked_add(count).ok_or(BackingError::Length)?as u64)?;
  let path=state.path.as_deref().map(copy).transpose()?;let source=state.source.as_ref().map(|v|Ok::<_,EngineError>(SourceIdentity{label:copy(&v.label)?,path:v.path.as_deref().map(copy).transpose()?,origin:v.origin})).transpose()?;
  let channel=ChannelView{target:state.channel_view.target,color_hidden:flags(&state.channel_view.color_hidden,c)?,alpha_visible:flags(&state.channel_view.alpha_visible,c)?,quick_mask_hidden:state.channel_view.quick_mask_hidden,layer_mask:None,editing_mask:None};
  let actual=path.as_ref().map_or(0,String::capacity).checked_add(source.as_ref().map_or(0,|v|v.label.capacity()+v.path.as_ref().map_or(0,String::capacity))).and_then(|n|n.checked_add(channel.color_hidden.capacity()+channel.alpha_visible.capacity())).and_then(|n|n.checked_add(std::mem::size_of::<Prepared<'_>>()+1024)).ok_or(BackingError::Length)?;lease.reconcile_capacity(actual as u64).map_err(|_|BackingError::Allocation)?;
  let zoom=self.params.get("zoom").and_then(Value::as_f64).unwrap_or(1.0)as f32;
  let plan=prepare_clipped_native_stroke(backed.checked_view(),backed.operative_bounds(),&recipe.brush,&recipe.points,None,alpha_locked,zoom,NativeStrokeLimits::STANDARD,owner.quota(),c).map_err(native)?;let patch=plan.render(c).map_err(native)?;
  let reply_lease=owner.quota().reserve(QuotaKind::Index,REPLY_ENVELOPE).map_err(|_|BackingError::Quota)?;c.check(REPLY_ENVELOPE)?;
  let changed=patch.changed();let mut history=state.history.clone();let snapshot=if changed{
   owner.validate_imported_native_edit(AccountedEdit::NativePixels{layer,rect:patch.bounds(),bytes:patch.bytes()},c)?;
   let tile_bytes=backed.format().bytes_per_pixel().checked_mul(256*256).ok_or(BackingError::Length)?;let tiles=patch.bounds().tiles().count();c.check((tiles as u64).checked_mul(tile_bytes as u64).and_then(|n|n.checked_mul(4)).and_then(|n|n.checked_add(4096)).ok_or(BackingError::Length)?)?;
   let staged=owner.try_edit(AccountedEdit::NativePixels{layer,rect:patch.bounds(),bytes:patch.bytes()},c)?;
   if alpha_locked{crate::accounted_session::verify_native_stroke_alpha(owner,layer,patch.bounds(),patch.bytes(),c)?;}
   history.max_states=s.prefs().performance.history_states.max(1)as usize;history.max_bytes=s.prefs().performance.history_budget_bytes();history.set_current_layers_borrowed(view.target(),c)?;history.try_commit_prepared_accounted(owner,staged,if recipe.brush.erase&&self.params.get("mode").and_then(Value::as_str)!=Some("clear"){"Eraser"}else if self.id=="paint.pencil"{"Pencil"}else{"Brush Tool"},view.target(),c)?
  }else{state.doc.clone()};
  let next_view=view.try_replace(snapshot.accounted().ok_or(BackingError::Integrity)?,state.active_layer,view.selected(),state.layer_anchor,c)?;
  let bounds=patch.bounds();let reply=serde_json::json!({"damage":[bounds.x0,bounds.y0,bounds.width(),bounds.height()]});
  Ok(Prepared{request:self,captured:state.doc.clone(),snapshot,old_history:state.history.clone(),history,view:view.clone(),next_view,revision:state.revision,view_revision:state.view_revision,active:state.active_layer,anchor:state.layer_anchor,channel,path,source,read_only:state.source_read_only,history_states:s.prefs().performance.history_states,history_bytes:s.prefs().performance.history_budget_bytes(),layer,alpha_locked,changed,recipe,patch,reply,_reply_fee:reply_lease,_fee:lease})
 }
}
pub(crate) struct Prepared<'a>{request:Request<'a>,captured:DocumentSnapshot,snapshot:DocumentSnapshot,old_history:DocumentHistory,history:DocumentHistory,view:AccountedView,next_view:AccountedView,revision:u64,view_revision:u64,active:Option<LayerId>,anchor:Option<LayerId>,channel:ChannelView,path:Option<String>,source:Option<SourceIdentity>,read_only:bool,history_states:u32,history_bytes:usize,layer:LayerId,alpha_locked:bool,changed:bool,recipe:crate::brush_cmds::AccountedRoundRecipe,patch:NativeStrokePatch,reply:Value,_reply_fee:QuotaLease,_fee:QuotaLease}
impl Prepared<'_>{pub(crate) fn execute(self,s:&mut Session,c:&ReadControl<'_>)->Result<Value>{
 c.check(1)?;idle(s,self.request.id)?;let state=s.active().ok_or(EngineError::NoDocument)?;let owner=state.doc.accounted().ok_or(BackingError::Unsupported)?;let view=state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
 c.check((self.channel.color_hidden.len()+self.channel.alpha_visible.len())as u64)?;for text in self.path.iter().chain(self.source.iter().flat_map(|v|std::iter::once(&v.label).chain(v.path.iter()))){c.check(text.len()as u64)?}
 if !state.doc.shares_state(&self.captured)||state.revision!=self.revision||state.view_revision!=self.view_revision||!state.history.shares_accounted_history(&self.old_history)||state.history.max_states!=self.old_history.max_states||state.history.max_bytes!=self.old_history.max_bytes||!view.shares_state(&self.view)||state.active_layer!=self.active||state.layer_anchor!=self.anchor||!state.selected_layers.is_empty()||state.channel_view!=self.channel||state.path!=self.path||state.source!=self.source||state.source_read_only!=self.read_only||s.prefs().performance.history_states!=self.history_states||s.prefs().performance.history_budget_bytes()!=self.history_bytes||!self.recipe.tools_match(s){return Err(fail(self.request.id,"native stroke targets an older document/view/source/tool state"))}
 if locks(owner,self.layer,self.recipe.brush.erase,c)?!=self.alpha_locked{return Err(BackingError::Integrity.into())}owner.validate_selection_replacement(c)?;if self.changed{owner.validate_imported_native_edit(AccountedEdit::NativePixels{layer:self.layer,rect:self.patch.bounds(),bytes:self.patch.bytes()},c)?;}
 let revision=state.revision.checked_add(u64::from(self.changed)).ok_or(BackingError::Length)?;c.check(1)?;
 if self.changed{let state=s.active_mut().ok_or(EngineError::NoDocument)?;state.doc=self.snapshot;state.history=self.history;state.accounted_view=Some(self.next_view);state.revision=revision;state.last_damage=Some(self.patch.bounds());state.coalesce=None;state._accounted_metadata_gesture=None;}
 Ok(self.reply)
}}
pub(crate) const fn reply_envelope()->u64{REPLY_ENVELOPE}


/// Opaque press-time target for the native one-shot lane. It owns paid metadata
/// and shared immutable model/view handles, never a resident pixel preview.
pub(crate) struct NativeStrokeWork{work:photocraft_io::accounted_operation_work::OperationWork,_fee:QuotaLease}
impl std::ops::Deref for NativeStrokeWork{type Target=photocraft_io::accounted_operation_work::OperationWork;fn deref(&self)->&Self::Target{&self.work}}
pub struct AccountedNativeStrokeTarget{
 snapshot:DocumentSnapshot,revision:u64,view_revision:u64,layer:LayerId,
 view:AccountedView,history:DocumentHistory,anchor:Option<LayerId>,channel:ChannelView,
 path:Option<String>,source:Option<SourceIdentity>,read_only:bool,work:std::sync::Arc<NativeStrokeWork>,point_admission:std::sync::Mutex<(usize,QuotaLease)>,aborted:std::sync::atomic::AtomicBool,history_states:u32,history_bytes:usize,brush:photocraft_paint::BrushSettings,foreground:[u32;4],background:[u32;4],_fee:QuotaLease,
}
fn bounded<R>(work:&photocraft_io::accounted_operation_work::OperationWork,run:impl FnOnce(&ReadControl<'_>)->Result<R>)->Result<R>{
 let cancelled=||work.deadline_expired();let interrupt=photocraft_raster::Interrupt::cancel_only(&cancelled);let charge=|n|work.check(n);run(&ReadControl{interrupt:&interrupt,work:&charge})
}
pub fn capture_accounted_native_stroke_target(s:&Session)->Result<Option<std::sync::Arc<AccountedNativeStrokeTarget>>>{
 let Some(owner)=s.active().and_then(|state|state.doc.accounted()).filter(|owner|owner.is_legacy_storage())else{return Ok(None)};
 let meter_fee=owner.quota().reserve(QuotaKind::Index,(std::mem::size_of::<NativeStrokeWork>()+128)as u64).map_err(|_|BackingError::Quota)?;
 let limits=photocraft_xcf::ResourceLimits::for_policy(Default::default()).map_err(|e|EngineError::Other(e.to_string()))?;
 let work=std::sync::Arc::new(NativeStrokeWork{work:photocraft_io::accounted_operation_work::OperationWork::standard(&limits)?,_fee:meter_fee});
 bounded(&work,|c|{
  let Some(state)=s.active()else{return Ok(None)};let Some(owner)=state.doc.accounted().filter(|o|o.is_legacy_storage())else{return Ok(None)};
  idle(s,"paint.stroke")?;owner.validate_selection_replacement(c)?;if owner.document().selection.is_some()||owner.document().selection_channel_id.is_some(){return Err(fail("paint.stroke","native brush selection is not yet supported"))}
  if !state.history.current()?.shares_state(&state.doc){return Err(BackingError::Integrity.into())}
  let layer=state.active_layer.ok_or(EngineError::NoDocument)?;let access=accounted_layer_access(owner.document(),layer,c)?.ok_or(EngineError::NoLayer(layer))?;
  let LayerContent::BackedRaster(backed)=&access.layer.content else{return Err(BackingError::Unsupported.into())};
  if !matches!(backed.format().mode,photocraft_color::ColorMode::Rgb|photocraft_color::ColorMode::Grayscale)||!matches!(backed.format().sample,photocraft_color::SampleType::U8|photocraft_color::SampleType::U16|photocraft_color::SampleType::F32){return Err(fail("paint.stroke","native Round brush format is not yet supported"))}
  let brush=photocraft_paint::native_stroke_controlled::checked_round_brush(&s.tools.brush).map_err(native)?;
  let view=state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;if state.active_layer!=view.target().active||state.layer_anchor!=view.anchor()||!state.selected_layers.is_empty(){return Err(BackingError::Integrity.into())}
  let strings=state.path.as_ref().map_or(0,String::len).checked_add(state.source.as_ref().map_or(0,|v|v.label.len()+v.path.as_ref().map_or(0,String::len))).ok_or(BackingError::Length)?;if strings>12288{return Err(BackingError::Length.into())}
  let count=state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;let planned=strings.checked_add(count).and_then(|n|n.checked_mul(2)).and_then(|n|n.checked_add(std::mem::size_of::<AccountedNativeStrokeTarget>()+512)).ok_or(BackingError::Length)?;
  let mut fee=owner.quota().reserve(QuotaKind::Index,planned as u64).map_err(|_|BackingError::Quota)?;c.check(strings.checked_add(count).ok_or(BackingError::Length)?as u64)?;
  let path=state.path.as_deref().map(copy).transpose()?;let source=state.source.as_ref().map(|v|Ok::<_,EngineError>(SourceIdentity{label:copy(&v.label)?,path:v.path.as_deref().map(copy).transpose()?,origin:v.origin})).transpose()?;
  let channel=ChannelView{target:state.channel_view.target,color_hidden:flags(&state.channel_view.color_hidden,c)?,alpha_visible:flags(&state.channel_view.alpha_visible,c)?,quick_mask_hidden:state.channel_view.quick_mask_hidden,layer_mask:None,editing_mask:None};
  let actual=path.as_ref().map_or(0,String::capacity).checked_add(source.as_ref().map_or(0,|v|v.label.capacity()+v.path.as_ref().map_or(0,String::capacity))).and_then(|n|n.checked_add(channel.color_hidden.capacity()+channel.alpha_visible.capacity())).and_then(|n|n.checked_add(std::mem::size_of::<AccountedNativeStrokeTarget>()+512)).ok_or(BackingError::Length)?;fee.reconcile_capacity(actual as u64).map_err(|_|BackingError::Allocation)?;
  let point_fee=owner.quota().reserve(QuotaKind::Index,0).map_err(|_|BackingError::Quota)?;
  Ok(Some(std::sync::Arc::new(AccountedNativeStrokeTarget{snapshot:state.doc.clone(),revision:state.revision,view_revision:state.view_revision,layer,view:view.clone(),history:state.history.clone(),anchor:state.layer_anchor,channel,path,source,read_only:state.source_read_only,work:work.clone(),point_admission:std::sync::Mutex::new((0,point_fee)),aborted:std::sync::atomic::AtomicBool::new(false),history_states:s.prefs().performance.history_states,history_bytes:s.prefs().performance.history_budget_bytes(),brush,foreground:s.tools.foreground.map(f32::to_bits),background:s.tools.background.map(f32::to_bits),_fee:fee})))
 })
}
impl AccountedNativeStrokeTarget{
 /// Prospectively admit UI point/stylus growth and release-time transport
 /// overlap on the same gesture ledger, before frontend allocation or fill.
 pub fn admit_samples(&self,count:usize)->Result<()>{
  let result=(||{if self.aborted.load(std::sync::atomic::Ordering::Relaxed){return Err(BackingError::Cancelled.into())}
   if count>100_000{return Err(BackingError::Length.into())}self.work.check(0)?;
   let mut admission=self.point_admission.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
   if count>admission.0{let bytes=count.checked_mul(2048).and_then(|n|n.checked_add(4096)).ok_or(BackingError::Length)?;self.work.check(((count-admission.0)as u64).checked_mul(2048).ok_or(BackingError::Length)?)?;admission.1.try_resize(bytes as u64).map_err(|_|BackingError::Quota)?;admission.0=count;}Ok(())
  })();if result.is_err(){self.aborted.store(true,std::sync::atomic::Ordering::Relaxed);}result
 }
 /// Pay repeated point traversal independently of capacity growth.
 pub fn visit_samples(&self,count:usize)->Result<()>{self.admit_samples(0)?;if count>100_000{return Err(BackingError::Length.into())}self.work.check((count as u64).checked_mul(64).ok_or(BackingError::Length)?)?;Ok(())}
 /// Arm only the next registered paint dispatch with the original gesture meter.
 /// This supplies no target or edit authority; that dispatch repeats every proof.
 pub fn arm_captured_work(&self,s:&mut Session)->Result<()>{
  self.admit_samples(0)?;if s.native_stroke_work.is_some(){return Err(BackingError::Integrity.into())}s.native_stroke_work=Some(self.work.clone());Ok(())
 }
 /// Clear an arm when a frontend interceptor refused before engine dispatch.
 pub fn clear_captured_work(&self,s:&mut Session){if s.native_stroke_work.as_ref().is_some_and(|w|std::sync::Arc::ptr_eq(w,&self.work)){s.native_stroke_work=None;}}

 pub fn inject_captured_params(&self,s:&Session,p:&mut Value)->Result<()>{bounded(&self.work,|c|{
  self.admit_samples(0)?;idle(s,"paint.stroke")?;let state=s.active().ok_or(EngineError::NoDocument)?;let owner=state.doc.accounted().filter(|o|o.is_legacy_storage()).ok_or(BackingError::Unsupported)?;
  let current=state.path.as_ref().map_or(0,String::len).checked_add(state.source.as_ref().map_or(0,|v|v.label.len()+v.path.as_ref().map_or(0,String::len))).ok_or(BackingError::Length)?;if current>12288{return Err(BackingError::Length.into())}c.check(current as u64+self.channel.color_hidden.len()as u64+self.channel.alpha_visible.len()as u64)?;
  if !state.doc.shares_state(&self.snapshot)||state.revision!=self.revision||state.view_revision!=self.view_revision||state.active_layer!=Some(self.layer)||state.layer_anchor!=self.anchor||!state.accounted_view.as_ref().is_some_and(|v|v.shares_state(&self.view))||!state.history.shares_accounted_history(&self.history)||state.channel_view!=self.channel||state.path!=self.path||state.source!=self.source||state.source_read_only!=self.read_only||s.prefs().performance.history_states!=self.history_states||s.prefs().performance.history_budget_bytes()!=self.history_bytes||state.history.max_states!=self.history.max_states||state.history.max_bytes!=self.history.max_bytes||s.tools.foreground.map(f32::to_bits)!=self.foreground||s.tools.background.map(f32::to_bits)!=self.background||!photocraft_paint::native_stroke_controlled::checked_round_brush(&s.tools.brush).is_ok_and(|b|b==self.brush){return Err(fail("paint.stroke","the pressed native target changed before release"))}
  owner.validate_selection_replacement(c)?;let object=p.as_object_mut().ok_or(BackingError::Integrity)?;object.insert("documentId".into(),Value::String(self.snapshot.id.0.to_string()));object.insert("expectedRevision".into(),Value::String(self.revision.to_string()));object.insert("layer".into(),Value::from(self.layer.0));Ok(())
 })}
}

impl std::fmt::Debug for AccountedNativeStrokeTarget{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("AccountedNativeStrokeTarget").field("document",&self.snapshot.id).field("revision",&self.revision).field("layer",&self.layer).finish()}}

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="accounted_native_stroke_tests.rs"]
mod tests;
