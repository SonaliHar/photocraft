//! Cooperative companion to the existing Transform recipe. No cache insertion,
//! profile interpretation changes or workspace dependencies.
use super::*;
use crate::control::{Budget,Error,Limits,Result,invalid};
#[derive(Clone,Copy,Debug)]
pub struct TransformRequirements {pub retained_bytes:usize,pub peak_resident_bytes:usize,pub work_units:u64}
fn add(a:usize,z:usize)->Result<usize>{a.checked_add(z).ok_or(Error::Limit("transform storage overflow"))}
fn mul(a:usize,z:usize)->Result<usize>{a.checked_mul(z).ok_or(Error::Limit("transform storage overflow"))}
fn work_add(a:u64,z:u64)->Result<u64>{a.checked_add(z).ok_or(Error::Limit("transform work overflow"))}
fn curves_equal(a:&[Curve],z:&[Curve],b:&mut Budget<'_>)->Result<bool>{if a.len()!=z.len(){return Ok(false)}for(x,y)in a.iter().zip(z){if !x.equals_controlled(y,b)?{return Ok(false)}}Ok(true)}
fn stages_equal(a:&[Stage],z:&[Stage],b:&mut Budget<'_>)->Result<bool>{if a.len()!=z.len(){return Ok(false)}for(x,y)in a.iter().zip(z){b.poll()?;let same=match(x,y){(Stage::Curves(x),Stage::Curves(y))|(Stage::InvCurves(x),Stage::InvCurves(y))=>curves_equal(x,y,b)?,(Stage::Clut(x),Stage::Clut(y))=>x.equals_controlled(y,b)?,(Stage::Matrix{rows:r,cols:c,m:x,offset:o},Stage::Matrix{rows:s,cols:d,m:y,offset:p})=>{if r!=s||c!=d||x.len()!=y.len()||o.len()!=p.len(){false}else{let mut same=true;for(a,z)in x.iter().chain(o).zip(y.iter().chain(p)){b.work(1)?;if a!=z{same=false;break;}}same}},_=>x==y};if !same{return Ok(false)}}Ok(true)}
fn model_equal(a:&Profile,z:&Profile,b:&mut Budget<'_>)->Result<bool>{
 if a.version!=z.version||a.class!=z.class||a.color_space!=z.color_space||a.pcs!=z.pcs||a.white_point!=z.white_point||a.black_point!=z.black_point||a.chad!=z.chad||a.matrix!=z.matrix{return Ok(false)}
 match(&a.trc,&z.trc){(Some(x),Some(y))=>if !curves_equal(x,y,b)?{return Ok(false)},(None,None)=>(),_=>return Ok(false)}
 match(&a.gray_trc,&z.gray_trc){(Some(x),Some(y))=>if !x.equals_controlled(y,b)?{return Ok(false)},(None,None)=>(),_=>return Ok(false)}
 for(x,y)in a.a2b.iter().chain(&a.b2a).zip(z.a2b.iter().chain(&z.b2a)){match(x,y){(Some(x),Some(y))=>if x.kind!=y.kind||x.inputs!=y.inputs||x.outputs!=y.outputs||!stages_equal(&x.stages,&y.stages,b)?{return Ok(false)},(None,None)=>(),_=>return Ok(false)}}Ok(true)
}
fn profile_classes(src:&Profile,dst:&Profile,b:&Budget<'_>)->Result<()>{for p in [src,dst]{b.poll()?;let lab=p.color_space==ColorSpace::Lab&&p.a2b.iter().flatten().all(|l|l.outputs==3)&&p.b2a.iter().flatten().all(|l|l.inputs==3);if matches!(p.class,ProfileClass::DeviceLink|ProfileClass::NamedColor|ProfileClass::Abstract)&&!lab{return Err(CmsError::Unsupported(format!("{:?} profiles cannot be used as a source or destination",p.class)).into());}}Ok(())}
fn cost(p:&Profile,input:bool,intent:Intent,b:&Budget<'_>)->Result<(usize,u64)>{let(bytes,work,_)=p.selected_side_cost(input,intent,b)?;Ok((bytes,work))}
fn identity_envelope(retained:usize)->Result<TransformRequirements>{
 Ok(TransformRequirements{retained_bytes:retained,peak_resident_bytes:add(retained,4096)?,work_units:work_add((retained as u64).checked_mul(4).ok_or(Error::Limit("identity transform work"))?,16<<20)?})
}
fn construction_envelope(retained:usize,copied:usize,side_work:u64,connection_work:u64,nodes:usize,same:bool)->Result<TransformRequirements>{
 let peak=add(retained,add(mul(copied,16)?,8<<20)?)?;
 let evaluations=(nodes as u64).checked_add(if same{128}else{(OUT_TABLE+TABLE_1D+256+128) as u64}).ok_or(Error::Limit("transform evaluations"))?;
 let work=side_work.checked_mul(evaluations).and_then(|v|v.checked_mul(4)).and_then(|v|v.checked_add(connection_work.checked_mul(4)?)).and_then(|v|v.checked_add((retained as u64).checked_mul(8)?)).and_then(|v|v.checked_add(1<<20)).ok_or(Error::Limit("transform sampling work"))?;
 Ok(TransformRequirements{retained_bytes:retained,peak_resident_bytes:peak,work_units:work})
}
#[path="controlled/effective_requirements.rs"]
mod effective_requirements;
pub use effective_requirements::{EffectivePairRequirements,effective_pair_requirements};
pub fn requirements(src:&Profile,dst:&Profile,opts:TransformOptions,limits:Limits,cancelled:&dyn Fn()->bool)->Result<TransformRequirements>{
 let mut b=Budget::new(limits,cancelled)?;let retained=add(src.controlled_retained_bytes(&b)?,dst.controlled_retained_bytes(&b)?)?;b.retained(retained)?;
 // Exact model equality is part of the actual legacy recipe, including original
 // wide CLUT storage/strides. It is cooperatively compared, never tolerance-fit.
 let same=model_equal(src,dst,&mut b)?;
 if same&&matches!(src.color_space,ColorSpace::Rgb|ColorSpace::Gray){crate::xcf_color::validate_profile_with_control(src,limits,cancelled)?;let required=identity_envelope(retained)?;let admit=Budget::new(limits,cancelled)?;admit.admit(required.peak_resident_bytes,required.work_units)?;return Ok(required)}
 profile_classes(src,dst,&b)?;let(sb,sw)=cost(src,true,opts.intent,&b)?;let(db,dw)=cost(dst,false,opts.intent,&b)?;let mut copied=add(sb,db)?;let side_work=work_add(sw,dw)?;let mut connection_work=0u64;
 if opts.bpc&&opts.intent!=Intent::AbsoluteColorimetric{for p in [src,dst]{let(a,w)=cost(p,true,opts.intent,&b)?;copied=add(copied,a)?;connection_work=work_add(connection_work,w)?;let reverse=if opts.intent==Intent::Saturation{Intent::Saturation}else{Intent::RelativeColorimetric};match cost(p,false,reverse,&b){Ok((a,w))=>{copied=add(copied,a)?;connection_work=work_add(connection_work,w)?;},Err(Error::Cms(_))=>(),Err(e)=>return Err(e)}}}
 let has_clut=[src.selected_lut(true,opts.intent),dst.selected_lut(false,opts.intent)].into_iter().flatten().any(|l|l.stages.iter().any(|s|matches!(s,Stage::Clut(_))));
 let nodes=if has_clut&&!same{match src.channels(){1=>TABLE_1D,2=>GRID_3D.checked_pow(2).ok_or(Error::Limit("grid nodes"))?,3=>GRID_3D.checked_pow(3).ok_or(Error::Limit("grid nodes"))?,4=>GRID_4D.checked_pow(4).ok_or(Error::Limit("grid nodes"))?,_=>0}}else{0};
 // Includes all cumulative fallible copies (link/BPC/optimizer/head), merged
 // matrices, worst 15-output fixed grid, shaper/post/grid8 and vector owners.
 let required=construction_envelope(retained,copied,side_work,connection_work,nodes,same)?;
 let admit=Budget::new(limits,cancelled)?;admit.admit(required.peak_resident_bytes,required.work_units)?;Ok(required)
}
fn copy_stages(stages:&[Stage],b:&mut Budget<'_>)->Result<Vec<Stage>>{let mut out=b.reserve(stages.len())?;for s in stages{out.push(s.copy_controlled(b)?);}Ok(out)}
fn optimize(pipe:&mut Pipeline,b:&mut Budget<'_>)->Result<()>{
 pipe.validate_controlled()?;let mut out=b.reserve(pipe.stages.len())?;
 for s in pipe.stages.drain(..){b.poll()?;let noop=match &s{Stage::Curves(cs)|Stage::InvCurves(cs)=>{let mut all=true;for c in cs{if !c.identity_controlled(b)?{all=false;break;}}all},Stage::Matrix{rows:3,cols:3,m,offset}=>offset.iter().all(|v|v.abs()<1e-12)&&m.iter().enumerate().all(|(i,v)|(v-if i%4==0{1.}else{0.}).abs()<1e-12),_=>false};if noop{continue}
 let cancel=matches!((out.last(),&s),(Some(Stage::LabToXyz),Stage::XyzToLab)|(Some(Stage::XyzToLab),Stage::LabToXyz)|(Some(Stage::EncodeLabV4),Stage::DecodeLabV4)|(Some(Stage::EncodeLabV2),Stage::DecodeLabV2)|(Some(Stage::EncodeXyz),Stage::DecodeXyz)|(Some(Stage::DecodeLabV4),Stage::EncodeLabV4)|(Some(Stage::DecodeLabV2),Stage::EncodeLabV2)|(Some(Stage::DecodeXyz),Stage::EncodeXyz));if cancel{out.pop();continue}
 if let(Some(Stage::Matrix{rows:3,cols:3,m:a,offset:oa}),Stage::Matrix{rows:3,cols:3,m:z,offset:oz})=(out.last(),&s){let mut m=b.reserve(9)?;let mut offset=b.reserve(3)?;for r in 0..3{for c in 0..3{let mut terms=[0.;3];for(k,term)in terms.iter_mut().enumerate(){*term=*z.get(r*3+k).ok_or_else(||invalid("matrix merge"))? * *a.get(k*3+c).ok_or_else(||invalid("matrix merge"))?;}m.push(terms.into_iter().sum::<f64>());}let mut terms=[0.;3];for(k,term)in terms.iter_mut().enumerate(){*term=*z.get(r*3+k).ok_or_else(||invalid("matrix merge"))? * *oa.get(k).ok_or_else(||invalid("matrix merge"))?;}offset.push(terms.into_iter().sum::<f64>()+*oz.get(r).ok_or_else(||invalid("matrix merge"))?);}b.work(64)?;out.pop();out.push(Stage::Matrix{rows:3,cols:3,m,offset});continue}out.push(s);
 }pipe.stages=out;b.poll()?;Ok(())
}
fn xyz_y(p:&Profile,stages:Vec<Stage>,pcs:Pcs,dev:&[f32],b:&mut Budget<'_>)->Result<f64>{let pipe=Pipeline::new(p.channels(),stages);let mut out=[0f32;16];pipe.eval_controlled(dev,out.get_mut(..pipe.outputs).ok_or_else(||invalid("black point channels"))?,b)?;let y=match pcs{Pcs::Xyz=>f64::from(out[1]),Pcs::Lab=>math::lab_to_xyz([f64::from(out[0]),0.,0.],math::D50)[1]};Ok(y.clamp(0.,0.5))}
fn black(p:&Profile,intent:Intent,b:&mut Budget<'_>)->Result<f64>{
 if matches!(p.color_space,ColorSpace::Lab|ColorSpace::Xyz){return Ok(0.)}let lut=p.a2b.iter().any(Option::is_some);if lut&&p.version.0>=4&&matches!(intent,Intent::Perceptual|Intent::Saturation){return Ok(PERCEPTUAL_BLACK[1])}
 let(to,pcs)=p.selected_side_controlled(true,intent,b)?;if matches!(p.color_space,ColorSpace::Rgb|ColorSpace::Gray)||!lut{let zero=[0f32;16];return xyz_y(p,to,pcs,zero.get(..p.channels()).ok_or_else(||invalid("black point source channels"))?,b)}
 let second=if intent==Intent::Saturation{Intent::Saturation}else{Intent::RelativeColorimetric};match p.selected_side_controlled(false,second,b){Ok((from,fpcs))=>{let mut st=b.reserve(add(from.len(),usize::from(fpcs==Pcs::Xyz))?)?;if fpcs==Pcs::Xyz{st.push(Stage::LabToXyz);}st.extend(from);let pipe=Pipeline::new(3,st);let mut dev=[0f32;16];pipe.eval_controlled(&[0.;3],dev.get_mut(..pipe.outputs).ok_or_else(||invalid("black point target channels"))?,b)?;xyz_y(p,to,pcs,dev.get(..p.channels()).ok_or_else(||invalid("black point target layout"))?,b)},Err(Error::Cms(_))=>{let ones=[1f32;16];xyz_y(p,to,pcs,ones.get(..p.channels()).ok_or_else(||invalid("black point source layout"))?,b)},Err(e)=>Err(e)}
}
fn connection(src:&Profile,dst:&Profile,opts:TransformOptions,b:&mut Budget<'_>)->Result<Option<Affine3>>{
 b.poll()?;if opts.intent==Intent::AbsoluteColorimetric{let(ws,wd)=(src.media_white(),dst.media_white());let usable=|w:&[f64;3]|w.iter().all(|v|v.is_finite()&&*v>0.);if !usable(&ws)||!usable(&wd){return Err(CmsError::Invalid(format!("absolute colorimetric needs a positive media white point (source {ws:?}, destination {wd:?})")).into())}let s=[ws[0]/wd[0],ws[1]/wd[1],ws[2]/wd[2]];return Ok(if s.iter().all(|v|(v-1.).abs()<1e-6){None}else{Some((s,[0.;3]))})}
 if !opts.bpc{return Ok(None)}let(bs,bd)=(black(src,opts.intent,b)?,black(dst,opts.intent,b)?);if (bs-bd).abs()<1e-6{return Ok(None)}let mut scale=[1.;3];let mut offset=[0.;3];for ((a,o),w)in scale.iter_mut().zip(&mut offset).zip(math::D50){let(s,d)=(bs*w,bd*w);*a=(w-d)/(w-s);*o=w*(1.-*a);}Ok(Some((scale,offset)))
}
fn link(src:&Profile,dst:&Profile,opts:TransformOptions,b:&mut Budget<'_>)->Result<Vec<Stage>>{
 profile_classes(src,dst,b)?;let(s,spcs)=src.selected_side_controlled(true,opts.intent,b)?;let(d,dpcs)=dst.selected_side_controlled(false,opts.intent,b)?;let adjust=connection(src,dst,opts,b)?;let mut out=b.reserve(add(add(s.len(),d.len())?,3)?)?;out.extend(s);
 if let Some((scale,offset))=adjust{if spcs==Pcs::Lab{out.push(Stage::LabToXyz);}let matrix=[scale[0],0.,0.,0.,scale[1],0.,0.,0.,scale[2]];out.push(Stage::Matrix{rows:3,cols:3,m:b.copy(&matrix)?,offset:b.copy(&offset)?});if dpcs==Pcs::Lab{out.push(Stage::XyzToLab);}}
 else{match(spcs,dpcs){(Pcs::Lab,Pcs::Xyz)=>out.push(Stage::LabToXyz),(Pcs::Xyz,Pcs::Lab)=>out.push(Stage::XyzToLab),_=>()}}out.extend(d);b.poll()?;Ok(out)
}
fn shaper(cs:&[Curve],matrix:Option<(usize,usize,&[f64],&[f64])>,b:&mut Budget<'_>)->Result<Core>{
 let mut curves8=b.reserve(cs.len())?;let mut curves=b.reserve(cs.len())?;for c in cs{let mut eight=[0f32;256];for(i,v)in eight.iter_mut().enumerate(){b.work(128)?;*v=c.eval64(i as f64/255.) as f32;}curves8.push(eight);let mut table=b.reserve(TABLE_1D+1)?;for i in 0..=TABLE_1D{b.work(128)?;table.push(c.eval64(i as f64/TABLE_1D as f64) as f32);}curves.push(table);}
 let matrix=match matrix{Some((rows,cols,m,off))=>{let mut converted=b.reserve(m.len())?;for &v in m{b.work(1)?;converted.push(v as f32);}let mut offset=b.reserve(off.len())?;for &v in off{b.work(1)?;offset.push(v as f32);}Some((rows,cols,converted,offset))},None=>None};Ok(Core::Shaper{curves8,curves,matrix})
}
fn from_pipeline(mut pipeline:Pipeline,opts:TransformOptions,b:&mut Budget<'_>)->Result<Transform>{
 optimize(&mut pipeline,b)?;let(inputs,outputs)=(pipeline.inputs,pipeline.outputs);let mut head=copy_stages(&pipeline.stages,b)?;let mut post=None;
 if let Some(Stage::InvCurves(cs))=head.last() {if cs.len()==outputs{let mut tables=b.reserve(cs.len())?;for c in cs{let mut table=b.reserve(OUT_TABLE)?;for i in 0..OUT_TABLE{let u=i as f64/(OUT_TABLE-1) as f64;table.push(c.inverse64_controlled(u*u*u*u,b)?.clamp(0.,1.) as f32);}tables.push(table);}post=Some(Post{tables});head.pop();if matches!(head.last(),Some(Stage::Clamp01)){head.pop();}}}
 let head_pipe=Pipeline::new(inputs,copy_stages(&head,b)?);let mid=head_pipe.outputs;
 let core=match head.as_slice(){[]=>shaper(&[],None,b)?,[Stage::Curves(cs)]if cs.len()==inputs=>shaper(cs,None,b)?,[Stage::Matrix{rows,cols,m,offset}]=>shaper(&[],Some((*rows,*cols,m,offset)),b)?,[Stage::Curves(cs),Stage::Matrix{rows,cols,m,offset}]if cs.len()==inputs=>shaper(cs,Some((*rows,*cols,m,offset)),b)?,_ if !head.iter().any(|s|matches!(s,Stage::Clut(_)))=>Core::Exact,_=>{
 let grid:Option<&[usize]>=match inputs{1=>Some(&[TABLE_1D]),2=>Some(&[GRID_3D;2]),3=>Some(&[GRID_3D;3]),4=>Some(&[GRID_4D;4]),_=>None};if let Some(grid)=grid{Core::Grid(Clut::sample_controlled(grid,mid,b,|i,o,b|head_pipe.eval_controlled(i,o,b))?)}else{Core::Exact}
 }};
 let mut grid8=Vec::new();if let Core::Grid(c)=&core{if c.inputs>=2{grid8=b.reserve(c.inputs)?;let mut prefix=1usize;for &n in &c.grid{prefix=mul(prefix,n)?;let stride=c.data.len().checked_div(c.outputs).and_then(|v|v.checked_div(prefix)).and_then(|v|v.checked_mul(c.outputs)).ok_or_else(||invalid("grid8 stride"))?;let mut table=[(0u32,0f32);256];for(v,entry)in table.iter_mut().enumerate(){b.work(1)?;let p=v as f32/255.*(n-1) as f32;let i=(p as usize).min(n-2);*entry=(u32::try_from(mul(i,stride)?).map_err(|_|Error::Limit("grid8 offset"))?,p-i as f32);}grid8.push(table);}}}
 Ok(Transform{inputs,outputs,opts,pipeline,core,post,grid8})
}
fn device_white_fixed(space:ColorSpace)->Option<([f32;4],usize)>{match space{ColorSpace::Rgb=>Some(([1.,1.,1.,0.],3)),ColorSpace::Gray=>Some(([1.,0.,0.,0.],1)),ColorSpace::Cmyk=>Some(([0.;4],4)),ColorSpace::Cmy=>Some(([0.;4],3)),ColorSpace::Lab=>Some(([1.,0.5,0.5,0.],3)),_=>None}}
fn white(t:&mut Transform,src:ColorSpace,dst:ColorSpace,b:&Budget<'_>)->Result<()>{b.poll()?;let(Some((sw,sn)),Some((mut dw,dn)))=(device_white_fixed(src),device_white_fixed(dst))else{return Ok(())};if t.post.is_some(){if !matches!(dst,ColorSpace::Rgb|ColorSpace::Gray){return Ok(())}for v in dw.iter_mut().take(dn){*v=1.;}}let Core::Grid(c)=&mut t.core else{return Ok(())};if c.inputs!=sn||c.outputs!=dn{return Ok(())}let mut off=0usize;let mut stride=c.outputs;for d in (0..c.inputs).rev(){let n=*c.grid.get(d).ok_or_else(||invalid("white grid"))?;let pos=*sw.get(d).ok_or_else(||invalid("white source"))?*(n-1) as f32;if (pos-pos.round()).abs()>1e-6{return Ok(())}off=add(off,mul(pos.round() as usize,stride)?)?;stride=mul(stride,n)?;}c.data.get_mut(off..add(off,c.outputs)?).ok_or_else(||invalid("white node"))?.copy_from_slice(dw.get(..dn).ok_or_else(||invalid("white output"))?);b.poll()?;Ok(())}
pub fn construct(src:&Profile,dst:&Profile,opts:TransformOptions,limits:Limits,cancelled:&dyn Fn()->bool)->Result<Transform>{
 let required=requirements(src,dst,opts,limits,cancelled)?;let mut b=Budget::new(limits,cancelled)?;b.retained(required.retained_bytes)?;let same=model_equal(src,dst,&mut b)?;
 if same&&matches!(src.color_space,ColorSpace::Rgb|ColorSpace::Gray){crate::xcf_color::validate_profile_with_control(src,limits,cancelled)?;return from_pipeline(Pipeline::new(src.channels(),Vec::new()),opts,&mut b)}
 let mut stages=link(src,dst,opts,&mut b)?;if same{stages.clear();}let mut t=from_pipeline(Pipeline::new(src.channels(),stages),opts,&mut b)?;if opts.intent!=Intent::AbsoluteColorimetric{white(&mut t,src.color_space,dst.color_space,&b)?;}b.poll()?;Ok(t)
}

#[cfg(test)]
mod tests {
 use super::*;
 use crate::Builtin;
 use std::cell::Cell;
 #[test]
 fn all_intents_bpc_options_preserve_pipeline_and_fast_words() {
  for(source,destination)in [(Builtin::Srgb,Builtin::DisplayP3),(Builtin::SGray,Builtin::Srgb),(Builtin::CoatedCmyk,Builtin::Srgb),(Builtin::Srgb,Builtin::LabD50)] {
   for intent in [Intent::Perceptual,Intent::RelativeColorimetric,Intent::Saturation,Intent::AbsoluteColorimetric] {for bpc in [false,true] {for precise_float in [false,true] {
    let opts=TransformOptions{intent,bpc,precise_float};let a=source.profile();let z=destination.profile();let old=Transform::with_options(a,z,opts).unwrap();let actual=construct(a,z,opts,Limits::default(),&||false).unwrap();assert_eq!(actual.options(),old.options());assert_eq!(actual.pipeline(),old.pipeline());
    for coordinate in [0.0f32,0.125,0.5,1.0] {let input=vec![coordinate;actual.inputs()];let mut x=vec![0.;actual.outputs()];let mut y=x.clone();old.eval_fast(&input,&mut x);actual.eval_fast(&input,&mut y);assert_eq!(x.iter().map(|v|v.to_bits()).collect::<Vec<_>>(),y.iter().map(|v|v.to_bits()).collect::<Vec<_>>());}
   }}}
  }
 }
 #[test]
 fn original_wide_lut_identity_and_model_bits_are_not_approximated() {
  let old=Builtin::CoatedCmyk.profile();let before=old.to_bytes();let exact=construct(old,old,TransformOptions::default(),Limits::default(),&||false).unwrap();assert!(exact.is_identity());assert_eq!(old.to_bytes(),before);
  let p=Builtin::Srgb.profile();let identical=construct(p,p,TransformOptions::default(),Limits::default(),&||false).unwrap();assert!(identical.is_identity());
 }
 #[test]
 fn limits_and_mid_sampling_cancel_leave_profiles_and_output_unpublished() {
  let src=Builtin::CoatedCmyk.profile();let dst=Builtin::Srgb.profile();let original_src=src.to_bytes();let original_dst=dst.to_bytes();assert!(matches!(construct(src,dst,TransformOptions::default(),Limits{resident_bytes:1,work_units:1},&||false),Err(Error::Limit(_))));
  let calls=Cell::new(0);assert!(matches!(construct(src,dst,TransformOptions::default(),Limits::default(),&||{calls.set(calls.get()+1);calls.get()>500}),Err(Error::Cancelled)));assert_eq!(src.to_bytes(),original_src);assert_eq!(dst.to_bytes(),original_dst);
 }
 #[test]
 fn malformed_pipeline_shape_is_error_not_panic() {
  let mut src=Builtin::Srgb.profile().clone();src.a2b[1]=Some(crate::profile::Lut{kind:crate::profile::LutKind::Ab,inputs:3,outputs:3,stages:vec![Stage::Matrix{rows:3,cols:3,m:vec![1.;8],offset:vec![0.;3]}]});assert!(construct(&src,Builtin::DisplayP3.profile(),TransformOptions{bpc:false,..Default::default()},Limits::default(),&||false).is_err());
 }
}

/// Actual public storage retained by a constructed Transform; shared input
/// profiles are not retained by Transform itself. No private allocator/RSS claim.
pub fn retained(transform:&Transform,limits:Limits,cancelled:&dyn Fn()->bool)->Result<usize>{
 let mut b=Budget::new(limits,cancelled)?;let mut total=std::mem::size_of::<Transform>();
 total=add(total,mul(transform.pipeline.stages.capacity(),std::mem::size_of::<Stage>())?)?;
 for stage in &transform.pipeline.stages{b.work(1)?;total=add(total,stage.controlled_bytes()?)?;b.admit(total,0)?;}
 total=add(total,mul(transform.grid8.capacity(),std::mem::size_of::<[(u32,f32);256]>())?)?;
 match &transform.core{Core::Exact=>(),Core::Grid(c)=>total=add(total,c.controlled_bytes()?)?,Core::Shaper{curves8,curves,matrix}=>{total=add(total,mul(curves8.capacity(),std::mem::size_of::<[f32;256]>())?)?;total=add(total,mul(curves.capacity(),std::mem::size_of::<Vec<f32>>())?)?;for c in curves{b.work(1)?;total=add(total,mul(c.capacity(),4)?)?;b.admit(total,0)?;}if let Some((_,_,m,o))=matrix{total=add(total,mul(add(m.capacity(),o.capacity())?,4)?)?;}}}
 if let Some(post)=&transform.post{total=add(total,mul(post.tables.capacity(),std::mem::size_of::<Vec<f32>>())?)?;for t in &post.tables{b.work(1)?;total=add(total,mul(t.capacity(),4)?)?;b.admit(total,0)?;}}
 b.admit(total,0)?;b.poll()?;Ok(total)
}
