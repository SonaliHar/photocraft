//! Strict current document-profile preparation. No compatibility fallback or
//! global cache insertion; original document/profile identities are never changed.
use super::{Builtin,ColorSpace,Profile,mode_space,cms_err};
use crate::{EngineError,Result};
use photocraft_cms::{control::{Limits,Error as CmsControlError},xcf_color::{self,Encoding}};
use photocraft_doc::{Document,SampleEncoding,WorkingProfileInterpretation};
use photocraft_raster::Interrupt;
use std::sync::Arc;
#[derive(Clone,Copy,Debug)]
pub struct DocumentProfileRequirements {
 pub retained_bytes:usize,
 pub result_retained_bytes_upper_bound:usize,
 pub peak_resident_bytes:usize,
 pub work_units:u64,
}
fn poll(ctl:&Interrupt<'_>)->Result<()>{ctl.check().map_err(|_|EngineError::Cancelled)}
fn fail(text:&str)->EngineError{EngineError::Other(format!("colour management: {text}"))}
fn map(error:CmsControlError)->EngineError{match error{CmsControlError::Cancelled=>EngineError::Cancelled,CmsControlError::Cms(e)=>cms_err(e),e=>fail(&e.to_string())}}
fn add(a:usize,b:usize)->Result<usize>{a.checked_add(b).ok_or_else(||fail("document profile size overflow"))}
fn mul(a:usize,b:usize)->Result<usize>{a.checked_mul(b).ok_or_else(||fail("document profile size overflow"))}
fn admit(r:DocumentProfileRequirements,limits:Limits)->Result<()>{let max=Limits::default();if limits.resident_bytes>max.resident_bytes||limits.work_units>max.work_units{return Err(fail("profile limits may only be lowered"));}if r.peak_resident_bytes>limits.resident_bytes||r.work_units>limits.work_units{return Err(fail("document profile preparation exceeds CMS storage/work limits"));}Ok(())}
fn remaining(limits:Limits,retained:usize,work:u64)->Result<Limits>{Ok(Limits{resident_bytes:limits.resident_bytes.checked_sub(retained).ok_or_else(||fail("profile retained budget"))?,work_units:limits.work_units.checked_sub(work).ok_or_else(||fail("profile work budget"))?})}
fn encoding(doc:&Document)->Encoding{match doc.sample_encoding.unwrap_or(SampleEncoding::NonlinearProfile){SampleEncoding::Linear=>Encoding::Linear,SampleEncoding::Perceptual=>Encoding::Perceptual,SampleEncoding::NonlinearProfile=>Encoding::NonlinearProfile}}
fn identity(doc:&Document,ctl:&Interrupt<'_>)->Result<()>{poll(ctl)?;doc.working_profile_interpretation.validate_with_interrupt(doc.icc_profile.as_deref().map(Vec::as_slice),doc.mode,ctl).map_err(|e|if ctl.cancelled(){EngineError::Cancelled}else{EngineError::Other(e.to_string())})?;poll(ctl)}
// Pointer-only cache peek. Never initializes/grows/prunes the global cache.
fn cached(bytes:&Arc<Vec<u8>>,ctl:&Interrupt<'_>)->Result<Option<Arc<Profile>>>{let Some(cache)=super::PARSED_PROFILE_CACHE.get()else{return Ok(None)};let entries=cache.lock().unwrap_or_else(|e|e.into_inner());if entries.len()>100_000{return Ok(None)}for(owner,p)in entries.iter(){poll(ctl)?;if owner.as_ptr()==Arc::as_ptr(bytes)&&owner.strong_count()>0{return Ok(Some(Arc::clone(p)))}}Ok(None)}
enum Definition<'a>{Cached(Arc<Profile>),Builtin(&'static Profile),Cold(&'a Arc<Vec<u8>>)}
struct Plan<'a>{definition:Definition<'a>,encoding:Encoding,implicit:bool,requirements:DocumentProfileRequirements}
fn plan<'a>(doc:&'a Document,limits:Limits,ctl:&Interrupt<'_>)->Result<Plan<'a>> {
 poll(ctl)?;admit(DocumentProfileRequirements{retained_bytes:0,result_retained_bytes_upper_bound:0,peak_resident_bytes:0,work_units:0},limits)?;
 let raw=doc.icc_profile.as_ref().map_or(0,|v|v.capacity());if raw>limits.resident_bytes{return Err(fail("current ICC exceeds profile resident budget"));}
 identity(doc,ctl)?;let space=mode_space(doc.mode);let enc=encoding(doc);let implicit=!matches!(doc.working_profile_interpretation,WorkingProfileInterpretation::Icc);
 let def=if implicit {if doc.icc_profile.is_none(){return Err(fail("implicit working profile is missing current ICC bytes"));}match space{ColorSpace::Rgb=>Definition::Builtin(Builtin::Srgb.profile()),ColorSpace::Gray=>Definition::Builtin(Builtin::SGray.profile()),_=>return Err(fail("XCF implicit color model unsupported"))}}
 else if let Some(bytes)=&doc.icc_profile {match cached(bytes,ctl)?{Some(p)=>Definition::Cached(p),None=>Definition::Cold(bytes)}}
 else {Definition::Builtin(photocraft_cms::builtin::default_for(space).unwrap_or(Builtin::Srgb.profile()))};poll(ctl)?;
 let cancelled=||ctl.cancelled();let requirements=match &def {
  Definition::Cold(bytes)=>{
   let parser=Profile::parse_requirements_with_control(bytes,limits,&cancelled).map_err(map)?;
   // The parser census includes all decoded vector/table/text capacities. With
   // transfer overrides LUTs are refused before encoding; retained scalar curve
   // tags cost at most two encoded bytes per original F32/F64 table word. Six
   // LUT validations and all 257-point curve schedules fit the fixed work term.
   // Thus six decoded-model envelopes cover source, clone, payload/data/output
   // and replacement description; no speculative profile allocation is needed.
   let peak=add(add(raw,bytes.len())?,add(mul(parser.model_bytes,6)?,64<<10)?)?;
   let factor=if enc==Encoding::NonlinearProfile{8}else{32};
   let work=parser.work_units.checked_mul(3).and_then(|v|v.checked_add((parser.model_bytes as u64).checked_mul(factor)?)).and_then(|v|v.checked_add(16<<20)).ok_or_else(||fail("profile work overflow"))?;
   DocumentProfileRequirements{retained_bytes:raw,result_retained_bytes_upper_bound:add(bytes.len(),add(mul(parser.model_bytes,3)?,64<<10)?)?,peak_resident_bytes:peak,work_units:work}
  },
  Definition::Cached(p)=>known(p,raw,enc,implicit,limits,ctl)?,Definition::Builtin(p)=>known(p,raw,enc,implicit,limits,ctl)?,
 };admit(requirements,limits)?;poll(ctl)?;Ok(Plan{definition:def,encoding:enc,implicit,requirements})
}
fn known(p:&Profile,raw:usize,enc:Encoding,implicit:bool,limits:Limits,ctl:&Interrupt<'_>)->Result<DocumentProfileRequirements>{
 let cancelled=||ctl.cancelled();let req=xcf_color::effective_profile_requirements_with_control(p,enc,limits,&cancelled).map_err(map)?;let clone=p.clone_requirements_with_control(limits,&cancelled).map_err(map)?;
 let mut returned=add(clone.retained_bytes,clone.cloned_model_bytes)?;
 if enc!=Encoding::NonlinearProfile{returned=add(returned,req.peak_resident_bytes.checked_sub(req.retained_bytes).ok_or_else(||fail("effective profile envelope"))?)?;}
 let extra=if implicit{raw.checked_mul(2).ok_or_else(||fail("implicit profile work"))? as u64}else{0};
 Ok(DocumentProfileRequirements{retained_bytes:add(raw,req.retained_bytes)?,result_retained_bytes_upper_bound:returned,peak_resident_bytes:add(raw,req.peak_resident_bytes)?,work_units:req.work_units.checked_mul(3).and_then(|v|v.checked_add(extra)).and_then(|v|v.checked_add(100_000)).ok_or_else(||fail("profile work overflow"))?})
}
pub(super) fn requirements(doc:&Document,limits:Limits,ctl:&Interrupt<'_>)->Result<DocumentProfileRequirements>{Ok(plan(doc,limits,ctl)?.requirements)}
pub(super) fn prepare(doc:&Document,limits:Limits,ctl:&Interrupt<'_>)->Result<Arc<Profile>> {
 let plan=plan(doc,limits,ctl)?;let cancelled=||ctl.cancelled();
 if plan.implicit {let bound=doc.icc_profile.as_ref().ok_or_else(||fail("implicit working profile is missing current ICC bytes"))?;let result=xcf_color::effective_implicit_profile_with_control(bound,mode_space(doc.mode),plan.encoding,remaining(limits,bound.capacity().checked_sub(bound.len()).ok_or_else(||fail("ICC capacity"))?,bound.len() as u64)?,&cancelled).map_err(map)?;poll(ctl)?;return Ok(Arc::new(result));}
 let mut used_work=0;let retained_raw=doc.icc_profile.as_ref().map_or(0,|v|v.capacity());
 let parsed;let profile=match &plan.definition {Definition::Cached(p)=>p.as_ref(),Definition::Builtin(p)=>p,Definition::Cold(bytes)=>{
  let req=Profile::parse_requirements_with_control(bytes,limits,&cancelled).map_err(map)?;used_work=req.work_units.checked_mul(3).ok_or_else(||fail("parse work overflow"))?;
  parsed=Profile::parse_with_control(bytes,limits,&cancelled).map_err(map)?;&parsed
 }};
 if profile.color_space!=mode_space(doc.mode){return Err(EngineError::Other("document profile does not match its color model".into()));}
 let phase=remaining(limits,retained_raw,used_work)?;
 let actual=xcf_color::effective_profile_requirements_with_control(profile,plan.encoding,phase,&cancelled).map_err(map)?;
 // The cold prospective envelope must cover the now exact reconstruction before
 // any clone/output allocation. Refuse rather than overrunning that reservation.
 if add(retained_raw,actual.peak_resident_bytes)?>plan.requirements.peak_resident_bytes||actual.work_units.checked_add(used_work).is_none_or(|v|v>plan.requirements.work_units){return Err(fail("decoded profile exceeds prospective reconstruction envelope"));}
 let result=xcf_color::effective_profile_from_definition_with_control(profile,plan.encoding,phase,&cancelled).map_err(map)?;poll(ctl)?;Ok(Arc::new(result))
}

#[cfg(test)]
mod tests {
 use super::*;
 use photocraft_color::{ColorMode,SampleType};
 use photocraft_geom::Size;
 use std::cell::Cell;
 fn doc(mode:ColorMode)->Document {Document::new("owned profile",Size::new(2,2),mode,SampleType::F64)}
 #[test]
 fn strict_assigned_cold_and_cached_definition_equal_legacy() {
  for mode in [ColorMode::Rgb,ColorMode::Grayscale] {for sample in [SampleEncoding::Linear,SampleEncoding::Perceptual,SampleEncoding::NonlinearProfile] {
   let mut d=doc(mode);d.sample_encoding=Some(sample);let base=if mode==ColorMode::Rgb{Builtin::DisplayP3}else{Builtin::SGray};let bytes=Arc::new(base.profile().to_bytes().as_ref().clone());d.icc_profile=Some(Arc::clone(&bytes));let before=d.clone();
   assert!(super::super::cached_profile_from_bytes(&bytes).is_none());let cold=prepare(&d,Limits::default(),&Interrupt::NONE).unwrap();assert!(super::super::cached_profile_from_bytes(&bytes).is_none());
   let legacy=super::super::try_document_profile(&d).unwrap();let cached=prepare(&d,Limits::default(),&Interrupt::NONE).unwrap();assert_eq!(cold.to_bytes(),legacy.to_bytes());assert_eq!(cached.to_bytes(),legacy.to_bytes());assert_eq!(cold.matrix,legacy.matrix);assert_eq!(d,before);
  }}
 }
 #[test]
 fn implicit_binding_and_encoding_preserved_without_inference() {
  let mut d=doc(ColorMode::Rgb);d.sample_encoding=Some(SampleEncoding::Linear);let p=xcf_color::effective_profile(None,ColorSpace::Rgb,Encoding::Linear).unwrap();d.icc_profile=Some(p.to_bytes());d.working_profile_interpretation=WorkingProfileInterpretation::implicit_srgb(d.icc_profile.as_ref().unwrap());
  let actual=prepare(&d,Limits::default(),&Interrupt::NONE).unwrap();assert_eq!(actual.to_bytes(),super::super::try_document_profile(&d).unwrap().to_bytes());let marker=d.working_profile_interpretation.clone();d.icc_profile=Some(Builtin::DisplayP3.profile().to_bytes());assert!(prepare(&d,Limits::default(),&Interrupt::NONE).is_err());assert_eq!(d.working_profile_interpretation,marker);
 }
 #[test]
 fn strict_malformed_mismatch_cancel_and_limits_leave_source_unchanged() {
  let mut d=doc(ColorMode::Rgb);d.icc_profile=Some(Arc::new(vec![0;140]));let before=d.clone();assert!(prepare(&d,Limits::default(),&Interrupt::NONE).is_err());assert_eq!(d,before);
  d.icc_profile=Some(Builtin::SGray.profile().to_bytes());assert!(prepare(&d,Limits::default(),&Interrupt::NONE).is_err());
  d.icc_profile=Some(Arc::new(Builtin::DisplayP3.profile().to_bytes().as_ref().clone()));let before=d.clone();let calls=Cell::new(0);let check=||{calls.set(calls.get()+1);calls.get()>25};assert!(matches!(prepare(&d,Limits::default(),&Interrupt::cancel_only(&check)),Err(EngineError::Cancelled)));assert_eq!(d,before);
  assert!(requirements(&d,Limits{resident_bytes:1,work_units:1},&Interrupt::NONE).is_err());assert_eq!(d,before);
 }
}
