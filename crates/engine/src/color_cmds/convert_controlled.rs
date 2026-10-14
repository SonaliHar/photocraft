//! Controlled jobs for the existing CMS color conversion lane.
//! Destination profiles use the existing ColorState resolver, never stored locators.
use super::{convert_document_with_prepared_profiles, validate_analytic_xcf_profile_change_with_interrupt, space_mode};
#[cfg(test)]
use super::convert_document_with_interrupt;
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_cms::{Intent, Profile};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, Surface, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value,json};
use std::sync::Arc;
const MAX_ITEMS:usize=100_000;
fn error(message:impl std::fmt::Display)->EngineError {EngineError::Other(format!("color conversion: {message}"))}
fn poll(control:&Interrupt<'_>)->Result<()> {control.check().map_err(|_|EngineError::Cancelled)}
fn mode(id:&str)->Option<ColorMode> {match id {"image.mode.rgb"=>Some(ColorMode::Rgb),"image.mode.grayscale"=>Some(ColorMode::Grayscale),"image.mode.cmyk"=>Some(ColorMode::Cmyk),"image.mode.lab"=>Some(ColorMode::Lab),_=>None}}
fn exact(value:&Value,nonzero:bool)->Result<u64> {
    let text=value.as_str().filter(|s|!s.is_empty() && s.len()<=20 && s.bytes().all(|b|b.is_ascii_digit()) && (s.len()==1 || !s.starts_with('0'))).ok_or_else(||error("captured target must use canonical decimal strings"))?;
    let number=text.parse::<u64>().map_err(|_|error("captured target exceeds range"))?;
    if nonzero && number==0 {return Err(error("documentId must be nonzero"));} Ok(number)
}
fn context(session:&Session,p:&Value,required_profile:bool,allow_legacy_normalization:bool)->Result<()> {
    if !p.is_null() && !p.as_object().is_some_and(|p|p.len()<=5 && p.keys().all(|k|matches!(k.as_str(),"profile"|"intent"|"bpc"|"documentId"|"expectedRevision"))) {
        return Err(error("expected bounded profile/intent/bpc and optional captured target"));
    }
    if let Some(profile)=p.get("profile") {
        if !profile.as_str().is_some_and(|s|!s.is_empty() && s.len()<=4096 && !s.as_bytes().contains(&0)) {return Err(error("profile must be a bounded nonempty string"));}
    } else if required_profile {return Err(EngineError::BadParams{cmd:"edit.convertToProfile".into(),msg:"missing `profile`".into()});}
    if let Some(intent)=p.get("intent") {
        if !intent.as_str().is_some_and(|s|s.len()<=32 && Intent::parse(s).is_some()) {return Err(error("intent must be perceptual, relative, saturation or absolute"));}
    }
    if p.get("bpc").is_some_and(|v|!v.is_boolean()) {return Err(error("bpc must be boolean"));}
    let state=session.active().ok_or(EngineError::NoDocument)?;
    match(p.get("documentId"),p.get("expectedRevision")) {
        (None,None)=>{},
        (Some(id),Some(revision)) if exact(id,true)?==state.doc.id.0 && exact(revision,false)?==state.revision=>{},
        _=>return Err(error("captured target changed or incomplete")),
    }
    if state.revision==u64::MAX || (state.floating.is_some() && (!allow_legacy_normalization || p.get("documentId").is_some())) || state.doc.xcf_state.as_ref().is_some_and(|s|s.floating.is_some()) || state.coalesce.is_some()
        || session.coalesce_request.is_some() || session.color_restrict.is_some() || session.has_jobs() || session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(error("finish transient edits or pending work before converting color"));
    }
    if state.doc.accounted().is_some() {return Err(error("accounted color conversion is unsupported"));} Ok(())
}
pub(super) fn dispatch_preflight(session:&Session,id:&str,p:&Value,_background:bool)->Result<bool> {
    if mode(id).is_none() && id!="edit.convertToProfile" {return Ok(false);}
    context(session,p,id=="edit.convertToProfile",true)?;Ok(true)
}
fn visit_layers(layers:&[Layer],level:usize,count:&mut usize,control:&Interrupt<'_>,visit:&mut dyn FnMut(&Surface)->Result<()>)->Result<()> {
    if level>photocraft_doc::MAX_GROUP_DEPTH {return Err(error("layer depth limit"));}
    for layer in layers {
        poll(control)?;*count=count.checked_add(1).filter(|n|*n<=MAX_ITEMS).ok_or_else(||error("layer count limit"))?;
        if let Some(cache)=&layer.fill_cache {visit(&cache.surface)?;}
        match &layer.content {
            LayerContent::Raster(surface)=>visit(surface)?,
            LayerContent::Text(text)=>if let Some(surface)=&text.cache {visit(surface)?;},
            LayerContent::Shape(shape)=>if let Some(surface)=&shape.cache {visit(surface)?;},
            LayerContent::Smart(smart)=>if let Some(surface)=&smart.cache {visit(surface)?;},
            LayerContent::Group(group)=>visit_layers(&group.children,level+1,count,control,visit)?,
            LayerContent::BackedRaster(_)=>return Err(error("backed raster color conversion is unsupported")),
            _=>{},
        }
    } Ok(())
}

#[derive(Clone,Copy)]
pub(super) struct ProfilePreparation {
    pub peak_bytes:usize,
    pub work_units:u64,
    pub source_limits:photocraft_cms::control::Limits,
    pub destination_limits:photocraft_cms::control::Limits,
}
fn cms_error(error:photocraft_cms::control::Error)->EngineError {
    match error {photocraft_cms::control::Error::Cancelled=>EngineError::Cancelled,photocraft_cms::control::Error::Cms(e)=>super::cms_err(e),e=>self::error(e)}
}
/// Borrowed complete profile-phase reservation before model/output cloning.
/// CMYK/Lab retain their existing compatibility provider boundary; no new policy.
pub(super) fn profile_preparation(doc:&photocraft_doc::Document,dst:&Profile,limits:photocraft_cms::control::Limits,control:&Interrupt<'_>)->Result<ProfilePreparation> {
    poll(control)?;let maximum=photocraft_cms::control::Limits::default();
    if limits.resident_bytes>maximum.resident_bytes||limits.work_units>maximum.work_units{return Err(error("CMS limits may only be lowered"));}
    let cancelled=||control.cancelled();
    let source=if matches!(doc.mode,ColorMode::Rgb|ColorMode::Grayscale|ColorMode::Indexed){Some(super::document_profile_requirements_with_limits(doc,limits,control)?)}else{None};
    let retained=dst.clone_requirements_with_control(limits,&cancelled).map_err(cms_error)?.retained_bytes;
    let(encoded_peak,encoding_work)=if dst.encoded_bytes().is_some(){(retained,0)}else{let req=dst.encode_requirements_with_control(limits,&cancelled).map_err(cms_error)?;(req.peak_resident_bytes,req.work_units)};
    let source_peak=source.map_or(0,|v|v.peak_resident_bytes);let source_result=source.map_or(0,|v|v.result_retained_bytes_upper_bound);let source_work=source.map_or(0,|v|v.work_units);
    // Conservative simultaneous ownership: destination model during source prep,
    // source result during destination encoding, and all phase census repeats.
    let peak=source_peak.checked_add(encoded_peak).and_then(|v|v.checked_add(source_result)).ok_or_else(||error("profile phase peak overflow"))?;
    let work=source_work.checked_add(encoding_work).and_then(|v|v.checked_mul(3)).ok_or_else(||error("profile phase work overflow"))?;
    if peak>limits.resident_bytes||work>limits.work_units{return Err(error("aggregate source/destination profile preparation limit"));}
    let source_limits=photocraft_cms::control::Limits{resident_bytes:limits.resident_bytes.checked_sub(retained).ok_or_else(||error("destination retained profile limit"))?,work_units:limits.work_units.checked_sub(encoding_work).ok_or_else(||error("destination encoding work limit"))?};
    let destination_limits=photocraft_cms::control::Limits{resident_bytes:limits.resident_bytes.checked_sub(source_result).ok_or_else(||error("source result retained profile limit"))?,work_units:limits.work_units.checked_sub(source_work).ok_or_else(||error("source profile work limit"))?};
    poll(control)?;Ok(ProfilePreparation{peak_bytes:peak,work_units:work,source_limits,destination_limits})
}

pub(super) struct PreparedConversion {
    pub transform:photocraft_cms::Transform,
    pub profile_bytes:Arc<Vec<u8>>,
}
/// Prepare all owned profile/transform state before the caller clones pixel/model
/// output. Earlier phase work and simultaneously live bytes remain reserved.
pub(super) fn prepare_profiles(doc:&photocraft_doc::Document,dst:&Profile,intent:Intent,bpc:bool,limits:photocraft_cms::control::Limits,control:&Interrupt<'_>)->Result<PreparedConversion> {
    let phases=profile_preparation(doc,dst,limits,control)?;
    let source=if matches!(doc.mode,ColorMode::Rgb|ColorMode::Grayscale|ColorMode::Indexed){super::try_document_profile_with_limits(doc,phases.source_limits,control)?}else{
        // Preserve the old CMYK/Lab compatibility provider boundary; the owned
        // resulting definition is admitted before controlled linking/copying.
        poll(control)?;let profile=super::document_profile(doc);poll(control)?;profile
    };
    let profile_bytes=dst.to_bytes_with_control(phases.destination_limits,&||control.cancelled()).map_err(cms_error)?;
    let outside=if dst.encoded_bytes().is_none(){profile_bytes.capacity()}else{0};
    let constructor_limits=photocraft_cms::control::Limits{
        resident_bytes:limits.resident_bytes.checked_sub(outside).ok_or_else(||error("destination encoded owner headroom"))?,
        work_units:limits.work_units.checked_sub(phases.work_units).ok_or_else(||error("profile preparation consumed constructor work"))?,
    };
    let options=photocraft_cms::transform::TransformOptions{intent,bpc,..Default::default()};let cancelled=||control.cancelled();
    let required=photocraft_cms::Transform::requirements_with_control(&source,dst,options,constructor_limits,&cancelled).map_err(cms_error)?;
    required.peak_resident_bytes.checked_add(outside).filter(|v|*v<=limits.resident_bytes).ok_or_else(||error("aggregate source/destination/transform headroom"))?;
    required.work_units.checked_add(phases.work_units).filter(|v|*v<=limits.work_units).ok_or_else(||error("aggregate profile/transform work"))?;
    let transform=photocraft_cms::Transform::with_options_with_control(&source,dst,options,constructor_limits,&cancelled).map_err(cms_error)?;
    poll(control)?;Ok(PreparedConversion{transform,profile_bytes})
}
fn prepare(snapshot:&DocumentSnapshot,dst:&Profile,intent:Intent,bpc:bool,control:&Interrupt<'_>)->Result<PreparedConversion> {
    poll(control)?;snapshot.validate_resident()?;
    snapshot.validate_xcf_state_with(control).map_err(|e|if control.cancelled(){EngineError::Cancelled}else{error(e)})?;
    validate_analytic_xcf_profile_change_with_interrupt(snapshot,Some(dst),control)?;
    let to_mode=space_mode(dst.color_space).ok_or_else(||error("unsupported destination profile color space"))?;
    let preparation=photocraft_io::NativeSaveLimits::default().preparation;
    let payload=photocraft_io::retained_native_payload_bytes(snapshot,preparation,control).map_err(|e|if control.cancelled(){EngineError::Cancelled}else{error(e)})? as u64;
    let model=photocraft_io::retained_native_model_bytes(snapshot,preparation,control).map_err(|e|if control.cancelled(){EngineError::Cancelled}else{error(e)})? as u64;
    let raster=snapshot.resident_raster_footprint_with(control)?;
    // No destination encoding is performed by this borrowed admission pass.
    let mut retained=model.checked_mul(5).and_then(|n|n.checked_add(payload)).and_then(|n|n.checked_add(raster.retained_bytes))
        .and_then(|n|n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .filter(|n|*n<=MAX_CHECKED_RASTER_BYTES).ok_or_else(||error("model/payload/COW headroom limit"))?;
    let mut work=0u64;
    let mut visit=|surface:&Surface|->Result<()> {
        poll(control)?;surface.try_validate_tile_origins_with(control).map_err(|e|if control.cancelled(){EngineError::Cancelled}else{error(e)})?;
        let format=PixelFormat::new(to_mode,surface.format().sample,surface.format().alpha);
        let tiles=surface.tile_count() as u64;
        let tile_bytes=(TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n|n.checked_mul(format.bytes_per_pixel() as u64)).ok_or_else(||error("tile output length overflow"))?;
        let output=tiles.checked_mul(tile_bytes.checked_add(512).ok_or_else(||error("tile map overflow"))?).and_then(|n|n.checked_add(format.bytes_per_pixel() as u64)).ok_or_else(||error("output budget overflow"))?;
        retained=retained.checked_add(output.checked_mul(3).ok_or_else(||error("conversion peak overflow"))?).filter(|n|*n<=MAX_CHECKED_RASTER_BYTES).ok_or_else(||error("aggregate converted output limit"))?;
        work=work.checked_add(tiles.checked_mul((TILE_SIZE as u64)*(TILE_SIZE as u64)).and_then(|n|n.checked_mul(format.channels() as u64)).ok_or_else(||error("sample work overflow"))?)
            .filter(|n|*n<=1<<32).ok_or_else(||error("sample work limit"))?;Ok(())
    };
    visit_layers(&snapshot.layers,0,&mut 0,control,&mut visit)?;
    let maximum=photocraft_cms::control::Limits::default();
    let remaining=MAX_CHECKED_RASTER_BYTES.checked_sub(retained).ok_or_else(||error("profile preparation resident limit"))?;
    let profile_limits=photocraft_cms::control::Limits{
        resident_bytes:usize::try_from(remaining).map_err(error)?.min(maximum.resident_bytes),
        work_units:(1u64<<32).checked_sub(work).ok_or_else(||error("profile preparation work limit"))?.min(maximum.work_units),
    };
    let profiles=profile_preparation(snapshot,dst,profile_limits,control)?;
    retained.checked_add(profiles.peak_bytes as u64).filter(|v|*v<=MAX_CHECKED_RASTER_BYTES).ok_or_else(||error("aggregate profile/model/output headroom limit"))?;
    work.checked_add(profiles.work_units).filter(|v|*v<=1<<32).ok_or_else(||error("aggregate profile/sample work limit"))?;
    // The returned owned bundle is prepaid under the same remaining envelope;
    // no source profile or Transform is rebuilt after the Document clone.
    let prepared=prepare_profiles(snapshot,dst,intent,bpc,profile_limits,control)?;
    poll(control)?;Ok(prepared)
}
fn run(session:&mut Session,dst:Arc<Profile>,intent:Intent,bpc:bool,label:&'static str,include_mode:bool)->Result<Value> {
    let state=session.active().ok_or(EngineError::NoDocument)?;
    let baseline=state.doc.clone();let revision=state.revision;let weak=baseline.downgrade();let work_base=baseline.clone();
    crate::jobs::run(session,label,true,move|ctx|ctx.stage(0.0,0.95,label,|control| {
        let profiles=prepare(&work_base,&dst,intent,bpc,control)?;poll(control)?;
        let mut document=(*work_base).clone();
        convert_document_with_prepared_profiles(&mut document,&dst,intent,bpc,&profiles,control)?;poll(control)?;
        let response=if include_mode {json!({"profile":dst.description,"mode":format!("{:?}",document.mode),"intent":intent.id(),"bpc":bpc})}
            else {json!({"profile":dst.description,"intent":intent.id(),"bpc":bpc})};
        Ok((document,response,ctx.clone()))
    }),move|session,(document,response,ctx)| {
        let current=session.active().ok_or(EngineError::NoDocument)?;
        if current.doc.id!=baseline.id || current.revision!=revision || !weak.is_current(&current.doc) || !current.doc.shares_state(&baseline)
            || current.floating.is_some() || current.doc.xcf_state.as_ref().is_some_and(|s|s.floating.is_some()) || current.coalesce.is_some()
            || session.coalesce_request.is_some() || session.color_restrict.is_some() {return Err(error("target changed while preparing color conversion"));}
        ctx.stage(0.95,1.0,label,|control| {poll(control)?;session.edit_with_interrupt(label,control,move|current,_|{poll(control)?;*current=document;Ok(())})?;Ok(response)})
    })
}
pub(super) fn convert_mode(session:&mut Session,mode:ColorMode,p:&Value)->Result<Value> {
    context(session,p,false,false)?;
    if !matches!(mode,ColorMode::Rgb|ColorMode::Grayscale|ColorMode::Cmyk|ColorMode::Lab) {return Err(error("unsupported controlled mode"));}
    let document=&session.active().ok_or(EngineError::NoDocument)?.doc;
    // The parent's existing Multichannel plate-reinterpretation branch remains
    // separate; this helper must not invent an equivalent CMS transform.
    if document.mode==ColorMode::Multichannel {return Err(error("Multichannel requires its existing dedicated conversion route"));}
    if document.mode==mode && p.get("profile").is_none() {return Ok(Value::Null);}
    let dst=match p.get("profile").and_then(Value::as_str) {Some(spec)=>session.color.resolve(spec,Some(document),Some(mode))?,None=>session.color.working(mode)};
    if space_mode(dst.color_space)!=Some(mode) {return Err(error(format!("profile `{}` is {:?}, not {mode:?}",dst.description,dst.color_space)));}
    let intent=super::intent_or(p,session.color.settings.intent())?;let bpc=super::bool_param(p,"bpc",session.color.settings.bpc);
    run(session,dst,intent,bpc,"Mode Change",false)
}
pub(super) fn convert_to_profile(session:&mut Session,p:&Value)->Result<Value> {
    context(session,p,true,false)?;
    let document=&session.active().ok_or(EngineError::NoDocument)?.doc;
    let spec=p.get("profile").and_then(Value::as_str).ok_or_else(||error("missing profile"))?;
    let dst=session.color.resolve(spec,Some(document),None)?;
    let intent=super::intent_or(p,session.color.settings.intent())?;let bpc=super::bool_param(p,"bpc",session.color.settings.bpc);
    run(session,dst,intent,bpc,"Convert to Profile",true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_legacy_color_float_can_normalize_but_captured_target_refuses() {
        let mut session=Session::new();session.execute("file.new",json!({"width":2,"height":2,"background":"#123456"})).unwrap();
        session.execute("select.all",Value::Null).unwrap();session.execute("select.float",json!({"dx":1,"dy":0,"copy":true})).unwrap();
        let state=session.active().unwrap();assert!(state.floating.is_some());
        let captured=json!({"documentId":state.doc.id.0.to_string(),"expectedRevision":state.revision.to_string()});
        assert!(dispatch_preflight(&session,"image.mode.grayscale",&Value::Null,false).unwrap());
        assert!(dispatch_preflight(&session,"image.mode.grayscale",&captured,false).is_err());
        assert!(context(&session,&Value::Null,false,false).is_err());assert!(session.active().unwrap().floating.is_some());
    }
    #[test]
    fn same_mode_keeps_real_redo_and_malformed_selectors_refuse() {
        let mut session=Session::new();session.execute("file.new",json!({"width":1,"height":1})).unwrap();
        session.edit("name",|d,_|{d.name="undo seed".into();Ok(())}).unwrap();assert!(session.undo());
        let revision=session.active().unwrap().revision;
        convert_mode(&mut session,ColorMode::Rgb,&Value::Null).unwrap();assert_eq!(session.active().unwrap().revision,revision);assert!(session.redo());
        for p in [json!({"profile":false}),json!({"intent":null}),json!({"bpc":1}),json!({"documentId":"01","expectedRevision":"0"})] {
            assert!(context(&session,&p,false,false).is_err());
        }
    }
    #[test]
    fn nonfinite_native_color_conversion_keeps_private_source_and_metadata_exact() {
        let mut doc=photocraft_doc::Document::new("Own nonfinite color",photocraft_doc::Size::new(1,1),ColorMode::Rgb,photocraft_color::SampleType::F64);
        let raw=[0x7ff8_0000_0000_1234u64,(-0.0f64).to_bits(),0.5f64.to_bits(),1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        doc.layers.push(Layer::new("Native raw",LayerContent::Raster(Surface::try_with_default_bytes(photocraft_color::PixelFormat::RGBA64F,&raw).unwrap())));
        let before=doc.clone();let dst=super::super::working_profile(ColorMode::Grayscale);
        assert!(convert_document_with_interrupt(&mut doc,&dst,Intent::RelativeColorimetric,true,&Interrupt::NONE).is_err());
        assert_eq!(doc,before);
    }
    #[test]
    fn controlled_preparation_cancels_without_profile_metadata_or_source_mutation() {
        let mut session=Session::new();session.execute("file.new",json!({"width":1,"height":1})).unwrap();
        let original=session.active().unwrap().doc.clone();let cancel=||true;
        let dst=session.color.working(ColorMode::Grayscale);
        assert!(matches!(prepare(&original,&dst,Intent::RelativeColorimetric,true,&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));
        assert!(original.shares_state(&session.active().unwrap().doc));
        assert_eq!(original.icc_profile,session.active().unwrap().doc.icc_profile);
        assert_eq!(original.physical_resolution,session.active().unwrap().doc.physical_resolution);
    }
    #[test]
    fn source_provider_and_uncached_destination_preserve_legacy_conversion_words() {
        use photocraft_cms::Builtin;
        use photocraft_doc::{Document,SampleEncoding};
        use photocraft_color::SampleType;
        let mut input=Document::new("profile phase",photocraft_geom::Size::new(1,1),ColorMode::Rgb,SampleType::F64);
        input.icc_profile=Some(Arc::new(Builtin::DisplayP3.profile().to_bytes().as_ref().clone()));
        input.sample_encoding=Some(SampleEncoding::NonlinearProfile);
        let raw=[0.125f64,0.5,0.75,1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
        input.layers.push(Layer::new("plain",LayerContent::Raster(Surface::try_with_default_bytes(photocraft_color::PixelFormat::RGBA64F,&raw).unwrap())));
        let mut destination=Builtin::SGray.profile().gray_as_rgb().unwrap();
        destination.matrix=Builtin::Srgb.profile().matrix;destination.trc=Builtin::Srgb.profile().trc.clone();
        assert!(destination.encoded_bytes().is_none());
        let bytes=destination.to_bytes();let original=input.clone();
        let mut legacy=input.clone();let mut actual=input.clone();
        super::super::convert_document(&mut legacy,&destination,Intent::RelativeColorimetric,true).unwrap();
        convert_document_with_interrupt(&mut actual,&destination,Intent::RelativeColorimetric,true,&Interrupt::NONE).unwrap();
        assert_eq!(actual,legacy);assert_eq!(actual.icc_profile.as_ref().unwrap().as_slice(),bytes.as_slice());assert_eq!(input,original);
    }
    #[test]
    fn profile_phase_limit_and_late_cancel_preserve_private_document() {
        use std::cell::Cell;
        let mut input=photocraft_doc::Document::new("private profile",photocraft_geom::Size::new(1,1),ColorMode::Rgb,photocraft_color::SampleType::U32);
        input.icc_profile=Some(Arc::new(photocraft_cms::Builtin::DisplayP3.profile().to_bytes().as_ref().clone()));let original=input.clone();
        let destination=photocraft_cms::Builtin::SGray.profile();
        assert!(profile_preparation(&input,destination,photocraft_cms::control::Limits{resident_bytes:1,work_units:1},&Interrupt::NONE).is_err());assert_eq!(input,original);
        let visits=Cell::new(0);let cancel=||{visits.set(visits.get()+1);visits.get()>80};
        assert!(matches!(convert_document_with_interrupt(&mut input,destination,Intent::RelativeColorimetric,true,&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));assert_eq!(input,original);
    }

    #[test]
    fn owned_constructor_bundle_preserves_requested_intent_bpc_and_legacy_pipeline() {
        let mut input=photocraft_doc::Document::new("owned transform",photocraft_geom::Size::new(1,1),ColorMode::Rgb,photocraft_color::SampleType::F64);
        input.icc_profile=Some(Arc::new(photocraft_cms::Builtin::DisplayP3.profile().to_bytes().as_ref().clone()));let before=input.clone();let destination=photocraft_cms::Builtin::Srgb.profile();
        for intent in [Intent::Perceptual,Intent::RelativeColorimetric,Intent::Saturation,Intent::AbsoluteColorimetric]{for bpc in [false,true]{let source=super::super::try_document_profile(&input).unwrap();let expected=photocraft_cms::Transform::new(&source,destination,intent,bpc).unwrap();let prepared=prepare_profiles(&input,destination,intent,bpc,photocraft_cms::control::Limits::default(),&Interrupt::NONE).unwrap();assert_eq!(prepared.transform.pipeline(),expected.pipeline());assert_eq!(prepared.transform.options(),expected.options());assert_eq!(prepared.profile_bytes,destination.to_bytes());}}
        assert_eq!(input,before);
    }

}
