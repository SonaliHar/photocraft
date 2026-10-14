//! Controlled ordinary CMYK scope. Legacy compatibility fallback stays explicit;
//! cancellation/resource failures never become None, and no global cache grows.
use super::*;
use photocraft_cms::{control::{Limits,Error,Result},transform::TransformOptions};
use std::hash::{Hash,Hasher};
#[derive(Clone,Copy,Debug)]
pub struct CmykSpaceRequirements {
 pub retained_bytes:usize,
 pub peak_resident_bytes:usize,
 pub work_units:u64,
 /// Cold transforms require decoded recipes. The supplied lower envelope is
 /// reserved upfront; exact phase requirements are checked within it afterwards.
 pub cold:bool,
}
fn poll(cancelled:&dyn Fn()->bool)->Result<()>{if cancelled(){Err(Error::Cancelled)}else{Ok(())}}
fn add(a:usize,b:usize)->Result<usize>{a.checked_add(b).ok_or(Error::Limit("CMYK scope bytes"))}
fn work(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or(Error::Limit("CMYK scope work"))}
fn check(limits:Limits,bytes:usize,units:u64,cancelled:&dyn Fn()->bool)->Result<()>{poll(cancelled)?;let maximum=Limits::default();if limits.resident_bytes>maximum.resident_bytes||limits.work_units>maximum.work_units{return Err(Error::Limit("limits may only be lowered"))}if bytes>limits.resident_bytes||units>limits.work_units{return Err(Error::Limit("CMYK aggregate preparation"))}Ok(())}
fn remaining(limits:Limits,outside:usize,used:u64)->Result<Limits>{Ok(Limits{resident_bytes:limits.resident_bytes.checked_sub(outside).ok_or(Error::Limit("CMYK retained headroom"))?,work_units:limits.work_units.checked_sub(used).ok_or(Error::Limit("CMYK remaining work"))?})}
fn retained(space:&CmykSpace,limits:Limits,cancelled:&dyn Fn()->bool)->Result<usize>{
 let to=space.to_srgb.retained_bytes_with_control(limits,cancelled)?;let from=space.from_srgb.retained_bytes_with_control(limits,cancelled)?;
 add(add(to,from)?,std::mem::size_of::<u64>()+2*std::mem::size_of::<usize>())
}
fn peek(bytes:&Arc<Vec<u8>>,limits:Limits,cancelled:&dyn Fn()->bool)->Result<Option<Option<Arc<CmykSpace>>>> {
 let Some(cache)=CMYK_PROFILE_CACHE.get()else{return Ok(None)};let entries=cache.lock().unwrap_or_else(|e|e.into_inner());if entries.len() as u64>limits.work_units{return Err(Error::Limit("CMYK cache scan work"))}
 for(owner,space)in entries.iter(){poll(cancelled)?;if owner.as_ptr()==Arc::as_ptr(bytes)&&owner.strong_count()>0{return Ok(Some(space.as_ref().map(Arc::clone)))}}Ok(None)
}
fn header_cmyk(bytes:&[u8])->bool{bytes.get(16..20)==Some(b"CMYK".as_slice())}
pub fn requirements(icc:Option<&Arc<Vec<u8>>>,limits:Limits,cancelled:&dyn Fn()->bool)->Result<CmykSpaceRequirements>{
 check(limits,0,0,cancelled)?;let Some(bytes)=icc else{return Ok(CmykSpaceRequirements{retained_bytes:0,peak_resident_bytes:0,work_units:0,cold:false})};check(limits,bytes.capacity(),0,cancelled)?;
 if let Some(hit)=peek(bytes,limits,cancelled)?{let size=hit.as_ref().map(|s|retained(s,remaining(limits,bytes.capacity(),0)?,cancelled)).transpose()?.unwrap_or(0);let peak=add(bytes.capacity(),size)?;let cache_work=CMYK_PROFILE_CACHE.get().map(|cache|cache.lock().unwrap_or_else(|e|e.into_inner()).len() as u64).unwrap_or(0);let warm_work=cache_work.checked_mul(3).and_then(|v|v.checked_add(32768)).ok_or(Error::Limit("CMYK cache/census work"))?;check(limits,peak,warm_work,cancelled)?;return Ok(CmykSpaceRequirements{retained_bytes:peak,peak_resident_bytes:peak,work_units:cache_work.checked_mul(3).and_then(|v|v.checked_add(32768)).ok_or(Error::Limit("CMYK cache/census work"))?,cold:false})}
 match Profile::parse_requirements_with_control(bytes,remaining(limits,bytes.capacity().checked_sub(bytes.len()).ok_or(Error::Limit("CMYK ICC capacity"))?,0)?,cancelled){Err(Error::Cms(_))=>{let units=work(bytes.len() as u64,64)?;check(limits,bytes.capacity(),units,cancelled)?;return Ok(CmykSpaceRequirements{retained_bytes:bytes.capacity(),peak_resident_bytes:bytes.capacity(),work_units:units,cold:false})},Err(e)=>return Err(e),Ok(_)=>()}
 if !header_cmyk(bytes){return Ok(CmykSpaceRequirements{retained_bytes:bytes.capacity(),peak_resident_bytes:bytes.capacity(),work_units:limits.work_units,cold:false})}
 // No arbitrary ICC size/class cap: decoding uses existing CMS limits. Complete
 // cold preparation reserves the caller envelope, then carries actual retained
 // owners and cumulative parse/hash/constructor work through each exact phase.
 Ok(CmykSpaceRequirements{retained_bytes:bytes.capacity(),peak_resident_bytes:limits.resident_bytes,work_units:limits.work_units,cold:true})
}
fn hash_bytes(bytes:&[u8],remaining_work:u64,cancelled:&dyn Fn()->bool)->Result<u64>{
 if (bytes.len() as u64).checked_add(8).is_none_or(|n|n>remaining_work){return Err(Error::Limit("CMYK ICC hash work"))}let mut hasher=std::collections::hash_map::DefaultHasher::new();bytes.len().hash(&mut hasher);for part in bytes.chunks(4096){poll(cancelled)?;hasher.write(part);}poll(cancelled)?;Ok(hasher.finish())
}
fn compatibility<T>(value:Result<T>)->Result<Option<T>>{match value{Ok(v)=>Ok(Some(v)),Err(Error::Cms(_))=>Ok(None),Err(e)=>Err(e)}}
pub fn prepare(icc:Option<&Arc<Vec<u8>>>,limits:Limits,cancelled:&dyn Fn()->bool)->Result<Option<Arc<CmykSpace>>>{
 let _=requirements(icc,limits,cancelled)?;let Some(bytes)=icc else{return Ok(None)};
 if let Some(hit)=peek(bytes,limits,cancelled)?{if let Some(space)=&hit{let size=retained(space,remaining(limits,bytes.capacity(),0)?,cancelled)?;check(limits,add(bytes.capacity(),size)?,0,cancelled)?;}poll(cancelled)?;return Ok(hit)}
 let spare=bytes.capacity().checked_sub(bytes.len()).ok_or(Error::Limit("CMYK ICC capacity"))?;let parsing_limits=remaining(limits,spare,0)?;
 let Some(parser)=compatibility(Profile::parse_requirements_with_control(bytes,parsing_limits,cancelled))? else{return Ok(None)};
 if !header_cmyk(bytes){poll(cancelled)?;return Ok(None)}
 let mut used=parser.work_units.checked_mul(3).ok_or(Error::Limit("CMYK parser work"))?;check(limits,add(parser.peak_resident_bytes,spare)?,used,cancelled)?;
 let Some(profile)=compatibility(Profile::parse_with_control(bytes,remaining(parsing_limits,0,parser.work_units)?,cancelled))? else{return Ok(None)};
 if profile.color_space!=ColorSpace::Cmyk{return Ok(None)}
 let assigned=profile.clone_requirements_with_control(limits,cancelled)?.retained_bytes;
 check(limits,add(bytes.capacity(),assigned)?,used,cancelled)?;
 // Builtin OnceLock initialization is an explicit outer-polled provider boundary.
 poll(cancelled)?;let coated=Builtin::CoatedCmyk.profile();poll(cancelled)?;
 let coated_ret=coated.clone_requirements_with_control(limits,cancelled)?.retained_bytes;
 let hash_limits=remaining(limits,add(bytes.capacity(),assigned)?,used)?;
 let encoded=coated.to_bytes_with_control(hash_limits,cancelled)?;
 check(limits,add(add(bytes.capacity(),assigned)?,coated_ret)?,used,cancelled)?;
 let id=hash_bytes(bytes,limits.work_units.checked_sub(used).ok_or(Error::Limit("CMYK hash work"))?,cancelled)?;used=work(used,work(bytes.len() as u64,8)?)?;
 let default_id=hash_bytes(&encoded,limits.work_units.checked_sub(used).ok_or(Error::Limit("CMYK default hash work"))?,cancelled)?;used=work(used,work(encoded.len() as u64,8)?)?;
 if id==default_id{return Ok(None)}
 poll(cancelled)?;let srgb=Builtin::Srgb.profile();poll(cancelled)?;
 // The assigned/sRGB models are included by Transform's own retained census.
 // Original input and coated comparison owners remain outside that census.
 let outside=add(bytes.capacity(),coated_ret)?;let first_limits=remaining(limits,outside,used)?;let options=TransformOptions{intent:Intent::RelativeColorimetric,bpc:true,..Default::default()};
 let Some(to_req)=compatibility(Transform::requirements_with_control(&profile,srgb,options,first_limits,cancelled))? else{return Ok(None)};
 check(limits,add(outside,to_req.peak_resident_bytes)?,work(used,to_req.work_units)?,cancelled)?;
 let Some(to_srgb)=compatibility(Transform::with_options_with_control(&profile,srgb,options,first_limits,cancelled))? else{return Ok(None)};used=work(used,to_req.work_units)?;
 let to_ret=to_srgb.retained_bytes_with_control(first_limits,cancelled)?;let second_outside=add(outside,to_ret)?;let second_limits=remaining(limits,second_outside,used)?;
 let Some(from_req)=compatibility(Transform::requirements_with_control(srgb,&profile,options,second_limits,cancelled))? else{return Ok(None)};
 check(limits,add(second_outside,from_req.peak_resident_bytes)?,work(used,from_req.work_units)?,cancelled)?;
 let Some(from_srgb)=compatibility(Transform::with_options_with_control(srgb,&profile,options,second_limits,cancelled))? else{return Ok(None)};
 let final_ret=add(add(to_ret,from_srgb.retained_bytes_with_control(second_limits,cancelled)?)?,std::mem::size_of::<u64>()+2*std::mem::size_of::<usize>())?;
 check(limits,add(add(second_outside,assigned)?,final_ret)?,work(used,from_req.work_units)?,cancelled)?;poll(cancelled)?;
 Ok(Some(Arc::new(CmykSpace{to_srgb,from_srgb,id})))
}

#[cfg(test)]
mod tests {
 use super::*;
 use std::cell::Cell;
 #[test]
 fn streamed_default_hasher_matches_profile_vec_hash_at_chunk_boundaries() {
  for n in [0,1,7,8,13,1023,1024,1025,4095,4096,4097,8193]{let bytes=(0..n).map(|i|(i%251) as u8).collect::<Vec<_>>();let mut expected=std::collections::hash_map::DefaultHasher::new();bytes.hash(&mut expected);assert_eq!(hash_bytes(&bytes,u64::MAX,&||false).unwrap(),expected.finish());}
  let profile=Builtin::CoatedCmyk.profile();assert_eq!(hash_bytes(&profile.to_bytes(),u64::MAX,&||false).unwrap(),profile.content_hash());
 }
 #[test]
 fn default_invalid_and_non_cmyk_preserve_none_but_resources_do_not_fallback() {
  assert!(prepare(None,Limits::default(),&||false).unwrap().is_none());let invalid=Arc::new(vec![0;140]);assert!(prepare(Some(&invalid),Limits::default(),&||false).unwrap().is_none());let rgb=Builtin::Srgb.profile().to_bytes();assert!(prepare(Some(&rgb),Limits::default(),&||false).unwrap().is_none());let coated=Builtin::CoatedCmyk.profile().to_bytes();assert!(prepare(Some(&coated),Limits::default(),&||false).unwrap().is_none());assert!(matches!(prepare(Some(&coated),Limits{resident_bytes:1,work_units:1},&||false),Err(Error::Limit(_))));
 }
 #[test]
 fn cold_private_factory_matches_legacy_space_and_readonly_cache_hit() {
  let mut raw=Builtin::CoatedCmyk.profile().to_bytes().as_ref().clone();raw.extend_from_slice(b"owned raw tail distinct identity");let bytes=Arc::new(raw);let original=bytes.as_ref().clone();assert!(peek(&bytes,Limits::default(),&||false).unwrap().is_none());
  let current=prepare(Some(&bytes),Limits::default(),&||false).unwrap().unwrap();assert!(peek(&bytes,Limits::default(),&||false).unwrap().is_none());let legacy=CmykSpace::for_profile(Some(&bytes)).unwrap();assert_eq!(current.id,legacy.id);assert_eq!(current.to_srgb.pipeline(),legacy.to_srgb.pipeline());assert_eq!(current.from_srgb.pipeline(),legacy.from_srgb.pipeline());
  let hit=prepare(Some(&bytes),Limits::default(),&||false).unwrap().unwrap();assert!(Arc::ptr_eq(&legacy,&hit));assert_eq!(bytes.as_ref(),&original);
 }
 #[test]
 fn cancellation_preserves_source_bytes_and_does_not_cache_a_partial_space() {
  let mut raw=Builtin::CoatedCmyk.profile().to_bytes().as_ref().clone();raw.extend_from_slice(b"owned cancellation source");let bytes=Arc::new(raw);let before=bytes.as_ref().clone();let calls=Cell::new(0);assert!(matches!(prepare(Some(&bytes),Limits::default(),&||{calls.set(calls.get()+1);calls.get()>50}),Err(Error::Cancelled)));assert_eq!(bytes.as_ref(),&before);assert!(peek(&bytes,Limits::default(),&||false).unwrap().is_none());
 }
}

#[derive(Clone,Copy,Debug)]
pub struct DefaultCmykRequirements {pub retained_bytes:usize,pub peak_resident_bytes:usize,pub work_units:u64,pub cold:bool}
struct DefaultPart {peak:usize,retained:usize,work:u64,missing:bool}
fn default_part(cache:&OnceLock<Option<Transform>>,source:&Profile,destination:&Profile,limits:Limits,cancelled:&dyn Fn()->bool)->Result<DefaultPart>{
 if let Some(current)=cache.get(){let retained=current.as_ref().map(|t|t.retained_bytes_with_control(limits,cancelled)).transpose()?.unwrap_or(0);return Ok(DefaultPart{peak:retained,retained,work:32768,missing:false})}
 let options=TransformOptions{intent:Intent::RelativeColorimetric,bpc:true,..Default::default()};
 let required=match Transform::requirements_with_control(source,destination,options,limits,cancelled){Ok(v)=>v,Err(Error::Cms(_))=>return Ok(DefaultPart{peak:0,retained:0,work:32768,missing:true}),Err(e)=>return Err(e)};
 Ok(DefaultPart{peak:required.peak_resident_bytes,retained:required.retained_bytes,work:required.work_units,missing:true})
}
/// Complete paired reservation before default table allocation. Initialized
/// cache entries (including legacy negative None) are borrowed without rebuilding.
pub fn default_requirements(limits:Limits,cancelled:&dyn Fn()->bool)->Result<DefaultCmykRequirements>{
 check(limits,0,0,cancelled)?;
 if let(Some(to),Some(from))=(DEFAULT_CMYK_TO_SRGB.get(),DEFAULT_SRGB_TO_CMYK.get()){
  let to=to.as_ref().map(|v|v.retained_bytes_with_control(limits,cancelled)).transpose()?.unwrap_or(0);let from=from.as_ref().map(|v|v.retained_bytes_with_control(limits,cancelled)).transpose()?.unwrap_or(0);let bytes=add(to,from)?;check(limits,bytes,65536,cancelled)?;return Ok(DefaultCmykRequirements{retained_bytes:bytes,peak_resident_bytes:bytes,work_units:65536,cold:false})
 }
 // Builtin OnceLock profile initialization/encoding remains an outer-polled
 // provider boundary; all Transform copies and table work below are controlled.
 poll(cancelled)?;let coated=Builtin::CoatedCmyk.profile();poll(cancelled)?;let srgb=Builtin::Srgb.profile();poll(cancelled)?;
 let first=default_part(&DEFAULT_CMYK_TO_SRGB,coated,srgb,limits,cancelled)?;
 let second_limits=remaining(limits,first.peak,first.work)?;
 let second=default_part(&DEFAULT_SRGB_TO_CMYK,srgb,coated,second_limits,cancelled)?;
 let peak=add(first.peak,second.peak)?;let units=work(first.work,second.work)?.checked_mul(3).ok_or(Error::Limit("default CMYK preparation work"))?;
 check(limits,peak,units,cancelled)?;Ok(DefaultCmykRequirements{retained_bytes:add(first.retained,second.retained)?,peak_resident_bytes:peak,work_units:units,cold:first.missing||second.missing})
}
fn default_candidate(cache:&OnceLock<Option<Transform>>,source:&Profile,destination:&Profile,limits:Limits,cancelled:&dyn Fn()->bool)->Result<Option<Option<Transform>>>{
 if cache.get().is_some(){return Ok(None)}
 Ok(Some(compatibility(Transform::new_with_control(source,destination,Intent::RelativeColorimetric,true,limits,cancelled))?))
}
fn default_pair_candidates(to:&OnceLock<Option<Transform>>,from:&OnceLock<Option<Transform>>,limits:Limits,cancelled:&dyn Fn()->bool)->Result<(Option<Option<Transform>>,Option<Option<Transform>>)> {
 poll(cancelled)?;let coated=Builtin::CoatedCmyk.profile();poll(cancelled)?;let srgb=Builtin::Srgb.profile();poll(cancelled)?;
 let first=default_part(to,coated,srgb,limits,cancelled)?;let second_limits=remaining(limits,first.peak,first.work)?;let second=default_part(from,srgb,coated,second_limits,cancelled)?;
 let units=work(first.work,second.work)?.checked_mul(3).ok_or(Error::Limit("default CMYK preparation work"))?;
 check(limits,add(first.peak,second.peak)?,units,cancelled)?;
 let first_value=default_candidate(to,coated,srgb,limits,cancelled)?;
 let second_value=default_candidate(from,srgb,coated,second_limits,cancelled)?;
 poll(cancelled)?;Ok((first_value,second_value))
}
fn publish_defaults(to:&OnceLock<Option<Transform>>,from:&OnceLock<Option<Transform>>,values:(Option<Option<Transform>>,Option<Option<Transform>>),cancelled:&dyn Fn()->bool)->Result<()> {
 // No cancellation/fallible preparation after this final fence. A concurrent
 // legacy initializer wins normally; discard our unused prepared candidate.
 poll(cancelled)?;if let Some(value)=values.0{let _=to.set(value);}if let Some(value)=values.1{let _=from.set(value);}Ok(())
}
pub fn warm_defaults(limits:Limits,cancelled:&dyn Fn()->bool)->Result<()>{
 let required=default_requirements(limits,cancelled)?;if !required.cold{poll(cancelled)?;return Ok(())}
 let values=default_pair_candidates(&DEFAULT_CMYK_TO_SRGB,&DEFAULT_SRGB_TO_CMYK,limits,cancelled)?;
 publish_defaults(&DEFAULT_CMYK_TO_SRGB,&DEFAULT_SRGB_TO_CMYK,values,cancelled)
}

#[cfg(test)]
mod default_tests {
 use super::*;
 #[test]
 fn controlled_default_pair_preserves_legacy_recipe_and_pixel_words() {
  let to=OnceLock::new();let from=OnceLock::new();
  let candidates=default_pair_candidates(&to,&from,Limits::default(),&||false).unwrap();
  publish_defaults(&to,&from,candidates,&||false).unwrap();
  let actual=to.get().unwrap().as_ref().unwrap();
  let reverse=from.get().unwrap().as_ref().unwrap();
  let legacy=Transform::new(Builtin::CoatedCmyk.profile(),Builtin::Srgb.profile(),Intent::RelativeColorimetric,true).unwrap();
  let legacy_reverse=Transform::new(Builtin::Srgb.profile(),Builtin::CoatedCmyk.profile(),Intent::RelativeColorimetric,true).unwrap();
  assert_eq!(actual.pipeline(),legacy.pipeline());assert_eq!(reverse.pipeline(),legacy_reverse.pipeline());
  for pixel in [[0.0,0.0,0.0,0.0],[0.25,0.5,0.75,0.125],[1.0,1.0,1.0,1.0]] {
   let mut a=[0.0;3];let mut b=[0.0;3];actual.eval_fast(&pixel,&mut a);legacy.eval_fast(&pixel,&mut b);assert_eq!(a.map(f32::to_bits),b.map(f32::to_bits));
  }
  for pixel in [[0.0,0.0,0.0],[0.125,0.5,0.875],[1.0,1.0,1.0]] {
   let mut a=[0.0;4];let mut b=[0.0;4];reverse.eval_fast(&pixel,&mut a);legacy_reverse.eval_fast(&pixel,&mut b);assert_eq!(a.map(f32::to_bits),b.map(f32::to_bits));
  }
  assert!(CmykSpace::for_profile_with_control(None,Limits::default(),&||false).unwrap().is_none());
 }
 #[test]
 fn cancelled_final_fence_publishes_neither_default_candidate() {
  let to=OnceLock::new();let from=OnceLock::new();
  // Negative values make this final-publication test independent of table costs.
  assert!(matches!(publish_defaults(&to,&from,(Some(None),Some(None)),&||true),Err(Error::Cancelled)));
  assert!(to.get().is_none());assert!(from.get().is_none());
 }
 #[test]
 fn resource_or_construction_cancellation_leaves_local_defaults_uninitialized() {
  let to=OnceLock::new();let from=OnceLock::new();
  assert!(matches!(default_pair_candidates(&to,&from,Limits{resident_bytes:1,work_units:1},&||false),Err(Error::Limit(_))));
  assert!(to.get().is_none());assert!(from.get().is_none());
  let calls=std::cell::Cell::new(0);
  assert!(matches!(default_pair_candidates(&to,&from,Limits::default(),&||{calls.set(calls.get()+1);calls.get()>20}),Err(Error::Cancelled)));
  assert!(to.get().is_none());assert!(from.get().is_none());
 }
 #[test]
 fn initialized_negative_default_is_not_rebuilt_or_replaced() {
  let to=OnceLock::new();let from=OnceLock::new();assert!(to.set(None).is_ok());assert!(from.set(None).is_ok());
  let values=default_pair_candidates(&to,&from,Limits::default(),&||false).unwrap();assert!(values.0.is_none());assert!(values.1.is_none());
  publish_defaults(&to,&from,values,&||false).unwrap();assert!(to.get().unwrap().is_none());assert!(from.get().unwrap().is_none());
 }
}
