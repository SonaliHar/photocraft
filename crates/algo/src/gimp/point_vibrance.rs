//! Clean-room composition of public GEGL 0.4.72 Vibrance metadata:
//! Saturation(scale) precedes Hue-Chroma(chroma); no third-party code is copied.
//! https://www.gimp.org/release-notes/gimp-3.2.html
//! Explicit linear RGB Float and LCh Float conversion remains caller-owned.
//! This is a point candidate, not source/profile/whole-filter qualification.
use super::{configured::{Transfer,WorkingContext},hue_chroma::{self,LchFloatContext},point::PointOperation};
use photocraft_raster::{Interrupt,RasterInputError as E};

#[derive(Clone,Copy,Debug,PartialEq)]
pub struct Parameters {pub vibrance:f64,pub saturation:f64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
/// `rgb` describes input to the existing independently retained Native-choice
/// Saturation evaluator (linear RGB Float). It does not describe imported owner
/// samples. `lch` separately identifies the caller-proven Hue-Chroma conversion.
pub struct Context {pub rgb:WorkingContext,pub lch:LchFloatContext}
/// Prospective known converter storage/work, supplied by its actual implementation.
/// The cooperative callback boundary cannot constrain hidden provider allocations.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct ConversionRequirements {pub retained_bytes:u64,pub scratch_bytes:u64,pub work_units:u64}
/// The caller must implement a proven, explicit same-profile conversion recipe.
/// No format string, default ICC or working matrix is inferred by this interface.
pub trait FloatConversions {
 fn context(&self)->Context;
 fn requirements(&self,control:&Interrupt<'_>)->Result<ConversionRequirements,E>;
 fn rgb_to_lch(&self,value:[f32;4],control:&Interrupt<'_>)->Result<[f32;4],E>;
 fn lch_to_rgb(&self,value:[f32;4],control:&Interrupt<'_>)->Result<[f32;4],E>;
}
pub const POINT_WORK:u64=128;
pub const POINT_SCRATCH:u64=128;
pub fn validate(p:Parameters,context:Context)->Result<(),E>{
 if !p.vibrance.is_finite()||!(-100.0..=100.0).contains(&p.vibrance)||!p.saturation.is_finite()||!(0.0..=10.0).contains(&p.saturation)||context.rgb.transfer!=Transfer::Linear||context.rgb.color_channels!=3||!context.rgb.alpha||context.rgb.profile_identity==[0;32]||context.rgb.profile_identity!=context.lch.profile_identity||context.lch.space_identity==[0;32]{return Err(E::UnsupportedConversion);}
 Ok(())
}
fn poll(c:&Interrupt<'_>)->Result<(),E>{c.check().map_err(|_|E::Cancelled)}
fn converted(value:[f32;4],alpha:u32)->Result<[f32;4],E>{
 if value.iter().any(|v|!v.is_finite())||value.get(3).is_none_or(|v|v.to_bits()!=alpha){return Err(E::InvalidSample);}Ok(value)
}
/// Fixed private scratch only. Conversion requirements must be admitted by the
/// enclosing caller before invoking this function; cancellation spans both stages.
/// Selected RGB input is finite unit; intermediate/output RGB may be finite HDR.
/// Alpha, including signed zero, is never arithmetically changed.
pub fn evaluate(input:[f32;4],p:Parameters,actual:Context,conversions:&impl FloatConversions,c:&Interrupt<'_>)->Result<[f32;4],E>{
 poll(c)?;validate(p,actual)?;if conversions.context()!=actual{return Err(E::UnsupportedConversion);}
 if input.iter().any(|v|!v.is_finite()||!(0.0..=1.0).contains(v)){return Err(E::InvalidSample);}
 let [_,_,_,alpha]=input;
 let mut saturation=[0.0f64;4];
 PointOperation::Saturation{scale:p.saturation,colorspace:0}.try_eval_pixel(&input.map(f64::from),3,&mut saturation)?;
 let saturation=converted(saturation.map(|v|v as f32),alpha.to_bits())?;
 poll(c)?;let lch=converted(conversions.rgb_to_lch(saturation,c)?,alpha.to_bits())?;
 let lch=hue_chroma::evaluate_lch_float(lch,hue_chroma::Parameters{hue:0.0,chroma:p.vibrance,lightness:0.0})?;
 poll(c)?;let output=converted(conversions.lch_to_rgb(lch,c)?,alpha.to_bits())?;poll(c)?;Ok(output)
}

#[cfg(test)]
mod tests {
 use super::*;
 struct StageWitness {context:Context}
 impl FloatConversions for StageWitness {
  fn context(&self)->Context{self.context}
  fn requirements(&self,_:&Interrupt<'_>)->Result<ConversionRequirements,E>{Ok(ConversionRequirements{retained_bytes:0,scratch_bytes:0,work_units:1})}
  fn rgb_to_lch(&self,p:[f32;4],_:&Interrupt<'_>)->Result<[f32;4],E>{assert_eq!(p[0].to_bits(),p[1].to_bits());assert_eq!(p[1].to_bits(),p[2].to_bits());assert_ne!(p[0],0.25);Ok([50.0,20.0,10.0,p[3]])}
  fn lch_to_rgb(&self,p:[f32;4],_:&Interrupt<'_>)->Result<[f32;4],E>{assert_eq!(p[..3],[50.0,25.0,10.0]);Ok([0.1,0.2,0.3,p[3]])}
 }
 fn context()->Context{Context{rgb:WorkingContext{transfer:Transfer::Linear,profile_identity:[1;32],color_channels:3,alpha:true},lch:LchFloatContext{profile_identity:[1;32],space_identity:[2;32]}}}
 #[test]
 fn established_stages_execute_in_order_and_preserve_negative_zero_alpha(){
  let context=context();let output=evaluate([0.25,0.5,0.75,-0.0],Parameters{vibrance:5.0,saturation:0.0},context,&StageWitness{context},&Interrupt::NONE).unwrap();assert_eq!(output[3].to_bits(),(-0.0f32).to_bits());
 }
 #[test]
 fn unsupported_context_nonfinite_input_and_cancel_never_call_conversions(){
  let context=context();let witness=StageWitness{context};let p=Parameters{vibrance:0.0,saturation:1.0};let stop=||true;
  assert_eq!(evaluate([0.0;4],p,context,&witness,&Interrupt::cancel_only(&stop)),Err(E::Cancelled));
  assert_eq!(evaluate([f32::NAN,0.,0.,1.],p,context,&witness,&Interrupt::NONE),Err(E::InvalidSample));
  let mut wrong=context;wrong.rgb.transfer=Transfer::NonlinearProfile;assert!(evaluate([0.;4],p,wrong,&witness,&Interrupt::NONE).is_err());
 }
}
