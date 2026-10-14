//! Captured preview preparation. No Session reference escapes capture, and no
//! document/history mutation occurs when a prepared preview is installed.
use super::*;
use crate::preset_store::TipStoreWitness;
use photocraft_doc::LayerId;
use std::sync::Arc;

/// Owned gesture-start recipe suitable for a preview worker. Capture uses the
/// existing bounded scalar/model copies and held preset-store authority.
pub struct CapturedLiveBegin { input:Input, initial_count:usize }
/// A private initial preview plus its immutable installation witness.
pub struct PreparedLiveBegin { input:Input, live:LiveStroke }
/// Runtime-only original gesture authority; never serializes locators or grants.
pub struct LiveBeginFence { input:Input, lock:bool, format:photocraft_color::PixelFormat }
impl CapturedLiveBegin {
    pub fn capture(s:&Session,cmd:&str,p:&Value)->Result<Self>{Self::capture_with_interrupt(s,cmd,p,&Interrupt::NONE)}
    pub fn capture_with_interrupt(s:&Session,cmd:&str,p:&Value,c:&Interrupt<'_>)->Result<Self>{
        poll(c)?;if !matches!(cmd,"paint.stroke"|"paint.pencil"){return Err(bad(cmd,"live strokes are `paint.stroke` or `paint.pencil`"));}
        stroke_controlled::dispatch_preflight(s,cmd,p,false)?;has_paintable(s).map_err(EngineError::Other)?;
        let id=if crate::channel_cmds::is_channel_target(p){None}else{Some(layer_id(s,p)?)};
        let mut recipe=brush_recipe_controlled::Recipe::capture_for(s,cmd,p,c)?;
        let stores:Vec<TipStoreWitness>=std::mem::take(&mut recipe.witnesses);
        let count=p.get("points").and_then(Value::as_array).map_or(0,Vec::len);
        let input=capture_input(s,cmd,p,"Live stroke",BrushSettings::default(),Vec::new(),id,num(p,"zoom").unwrap_or(1.0),cmd=="paint.pencil"&&flag(p,"autoErase",false),Some(recipe),stores)?;
        poll(c)?;Ok(Self{input,initial_count:count})
    }
    /// Number of initial serialized events, for the UI's ordered queue cursor.
    pub fn initial_point_count(&self)->usize{self.initial_count}
    pub fn prepare(self,c:&Interrupt<'_>)->Result<PreparedLiveBegin>{
        poll(c)?;let mut input=self.input;let standard=StrokeLimits::default();
        let doc=&input.snapshot;let native=photocraft_io::NativeSaveLimits::default().preparation;
        let payload=photocraft_io::retained_native_payload_bytes(doc,native,c).map_err(|e|if c.cancelled(){EngineError::Cancelled}else{fail(e.to_string())})? as u64;
        let model=photocraft_io::retained_native_model_bytes(doc,native,c).map_err(|e|if c.cancelled(){EngineError::Cancelled}else{fail(e.to_string())})? as u64;
        let raster=DocumentSnapshot::resident_raster_footprint_of(doc,c)?;
        let fixed=add(input.capture_bytes,add(payload,add(mul(model,4)?,mul(raster.clone_metadata_bytes,4)?)?)?)?;
        let mut remaining=standard.resident_bytes.checked_sub(fixed).ok_or_else(||fail("live recipe document headroom"))?;
        let recipe=input.recipe.take().ok_or_else(||fail("live recipe missing"))?;
        let(mut brush,points)=recipe.resolve_for(&input.cmd,&input.params,StrokeLimits{resident_bytes:remaining/2,work_units:standard.work_units/8},c)?;
        check_coords(&points,&input.cmd)?;
        remaining=remaining.checked_sub(mul(points.capacity() as u64,size_of::<StrokePoint>() as u64)?).ok_or_else(||fail("live prepared point capacity"))?;
        if (is_mask_target(&input.params)||crate::channel_cmds::is_channel_target(&input.params))&&brush.erase{brush.erase=false;brush.color=input.background;}
        let foreground=brush.color;let source=crate::channel_cmds::read_target_surface(doc,input.id,&input.params)?;
        source.try_validate_tile_origins_with(c).map_err(super::raster)?;
        let format=source.format();let lock=read_lock(doc,input.id,&input.params)?;
        let(wide,profile_work)=wide_stroke_context_with_limits(doc,input.id,&input.params,&brush,format,lock,input.restricted,input.auto_erase,StrokeLimits{resident_bytes:remaining/3,work_units:standard.work_units/8},c)?;
        if profile_work>standard.work_units/8{return Err(fail("live native profile work"));}
        if let Some(wide)=&wide{remaining=remaining.checked_sub(add(wide.color.resident_payload_bytes().map_err(super::raster)?,mul(wide.color.tile_count() as u64,512)?)?).ok_or_else(||fail("live native color retention"))?;}
        let(cmyk,cmyk_held)=ordinary_one_shot_controlled::cmyk_scope(doc,format.mode==photocraft_color::ColorMode::Cmyk,remaining,c)?;
        remaining=remaining.checked_sub(cmyk_held).ok_or_else(||fail("live profile/renderer headroom"))?;
        erase_locked(&mut brush,lock,input.background);
        photocraft_color::convert::with_cmyk_space(cmyk.as_ref(),||{if input.auto_erase{apply_auto_erase(&mut brush,source,points.first(),foreground,input.background);}});poll(c)?;
        let count=input.axis.as_ref().map_or(0,|axis|axis.mirror_count()).checked_add(1).filter(|n|*n<=4).ok_or_else(||fail("live renderer count"))?;
        let construction=StrokeLimits{resident_bytes:remaining,work_units:standard.work_units/8};
        let per=StrokeRenderer::new_requirements(&brush,Some(format),input.zoom,construction,c).map_err(paint)?;
        let arrays=mul((count-1) as u64,(size_of::<StrokeRenderer>()+size_of::<bool>()) as u64)?;
        let held=add(arrays,mul(per.retained_bytes,count as u64)?)?;
        let staged=mul(per.staged_bytes,count as u64)?;
        if add(held,staged)?>remaining||mul(per.work_units,count as u64)?>construction.work_units{return Err(fail("live aggregate constructor limit"));}
        let each=StrokeLimits{resident_bytes:add(per.retained_bytes,per.staged_bytes)?,work_units:per.work_units};
        let mut mirrors=Vec::new();mirrors.try_reserve_exact(count-1).map_err(|_|fail("live mirrors allocation"))?;
        let mut distinct=Vec::new();distinct.try_reserve_exact(count-1).map_err(|_|fail("live distinct flags allocation"))?;
        let actual_arrays=add(mul(mirrors.capacity() as u64,size_of::<StrokeRenderer>() as u64)?,mul(distinct.capacity() as u64,size_of::<bool>() as u64)?)?;
        if add(actual_arrays,mul(add(per.retained_bytes,per.staged_bytes)?,count as u64)?)?>remaining{return Err(fail("live constructor array capacity"));}
        let renderer=StrokeRenderer::try_new_with_interrupt(&brush,Some(format),input.zoom,each,c).map_err(paint)?;
        for _ in 1..count{poll(c)?;mirrors.push(StrokeRenderer::try_new_with_interrupt(&brush,Some(format),input.zoom,each,c).map_err(paint)?);distinct.push(false);}
        poll(c)?;
        // Canonical model/map clone is prepaid; immutable payload Arcs stay shared.
        let mut document=(**doc).clone();let selection=document.selection.clone();
        let (target,current_lock)=crate::channel_cmds::target_surface(&mut document,input.id,&input.params)?;
        if current_lock!=lock||target.format()!=format{return Err(fail("live private target changed"));}
        let pre=target.clone();poll(c)?;
        let mut live=LiveStroke{doc:DocumentSnapshot::from_resident(Arc::new(document))?,seed:brush.seed,cmd:input.cmd.clone(),renderer,symmetry:input.axis.clone(),mirrors,mirror_distinct:distinct,union_live:None,pre,sel:selection,lock,layer:input.id,params:input.params.clone(),tail:Rect::EMPTY,wide,cmyk,points:Vec::new(),point_ledger:Default::default(),base:(doc.id,input.revision)};
        // This first state is already private: failure drops it, with no original
        // preview to clone. Each repeated generator/tail/union phase receives a
        // conservative partition rather than resetting the full command work.
        let initial=StrokeLimits{resident_bytes:remaining.checked_sub(held).ok_or_else(||fail("live initial raster headroom"))?,work_units:standard.work_units/2/((count as u64)*8+8)};
        live.push_staged_with_limits(&points,initial,c)?;input.brush=brush;input.points=points;poll(c)?;Ok(PreparedLiveBegin{input,live})
    }
}
fn read_lock(doc:&photocraft_doc::Document,id:Option<LayerId>,p:&Value)->Result<bool>{
    match crate::channel_cmds::target_of(p){
        crate::channel_cmds::Target::Pixels=>{let id=id.ok_or_else(||fail("live pixel owner missing"))?;crate::commands::check_pixels_unlocked(doc,id)?;crate::xcf_item_locks::transparency_locked(doc,id)},
        crate::channel_cmds::Target::Mask=>{let id=id.ok_or_else(||fail("live mask owner missing"))?;let layer=doc.layer(id).ok_or_else(||fail("live mask owner missing"))?;if doc.effective_locks(id).all{return Err(fail(format!("Could not complete your request because the layer \"{}\" is locked",layer.name)));}crate::xcf_item_locks::require_layer_mask_content(doc,id)?;Ok(false)},
        crate::channel_cmds::Target::Alpha(index)=>{crate::xcf_item_locks::require_channel_content(doc,doc.channels.get(index).ok_or_else(||fail("live channel missing"))?.id)?;Ok(false)},
        crate::channel_cmds::Target::QuickMask=>Ok(false),
    }
}
fn validate_context(i:&Input,s:&Session,lock:bool,format:photocraft_color::PixelFormat,c:&Interrupt<'_>)->Result<()> {
    poll(c)?;let state=s.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id!=i.snapshot.id||state.revision!=i.revision||state.active_layer!=i.active||!i.weak.is_current(&state.doc)||!state.doc.shares_state(&i.snapshot)||state.channel_view!=i.view||state.symmetry!=i.axis||state.floating.is_some()||state.coalesce!=i.coalesce||s.coalesce_request!=i.request||s.tools.brush!=i.tool_brush||s.tools.foreground!=i.foreground||s.tools.background!=i.background||s.color_restrict.is_some()!=i.restricted||s.tools.presets_rev!=i.presets_revision||i.stores.iter().any(|w|!w.matches(s))||s.command_has_job_on(state.doc.id)||s.command_has_pending_job(){return Err(fail("captured live begin context changed"));}
    if read_lock(&state.doc,i.id,&i.params)?!=lock||crate::channel_cmds::read_target_surface(&state.doc,i.id,&i.params)?.format()!=format{return Err(fail("captured live target changed"));}
    poll(c)
}
impl PreparedLiveBegin {
    pub fn seed(&self)->u64{self.live.seed}
    pub fn install(self,s:&Session)->Result<LiveStroke>{self.install_with_interrupt(s,&Interrupt::NONE)}
    pub fn install_with_interrupt(self,s:&Session,c:&Interrupt<'_>)->Result<LiveStroke>{self.install_with_fence_with_interrupt(s,c).map(|(live,_)|live)}
    /// Move the same original witness into the gesture; no settings recapture.
    pub fn install_with_fence(self,s:&Session)->Result<(LiveStroke,LiveBeginFence)>{self.install_with_fence_with_interrupt(s,&Interrupt::NONE)}
    pub fn install_with_fence_with_interrupt(self,s:&Session,c:&Interrupt<'_>)->Result<(LiveStroke,LiveBeginFence)>{
        let lock=self.live.lock;let format=self.live.pre.format();validate_context(&self.input,s,lock,format,c)?;
        Ok((self.live,LiveBeginFence{input:self.input,lock,format}))
    }
}
impl LiveBeginFence {
    pub fn validate(&self,s:&Session)->Result<()>{self.validate_with_interrupt(s,&Interrupt::NONE)}
    pub fn validate_with_interrupt(&self,s:&Session,c:&Interrupt<'_>)->Result<()>{validate_context(&self.input,s,self.lock,self.format,c)}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session()->Session {
        let mut session=Session::new();session.execute("file.new",json!({"width":16,"height":16})).unwrap();session.tools.brush.pressure_size=false;session.tools.brush.smoothing.enabled=false;session
    }
    fn params()->Value {json!({"points":[[2,2,0.7]],"size":3,"color":[0.8,0.2,0.5,1.0]})}
    #[test]
    fn captured_initial_seed_survives_worker_and_later_ordered_points() {
        let session=session();let p=params();let initial=with_blend_mode(resolve_brush(&session,&p,"paint.stroke").unwrap(),&p);
        let capture=CapturedLiveBegin::capture(&session,"paint.stroke",&p).unwrap();assert_eq!(capture.initial_point_count(),1);
        let prepared=capture.prepare(&Interrupt::NONE).unwrap();assert_eq!(prepared.seed(),initial.seed);
        let mut live=prepared.install(&session).unwrap();let next=[StrokePoint::new(8.0,7.0,0.4),StrokePoint::new(9.0,9.0,1.0)];live.push(&next).unwrap();assert_eq!(live.seed,initial.seed);
        assert_eq!(live.points.len(),3);assert_eq!(&live.points[1..],&next);assert_eq!(session.active().unwrap().revision,0);
    }
    #[test]
    fn stale_install_refuses_without_retargeting_or_consuming_real_redo() {
        let mut session=session();session.edit("Rename",|doc,_|{doc.name="Changed".into();Ok(())}).unwrap();assert!(session.undo());
        let prepared=CapturedLiveBegin::capture(&session,"paint.pencil",&params()).unwrap().prepare(&Interrupt::NONE).unwrap();let before=session.active().unwrap().doc.clone();session.tools.background=[0.3,0.4,0.5,1.0];
        assert!(prepared.install(&session).is_err());assert!(session.active().unwrap().doc.shares_state(&before));assert!(session.active().unwrap().history.can_redo());
    }
    #[test]
    fn cancelled_prepare_does_not_publish_preview_or_change_document() {
        let session=session();let before=session.active().unwrap().doc.clone();let capture=CapturedLiveBegin::capture(&session,"paint.stroke",&params()).unwrap();let stop=||true;
        assert!(matches!(capture.prepare(&Interrupt::cancel_only(&stop)),Err(EngineError::Cancelled)));assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
