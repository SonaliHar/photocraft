//! Controlled native stored-color Exposure. Alpha/source metadata are copied raw.
use crate::{EngineError,Result,Session,channel_cmds::ChannelTarget};
use photocraft_doc::{Document,LayerContent,LayerId,item_identity::ItemRef};
use photocraft_color::{SampleType,ColorMode};
use photocraft_geom::{Rect,TileCoord,TILE_SIZE};
use photocraft_raster::{Surface,Interrupt,MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;
use std::sync::Arc;
const ID:&str="image.adjustments.exposure";
fn bad(m:impl Into<String>)->EngineError{EngineError::BadParams{cmd:ID.into(),msg:m.into()}}
fn poll(c:&Interrupt<'_>)->Result<()>{c.check().map_err(|_|EngineError::Cancelled)}
fn error(c:&Interrupt<'_>,e:impl std::fmt::Display)->EngineError{if c.cancelled(){EngineError::Cancelled}else{bad(e.to_string())}}
fn decimal(v:&Value)->Option<u64>{let s=v.as_str()?;if s.is_empty()||s.len()>20||(s.len()>1&&s.starts_with('0'))||!s.bytes().all(|b|b.is_ascii_digit()){return None}s.parse().ok()}
struct Request{layer:LayerId,opacity:f64,multiplier:f64,offset:f64,inverse_gamma:f64}
fn working(doc:&Document,control:&Interrupt<'_>)->Result<()>{
    crate::native_linear_context::validate(doc,control).map_err(|message|error(control,message))
}
fn target<'a>(doc:&'a Document,id:LayerId,c:&Interrupt<'_>)->Result<(&'a Surface,Rect)>{
    working(doc,c)?;let mut found=0usize;
    photocraft_doc::resident_structure::visit_layers_with(doc,c,|layer|{if layer.id==id{found=found.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;}Ok(true)})?;
    if id.0==0||found!=1{return Err(bad("missing or ambiguous raster owner"));}
    crate::commands::check_pixels_unlocked(doc,id)?;crate::xcf_item_locks::transparency_locked(doc,id)?;
    let layer=doc.layer(id).ok_or(EngineError::NoLayer(id))?;let LayerContent::Raster(source)=&layer.content else{return Err(bad("native Exposure requires a plain resident raster"))};
    let format=source.format();
    if !matches!(format.mode,ColorMode::Rgb|ColorMode::Grayscale)||!matches!(format.sample,SampleType::U32|SampleType::F64)||format.mode!=doc.mode||format.sample!=doc.depth
        ||doc.color_table.is_some()||layer.indexed_source.is_some()||layer.video.is_some()||layer.import_compositing.is_some(){return Err(bad("native raster color/geometry context unsupported"));}
    if source.tile_count()>100_000{return Err(bad("native raster tile limit"));}
    let area=source.try_content_bounds_with_interrupt(c).map_err(|e|error(c,e))?;
    if let Some(state)=&doc.xcf_state{
        if state.items.len()>10_000||state.source_items.len()>10_000||state.filter_stacks.len()>10_000||state.editable_indexed.len()>10_000{return Err(bad("current state limit"));}
        if state.source_items.iter().any(|i|i.owner==ItemRef::Layer(id))||state.filter_stacks.iter().any(|i|i.owner==ItemRef::Layer(id))||state.editable_indexed.iter().any(|i|i.owner==id){return Err(bad("source/indexed/filter carrier exposure is not qualified"));}
        let mut items=state.items.iter().filter(|i|i.item==ItemRef::Layer(id));let item=items.next().ok_or_else(||bad("missing current typed raster"))?;
        if items.next().is_some()||item.composition.is_some(){return Err(bad("ambiguous or interpreted current raster"));}
        let extent=item.raster_bounds.ok_or_else(||bad("missing current raster extent"))?;
        if !area.is_empty()&&area.intersect(&extent)!=area{return Err(bad("retained pixels exceed declared current extent"));}
    }
    Ok((source,area))
}
fn context(s:&Session)->Result<()>{let st=s.active().ok_or(EngineError::NoDocument)?;
    if st.revision==u64::MAX||st.doc.accounted().is_some()||st.floating.is_some()||st.coalesce.is_some()||s.has_jobs()||s.command_has_pending_job()||s.coalesce_request.is_some()||s.color_restrict.is_some()
        ||st.doc.xcf_state.as_ref().is_some_and(|x|x.floating.is_some()){return Err(bad("finish transient work before native raster Exposure"));}Ok(())}
fn request(s:&Session,p:&Value)->Result<Option<Request>>{
    let st=s.active().ok_or(EngineError::NoDocument)?;
    if let Some(t)=p.get("target") {if t.as_str()!=Some("pixels"){return Ok(None);}}
    else if matches!(st.channel_view.target,ChannelTarget::Alpha(_)|ChannelTarget::LayerMask|ChannelTarget::VectorMask)||st.doc.quick_mask.is_some()||crate::mask_view_cmds::current(st).is_some(){return Ok(None);}
    if let Some(v)=p.get("layer"){if v.as_u64().is_none_or(|v|v==0){return Err(bad("layer must be a nonzero unsigned integer"));}}
    let layer=crate::commands::layer_param(s,p)?;let current=st.doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
    if current.surface().is_none_or(|v|!matches!(v.format().sample,SampleType::U32|SampleType::F64)){return Ok(None);}
    if matches!(st.channel_view.target,ChannelTarget::Color(_)){return Err(bad("native Exposure requires all color channels"));}
    if !p.is_null(){let o=p.as_object().filter(|o|o.len()<=9).ok_or_else(||bad("bounded native Exposure parameters required"))?;
        if o.keys().any(|k|!matches!(k.as_str(),"layer"|"target"|"documentId"|"expectedRevision"|"opacity"|"__kind"|"exposure"|"offset"|"gamma")){return Err(bad("unknown native raster Exposure parameter"));}}
    if p.get("__kind").is_some_and(|v|v.as_str()!=Some("exposure")){return Err(bad("native raster route requires Exposure"));}
    match(p.get("documentId"),p.get("expectedRevision")){(None,None)=>{},(Some(d),Some(r))if st.doc.id.0!=0&&decimal(d)==Some(st.doc.id.0)&&decimal(r)==Some(st.revision)=>{},_=>return Err(bad("captured target changed or incomplete")),}
    context(s)?;target(&st.doc,layer,&Interrupt::NONE)?;
    let percent=match p.get("opacity"){None|Some(Value::Null)=>100.0,Some(v)=>v.as_f64().filter(|v|v.is_finite()).ok_or_else(||bad("opacity must be finite"))?.clamp(0.0,100.0)};
    let parsed=crate::adjust_params::from_params("exposure",p,None,st.doc.mode)?;
    let photocraft_doc::Adjustment::Exposure{exposure,offset,gamma}=parsed else{return Err(bad("Exposure parameter mismatch"))};
    // Parameter arithmetic has exactly the existing typed f32 semantics. Only
    // these small configuration scalars are promoted; source words never use f32.
    let multiplier=f64::from(2f32.powf(exposure));let inverse_gamma=f64::from(1.0f32/gamma.max(0.01));
    if !multiplier.is_finite()||!inverse_gamma.is_finite(){return Err(bad("Exposure parameter arithmetic domain"));}
    Ok(Some(Request{layer,opacity:percent/100.0,multiplier,offset:f64::from(offset),inverse_gamma}))
}
pub(crate) fn dispatch_preflight(s:&Session,id:&str,p:&Value,_background:bool)->Result<bool>{if id!=ID{return Ok(false)}request(s,p).map(|r|r.is_some())}
fn cells(r:Rect)->Result<u64>{if r.is_empty(){return Ok(0)}let a=TileCoord::containing(r.x0,r.y0);let b=TileCoord::containing(r.x1.checked_sub(1).ok_or_else(||bad("extent"))?,r.y1.checked_sub(1).ok_or_else(||bad("extent"))?);
    u64::try_from(i64::from(b.tx)-i64::from(a.tx)+1).ok().and_then(|w|u64::try_from(i64::from(b.ty)-i64::from(a.ty)+1).ok().and_then(|h|w.checked_mul(h))).ok_or_else(||bad("tile span overflow"))}
fn evaluate(before:f64,r:&Request,weight:f64)->Result<f64>{
    if !weight.is_finite()||!(0.0..=1.0).contains(&weight){return Err(bad("Exposure coverage is not finite unit"));}
    if weight==0.0{return Ok(before)}
    if !before.is_finite()||!(0.0..=1.0).contains(&before){return Err(bad("native Exposure requires finite unit colors; F64 does not imply HDR"));}
    // Explicit identity transfer, using the original NON-HDR clamp sequence.
    // This is binary64 native math, not an exact rational transcendental claim.
    let decoded=before.max(0.0);let scaled=decoded*r.multiplier;let shifted=scaled+r.offset;
    let linear=shifted.max(0.0);let corrected=if r.inverse_gamma==1.0{linear}else{linear.powf(r.inverse_gamma)};
    let mapped=corrected.max(0.0).clamp(0.0,1.0);
    if !corrected.is_finite(){return Err(bad("Exposure arithmetic is nonfinite"));}
    let result=if weight==1.0{mapped}else{let original=before*(1.0-weight);let edited=mapped*weight;original+edited};
    if !result.is_finite()||!(0.0..=1.0).contains(&result){return Err(bad("Exposure output domain"));}Ok(result)
}
fn prepare(doc:&Document,r:&Request,c:&Interrupt<'_>)->Result<Option<Surface>>{
    let(source,area)=target(doc,r.layer,c)?;let fmt=source.format();let bpp=fmt.bytes_per_pixel();let colors=fmt.mode.color_channels();
    let limits=photocraft_io::NativeSaveLimits::default().preparation;
    let payload=photocraft_io::retained_native_payload_bytes(doc,limits,c).map_err(|e|error(c,e))? as u64;let model=photocraft_io::retained_native_model_bytes(doc,limits,c).map_err(|e|error(c,e))? as u64;let footprint=crate::DocumentSnapshot::resident_raster_footprint_of(doc,c)?;
    let count=cells(area)?;let pixels=count.checked_mul(TILE_SIZE as u64).and_then(|n|n.checked_mul(TILE_SIZE as u64)).ok_or_else(||bad("Exposure extent overflow"))?;
    let output=pixels.checked_mul(bpp as u64).and_then(|n|n.checked_add(count.checked_mul(2048)?)).ok_or_else(||bad("Exposure output overflow"))?;
    let peak=payload.checked_add(model.checked_mul(6).ok_or_else(||bad("model overflow"))?).and_then(|n|n.checked_add(footprint.retained_bytes.checked_mul(2)?)).and_then(|n|n.checked_add(footprint.clone_metadata_bytes.checked_mul(6)?)).and_then(|n|n.checked_add(output)).and_then(|n|n.checked_add(2<<20)).ok_or_else(||bad("Exposure staging overflow"))?;
    let work=pixels.checked_mul((colors as u64+4)*128).and_then(|n|n.checked_add((source.tile_count() as u64).checked_add(doc.selection.as_ref().map_or(0,|s|s.tile_count() as u64))?.checked_mul(TILE_SIZE as u64)?.checked_mul(TILE_SIZE as u64)?.checked_mul(64)?)).and_then(|n|n.checked_add(model.checked_add(payload)?.checked_mul(16)?)).ok_or_else(||bad("Exposure work overflow"))?;
    if peak>MAX_CHECKED_RASTER_BYTES||work>8_000_000_000{return Err(bad("Exposure retained/output/work limit"));}
    if let Some(selection)=&doc.selection{if selection.format().mode!=ColorMode::Grayscale||selection.format().alpha{return Err(bad("selection requires Gray coverage without alpha"));}selection.try_validate_tile_origins_with(c).map_err(|e|error(c,e))?;}
    if area.is_empty()||r.opacity==0.0{return Ok(None);}
    let mut out=Surface::try_with_default_bytes(fmt,source.default_bytes()).map_err(|e|error(c,e))?;for(cell,tile)in source.tiles(){poll(c)?;out.put_tiles(std::iter::once((*cell,Arc::clone(tile))));}
    let mut any=false;
    for cell in area.tiles(){poll(c)?;let part=cell.rect().intersect(&area);if part.is_empty(){continue;}
        let mut raw=source.try_read_interleaved_with(part,c).map_err(|e|error(c,e))?;let width=usize::try_from(part.width()).map_err(|_|bad("Exposure row"))?;let mut changed=false;
        for(i,pixel)in raw.chunks_exact_mut(bpp).enumerate(){if i%256==0{poll(c)?;}let x=i32::try_from(i64::from(part.x0)+(i%width) as i64).map_err(|_|bad("Exposure x"))?;let y=i32::try_from(i64::from(part.y0)+(i/width) as i64).map_err(|_|bad("Exposure y"))?;
            let selected=if let Some(selection)=&doc.selection{let mut value=[0f64];selection.try_read_pixel_f64(x,y,&mut value).map_err(|e|error(c,e))?;value[0]}else{1.0};
            if !selected.is_finite()||!(0.0..=1.0).contains(&selected){return Err(bad("selection is not finite unit"));}
            let weight=selected*r.opacity;if weight==0.0{continue;}
            if crate::native_adjustment_alpha::skips(pixel,fmt).map_err(bad)?{continue;}
            // Alpha is only read by the raw nonpositive skip gate; never rewritten.
            for color in 0..colors{
                let start=color.checked_mul(fmt.sample.bytes()).ok_or_else(||bad("color offset"))?;let end=start.checked_add(fmt.sample.bytes()).ok_or_else(||bad("color offset"))?;let sample=pixel.get_mut(start..end).ok_or_else(||bad("color sample width"))?;
                match fmt.sample{
                    SampleType::U32=>{let before=u32::from_ne_bytes((&*sample).try_into().map_err(|_|bad("U32 width"))?);
                        let result=evaluate(f64::from(before)/f64::from(u32::MAX),r,weight)?;
                        // One nearest native quantization AFTER binary64 Exposure
                        // and coverage, with no intermediate native/f32 sample.
                        let rounded=(result*f64::from(u32::MAX)).round();
                        if !rounded.is_finite()||!(0.0..=f64::from(u32::MAX)).contains(&rounded){return Err(bad("Exposure native quantization domain"));}
                        let after=rounded as u32;if after!=before{sample.copy_from_slice(&after.to_ne_bytes());changed=true;}
                    },
                    SampleType::F64=>{let before=f64::from_ne_bytes((&*sample).try_into().map_err(|_|bad("F64 width"))?);let after=evaluate(before,r,weight)?;
                        if after.to_bits()!=before.to_bits(){sample.copy_from_slice(&after.to_ne_bytes());changed=true;}
                    },
                    _=>return Err(bad("native Exposure depth")),
                }
            }
        }
        if changed{out.try_write_interleaved_with(part,&raw,c).map_err(|e|error(c,e))?;any=true;}
    }
    poll(c)?;Ok(any.then_some(out))
}
pub(crate) fn try_run(s:&mut Session,p:&Value)->Result<Option<Value>>{
    let Some(r)=request(s,p)?else{return Ok(None)};let st=s.active().ok_or(EngineError::NoDocument)?;let before=st.doc.clone();let worker=before.clone();let weak=before.downgrade();let revision=st.revision;let active=st.active_layer;let view=st.channel_view.target.clone();let editing=st.channel_view.editing_mask;let shown=st.channel_view.layer_mask;let layer=r.layer;
    let value=crate::jobs::run(s,"Exposure",true,move|ctx|ctx.stage(0.0,0.95,"Exposure",|c|Ok((prepare(&worker,&r,c)?,ctx.clone()))),move|s,(output,ctx)|ctx.stage(0.95,1.0,"Exposure",|c|{
        poll(c)?;let st=s.active().ok_or(EngineError::NoDocument)?;
        if st.doc.id!=before.id||st.revision!=revision||!st.doc.shares_state(&before)||!weak.is_current(&st.doc)||st.active_layer!=active||st.channel_view.target!=view||st.channel_view.editing_mask!=editing||st.channel_view.layer_mask!=shown||st.floating.is_some()||st.coalesce.is_some()||s.command_has_job_on(before.id)||s.command_has_pending_job()||s.coalesce_request.is_some()||s.color_restrict.is_some(){return Err(bad("captured raster Exposure changed"));}
        target(&st.doc,layer,c)?;let Some(output)=output else{return Ok(Value::Null)};
        s.edit_with_interrupt("Exposure",c,move|doc,_|{poll(c)?;target(doc,layer,c)?;doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content=LayerContent::Raster(output);doc.validate_xcf_state_with(c).map_err(|e|error(c,e))?;poll(c)?;Ok(Value::Null)})
    }))?;Ok(Some(value))
}


// Regression drafts only; execution deferred to Root's coordinated testing phase.
#[cfg(test)]
mod tests{
    use super::*;
    use photocraft_doc::{Layer,Size,SampleEncoding};
    use serde_json::json;
    fn fixture(sample:SampleType)->(Session,LayerId,Vec<u8>){
        let mut doc=Document::new("Explicit linear native Exposure",Size::new(1,1),ColorMode::Rgb,sample);
        doc.sample_encoding=Some(SampleEncoding::Linear);doc.icc_profile=Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        let mut layer=Layer::raster("Pixels",doc.pixel_format());let id=layer.id;
        let raw=if sample==SampleType::U32{[1u32,u32::MAX/4,u32::MAX,17].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()}else{
            [0.25f64.to_bits(),0.5f64.to_bits(),0.75f64.to_bits(),0x7ff8_0000_0000_0042u64].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()};
        layer.content=LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(),Rect::new(-1,0,0,1),&raw,&Interrupt::NONE).unwrap());doc.layers.push(layer);
        let mut session=Session::new();session.add_document(doc,None).unwrap();(session,id,raw)
    }
    #[test]
    fn explicit_linear_exposure_is_non_hdr_and_keeps_native_alpha_default_offcanvas(){
        for sample in [SampleType::U32,SampleType::F64]{
            let(session,id,raw)=fixture(sample);let before=session.active().unwrap().doc.clone();let r=request(&session,&json!({"exposure":1})).unwrap().unwrap();
            let output=prepare(&before,&r,&Interrupt::NONE).unwrap().unwrap();let edited=output.to_interleaved(Rect::new(-1,0,0,1));
            let expected=if sample==SampleType::U32{[2u32,(u32::MAX/4)*2,u32::MAX,17].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()}else{
                [0.5f64.to_bits(),1.0f64.to_bits(),1.0f64.to_bits(),0x7ff8_0000_0000_0042u64].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()};
            assert_eq!(edited,expected);assert_ne!(edited,raw);assert_eq!(output.default_bytes(),before.layer(id).unwrap().surface().unwrap().default_bytes());
            assert_eq!(session.active().unwrap().doc.icc_profile,before.icc_profile);assert_eq!(session.active().unwrap().doc.sample_encoding,Some(SampleEncoding::Linear));
            assert!(evaluate(2.0,&r,1.0).is_err());assert!(evaluate(-0.1,&r,1.0).is_err());
        }
    }
    #[test]
    fn missing_or_nonlinear_context_refuses_without_implicit_gamma_or_profile(){
        let(session,_,_)=fixture(SampleType::F64);let before=session.active().unwrap().doc.clone();
        let mut doc=before.as_ref().clone();doc.sample_encoding=None;assert!(working(&doc,&Interrupt::NONE).is_err());
        doc.sample_encoding=Some(SampleEncoding::NonlinearProfile);assert!(working(&doc,&Interrupt::NONE).is_err());
        doc.sample_encoding=Some(SampleEncoding::Linear);doc.icc_profile=None;assert!(working(&doc,&Interrupt::NONE).is_err());
        doc.icc_profile=Some(vec![0;128]);assert!(working(&doc,&Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn stale_cancel_zero_coverage_and_identity_keep_original_and_real_redo(){
        let(mut session,id,raw)=fixture(SampleType::F64);session.edit("Seed",|doc,_|{doc.name="Seed".into();Ok(())}).unwrap();session.execute("edit.undo",Value::Null).unwrap();
        let before=session.active().unwrap().doc.clone();let revision=session.active().unwrap().revision;
        let r=request(&session,&json!({"exposure":1,"offset":-0.5})).unwrap().unwrap();
        let nan=f64::from_bits(0x7ff8_0000_0000_0042);assert_eq!(evaluate(nan,&r,0.0).unwrap().to_bits(),nan.to_bits());
        let cancel=||true;assert!(matches!(prepare(&before,&r,&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));
        assert!(try_run(&mut session,&json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
        try_run(&mut session,&json!({})).unwrap();let current=session.active().unwrap();assert!(current.doc.shares_state(&before));assert!(current.history.can_redo());
        assert_eq!(current.doc.layer(id).unwrap().surface().unwrap().to_interleaved(Rect::new(-1,0,0,1)),raw);
    }
}
