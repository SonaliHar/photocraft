//! Owned CMS preparation for an explicitly bound retained configuration.
//! Resolved effective profiles/transfer are caller inputs, never inferred ICC
//! defaults. No default dispatcher, composition envelope or source admission.
use super::*;
use crate::gimp_configured_candidate::{BoundSource,bind_source_node_with_sample_policy};
use photocraft_cms::{Profile,Intent,control};

const CONFIG_BYTES:u64=65536;
const CONFIG_WORK:u64=16384;
const CENSUS_WORK:u64=2*32*(3*4096+256);
fn cms_error(error:control::Error)->RasterInputError {
 match error {control::Error::Cancelled=>RasterInputError::Cancelled,control::Error::Allocation=>RasterInputError::AllocationFailed,control::Error::Limit(_)=>RasterInputError::LimitExceeded,control::Error::Cms(_)=>RasterInputError::UnsupportedConversion}
}
fn lowered(limits:control::Limits,bytes:usize,work:u64)->Result<control::Limits,RasterInputError>{
 Ok(control::Limits{resident_bytes:limits.resident_bytes.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?,work_units:limits.work_units.checked_sub(work).ok_or(RasterInputError::LimitExceeded)?})
}
fn profile_domain(profile:&Profile,control:&Interrupt<'_>)->Result<(),RasterInputError>{
 // Same non-CLUT matrix/TRC/analytic domain as the existing color candidate.
 // Reject an unsupported recipe before constructing its acceleration tables.
 if !matches!(profile.channels(),1|3)||profile.a2b.iter().chain(&profile.b2a).any(Option::is_some){return Err(RasterInputError::UnsupportedConversion);}
 for curve in profile.trc.iter().flatten().chain(profile.gray_trc.iter()) {
  poll(control)?;
  let size=match curve{Curve::Table(v)=>v.len(),Curve::Table64(v)=>v.len(),_=>0};
  if size>4096{return Err(RasterInputError::UnsupportedConversion);}
 }
 Ok(())
}
fn digest(bytes:&[u8],bound:usize,control:&Interrupt<'_>)->Result<[u8;32],RasterInputError>{
 use sha2::{Digest,Sha256};
 if bytes.len()>bound{return Err(RasterInputError::LimitExceeded);}
 let mut hash=Sha256::new();for chunk in bytes.chunks(4096){poll(control)?;hash.update(chunk);}poll(control)?;Ok(hash.finalize().into())
}
/// Owns the pair's complete acceleration/Post storage, while the exact original
/// node remains immutably borrowed. It cannot manufacture a source qualification.
/// PreparedConfiguration remains closure-local because it borrows decoded fixed
/// configuration scratch; this avoids a self-referential owned structure.
pub struct OwnedConfiguredColorCandidate<'a> {
 bound:BoundSource<'a>,
 forward:Transform,
 inverse:Transform,
 source:WorkingContext,
 working:WorkingContext,
 retained_bytes:u64,
 work_per_pixel:u64,
 preparation_work:u64,
 ceiling:Limits,
}
impl OwnedConfiguredColorCandidate<'_> {
 pub fn original_node(&self)->&FilterNode{self.bound.original_node()}
 pub fn preparation_work(&self)->u64{self.preparation_work}
 pub fn retained_bytes(&self)->u64{self.retained_bytes}
 fn pair(&self)->TransformPair<'_>{TransformPair{source:self.source,working:self.working,forward:&self.forward,inverse:&self.inverse,retained_bytes:self.retained_bytes,work_per_pixel:self.work_per_pixel}}
 /// Configuration is prepared before the first ROI/source copy. Total work
 /// retains constructor/hash/preparation charges, with no upstream refunds.
 pub fn render(&self,source:&Surface,extent:Rect,roi:Rect,actual_source:WorkingContext,working_default:&[u8],policy:OutputPolicy,limits:Limits,control:&Interrupt<'_>)->Result<RenderedCandidate,RasterInputError>{
  poll(control)?;
  if actual_source!=self.source{return Err(RasterInputError::UnsupportedConversion);}
  if limits.resident_bytes>self.ceiling.resident_bytes||limits.work_units>self.ceiling.work_units{return Err(RasterInputError::LimitExceeded);}
  let scan_work=requirements_preparation_work(source,&self.pair())?;
  let execution=Limits{resident_bytes:limits.resident_bytes,work_units:limits.work_units.checked_sub(self.preparation_work).and_then(|n|n.checked_sub(CONFIG_WORK)).and_then(|n|n.checked_sub(scan_work)).ok_or(RasterInputError::LimitExceeded)?};
  let config_limits=Limits{resident_bytes:execution.resident_bytes.checked_sub(self.retained_bytes).ok_or(RasterInputError::LimitExceeded)?.min(CONFIG_BYTES),work_units:CONFIG_WORK};
  self.bound.with_prepared(self.working,config_limits,control,|prepared|{
   render_prepared_color_candidate(prepared,source,extent,roi,actual_source,&self.pair(),working_default,policy,execution,control)
  })
 }
}
/// Source/working profiles must already represent the caller's explicit transfer
/// contexts. Exact encoded-byte SHA256s and channel/alpha identities are checked;
/// this does not establish their provenance or reinterpret a retained source ICC.
/// Constructors preserve RelativeColorimetric/no-BPC and existing native ROI math.
/// Both profile/encoding and constructor envelopes are admitted before copying.
#[allow(clippy::too_many_arguments)]
pub fn prepare_owned_color_candidate<'a>(node:&'a FilterNode,binding:SourceBinding<'a>,sampling:CurveSamplePolicy,actual_source:WorkingContext,source_profile:&Profile,working_profile:&Profile,limits:Limits,control:&Interrupt<'_>)->Result<OwnedConfiguredColorCandidate<'a>,RasterInputError>{
 poll(control)?;
 if limits.resident_bytes==0||limits.resident_bytes>MAX_CHECKED_RASTER_BYTES||!matches!(actual_source.color_channels,1|3)||actual_source.color_channels!=source_profile.channels()||binding.working.color_channels!=working_profile.channels()||actual_source.alpha!=binding.working.alpha{return Err(RasterInputError::UnsupportedConversion);}
 let bound=bind_source_node_with_sample_policy(node,binding,sampling,control)?;
 profile_domain(source_profile,control)?;profile_domain(working_profile,control)?;
 // Existing typed domain is five channels, <=64 points and canonical256 stored
 // tables. Reserve its complete fixed local preparation envelope before tables.
 let resident=limits.resident_bytes.checked_sub(CONFIG_BYTES).ok_or(RasterInputError::LimitExceeded)?;
 let work_limit=limits.work_units.checked_sub(CONFIG_WORK).and_then(|n|n.checked_sub(CENSUS_WORK)).ok_or(RasterInputError::LimitExceeded)?;
 let maximum=control::Limits::default();
 let cms_limits=control::Limits{resident_bytes:usize::try_from(resident.min(maximum.resident_bytes as u64)).map_err(|_|RasterInputError::LimitExceeded)?,work_units:work_limit.min(maximum.work_units)};
 let cancelled=||control.cancelled();
 let se=source_profile.encode_requirements_with_control(cms_limits,&cancelled).map_err(cms_error)?;
 let we=working_profile.encode_requirements_with_control(cms_limits,&cancelled).map_err(cms_error)?;
 let encoding_peak=se.peak_resident_bytes.checked_add(we.peak_resident_bytes).ok_or(RasterInputError::LimitExceeded)?;
 let source_hash_bound=se.encoded_bytes_upper_bound.max(se.retained_bytes);
 let working_hash_bound=we.encoded_bytes_upper_bound.max(we.retained_bytes);
 let encoding_work=add(mul(add(se.work_units,we.work_units)?,3)?,add(source_hash_bound as u64,working_hash_bound as u64)?)?;
 let pair_limits=lowered(cms_limits,encoding_peak,encoding_work)?;
 let options=photocraft_cms::transform::TransformOptions{intent:Intent::RelativeColorimetric,bpc:false,..Default::default()};
 let f=Transform::requirements_with_control(source_profile,working_profile,options,pair_limits,&cancelled).map_err(cms_error)?;
 let r=Transform::requirements_with_control(working_profile,source_profile,options,pair_limits,&cancelled).map_err(cms_error)?;
 let pair_peak=f.peak_resident_bytes.checked_add(r.peak_resident_bytes).ok_or(RasterInputError::LimitExceeded)?;
 let pair_work=mul(add(f.work_units,r.work_units)?,3)?;
 if pair_peak>pair_limits.resident_bytes||pair_work>pair_limits.work_units{return Err(RasterInputError::LimitExceeded);}
 // No raster/source clone occurs during either schema validation or preparation.
 bound.with_prepared(binding.working,Limits{resident_bytes:CONFIG_BYTES,work_units:CONFIG_WORK},control,|_|Ok(()))?;
 let source_bytes=source_profile.to_bytes_with_control(lowered(cms_limits,we.peak_resident_bytes,we.work_units)?,&cancelled).map_err(cms_error)?;
 let working_bytes=working_profile.to_bytes_with_control(lowered(cms_limits,se.peak_resident_bytes,se.work_units)?,&cancelled).map_err(cms_error)?;
 if digest(&source_bytes,source_hash_bound,control)?!=actual_source.profile_identity||digest(&working_bytes,working_hash_bound,control)?!=binding.working.profile_identity{return Err(RasterInputError::UnsupportedConversion);}
 let forward=Transform::with_options_with_control(source_profile,working_profile,options,lowered(pair_limits,r.peak_resident_bytes,mul(r.work_units,3)?)?,&cancelled).map_err(cms_error)?;
 let inverse=Transform::with_options_with_control(working_profile,source_profile,options,lowered(pair_limits,f.peak_resident_bytes,mul(f.work_units,3)?)?,&cancelled).map_err(cms_error)?;
 let retained=add(forward.retained_bytes_with_control(pair_limits,&cancelled).map_err(cms_error)? as u64,inverse.retained_bytes_with_control(pair_limits,&cancelled).map_err(cms_error)? as u64)?;
 let (_,fw)=transform_requirements(&forward,control)?;let (_,rw)=transform_requirements(&inverse,control)?;
 // The actual pipeline scan is prepaid from the existing bounded 32-stage,
 // three-channel <=4096-table candidate domain, once for each pair member.
 let preparation_work=add(add(add(CONFIG_WORK,encoding_work)?,pair_work)?,CENSUS_WORK)?;
 if preparation_work>limits.work_units||retained.checked_add(CONFIG_BYTES).is_none_or(|n|n>limits.resident_bytes){return Err(RasterInputError::LimitExceeded);}
 poll(control)?;
 Ok(OwnedConfiguredColorCandidate{bound,forward,inverse,source:actual_source,working:binding.working,retained_bytes:retained,work_per_pixel:add(fw,rw)?,preparation_work,ceiling:limits})
}

#[cfg(test)]
mod tests {
 use super::*;
 use crate::gimp_configured_candidate::{Kind,LEVELS_SCHEMA};
 use photocraft_doc::{item_identity::FilterId,xcf_filters::{FilterArgument,FilterValue,ExactNumber,OperationIdentity}};
 fn fixture()->FilterNode {
  let channels=["value","red","green","blue","alpha"].map(|name|FilterArgument{name:name.into(),value:FilterValue::Config([("low-input",0.0f64),("high-input",1.0),("gamma",1.0),("low-output",0.0),("high-output",1.0)].map(|(name,v)|FilterArgument{name:name.into(),value:FilterValue::Number(ExactNumber::Float64Bits(v.to_bits()))}).into())});
  FilterNode{id:FilterId(1),operation:OperationIdentity{name:"gimp:levels".into(),stored_version:"0:0".into(),schema_epoch:"owned-test".into()},source_display_name:None,source_icon:None,enabled:true,opacity_f64_bits:1f64.to_bits(),source_clipping:Some(0),source_region:Some(0),mask:None,auxiliary:vec![],composition:None,arguments:vec![FilterArgument{name:"trc".into(),value:FilterValue::Enum{domain:"gimp:levels::trc".into(),value:1}},FilterArgument{name:"clamp-input".into(),value:FilterValue::Bool(false)},FilterArgument{name:"clamp-output".into(),value:FilterValue::Bool(false)},FilterArgument{name:"config".into(),value:FilterValue::Config(channels.into())}]}
 }
 fn context(profile:&Profile)->WorkingContext{let bytes=profile.to_bytes();WorkingContext{transfer:photocraft_algo::gimp::configured::Transfer::NonlinearProfile,profile_identity:digest(&bytes,bytes.len(),&Interrupt::NONE).unwrap(),color_channels:3,alpha:true}}
 fn binding(context:WorkingContext)->SourceBinding<'static>{SourceBinding{xcf_version:22,expected_epoch:"owned-test",schema_sha256:LEVELS_SCHEMA,kind:Kind::Levels,working:context}}
 fn limits()->Limits{Limits{resident_bytes:256<<20,work_units:1_000_000_000}}
 #[test]
 fn owned_pair_preserves_exact_original_constructor_and_borrowed_config() {
  let node=fixture();let before=node.clone();let profile=photocraft_cms::Builtin::Srgb.profile();let context=context(profile);
  let prepared=prepare_owned_color_candidate(&node,binding(context),CurveSamplePolicy::RequireExact256,context,profile,profile,limits(),&Interrupt::NONE).unwrap();
  let old=Transform::new(profile,profile,Intent::RelativeColorimetric,false).unwrap();assert_eq!(prepared.forward.pipeline(),old.pipeline());assert_eq!(prepared.inverse.pipeline(),old.pipeline());assert_eq!(prepared.original_node(),&before);
  prepared.bound.with_prepared(context,Limits{resident_bytes:CONFIG_BYTES,work_units:CONFIG_WORK},&Interrupt::NONE,|config|{let input=[0.125,0.5,0.875,0.25];let mut output=[0.0;4];config.try_eval_pixel(&input,&mut output,context,&Interrupt::NONE)?;assert_eq!(input.map(f64::to_bits),output.map(f64::to_bits));Ok(())}).unwrap();
 }
 #[test]
 fn changed_profile_identity_schema_and_resource_limits_refuse_before_roi() {
  let node=fixture();let profile=photocraft_cms::Builtin::Srgb.profile();let context=context(profile);let mut stale=context;stale.profile_identity[0]^=1;
  assert!(matches!(prepare_owned_color_candidate(&node,binding(context),CurveSamplePolicy::RequireExact256,stale,profile,profile,limits(),&Interrupt::NONE),Err(RasterInputError::UnsupportedConversion)));
  let mut wrong=binding(context);wrong.schema_sha256="wrong";assert!(prepare_owned_color_candidate(&node,wrong,CurveSamplePolicy::RequireExact256,context,profile,profile,limits(),&Interrupt::NONE).is_err());
  assert!(matches!(prepare_owned_color_candidate(&node,binding(context),CurveSamplePolicy::RequireExact256,context,profile,profile,Limits{resident_bytes:1,work_units:1},&Interrupt::NONE),Err(RasterInputError::LimitExceeded)));
  let stop=||true;assert!(matches!(prepare_owned_color_candidate(&node,binding(context),CurveSamplePolicy::RequireExact256,context,profile,profile,limits(),&Interrupt::cancel_only(&stop)),Err(RasterInputError::Cancelled)));
 }
}
