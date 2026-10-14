//! Borrowed prospective effective-transfer pair. No Profile or ICC buffer is
//! constructed; existing side-cost/writer/constructor envelopes are reused.
use super::*;
use crate::{profile::EncodeRequirements,xcf_color::{self,Encoding,EffectiveProfileRequirements}};
#[derive(Clone,Copy,Debug)]
pub struct EffectivePairRequirements{
 pub original_retained_bytes:usize,
 pub source_profile:EffectiveProfileRequirements,
 pub working_profile:EffectiveProfileRequirements,
 pub source_encoding:EncodeRequirements,
 pub working_encoding:EncodeRequirements,
 pub forward:TransformRequirements,
 pub inverse:TransformRequirements,
}
fn encoded(basis:&Profile,recipe:Option<(&str,&Curve)>,model_upper:usize,limits:Limits,cancelled:&dyn Fn()->bool)->Result<EncodeRequirements>{
 let old=match recipe{Some((suffix,curve))=>crate::write::controlled::transfer_requirements(basis,suffix,curve,limits,cancelled)?,None=>basis.encode_requirements_with_control(limits,cancelled)?};
 // The effective-profile peak bounds its eventual public model plus retained
 // encoded cache. Substitute that prospective owner into the existing writer
 // envelope; writer recipe/payload sizes remain those of the exact override.
 let peak=add(model_upper,old.peak_resident_bytes.checked_sub(old.retained_bytes).ok_or(Error::Limit("effective encoded ownership"))?)?;
 let check=Budget::new(limits,cancelled)?;check.admit(peak,old.work_units)?;
 Ok(EncodeRequirements{retained_bytes:model_upper,peak_resident_bytes:peak,..old})
}
fn effective_equal(basis:&Profile,source:Option<&Curve>,working:Option<&Curve>,budget:&mut Budget<'_>)->Result<bool>{
 // Effective construction only replaces the active TRC; every other field
 // compared by model_equal is cloned unchanged. Description/encoded cache do
 // not participate in the legacy identity predicate.
 budget.poll()?;
 match basis.color_space{
  ColorSpace::Rgb=>{
   let original=basis.trc.as_ref().ok_or_else(||invalid("RGB effective identity without curves"))?.each_ref();
   let a=source.map_or(original,|curve|[curve;3]);let z=working.map_or(original,|curve|[curve;3]);
   for (x,y) in a.into_iter().zip(z){if !x.equals_controlled(y,budget)?{return Ok(false);}}
  },
  ColorSpace::Gray=>{
   let original=basis.gray_trc.as_ref().ok_or_else(||invalid("Gray effective identity without curve"))?;
   if !source.unwrap_or(original).equals_controlled(working.unwrap_or(original),budget)?{return Ok(false);}
  },_=>return Err(invalid("effective identity color space")),
 }budget.poll()?;Ok(true)
}
fn envelope(basis:&Profile,source:Option<&Curve>,working:Option<&Curve>,retained:usize,identity_only:bool,limits:Limits,cancelled:&dyn Fn()->bool)->Result<TransformRequirements>{
 let budget=Budget::new(limits,cancelled)?;
 if identity_only{let required=identity_envelope(retained)?;budget.admit(required.peak_resident_bytes,required.work_units)?;return Ok(required);}
 let(sb,sw,_)=basis.selected_side_cost_override(true,Intent::RelativeColorimetric,source,&budget)?;
 let(db,dw,_)=basis.selected_side_cost_override(false,Intent::RelativeColorimetric,working,&budget)?;
 // Effective definitions may compare equal even for different transfer markers
 // (for example an existing Identity curve). Admit both actual constructor
 // branches; a name/marker never substitutes for model equality.
 let ordinary=construction_envelope(retained,add(sb,db)?,work_add(sw,dw)?,0,0,false)?;
 let identity=identity_envelope(retained)?;
 let required=TransformRequirements{retained_bytes:retained,peak_resident_bytes:ordinary.peak_resident_bytes.max(identity.peak_resident_bytes),work_units:ordinary.work_units.max(identity.work_units)};
 budget.admit(required.peak_resident_bytes,required.work_units)?;Ok(required)
}
/// Exact Relative/no-BPC analytical RGB/Gray transfer recipes only. Existing
/// LUT/profile options retain their ordinary constructor requirements API.
pub fn effective_pair_requirements(basis:&Profile,source:Encoding,working:Encoding,limits:Limits,cancelled:&dyn Fn()->bool)->Result<EffectivePairRequirements>{
 let mut budget=Budget::new(limits,cancelled)?;
 if !matches!(basis.color_space,ColorSpace::Rgb|ColorSpace::Gray)||basis.a2b.iter().chain(&basis.b2a).any(Option::is_some){return Err(invalid("effective analytical transfer pair domain"));}
 let original_retained_bytes=basis.controlled_retained_bytes(&budget)?;
 let source_profile=xcf_color::effective_profile_requirements_with_control(basis,source,limits,cancelled)?;
 let working_profile=xcf_color::effective_profile_requirements_with_control(basis,working,limits,cancelled)?;
 let source_recipe=xcf_color::transfer_recipe(source);let working_recipe=xcf_color::transfer_recipe(working);
 let source_curve=source_recipe.as_ref().map(|(_,curve)|curve);let working_curve=working_recipe.as_ref().map(|(_,curve)|curve);
 let exact_identity=effective_equal(basis,source_curve,working_curve,&mut budget)?;
 // Preserve the actual constructor's early identity branch, including public
 // RGB/Gray models whose class is otherwise unsupported. Never infer equality
 // merely from the shared original definition or from transfer marker names.
 if !exact_identity{profile_classes(basis,basis,&budget)?;}
 let source_encoding=encoded(basis,source_recipe.as_ref().map(|(suffix,curve)|(*suffix,curve)),source_profile.peak_resident_bytes,limits,cancelled)?;
 let working_encoding=encoded(basis,working_recipe.as_ref().map(|(suffix,curve)|(*suffix,curve)),working_profile.peak_resident_bytes,limits,cancelled)?;
 let held=add(source_profile.peak_resident_bytes,working_profile.peak_resident_bytes)?;
 let forward=envelope(basis,source_curve,working_curve,held,exact_identity,limits,cancelled)?;
 let inverse=envelope(basis,working_curve,source_curve,held,exact_identity,limits,cancelled)?;
 budget.poll()?;Ok(EffectivePairRequirements{original_retained_bytes,source_profile,working_profile,source_encoding,working_encoding,forward,inverse})
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]
 fn borrowed_bound_covers_real_identity_and_transfer_override_pairs_and_exact_encoder_recipe(){
  for builtin in [crate::Builtin::Srgb,crate::Builtin::SGray]{let basis=builtin.profile();let before=basis.to_bytes();
   for source in [Encoding::NonlinearProfile,Encoding::Linear,Encoding::Perceptual]{for working in [Encoding::NonlinearProfile,Encoding::Linear,Encoding::Perceptual]{
    let prospective=effective_pair_requirements(basis,source,working,Limits::default(),&||false).unwrap();
    let a=xcf_color::effective_profile_from_definition_with_control(basis,source,Limits::default(),&||false).unwrap();let z=xcf_color::effective_profile_from_definition_with_control(basis,working,Limits::default(),&||false).unwrap();
    let options=TransformOptions{intent:Intent::RelativeColorimetric,bpc:false,..Default::default()};
    let f=requirements(&a,&z,options,Limits::default(),&||false).unwrap();let r=requirements(&z,&a,options,Limits::default(),&||false).unwrap();
    assert!(f.peak_resident_bytes<=prospective.forward.peak_resident_bytes&&f.work_units<=prospective.forward.work_units);assert!(r.peak_resident_bytes<=prospective.inverse.peak_resident_bytes&&r.work_units<=prospective.inverse.work_units);
    let ae=a.encode_requirements_with_control(Limits::default(),&||false).unwrap();let ze=z.encode_requirements_with_control(Limits::default(),&||false).unwrap();
    assert!(ae.peak_resident_bytes<=prospective.source_encoding.peak_resident_bytes);assert!(ze.peak_resident_bytes<=prospective.working_encoding.peak_resident_bytes);assert_eq!(ae.encoded_bytes_upper_bound,prospective.source_encoding.encoded_bytes_upper_bound);assert_eq!(ze.encoded_bytes_upper_bound,prospective.working_encoding.encoded_bytes_upper_bound);
   }}assert_eq!(basis.to_bytes().as_slice(),before.as_slice());
  }
 }
 #[test]
 fn lowered_quota_cancel_and_unsupported_domain_leave_original_model_unchanged(){
  let basis=crate::Builtin::Srgb.profile();let before=basis.to_bytes();
  assert!(matches!(effective_pair_requirements(basis,Encoding::Linear,Encoding::Perceptual,Limits{resident_bytes:1,work_units:1},&||false),Err(Error::Limit(_))));
  assert!(matches!(effective_pair_requirements(basis,Encoding::Linear,Encoding::Perceptual,Limits::default(),&||true),Err(Error::Cancelled)));
  assert!(effective_pair_requirements(crate::Builtin::CoatedCmyk.profile(),Encoding::Linear,Encoding::Perceptual,Limits::default(),&||false).is_err());assert_eq!(basis.to_bytes().as_slice(),before.as_slice());
 }
 #[test]
 fn legacy_early_identity_class_acceptance_requires_exact_effective_curve_equality(){
  for class in [ProfileClass::DeviceLink,ProfileClass::NamedColor,ProfileClass::Abstract]{
   let mut basis=crate::Builtin::Srgb.profile().clone();basis.class=class;let before=basis.clone();
   let same=effective_pair_requirements(&basis,Encoding::NonlinearProfile,Encoding::NonlinearProfile,Limits::default(),&||false).unwrap();
   let old=requirements(&basis,&basis,TransformOptions{intent:Intent::RelativeColorimetric,bpc:false,..Default::default()},Limits::default(),&||false).unwrap();assert!(old.peak_resident_bytes<=same.forward.peak_resident_bytes&&old.work_units<=same.forward.work_units);
   assert!(effective_pair_requirements(&basis,Encoding::NonlinearProfile,Encoding::Linear,Limits::default(),&||false).is_err());
   // Different markers alone are not an identity proof; actual Identity curves
   // make this pair equal under the existing constructor predicate.
   basis.trc=Some([Curve::Identity,Curve::Identity,Curve::Identity]);assert!(effective_pair_requirements(&basis,Encoding::NonlinearProfile,Encoding::Linear,Limits::default(),&||false).is_ok());
   assert_eq!(before.class,class);
  }
 }
}
