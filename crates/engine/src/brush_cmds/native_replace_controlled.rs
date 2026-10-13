//! Actual one-shot native RGB Luminosity Color Replacement. Native live preview
//! remains unsupported; never send this precision domain through LiveDab<f32>.
use crate::{EngineError,Result,Session,DocumentSnapshot,WeakDocumentSnapshot};
use photocraft_color::{ColorMode,SampleType};
use photocraft_doc::{LayerContent,LayerId};
use photocraft_paint::{StrokeRenderer,StrokeLimits,StrokeError,StrokePoint,BrushSettings};
use photocraft_paint::native_replace_controlled::{self,NativeLuminositySettings};
use photocraft_paint::replace::{Sampling,Limits};
use photocraft_raster::Interrupt;
use serde_json::{Value,json};
fn poll(c:&Interrupt<'_>)->Result<()> {c.check().map_err(|_|EngineError::Cancelled)}
fn bad(message:impl Into<String>)->EngineError {super::bad("paint.colorReplacement",message)}
fn add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or_else(||bad("native replacement accounting overflow"))}
fn mul(a:u64,b:u64)->Result<u64>{a.checked_mul(b).ok_or_else(||bad("native replacement accounting overflow"))}
fn paint(error:StrokeError)->EngineError {if error==StrokeError::Cancelled{EngineError::Cancelled}else{bad(error.to_string())}}
fn io(error:photocraft_io::IoError,c:&Interrupt<'_>)->EngineError{if matches!(error,photocraft_io::IoError::Cancelled)||c.cancelled(){EngineError::Cancelled}else{bad(error.to_string())}}
fn target(s:&Session,p:&Value)->Result<(LayerId,SampleType)> {
    if p.get("target").is_some_and(|value|value.as_str()!=Some("pixels")){return Err(bad("Color Replacement paints layer pixels only"));}
    let id=super::layer_id(s,p)?;let state=s.active().ok_or(EngineError::NoDocument)?;
    let layer=state.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let surface=layer.surface().ok_or_else(||bad("target has no editable pixels"))?;Ok((id,surface.format().sample))
}
fn tolerance(p:&Value)->Result<f64>{let v=match p.get("tolerance"){None|Some(Value::Null)=>30.0,Some(v)=>v.as_f64().filter(|v|v.is_finite()).ok_or_else(||bad("tolerance must be finite"))?};Ok(v.clamp(0.0,100.0)/100.0)}
fn sampling(p:&Value)->Result<Sampling>{match p.get("sampling"){None=>Ok(Sampling::Continuous),Some(Value::String(s))=>match s.as_str(){"continuous"=>Ok(Sampling::Continuous),"once"=>Ok(Sampling::Once),"backgroundSwatch"=>Ok(Sampling::BackgroundSwatch),_=>Err(bad("unsupported sampling"))},_=>Err(bad("sampling must be a named policy"))}}
fn limit(p:&Value)->Result<Limits>{match p.get("limits"){None=>Ok(Limits::Contiguous),Some(Value::String(s))=>match s.as_str(){"contiguous"=>Ok(Limits::Contiguous),"discontiguous"=>Ok(Limits::Discontiguous),"findEdges"=>Ok(Limits::FindEdges),_=>Err(bad("unsupported replacement limits"))},_=>Err(bad("limits must be a named policy"))}}
fn anti_alias(p:&Value)->Result<bool>{match p.get("antiAlias"){None=>Ok(true),Some(v)=>v.as_bool().ok_or_else(||bad("antiAlias must be boolean"))}}
/// false means the genuine legacy narrow handler; true admits only this explicit
/// RGB unit Luminosity lane. Call BEFORE float normalization/params cloning.
pub(super) fn dispatch_preflight(s:&Session,p:&Value,c:&Interrupt<'_>)->Result<bool>{
    poll(c)?;let (id,sample)=target(s,p)?;
    if !matches!(sample,SampleType::U32|SampleType::F64){return Ok(false);}
    super::stroke_controlled::dispatch_preflight(s,"paint.colorReplacement",p,false)?;
    let state=s.active().ok_or(EngineError::NoDocument)?;let doc=&state.doc;let layer=doc.layer(id).ok_or(EngineError::NoLayer(id))?;let format=layer.surface().ok_or_else(||bad("missing native target"))?.format();
    if format.mode!=ColorMode::Rgb||doc.mode!=ColorMode::Rgb||doc.depth!=sample||!matches!(layer.content,LayerContent::Raster(_))
        ||doc.xcf_state.is_some()||doc.source_color_context.is_some()||doc.color_table.is_some()||layer.indexed_source.is_some()||layer.video.is_some()||layer.import_compositing.is_some(){return Err(bad("native Luminosity requires a source-free resident RGB raster"));}
    if p.get("mode").and_then(Value::as_str)!=Some("luminosity"){return Err(bad("native Color Replacement supports explicit Luminosity only"));}
    if state.floating.is_some()||state.coalesce.is_some()||s.coalesce_request.is_some()||s.color_restrict.is_some(){return Err(bad("finish transient work before native Color Replacement"));}
    crate::commands::check_pixels_unlocked(doc,id)?;
    crate::native_linear_context::validate(doc,c).map_err(|message|if c.cancelled(){EngineError::Cancelled}else{bad(message)})?;
    if doc.selection.as_ref().is_some_and(|selection|selection.format().mode!=ColorMode::Grayscale||selection.format().alpha){return Err(bad("native replacement selection must be Gray coverage without alpha"));}
    let tol=tolerance(p)?;sampling(p)?;let limits=limit(p)?;let aa=anti_alias(p)?;if aa&&limits!=Limits::FindEdges&&tol>0.0&&tol*0.25==0.0{return Err(bad("unrepresentable tolerance falloff"));}poll(c)?;Ok(true)
}
/// Native preview needs a distinct f64 sequential-work state; the existing
/// public LiveDab facade remains genuinely legacy rather than silently narrowing.
pub(super) fn refuse_native_preview(s:&Session,p:&Value,c:&Interrupt<'_>)->Result<()> {
    poll(c)?;let (_,sample)=target(s,p)?;if matches!(sample,SampleType::U32|SampleType::F64){return Err(bad("native Color Replacement live preview is unsupported"));}Ok(())
}
#[path="native_replace_preview.rs"]
mod preview;
pub use preview::{NativeLuminosityPreview,NativeLuminosityCommit,try_native_color_replacement_preview};
#[derive(Clone)]
struct Capture {
    snapshot:DocumentSnapshot,weak:WeakDocumentSnapshot,revision:u64,
    active:Option<LayerId>,id:LayerId,format:photocraft_color::PixelFormat,
    lock:bool,view:crate::channel_cmds::ChannelView,
}
struct Input {
    capture:Capture,points:Vec<StrokePoint>,brush:BrushSettings,
    foreground:[f64;4],background:[f64;4],settings:NativeLuminositySettings,
}
struct Candidate {capture:Capture,output:photocraft_raster::Surface,damage:photocraft_geom::Rect,response:Value}
fn capture(s:&Session,p:&Value,c:&Interrupt<'_>)->Result<Option<Input>> {
    if !dispatch_preflight(s,p,c)?{return Ok(None);}
    let (id,_)=target(s,p)?;let state=s.active().ok_or(EngineError::NoDocument)?;
    if state.channel_view.color_hidden.len()>16||state.channel_view.alpha_visible.len()>10_000{return Err(bad("native replacement view capture limit"));}
    let snapshot=state.doc.clone();let format=snapshot.layer(id).and_then(|layer|layer.surface()).ok_or_else(||bad("native replacement target missing"))?.format();
    let captured=Capture{weak:snapshot.downgrade(),revision:state.revision,active:state.active_layer,id,format,lock:snapshot.effective_locks(id).transparency,view:state.channel_view.clone(),snapshot};
    // The legacy bounded parameter/tip resolver is intentionally front-end work:
    // authorized Stored tips are resolved here, never by worker ambient reads.
    // Its JSON/asset internals remain boundary-polled, not deeply cancellable.
    let points=super::parse_points(p,"paint.colorReplacement")?;let brush=super::resolve_brush(s,p,"paint.colorReplacement")?;poll(c)?;
    if points.len()>100_000{return Err(bad("native replacement point count limit"));}
    let foreground=crate::paint_cmds::wide_color(p.get("color"),brush.color)?;
    let settings=NativeLuminositySettings{foreground:[0.0;3],background:[0.0;3],sampling:sampling(p)?,limits:limit(p)?,tolerance:tolerance(p)?,anti_alias:anti_alias(p)?,opacity:f64::from(brush.opacity)};
    poll(c)?;Ok(Some(Input{capture:captured,points,brush,foreground,background:s.tools.background.map(f64::from),settings}))
}
fn prepare(mut input:Input,c:&Interrupt<'_>)->Result<Candidate> {
    poll(c)?;let snapshot=&input.capture.snapshot;let native=photocraft_io::NativeSaveLimits::default().preparation;
    let model=photocraft_io::retained_native_model_bytes(snapshot,native,c).map_err(|e|io(e,c))? as u64;
    let payload=photocraft_io::retained_native_payload_bytes(snapshot,native,c).map_err(|e|io(e,c))? as u64;
    let raster=DocumentSnapshot::resident_raster_footprint_of(snapshot,c)?;
    let views=add(input.capture.view.color_hidden.capacity() as u64,input.capture.view.alpha_visible.capacity() as u64)?;
    let point_bytes=mul(input.points.capacity() as u64,std::mem::size_of::<StrokePoint>() as u64)?;
    let held=add(payload,add(mul(model,3)?,add(mul(raster.clone_metadata_bytes,3)?,add(point_bytes,add(views,512)?)?)?)?)?;
    let standard=StrokeLimits::default();let remaining=standard.resident_bytes.checked_sub(held).ok_or_else(||bad("native replacement document/input headroom"))?;
    crate::native_linear_context::validate(snapshot,c).map_err(|message|if c.cancelled(){EngineError::Cancelled}else{bad(message)})?;
    // The admitted lane is explicitly pixel-only. Value::Null is the borrowed
    // pixel target counterpart, not an inferred channel route or copied params.
    let fg=super::native_wide_color(snapshot,input.capture.format,&Value::Null,input.foreground)?;poll(c)?;
    input.settings.foreground=[fg[0],fg[1],fg[2]];
    if input.settings.sampling==Sampling::BackgroundSwatch{let bg=super::native_wide_color(snapshot,input.capture.format,&Value::Null,input.background)?;poll(c)?;input.settings.background=[bg[0],bg[1],bg[2]];}
    // Reserve constructor/push/finish/census work separately: their four lowered
    // envelopes total at most one quarter, and native replacement gets 3/4.
    let generation=StrokeLimits{resident_bytes:remaining/4,work_units:standard.work_units/16};
    let mut renderer=StrokeRenderer::try_new_with_interrupt(&input.brush,None,1.0,generation,c).map_err(paint)?.record_dabs();
    renderer.push_with_interrupt(&input.points,generation,c).map_err(paint)?;renderer.finish_with_interrupt(generation,c).map_err(paint)?;
    let req=renderer.incremental_clone_requirements(generation,c).map_err(paint)?;
    let source=snapshot.layer(input.capture.id).and_then(|layer|layer.surface()).ok_or_else(||bad("native replacement source missing"))?;
    let mut output=SurfaceCopy::copy(source,remaining,c)?;
    let lower=StrokeLimits{resident_bytes:remaining.checked_sub(req.retained_bytes).ok_or_else(||bad("native replacement renderer headroom"))?,work_units:standard.work_units-standard.work_units/4};
    let damage=native_replace_controlled::try_apply_luminosity(&mut output,&renderer,&input.settings,snapshot.selection.as_ref(),input.capture.lock,lower,c).map_err(paint)?;
    let response=if damage.is_empty(){json!({"damage":[0,0,0,0],"nativeLuminosity":true})}else{json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()],"nativeLuminosity":true})};
    poll(c)?;Ok(Candidate{capture:input.capture,output,damage,response})
}
fn validate_capture(s:&Session,captured:&Capture,c:&Interrupt<'_>)->Result<()> {
    poll(c)?;let state=s.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id!=captured.snapshot.id||state.revision!=captured.revision||state.active_layer!=captured.active
        ||!captured.weak.is_current(&state.doc)||!state.doc.shares_state(&captured.snapshot)||state.channel_view!=captured.view
        ||state.floating.is_some()||state.coalesce.is_some()||s.coalesce_request.is_some()||s.color_restrict.is_some(){return Err(bad("captured native replacement document/view changed"));}
    crate::commands::check_pixels_unlocked(&state.doc,captured.id)?;
    if state.doc.effective_locks(captured.id).transparency!=captured.lock{return Err(bad("native target alpha lock changed"));}
    crate::native_linear_context::validate(&state.doc,c).map_err(|message|if c.cancelled(){EngineError::Cancelled}else{bad(message)})?;
    poll(c)
}
fn apply(s:&mut Session,candidate:Candidate,c:&Interrupt<'_>)->Result<Value> {
    validate_capture(s,&candidate.capture,c)?;let captured=&candidate.capture;
    poll(c)?;if candidate.damage.is_empty(){return Ok(candidate.response);}
    let id=captured.id;let format=captured.format;let lock=captured.lock;let damage=candidate.damage;
    // Response is owned from worker preparation. Publication is followed only
    // by returning it, with no response allocation or private rendering.
    s.edit_with_interrupt("Color Replacement",c,|doc,_|{
        poll(c)?;crate::commands::check_pixels_unlocked(doc,id)?;
        if doc.effective_locks(id).transparency!=lock{return Err(bad("native target alpha lock changed"));}
        let target=doc.layer_mut(id).and_then(|layer|layer.surface_mut()).ok_or_else(||bad("native replacement target changed"))?;
        if target.format()!=format{return Err(bad("native replacement target format changed"));}
        *target=candidate.output;Ok(damage)
    })?;
    Ok(candidate.response)
}
/// Registered entry: jobs::run executes inline/wasm with its inline token, or
/// on the native worker when Session::start is used. No Session enters work.
pub(super) fn try_run_job(s:&mut Session,p:&Value)->Result<Option<Value>> {
    if let Some(commit)=s.native_replace_commit.take(){
        let candidate=commit.into_candidate_for_command(s,p,&Interrupt::NONE)?;
        let value=crate::jobs::run(s,"Color Replacement",true,move|ctx|{ctx.stage(0.0,0.1,"Captured native preview",poll)?;Ok((candidate,ctx.clone()))},move|session,(candidate,ctx)|ctx.stage(0.1,1.0,"Apply native preview",|control|apply(session,candidate,control)))?;
        return Ok(Some(value));
    }
    let Some(input)=capture(s,p,&Interrupt::NONE)?else{return Ok(None);};
    let response=crate::jobs::run(s,"Color Replacement",true,move|ctx|{
        ctx.stage(0.0,0.95,"Native Luminosity",|control|prepare(input,control)).map(|candidate|(candidate,ctx.clone()))
    },move|session,(candidate,ctx)|{
        ctx.stage(0.95,1.0,"Apply native Luminosity",|control|apply(session,candidate,control))
    })?;
    Ok(Some(response))
}
/// Explicit synchronous token seam for cancellation callers/drafts. It shares
/// the exact capture, preparation and publication logic with the registered job.
#[cfg_attr(not(test),allow(dead_code,reason="explicit token seam; registered handler uses try_run_job"))]
pub(super) fn try_run(s:&mut Session,p:&Value,c:&Interrupt<'_>)->Result<Option<Value>> {
    let Some(input)=capture(s,p,c)?else{return Ok(None);};let candidate=prepare(input,c)?;Ok(Some(apply(s,candidate,c)?))
}
// Clone-only sparse carrier: payload Arcs stay shared. BTree node allocation
// remains standard-library managed under admitted metadata, not all-allocator/RSS.
struct SurfaceCopy;
impl SurfaceCopy {
    fn copy(source:&photocraft_raster::Surface,limit:u64,c:&Interrupt<'_>)->Result<photocraft_raster::Surface>{
        let bytes=add(source.default_bytes().len() as u64,mul(source.tile_count() as u64,512)?)?;if bytes>limit{return Err(bad("native output map headroom"));}
        let mut out=photocraft_raster::Surface::try_with_default_bytes(source.format(),source.default_bytes()).map_err(|e|bad(e.to_string()))?;
        for (coord,tile)in source.tiles(){poll(c)?;out.put_tiles(std::iter::once((*coord,std::sync::Arc::clone(tile))));}poll(c)?;Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->Session {
        let mut doc=photocraft_doc::Document::new("Cancelable native replacement",photocraft_doc::Size::new(8,8),ColorMode::Rgb,SampleType::F64);
        doc.sample_encoding=Some(photocraft_doc::SampleEncoding::Linear);doc.icc_profile=Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        doc.layers.push(photocraft_doc::Layer::new("Native source",LayerContent::Raster(photocraft_raster::Surface::with_default(doc.pixel_format(),&[0.5,0.5,0.5,0.75]))));
        let mut session=Session::new();session.add_document(doc,None).unwrap();session.tools.brush.pressure_size=false;session.tools.brush.smoothing.enabled=false;
        session.edit("Real redo",|doc,_|{doc.name="Redo".into();Ok(())}).unwrap();assert!(session.undo());session
    }
    fn params()->Value{json!({"mode":"luminosity","points":[[2,2,1],[5,5,1]],"color":[0.8,0.8,0.8,1],"size":4,"flow":1,"smoothing":0,"sampling":"once","limits":"discontiguous","tolerance":100})}
    #[test]
    fn synchronous_token_cancel_leaves_snapshot_and_redo() {
        let mut session=fixture();let before=session.active().unwrap().doc.clone();let revision=session.active().unwrap().revision;
        let count=std::cell::Cell::new(0usize);let cancel=||{count.set(count.get()+1);count.get()>50};
        assert!(matches!(try_run(&mut session,&params(),&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));assert_eq!(session.active().unwrap().revision,revision);assert!(session.active().unwrap().history.can_redo());
    }
    #[test]
    fn registered_inline_job_obeys_job_context_cancel() {
        let mut session=fixture();let before=session.active().unwrap().doc.clone();let revision=session.active().unwrap().revision;
        let context=crate::jobs::JobCtx::default();context.cancel();session.set_inline_job_ctx(Some(context));
        assert!(matches!(session.execute("paint.colorReplacement",params()),Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));assert_eq!(session.active().unwrap().revision,revision);assert!(session.active().unwrap().history.can_redo());
    }
    #[test]
    fn prepared_native_result_cannot_apply_after_target_view_change() {
        let mut session=fixture();let input=capture(&session,&params(),&Interrupt::NONE).unwrap().unwrap();let candidate=prepare(input,&Interrupt::NONE).unwrap();
        let before=session.active().unwrap().doc.clone();let revision=session.active().unwrap().revision;
        session.active_mut().unwrap().channel_view.target=crate::channel_cmds::ChannelTarget::Color(1);
        assert!(apply(&mut session,candidate,&Interrupt::NONE).is_err());assert!(session.active().unwrap().doc.shares_state(&before));assert_eq!(session.active().unwrap().revision,revision);assert!(session.active().unwrap().history.can_redo());
    }
}
