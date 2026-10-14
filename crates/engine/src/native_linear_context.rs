//! Explicit native Linear working context; no transfer/profile fallback or conversion.
use photocraft_doc::{Document,SampleEncoding,WorkingProfileInterpretation};
use photocraft_color::ColorMode;
use photocraft_raster::Interrupt;

pub(crate) fn validate(doc:&Document,control:&Interrupt<'_>)->std::result::Result<(),String>{
    control.check().map_err(|_|"native Linear context interrupted".to_string())?;
    if doc.sample_encoding!=Some(SampleEncoding::Linear){return Err("native arithmetic requires explicit Linear sample encoding".into());}
    let bytes=doc.icc_profile.as_ref().filter(|bytes|!bytes.is_empty()&&bytes.len()<=16<<20).ok_or_else(||"native arithmetic requires a bounded explicit current ICC identity".to_string())?;
    doc.working_profile_interpretation.validate_with_interrupt(Some(bytes),doc.mode,control).map_err(|error|error.to_string())?;
    let space=match doc.mode{ColorMode::Rgb=>photocraft_cms::ColorSpace::Rgb,ColorMode::Grayscale=>photocraft_cms::ColorSpace::Gray,_=>return Err("native Linear color model unsupported".into())};
    let encoding=photocraft_cms::xcf_color::Encoding::Linear;
    match &doc.working_profile_interpretation{
        WorkingProfileInterpretation::Icc=>{photocraft_cms::xcf_color::effective_profile(Some(bytes),space,encoding).map_err(|error|error.to_string())?;},
        _=>{photocraft_cms::xcf_color::effective_implicit_profile(bytes,space,encoding).map_err(|error|error.to_string())?;},
    }
    control.check().map_err(|_|"native Linear context interrupted".to_string())
}

/// Borrowed strict encoded gate before private profile parsing/copying. The
/// provider still completes effective-profile and implicit-definition validation.
pub(crate) fn gate_requirements_with_limits(doc:&Document,limits:photocraft_cms::control::Limits,control:&Interrupt<'_>)->crate::Result<photocraft_cms::xcf_color::EncodedProfileGateRequirements>{
    let error=|message:&str|crate::EngineError::Other(message.into());
    control.check().map_err(|_|crate::EngineError::Cancelled)?;
    let maximum=photocraft_cms::control::Limits::default();
    if limits.resident_bytes>maximum.resident_bytes||limits.work_units>maximum.work_units{return Err(error("native Linear profile limits are lower-only"));}
    if doc.sample_encoding!=Some(SampleEncoding::Linear){return Err(error("native arithmetic requires explicit Linear sample encoding"));}
    if !matches!(doc.mode,ColorMode::Rgb|ColorMode::Grayscale){return Err(error("native Linear color model unsupported"));}
    let bytes=doc.icc_profile.as_ref().filter(|bytes|!bytes.is_empty()&&bytes.len()<=16<<20).ok_or_else(||error("native arithmetic requires a bounded explicit current ICC identity"))?;
    if bytes.capacity()>limits.resident_bytes{return Err(error("native current ICC retained budget"));}
    let cancelled=||control.cancelled();let map=|e:photocraft_cms::control::Error|match e{photocraft_cms::control::Error::Cancelled=>crate::EngineError::Cancelled,e=>crate::EngineError::Other(e.to_string())};
    let gate=photocraft_cms::xcf_color::encoded_profile_gate_requirements_with_control(bytes,limits,&cancelled).map_err(map)?;
    let work=gate.work_units.checked_mul(2).and_then(|n|n.checked_add((bytes.len() as u64).checked_mul(16)?)).ok_or_else(||error("native encoded ICC work overflow"))?;
    if work>limits.work_units{return Err(error("native encoded ICC work limit"));}
    doc.working_profile_interpretation.validate_with_interrupt(Some(bytes),doc.mode,control).map_err(|e|if control.cancelled(){crate::EngineError::Cancelled}else{crate::EngineError::Other(e.to_string())})?;
    photocraft_cms::xcf_color::validate_encoded_profile_with_control(bytes,limits,&cancelled).map_err(map)?;
    control.check().map_err(|_|crate::EngineError::Cancelled)?;
    Ok(photocraft_cms::xcf_color::EncodedProfileGateRequirements{retained_bytes:bytes.capacity(),work_units:work})
}
