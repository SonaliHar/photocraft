//! Explicit source-preserving Vibrance Float-stage candidate, not default import
//! admission or original filter compositing. Stored scalar words are consumed;
//! original profile/config/cache/opacity/mask/space records remain borrowed.
//! RGB Float -> LCh Float -> RGB Float conversion is caller-proven and explicit.
//! No display tolerance, inferred working matrix or Photoshop Vibrance alias.
use photocraft_algo::gimp::point_vibrance::{self,Context,FloatConversions,Parameters,ConversionRequirements};
use photocraft_doc::xcf_filters::{ExactNumber,FilterNode,FilterValue};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt,MAX_CHECKED_RASTER_BYTES,RasterInputError as E};
pub const SCHEMA_326:&str="43c8c37c33d3173b366fce593fc1495358d7958a1e1bfe15ba3319b2a2120ddd";
pub const EPOCH_326:&str="gimp-3.2.6-windows-43c8c37c33d3173b";
const MAX_WORK:u64=1<<30;
const BIND_WORK:u64=32;
const SAMPLE_VALIDATION_WORK:u64=16;
#[derive(Clone,Copy)]
pub struct SourceBinding<'a>{pub expected_epoch:&'a str,pub schema_sha256:&'a str,pub context:Context}
#[derive(Clone,Copy)]
/// Already-converted input to the retained linear RGB Float Saturation stage.
/// This context is not the original image's native encoding; its conversion and
/// profile identity must be independently established by the caller.
pub struct RgbFloatView<'a>{pub extent:Rect,pub pixels:&'a [[f32;4]],pub context:Context}
pub struct RgbFloatWindow{pub roi:Rect,pub pixels:Vec<[f32;4]>,pub context:Context}
#[derive(Clone,Copy,Debug)]
pub struct Limits{pub resident_bytes:u64,pub work_units:u64}
impl Default for Limits{fn default()->Self{Self{resident_bytes:MAX_CHECKED_RASTER_BYTES,work_units:MAX_WORK}}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Requirements{pub borrowed_source_bytes:u64,pub output_bytes:u64,pub retained_conversion_bytes:u64,pub scratch_bytes:u64,pub peak_bytes:u64,pub work_units:u64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum EvaluationScope{PointStagesOnly}
pub struct BoundSource<'a>{node:&'a FilterNode,binding:SourceBinding<'a>,parameters:Parameters}
fn poll(c:&Interrupt<'_>)->Result<(),E>{c.check().map_err(|_|E::Cancelled)}
fn add(a:u64,b:u64)->Result<u64,E>{a.checked_add(b).ok_or(E::LimitExceeded)}
fn mul(a:u64,b:u64)->Result<u64,E>{a.checked_mul(b).ok_or(E::LimitExceeded)}
fn count(rect:Rect)->Result<usize,E>{
 if rect.x1<rect.x0||rect.y1<rect.y0{return Err(E::InvalidBounds);}
 let width=usize::try_from(i64::from(rect.x1)-i64::from(rect.x0)).map_err(|_|E::LimitExceeded)?;let height=usize::try_from(i64::from(rect.y1)-i64::from(rect.y0)).map_err(|_|E::LimitExceeded)?;width.checked_mul(height).ok_or(E::LimitExceeded)
}
fn admitted(limits:Limits)->Result<(),E>{if limits.resident_bytes>MAX_CHECKED_RASTER_BYTES||limits.work_units>MAX_WORK{return Err(E::LimitExceeded);}Ok(())}
fn selected_pixel(input:RgbFloatView<'_>,x:i32,y:i32)->Result<[f32;4],E>{
 let width=usize::try_from(i64::from(input.extent.x1)-i64::from(input.extent.x0)).map_err(|_|E::InvalidBounds)?;
 let row=usize::try_from(i64::from(y)-i64::from(input.extent.y0)).map_err(|_|E::InvalidBounds)?;
 let column=usize::try_from(i64::from(x)-i64::from(input.extent.x0)).map_err(|_|E::InvalidBounds)?;
 let index=row.checked_mul(width).and_then(|n|n.checked_add(column)).ok_or(E::LimitExceeded)?;
 input.pixels.get(index).copied().ok_or(E::LengthMismatch)
}
/// Allocation-free, selected-window-only numeric admission. The complete work
/// envelope is admitted before this scan, and the output reserve follows it.
fn validate_selected(input:RgbFloatView<'_>,roi:Rect,c:&Interrupt<'_>)->Result<(),E>{
 for y in roi.y0..roi.y1{poll(c)?;for x in roi.x0..roi.x1{poll(c)?;let value=selected_pixel(input,x,y)?;
  if value.iter().any(|v|!v.is_finite()||!(0.0..=1.0).contains(v)){return Err(E::InvalidSample);}
 }}poll(c)
}
/// Only observed raw Float32 scalar records are admitted in this initial source
/// lane. Public gdouble getter type does not grant Float64 serialization aliases.
pub fn bind_source_node<'a>(node:&'a FilterNode,binding:SourceBinding<'a>,c:&Interrupt<'_>)->Result<BoundSource<'a>,E>{
 poll(c)?;
 if binding.schema_sha256!=SCHEMA_326||binding.expected_epoch!=EPOCH_326||node.operation.schema_epoch!=binding.expected_epoch||node.operation.name!="gegl:vibrance"||node.operation.stored_version!="0:0"||node.id.0==0||node.arguments.len()!=2{return Err(E::UnsupportedConversion);}
 let mut vibrance=None;let mut saturation=None;
 for argument in &node.arguments{
  poll(c)?;let slot=match argument.name.as_str(){"vibrance"=>&mut vibrance,"saturation"=>&mut saturation,_=>return Err(E::UnsupportedConversion)};
  if slot.is_some(){return Err(E::UnsupportedConversion);}let FilterValue::Number(ExactNumber::Float32Bits(bits))=&argument.value else{return Err(E::UnsupportedConversion)};*slot=Some(f64::from(f32::from_bits(*bits)));
 }
 let parameters=Parameters{vibrance:vibrance.ok_or(E::UnsupportedConversion)?,saturation:saturation.ok_or(E::UnsupportedConversion)?};point_vibrance::validate(parameters,binding.context)?;poll(c)?;Ok(BoundSource{node,binding,parameters})
}
impl<'a> BoundSource<'a>{
 pub fn original_node(&self)->&'a FilterNode{self.node}
 pub fn scope(&self)->EvaluationScope{EvaluationScope::PointStagesOnly}
 pub fn parameters(&self)->Parameters{self.parameters}
 fn plan(&self,input:RgbFloatView<'_>,roi:Rect,conversions:&impl FloatConversions,limits:Limits,c:&Interrupt<'_>)->Result<(Requirements,ConversionRequirements),E>{
  poll(c)?;admitted(limits)?;
  if input.context!=self.binding.context||conversions.context()!=input.context||count(input.extent)?!=input.pixels.len()||roi.is_empty()||roi.intersect(&input.extent)!=roi{return Err(E::UnsupportedConversion);}
  let input_bytes=mul(u64::try_from(input.pixels.len()).map_err(|_|E::LimitExceeded)?,16)?;let output_bytes=mul(u64::try_from(count(roi)?).map_err(|_|E::LimitExceeded)?,16)?;
  // Fixed source/output envelope is refused before asking a converter to inspect
  // its own prospective storage. Converter requirements must be borrowed/polled.
  if add(add(input_bytes,output_bytes)?,point_vibrance::POINT_SCRATCH)?>limits.resident_bytes{return Err(E::LimitExceeded);}
  let conversion=conversions.requirements(c)?;let scratch_bytes=add(conversion.scratch_bytes,point_vibrance::POINT_SCRATCH)?;let peak_bytes=add(add(add(input_bytes,output_bytes)?,conversion.retained_bytes)?,scratch_bytes)?;
  // Preparation remains charged once for this evaluation; repeating it per
  // pixel would conflate constructor work with the steady paired conversion.
  let work_units=add(add(mul(u64::try_from(count(roi)?).map_err(|_|E::LimitExceeded)?,add(add(point_vibrance::POINT_WORK,conversion.work_units)?,SAMPLE_VALIDATION_WORK)?)?,conversion.preparation_work_units)?,BIND_WORK)?;
  if peak_bytes>limits.resident_bytes||work_units>limits.work_units{return Err(E::LimitExceeded);}poll(c)?;
  Ok((Requirements{borrowed_source_bytes:input_bytes,output_bytes,retained_conversion_bytes:conversion.retained_bytes,scratch_bytes,peak_bytes,work_units},conversion))
 }
 pub fn requirements(&self,input:RgbFloatView<'_>,roi:Rect,conversions:&impl FloatConversions,limits:Limits,c:&Interrupt<'_>)->Result<Requirements,E>{self.plan(input,roi,conversions,limits,c).map(|(plan,_)|plan)}
 pub fn evaluate(&self,input:RgbFloatView<'_>,roi:Rect,conversions:&impl FloatConversions,limits:Limits,c:&Interrupt<'_>)->Result<RgbFloatWindow,E>{
  let (required,_)=self.plan(input,roi,conversions,limits,c)?;
  validate_selected(input,roi,c)?;
  let pixels=count(roi)?;let mut output=Vec::new();output.try_reserve_exact(pixels).map_err(|_|E::AllocationFailed)?;
  let actual_bytes=mul(u64::try_from(output.capacity()).map_err(|_|E::LimitExceeded)?,16)?;if add(required.peak_bytes.checked_sub(required.output_bytes).ok_or(E::LimitExceeded)?,actual_bytes)?>limits.resident_bytes{return Err(E::LimitExceeded);}
  for y in roi.y0..roi.y1{poll(c)?;
   for x in roi.x0..roi.x1{poll(c)?;let value=selected_pixel(input,x,y)?;output.push(point_vibrance::evaluate(value,self.parameters,input.context,conversions,c)?);}
  }
  poll(c)?;Ok(RgbFloatWindow{roi,pixels:output,context:input.context})
 }
}

#[cfg(test)]
mod tests {
 use super::*;
 use photocraft_algo::gimp::{configured::{Transfer,WorkingContext},hue_chroma::LchFloatContext};
 use photocraft_doc::{item_identity::FilterId,xcf_filters::{FilterArgument,OperationIdentity}};
 use std::sync::atomic::{AtomicUsize,Ordering};
 fn context()->Context{Context{rgb:WorkingContext{transfer:Transfer::Linear,profile_identity:[1;32],color_channels:3,alpha:true},lch:LchFloatContext{profile_identity:[1;32],space_identity:[2;32]}}}
 fn node()->FilterNode{FilterNode{id:FilterId(1),operation:OperationIdentity{name:"gegl:vibrance".into(),stored_version:"0:0".into(),schema_epoch:EPOCH_326.into()},source_display_name:Some("Retained own node".into()),source_icon:None,enabled:false,opacity_f64_bits:0.5f64.to_bits(),source_clipping:Some(0),source_region:Some(0),mask:None,auxiliary:vec![],composition:None,arguments:vec![FilterArgument{name:"vibrance".into(),value:FilterValue::Number(ExactNumber::Float32Bits(5.0f32.to_bits()))},FilterArgument{name:"saturation".into(),value:FilterValue::Number(ExactNumber::Float32Bits(0.0f32.to_bits()))}]}}
 fn binding()->SourceBinding<'static>{SourceBinding{expected_epoch:EPOCH_326,schema_sha256:SCHEMA_326,context:context()}}
 struct Stages{calls:AtomicUsize}
 impl FloatConversions for Stages{
  fn context(&self)->Context{context()}
  fn requirements(&self,_:&Interrupt<'_>)->Result<ConversionRequirements,E>{Ok(ConversionRequirements{retained_bytes:0,scratch_bytes:0,preparation_work_units:0,work_units:1})}
  fn rgb_to_lch(&self,p:[f32;4],_:&Interrupt<'_>)->Result<[f32;4],E>{self.calls.fetch_add(1,Ordering::Relaxed);assert_eq!(p[0].to_bits(),p[1].to_bits());assert_eq!(p[1].to_bits(),p[2].to_bits());Ok([50.,40.,60.,p[3]])}
  fn lch_to_rgb(&self,p:[f32;4],_:&Interrupt<'_>)->Result<[f32;4],E>{assert_eq!(p[..3],[50.,45.,60.]);Ok([-0.25,1.25,0.5,p[3]])}
 }
 struct PreparedStages<'a>{inner:&'a Stages,preparation:u64}
 impl FloatConversions for PreparedStages<'_>{
  fn context(&self)->Context{self.inner.context()}
  fn requirements(&self,c:&Interrupt<'_>)->Result<ConversionRequirements,E>{let mut required=self.inner.requirements(c)?;required.preparation_work_units=self.preparation;Ok(required)}
  fn rgb_to_lch(&self,p:[f32;4],c:&Interrupt<'_>)->Result<[f32;4],E>{self.inner.rgb_to_lch(p,c)}
  fn lch_to_rgb(&self,p:[f32;4],c:&Interrupt<'_>)->Result<[f32;4],E>{self.inner.lch_to_rgb(p,c)}
 }
 #[test]
 fn exact_stored_scalar_binding_preserves_disabled_outer_metadata_and_raw_input(){
  let node=node();let before=node.clone();let bound=bind_source_node(&node,binding(),&Interrupt::NONE).unwrap();let data=[[0.25,0.5,0.75,-0.0],[0.5,0.25,0.75,1.0]];let bits=data.map(|p|p.map(f32::to_bits));let view=RgbFloatView{extent:Rect::new(-1,2,1,3),pixels:&data,context:context()};let converters=Stages{calls:AtomicUsize::new(0)};
  let window=bound.evaluate(view,view.extent,&converters,Limits::default(),&Interrupt::NONE).unwrap();assert_eq!(window.pixels[0],[ -0.25,1.25,0.5,-0.0]);assert_eq!(window.pixels[0][3].to_bits(),(-0.0f32).to_bits());assert_eq!(data.map(|p|p.map(f32::to_bits)),bits);assert_eq!(bound.original_node(),&before);assert_eq!(bound.scope(),EvaluationScope::PointStagesOnly);
 }
 #[test]
 fn float64_alias_duplicate_arguments_and_tiny_admission_are_refused(){
  let mut malformed=node();malformed.arguments[0].value=FilterValue::Number(ExactNumber::Float64Bits(5f64.to_bits()));assert!(bind_source_node(&malformed,binding(),&Interrupt::NONE).is_err());
  malformed=node();malformed.arguments[1].name="vibrance".into();assert!(bind_source_node(&malformed,binding(),&Interrupt::NONE).is_err());
  let node=node();let bound=bind_source_node(&node,binding(),&Interrupt::NONE).unwrap();let data=[[0.25,0.5,0.75,1.0]];let view=RgbFloatView{extent:Rect::new(0,0,1,1),pixels:&data,context:context()};let converters=Stages{calls:AtomicUsize::new(0)};
  assert!(matches!(bound.evaluate(view,view.extent,&converters,Limits{resident_bytes:1,work_units:1},&Interrupt::NONE),Err(E::LimitExceeded)));assert_eq!(converters.calls.load(Ordering::Relaxed),0);
 }
 #[test]
 fn late_cancel_discards_private_window_and_original_node_stays_exact(){
  let node=node();let before=node.clone();let bound=bind_source_node(&node,binding(),&Interrupt::NONE).unwrap();let data=[[0.25,0.5,0.75,1.0];2];let view=RgbFloatView{extent:Rect::new(0,0,2,1),pixels:&data,context:context()};let converters=Stages{calls:AtomicUsize::new(0)};let stop=||converters.calls.load(Ordering::Relaxed)>0;
  assert!(matches!(bound.evaluate(view,view.extent,&converters,Limits::default(),&Interrupt::cancel_only(&stop)),Err(E::Cancelled)));assert_eq!(node,before);assert_eq!(data,[[0.25,0.5,0.75,1.0];2]);
 }
 #[test]
 fn malformed_last_selected_pixel_is_admitted_before_any_output_reserve_or_conversion(){
  let node=node();let bound=bind_source_node(&node,binding(),&Interrupt::NONE).unwrap();
  let data=[[0.25,0.5,0.75,1.0],[0.25,f32::NAN,0.75,1.0]];let before=data.map(|p|p.map(f32::to_bits));
  let view=RgbFloatView{extent:Rect::new(-1,2,1,3),pixels:&data,context:context()};let converters=Stages{calls:AtomicUsize::new(0)};
  assert_eq!(validate_selected(view,view.extent,&Interrupt::NONE),Err(E::InvalidSample));
  assert!(matches!(bound.evaluate(view,view.extent,&converters,Limits::default(),&Interrupt::NONE),Err(E::InvalidSample)));
  assert_eq!(converters.calls.load(Ordering::Relaxed),0);assert_eq!(data.map(|p|p.map(f32::to_bits)),before);
  // Invalid data outside the selected ROI is retained and never interpreted.
  assert!(bound.evaluate(view,Rect::new(-1,2,0,3),&converters,Limits::default(),&Interrupt::NONE).is_ok());
 }
 #[test]
 fn selected_validation_obeys_cancel_and_its_additional_lower_work_charge(){
  let node=node();let bound=bind_source_node(&node,binding(),&Interrupt::NONE).unwrap();let data=[[0.25,0.5,0.75,1.0];2];
  let view=RgbFloatView{extent:Rect::new(0,0,2,1),pixels:&data,context:context()};let converters=Stages{calls:AtomicUsize::new(0)};
  let required=bound.requirements(view,view.extent,&converters,Limits::default(),&Interrupt::NONE).unwrap();
  assert_eq!(required.work_units,2*(point_vibrance::POINT_WORK+1+SAMPLE_VALIDATION_WORK)+BIND_WORK);
  assert!(matches!(bound.evaluate(view,view.extent,&converters,Limits{resident_bytes:required.peak_bytes,work_units:required.work_units-1},&Interrupt::NONE),Err(E::LimitExceeded)));
  let polls=AtomicUsize::new(0);let stop=||polls.fetch_add(1,Ordering::Relaxed)>=2;
  assert_eq!(validate_selected(view,view.extent,&Interrupt::cancel_only(&stop)),Err(E::Cancelled));
  assert_eq!(converters.calls.load(Ordering::Relaxed),0);
 }
 #[test]
 fn converter_preparation_is_debited_once_not_per_pixel_and_cannot_overflow(){
  let node=node();let bound=bind_source_node(&node,binding(),&Interrupt::NONE).unwrap();let data=[[0.25,0.5,0.75,1.0];2];
  let view=RgbFloatView{extent:Rect::new(0,0,2,1),pixels:&data,context:context()};let stages=Stages{calls:AtomicUsize::new(0)};let prepared=PreparedStages{inner:&stages,preparation:7};
  let one=bound.requirements(view,Rect::new(0,0,1,1),&prepared,Limits::default(),&Interrupt::NONE).unwrap();let two=bound.requirements(view,view.extent,&prepared,Limits::default(),&Interrupt::NONE).unwrap();
  assert_eq!(one.work_units,184);assert_eq!(two.work_units,329);assert_eq!(two.work_units-one.work_units,145);
  assert!(matches!(bound.evaluate(view,view.extent,&prepared,Limits{resident_bytes:two.peak_bytes,work_units:328},&Interrupt::NONE),Err(E::LimitExceeded)));assert_eq!(stages.calls.load(Ordering::Relaxed),0);
  let overflow=PreparedStages{inner:&stages,preparation:u64::MAX};assert!(matches!(bound.requirements(view,view.extent,&overflow,Limits::default(),&Interrupt::NONE),Err(E::LimitExceeded)));
 }
}
