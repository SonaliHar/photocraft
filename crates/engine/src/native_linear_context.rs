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
