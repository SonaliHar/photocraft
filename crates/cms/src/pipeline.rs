//! Float evaluation pipelines: a list of stages mapping normalised device values to the
//! profile connection space (or back). This is the exact reference path; [`crate::Transform`]
//! builds faster lookup tables from it.

use crate::clut::Clut;
use crate::curve::Curve;
use crate::math;

/// Largest channel count flowing through a pipeline.
pub const MAX: usize = 16;

/// One processing element.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Per-channel curves.
    Curves(Vec<Curve>),
    /// Per-channel inverse curves.
    InvCurves(Vec<Curve>),
    /// `out[r] = Σ m[r][c]·in[c] + offset[r]`, `rows × cols`.
    Matrix {
        rows: usize,
        cols: usize,
        m: Vec<f64>,
        offset: Vec<f64>,
    },
    Clut(Clut),
    /// Float CIE Lab (L 0–100) → XYZ (D50, Y = 1).
    LabToXyz,
    XyzToLab,
    /// ICC v4 / 8-bit Lab encoding (normalised) → float Lab, and back.
    DecodeLabV4,
    EncodeLabV4,
    /// ICC v2 legacy 16-bit Lab encoding (used by `lut16Type`) → float Lab, and back.
    DecodeLabV2,
    EncodeLabV2,
    /// u1Fixed15 XYZ encoding (normalised over 0..=0xFFFF) → XYZ, and back.
    DecodeXyz,
    EncodeXyz,
    /// Clamp every channel to `[0, 1]`.
    Clamp01,
}

const XYZ_SCALE: f32 = 65535.0 / 32768.0;
const V2_L: f32 = 100.0 * 65535.0 / 65280.0;
const V2_AB: f32 = 255.0 * 65535.0 / 65280.0;

impl Stage {
    /// Output channel count given `n` inputs.
    pub fn outputs(&self, n: usize) -> usize {
        match self {
            Stage::Matrix { rows, .. } => *rows,
            Stage::Clut(c) => c.outputs,
            _ => n,
        }
    }

    pub fn matrix3(m: &math::Mat3) -> Stage {
        Stage::Matrix { rows: 3, cols: 3, m: m.iter().flatten().copied().collect(), offset: vec![0.0; 3] }
    }

    #[inline]
    fn run(&self, v: &mut [f32; MAX], n: usize) -> usize {
        match self {
            Stage::Curves(cs) => {
                for (x, c) in v.iter_mut().zip(cs) {
                    *x = c.eval(*x);
                }
                n
            }
            Stage::InvCurves(cs) => {
                for (x, c) in v.iter_mut().zip(cs) {
                    *x = c.eval_inverse(*x);
                }
                n
            }
            Stage::Matrix { rows, cols, m, offset } => {
                let mut o = [0.0f32; MAX];
                for r in 0..*rows {
                    let mut s = offset[r];
                    for c in 0..*cols {
                        s += m[r * cols + c] * v[c] as f64;
                    }
                    o[r] = s as f32;
                }
                v[..*rows].copy_from_slice(&o[..*rows]);
                *rows
            }
            Stage::Clut(c) => {
                let mut o = [0.0f32; MAX];
                let mut inp = [0.0f32; MAX];
                inp[..n].copy_from_slice(&v[..n]);
                c.eval(&inp[..c.inputs], &mut o[..c.outputs]);
                v[..c.outputs].copy_from_slice(&o[..c.outputs]);
                c.outputs
            }
            Stage::LabToXyz => {
                let x = math::lab_to_xyz_f32([v[0], v[1], v[2]]);
                v[..3].copy_from_slice(&x);
                3
            }
            Stage::XyzToLab => {
                let l = math::xyz_to_lab_f32([v[0].max(0.0), v[1].max(0.0), v[2].max(0.0)]);
                v[..3].copy_from_slice(&l);
                3
            }
            Stage::DecodeLabV4 => {
                v[0] *= 100.0;
                v[1] = v[1] * 255.0 - 128.0;
                v[2] = v[2] * 255.0 - 128.0;
                3
            }
            Stage::EncodeLabV4 => {
                v[0] = (v[0] / 100.0).clamp(0.0, 1.0);
                v[1] = ((v[1] + 128.0) / 255.0).clamp(0.0, 1.0);
                v[2] = ((v[2] + 128.0) / 255.0).clamp(0.0, 1.0);
                3
            }
            Stage::DecodeLabV2 => {
                v[0] *= V2_L;
                v[1] = v[1] * V2_AB - 128.0;
                v[2] = v[2] * V2_AB - 128.0;
                3
            }
            Stage::EncodeLabV2 => {
                v[0] = (v[0] / V2_L).clamp(0.0, 1.0);
                v[1] = ((v[1] + 128.0) / V2_AB).clamp(0.0, 1.0);
                v[2] = ((v[2] + 128.0) / V2_AB).clamp(0.0, 1.0);
                3
            }
            Stage::DecodeXyz => {
                for x in &mut v[..3] {
                    *x *= XYZ_SCALE;
                }
                3
            }
            Stage::EncodeXyz => {
                for x in &mut v[..3] {
                    *x = (*x / XYZ_SCALE).clamp(0.0, 1.0);
                }
                3
            }
            Stage::Clamp01 => {
                for x in &mut v[..n] {
                    *x = x.clamp(0.0, 1.0);
                }
                n
            }
        }
    }
}

/// A sequence of stages with fixed input and output channel counts.
#[derive(Clone, Debug, PartialEq)]
pub struct Pipeline {
    pub inputs: usize,
    pub outputs: usize,
    pub stages: Vec<Stage>,
}

impl Pipeline {
    pub fn new(inputs: usize, stages: Vec<Stage>) -> Self {
        let outputs = stages.iter().fold(inputs, |n, s| s.outputs(n));
        Pipeline { inputs, outputs, stages }
    }

    /// Appends stages.
    pub fn extend(&mut self, stages: impl IntoIterator<Item = Stage>) {
        for s in stages {
            self.outputs = s.outputs(self.outputs);
            self.stages.push(s);
        }
    }

    /// Evaluates one pixel: `input` has `inputs` values, `out` receives `outputs` values.
    #[inline]
    pub fn eval(&self, input: &[f32], out: &mut [f32]) {
        let mut v = [0.0f32; MAX];
        v[..self.inputs].copy_from_slice(&input[..self.inputs]);
        let mut n = self.inputs;
        for s in &self.stages {
            n = s.run(&mut v, n);
        }
        out[..self.outputs].copy_from_slice(&v[..self.outputs]);
    }

    /// Checked wide evaluation. Matrix/TRC and analytic PCS stages never pass through f32.
    /// Output is committed only after every stage succeeds and remains finite.
    pub fn try_eval_f64(&self, input: &[f64], out: &mut [f64]) -> Result<(), crate::CmsError> {
        use crate::CmsError;
        let invalid = || CmsError::Invalid("invalid wide ICC pipeline dimensions or nonfinite samples".into());
        if self.inputs == 0
            || self.inputs > MAX
            || self.outputs > MAX
            || input.len() != self.inputs
            || out.len() != self.outputs
            || input.iter().any(|v| !v.is_finite())
        {
            return Err(invalid());
        }
        let mut v = [0.0_f64; MAX];
        v.get_mut(..self.inputs).ok_or_else(invalid)?.copy_from_slice(input);
        let mut n = self.inputs;
        for stage in &self.stages {
            match stage {
                Stage::Curves(curves) | Stage::InvCurves(curves) => {
                    if curves.len() != n {
                        return Err(invalid());
                    }
                    for (sample, curve) in v.get_mut(..n).ok_or_else(invalid)?.iter_mut().zip(curves) {
                        *sample = if matches!(stage, Stage::Curves(_)) { curve.eval64(*sample) } else { curve.eval_inverse64(*sample) };
                    }
                }
                Stage::Matrix { rows, cols, m, offset } => {
                    if *rows == 0 || *rows > MAX || *cols != n || m.len() != rows.checked_mul(*cols).ok_or_else(invalid)? || offset.len() != *rows {
                        return Err(invalid());
                    }
                    let mut values = [0.0_f64; MAX];
                    for (row, target) in values.get_mut(..*rows).ok_or_else(invalid)?.iter_mut().enumerate() {
                        let mut value = *offset.get(row).ok_or_else(invalid)?;
                        for (column, &sample) in v.get(..n).ok_or_else(invalid)?.iter().enumerate() {
                            let i = row.checked_mul(*cols).and_then(|i| i.checked_add(column)).ok_or_else(invalid)?;
                            value += *m.get(i).ok_or_else(invalid)? * sample;
                        }
                        *target = value;
                    }
                    v = values;
                    n = *rows;
                }
                Stage::Clut(clut) => {
                    if n != clut.inputs || clut.outputs > MAX {
                        return Err(invalid());
                    }
                    let mut output = [0.0_f64; MAX];
                    clut.try_eval_f64(v.get(..n).ok_or_else(invalid)?, output.get_mut(..clut.outputs).ok_or_else(invalid)?)?;
                    v = output;
                    n = clut.outputs;
                }
                Stage::Clamp01 => {
                    for sample in v.get_mut(..n).ok_or_else(invalid)? {
                        *sample = sample.clamp(0.0, 1.0);
                    }
                }
                _ => {
                    if n != 3 {
                        return Err(invalid());
                    }
                    let first: [f64; 3] = v.get(..3).ok_or_else(invalid)?.try_into().map_err(|_| invalid())?;
                    let [x, y, z] = first;
                    let xyz_scale = 65535.0_f64 / 32768.0;
                    let v2_l = 100.0_f64 * 65535.0 / 65280.0;
                    let v2_ab = 255.0_f64 * 65535.0 / 65280.0;
                    let result = match stage {
                        Stage::LabToXyz => math::lab_to_xyz(first, math::D50),
                        Stage::XyzToLab => math::xyz_to_lab([x.max(0.0), y.max(0.0), z.max(0.0)], math::D50),
                        Stage::DecodeLabV4 => [x * 100.0, y * 255.0 - 128.0, z * 255.0 - 128.0],
                        Stage::EncodeLabV4 => [(x / 100.0).clamp(0.0, 1.0), ((y + 128.0) / 255.0).clamp(0.0, 1.0), ((z + 128.0) / 255.0).clamp(0.0, 1.0)],
                        Stage::DecodeLabV2 => [x * v2_l, y * v2_ab - 128.0, z * v2_ab - 128.0],
                        Stage::EncodeLabV2 => [(x / v2_l).clamp(0.0, 1.0), ((y + 128.0) / v2_ab).clamp(0.0, 1.0), ((z + 128.0) / v2_ab).clamp(0.0, 1.0)],
                        Stage::DecodeXyz => [x * xyz_scale, y * xyz_scale, z * xyz_scale],
                        Stage::EncodeXyz => [(x / xyz_scale).clamp(0.0, 1.0), (y / xyz_scale).clamp(0.0, 1.0), (z / xyz_scale).clamp(0.0, 1.0)],
                        _ => return Err(invalid()),
                    };
                    v.get_mut(..3).ok_or_else(invalid)?.copy_from_slice(&result);
                }
            }
            if v.get(..n).ok_or_else(invalid)?.iter().any(|value| !value.is_finite()) {
                return Err(invalid());
            }
        }
        if n != self.outputs {
            return Err(invalid());
        }
        out.copy_from_slice(v.get(..n).ok_or_else(invalid)?);
        Ok(())
    }

    /// Removes no-op stages and cancels adjacent inverse pairs (Lab↔XYZ, encode/decode).
    pub fn optimize(&mut self) {
        let mut out: Vec<Stage> = Vec::with_capacity(self.stages.len());
        for s in self.stages.drain(..) {
            let noop = match &s {
                Stage::Curves(c) | Stage::InvCurves(c) => c.iter().all(Curve::is_identity),
                Stage::Matrix { rows: 3, cols: 3, m, offset } => {
                    offset.iter().all(|o| o.abs() < 1e-12) && m.iter().enumerate().all(|(i, v)| (v - if i % 4 == 0 { 1.0 } else { 0.0 }).abs() < 1e-12)
                }
                _ => false,
            };
            if noop {
                continue;
            }
            let cancels = matches!(
                (out.last(), &s),
                (Some(Stage::LabToXyz), Stage::XyzToLab)
                    | (Some(Stage::XyzToLab), Stage::LabToXyz)
                    | (Some(Stage::EncodeLabV4), Stage::DecodeLabV4)
                    | (Some(Stage::EncodeLabV2), Stage::DecodeLabV2)
                    | (Some(Stage::EncodeXyz), Stage::DecodeXyz)
            );
            // Decode→encode of the same encoding is only exact inside the gamut of the encoding,
            // which always holds for data coming out of a table, so those pairs cancel too.
            let cancels_dec = matches!(
                (out.last(), &s),
                (Some(Stage::DecodeLabV4), Stage::EncodeLabV4) | (Some(Stage::DecodeLabV2), Stage::EncodeLabV2) | (Some(Stage::DecodeXyz), Stage::EncodeXyz)
            );
            if cancels || cancels_dec {
                out.pop();
                continue;
            }
            // Merge consecutive 3×3 matrices.
            if let (Some(Stage::Matrix { rows: 3, cols: 3, m: a, offset: oa }), Stage::Matrix { rows: 3, cols: 3, m: b, offset: ob }) = (out.last(), &s) {
                let mut m = vec![0.0; 9];
                let mut off = vec![0.0; 3];
                for r in 0..3 {
                    for c in 0..3 {
                        m[r * 3 + c] = (0..3).map(|k| b[r * 3 + k] * a[k * 3 + c]).sum();
                    }
                    off[r] = (0..3).map(|k| b[r * 3 + k] * oa[k]).sum::<f64>() + ob[r];
                }
                out.pop();
                out.push(Stage::Matrix { rows: 3, cols: 3, m, offset: off });
                continue;
            }
            out.push(s);
        }
        self.stages = out;
    }
}

impl Stage {
 pub(crate) fn controlled_bytes(&self)->crate::control::Result<usize>{
  use crate::control::Error;
  match self{
   Self::Curves(v)|Self::InvCurves(v)=>{if v.len()>MAX{return Err(crate::control::invalid("curve channel count"));}let mut bytes=v.capacity().checked_mul(std::mem::size_of::<crate::Curve>()).ok_or(Error::Limit("curve inventory"))?;for c in v{bytes=bytes.checked_add(c.controlled_bytes()?).ok_or(Error::Limit("curve inventory"))?;}Ok(bytes)},
   Self::Matrix{rows,cols,m,offset}=>{if *rows==0||*cols==0||*rows>MAX||*cols>MAX||m.len()!=rows.checked_mul(*cols).ok_or(Error::Limit("matrix shape"))?||offset.len()!=*rows{return Err(crate::control::invalid("matrix shape"));}m.capacity().checked_add(offset.capacity()).and_then(|n|n.checked_mul(8)).ok_or(Error::Limit("matrix capacity"))},
   Self::Clut(c)=>c.controlled_bytes(),_=>Ok(0)
  }
 }
 pub(crate) fn controlled_work(&self)->crate::control::Result<u64>{
  Ok(match self{Self::Curves(v)=>(v.len() as u64).checked_mul(128).ok_or(crate::control::Error::Limit("curve work"))?,Self::InvCurves(v)=>v.iter().try_fold(0u64,|n,c|n.checked_add(c.controlled_work()).ok_or(crate::control::Error::Limit("curve work")))?,Self::Clut(c)=>c.interpolation_taps()?.checked_mul(c.outputs as u64).ok_or(crate::control::Error::Limit("CLUT work"))?,Self::Matrix{rows,cols,..}=>(*rows as u64)*(*cols as u64)+128,_=>128})
 }
 pub(crate) fn copy_controlled(&self,b:&mut crate::control::Budget<'_>)->crate::control::Result<Self>{
  b.poll()?;self.controlled_bytes()?;Ok(match self{Self::Curves(v)|Self::InvCurves(v)=>{let mut cs=b.reserve(v.len())?;for c in v{cs.push(c.copy_controlled(b)?);}if matches!(self,Self::Curves(_)){Self::Curves(cs)}else{Self::InvCurves(cs)}},Self::Matrix{rows,cols,m,offset}=>Self::Matrix{rows:*rows,cols:*cols,m:b.copy(m)?,offset:b.copy(offset)?},Self::Clut(c)=>Self::Clut(c.copy_controlled(b)?),_=>self.clone()})
 }
}
impl Pipeline {
 pub(crate) fn validate_controlled(&self)->crate::control::Result<()>{
  if self.inputs==0||self.inputs>MAX||self.outputs==0||self.outputs>MAX||self.stages.len()>4096{return Err(crate::control::invalid("pipeline dimensions/stage count"));}
  let mut n=self.inputs;for stage in &self.stages{stage.controlled_bytes()?;match stage{Stage::Curves(cs)|Stage::InvCurves(cs)if cs.len()!=n=>return Err(crate::control::invalid("curve pipeline dimensions")),Stage::Matrix{cols,..}if *cols!=n=>return Err(crate::control::invalid("matrix pipeline dimensions")),Stage::Clut(c)if c.inputs!=n=>return Err(crate::control::invalid("CLUT pipeline dimensions")),Stage::LabToXyz|Stage::XyzToLab|Stage::DecodeLabV4|Stage::EncodeLabV4|Stage::DecodeLabV2|Stage::EncodeLabV2|Stage::DecodeXyz|Stage::EncodeXyz if n!=3=>return Err(crate::control::invalid("PCS pipeline dimensions")),_=>{}}n=stage.outputs(n);}
  if n!=self.outputs{return Err(crate::control::invalid("pipeline output dimensions"));}Ok(())
 }
 pub(crate) fn controlled_work(&self)->crate::control::Result<u64>{self.stages.iter().try_fold(0u64,|n,s|n.checked_add(s.controlled_work()?).ok_or(crate::control::Error::Limit("pipeline work")))}
 pub(crate) fn eval_controlled(&self,input:&[f32],out:&mut[f32],b:&mut crate::control::Budget<'_>)->crate::control::Result<()>{
  use crate::control::{Error,invalid};b.poll()?;self.validate_controlled()?;
  if input.len()!=self.inputs||out.len()!=self.outputs||input.iter().any(|v|!v.is_finite()){return Err(invalid("pipeline input/output"));}
  let mut values=[0f32;MAX];values.get_mut(..self.inputs).ok_or(Error::Limit("pipeline input"))?.copy_from_slice(input);let mut n=self.inputs;
  for stage in &self.stages{b.poll()?;match stage{
   Stage::InvCurves(cs)=>{for (v,c) in values.get_mut(..n).ok_or(Error::Limit("pipeline curves"))?.iter_mut().zip(cs){*v=c.eval_inverse_controlled(*v,b)?;}},
   Stage::Clut(c)=>{let mut next=[0f32;MAX];c.eval_controlled(values.get(..n).ok_or(Error::Limit("pipeline CLUT input"))?,next.get_mut(..c.outputs).ok_or(Error::Limit("pipeline CLUT output"))?,b)?;values.get_mut(..c.outputs).ok_or(Error::Limit("pipeline CLUT output"))?.copy_from_slice(next.get(..c.outputs).ok_or(Error::Limit("pipeline CLUT output"))?);n=c.outputs;},
   _=>{b.work(stage.controlled_work()?)?;n=stage.run(&mut values,n);}
  }if values.get(..n).ok_or(Error::Limit("pipeline result"))?.iter().any(|v|!v.is_finite()){return Err(invalid("nonfinite pipeline output"));}}
  b.poll()?;out.copy_from_slice(values.get(..self.outputs).ok_or(Error::Limit("pipeline output"))?);Ok(())
 }
}
