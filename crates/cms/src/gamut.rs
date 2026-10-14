//! Gamut checks against a proof (output) profile: Photoshop's View › Gamut Warning.

use crate::clut::Clut;
use crate::pipeline::{Pipeline, Stage};
use crate::profile::Profile;
use crate::transform::link_stages;
use crate::{CmsError, Intent};

/// Default ΔE76 threshold above which a colour counts as out of gamut.
pub const DEFAULT_THRESHOLD: f32 = 4.0;

/// Precomputed out-of-gamut test for colours of `src` against `proof`.
///
/// A colour is out of gamut when sending it to the proof device (relative colorimetric,
/// clipping) and back to Lab moves it by more than the threshold (ΔE76). The ΔE is tabulated
/// over the source device space (33³ grid for 3 channels, 17⁴ for 4, exact otherwise).
#[derive(Debug)]
pub struct GamutCheck {
    inputs: usize,
    grid: Option<Clut>,
    exact: (Pipeline, Pipeline),
    pub threshold: f32,
}

impl GamutCheck {
    pub fn new(src: &Profile, proof: &Profile, threshold: f32) -> Result<GamutCheck, CmsError> {
        let lab = crate::Builtin::LabD50.profile();
        let direct = Pipeline::new(src.channels(), link_stages(src, lab, Intent::RelativeColorimetric, false)?);
        let mut via = link_stages(src, proof, Intent::RelativeColorimetric, false)?;
        via.push(Stage::Clamp01);
        via.extend(link_stages(proof, lab, Intent::RelativeColorimetric, false)?);
        let via = Pipeline::new(src.channels(), via);
        let de = |p: &[f32], o: &mut [f32]| {
            let (mut a, mut b) = ([0.0f32; 16], [0.0f32; 16]);
            direct.eval(p, &mut a);
            via.eval(p, &mut b);
            let d = |i: usize, s: f32| (a[i] - b[i]) * s;
            o[0] = (d(0, 100.0).powi(2) + d(1, 255.0).powi(2) + d(2, 255.0).powi(2)).sqrt();
        };
        let n = src.channels();
        let grid = match n {
            1 => Some(Clut::sample(vec![256], 1, de)),
            3 => Some(Clut::sample(vec![33; 3], 1, de)),
            4 => Some(Clut::sample(vec![17; 4], 1, de)),
            _ => None,
        };
        Ok(GamutCheck { inputs: n, grid, exact: (direct, via), threshold })
    }

    pub fn inputs(&self) -> usize {
        self.inputs
    }

    /// ΔE76 between the colour and its proof round trip.
    pub fn delta_e(&self, px: &[f32]) -> f32 {
        let mut o = [0.0f32; 1];
        match &self.grid {
            Some(g) => {
                let mut c = [0.0f32; 16];
                for (d, s) in c.iter_mut().zip(&px[..self.inputs]) {
                    *d = s.clamp(0.0, 1.0);
                }
                g.eval(&c[..self.inputs], &mut o);
            }
            None => {
                let (mut a, mut b) = ([0.0f32; 16], [0.0f32; 16]);
                self.exact.0.eval(px, &mut a);
                self.exact.1.eval(px, &mut b);
                o[0] = ((a[0] - b[0]) * 100.0).hypot((a[1] - b[1]) * 255.0).hypot((a[2] - b[2]) * 255.0);
            }
        }
        o[0]
    }

    pub fn out_of_gamut(&self, px: &[f32]) -> bool {
        self.delta_e(px) > self.threshold
    }
}

/// Known public model retention and controlled construction envelope only.
/// This excludes provider-private allocator overhead and process RSS.
#[derive(Clone,Copy,Debug)]
pub struct GamutRequirements { pub retained_bytes:usize, pub peak_resident_bytes:usize, pub work_units:u64, pub evaluation_work_units:u64 }

impl GamutCheck {
 /// Borrowed prospective census before pipeline/table copies. LabD50 provider
 /// initialization remains boundary-only, polled before and after.
 pub fn requirements_with_control(src:&Profile,proof:&Profile,threshold:f32,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<GamutRequirements>{
  use crate::control::{Budget,Error,invalid};
  let mut b=Budget::new(limits,cancelled)?;
  if !threshold.is_finite()||threshold<0.0{return Err(invalid("finite nonnegative gamut threshold required"));}
  let lab=crate::Builtin::LabD50.profile();b.poll()?;
  let source_bytes=src.controlled_retained_bytes(&b)?;let proof_bytes=proof.controlled_retained_bytes(&b)?;let lab_bytes=lab.controlled_retained_bytes(&b)?;
  let retained=source_bytes.checked_add(proof_bytes).and_then(|n|n.checked_add(lab_bytes)).ok_or(Error::Limit("retained proof/source profile model"))?;b.retained(retained)?;
  let si=src.controlled_side_cost(true,&b)?;let po=proof.controlled_side_cost(false,&b)?;let pi=proof.controlled_side_cost(true,&b)?;let lo=lab.controlled_side_cost(false,&b)?;
  let n=src.channels();if n==0||n>crate::pipeline::MAX{return Err(invalid("gamut source channels"));}
  let direct_count=si.2.checked_add(lo.2).and_then(|n|n.checked_add(1)).ok_or(Error::Limit("direct gamut stages"))?;
  let via_count=si.2.checked_add(po.2).and_then(|n|n.checked_add(pi.2)).and_then(|n|n.checked_add(lo.2)).and_then(|n|n.checked_add(3)).ok_or(Error::Limit("roundtrip gamut stages"))?;
  if direct_count>4096||via_count>4096{return Err(Error::Limit("gamut pipeline stage count"));}
  let mut bytes=0usize;let mut per_node=512u64;
  for side in [si,lo,si,po,pi,lo]{bytes=bytes.checked_add(side.0.checked_mul(2).ok_or(Error::Limit("copied gamut pipeline"))?).ok_or(Error::Limit("copied gamut pipeline"))?;per_node=per_node.checked_add(side.1).ok_or(Error::Limit("gamut pipeline work"))?;}
  bytes=bytes.checked_add(via_count.checked_mul(std::mem::size_of::<Stage>()).ok_or(Error::Limit("combined gamut stages"))?).ok_or(Error::Limit("combined gamut stages"))?;
  let grid:&[usize]=match n{1=>&[256],3=>&[33,33,33],4=>&[17,17,17,17],_=>&[]};
  let nodes=if grid.is_empty(){0}else{grid.iter().try_fold(1usize,|n,&v|n.checked_mul(v).ok_or(Error::Limit("gamut grid product")))?};
  let prospective=bytes.checked_add(nodes.checked_mul(4).ok_or(Error::Limit("gamut grid storage"))?).and_then(|n|n.checked_add(4096)).ok_or(Error::Limit("gamut prospective storage"))?;
  let work=(nodes as u64).checked_mul(per_node.checked_add(1).ok_or(Error::Limit("gamut node work"))?).and_then(|n|n.checked_add(bytes as u64)).ok_or(Error::Limit("gamut prospective work"))?;
  b.admit(prospective,work)?;
  Ok(GamutRequirements{retained_bytes:retained,peak_resident_bytes:retained.checked_add(prospective).ok_or(Error::Limit("gamut prospective total"))?,work_units:work,evaluation_work_units:if nodes>0{256}else{per_node.checked_add(128).ok_or(Error::Limit("gamut evaluation work"))?}})
 }
 /// Cooperative, bounded construction with the identical existing relative-
 /// colorimetric round trip, clamp placement, f32 grid and DeltaE arithmetic.
 /// Builtin LabD50 OnceLock initialization/encoding is a provider boundary,
 /// polled before/after; its internal allocator is not controlled here.
 pub fn new_with_control(src:&Profile,proof:&Profile,threshold:f32,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<Self>{
  use crate::control::{Budget,Error,invalid};
  let required=Self::requirements_with_control(src,proof,threshold,limits,cancelled)?;
  let mut b=Budget::new(limits,cancelled)?;b.retained(required.retained_bytes)?;
  let lab=crate::Builtin::LabD50.profile();b.poll()?;let n=src.channels();
  let grid:&[usize]=match n{1=>&[256],3=>&[33,33,33],4=>&[17,17,17,17],_=>&[]};
  let direct=Pipeline::new(n,crate::transform::link_stages_relative_controlled(src,lab,&mut b)?);
  let first=crate::transform::link_stages_relative_controlled(src,proof,&mut b)?;let last=crate::transform::link_stages_relative_controlled(proof,lab,&mut b)?;
  let count=first.len().checked_add(last.len()).and_then(|n|n.checked_add(1)).ok_or(Error::Limit("combined gamut stage length"))?;
  let mut via=b.reserve(count)?;via.extend(first);via.push(Stage::Clamp01);via.extend(last);let via=Pipeline::new(n,via);
  direct.validate_controlled()?;via.validate_controlled()?;
  if direct.outputs!=3||via.outputs!=3{return Err(invalid("gamut pipelines must terminate in Lab"));}
  let table=if grid.is_empty(){None}else{Some(Clut::sample_controlled(grid,1,&mut b,|p,o,b|{
   let(mut a,mut z)=([0f32;3],[0f32;3]);direct.eval_controlled(p,&mut a,b)?;via.eval_controlled(p,&mut z,b)?;
   let d=|i:usize,scale:f32|->crate::control::Result<f32>{Ok((*a.get(i).ok_or(Error::Limit("gamut Lab coordinate"))?-*z.get(i).ok_or(Error::Limit("gamut Lab coordinate"))?)*scale)};
   *o.first_mut().ok_or(Error::Limit("gamut grid output"))?=(d(0,100.0)?.powi(2)+d(1,255.0)?.powi(2)+d(2,255.0)?.powi(2)).sqrt();Ok(())
  })?)};b.poll()?;Ok(Self{inputs:n,grid:table,exact:(direct,via),threshold})
 }
 /// Checked existing f32 gamut policy; no native precision/profile inference.
 pub fn delta_e_with_control(&self,px:&[f32],cancelled:&dyn Fn()->bool)->crate::control::Result<f32>{
  use crate::control::{Budget,Error,invalid};
  if px.len()!=self.inputs||px.iter().any(|v|!v.is_finite()){return Err(invalid("gamut input length/nonfinite sample"));}
  let mut b=Budget::new(crate::control::Limits::default(),cancelled)?;
  let value=if let Some(grid)=&self.grid{let mut c=[0f32;16];for (dst,&v) in c.iter_mut().zip(px){*dst=v.clamp(0.0,1.0);}let mut out=[0f32;1];grid.eval_controlled(c.get(..self.inputs).ok_or(Error::Limit("gamut input channels"))?,&mut out,&mut b)?;out[0]}
  else{let work=self.exact.0.controlled_work()?.checked_add(self.exact.1.controlled_work()?).ok_or(Error::Limit("gamut evaluation work"))?;b.admit(0,work)?;let(mut a,mut z)=([0f32;3],[0f32;3]);self.exact.0.eval_controlled(px,&mut a,&mut b)?;self.exact.1.eval_controlled(px,&mut z,&mut b)?;((a[0]-z[0])*100.0).hypot((a[1]-z[1])*255.0).hypot((a[2]-z[2])*255.0)};
  b.poll()?;if !value.is_finite(){return Err(invalid("nonfinite gamut result"));}Ok(value)
 }
 pub fn out_of_gamut_with_control(&self,px:&[f32],cancelled:&dyn Fn()->bool)->crate::control::Result<bool>{Ok(self.delta_e_with_control(px,cancelled)?>self.threshold)}
}

#[cfg(test)]
mod controlled_tests {
 use super::*;
 #[test]
 fn controlled_gamut_matches_existing_f32_grids_and_evaluation_words(){
  for src in [crate::Builtin::SGray,crate::Builtin::Srgb,crate::Builtin::CoatedCmyk]{
   let source=src.profile();let proof=crate::Builtin::DisplayP3.profile();let old=GamutCheck::new(source,proof,4.0).unwrap();let new=GamutCheck::new_with_control(source,proof,4.0,crate::control::Limits::default(),&||false).unwrap();
   assert_eq!(old.grid,new.grid);assert_eq!(old.exact,new.exact);
   for value in [0.0,0.125,0.5,1.0]{let p=vec![value;source.channels()];assert_eq!(old.delta_e(&p).to_bits(),new.delta_e_with_control(&p,&||false).unwrap().to_bits());}
  }
 }
 #[test]
 fn controlled_construction_and_evaluation_refuse_cancel_shape_and_tiny_budget(){
  let src=crate::Builtin::Srgb.profile();let proof=crate::Builtin::DisplayP3.profile();
  assert!(matches!(GamutCheck::new_with_control(src,proof,4.0,crate::control::Limits::default(),&||true),Err(crate::control::Error::Cancelled)));
  assert!(matches!(GamutCheck::new_with_control(src,proof,4.0,crate::control::Limits{resident_bytes:1,work_units:1},&||false),Err(crate::control::Error::Limit(_))));
  let check=GamutCheck::new_with_control(src,proof,4.0,crate::control::Limits::default(),&||false).unwrap();
  assert!(check.delta_e_with_control(&[0.5],&||false).is_err());assert!(check.delta_e_with_control(&[f32::NAN,0.5,0.5],&||false).is_err());assert!(matches!(check.delta_e_with_control(&[0.5;3],&||true),Err(crate::control::Error::Cancelled)));
 }
 #[test]
 fn construction_cancellation_during_grid_leaves_profiles_immutable(){
  let src=crate::Builtin::Srgb.profile();let proof=crate::Builtin::DisplayP3.profile();let before=(src.clone(),proof.clone());let polls=std::cell::Cell::new(0usize);
  let cancelled=||{let n=polls.get();polls.set(n+1);n>500};
  assert!(matches!(GamutCheck::new_with_control(src,proof,4.0,crate::control::Limits::default(),&cancelled),Err(crate::control::Error::Cancelled)));assert_eq!((src,proof),(&before.0,&before.1));
 }
}
