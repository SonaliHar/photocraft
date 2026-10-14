//! Strict Linear native color preparation under the caller's brush envelope.
//! Other encodings/numeric targets keep their existing compatibility conversion.
//! Transform construction and builtin initialization are prepaid boundaries;
//! this API does not claim cancellation inside those legacy operations.
use super::*;
use photocraft_raster::Interrupt;
use photocraft_paint::stroke_controlled::StrokeLimits;
use photocraft_cms::{Builtin,Intent,Transform,control::Limits};
use photocraft_color::{ColorMode,PixelFormat};
fn poll(c:&Interrupt<'_>)->Result<()>{c.check().map_err(|_|EngineError::Cancelled)}
fn fail(m:impl Into<String>)->EngineError{bad("paint.stroke",m)}
fn add(a:usize,b:usize)->Result<usize>{a.checked_add(b).ok_or_else(||fail("native profile size overflow"))}
fn mul(a:usize,b:usize)->Result<usize>{a.checked_mul(b).ok_or_else(||fail("native profile size overflow"))}
fn map(e:photocraft_cms::control::Error)->EngineError{match e{photocraft_cms::control::Error::Cancelled=>EngineError::Cancelled,e=>fail(e.to_string())}}
pub(super) struct Requirements{pub peak_resident_bytes:u64,pub work_units:u64,provider_limits:Limits,profile_upper:usize,transform_reserve:usize}
pub(super) fn eligible(doc:&photocraft_doc::Document,p:&Value)->bool{
    crate::channel_cmds::target_of(p)==crate::channel_cmds::Target::Pixels&&doc.sample_encoding==Some(photocraft_doc::SampleEncoding::Linear)&&doc.icc_profile.as_ref().is_some_and(|b|!b.is_empty()&&b.len()<=16<<20)
}
fn requirements(doc:&photocraft_doc::Document,format:PixelFormat,limits:StrokeLimits,c:&Interrupt<'_>)->Result<Requirements>{
    poll(c)?;let standard=StrokeLimits::default();if limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units{return Err(fail("native color limits are lower-only"));}
    if !matches!(format.mode,ColorMode::Rgb|ColorMode::Grayscale)||format.mode!=doc.mode{return Err(fail("assigned profile does not match wide paint model"));}
    let maximum=Limits::default();let cms=Limits{resident_bytes:usize::try_from(limits.resident_bytes).map_err(|_|fail("native profile headroom"))?.min(maximum.resident_bytes),work_units:limits.work_units.min(maximum.work_units)};
    // Do not replace the original strict ICC header/intent/overlap gate with the
    // general current-profile parser's compatibility behavior.
    let gate=crate::native_linear_context::gate_requirements_with_limits(doc,cms,c)?;
    let phase=Limits{resident_bytes:cms.resident_bytes,work_units:cms.work_units.checked_sub(gate.work_units).ok_or_else(||fail("native profile gate work"))?};
    let profile=crate::color_cmds::document_profile_requirements_with_limits(doc,phase,c)?;
    poll(c)?;let cancelled=||c.cancelled();
    let source=Builtin::Srgb.profile();poll(c)?;let source_req=source.clone_requirements_with_control(cms,&cancelled).map_err(map)?;
    // Strict Linear preparation refuses LUT transfer overrides. Thus linking
    // sRGB to the resulting RGB/Gray analytic definition cannot select a CLUT
    // sampling grid. Sixteen model envelopes plus 256 KiB cover copied stages,
    // curve/shaper/output tables and BPC vectors (4096 output knots/channel).
    let transform_reserve=add(mul(profile.result_retained_bytes_upper_bound,16)?,add(mul(source_req.retained_bytes,16)?,256<<10)?)?;
    let peak=add(profile.peak_resident_bytes,transform_reserve)?;
    let transform_work=(transform_reserve as u64).checked_mul(32).and_then(|n|n.checked_add(16<<20)).ok_or_else(||fail("native transform work overflow"))?;
    let outside=gate.work_units.checked_mul(2).and_then(|n|n.checked_add(transform_work)).ok_or_else(||fail("native profile outer work overflow"))?;
    let work=profile.work_units.checked_mul(3).and_then(|n|n.checked_add(outside)).ok_or_else(||fail("native color work overflow"))?;
    if peak>cms.resident_bytes||work>cms.work_units{return Err(fail("native aggregate profile/transform budget"));}
    let provider_limits=Limits{resident_bytes:cms.resident_bytes.checked_sub(transform_reserve).ok_or_else(||fail("native transform storage reservation"))?,work_units:cms.work_units.checked_sub(outside).ok_or_else(||fail("native transform work reservation"))?};
    poll(c)?;Ok(Requirements{peak_resident_bytes:peak as u64,work_units:work,provider_limits,profile_upper:profile.result_retained_bytes_upper_bound,transform_reserve})
}
pub(super) fn prepare(doc:&photocraft_doc::Document,format:PixelFormat,p:&Value,rgba:[f64;4],limits:StrokeLimits,c:&Interrupt<'_>)->Result<([f64;4],Requirements)>{
    poll(c)?;if !eligible(doc,p){return Err(fail("controlled native color requires explicit current Linear ICC"));}
    if !rgba.iter().all(|v|v.is_finite())||!(0.0..=1.0).contains(&rgba[3]){return Err(fail("invalid or unsupported native wide paint color"));}
    let req=requirements(doc,format,limits,c)?;
    let profile=crate::color_cmds::try_document_profile_with_limits(doc,req.provider_limits,c)?;
    poll(c)?;let cancelled=||c.cancelled();let actual=profile.clone_requirements_with_control(req.provider_limits,&cancelled).map_err(map)?;
    if actual.retained_bytes>req.profile_upper||add(mul(actual.retained_bytes,16)?,256<<10)?>req.transform_reserve{return Err(fail("actual native profile exceeds reserved model"));}
    if profile.color_space!=crate::color_cmds::mode_space(format.mode){return Err(fail("assigned profile does not match wide paint model"));}
    // Exact existing options: UI sRGB input, RelativeColorimetric, BPC true.
    // Legacy Transform::new/from_pipeline has no cooperative constructor API.
    poll(c)?;let transform=Transform::new(Builtin::Srgb.profile(),&profile,Intent::RelativeColorimetric,true).map_err(|e|fail(e.to_string()))?;poll(c)?;
    let n=format.mode.color_channels();let mut output=[0.0;4];
    transform.try_eval_f64(&[rgba[0],rgba[1],rgba[2]],output.get_mut(..n).ok_or_else(||fail("native channel count"))?).map_err(|e|fail(format!("wide paint color conversion is unsupported: {e}")))?;poll(c)?;
    if format.alpha{*output.get_mut(n).ok_or_else(||fail("native alpha channel"))?=rgba[3];}
    if output.iter().take(format.channels()).any(|v|!v.is_finite()){return Err(fail("invalid converted wide color"));}
    Ok((output,req))
}

#[cfg(test)]
mod tests{
    use super::*;
    use photocraft_color::SampleType;
    fn doc(mode:ColorMode)->photocraft_doc::Document{
        let mut d=photocraft_doc::Document::new("Strict native color",photocraft_doc::Size::new(8,8),mode,SampleType::F64);
        d.sample_encoding=Some(photocraft_doc::SampleEncoding::Linear);d.icc_profile=Some(match mode{ColorMode::Grayscale=>Builtin::SGray.profile().to_bytes(),_=>Builtin::DisplayP3.profile().to_bytes()});d
    }
    #[test]
    fn exact_existing_relative_bpc_pipeline_rgb_and_gray(){
        for mode in [ColorMode::Rgb,ColorMode::Grayscale]{let d=doc(mode);let before=d.clone();let rgba=[0.3,0.6,0.2,0.75];let expected=native_wide_color(&d,d.pixel_format(),&Value::Null,rgba).unwrap();let(actual,_)=prepare(&d,d.pixel_format(),&Value::Null,rgba,StrokeLimits::default(),&Interrupt::NONE).unwrap();assert_eq!(actual.map(f64::to_bits),expected.map(f64::to_bits));assert_eq!(d,before);}
    }
    #[test]
    fn strict_encoded_gate_cancel_and_lower_envelope_do_not_change_original(){
        let mut d=doc(ColorMode::Rgb);let original=d.icc_profile.clone().unwrap();let mut bad_bytes=original.as_ref().clone();bad_bytes.get_mut(64..68).unwrap().copy_from_slice(&4u32.to_be_bytes());d.icc_profile=Some(std::sync::Arc::new(bad_bytes));let before=d.clone();
        assert!(prepare(&d,d.pixel_format(),&Value::Null,[0.5;4],StrokeLimits::default(),&Interrupt::NONE).is_err());assert_eq!(d,before);
        d.icc_profile=Some(original.clone());let cancel=||true;assert!(matches!(prepare(&d,d.pixel_format(),&Value::Null,[0.5;4],StrokeLimits::default(),&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));
        assert!(prepare(&d,d.pixel_format(),&Value::Null,[0.5;4],StrokeLimits{resident_bytes:1,work_units:1},&Interrupt::NONE).is_err());assert!(std::sync::Arc::ptr_eq(d.icc_profile.as_ref().unwrap(),&original));
        d.sample_encoding=Some(photocraft_doc::SampleEncoding::NonlinearProfile);assert!(!eligible(&d,&Value::Null));assert!(!eligible(&doc(ColorMode::Rgb),&json!({"target":"mask"})));
    }
}
