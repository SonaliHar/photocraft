//! Controlled native stored-color Levels. Alpha/source metadata are copied raw.
use crate::{EngineError,Result,Session,channel_cmds::ChannelTarget};
use photocraft_doc::{Document,LayerContent,LayerId,item_identity::ItemRef};
use photocraft_color::{SampleType,ColorMode};
use photocraft_geom::{Rect,TileCoord,TILE_SIZE};
use photocraft_raster::{Surface,Interrupt,MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;
use std::sync::Arc;
const ID:&str="image.adjustments.levels";
fn bad(m:impl Into<String>)->EngineError{EngineError::BadParams{cmd:ID.into(),msg:m.into()}}
fn poll(c:&Interrupt<'_>)->Result<()>{c.check().map_err(|_|EngineError::Cancelled)}
fn error(c:&Interrupt<'_>,e:impl std::fmt::Display)->EngineError{if c.cancelled(){EngineError::Cancelled}else{bad(e.to_string())}}
fn decimal(v:&Value)->Option<u64>{let s=v.as_str()?;if s.is_empty()||s.len()>20||(s.len()>1&&s.starts_with('0'))||!s.bytes().all(|b|b.is_ascii_digit()){return None}s.parse().ok()}
struct Request{layer:LayerId,opacity:f64,percent:Option<u8>,master:photocraft_doc::adjust::LevelsChannel,per_channel:[photocraft_doc::adjust::LevelsChannel;3]}
fn target<'a>(doc:&'a Document,id:LayerId,c:&Interrupt<'_>)->Result<(&'a Surface,Rect)>{
    poll(c)?;let mut found=0usize;
    photocraft_doc::resident_structure::visit_layers_with(doc,c,|layer|{if layer.id==id{found=found.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;}Ok(true)})?;
    if id.0==0||found!=1{return Err(bad("missing or ambiguous raster owner"));}
    crate::commands::check_pixels_unlocked(doc,id)?;crate::xcf_item_locks::transparency_locked(doc,id)?;
    let layer=doc.layer(id).ok_or(EngineError::NoLayer(id))?;let LayerContent::Raster(source)=&layer.content else{return Err(bad("native Levels requires a plain resident raster"))};
    let format=source.format();
    if !matches!(format.mode,ColorMode::Rgb|ColorMode::Grayscale)||!matches!(format.sample,SampleType::U32|SampleType::F64)||format.mode!=doc.mode||format.sample!=doc.depth
        ||doc.color_table.is_some()||layer.indexed_source.is_some()||layer.video.is_some()||layer.import_compositing.is_some(){return Err(bad("native raster color/geometry context unsupported"));}
    if source.tile_count()>100_000{return Err(bad("native raster tile limit"));}
    let area=source.try_content_bounds_with_interrupt(c).map_err(|e|error(c,e))?;
    if let Some(state)=&doc.xcf_state{
        if state.items.len()>10_000||state.source_items.len()>10_000||state.filter_stacks.len()>10_000||state.editable_indexed.len()>10_000{return Err(bad("current state limit"));}
        if state.source_items.iter().any(|i|i.owner==ItemRef::Layer(id))||state.filter_stacks.iter().any(|i|i.owner==ItemRef::Layer(id))||state.editable_indexed.iter().any(|i|i.owner==id){return Err(bad("source/indexed/filter carrier levels is not qualified"));}
        let mut items=state.items.iter().filter(|i|i.item==ItemRef::Layer(id));let item=items.next().ok_or_else(||bad("missing current typed raster"))?;
        if items.next().is_some()||item.composition.is_some(){return Err(bad("ambiguous or interpreted current raster"));}
        let extent=item.raster_bounds.ok_or_else(||bad("missing current raster extent"))?;
        if !area.is_empty()&&area.intersect(&extent)!=area{return Err(bad("retained pixels exceed declared current extent"));}
    }
    Ok((source,area))
}
fn context(s:&Session)->Result<()>{let st=s.active().ok_or(EngineError::NoDocument)?;
    if st.revision==u64::MAX||st.doc.accounted().is_some()||st.floating.is_some()||st.coalesce.is_some()||s.has_jobs()||s.command_has_pending_job()||s.coalesce_request.is_some()||s.color_restrict.is_some()
        ||st.doc.xcf_state.as_ref().is_some_and(|x|x.floating.is_some()){return Err(bad("finish transient work before native raster Levels"));}Ok(())}
fn request(s:&Session,p:&Value)->Result<Option<Request>>{
    let st=s.active().ok_or(EngineError::NoDocument)?;
    if let Some(t)=p.get("target") {if t.as_str()!=Some("pixels"){return Ok(None);}}
    else if matches!(st.channel_view.target,ChannelTarget::Alpha(_)|ChannelTarget::LayerMask|ChannelTarget::VectorMask)||st.doc.quick_mask.is_some()||crate::mask_view_cmds::current(st).is_some(){return Ok(None);}
    if let Some(v)=p.get("layer"){if v.as_u64().is_none_or(|v|v==0){return Err(bad("layer must be a nonzero unsigned integer"));}}
    let layer=crate::commands::layer_param(s,p)?;let current=st.doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
    if current.surface().is_none_or(|v|!matches!(v.format().sample,SampleType::U32|SampleType::F64)){return Ok(None);}
    if matches!(st.channel_view.target,ChannelTarget::Color(_)){return Err(bad("native Levels requires all color channels"));}
    envelope(p)?;
    if p.get("__kind").is_some_and(|v|v.as_str()!=Some("levels")){return Err(bad("native raster route requires Levels"));}
    match(p.get("documentId"),p.get("expectedRevision")){(None,None)=>{},(Some(d),Some(r))if st.doc.id.0!=0&&decimal(d)==Some(st.doc.id.0)&&decimal(r)==Some(st.revision)=>{},_=>return Err(bad("captured target changed or incomplete")),}
    context(s)?;target(&st.doc,layer,&Interrupt::NONE)?;
    let percent=match p.get("opacity"){None|Some(Value::Null)=>100.0,Some(v)=>v.as_f64().filter(|v|v.is_finite()).ok_or_else(||bad("opacity must be finite"))?.clamp(0.0,100.0)};
    let sample=current.surface().ok_or_else(||bad("raster disappeared"))?.format().sample;
    if sample==SampleType::U32&&percent.fract()!=0.0{return Err(bad("U32 native Levels requires whole-percent opacity"));}
    let parsed=crate::adjust_params::from_params("levels",p,None,st.doc.mode)?;
    let photocraft_doc::Adjustment::Levels{master,per_channel,space,..}=parsed else{return Err(bad("Levels parameter mismatch"))};
    if space!=photocraft_doc::adjust::ToneSpace::Rgb{return Err(bad("native Levels supports RGB tone space only"));}
    if st.doc.mode==ColorMode::Grayscale&&per_channel.iter().any(|channel|*channel!=photocraft_doc::adjust::LevelsChannel::default()){return Err(bad("native Gray Levels supports the master record only; RGB channel remapping is unsupported"));}
    let request=Request{layer,opacity:percent/100.0,percent:(sample==SampleType::U32).then_some(percent as u8),master,per_channel};
    // Verify every typed table word has the proven exact rational domain before allocation.
    visit_tables(&request,&Interrupt::NONE,|_,_|Ok(()))?;
    Ok(Some(request))
}
pub(crate) fn dispatch_preflight(s:&Session,id:&str,p:&Value,_background:bool)->Result<bool>{if id!=ID{return Ok(false)}request(s,p).map(|r|r.is_some())}
fn cells(r:Rect)->Result<u64>{if r.is_empty(){return Ok(0)}let a=TileCoord::containing(r.x0,r.y0);let b=TileCoord::containing(r.x1.checked_sub(1).ok_or_else(||bad("extent"))?,r.y1.checked_sub(1).ok_or_else(||bad("extent"))?);
    u64::try_from(i64::from(b.tx)-i64::from(a.tx)+1).ok().and_then(|w|u64::try_from(i64::from(b.ty)-i64::from(a.ty)+1).ok().and_then(|h|w.checked_mul(h))).ok_or_else(||bad("tile span overflow"))}
fn ratio(s:Option<&Surface>,x:i32,y:i32)->Result<(u32,u32)>{let Some(s)=s else{return Ok((1,1))};let sample=s.format().sample;let mut raw=[0u8;8];let bytes=raw.get_mut(..sample.bytes()).ok_or_else(||bad("selection width"))?;s.try_read_pixel_bytes(x,y,bytes).map_err(|e|bad(e.to_string()))?;
    Ok(match sample{SampleType::U8=>(*bytes.first().ok_or_else(||bad("selection width"))? as u32,255),SampleType::U16=>(u32::from(u16::from_ne_bytes((&*bytes).try_into().map_err(|_|bad("selection width"))?)),65535),SampleType::U32=>(u32::from_ne_bytes((&*bytes).try_into().map_err(|_|bad("selection width"))?),u32::MAX),SampleType::F32|SampleType::F64=>match photocraft_color::try_read_sample_f64(bytes,sample,0).map_err(|e|bad(e.to_string()))?{0.0=>(0,1),1.0=>(1,1),_=>return Err(bad("U32 Levels requires integer selection or Boolean floating coverage"))}})
}
use crate::native_tone_lut::TABLE_LEN;
#[cfg(test)]
use crate::native_tone_lut::TABLE_SCALE;
fn envelope(params:&Value)->Result<()>{
    if params.is_null(){return Ok(())}
    let object=params.as_object().filter(|object|object.len()<=15).ok_or_else(||bad("bounded Levels parameters required"))?;
    for(key,value)in object{
        match key.as_str(){
            "layer"|"target"|"documentId"|"expectedRevision"|"opacity"|"__kind"=>{},
            "inBlack"|"inWhite"|"gamma"|"outBlack"|"outWhite"=>{if value.as_f64().is_none_or(|value|!value.is_finite()){return Err(bad("Levels parameters must be finite numbers"));}},
            "gray"|"red"|"green"|"blue"=>{
                let record=value.as_object().filter(|record|record.len()<=5).ok_or_else(||bad("bounded Levels channel record required"))?;
                for(field,value)in record{if !matches!(field.as_str(),"inBlack"|"inWhite"|"gamma"|"outBlack"|"outWhite")||value.as_f64().is_none_or(|value|!value.is_finite()){return Err(bad("invalid Levels channel parameter"));}}
            },
            _=>return Err(bad("unsupported native Levels parameter")),
        }
    }
    Ok(())
}
fn visit_tables(request:&Request,control:&Interrupt<'_>,mut accept:impl FnMut(usize,u64)->Result<()>)->Result<()>{
    for(channel,record)in request.per_channel.iter().enumerate(){
        for index in 0..TABLE_LEN{
            if index%128==0{poll(control)?;}
            // This is the exact existing typed f32 channel-before-master table recipe,
            // not evaluation of native source words through f32.
            let x=index as f32/(TABLE_LEN-1) as f32;
            let output=photocraft_compose::adjust::levels_q(&request.master,photocraft_compose::adjust::levels_q(record,x,None),None);
            accept(channel,crate::native_tone_lut::exact_word(output).map_err(bad)?)?;
        }
    }
    poll(control)
}
fn tables(request:&Request,control:&Interrupt<'_>)->Result<[Vec<u64>;3]>{
    let mut tables:[Vec<u64>;3]=std::array::from_fn(|_|Vec::new());
    for table in &mut tables{table.try_reserve_exact(TABLE_LEN).map_err(|_|bad("Levels table allocation denied"))?;}
    visit_tables(request,control,|channel,value|{tables.get_mut(channel).ok_or_else(||bad("Levels table channel"))?.push(value);Ok(())})?;
    let allocated=tables.iter().try_fold(0usize,|sum,table|sum.checked_add(table.capacity().checked_mul(std::mem::size_of::<u64>())?)).ok_or_else(||bad("Levels table capacity overflow"))?;
    if allocated>1<<20{return Err(bad("Levels table allocator capacity exceeded admitted scratch"));}
    Ok(tables)
}
fn interpolate_u32(table:&[u64],before:u32,percent:u8,selection:u32,selection_max:u32)->Result<u32>{crate::native_tone_lut::interpolate_u32(table,before,percent,selection,selection_max).map_err(bad)}
fn interpolate_f64(table:&[u64],before:f64,weight:f64)->Result<f64>{crate::native_tone_lut::interpolate_f64(table,before,weight).map_err(bad)}
fn prepare(doc:&Document,r:&Request,c:&Interrupt<'_>)->Result<Option<Surface>>{
    let(source,area)=target(doc,r.layer,c)?;let fmt=source.format();let bpp=fmt.bytes_per_pixel();let colors=fmt.mode.color_channels();
    let limits=photocraft_io::NativeSaveLimits::default().preparation;
    let payload=photocraft_io::retained_native_payload_bytes(doc,limits,c).map_err(|e|error(c,e))? as u64;let model=photocraft_io::retained_native_model_bytes(doc,limits,c).map_err(|e|error(c,e))? as u64;let footprint=crate::DocumentSnapshot::resident_raster_footprint_of(doc,c)?;
    let count=cells(area)?;let pixels=count.checked_mul(TILE_SIZE as u64).and_then(|n|n.checked_mul(TILE_SIZE as u64)).ok_or_else(||bad("Levels extent overflow"))?;
    let output=pixels.checked_mul(bpp as u64).and_then(|n|n.checked_add(count.checked_mul(2048)?)).ok_or_else(||bad("Levels output overflow"))?;
    let peak=payload.checked_add(model.checked_mul(6).ok_or_else(||bad("model overflow"))?).and_then(|n|n.checked_add(footprint.retained_bytes.checked_mul(2)?)).and_then(|n|n.checked_add(footprint.clone_metadata_bytes.checked_mul(6)?)).and_then(|n|n.checked_add(output)).and_then(|n|n.checked_add(4<<20)).ok_or_else(||bad("Levels staging overflow"))?;
    let work=pixels.checked_mul((colors as u64+4)*128).and_then(|n|n.checked_add((source.tile_count() as u64).checked_add(doc.selection.as_ref().map_or(0,|s|s.tile_count() as u64))?.checked_mul(TILE_SIZE as u64)?.checked_mul(TILE_SIZE as u64)?.checked_mul(64)?)).and_then(|n|n.checked_add(model.checked_add(payload)?.checked_mul(16)?)).and_then(|n|n.checked_add(3*TABLE_LEN as u64*256)).ok_or_else(||bad("Levels work overflow"))?;
    if peak>MAX_CHECKED_RASTER_BYTES||work>8_000_000_000{return Err(bad("Levels retained/output/work limit"));}
    if let Some(selection)=&doc.selection{if selection.format().mode!=ColorMode::Grayscale||selection.format().alpha{return Err(bad("selection requires Gray coverage without alpha"));}selection.try_validate_tile_origins_with(c).map_err(|e|error(c,e))?;}
    if area.is_empty()||r.opacity==0.0{return Ok(None);}
    let tables=tables(r,c)?;
    let mut out=Surface::try_with_default_bytes(fmt,source.default_bytes()).map_err(|e|error(c,e))?;for(cell,tile)in source.tiles(){poll(c)?;out.put_tiles(std::iter::once((*cell,Arc::clone(tile))));}
    let mut any=false;
    for cell in area.tiles(){poll(c)?;let part=cell.rect().intersect(&area);if part.is_empty(){continue;}
        let mut raw=source.try_read_interleaved_with(part,c).map_err(|e|error(c,e))?;let width=usize::try_from(part.width()).map_err(|_|bad("Levels row"))?;let mut changed=false;
        for(i,pixel)in raw.chunks_exact_mut(bpp).enumerate(){if i%256==0{poll(c)?;}let x=i32::try_from(i64::from(part.x0)+(i%width) as i64).map_err(|_|bad("Levels x"))?;let y=i32::try_from(i64::from(part.y0)+(i/width) as i64).map_err(|_|bad("Levels y"))?;
            let selection=doc.selection.as_ref();let fraction=if fmt.sample==SampleType::U32{Some(ratio(selection,x,y)?)}else{None};
            let weight=if fmt.sample==SampleType::F64{let selected=if let Some(s)=selection{let mut value=[0f64];s.try_read_pixel_f64(x,y,&mut value).map_err(|e|error(c,e))?;value[0]}else{1.0};if !selected.is_finite()||!(0.0..=1.0).contains(&selected){return Err(bad("selection is not finite unit"));}selected*r.opacity}else{1.0};
            if weight==0.0||fraction.is_some_and(|(num,_)|num==0){continue;}
            if crate::native_adjustment_alpha::skips(pixel,fmt).map_err(bad)?{continue;}
            // Alpha is only read by the raw nonpositive skip gate; never rewritten.
            for color in 0..colors{
                let start=color.checked_mul(fmt.sample.bytes()).ok_or_else(||bad("color offset"))?;let end=start.checked_add(fmt.sample.bytes()).ok_or_else(||bad("color offset"))?;let sample=pixel.get_mut(start..end).ok_or_else(||bad("color sample width"))?;
                let table=tables.get(color).ok_or_else(||bad("Levels table channel"))?;
                match fmt.sample{
                    SampleType::U32=>{let before=u32::from_ne_bytes((&*sample).try_into().map_err(|_|bad("U32 width"))?);let(num,den)=fraction.ok_or_else(||bad("integer selection domain"))?;let after=interpolate_u32(table,before,r.percent.ok_or_else(||bad("integer opacity domain"))?,num,den)?;if after!=before{sample.copy_from_slice(&after.to_ne_bytes());changed=true;}},
                    SampleType::F64=>{let before=f64::from_ne_bytes((&*sample).try_into().map_err(|_|bad("F64 width"))?);let after=interpolate_f64(table,before,weight)?;if after.to_bits()!=before.to_bits(){sample.copy_from_slice(&after.to_ne_bytes());changed=true;}},
                    _=>return Err(bad("native Levels depth")),
                }
            }
        }
        if changed{out.try_write_interleaved_with(part,&raw,c).map_err(|e|error(c,e))?;any=true;}
    }
    poll(c)?;Ok(any.then_some(out))
}
pub(crate) fn try_run(s:&mut Session,p:&Value)->Result<Option<Value>>{
    let Some(r)=request(s,p)?else{return Ok(None)};let st=s.active().ok_or(EngineError::NoDocument)?;let before=st.doc.clone();let worker=before.clone();let weak=before.downgrade();let revision=st.revision;let active=st.active_layer;let view=st.channel_view.target.clone();let editing=st.channel_view.editing_mask;let shown=st.channel_view.layer_mask;let layer=r.layer;
    let value=crate::jobs::run(s,"Levels",true,move|ctx|ctx.stage(0.0,0.95,"Levels",|c|Ok((prepare(&worker,&r,c)?,ctx.clone()))),move|s,(output,ctx)|ctx.stage(0.95,1.0,"Levels",|c|{
        poll(c)?;let st=s.active().ok_or(EngineError::NoDocument)?;
        if st.doc.id!=before.id||st.revision!=revision||!st.doc.shares_state(&before)||!weak.is_current(&st.doc)||st.active_layer!=active||st.channel_view.target!=view||st.channel_view.editing_mask!=editing||st.channel_view.layer_mask!=shown||st.floating.is_some()||st.coalesce.is_some()||s.command_has_job_on(before.id)||s.command_has_pending_job()||s.coalesce_request.is_some()||s.color_restrict.is_some(){return Err(bad("captured raster Levels changed"));}
        target(&st.doc,layer,c)?;let Some(output)=output else{return Ok(Value::Null)};
        s.edit_with_interrupt("Levels",c,move|doc,_|{poll(c)?;target(doc,layer,c)?;doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content=LayerContent::Raster(output);doc.validate_xcf_state_with(c).map_err(|e|error(c,e))?;poll(c)?;Ok(Value::Null)})
    }))?;Ok(Some(value))
}



// Draft regressions only; no execution during implementation-first integration.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer,Size};
    use serde_json::json;
    fn fixture()->(Session,LayerId,Vec<u8>){
        let mut doc=Document::new("Native Levels",Size::new(1,1),ColorMode::Rgb,SampleType::U32);
        let mut layer=Layer::raster("Original",doc.pixel_format());let id=layer.id;
        let raw=[u32::MAX/4,u32::MAX/2,u32::MAX-1,29].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        layer.content=LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(),Rect::new(-1,0,0,1),&raw,&Interrupt::NONE).unwrap());doc.layers.push(layer);
        let mut session=Session::new();session.add_document(doc,None).unwrap();(session,id,raw)
    }
    #[test]
    fn typed_table_words_are_the_existing_upstream_channel_before_master_recipe(){
        let(session,id,_)=fixture();let r=request(&session,&json!({"red":{"outBlack":64},"outWhite":192})).unwrap().unwrap();
        let actual=tables(&r,&Interrupt::NONE).unwrap();
        let adj=photocraft_doc::Adjustment::Levels{master:r.master.clone(),per_channel:r.per_channel.clone(),space:photocraft_doc::adjust::ToneSpace::Rgb,black:Default::default()};
        let original=photocraft_compose::adjust::tone_luts(&adj);
        for(channel,table)in actual.iter().enumerate(){for(index,value)in table.iter().enumerate(){assert_eq!(*value as f64/TABLE_SCALE as f64,f64::from(original[channel][index]));}}
        assert_eq!(r.layer,id);
    }
    #[test]
    fn u32_interpolation_and_coverage_round_only_once(){
        let mut table=vec![0u64;TABLE_LEN];table[1]=TABLE_SCALE as u64;
        // Position is below first knot. Half-opacity is applied to the exact
        // interpolated rational, not a quantized native temporary result.
        let input=1u32;let mapped=i128::from(input)*(TABLE_LEN-1) as i128*TABLE_SCALE;
        let expected=((i128::from(input)*TABLE_SCALE*50+mapped*50)+TABLE_SCALE*100/2)/(TABLE_SCALE*100);
        assert_eq!(interpolate_u32(&table,input,50,1,1).unwrap(),u32::try_from(expected).unwrap());
        assert_eq!(interpolate_u32(&table,input,50,0,u32::MAX).unwrap(),input);
    }
    #[test]
    fn native_transaction_keeps_alpha_default_source_and_real_redo_on_refusal(){
        let(mut session,id,raw)=fixture();session.edit("Seed",|doc,_|{doc.name="Seed".into();Ok(())}).unwrap();session.execute("edit.undo",Value::Null).unwrap();
        let before=session.active().unwrap().doc.clone();let revision=session.active().unwrap().revision;
        for params in [json!({"outBlack":1e-20}),json!({"eyedropper":"black"}),json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})]{assert!(try_run(&mut session,&params).is_err());assert!(session.active().unwrap().doc.shares_state(&before));assert!(session.active().unwrap().history.can_redo());}
        let r=request(&session,&json!({"outWhite":128})).unwrap().unwrap();let out=prepare(&before,&r,&Interrupt::NONE).unwrap().unwrap();
        let changed=out.to_interleaved(Rect::new(-1,0,0,1));assert_ne!(changed,raw);assert_eq!(changed.get(12..),raw.get(12..));assert_eq!(out.default_bytes(),before.layer(id).unwrap().surface().unwrap().default_bytes());
        let cancel=||true;assert!(matches!(prepare(&before,&r,&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));
    }
    #[test]
    fn nonlinear_gamma_uses_original_typed_recipe_and_exact_native_interpolation(){
        let(session,_,_)=fixture();
        let r=request(&session,&json!({"gamma":1.75,"red":{"gamma":1.2}})).unwrap().unwrap();
        let actual=tables(&r,&Interrupt::NONE).unwrap();
        let adjustment=photocraft_doc::Adjustment::Levels{master:r.master.clone(),per_channel:r.per_channel.clone(),space:photocraft_doc::adjust::ToneSpace::Rgb,black:Default::default()};
        let expected=photocraft_compose::adjust::tone_luts(&adjustment);
        for(channel,row)in actual.iter().enumerate(){for(index,word)in row.iter().enumerate(){assert_eq!(*word as f64/TABLE_SCALE as f64,f64::from(expected[channel][index]));}}
        let input=0.375f64;let position=input*(TABLE_LEN-1) as f64;let index=position.floor() as usize;let fraction=position-index as f64;
        let left=f64::from(expected[0][index])*(1.0-fraction);let right=f64::from(expected[0][index+1])*fraction;
        assert_eq!(interpolate_f64(&actual[0],input,1.0).unwrap().to_bits(),(left+right).to_bits());
        let value=123456789u32;assert_eq!(interpolate_u32(&actual[0],value,0,u32::MAX,u32::MAX).unwrap(),value);
    }
    #[test]
    fn parser_valid_nonlinear_or_tiny_table_words_can_still_refuse_exact_domain(){
        let(session,_,_)=fixture();
        assert!(crate::adjust_params::from_params("levels",&json!({"gamma":0.25}),None,ColorMode::Rgb).is_ok());
        assert!(request(&session,&json!({"gamma":0.25})).is_err());
        assert!(request(&session,&json!({"outBlack":1e-20})).is_err());
        assert!(request(&session,&json!({"gamma":"wrong type"})).is_err());
    }

}
