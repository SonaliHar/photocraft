//! Frozen Load Selection. Explicit native unit coverage is separate from legacy GRAY8.
#[path="canonical_selection_storage.rs"]
pub(super) mod canonical_selection_storage;
#[path="stored_load_publication.rs"]
pub(crate) mod stored_load_publication;
use super::{ChanRef,SnapshotSource,bad,source_dimensions,source_reference,source_owner,source_from_snapshot_with_interrupt};
use crate::{DocumentSnapshot,EngineError,Result,Session};
use photocraft_algo::selection::SelectionMode;
use photocraft_algo::gimp::multi_input_coverage::{self as coverage,Operation};
use photocraft_color::{ColorMode,PixelFormat,SampleType};
use photocraft_color::checked_arithmetic::{self as arithmetic,Domain,Operand,Policy,Rounding,Storage,UnitOverflow};
use photocraft_doc::{Document,LayerId};
use photocraft_geom::{Rect,TILE_SIZE};
use photocraft_raster::{Interrupt,Surface,MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value,json};
const CMD:&str="select.loadSelection";
fn poll(ctl:&Interrupt<'_>)->Result<()>{ctl.check().map_err(|_|EngineError::Cancelled)}
fn failure(e:impl std::fmt::Display,ctl:&Interrupt<'_>)->EngineError{if ctl.cancelled(){EngineError::Cancelled}else{bad(CMD,e.to_string())}}
fn exact(value:&Value,nonzero:bool)->Result<u64>{
    let text=value.as_str().filter(|s|!s.is_empty()&&s.len()<=20&&s.bytes().all(|b|b.is_ascii_digit())&&(s.len()==1||!s.starts_with('0'))).ok_or_else(||bad(CMD,"captured IDs must be canonical decimal strings"))?;
    let value=text.parse::<u64>().map_err(|_|bad(CMD,"ID exceeds range"))?;if nonzero&&value==0{return Err(bad(CMD,"ID must be nonzero"));}Ok(value)
}
#[derive(Clone,Copy)]
struct Request{source_index:usize,source:SnapshotSource,mode:SelectionMode,native:bool,raw_storage:bool}
fn request(session:&Session,value:&Value,allow_normalize:bool)->Result<Request>{
    let object=value.as_object().filter(|p|p.len()<=11&&p.keys().all(|k|matches!(k.as_str(),"document"|"layer"|"channel"|"channelId"|"invert"|"operation"|"documentId"|"expectedRevision"|"nativeCoverage"|"coverageContext"|"rawStorage"))).ok_or_else(||bad(CMD,"expected bounded Load Selection parameters"))?;
    let target=session.active().ok_or(EngineError::NoDocument)?;
    if session.documents().len()>4096{return Err(bad(CMD,"document count limit"));}
    match(value.get("documentId"),value.get("expectedRevision")){
        (None,None)=>{},(Some(id),Some(rev)) if exact(id,true)?==target.doc.id.0&&exact(rev,false)?==target.revision=>{},_=>return Err(bad(CMD,"captured target changed or incomplete")),
    }
    let raw_storage=match value.get("rawStorage"){None=>false,Some(v)=>v.as_bool().ok_or_else(||bad(CMD,"rawStorage must be boolean"))?};
    let numeric_native=match value.get("nativeCoverage"){None=>false,Some(v)=>v.as_bool().ok_or_else(||bad(CMD,"nativeCoverage must be boolean"))?};
    match(value.get("coverageContext"),numeric_native){(Some(Value::String(s)),true)if s=="unit"=>{},(None,false)=>{},_=>return Err(bad(CMD,"nativeCoverage requires explicit coverageContext unit")),}
    if raw_storage&&numeric_native{return Err(bad(CMD,"rawStorage and nativeCoverage are distinct policies"));}
    let native=numeric_native||raw_storage;
    let mode=match value.get("operation"){None|Some(Value::Null)=>SelectionMode::Replace,Some(Value::String(s))=>match s.as_str(){"new"|"replace"=>SelectionMode::Replace,"add"=>SelectionMode::Add,"subtract"=>SelectionMode::Subtract,"intersect"=>SelectionMode::Intersect,_=>return Err(bad(CMD,"unknown selection operation"))},_=>return Err(bad(CMD,"operation must be a name")),};
    let source_index=match value.get("document"){
        None|Some(Value::Null)=>session.active_index().ok_or(EngineError::NoDocument)?,
        Some(v)=>if let Some(n)=v.as_u64(){usize::try_from(n).map_err(|_|bad(CMD,"source document index exceeds range"))?}else{let n=v.as_f64().filter(|n|n.is_finite()&&n.fract()==0.0&&*n>=0.0&&*n<(session.documents().len() as f64)).ok_or_else(||bad(CMD,"invalid source document index"))?;n as usize},
    };
    let source=session.documents().get(source_index).ok_or_else(||bad(CMD,"source document is not open"))?;
    if source.doc.channels.len()>10000{return Err(bad(CMD,"source channel count limit"));}
    if object.contains_key("channel")&&object.contains_key("channelId"){return Err(bad(CMD,"supply one channel selector"));}
    let channel=if let Some(value)=value.get("channelId"){
        let id=super::channel_identity(value).ok_or_else(||bad(CMD,"invalid channel identity"))?;
        let mut found=None;for(index,channel)in source.doc.channels.iter().enumerate(){if channel.id.0==id{if found.is_some(){return Err(bad(CMD,"ambiguous source channel ID"));}found=Some(index);}}
        ChanRef::Alpha(found.ok_or_else(||bad(CMD,"unknown source channel ID"))?)
    }else{source_reference(value.get("channel").ok_or_else(||bad(CMD,"missing channel"))?,&source.doc,CMD)?};
    let layer=match value.get("layer"){
        None|Some(Value::Null)=>None,Some(Value::String(s)) if s.is_empty()||s=="merged"=>None,
        Some(Value::String(_))=>Some(LayerId(exact(value.get("layer").ok_or_else(||bad(CMD,"missing layer"))?,true)?)),
        Some(v)=>Some(LayerId(v.as_u64().filter(|n|*n!=0).ok_or_else(||bad(CMD,"source layer must be nonzero ID or merged"))?)),
    };
    let invert=match value.get("invert"){None|Some(Value::Null)=>false,Some(v)=>v.as_bool().ok_or_else(||bad(CMD,"invert must be boolean"))?};
    if raw_storage&&(!matches!(channel,ChanRef::Alpha(_))||invert||mode!=SelectionMode::Replace){return Err(bad(CMD,"rawStorage requires saved Gray replace without invert"));}
    source_dimensions(&source.doc,CMD)?;source_dimensions(&target.doc,CMD)?;
    if source.doc.size!=target.doc.size{return Err(bad(CMD,"source dimensions must match the active target"));}
    let normalizes=allow_normalize&&value.get("documentId").is_none()&&!native&&target.doc.xcf_state.is_none()&&matches!(target.doc.depth,SampleType::U8|SampleType::U16|SampleType::F32);
    for state in [source,target]{
        if state.coalesce.is_some()||state.doc.accounted().is_some()||state.doc.xcf_state.as_ref().is_some_and(|s|s.floating.is_some())||session.command_has_job_on(state.doc.id)
            ||(state.floating.is_some()&&(!normalizes||state.doc.id!=target.doc.id)){return Err(bad(CMD,"finish selected source/target preview or pending work first"));}
    }
    if target.revision==u64::MAX||session.has_jobs()||session.command_has_pending_job()||session.coalesce_request.is_some()||session.color_restrict.is_some(){return Err(bad(CMD,"finish jobs/coalescing before Load Selection"));}
    if native{
        if !matches!(channel,ChanRef::Alpha(_)|ChanRef::QuickMask|ChanRef::LayerMask){return Err(bad(CMD,"nativeCoverage supports only plain saved Gray channels or native layer masks"));}
        if source.doc.xcf_state.is_some()||target.doc.xcf_state.is_some(){
            if !raw_storage{return Err(bad(CMD,"retained source coverage context is not qualified"));}
            let ChanRef::Alpha(index)=channel else{return Err(bad(CMD,"canonical raw transfer requires a saved channel"));};
            let id=source.doc.channels.get(index).ok_or_else(||bad(CMD,"missing source channel"))?.id;
            canonical_selection_storage::prepare(&source.doc,&target.doc,id,&Interrupt::NONE)?;
        }
    }else if matches!(source.doc.depth,SampleType::U32|SampleType::F64)||matches!(target.doc.depth,SampleType::U32|SampleType::F64)
        ||target.doc.selection.as_ref().is_some_and(|s|matches!(s.format().sample,SampleType::U32|SampleType::F64)){
        return Err(bad(CMD,"wide coverage requires explicit nativeCoverage unit context"));
    }
    Ok(Request{source_index,source:SnapshotSource{layer,channel,gray:true,invert},mode,native,raw_storage})
}
pub(super) fn dispatch_preflight(session:&Session,value:&Value,background:bool)->Result<()>{
    if value.get("storedSource").is_some() { return stored_load_publication::preflight(session,value,background); }
    request(session,value,true).map(|_|())
}
fn native_source<'a>(doc:&'a Document,active:Option<LayerId>,request:Request,ctl:&Interrupt<'_>)->Result<&'a Surface>{
    poll(ctl)?;
    match request.source.channel{
        ChanRef::Alpha(index)=>{
            let channel=doc.channels.get(index).ok_or_else(||bad(CMD,"missing saved channel"))?;
            if channel.spot.is_some(){return Err(bad(CMD,"nativeCoverage requires a plain saved alpha channel"));}
            if channel.id.0==0||doc.channels.iter().filter(|other|other.id==channel.id).count()!=1{return Err(bad(CMD,"ambiguous saved channel identity"));}Ok(&channel.surface)
        },
        ChanRef::QuickMask=>{let channel=doc.quick_mask.as_ref().ok_or_else(||bad(CMD,"not in Quick Mask mode"))?;if channel.spot.is_some(){return Err(bad(CMD,"native Quick Mask cannot carry spot ink"));}Ok(&channel.surface)},
        ChanRef::LayerMask=>{let id=request.source.layer.or(active).ok_or_else(||bad(CMD,"layer mask needs a layer"))?;let layer=source_owner(doc,Some(id),false,ctl)?.ok_or(EngineError::NoLayer(id))?;
            if !matches!(layer.content,photocraft_doc::LayerContent::Raster(_))||layer.indexed_source.is_some()||layer.video.is_some(){return Err(bad(CMD,"native mask requires a plain resident raster owner"));}
            Ok(&layer.mask.as_ref().ok_or_else(||bad(CMD,"source layer has no mask"))?.surface)},
        _=>Err(bad(CMD,"unsupported native source")),
    }
}
fn unit(bytes:&[u8],sample:SampleType)->Result<f64>{arithmetic::decode_finite(Operand{bytes,storage:Storage::Native(sample),index:0},Domain::Unit).map_err(|e|bad(CMD,e.to_string()))}
fn plain(surface:&Surface,ctl:&Interrupt<'_>)->Result<()>{
    poll(ctl)?;let format=surface.format();
    if format.mode!=ColorMode::Grayscale||format.alpha||format.bytes_per_pixel()>8{return Err(bad(CMD,"nativeCoverage requires Gray without alpha"));}
    surface.try_validate_tile_origins_with(ctl).map_err(|e|failure(e,ctl))?;
    if surface.default_bytes().len()!=format.bytes_per_pixel(){return Err(bad(CMD,"coverage default length"));}unit(surface.default_bytes(),format.sample)?;
    let expected=(TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n|n.checked_mul(format.bytes_per_pixel())).ok_or_else(||bad(CMD,"coverage tile length overflow"))?;
    for(_,tile)in surface.tiles(){poll(ctl)?;if tile.bytes().len()!=expected{return Err(bad(CMD,"coverage tile length"));}
        for chunk in tile.bytes().chunks(format.bytes_per_pixel()*1024){poll(ctl)?;for sample in chunk.chunks_exact(format.bytes_per_pixel()){unit(sample,format.sample)?;}}
    }poll(ctl)
}
fn tile_count(bounds:Rect)->Result<u64>{if bounds.is_empty(){Ok(0)}else{crate::xcf_indexed_pixel_cmds::tile_count(bounds)}}
fn prepare(source:&DocumentSnapshot,target:&DocumentSnapshot,active:Option<LayerId>,request:Request,ctl:&Interrupt<'_>)->Result<()> {
    poll(ctl)?;source.validate_resident()?;target.validate_resident()?;
    let(area,count)=source_dimensions(target,CMD)?;
    let mut peak=0u64;let mut work=(count as u64).checked_mul(if request.native{512}else{128}).ok_or_else(||bad(CMD,"coverage work overflow"))?;
    for document in [source,target]{let limits=photocraft_io::NativeSaveLimits::default().preparation;
        let model=photocraft_io::retained_native_model_bytes(document,limits,ctl).map_err(|e|failure(e,ctl))? as u64;
        let payload=photocraft_io::retained_native_payload_bytes(document,limits,ctl).map_err(|e|failure(e,ctl))? as u64;
        let raster=document.resident_raster_footprint_with(ctl)?;
        peak=peak.checked_add(payload).and_then(|n|n.checked_add(model.checked_mul(4)?)).and_then(|n|n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?)).ok_or_else(||bad(CMD,"source/target model headroom overflow"))?;
    }
    let format=if request.native{let source=native_source(source,active,request,ctl)?;let format=source.format();if format.sample!=target.depth{return Err(bad(CMD,"native source and target depth must match"));}
        if let Some(old)=target.selection.as_ref().filter(|_|!request.raw_storage){if old.format()!=format{return Err(bad(CMD,"native current selection format must match source"));}}
        format
    }else{PixelFormat::GRAY8};
    let tiles=tile_count(area)?;let payload=(TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n|n.checked_mul(format.bytes_per_pixel() as u64)).ok_or_else(||bad(CMD,"coverage tile charge overflow"))?;
    let staged=payload.checked_add(1024).and_then(|n|n.checked_mul(tiles)).and_then(|n|n.checked_mul(3)).and_then(|n|n.checked_add(payload.checked_mul(8)?)).and_then(|n|n.checked_add((count as u64).checked_mul(if request.native{0}else{48})?)).ok_or_else(||bad(CMD,"coverage stage charge overflow"))?;
    peak=peak.checked_add(staged).filter(|n|*n<=MAX_CHECKED_RASTER_BYTES).ok_or_else(||bad(CMD,"aggregate coverage output/plane limit"))?;
    for surface in target.selection.iter(){work=work.checked_add((surface.tile_count() as u64).checked_mul((TILE_SIZE as u64)*(TILE_SIZE as u64)).and_then(|n|n.checked_mul(16)).ok_or_else(||bad(CMD,"selection validation work overflow"))?).ok_or_else(||bad(CMD,"selection validation work overflow"))?;}
    if request.native{let surface=native_source(source,active,request,ctl)?;work=work.checked_add((surface.tile_count() as u64).checked_mul((TILE_SIZE as u64)*(TILE_SIZE as u64)).and_then(|n|n.checked_mul(16)).ok_or_else(||bad(CMD,"source coverage validation work overflow"))?).ok_or_else(||bad(CMD,"source coverage work overflow"))?;}
    if peak>MAX_CHECKED_RASTER_BYTES||work>8_000_000_000{return Err(bad(CMD,"aggregate coverage work/resident limit"));}
    if request.native&&!request.raw_storage{plain(native_source(source,active,request,ctl)?,ctl)?;if let Some(old)=&target.selection{plain(old,ctl)?;}}
    poll(ctl)
}
fn operation(mode:SelectionMode)->Operation{match mode{SelectionMode::Add=>Operation::UnionMax,SelectionMode::Subtract=>Operation::SubtractSaturating,_=>Operation::IntersectionMin}}
fn policy()->Policy{Policy{domain:Domain::Unit,unit_overflow:UnitOverflow::Reject,rounding:Rounding::Nearest}}
fn sample_native(format:PixelFormat,old:&[u8],source:&[u8],request:Request,one:&[u8],output:&mut[u8])->Result<()> {
    let mut inverted=[0u8;8];let source=if request.source.invert{let native=inverted.get_mut(..format.bytes_per_pixel()).ok_or_else(||bad(CMD,"native inversion length"))?;coverage::evaluate_sample_into(format,one,source,native,coverage::Params{operation:Operation::SubtractSaturating,policy:policy()}).map_err(|e|bad(CMD,e.to_string()))?;native}else{source};
    if request.mode==SelectionMode::Replace{output.copy_from_slice(source);Ok(())}else{coverage::evaluate_sample_into(format,old,source,output,coverage::Params{operation:operation(request.mode),policy:policy()}).map_err(|e|bad(CMD,e.to_string()))}
}
fn native_result(source:&Surface,old:Option<&Surface>,area:Rect,request:Request,ctl:&Interrupt<'_>)->Result<Surface>{
    let format=source.format();let bpp=format.bytes_per_pixel();let mut one=[0u8;8];let one=one.get_mut(..bpp).ok_or_else(||bad(CMD,"unit word length"))?;photocraft_color::try_write_sample_f64(one,format.sample,0,1.0).map_err(|e|bad(CMD,e.to_string()))?;
    let zero=[0u8;8];let zero=zero.get(..bpp).ok_or_else(||bad(CMD,"zero word length"))?;
    let mut default=[0u8;8];let default=default.get_mut(..bpp).ok_or_else(||bad(CMD,"default word length"))?;
    sample_native(format,old.map_or(zero,Surface::default_bytes),source.default_bytes(),request,one,default)?;
    let mut output=Surface::try_with_default_bytes(format,default).map_err(|e|bad(CMD,e.to_string()))?;
    for coordinate in area.tiles(){poll(ctl)?;let part=coordinate.rect().intersect(&area);let length=photocraft_raster::checked_region_byte_len(part,format).map_err(|e|bad(CMD,e.to_string()))?;
        let mut bytes=Vec::new();bytes.try_reserve_exact(length).map_err(|e|bad(CMD,e.to_string()))?;if bytes.capacity()>length.checked_mul(2).ok_or_else(||bad(CMD,"coverage scratch overflow"))?{return Err(bad(CMD,"coverage scratch capacity"));}bytes.resize(length,0);
        let mut a=[0u8;8];let mut b=[0u8;8];let mut position=0usize;
        for y in part.y0..part.y1{poll(ctl)?;for x in part.x0..part.x1{
            if position.is_multiple_of(1024*bpp){poll(ctl)?;}
            let a=a.get_mut(..bpp).ok_or_else(||bad(CMD,"current coverage word"))?;if let Some(old)=old{old.try_read_pixel_bytes(x,y,a).map_err(|e|failure(e,ctl))?;}else{a.copy_from_slice(zero);}
            let b=b.get_mut(..bpp).ok_or_else(||bad(CMD,"source coverage word"))?;source.try_read_pixel_bytes(x,y,b).map_err(|e|failure(e,ctl))?;
            let end=position.checked_add(bpp).ok_or_else(||bad(CMD,"coverage cursor overflow"))?;sample_native(format,a,b,request,one,bytes.get_mut(position..end).ok_or_else(||bad(CMD,"coverage tile shape"))?)?;position=end;
        }}
        output.try_write_interleaved_with(part,&bytes,ctl).map_err(|e|failure(e,ctl))?;
    }output.try_prune_with(ctl).map_err(|e|failure(e,ctl))?;poll(ctl)?;Ok(output)
}
fn legacy_result(old:Option<&Surface>,plane:&[f32],area:Rect,mode:SelectionMode,ctl:&Interrupt<'_>)->Result<Option<Surface>>{
    let expected=usize::try_from(u64::from(area.width()).checked_mul(u64::from(area.height())).ok_or_else(||bad(CMD,"legacy coverage count"))?).map_err(|_|bad(CMD,"legacy count range"))?;
    if plane.len()!=expected{return Err(bad(CMD,"source plane length mismatch"));}
    if let Some(old)=old{if matches!(old.format().sample,SampleType::U32|SampleType::F64){return Err(bad(CMD,"wide current selection needs nativeCoverage"));}}
    let width=usize::try_from(area.width()).map_err(|_|bad(CMD,"coverage width"))?;let mut output=Surface::new(PixelFormat::GRAY8);let mut nonzero=false;
    for coordinate in area.tiles(){poll(ctl)?;let part=coordinate.rect().intersect(&area);let length=photocraft_raster::checked_region_byte_len(part,PixelFormat::GRAY8).map_err(|e|bad(CMD,e.to_string()))?;
        let mut encoded=Vec::new();encoded.try_reserve_exact(length).map_err(|e|bad(CMD,e.to_string()))?;if encoded.capacity()>length.checked_mul(2).ok_or_else(||bad(CMD,"legacy tile capacity"))?{return Err(bad(CMD,"legacy coverage tile capacity"));}
        for y in part.y0..part.y1{poll(ctl)?;for x in part.x0..part.x1{
            let offset=usize::try_from(i64::from(y)-i64::from(area.y0)).map_err(|_|bad(CMD,"coverage row"))?.checked_mul(width).and_then(|n|usize::try_from(i64::from(x)-i64::from(area.x0)).ok().and_then(|x|n.checked_add(x))).ok_or_else(||bad(CMD,"coverage offset"))?;
            let value=*plane.get(offset).filter(|n|n.is_finite()).ok_or_else(||bad(CMD,"nonfinite or missing source coverage"))?;
            let quantized=(value.clamp(0.0,1.0)*255.0+0.5) as u8;let q=f32::from(quantized)/255.0;
            let previous=if mode==SelectionMode::Replace{0.0}else if let Some(old)=old{let format=old.format();let mut raw=[0u8;40];let bytes=raw.get_mut(..format.bytes_per_pixel()).ok_or_else(||bad(CMD,"current selection format"))?;old.try_read_pixel_bytes(x,y,bytes).map_err(|e|failure(e,ctl))?;photocraft_color::read_sample(bytes,format.sample,0)}else{0.0};
            if !previous.is_finite(){return Err(bad(CMD,"nonfinite current selection"));}
            let result=match mode{SelectionMode::Replace=>value,SelectionMode::Add=>previous.max(q),SelectionMode::Subtract=>(previous-q).max(0.0),SelectionMode::Intersect=>previous.min(q)};
            let code=(result.clamp(0.0,1.0)*255.0+0.5) as u8;nonzero|=code!=0;encoded.push(code);
        }}output.try_write_interleaved_with(part,&encoded,ctl).map_err(|e|failure(e,ctl))?;
    }output.try_prune_with(ctl).map_err(|e|failure(e,ctl))?;poll(ctl)?;Ok(if nonzero{Some(output)}else{None})
}
fn stage(source:&DocumentSnapshot,target:&DocumentSnapshot,active:Option<LayerId>,request:Request,ctl:&Interrupt<'_>)->Result<(Option<Surface>,Option<canonical_selection_storage::LoadProof>)>{
    prepare(source,target,active,request,ctl)?;let area=target.bounds();
    if request.raw_storage{
        let surface=native_source(source,active,request,ctl)?;
        if source.xcf_state.is_some()||target.xcf_state.is_some(){
            let ChanRef::Alpha(index)=request.source.channel else{return Err(bad(CMD,"canonical raw source must be saved channel"));};
            let id=source.channels.get(index).ok_or_else(||bad(CMD,"source channel disappeared"))?.id;
            let proof=canonical_selection_storage::prepare(source,target,id,ctl)?;
            let output=super::selection_storage_controlled::copy_canonical_load(source,target,surface,surface.format(),&proof,CMD,ctl)?;
            Ok((Some(output),Some(proof)))
        }else{Ok((Some(super::selection_storage_controlled::copy(source,target,surface,surface.format(),CMD,ctl)?),None))}
    }
    else if request.native{Ok((Some(native_result(native_source(source,active,request,ctl)?,target.selection.as_ref(),area,request,ctl)?),None))}
    else{let mut planes=source_from_snapshot_with_interrupt(source,active,request.source,CMD,ctl)?;if planes.len()!=1{return Err(bad(CMD,"source must yield one coverage plane"));}let plane=planes.pop().ok_or_else(||bad(CMD,"missing source plane"))?;legacy_result(target.selection.as_ref(),&plane,area,request.mode,ctl).map(|selection|(selection,None))}
}
pub(super) fn run(session:&mut Session,value:&Value)->Result<Value>{
    if value.get("storedSource").is_some() { return stored_load_publication::run(session,value); }
    let request=request(session,value,false)?;let target=session.active().ok_or(EngineError::NoDocument)?;let original=target.doc.clone();let revision=target.revision;let weak=original.downgrade();
    let source_state=session.documents().get(request.source_index).ok_or_else(||bad(CMD,"source disappeared"))?;let source=source_state.doc.clone();let source_revision=source_state.revision;let source_weak=source.downgrade();let active=source_state.active_layer;
    let target_work=original.clone();let source_work=source.clone();let label=match request.source.channel{ChanRef::Transparency=>"Load Transparency",ChanRef::LayerMask=>"Load Layer Mask",_=>"Load Selection"};
    crate::jobs::run(session,label,true,move|ctx|ctx.stage(0.0,0.95,label,|ctl|{let selection=stage(&source_work,&target_work,active,request,ctl)?;Ok((selection,ctx.clone()))}),move|session,((selection,proof),ctx)|{
        let current=session.active().ok_or(EngineError::NoDocument)?;
        let source_current=session.documents().iter().find(|state|state.doc.id==source.id).ok_or_else(||bad(CMD,"source closed while preparing selection"))?;
        if current.doc.id!=original.id||current.revision!=revision||!weak.is_current(&current.doc)||!current.doc.shares_state(&original)||current.floating.is_some()||current.coalesce.is_some()
            ||source_current.revision!=source_revision||!source_weak.is_current(&source_current.doc)||!source_current.doc.shares_state(&source)||source_current.floating.is_some()||source_current.coalesce.is_some()
            ||(request.source.layer.is_none()&&matches!(request.source.channel,ChanRef::LayerMask|ChanRef::Transparency|ChanRef::VectorMask)&&source_current.active_layer!=active)
            ||session.command_has_job_on(current.doc.id)||session.command_has_job_on(source_current.doc.id)
            ||session.coalesce_request.is_some()||session.color_restrict.is_some(){return Err(bad(CMD,"source or target changed while preparing selection"));}
        ctx.stage(0.95,1.0,label,|ctl|{poll(ctl)?;let selected=selection.is_some();if proof.as_ref().is_none_or(|proof|!proof.changes_bounds())&&original.selection.as_ref()==selection.as_ref(){return Ok(json!({"selected":selected}));}
            session.edit_with_interrupt(label,ctl,move|doc,_|{poll(ctl)?;doc.selection=selection;if let Some(proof)=proof{proof.apply_bounds(doc,ctl)?;}Ok(())})?;Ok(json!({"selected":selected}))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(mode:SelectionMode)->Request {Request{source_index:0,source:SnapshotSource{layer:None,channel:ChanRef::Alpha(0),gray:true,invert:false},mode,native:true,raw_storage:false}}
    #[test]
    fn native_f64_low_words_and_signed_default_survive_replace_and_ties() {
        let format=PixelFormat::new(ColorMode::Grayscale,SampleType::F64,false);let area=Rect::new(0,0,2,1);
        let mut source=Surface::try_with_default_bytes(format,&(-0.0f64).to_ne_bytes()).unwrap();
        let values=[f64::from_bits(0.5f64.to_bits()+1),-0.0];let bytes=values.into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved_with(area,&bytes,&Interrupt::NONE).unwrap();plain(&source,&Interrupt::NONE).unwrap();
        let output=native_result(&source,None,area,req(SelectionMode::Replace),&Interrupt::NONE).unwrap();
        assert_eq!(output.default_bytes(),&(-0.0f64).to_ne_bytes());assert_eq!(output.try_read_interleaved(area).unwrap(),bytes);
        let minus=Surface::try_with_default_bytes(format,&(-0.0f64).to_ne_bytes()).unwrap();let plus=Surface::try_with_default_bytes(format,&0.0f64.to_ne_bytes()).unwrap();
        for mode in [SelectionMode::Add,SelectionMode::Intersect]{let output=native_result(&plus,Some(&minus),area,req(mode),&Interrupt::NONE).unwrap();assert_eq!(output.default_bytes(),&(-0.0f64).to_ne_bytes());assert_eq!(output.try_read_interleaved(area).unwrap(),[(-0.0f64).to_ne_bytes(),(-0.0f64).to_ne_bytes()].concat());}
    }
    #[test]
    fn native_u32_subtraction_keeps_low_codes_without_f32_ingress() {
        let format=PixelFormat::new(ColorMode::Grayscale,SampleType::U32,false);let area=Rect::new(0,0,1,1);
        let a=Surface::try_with_default_bytes(format,&0x80000001u32.to_ne_bytes()).unwrap();let b=Surface::try_with_default_bytes(format,&1u32.to_ne_bytes()).unwrap();
        let output=native_result(&b,Some(&a),area,req(SelectionMode::Subtract),&Interrupt::NONE).unwrap();assert_eq!(output.default_bytes(),&0x80000000u32.to_ne_bytes());assert_eq!(output.try_read_interleaved(area).unwrap(),0x80000000u32.to_ne_bytes());
    }
    #[test]
    fn legacy_tile_combine_matches_existing_quantization_and_all_modes() {
        let area=Rect::new(0,0,3,2);let source=[0.001,0.2,0.5,0.9,1.1,-0.2];
        let old=Surface::try_from_interleaved(PixelFormat::GRAY8,area,&[1,64,128,192,255,0]).unwrap();
        for mode in [SelectionMode::Replace,SelectionMode::Add,SelectionMode::Subtract,SelectionMode::Intersect]{
            assert_eq!(legacy_result(Some(&old),&source,area,mode,&Interrupt::NONE).unwrap(),photocraft_algo::selection::combine(Some(&old),&source,area,mode));
        }
    }
    #[test]
    fn native_dispatch_noop_cancel_history_and_source_are_preserved() {
        let mut doc=Document::new("Own coverage",photocraft_doc::Size::new(2,1),ColorMode::Rgb,SampleType::F64);
        let format=PixelFormat::new(ColorMode::Grayscale,SampleType::F64,false);let surface=Surface::try_with_default_bytes(format,&(-0.0f64).to_ne_bytes()).unwrap();
        doc.channels.push(photocraft_doc::AlphaChannel::new("Own native",surface));
        let mut session=Session::new();session.add_document(doc,None).unwrap();
        let value=json!({"channel":0,"nativeCoverage":true,"coverageContext":"unit"});let request=request(&session,&value,false).unwrap();
        let snapshot=session.active().unwrap().doc.clone();assert!(matches!(stage(&snapshot,&snapshot,None,request,&Interrupt::cancel_only(&||true)),Err(EngineError::Cancelled)));
        run(&mut session,&value).unwrap();assert!(session.active().unwrap().doc.selection.is_some());
        assert_eq!(session.active().unwrap().doc.channels,snapshot.channels);assert!(session.undo());assert!(session.redo());
        session.edit("Owned redo seed",|doc,_|{doc.name.push_str(" changed");Ok(())}).unwrap();assert!(session.undo());let revision=session.active().unwrap().revision;
        run(&mut session,&value).unwrap();assert_eq!(session.active().unwrap().revision,revision);assert!(session.redo());
        for bad in [json!({"channel":0,"nativeCoverage":true}),json!({"channel":0,"nativeCoverage":true,"coverageContext":"hdr"}),json!({"channel":0,"operation":{}}),json!({"channel":0,"layer":"00"})]{assert!(dispatch_preflight(&session,&bad,false).is_err());}
    }
}
