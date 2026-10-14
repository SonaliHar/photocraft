//! Existing fixed-color native Normal/erase math, staged under the real job token.
//! CMS provider setup is boundary-polled; its internal execution is not interruptible.
//! Owned recipe resolution runs in the job; compatibility entry takes a resolved brush.
//! Bounded JSON/tool/axis capture
//! uses the standard allocator after prospective admission, not a deep token API.
use super::*;
use crate::{DocumentSnapshot,WeakDocumentSnapshot,EngineError,Result};
use photocraft_paint::stroke_controlled::{StrokeLimits,StrokeError};
use photocraft_raster::{Interrupt,Surface};
use std::mem::size_of;
#[path="ordinary_one_shot_controlled.rs"]
mod ordinary_one_shot_controlled;
#[path="live_begin_recipe_controlled.rs"]
pub(super) mod live_begin_recipe_controlled;
fn poll(c:&Interrupt<'_>)->Result<()>{c.check().map_err(|_|EngineError::Cancelled)}
fn fail(m:impl Into<String>)->EngineError{bad("paint.stroke",m)}
fn add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or_else(||fail("wide stroke accounting overflow"))}
fn mul(a:u64,b:u64)->Result<u64>{a.checked_mul(b).ok_or_else(||fail("wide stroke accounting overflow"))}
fn paint(e:StrokeError)->EngineError{match e{StrokeError::Cancelled=>EngineError::Cancelled,e=>fail(e.to_string())}}
fn raster(e:photocraft_raster::RasterInputError)->EngineError{match e{photocraft_raster::RasterInputError::Cancelled=>EngineError::Cancelled,e=>fail(e.to_string())}}
// Bounded borrowed parameter admission precedes Value::clone (standard allocator).
fn json_bytes(v:&Value,depth:usize,n:&mut usize,c:&Interrupt<'_>)->Result<u64>{
    poll(c)?;*n=n.checked_add(1).filter(|n|*n<=1_000_000).ok_or_else(||fail("wide parameter count"))?;
    if depth>16{return Err(fail("wide parameter depth"));}
    let mut b=size_of::<Value>() as u64;
    match v{Value::String(s)=>b=add(b,s.len() as u64)?,Value::Array(a)=>{b=add(b,mul(a.capacity() as u64,size_of::<Value>() as u64)?)?;for v in a{b=add(b,json_bytes(v,depth+1,n,c)?)?;}},Value::Object(o)=>for(k,v)in o{b=add(b,add(k.len() as u64+256,json_bytes(v,depth+1,n,c)?)?)?;},_=>{}}
    if b>64<<20{return Err(fail("wide parameter capacity"));}Ok(b)
}
pub(super) fn parameter_bytes(v:&Value)->Result<u64>{json_bytes(v,0,&mut 0,&Interrupt::NONE)}
pub(super) fn parameter_bytes_with_interrupt(v:&Value,c:&Interrupt<'_>)->Result<u64>{json_bytes(v,0,&mut 0,c)}
fn axis_copy(axis:Option<&crate::symmetry_cmds::PaintingSymmetry>)->Result<Option<crate::symmetry_cmds::PaintingSymmetry>>{
    use crate::symmetry_cmds::{PaintingSymmetry,SymmetryAxis};
    Ok(match axis{None=>None,Some(PaintingSymmetry::Preset(p))=>Some(PaintingSymmetry::Preset(*p)),Some(PaintingSymmetry::Path(a))=>{
        if a.source.len()>1<<20||a.segments.len()>1024{return Err(fail("wide symmetry capture limit"));}
        let mut source=String::new();source.try_reserve_exact(a.source.len()).map_err(|_|fail("symmetry capture allocation"))?;source.push_str(&a.source);
        let mut segments=Vec::new();segments.try_reserve_exact(a.segments.len()).map_err(|_|fail("symmetry capture allocation"))?;segments.extend_from_slice(&a.segments);
        Some(PaintingSymmetry::Path(SymmetryAxis{source,segments}))
    }})
}
/// Existing unit blend scope, checked before held-tip capture and again in
/// worker. Full strict encoded/profile interpretation stays in controlled CMS;
/// this cheap borrowed guard allocates no profile or raster payload.
pub(super) fn admit_blend_source(doc:&photocraft_doc::Document,id:Option<photocraft_doc::LayerId>,p:&Value,mode:photocraft_color::BlendMode,c:&Interrupt<'_>)->Result<()>{
    poll(c)?;if !matches!(mode,photocraft_color::BlendMode::Multiply|photocraft_color::BlendMode::Screen){return Ok(());}
    let source=crate::channel_cmds::read_target_surface(doc,id,p)?;
    if !matches!(source.format().sample,photocraft_color::SampleType::U32|photocraft_color::SampleType::F64){return Ok(());}
    if crate::channel_cmds::target_of(p)!=crate::channel_cmds::Target::Pixels||doc.xcf_state.is_some()||doc.source_color_context.is_some()||doc.color_table.is_some(){return Err(fail("native Multiply/Screen requires plain source-free pixel target"));}
    let layer=doc.layer(id.ok_or_else(||fail("native blend owner missing"))?).ok_or_else(||fail("native blend owner missing"))?;
    if !matches!(layer.content,LayerContent::Raster(_))||layer.indexed_source.is_some()||layer.video.is_some()||layer.import_compositing.is_some(){return Err(fail("native blend source carrier is unsupported"));}
    if !super::native_wide_color_controlled::eligible(doc,p){return Err(fail("native Multiply/Screen requires explicit Linear current ICC bounded to 16 MiB"));}
    if !matches!(source.format().mode,photocraft_color::ColorMode::Rgb|photocraft_color::ColorMode::Grayscale){return Err(fail("native unit blend model is unsupported"));}
    poll(c)
}
pub(super) fn eligible(s:&Session,cmd:&str,p:&Value,b:&BrushSettings,pts:&[StrokePoint],id:Option<photocraft_doc::LayerId>)->Result<bool>{
    if !matches!(b.mode,photocraft_color::BlendMode::Normal|photocraft_color::BlendMode::Multiply|photocraft_color::BlendMode::Screen)||s.live_commit.as_ref().is_some_and(|l|l.matches(s,cmd,p,pts)){return Ok(false);}
    let state=s.active().ok_or(EngineError::NoDocument)?;
    Ok(matches!(crate::channel_cmds::read_target_surface(&state.doc,id,p)?.format().sample,photocraft_color::SampleType::U32|photocraft_color::SampleType::F64))
}
fn equal_value(a:&Value,b:&Value,c:&Interrupt<'_>)->Result<bool>{
    poll(c)?;match(a,b){
        (Value::String(a),Value::String(b))=>{if a.len()!=b.len(){return Ok(false);}for(a,b)in a.as_bytes().chunks(4096).zip(b.as_bytes().chunks(4096)){poll(c)?;if a!=b{return Ok(false);}}Ok(true)},
        (Value::Array(a),Value::Array(b))=>{if a.len()!=b.len(){return Ok(false);}for(a,b)in a.iter().zip(b){if !equal_value(a,b,c)?{return Ok(false);}}Ok(true)},
        (Value::Object(a),Value::Object(b))=>{if a.len()!=b.len(){return Ok(false);}for(k,v)in a{poll(c)?;let Some(other)=b.get(k)else{return Ok(false);};if !equal_value(v,other,c)?{return Ok(false);}}Ok(true)},
        _=>Ok(a==b),
    }
}
pub(super) fn match_offer(offer:&LiveCommit,base:(photocraft_doc::DocId,u64),cmd:&str,p:&Value,points:&[StrokePoint],c:&Interrupt<'_>)->Result<bool>{
    poll(c)?;
    let work=mul(add(parameter_bytes_with_interrupt(p,c)?,parameter_bytes_with_interrupt(&offer.params,c)?)?,16)?;
    if work>StrokeLimits::default().work_units/16||offer.points.len()>100_000{return Err(fail("live offer match work limit"));}
    if offer.cmd!=cmd||offer.base!=base||p.get("seed").and_then(Value::as_u64)!=offer.seed||offer.points.len()!=points.len(){return Ok(false);}
    for(a,b)in offer.points.iter().zip(points){poll(c)?;if a!=b{return Ok(false);}}
    let (Some(a),Some(b))=(p.as_object(),offer.params.as_object())else{return Ok(false);};
    let count=|v:&serde_json::Map<String,Value>|v.len()-usize::from(v.contains_key("points"))-usize::from(v.contains_key("seed"));
    if count(a)!=count(b){return Ok(false);}for(k,v)in a{poll(c)?;if matches!(k.as_str(),"points"|"seed"){continue;}let Some(other)=b.get(k)else{return Ok(false);};if !equal_value(v,other,c)?{return Ok(false);}}
    poll(c)?;Ok(true)
}
struct Input{snapshot:DocumentSnapshot,weak:WeakDocumentSnapshot,revision:u64,active:Option<photocraft_doc::LayerId>,view:crate::channel_cmds::ChannelView,axis:Option<crate::symmetry_cmds::PaintingSymmetry>,id:Option<photocraft_doc::LayerId>,params:Value,brush:BrushSettings,points:Vec<StrokePoint>,zoom:f32,cmd:String,label:String,restricted:bool,auto_erase:bool,coalesce:Option<String>,request:Option<String>,tool_brush:BrushSettings,foreground:[f32;4],background:[f32;4],capture_bytes:u64,recipe:Option<super::brush_recipe_controlled::Recipe>,stores:Vec<crate::preset_store::TipStoreWitness>,presets_revision:u64,offer:Option<LiveCommit>}
struct Candidate{input:Input,output:Surface,damage:Rect,response:Value,lock:bool,changed:bool}
fn equal(a:&Surface,b:&Surface,c:&Interrupt<'_>)->Result<bool>{
    if a.format()!=b.format()||a.default_bytes()!=b.default_bytes(){return Ok(false);}
    for(coordinate,tile)in a.tiles(){poll(c)?;if let Some(other)=b.tile(*coordinate){for(x,y)in tile.bytes().chunks(4096).zip(other.bytes().chunks(4096)){poll(c)?;if x!=y{return Ok(false);}}}else{for chunk in tile.bytes().chunks(a.default_bytes().len()*256){poll(c)?;if chunk.chunks_exact(a.default_bytes().len()).any(|p|p!=a.default_bytes()){return Ok(false);}}}}
    for(coordinate,tile)in b.tiles(){poll(c)?;if a.tile(*coordinate).is_none(){for chunk in tile.bytes().chunks(b.default_bytes().len()*256){poll(c)?;if chunk.chunks_exact(b.default_bytes().len()).any(|p|p!=b.default_bytes()){return Ok(false);}}}}
    poll(c)?;Ok(true)
}
fn prepare(mut input:Input,c:&Interrupt<'_>)->Result<Candidate>{
    poll(c)?;let doc=&input.snapshot;
    DocumentSnapshot::resident_raster_footprint_of(doc,c)?;
    let native=photocraft_io::NativeSaveLimits::default().preparation;
    let held=photocraft_io::retained_native_input_bytes(doc,native,c).map_err(|e|if c.cancelled(){EngineError::Cancelled}else{fail(e.to_string())})? as u64;
    if let Some(recipe)=input.recipe.take(){
        let available=StrokeLimits::default().resident_bytes.checked_sub(add(held,input.capture_bytes)?).ok_or_else(||fail("brush recipe document headroom"))?;
        let (mut brush,points)=recipe.resolve_for(&input.cmd,&input.params,StrokeLimits{resident_bytes:available/2,work_units:StrokeLimits::default().work_units/8},c)?;
        input.label=if brush.erase&&!clear_mode(&input.params){"Eraser".into()}else{if input.cmd=="paint.pencil"{"Pencil".into()}else{"Brush Tool".into()}};
        if (crate::commands::is_mask_target(&input.params)||crate::channel_cmds::is_channel_target(&input.params))&&brush.erase{brush.erase=false;brush.color=input.background;}
        input.brush=brush;input.points=points;
    }
    // Source validation and current profile/selection context remain the same
    // frozen document as the old route. The checked constructor/composite scan
    // native origins and samples with the actual token.
    let source=crate::channel_cmds::read_target_surface(doc,input.id,&input.params)?;
    let format=source.format();let lock=match crate::channel_cmds::target_of(&input.params){
        crate::channel_cmds::Target::Pixels=>{let id=input.id.ok_or_else(||fail("pixel owner missing"))?;crate::commands::check_pixels_unlocked(doc,id)?;crate::xcf_item_locks::transparency_locked(doc,id)?},
        crate::channel_cmds::Target::Mask=>{crate::xcf_item_locks::require_layer_mask_content(doc,input.id.ok_or_else(||fail("mask owner missing"))?)?;false},
        crate::channel_cmds::Target::Alpha(i)=>{let channel=doc.channels.get(i).ok_or_else(||fail("channel missing"))?;crate::xcf_item_locks::require_channel_content(doc,channel.id)?;false},
        crate::channel_cmds::Target::QuickMask=>false,
    };
    if matches!(format.sample,photocraft_color::SampleType::U8|photocraft_color::SampleType::U16|photocraft_color::SampleType::F32){return ordinary_one_shot_controlled::prepare(input,lock,c);}
    admit_blend_source(doc,input.id,&input.params,input.brush.mode,c)?;
    if input.auto_erase{return Err(fail("native Pencil Auto Erase is unsupported"));}
    if let Some(offer)=input.offer.take(){
        if offer.matches_captured_with_interrupt((doc.id,input.revision),&input.cmd,&input.params,&input.points,c)?{
            if offer.surface.format()!=format{return Err(fail("live native offer format changed"));}
            offer.surface.try_validate_tile_origins_with(c).map_err(raster)?;
            let work=mul(add(source.resident_payload_bytes().map_err(raster)?,offer.surface.resident_payload_bytes().map_err(raster)?)?,4)?;
            if work>StrokeLimits::default().work_units/16{return Err(fail("live offer equality work"));}
            if i64::from(offer.bounds.x1)-i64::from(offer.bounds.x0)>i64::from(i32::MAX)||i64::from(offer.bounds.y1)-i64::from(offer.bounds.y0)>i64::from(i32::MAX){return Err(fail("live offer damage bounds"));}
            let changed=!equal(source,&offer.surface,c)?;let damage=if changed{offer.bounds}else{Rect::EMPTY};
            let response=json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()],"fromLive":true});poll(c)?;
            return Ok(Candidate{input,output:offer.surface,damage,response,lock,changed});
        }
    }
    let standard=StrokeLimits::default();
    let base_headroom=standard.resident_bytes.checked_sub(add(held,input.capture_bytes)?).ok_or_else(||fail("native profile document headroom"))?;
    // Recipe-loaded full tips are now retained alongside the profile phase,
    // whereas their future mip/output scratch is sequential. Pay the actual
    // current brush and typed points before any private profile copies.
    // This allocation-free census reports prospective constructor/mip work;
    // that work is debited once by one_shot_limits below, not spent again here.
    let brush_req=StrokeRenderer::new_requirements(&input.brush,Some(format),input.zoom,StrokeLimits{resident_bytes:base_headroom,work_units:standard.work_units},c).map_err(paint)?;
    let color_headroom=base_headroom.checked_sub(add(brush_req.retained_bytes,mul(input.points.capacity() as u64,size_of::<StrokePoint>() as u64)?)?).ok_or_else(||fail("native profile/loaded tip headroom"))?;
    poll(c)?;let (wide,profile_work)=wide_stroke_context_with_limits(doc,input.id,&input.params,&input.brush,format,lock,input.restricted,input.auto_erase,StrokeLimits{resident_bytes:color_headroom,work_units:standard.work_units/16},c)?;
    let mut wide=wide.ok_or_else(||fail("native target changed"))?;poll(c)?;
    if profile_work>standard.work_units/16{return Err(fail("native profile work envelope"));}
    let reflections=match &input.axis{Some(axis)=>stroke_controlled::prepare_reflected_passes(axis,&input.points,StrokeLimits{resident_bytes:standard.resident_bytes.checked_sub(input.capture_bytes).ok_or_else(||fail("capture headroom"))?,work_units:standard.work_units/8},c)?,None=>Vec::new()};
    let mut limits=live_begin_controlled::one_shot_limits(doc,&input.brush,format,input.zoom,reflections.len(),c)?;
    let mut capture_bytes=add(input.capture_bytes,mul(reflections.capacity() as u64,size_of::<Vec<StrokePoint>>() as u64)?)?;
    for pass in &reflections{poll(c)?;capture_bytes=add(capture_bytes,mul(pass.capacity() as u64,size_of::<StrokePoint>() as u64)?)?;}
    capture_bytes=add(capture_bytes,wide.color.resident_payload_bytes().map_err(raster)?)?;
    limits.resident_bytes=limits.resident_bytes.checked_sub(capture_bytes).ok_or_else(||fail("wide captured storage"))?;
    // Partition reflection/equality envelopes out of the aggregate stage work.
    let stage_count=2*(reflections.len() as u64+1)+1;
    limits.work_units=limits.work_units.checked_sub(standard.work_units/2/stage_count).ok_or_else(||fail("wide work headroom"))?;
    let comparison_budget=standard.work_units/16;
    wide.budget.extend(&input.brush,&input.points)?;for pass in &reflections{poll(c)?;wide.budget.extend(&input.brush,pass)?;}
    let mut primary=StrokeRenderer::try_new_with_interrupt(&input.brush,Some(format),input.zoom,limits,c).map_err(paint)?;
    primary.push_with_interrupt(&input.points,limits,c).map_err(paint)?;primary.finish_with_interrupt(limits,c).map_err(paint)?;
    let mut mirrors=Vec::new();mirrors.try_reserve_exact(reflections.len()).map_err(|_|fail("mirror allocation"))?;
    for pass in &reflections{poll(c)?;if !crate::symmetry_cmds::SymmetryAxis::has_distinct_mirror(&input.points,pass){continue;}
        let mut mirror=StrokeRenderer::try_new_with_interrupt(&input.brush,Some(format),input.zoom,limits,c).map_err(paint)?;mirror.push_with_interrupt(pass,limits,c).map_err(paint)?;mirror.finish_with_interrupt(limits,c).map_err(paint)?;mirrors.push(mirror);
    }
    let mut references=Vec::new();references.try_reserve_exact(mirrors.len()).map_err(|_|fail("mirror reference allocation"))?;references.extend(mirrors.iter());
    let mut output=source.clone();poll(c)?;
    let damage=primary.try_composite_union_many_wide_with_blend(&references,source,&mut output,&wide.color,doc.selection.as_ref(),lock,wide.clip,wide.blend,limits,c).map_err(raster)?;
    let comparison_work=mul(add(source.resident_payload_bytes().map_err(raster)?,output.resident_payload_bytes().map_err(raster)?)?,4)?;
    if comparison_work>comparison_budget{return Err(fail("wide equality work limit"));}
    let changed=!equal(source,&output,c)?;
    let damage=if changed{damage}else{Rect::EMPTY};
    let response=json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()]});poll(c)?;
    Ok(Candidate{input,output,damage,response,lock,changed})
}
fn apply(s:&mut Session,candidate:Candidate,c:&Interrupt<'_>)->Result<Value>{
    poll(c)?;let i=&candidate.input;let state=s.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id!=i.snapshot.id||state.revision!=i.revision||state.active_layer!=i.active||!i.weak.is_current(&state.doc)||!state.doc.shares_state(&i.snapshot)||state.channel_view!=i.view||state.symmetry!=i.axis||state.floating.is_some()||state.coalesce!=i.coalesce||s.coalesce_request!=i.request||s.tools.brush!=i.tool_brush||s.tools.foreground!=i.foreground||s.tools.background!=i.background||s.color_restrict.is_some()!=i.restricted||i.stores.iter().any(|w|!w.matches(s))||s.tools.presets_rev!=i.presets_revision{return Err(fail("captured native stroke context changed"));}
    let source=crate::channel_cmds::read_target_surface(&state.doc,i.id,&i.params)?;if source.format()!=candidate.output.format(){return Err(fail("native stroke format changed"));}
    let id=i.id;let params=&i.params;let lock=candidate.lock;let damage=candidate.damage;
    if candidate.changed{s.edit_with_interrupt(&i.label,c,|doc,_|{poll(c)?;let(target,current_lock)=crate::channel_cmds::target_surface(doc,id,params)?;if current_lock!=lock{return Err(fail("native stroke lock changed"));}*target=candidate.output;Ok(damage)})?;}
    // Scalar damage and prebuilt response only after the one document commit.
    if let Some(state)=s.active_mut(){state.last_damage=Some(damage);}Ok(candidate.response)
}
fn capture_input(s:&Session,cmd:&str,p:&Value,label:&str,brush:BrushSettings,points:Vec<StrokePoint>,id:Option<photocraft_doc::LayerId>,zoom:f32,auto_erase:bool,recipe:Option<super::brush_recipe_controlled::Recipe>,stores:Vec<crate::preset_store::TipStoreWitness>)->Result<Input>{
    stroke_controlled::dispatch_preflight(s,cmd,p,false)?;check_coords(&points,cmd)?;
    let state=s.active().ok_or(EngineError::NoDocument)?;
    if state.floating.is_some()||state.channel_view.color_hidden.len()>16||state.channel_view.alpha_visible.len()>10_000{return Err(fail("native stroke capture context limit"));}
    let params_bytes=parameter_bytes(p)?;
    let tool=StrokeRenderer::new_requirements(&s.tools.brush,None,zoom,StrokeLimits::default(),&Interrupt::NONE).map_err(paint)?;
    if state.coalesce.as_ref().is_some_and(|v|v.len()>4096)||s.coalesce_request.as_ref().is_some_and(|v|v.len()>4096)||cmd.len()>256||label.len()>4096{return Err(fail("wide scalar capture limit"));}
    let axis_bytes=match &state.symmetry{Some(crate::symmetry_cmds::PaintingSymmetry::Path(a))=>add(a.source.capacity() as u64,mul(a.segments.capacity() as u64,size_of::<[[f64;2];2]>() as u64)?)?,_=>64};
    let mut capture_bytes=add(add(tool.retained_bytes,tool.staged_bytes)?,add(mul(params_bytes,2)?,add(mul(points.capacity() as u64,size_of::<StrokePoint>() as u64)?,add(mul(axis_bytes,2)?,add(state.channel_view.color_hidden.capacity() as u64,state.channel_view.alpha_visible.capacity() as u64+16_384)?)?)?)?)?;
    capture_bytes=add(capture_bytes,recipe.as_ref().map_or(0,|r|r.retained_bytes))?;
    if capture_bytes>StrokeLimits::default().resident_bytes/4{return Err(fail("wide capture budget"));}
    let axis=axis_copy(state.symmetry.as_ref())?;let snapshot=state.doc.clone();let weak=snapshot.downgrade();
    let input=Input{snapshot,weak,revision:state.revision,active:state.active_layer,view:state.channel_view.clone(),axis,id,params:p.clone(),brush,points,zoom,cmd:cmd.into(),label:label.into(),restricted:s.color_restrict.is_some(),auto_erase,coalesce:state.coalesce.clone(),request:s.coalesce_request.clone(),tool_brush:s.tools.brush.clone(),foreground:s.tools.foreground,background:s.tools.background,capture_bytes,recipe,stores,presets_revision:s.tools.presets_rev,offer:None};
    Ok(input)
}
fn launch(s:&mut Session,input:Input,label:&str)->Result<Value>{
    crate::jobs::run(s,label,true,move|ctx|ctx.stage(0.0,0.95,"Prepare stroke",|c|prepare(input,c)).map(|candidate|(candidate,ctx.clone())),move|session,(candidate,ctx)|ctx.stage(0.95,1.0,"Apply stroke",|c|apply(session,candidate,c)))
}
pub(super) fn run(s:&mut Session,cmd:&str,p:&Value,label:&str,brush:BrushSettings,points:Vec<StrokePoint>,id:Option<photocraft_doc::LayerId>,zoom:f32,auto_erase:bool)->Result<Value>{
    let input=capture_input(s,cmd,p,label,brush,points,id,zoom,auto_erase,None,Vec::new())?;launch(s,input,label)
}
pub(super) fn try_run_recipe(s:&mut Session,cmd:&str,p:&Value)->Result<Option<Value>>{
    if !matches!(cmd,"paint.stroke"|"paint.pencil"){return Ok(None);}
    let state=s.active().ok_or(EngineError::NoDocument)?;
    let id=if crate::channel_cmds::is_channel_target(p){None}else{Some(layer_id(s,p)?)};
    let source=crate::channel_cmds::read_target_surface(&state.doc,id,p)?;
    let native_wide=matches!(source.format().sample,photocraft_color::SampleType::U32|photocraft_color::SampleType::F64);
    if !native_wide&&!matches!(source.format().sample,photocraft_color::SampleType::U8|photocraft_color::SampleType::U16|photocraft_color::SampleType::F32){return Ok(None);}
    match crate::channel_cmds::target_of(p){
        crate::channel_cmds::Target::Pixels=>crate::commands::check_pixels_unlocked(&state.doc,id.ok_or_else(||fail("pixel target missing"))?)?,
        crate::channel_cmds::Target::Mask=>crate::xcf_item_locks::require_layer_mask_content(&state.doc,id.ok_or_else(||fail("mask target missing"))?)?,
        crate::channel_cmds::Target::Alpha(index)=>crate::xcf_item_locks::require_channel_content(&state.doc,state.doc.channels.get(index).ok_or_else(||fail("channel target missing"))?.id)?,
        crate::channel_cmds::Target::QuickMask=>{},
    }
    if native_wide&&cmd=="paint.pencil"&&flag(p,"autoErase",false){return Err(fail("native Pencil Auto Erase is unsupported"));}
    let mut recipe=super::brush_recipe_controlled::Recipe::capture_for(s,cmd,p,&Interrupt::NONE)?;
    if native_wide&&!matches!(recipe.effective_mode(cmd,p),photocraft_color::BlendMode::Normal|photocraft_color::BlendMode::Multiply|photocraft_color::BlendMode::Screen){return Ok(None);}
    let stores=std::mem::take(&mut recipe.witnesses);
    let label=if cmd=="paint.pencil"{"Pencil"}else{"Brush Tool"};
    let mut input=capture_input(s,cmd,p,label,BrushSettings::default(),Vec::new(),id,num(p,"zoom").unwrap_or(1.0),cmd=="paint.pencil"&&flag(p,"autoErase",false),Some(recipe),stores)?;
    if let Some(offer)=s.live_commit.as_ref(){
        let held=add(parameter_bytes(&offer.params)?,add(mul(offer.points.capacity() as u64,size_of::<StrokePoint>() as u64)?,add(offer.surface.resident_payload_bytes().map_err(raster)?,mul(offer.surface.tile_count() as u64,512)?)?)?)?;
        input.capture_bytes=add(input.capture_bytes,held)?;
        if input.capture_bytes>StrokeLimits::default().resident_bytes/2{return Err(fail("live offer captured budget"));}
    }
    // Move only after all fallible capture/admission. A dispatch attempt consumes
    // offers under the existing command semantics; no preview API changes.
    input.offer=s.live_commit.take();
    let value=launch(s,input,label)?;s.live_commit=None;Ok(Some(value))
}

#[cfg(test)]
mod tests{
    use super::*;
    use photocraft_color::{ColorMode,SampleType};
    fn fixture(sample:SampleType)->Session{
        let mut doc=photocraft_doc::Document::new("Wide stroke",photocraft_doc::Size::new(8,8),ColorMode::Rgb,sample);
        doc.sample_encoding=Some(photocraft_doc::SampleEncoding::Linear);doc.icc_profile=Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        doc.layers.push(photocraft_doc::Layer::new("Pixels",LayerContent::Raster(Surface::with_default(doc.pixel_format(),&[0.25,0.5,0.75,0.8]))));
        let mut s=Session::new();s.add_document(doc,None).unwrap();s.tools.brush.pressure_size=false;s.tools.brush.smoothing.enabled=false;s
    }
    fn params()->Value{json!({"points":[[2,2,1],[5,5,1]],"color":[0.8,0.3,0.2,0.6],"size":3,"seed":17})}
    #[test]
    fn registered_native_normal_erase_single_history_and_raw_undo(){
        for sample in [SampleType::U32,SampleType::F64]{for erase in [false,true]{let mut s=fixture(sample);let mut p=params();p["erase"]=json!(erase);let before=s.active().unwrap().doc.clone();let rev=s.active().unwrap().revision;
            s.execute("paint.stroke",p).unwrap();assert_eq!(s.active().unwrap().revision,rev+1);let edited=s.active().unwrap().doc.clone();assert!(s.undo());assert_eq!(&*s.active().unwrap().doc,&*before);assert!(s.redo());assert_eq!(&*s.active().unwrap().doc,&*edited);
        }}
    }
    #[test]
    fn native_job_result_matches_legacy_generation_and_normal_composite(){
        for sample in [SampleType::U32,SampleType::F64]{let mut s=fixture(sample);let p=params();let points=parse_points(&p,"paint.stroke").unwrap();let brush=resolve_brush(&s,&p,"paint.stroke").unwrap();let(id,brush,zoom)=stroke_target(&s,&p,brush).unwrap();
            let doc=&s.active().unwrap().doc;let pre=crate::channel_cmds::read_target_surface(doc,id,&p).unwrap();
            let wide=wide_stroke_context(doc,id,&p,&brush,pre.format(),false,false,false).unwrap().unwrap();
            let mut renderer=StrokeRenderer::new(&brush,Some(pre.format()),zoom);renderer.push(&points);renderer.finish();
            let mut expected=pre.clone();renderer.try_composite_wide(pre,&mut expected,&wide.color,doc.selection.as_ref(),false,wide.clip,&Interrupt::NONE).unwrap();
            s.execute("paint.stroke",p).unwrap();let actual=s.active().unwrap().doc.layer(id.unwrap()).unwrap().surface().unwrap();assert!(equal(&expected,actual,&Interrupt::NONE).unwrap());
        }
    }
    #[test]
    fn registered_native_pencil_and_block_reuse_exact_live_offer(){
        for sample in [SampleType::U32,SampleType::F64]{for block in [false,true]{let mut s=fixture(sample);let mut p=params();p["block"]=json!(block);p["zoom"]=json!(2.5);
            let live=LiveStroke::begin_with(&s,"paint.pencil",&p).unwrap();let expected=crate::channel_cmds::read_target_surface(&live.doc,live.layer,&live.params).unwrap().clone();let rev=s.active().unwrap().revision;let before=s.active().unwrap().doc.clone();s.offer_live_stroke(live).unwrap();
            let response=s.execute("paint.pencil",p).unwrap();assert_eq!(response.get("fromLive"),Some(&json!(true)));assert_eq!(s.active().unwrap().revision,rev+1);assert!(equal(s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap(),&expected,&Interrupt::NONE).unwrap());assert!(s.undo());assert_eq!(&*s.active().unwrap().doc,&*before);
        }}
    }
    #[test]
    fn moved_stale_offer_falls_back_and_native_autoerase_preserves_redo(){
        let mut s=fixture(SampleType::F64);let p=params();let live=LiveStroke::begin_with(&s,"paint.stroke",&p).unwrap();s.offer_live_stroke(live).unwrap();let mut changed=p.clone();changed["seed"]=json!(23);let response=s.execute("paint.stroke",changed).unwrap();assert!(response.get("fromLive").is_none());assert!(s.undo());let before=s.active().unwrap().doc.clone();let mut refused=p;refused["autoErase"]=json!(true);
        assert!(s.execute("paint.pencil",refused).is_err());assert!(s.active().unwrap().doc.shares_state(&before));assert!(s.active().unwrap().history.can_redo());
    }
    #[test]
    fn registered_native_blends_match_existing_union_and_actual_job_cancel(){
        use photocraft_paint::wide_blend_controlled::NativeUnitBlend;
        for sample in [SampleType::U32,SampleType::F64]{for(mode,blend)in [("multiply",NativeUnitBlend::Multiply),("screen",NativeUnitBlend::Screen)]{
            let mut s=fixture(sample);let mut p=params();p["mode"]=json!(mode);let pts=parse_points(&p,"paint.stroke").unwrap();let brush=with_blend_mode(resolve_brush(&s,&p,"paint.stroke").unwrap(),&p);let(id,brush,zoom)=stroke_target(&s,&p,brush).unwrap();let doc=&s.active().unwrap().doc;let pre=crate::channel_cmds::read_target_surface(doc,id,&p).unwrap();
            let wide=wide_stroke_context(doc,id,&p,&brush,pre.format(),false,false,false).unwrap().unwrap();let mut renderer=StrokeRenderer::new(&brush,Some(pre.format()),zoom);renderer.push(&pts);renderer.finish();let mut expected=pre.clone();renderer.try_composite_union_many_wide_with_blend(&[],pre,&mut expected,&wide.color,doc.selection.as_ref(),false,wide.clip,Some(blend),StrokeLimits::default(),&Interrupt::NONE).unwrap();
            let before=s.active().unwrap().doc.clone();let revision=s.active().unwrap().revision;s.execute("paint.stroke",p.clone()).unwrap();assert_eq!(s.active().unwrap().revision,revision+1);let actual=s.active().unwrap().doc.layer(id.unwrap()).unwrap().surface().unwrap();assert!(equal(actual,&expected,&Interrupt::NONE).unwrap());assert!(s.undo());assert_eq!(&*s.active().unwrap().doc,&*before);
            let ctx=crate::jobs::JobCtx::default();ctx.cancel();s.set_inline_job_ctx(Some(ctx));assert!(matches!(s.execute("paint.stroke",p),Err(EngineError::Cancelled)));assert!(s.active().unwrap().doc.shares_state(&before));assert!(s.active().unwrap().history.can_redo());
        }}
    }
    #[test]
    fn native_unit_blend_live_match_and_nonlinear_refusal_preserve_authority(){
        let mut s=fixture(SampleType::F64);let mut p=params();p["mode"]=json!("screen");let live=LiveStroke::begin(&s,&p).unwrap();s.offer_live_stroke(live).unwrap();let response=s.execute("paint.stroke",p.clone()).unwrap();assert_eq!(response.get("fromLive"),Some(&json!(true)));assert!(s.undo());
        s.edit("Nonlinear",|doc,_|{doc.sample_encoding=Some(photocraft_doc::SampleEncoding::NonlinearProfile);Ok(())}).unwrap();let before=s.active().unwrap().doc.clone();let revision=s.active().unwrap().revision;assert!(s.execute("paint.stroke",p).is_err());assert!(s.active().unwrap().doc.shares_state(&before));assert_eq!(s.active().unwrap().revision,revision);
    }
    #[test]
    fn actual_inline_job_cancellation_and_zero_opacity_preserve_redo(){
        let mut s=fixture(SampleType::F64);s.edit("Redo",|doc,_|{doc.name="Redo".into();Ok(())}).unwrap();assert!(s.undo());let before=s.active().unwrap().doc.clone();
        let ctx=crate::jobs::JobCtx::default();ctx.cancel();s.set_inline_job_ctx(Some(ctx));assert!(matches!(s.execute("paint.stroke",params()),Err(EngineError::Cancelled)));assert!(s.active().unwrap().doc.shares_state(&before));assert!(s.active().unwrap().history.can_redo());
        s.set_inline_job_ctx(None);let mut p=params();p["opacity"]=json!(0);s.execute("paint.stroke",p).unwrap();assert!(s.active().unwrap().doc.shares_state(&before));assert!(s.active().unwrap().history.can_redo());
    }
}
