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
pub struct ConversionRequirements {
 pub retained_bytes:u64,
 pub scratch_bytes:u64,
 /// Conservatively retained construction/preparation work, debited once per ROI.
 pub preparation_work_units:u64,
 /// Steady work for the paired RGB-to-LCh and LCh-to-RGB conversion of one pixel.
 pub work_units:u64,
}
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
/// Explicit Linear RGB Float stage, without source/profile authority. Public
/// observations establish exact scale-one identity at this boundary. This is
/// not a whole-Vibrance shortcut: physical LCh conversion still follows it.
/// Other scales retain the existing Native-choice Saturation arithmetic.
pub fn saturation_linear_float(input: [f32; 4], saturation: f64) -> Result<[f32; 4], E> {
    if !saturation.is_finite() || !(0.0..=10.0).contains(&saturation) {
        return Err(E::UnsupportedConversion);
    }
    if input.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err(E::InvalidSample);
    }
    if saturation == 1.0 {
        return Ok(input);
    }
    let mut output = [0.0f64; 4];
    PointOperation::Saturation { scale: saturation, colorspace: 0 }.try_eval_pixel(&input.map(f64::from), 3, &mut output)?;
    let [_, _, _, alpha] = input;
    converted(output.map(|value| value as f32), alpha.to_bits())
}
/// An additive stage boundary for already derived Linear RGB Float. Finite HDR
/// RGB is retained, alpha stays unit and exact, and scale one is still identity.
/// This arithmetic helper grants neither original HDR intake nor graph formats.
pub fn saturation_linear_float_derived(input: [f32; 4], saturation: f64) -> Result<[f32; 4], E> {
    if !saturation.is_finite() || !(0.0..=10.0).contains(&saturation) {
        return Err(E::UnsupportedConversion);
    }
    let [r, g, b, alpha] = input;
    if input.iter().any(|value| !value.is_finite()) || !(0.0..=1.0).contains(&alpha) {
        return Err(E::InvalidSample);
    }
    if saturation == 1.0 {
        return Ok(input);
    }
    let mut colors = [f64::from(r), f64::from(g), f64::from(b)];
    super::point::saturation_native_float(&mut colors, saturation)?;
    let [r, g, b] = colors.map(|value| value as f32);
    converted([r, g, b, alpha], alpha.to_bits())
}
/// Fixed private scratch only. Conversion requirements must be admitted by the
/// enclosing caller before invoking this function; cancellation spans both stages.
/// Selected RGB input is finite unit; intermediate/output RGB may be finite HDR.
/// Alpha, including signed zero, is never arithmetically changed.
pub fn evaluate(input:[f32;4],p:Parameters,actual:Context,conversions:&impl FloatConversions,c:&Interrupt<'_>)->Result<[f32;4],E>{
 poll(c)?;validate(p,actual)?;if conversions.context()!=actual{return Err(E::UnsupportedConversion);}
 let [_,_,_,alpha]=input;
 let saturation=saturation_linear_float(input,p.saturation)?;
 poll(c)?;let lch=converted(conversions.rgb_to_lch(saturation,c)?,alpha.to_bits())?;
 let lch=hue_chroma::evaluate_lch_float(lch,hue_chroma::Parameters{hue:0.0,chroma:p.vibrance,lightness:0.0})?;
 poll(c)?;let output=converted(conversions.lch_to_rgb(lch,c)?,alpha.to_bits())?;poll(c)?;Ok(output)
}

#[cfg(test)]
#[path = "point_vibrance_identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
#[path = "point_derived_float_stage_tests.rs"]
mod derived_stage_tests;

#[cfg(test)]
mod tests {
 use super::*;
 struct StageWitness {context:Context}
 impl FloatConversions for StageWitness {
  fn context(&self)->Context{self.context}
  fn requirements(&self,_:&Interrupt<'_>)->Result<ConversionRequirements,E>{Ok(ConversionRequirements{retained_bytes:0,scratch_bytes:0,preparation_work_units:0,work_units:1})}
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
