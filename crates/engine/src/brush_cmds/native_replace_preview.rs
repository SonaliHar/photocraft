//! Native-only preview/commit ticket; no serialized authority or f32 image work.
use super::*;
use photocraft_geom::Rect;
use photocraft_paint::native_replace_controlled::NativeLuminosityState;
use photocraft_paint::render::NativeReplacementGenerator;
use std::mem::size_of;
pub struct NativeLuminosityPreview {
    pub doc:DocumentSnapshot,pub seed:u64,capture:Capture,generator:NativeReplacementGenerator,
    state:NativeLuminosityState,settings:NativeLuminositySettings,points:Vec<StrokePoint>,
    raw_foreground:[f64;4],raw_background:[f64;4],remaining:u64,bounds:Rect,tail_bounds:Rect,presentation_work:u64,
}
pub struct NativeLuminosityCommit {
    candidate:Candidate,brush:BrushSettings,settings:NativeLuminositySettings,
    foreground:[f64;4],background:[f64;4],points:Vec<StrokePoint>,seed:u64,
}
fn same_points(a:&[StrokePoint],b:&[StrokePoint])->bool {
    a.len()==b.len()&&a.iter().zip(b).all(|(a,b)|a.x.to_bits()==b.x.to_bits()&&a.y.to_bits()==b.y.to_bits()&&a.time.to_bits()==b.time.to_bits()&&a.pressure.to_bits()==b.pressure.to_bits()&&a.tilt_x.to_bits()==b.tilt_x.to_bits()&&a.tilt_y.to_bits()==b.tilt_y.to_bits()&&a.rotation.to_bits()==b.rotation.to_bits()&&a.wheel.to_bits()==b.wheel.to_bits())
}
fn same_color(a:[f64;4],b:[f64;4])->bool {a.into_iter().zip(b).all(|(a,b)|a.to_bits()==b.to_bits())}
fn validate_preview(s:&Session,captured:&Capture,c:&Interrupt<'_>)->Result<()> {
    validate_capture(s,captured,c)?;
    if s.command_has_job_on(captured.snapshot.id)||s.command_has_pending_job(){return Err(bad("native preview target is busy"));}poll(c)
}
fn backing(e:photocraft_raster::backing::BackingError,c:&Interrupt<'_>)->EngineError {if c.cancelled(){EngineError::Cancelled}else{bad(e.to_string())}}
/// None is the narrow legacy domain only. Native refusal must not fall back.
pub fn try_native_color_replacement_preview(s:&Session,p:&Value,c:&Interrupt<'_>)->Result<Option<NativeLuminosityPreview>> {
    let Some(mut input)=capture(s,p,c)?else{return Ok(None);};
    let snapshot=&input.capture.snapshot;let native=photocraft_io::NativeSaveLimits::default().preparation;
    let model=photocraft_io::retained_native_model_bytes(snapshot,native,c).map_err(|e|io(e,c))? as u64;
    let payload=photocraft_io::retained_native_payload_bytes(snapshot,native,c).map_err(|e|io(e,c))? as u64;
    let raster=DocumentSnapshot::resident_raster_footprint_of(snapshot,c)?;
    let held=add(payload,add(mul(model,4)?,add(mul(raster.clone_metadata_bytes,4)?,mul(100_000,size_of::<StrokePoint>() as u64)?)?)?)?;
    let standard=StrokeLimits::default();let remaining=standard.resident_bytes.checked_sub(held).ok_or_else(||bad("native preview model/input headroom"))?;
    let fg=super::super::native_wide_color(snapshot,input.capture.format,&Value::Null,input.foreground)?;input.settings.foreground=[fg[0],fg[1],fg[2]];
    if input.settings.sampling==Sampling::BackgroundSwatch{let bg=super::super::native_wide_color(snapshot,input.capture.format,&Value::Null,input.background)?;input.settings.background=[bg[0],bg[1],bg[2]];}
    let generator=NativeReplacementGenerator::new(&input.brush,StrokeLimits{resident_bytes:remaining/4,work_units:standard.work_units/16},c).map_err(paint)?;
    let points=input.points;let mut out=NativeLuminosityPreview{doc:snapshot.clone(),seed:input.brush.seed,capture:input.capture,generator,state:NativeLuminosityState::new(),settings:input.settings,points:Vec::new(),raw_foreground:input.foreground,raw_background:input.background,remaining,bounds:Rect::EMPTY,tail_bounds:Rect::EMPTY,presentation_work:0};
    out.push_for_session_with_interrupt(s,&points,standard,c)?;Ok(Some(out))
}
impl NativeLuminosityPreview {
    pub fn bounds(&self)->Rect {self.bounds}
    fn quotas(&self,limits:StrokeLimits)->Result<(StrokeLimits,StrokeLimits)> {
        let standard=StrokeLimits::default();if limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units{return Err(bad("native preview lower-only limits"));}
        let bytes=self.remaining.min(limits.resident_bytes);
        // Permanent state, private next state and speculative tail coexist. Each
        // lower envelope is deliberately conservative, not an allocator/RSS claim.
        Ok((StrokeLimits{resident_bytes:bytes/4,work_units:limits.work_units/16},StrokeLimits{resident_bytes:bytes/4,work_units:(limits.work_units/2).checked_sub(self.presentation_work).ok_or_else(||bad("native preview aggregate presentation work"))?}))
    }
    /// Feed only new points. Permanent generator/Once/WORK and display swap together.
    pub fn push_for_session_with_interrupt(&mut self,s:&Session,points:&[StrokePoint],limits:StrokeLimits,c:&Interrupt<'_>)->Result<Rect> {
        validate_preview(s,&self.capture,c)?;let(generation,lower)=self.quotas(limits)?;
        let len=self.points.len().checked_add(points.len()).filter(|n|*n<=100_000).ok_or_else(||bad("native preview point limit"))?;
        let mut saved=Vec::new();saved.try_reserve_exact(len).map_err(|_|bad("native preview point allocation"))?;if saved.capacity()!=len{return Err(bad("native preview actual point capacity"));}
        for chunk in self.points.chunks(256).chain(points.chunks(256)){poll(c)?;saved.extend_from_slice(chunk);}
        let mut generator=self.generator.try_clone_with_interrupt(generation,c).map_err(paint)?;
        let dabs=generator.delta_with_interrupt(points,false,generation,c).map_err(paint)?;
        let mut state=self.state.try_clone_with_interrupt(lower,c).map_err(paint)?;
        let pre=self.capture.snapshot.layer(self.capture.id).and_then(|l|l.surface()).ok_or_else(||bad("native preview PRE missing"))?;
        let delta=state.apply_dabs_with_interrupt(pre,&generator.ctx,&dabs,&self.settings,lower,c).map_err(paint)?;
        let mut tail_generator=generator.try_clone_with_interrupt(generation,c).map_err(paint)?;
        let tail=tail_generator.delta_with_interrupt(&[],true,generation,c).map_err(paint)?;
        let mut display=state.try_clone_with_interrupt(lower,c).map_err(paint)?;
        let tail_bounds=display.apply_dabs_with_interrupt(pre,&generator.ctx,&tail,&self.settings,lower,c).map_err(paint)?;
        let dirty=delta.union(&self.tail_bounds).union(&tail_bounds);
        let base=self.doc.layer(self.capture.id).and_then(|l|l.surface()).ok_or_else(||bad("native preview display missing"))?;
        let(output,damage)=display.output_region_with_interrupt(pre,base,dirty,self.capture.snapshot.selection.as_ref(),self.capture.lock,&self.settings,lower,c).map_err(paint)?;
        let presentation_work=add(self.presentation_work,add(display.output_work_units(base,dirty).map_err(paint)?,add(tail_generator.work_units().checked_sub(generator.work_units()).ok_or_else(||bad("native tail work ledger"))?,display.work_units().checked_sub(state.work_units()).ok_or_else(||bad("native tail state ledger"))?)?)?)?;
        let mut doc=self.capture.snapshot.clone();let private=doc.try_resident_mut_with_raster_limit(limits.resident_bytes,c).map_err(|e|backing(e,c))?;
        *private.layer_mut(self.capture.id).and_then(|l|l.surface_mut()).ok_or_else(||bad("native preview target missing"))?=output;
        let changed=dirty;validate_preview(s,&self.capture,c)?;poll(c)?;
        self.doc=doc;self.generator=generator;self.state=state;self.points=saved;self.bounds=display.bounds();self.tail_bounds=tail_bounds;self.presentation_work=presentation_work;
        // Damage includes restoring the preceding speculative tail even if the
        // current projection happens to contain no altered native words.
        Ok(changed.union(&damage))
    }
    /// Upstream pen tap may raise its only point to peak pressure at release.
    /// Rebuild this one point from the original captured brush/PRE, never tools.
    pub fn replace_single_point_for_session_with_interrupt(&mut self,s:&Session,point:StrokePoint,limits:StrokeLimits,c:&Interrupt<'_>)->Result<Rect> {
        validate_preview(s,&self.capture,c)?;
        let old=self.points.first().filter(|_|self.points.len()==1).ok_or_else(||bad("native tap replacement requires exactly one point"))?;
        let mut normalized=point;normalized.pressure=old.pressure;
        if !same_points(std::slice::from_ref(old),std::slice::from_ref(&normalized))||!point.pressure.is_finite()||!(0.0..=1.0).contains(&point.pressure){return Err(bad("native tap replacement changes more than pressure"));}
        let(generation,_)=self.quotas(limits)?;
        let generator=NativeReplacementGenerator::new(&self.generator.ctx.brush,generation,c).map_err(paint)?;
        let mut next=Self{doc:self.capture.snapshot.clone(),seed:self.seed,capture:self.capture.clone(),generator,state:NativeLuminosityState::new(),settings:self.settings,points:Vec::new(),raw_foreground:self.raw_foreground,raw_background:self.raw_background,remaining:self.remaining,bounds:Rect::EMPTY,tail_bounds:Rect::EMPTY,presentation_work:self.presentation_work};
        let damage=next.push_for_session_with_interrupt(s,std::slice::from_ref(&point),limits,c)?.union(&self.bounds);
        validate_preview(s,&self.capture,c)?;poll(c)?;*self=next;Ok(damage)
    }
    /// Consumes captured state; no original document mutation. Root offers ticket
    /// to the registered command, which validates settings before one Undo step.
    pub fn finish_for_session(mut self,s:&Session,limits:StrokeLimits,c:&Interrupt<'_>)->Result<NativeLuminosityCommit> {
        validate_preview(s,&self.capture,c)?;let(generation,lower)=self.quotas(limits)?;
        let dabs=self.generator.delta_with_interrupt(&[],true,generation,c).map_err(paint)?;
        let pre=self.capture.snapshot.layer(self.capture.id).and_then(|l|l.surface()).ok_or_else(||bad("native preview PRE missing"))?;
        self.state.apply_dabs_with_interrupt(pre,&self.generator.ctx,&dabs,&self.settings,lower,c).map_err(paint)?;
        let(output,damage)=self.state.output_with_interrupt(pre,self.capture.snapshot.selection.as_ref(),self.capture.lock,&self.settings,lower,c).map_err(paint)?;
        let response=if damage.is_empty(){json!({"damage":[0,0,0,0],"nativeLuminosity":true})}else{json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()],"nativeLuminosity":true})};
        validate_preview(s,&self.capture,c)?;poll(c)?;
        Ok(NativeLuminosityCommit{candidate:Candidate{capture:self.capture,output,damage,response},brush:self.generator.ctx.brush,settings:self.settings,foreground:self.raw_foreground,background:self.raw_background,points:self.points,seed:self.seed})
    }
}
impl NativeLuminosityCommit {
    pub(crate) fn validate_for_session(&self,s:&Session)->Result<()> {validate_preview(s,&self.candidate.capture,&Interrupt::NONE)}
    pub(super) fn into_candidate_for_command(self,s:&Session,p:&Value,c:&Interrupt<'_>)->Result<Candidate> {
        self.validate_for_session(s)?;let Some(input)=capture(s,p,c)?else{return Err(bad("native preview commit domain changed"));};
        if p.get("seed").and_then(Value::as_u64)!=Some(self.seed)||input.brush!=self.brush||!same_points(&self.points,&input.points)||!same_color(input.foreground,self.foreground)
            ||(self.settings.sampling==Sampling::BackgroundSwatch&&!same_color(input.background,self.background))
            ||input.settings.sampling!=self.settings.sampling||input.settings.limits!=self.settings.limits||input.settings.anti_alias!=self.settings.anti_alias||input.settings.tolerance.to_bits()!=self.settings.tolerance.to_bits()||input.settings.opacity.to_bits()!=self.settings.opacity.to_bits(){return Err(bad("native preview commit stroke/settings changed"));}
        poll(c)?;Ok(self.candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(sample:SampleType)->Session {
        let mut doc=photocraft_doc::Document::new("Native preview",photocraft_doc::Size::new(8,8),ColorMode::Rgb,sample);
        doc.sample_encoding=Some(photocraft_doc::SampleEncoding::Linear);doc.icc_profile=Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        doc.layers.push(photocraft_doc::Layer::new("Pixels",LayerContent::Raster(photocraft_raster::Surface::with_default(doc.pixel_format(),&[0.5,0.5,0.5,0.75]))));
        let mut s=Session::new();s.add_document(doc,None).unwrap();s.tools.brush.pressure_size=false;s.tools.brush.smoothing.enabled=false;s
    }
    fn params()->Value {json!({"mode":"luminosity","points":[[2,2,1],[5,5,1]],"seed":21,"color":[0.8,0.8,0.8,1],"size":4,"flow":0.4,"smoothing":0,"sampling":"once","limits":"discontiguous","tolerance":100})}
    #[test]
    fn offered_preview_matches_registered_one_shot_and_one_undo() {
        for sample in [SampleType::U32,SampleType::F64]{let mut expected=fixture(sample);let mut actual=fixture(sample);let p=params();expected.execute("paint.colorReplacement",p.clone()).unwrap();
            let mut initial=p.clone();initial["points"]=json!([[2,2,1]]);let mut preview=try_native_color_replacement_preview(&actual,&initial,&Interrupt::NONE).unwrap().unwrap();
            preview.push_for_session_with_interrupt(&actual,&[StrokePoint::new(5.0,5.0,1.0)],StrokeLimits::default(),&Interrupt::NONE).unwrap();
            let before=actual.active().unwrap().doc.clone();let revision=actual.active().unwrap().revision;
            let commit=preview.finish_for_session(&actual,StrokeLimits::default(),&Interrupt::NONE).unwrap();actual.offer_native_color_replacement(commit).unwrap();actual.execute("paint.colorReplacement",p).unwrap();
            assert_eq!(actual.active().unwrap().doc.layers.first().unwrap().surface(),expected.active().unwrap().doc.layers.first().unwrap().surface());assert_eq!(actual.active().unwrap().revision,revision+1);
            assert!(actual.undo());assert_eq!(&*actual.active().unwrap().doc,&*before);assert!(actual.redo());
        }
    }
    #[test]
    fn stale_view_or_cancel_keeps_preview_and_source() {
        let mut s=fixture(SampleType::F64);let p=params();let mut preview=try_native_color_replacement_preview(&s,&p,&Interrupt::NONE).unwrap().unwrap();let original=s.active().unwrap().doc.clone();let displayed=preview.doc.clone();
        let cancel=||true;assert!(matches!(preview.push_for_session_with_interrupt(&s,&[StrokePoint::new(7.0,7.0,1.0)],StrokeLimits::default(),&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));assert!(preview.doc.shares_state(&displayed));
        s.active_mut().unwrap().channel_view.target=crate::channel_cmds::ChannelTarget::Color(1);assert!(preview.finish_for_session(&s,StrokeLimits::default(),&Interrupt::NONE).is_err());assert!(s.active().unwrap().doc.shares_state(&original));
    }
}
