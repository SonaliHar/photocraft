//! Actual job-token consumer of the existing ordinary f32 paint equations.
//! No native/source operation mapping is inferred. Controlled assigned/default
//! CMYK preparation precedes pixels; builtin initialization retains outer polls.
use super::*;
use photocraft_paint::stroke_controlled::{render_stroke_with_interrupt,render_stroke_passes_with_interrupt};
use std::sync::Arc;
fn target_lock(doc:&photocraft_doc::Document,id:Option<photocraft_doc::LayerId>,p:&Value)->Result<()>{
    match crate::channel_cmds::target_of(p){crate::channel_cmds::Target::Mask=>{let id=id.ok_or_else(||fail("mask owner missing"))?;let layer=doc.layer(id).ok_or_else(||fail("mask owner missing"))?;if doc.effective_locks(id).all{return Err(fail(format!("Could not complete your request because the layer \"{}\" is locked",layer.name)));}crate::xcf_item_locks::require_layer_mask_content(doc,id)},crate::channel_cmds::Target::Pixels=>crate::commands::check_pixels_unlocked(doc,id.ok_or_else(||fail("pixel owner missing"))?),crate::channel_cmds::Target::Alpha(i)=>crate::xcf_item_locks::require_channel_content(doc,doc.channels.get(i).ok_or_else(||fail("channel missing"))?.id),crate::channel_cmds::Target::QuickMask=>Ok(())}
}
// Untouched COW tile identity avoids a whole-surface byte scan for a tiny stroke.
// Header/map lookups remain polled and charged, mismatching tiles compare raw.
fn comparison_work(a:&Surface,b:&Surface,c:&Interrupt<'_>)->Result<u64>{
    let mut work=mul(add(a.tile_count() as u64,b.tile_count() as u64)?,256)?;
    for(position,tile)in a.tiles(){poll(c)?;if b.tile(*position).is_none_or(|other|!Arc::ptr_eq(tile,other)){work=add(work,mul(tile.bytes().len() as u64,8)?)?;}}
    for(position,tile)in b.tiles(){poll(c)?;if a.tile(*position).is_none(){work=add(work,mul(tile.bytes().len() as u64,8)?)?;}}Ok(work)
}
fn unchanged(a:&Surface,b:&Surface,c:&Interrupt<'_>)->Result<bool>{
    if a.format()!=b.format()||a.default_bytes()!=b.default_bytes(){return Ok(false);}
    if a.default_bytes().is_empty(){return Err(fail("ordinary empty native default"));}
    for(position,tile)in a.tiles(){poll(c)?;if let Some(other)=b.tile(*position){if Arc::ptr_eq(tile,other){continue;}if tile.bytes().len()!=other.bytes().len(){return Ok(false);}for(a,b)in tile.bytes().chunks(4096).zip(other.bytes().chunks(4096)){poll(c)?;if a!=b{return Ok(false);}}}else{for chunk in tile.bytes().chunks(a.default_bytes().len()*256){poll(c)?;if chunk.chunks_exact(a.default_bytes().len()).any(|px|px!=a.default_bytes()){return Ok(false);}}}}
    for(position,tile)in b.tiles(){poll(c)?;if a.tile(*position).is_none(){for chunk in tile.bytes().chunks(b.default_bytes().len()*256){poll(c)?;if chunk.chunks_exact(b.default_bytes().len()).any(|px|px!=b.default_bytes()){return Ok(false);}}}}poll(c)?;Ok(true)
}
fn finish(input:Input,output:Surface,damage:Rect,lock:bool,from_live:bool,c:&Interrupt<'_>)->Result<Candidate>{
    let source=crate::channel_cmds::read_target_surface(&input.snapshot,input.id,&input.params)?;
    if comparison_work(source,&output,c)?>StrokeLimits::default().work_units/16{return Err(fail("ordinary raw equality work limit"));}
    let changed=!unchanged(source,&output,c)?;let damage=if changed{damage}else{Rect::EMPTY};
    let response=if from_live{json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()],"fromLive":true})}else{json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()]})};poll(c)?;Ok(Candidate{input,output,damage,response,lock,changed})
}
// Reserve sequential provider peaks against already retained document/tips.
// Work is never refunded: default preparation is partitioned before the cold
// custom factory, whose requirements reserve its entire supplied envelope.
fn cmyk_scope(doc:&photocraft_doc::Document,needs_default:bool,available:u64,c:&Interrupt<'_>)->Result<(Option<Arc<photocraft_color::convert::CmykSpace>>,u64)> {
    use photocraft_color::convert::{CmykSpace,default_cmyk_requirements_with_control,warm_default_cmyk_with_control};
    use photocraft_cms::control::{Limits,Error};
    let cancelled=||c.cancelled();
    let map=|e:Error|match e{Error::Cancelled=>EngineError::Cancelled,e=>fail(e.to_string())};
    let maximum=Limits::default();
    let phase=Limits{resident_bytes:usize::try_from(available/3).map_err(|_|fail("ordinary CMYK headroom"))?.min(maximum.resident_bytes),work_units:(StrokeLimits::default().work_units/8).min(maximum.work_units)};
    poll(c)?;
    let defaults=if needs_default{Some(default_cmyk_requirements_with_control(phase,&cancelled).map_err(map)?)}else{None};
    let default_work=defaults.as_ref().map_or(Ok(0),|r|mul(r.work_units,2))?;
    let custom=Limits{resident_bytes:phase.resident_bytes,work_units:phase.work_units.checked_sub(default_work).ok_or_else(||fail("ordinary default/custom CMYK work"))?/3};
    let required=CmykSpace::profile_requirements_with_control(doc.icc_profile.as_ref(),custom,&cancelled).map_err(map)?;
    if required.peak_resident_bytes>custom.resident_bytes||required.retained_bytes>required.peak_resident_bytes||required.work_units>custom.work_units{return Err(fail("ordinary custom CMYK requirements"));}
    let space=CmykSpace::for_profile_with_control(doc.icc_profile.as_ref(),custom,&cancelled).map_err(map)?;
    let mut held=required.peak_resident_bytes as u64;
    if space.is_none(){if let Some(required)=defaults{
        if required.peak_resident_bytes>phase.resident_bytes||required.retained_bytes>required.peak_resident_bytes{return Err(fail("ordinary default CMYK requirements"));}
        warm_default_cmyk_with_control(Limits{resident_bytes:phase.resident_bytes,work_units:required.work_units},&cancelled).map_err(map)?;
        // Custom None drops private preparation. Its reserved peak and paired
        // default peak are sequential; use their maximum, not two full envelopes.
        held=held.max(required.peak_resident_bytes as u64);
    }}
    poll(c)?;Ok((space,held))
}
pub(super) fn prepare(mut input:Input,lock:bool,c:&Interrupt<'_>)->Result<Candidate>{
    poll(c)?;let doc=&input.snapshot;target_lock(doc,input.id,&input.params)?;
    let source=crate::channel_cmds::read_target_surface(doc,input.id,&input.params)?;source.try_validate_tile_origins_with(c).map_err(raster)?;
    if let Some(offer)=input.offer.take(){if offer.matches_captured_with_interrupt((doc.id,input.revision),&input.cmd,&input.params,&input.points,c)?{
        if offer.surface.format()!=source.format(){return Err(fail("ordinary live offer format changed"));}offer.surface.try_validate_tile_origins_with(c).map_err(raster)?;
        if i64::from(offer.bounds.x1)-i64::from(offer.bounds.x0)>i64::from(i32::MAX)||i64::from(offer.bounds.y1)-i64::from(offer.bounds.y0)>i64::from(i32::MAX){return Err(fail("ordinary offer damage bounds"));}
        return finish(input,offer.surface,offer.bounds,lock,true,c);
    }}
    let standard=StrokeLimits::default();let native=photocraft_io::NativeSaveLimits::default().preparation;
    let payload=photocraft_io::retained_native_payload_bytes(doc,native,c).map_err(|e|if c.cancelled(){EngineError::Cancelled}else{fail(e.to_string())})? as u64;
    let model=photocraft_io::retained_native_model_bytes(doc,native,c).map_err(|e|if c.cancelled(){EngineError::Cancelled}else{fail(e.to_string())})? as u64;
    let raster=DocumentSnapshot::resident_raster_footprint_of(doc,c)?;
    let current_brush=StrokeRenderer::new_requirements(&input.brush,Some(source.format()),input.zoom,standard,c).map_err(paint)?;
    let retained=add(input.capture_bytes,add(payload,add(mul(model,4)?,add(mul(raster.retained_bytes,2)?,add(mul(raster.clone_metadata_bytes,4)?,add(current_brush.retained_bytes,1<<20)?)?)?)?)?)?;
    let available=standard.resident_bytes.checked_sub(retained).ok_or_else(||fail("ordinary document/brush headroom"))?;
    let mirrors=match &input.axis{Some(axis)=>stroke_controlled::prepare_reflected_passes(axis,&input.points,StrokeLimits{resident_bytes:available,work_units:standard.work_units/8},c)?,None=>Vec::new()};
    let mut reflected=mul(mirrors.capacity() as u64,size_of::<Vec<StrokePoint>>() as u64)?;for pass in &mirrors{poll(c)?;reflected=add(reflected,mul(pass.capacity() as u64,size_of::<StrokePoint>() as u64)?)?;}
    let available=available.checked_sub(reflected).ok_or_else(||fail("ordinary reflected point headroom"))?;
    let mut passes=[input.points.as_slice();4];let mut count=1usize;
    for pass in &mirrors{poll(c)?;if pass.len()!=input.points.len(){return Err(fail("ordinary symmetry point count"));}let mut distinct=false;for(a,b)in input.points.iter().zip(pass){poll(c)?;if (a.x-b.x).abs()>1.0e-6||(a.y-b.y).abs()>1.0e-6{distinct=true;break;}}if distinct{*passes.get_mut(count).ok_or_else(||fail("ordinary symmetry pass count"))?=pass;count+=1;}}
    let mut brush=input.brush.clone();erase_locked(&mut brush,lock,input.background);
    // Preserve the original assigned scope and compatibility None. Only actual
    // CMYK pixel work can need the default pair; RGB does not warm it.
    let (cmyk,cmyk_retained)=cmyk_scope(doc,source.format().mode==photocraft_color::ColorMode::Cmyk,available,c)?;
    let available=available.checked_sub(cmyk_retained).ok_or_else(||fail("ordinary CMYK/render simultaneous headroom"))?;
    let mut output=source.clone();poll(c)?;
    let limits=StrokeLimits{resident_bytes:available,work_units:standard.work_units/2};
    let damage=photocraft_color::convert::with_cmyk_space(cmyk.as_ref(),||{
        // Auto Erase reads PRE inside the same assigned CMYK scope as legacy.
        poll(c).map_err(|_|StrokeError::Cancelled)?;
        if input.auto_erase{apply_auto_erase(&mut brush,source,input.points.first(),input.brush.color,input.background);}
        poll(c).map_err(|_|StrokeError::Cancelled)?;
        if count==1{render_stroke_with_interrupt(&mut output,&brush,&input.points,doc.selection.as_ref(),lock,input.zoom,limits,c)}else{render_stroke_passes_with_interrupt(&mut output,&brush,passes.get(..count).ok_or(StrokeError::Limit)?,doc.selection.as_ref(),lock,input.zoom,limits,c)}
    }).map_err(paint)?;poll(c)?;finish(input,output,damage,lock,false,c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode,SampleType};
    fn fixture(mode:ColorMode,sample:SampleType)->Session {
        let mut doc=photocraft_doc::Document::new("Ordinary stroke",photocraft_doc::Size::new(12,12),mode,sample);
        let values:&[f32]=match mode{ColorMode::Grayscale=>&[0.25,0.8],ColorMode::Cmyk=>&[0.25,0.5,0.75,0.1,0.8],_=>&[0.25,0.5,0.75,0.8]};
        doc.layers.push(photocraft_doc::Layer::new("Pixels",LayerContent::Raster(Surface::with_default(doc.pixel_format(),values))));
        let mut session=Session::new();session.add_document(doc,None).unwrap();session.tools.brush.pressure_size=false;session.tools.brush.smoothing.enabled=false;session
    }
    fn params()->Value {json!({"points":[[2,2,1],[8,7,0.7]],"color":[0.7,0.2,0.4,0.6],"size":3,"seed":31})}
    // Independent old public renderer, not a second copy of the controlled math.
    #[test]
    fn registered_recipe_matches_ordinary_renderer_across_depths_models_and_pencil() {
        for mode in [ColorMode::Rgb,ColorMode::Grayscale,ColorMode::Cmyk] {
            for sample in [SampleType::U8,SampleType::U16,SampleType::F32] {
                for cmd in ["paint.stroke","paint.pencil"] {
                    let mut session=fixture(mode,sample);let p=params();
                    let brush=if cmd=="paint.pencil"{pencil_brush(&session,&p).unwrap()}else{with_blend_mode(resolve_brush(&session,&p,cmd).unwrap(),&p)};
                    let(id,mut brush,zoom)=stroke_target(&session,&p,brush).unwrap();let points=parse_points(&p,cmd).unwrap();
                    let doc=&session.active().unwrap().doc;let pre=crate::channel_cmds::read_target_surface(doc,id,&p).unwrap();let mut expected=pre.clone();
                    erase_locked(&mut brush,false,session.tools.background);
                    let cmyk=doc_cmyk_space(&session);
                    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(),||photocraft_paint::render_stroke(&mut expected,&brush,&points,doc.selection.as_ref(),false,zoom));
                    let before=doc.clone();let revision=session.active().unwrap().revision;
                    session.execute(cmd,p).unwrap();let actual=session.active().unwrap().doc.layer(id.unwrap()).unwrap().surface().unwrap();
                    assert!(unchanged(&expected,actual,&Interrupt::NONE).unwrap());assert_eq!(session.active().unwrap().revision,revision+1);
                    let after=session.active().unwrap().doc.clone();assert!(session.undo());assert_eq!(&*session.active().unwrap().doc,&*before);assert!(session.redo());assert_eq!(&*session.active().unwrap().doc,&*after);
                }
            }
        }
    }
    #[test]
    fn controlled_cmyk_fallback_retains_none_and_cancellation_is_not_fallback() {
        let mut session=fixture(ColorMode::Cmyk,SampleType::F32);
        session.edit("Invalid assigned profile",|doc,_|{doc.icc_profile=Some(Arc::new(vec![0;140]));Ok(())}).unwrap();
        let doc=&session.active().unwrap().doc;let before=doc.clone();
        let legacy=photocraft_color::convert::CmykSpace::for_profile(doc.icc_profile.as_ref());assert!(legacy.is_none());
        let(scope,_)=cmyk_scope(doc,true,StrokeLimits::default().resident_bytes,&Interrupt::NONE).unwrap();assert!(scope.is_none());
        let cmyk=[0.25,0.5,0.75,0.1];
        let expected=photocraft_color::convert::with_cmyk_space(legacy.as_ref(),||photocraft_color::convert::cmyk_to_rgb(cmyk));
        let actual=photocraft_color::convert::with_cmyk_space(scope.as_ref(),||photocraft_color::convert::cmyk_to_rgb(cmyk));assert_eq!(actual.map(f32::to_bits),expected.map(f32::to_bits));
        let cancel=||true;assert!(matches!(cmyk_scope(doc,true,StrokeLimits::default().resident_bytes,&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));assert!(cmyk_scope(doc,true,1,&Interrupt::NONE).is_err());assert_eq!(&**doc,&*before);
    }
    #[test]
    fn ordinary_moved_preview_and_cancelled_job_preserve_real_redo() {
        for sample in [SampleType::U8,SampleType::U16,SampleType::F32] {
            let mut session=fixture(ColorMode::Rgb,sample);let p=params();let live=LiveStroke::begin(&session,&p).unwrap();
            let expected=crate::channel_cmds::read_target_surface(&live.doc,live.layer,&live.params).unwrap().clone();
            session.offer_live_stroke(live).unwrap();let response=session.execute("paint.stroke",p.clone()).unwrap();assert_eq!(response.get("fromLive"),Some(&json!(true)));
            let actual=session.active().unwrap().doc.layer(session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();assert!(unchanged(&expected,actual,&Interrupt::NONE).unwrap());
            assert!(session.undo());let before=session.active().unwrap().doc.clone();let ctx=crate::jobs::JobCtx::default();ctx.cancel();session.set_inline_job_ctx(Some(ctx));
            assert!(matches!(session.execute("paint.stroke",p.clone()),Err(EngineError::Cancelled)));assert!(session.active().unwrap().doc.shares_state(&before));assert!(session.active().unwrap().history.can_redo());
            session.set_inline_job_ctx(None);let mut noop=p;noop["opacity"]=json!(0);session.execute("paint.stroke",noop).unwrap();assert!(session.active().unwrap().doc.shares_state(&before));assert!(session.active().unwrap().history.can_redo());
        }
    }
}
