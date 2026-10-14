//! Multi-dimensional colour lookup tables (ICC CLUTs) with tetrahedral interpolation.
//!
//! Grid layout follows ICC.1: the first input channel varies slowest, output values of one
//! node are stored together. 3-input tables use tetrahedral interpolation (Sakamoto 1980 /
//! Kasson et al. 1995); 4-input tables interpolate linearly along the first input between
//! two tetrahedral evaluations; other input counts use multilinear interpolation.

/// Maximum number of input or output channels of a CLUT.
pub const MAX_CHANNELS: usize = 15;

#[derive(Clone, Debug, PartialEq)]
pub struct Clut {
    pub inputs: usize,
    pub outputs: usize,
    /// Grid points per input dimension (each ≥ 2, or 1 for a degenerate dimension).
    pub grid: Vec<usize>,
    /// Normalised node values, `outputs` per node.
    pub data: Vec<f32>,
    strides: Vec<usize>,
    wide_data: Option<Vec<f64>>,
}

impl Clut {
    /// Maximum node visits per channel for the interpolation used by this table.
    pub fn interpolation_taps(&self) -> Result<u64, crate::CmsError> {
        if !(1..=MAX_CHANNELS).contains(&self.inputs) {
            return Err(wide_invalid());
        }
        Ok(match self.inputs {
            1 => 2,
            3 => 4,
            4 => 8,
            n => 1_u64 << n,
        })
    }

    /// Checked wide interpolation. Node entries retain their ICC precision, but sample
    /// coordinates, weights and accumulators never narrow to f32. Output is atomic.
    /// ICC CLUT coordinates clamp to the defined unit cube; nonfinite coordinates fail.
    pub fn try_eval_f64(&self, input: &[f64], out: &mut [f64]) -> Result<(), crate::CmsError> {
        if !(1..=MAX_CHANNELS).contains(&self.inputs)
            || !(1..=MAX_CHANNELS).contains(&self.outputs)
            || self.grid.len() != self.inputs
            || input.len() != self.inputs
            || out.len() != self.outputs
            || input.iter().any(|v| !v.is_finite())
        {
            return Err(wide_invalid());
        }
        let mut strides = [0_usize; MAX_CHANNELS];
        let mut length = self.outputs;
        for d in (0..self.inputs).rev() {
            let points = *self.grid.get(d).ok_or_else(wide_invalid)?;
            if points == 0 {
                return Err(wide_invalid());
            }
            *strides.get_mut(d).ok_or_else(wide_invalid)? = length;
            length = length.checked_mul(points).ok_or_else(wide_invalid)?;
        }
        if length != self.data.len() {
            return Err(wide_invalid());
        }
        let mut base = 0_usize;
        let mut fractions = [0.0_f64; MAX_CHANNELS];
        let mut steps = [0_usize; MAX_CHANNELS];
        for (d, &value) in input.iter().enumerate() {
            let points = *self.grid.get(d).ok_or_else(wide_invalid)?;
            if points == 1 {
                continue;
            }
            let position = value.clamp(0.0, 1.0) * (points - 1) as f64;
            let index = (position as usize).min(points - 2);
            let stride = *strides.get(d).ok_or_else(wide_invalid)?;
            base = base.checked_add(index.checked_mul(stride).ok_or_else(wide_invalid)?).ok_or_else(wide_invalid)?;
            *fractions.get_mut(d).ok_or_else(wide_invalid)? = position - index as f64;
            *steps.get_mut(d).ok_or_else(wide_invalid)? = stride;
        }
        let mut result = [0.0_f64; MAX_CHANNELS];
        match self.inputs {
            3 => self.tetra64(base, &steps[..3], &fractions[..3], &mut result[..self.outputs])?,
            4 => {
                self.tetra64(base, &steps[1..4], &fractions[1..4], &mut result[..self.outputs])?;
                let mut high = [0.0_f64; MAX_CHANNELS];
                self.tetra64(base.checked_add(steps[0]).ok_or_else(wide_invalid)?, &steps[1..4], &fractions[1..4], &mut high[..self.outputs])?;
                for (low, high) in result[..self.outputs].iter_mut().zip(high) {
                    *low += (high - *low) * fractions[0];
                }
            }
            _ => {
                for corner in 0..(1_usize << self.inputs) {
                    let mut weight = 1.0;
                    let mut offset = base;
                    for d in 0..self.inputs {
                        let fraction = *fractions.get(d).ok_or_else(wide_invalid)?;
                        if corner & (1 << d) != 0 {
                            weight *= fraction;
                            offset = offset.checked_add(*steps.get(d).ok_or_else(wide_invalid)?).ok_or_else(wide_invalid)?;
                        } else {
                            weight *= 1.0 - fraction;
                        }
                    }
                    if weight == 0.0 {
                        continue;
                    }
                    for (channel, value) in result[..self.outputs].iter_mut().enumerate() {
                        *value += weight * self.node64(offset, channel)?;
                    }
                }
            }
        }
        if result[..self.outputs].iter().any(|v| !v.is_finite()) {
            return Err(wide_invalid());
        }
        out.copy_from_slice(&result[..self.outputs]);
        Ok(())
    }

    /// Cooperative wide evaluation with the exact legacy axis/corner order.
    pub fn try_eval_f64_with_control(&self,input:&[f64],out:&mut[f64],limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<()> {
        let mut budget=crate::control::Budget::new(limits,cancelled)?;
        budget.retained(self.controlled_bytes()?)?;
        self.eval_f64_controlled(input,out,&mut budget)
    }
    pub(crate) fn eval_f64_controlled(&self, input: &[f64], out: &mut [f64], budget: &mut crate::control::Budget<'_>) -> crate::control::Result<()> {
        if !(1..=MAX_CHANNELS).contains(&self.inputs)
            || !(1..=MAX_CHANNELS).contains(&self.outputs)
            || self.grid.len() != self.inputs
            || input.len() != self.inputs
            || out.len() != self.outputs
            || input.iter().any(|v| !v.is_finite())
        {
            return Err(wide_invalid().into());
        }
        budget.poll()?;
        let visits=match self.inputs {3=>4u64,4=>8u64,n=>1u64.checked_shl(n as u32).ok_or(crate::control::Error::Limit("CLUT corners"))?};
        budget.work(visits.checked_mul((self.inputs+self.outputs+1) as u64).ok_or(crate::control::Error::Limit("CLUT work"))?)?;
        let mut strides = [0_usize; MAX_CHANNELS];
        let mut length = self.outputs;
        for d in (0..self.inputs).rev() {
            let points = *self.grid.get(d).ok_or_else(wide_invalid)?;
            if points == 0 {
                return Err(wide_invalid().into());
            }
            *strides.get_mut(d).ok_or_else(wide_invalid)? = length;
            length = length.checked_mul(points).ok_or_else(wide_invalid)?;
        }
        if length != self.data.len() {
            return Err(wide_invalid().into());
        }
        let mut base = 0_usize;
        let mut fractions = [0.0_f64; MAX_CHANNELS];
        let mut steps = [0_usize; MAX_CHANNELS];
        for (d, &value) in input.iter().enumerate() {
            let points = *self.grid.get(d).ok_or_else(wide_invalid)?;
            if points == 1 {
                continue;
            }
            let position = value.clamp(0.0, 1.0) * (points - 1) as f64;
            let index = (position as usize).min(points - 2);
            let stride = *strides.get(d).ok_or_else(wide_invalid)?;
            base = base.checked_add(index.checked_mul(stride).ok_or_else(wide_invalid)?).ok_or_else(wide_invalid)?;
            *fractions.get_mut(d).ok_or_else(wide_invalid)? = position - index as f64;
            *steps.get_mut(d).ok_or_else(wide_invalid)? = stride;
        }
        let mut result = [0.0_f64; MAX_CHANNELS];
        match self.inputs {
            3 => self.tetra64(base, &steps[..3], &fractions[..3], result.get_mut(..self.outputs).ok_or_else(wide_invalid)?)?,
            4 => {
                self.tetra64(base, &steps[1..4], &fractions[1..4], result.get_mut(..self.outputs).ok_or_else(wide_invalid)?)?;
                let mut high = [0.0_f64; MAX_CHANNELS];
                self.tetra64(base.checked_add(steps[0]).ok_or_else(wide_invalid)?, &steps[1..4], &fractions[1..4], high.get_mut(..self.outputs).ok_or_else(wide_invalid)?)?;
                for (low, high) in result.get_mut(..self.outputs).ok_or_else(wide_invalid)?.iter_mut().zip(high) {
                    *low += (high - *low) * fractions[0];
                }
            }
            _ => {
                for corner in 0..(1_usize << self.inputs) {
                    if corner.is_multiple_of(64) { budget.poll()?; }
                    let mut weight = 1.0;
                    let mut offset = base;
                    for d in 0..self.inputs {
                        let fraction = *fractions.get(d).ok_or_else(wide_invalid)?;
                        if corner & (1 << d) != 0 {
                            weight *= fraction;
                            offset = offset.checked_add(*steps.get(d).ok_or_else(wide_invalid)?).ok_or_else(wide_invalid)?;
                        } else {
                            weight *= 1.0 - fraction;
                        }
                    }
                    if weight == 0.0 {
                        continue;
                    }
                    for (channel, value) in result.get_mut(..self.outputs).ok_or_else(wide_invalid)?.iter_mut().enumerate() {
                        *value += weight * self.node64(offset, channel)?;
                    }
                }
            }
        }
        if result.get(..self.outputs).ok_or_else(wide_invalid)?.iter().any(|v| !v.is_finite()) {
            return Err(wide_invalid().into());
        }
        budget.poll()?;
        out.copy_from_slice(result.get(..self.outputs).ok_or_else(wide_invalid)?);
        Ok(())
    }

    fn node64(&self, offset: usize, channel: usize) -> Result<f64, crate::CmsError> {
        let index = offset.checked_add(channel).ok_or_else(wide_invalid)?;
        let preview = *self.data.get(index).ok_or_else(wide_invalid)?;
        let exact = self.wide_data.as_ref().and_then(|data| data.get(index)).copied();
        // Public data remains editable for existing clients. A changed preview entry overrides
        // its original exact value rather than silently retaining a stale cached node.
        let value = exact.filter(|value| (*value as f32).to_bits() == preview.to_bits()).unwrap_or(f64::from(preview));
        if !value.is_finite() {
            return Err(wide_invalid());
        }
        Ok(value)
    }

    fn tetra64(&self, base: usize, steps: &[usize], fractions: &[f64], out: &mut [f64]) -> Result<(), crate::CmsError> {
        let mut order = [0_usize, 1, 2];
        // Three-element insertion sort selects the same six tetrahedra as the reference path.
        for i in 1..3 {
            let mut j = i;
            while j > 0 && fractions.get(order[j]).ok_or_else(wide_invalid)? > fractions.get(order[j - 1]).ok_or_else(wide_invalid)? {
                order.swap(j, j - 1);
                j -= 1;
            }
        }
        for (channel, value) in out.iter_mut().enumerate() {
            let mut offset = base;
            let mut previous = self.node64(base, channel)?;
            *value = previous;
            for axis in order {
                offset = offset.checked_add(*steps.get(axis).ok_or_else(wide_invalid)?).ok_or_else(wide_invalid)?;
                let next = self.node64(offset, channel)?;
                *value += (next - previous) * *fractions.get(axis).ok_or_else(wide_invalid)?;
                previous = next;
            }
        }
        Ok(())
    }

    /// Builds a table from node data. Panics if `data` has the wrong length.
    pub fn new(grid: Vec<usize>, outputs: usize, data: Vec<f32>) -> Self {
        let inputs = grid.len();
        assert!((1..=MAX_CHANNELS).contains(&inputs) && (1..=MAX_CHANNELS).contains(&outputs), "bad CLUT dimensions");
        let mut strides = vec![0usize; inputs];
        let mut s = outputs;
        for i in (0..inputs).rev() {
            strides[i] = s;
            s *= grid[i].max(1);
        }
        assert_eq!(data.len(), s, "CLUT data length");
        Clut { inputs, outputs, grid, data, strides, wide_data: None }
    }

    /// Checked construction preserving f64 node values (including exact decoded ICC u16/u8).
    pub fn try_new_f64(grid: Vec<usize>, outputs: usize, values: Vec<f64>) -> Result<Self, crate::CmsError> {
        let inputs = grid.len();
        if !(1..=MAX_CHANNELS).contains(&inputs) || !(1..=MAX_CHANNELS).contains(&outputs) || values.iter().any(|v| !v.is_finite()) {
            return Err(wide_invalid());
        }
        let mut length = outputs;
        let mut strides = vec![0; inputs];
        for d in (0..inputs).rev() {
            let points = *grid.get(d).ok_or_else(wide_invalid)?;
            if points == 0 {
                return Err(wide_invalid());
            }
            *strides.get_mut(d).ok_or_else(wide_invalid)? = length;
            length = length.checked_mul(points).ok_or_else(wide_invalid)?;
        }
        if values.len() != length {
            return Err(wide_invalid());
        }
        let mut data = Vec::new();
        data.try_reserve_exact(values.len()).map_err(|_| crate::CmsError::Invalid("wide ICC CLUT preview allocation denied".into()))?;
        data.extend(values.iter().map(|v| *v as f32));
        Ok(Self { inputs, outputs, grid, data, strides, wide_data: Some(values) })
    }

    /// Number of nodes.
    pub fn nodes(&self) -> usize {
        self.data.len() / self.outputs
    }

    /// Builds a table by sampling `f` at every node (inputs normalised to `[0, 1]`).
    pub fn sample(grid: Vec<usize>, outputs: usize, mut f: impl FnMut(&[f32], &mut [f32])) -> Self {
        let inputs = grid.len();
        let total: usize = grid.iter().product();
        let mut data = vec![0.0f32; total * outputs];
        let mut idx = vec![0usize; inputs];
        let mut inp = vec![0.0f32; inputs];
        for node in 0..total {
            let mut rem = node;
            for d in (0..inputs).rev() {
                idx[d] = rem % grid[d];
                rem /= grid[d];
            }
            for d in 0..inputs {
                inp[d] = if grid[d] > 1 { idx[d] as f32 / (grid[d] - 1) as f32 } else { 0.0 };
            }
            f(&inp, &mut data[node * outputs..(node + 1) * outputs]);
        }
        Clut::new(grid, outputs, data)
    }

    #[inline]
    fn locate(&self, d: usize, v: f32) -> (usize, f32, usize) {
        let n = self.grid[d];
        if n < 2 {
            return (0, 0.0, 0);
        }
        let p = v.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (p as usize).min(n - 2);
        (i * self.strides[d], p - i as f32, self.strides[d])
    }

    /// Interpolates at `input` (`inputs` values) into `out` (`outputs` values).
    #[inline]
    pub fn eval(&self, input: &[f32], out: &mut [f32]) {
        match self.inputs {
            1 => self.eval1(input[0], out),
            3 => {
                let (b0, r0, s0) = self.locate(0, input[0]);
                let (b1, r1, s1) = self.locate(1, input[1]);
                let (b2, r2, s2) = self.locate(2, input[2]);
                self.tetra(b0 + b1 + b2, [s0, s1, s2], [r0, r1, r2], out);
            }
            4 => {
                let (b0, r0, s0) = self.locate(0, input[0]);
                let (b1, r1, s1) = self.locate(1, input[1]);
                let (b2, r2, s2) = self.locate(2, input[2]);
                let (b3, r3, s3) = self.locate(3, input[3]);
                let base = b0 + b1 + b2 + b3;
                let mut lo = [0.0f32; MAX_CHANNELS];
                self.tetra(base, [s1, s2, s3], [r1, r2, r3], &mut lo[..self.outputs]);
                if r0 > 0.0 && s0 > 0 {
                    let mut hi = [0.0f32; MAX_CHANNELS];
                    self.tetra(base + s0, [s1, s2, s3], [r1, r2, r3], &mut hi[..self.outputs]);
                    for (o, (l, h)) in out.iter_mut().zip(lo.iter().zip(&hi)) {
                        *o = l + (h - l) * r0;
                    }
                } else {
                    out[..self.outputs].copy_from_slice(&lo[..self.outputs]);
                }
            }
            _ => self.eval_multilinear(input, out),
        }
    }

    fn eval1(&self, v: f32, out: &mut [f32]) {
        let (b, r, s) = self.locate(0, v);
        for (k, o) in out.iter_mut().enumerate().take(self.outputs) {
            let a = self.data[b + k];
            let c = if s > 0 { self.data[b + s + k] } else { a };
            *o = a + (c - a) * r;
        }
    }

    /// Tetrahedral interpolation inside one cube of three dimensions.
    #[inline]
    fn tetra(&self, base: usize, s: [usize; 3], r: [f32; 3], out: &mut [f32]) {
        let d = &self.data;
        let [rx, ry, rz] = r;
        let [sx, sy, sz] = s;
        let c000 = base;
        let c100 = base + sx;
        let c010 = base + sy;
        let c001 = base + sz;
        let c110 = base + sx + sy;
        let c101 = base + sx + sz;
        let c011 = base + sy + sz;
        let c111 = base + sx + sy + sz;
        // Pick the tetrahedron containing (rx, ry, rz) and the path through the cube corners.
        let (p1, p2, p3, w1, w2, w3) = if rx >= ry {
            if ry >= rz {
                (c100, c110, c111, rx, ry, rz)
            } else if rx >= rz {
                (c100, c101, c111, rx, rz, ry)
            } else {
                (c001, c101, c111, rz, rx, ry)
            }
        } else if rx >= rz {
            (c010, c110, c111, ry, rx, rz)
        } else if ry >= rz {
            (c010, c011, c111, ry, rz, rx)
        } else {
            (c001, c011, c111, rz, ry, rx)
        };
        for (k, o) in out.iter_mut().enumerate().take(self.outputs) {
            let v0 = d[c000 + k];
            let v1 = d[p1 + k];
            let v2 = d[p2 + k];
            let v3 = d[p3 + k];
            *o = v0 + (v1 - v0) * w1 + (v2 - v1) * w2 + (v3 - v2) * w3;
        }
    }

    /// Trilinear interpolation (3 inputs), kept as a reference for accuracy comparisons.
    pub fn eval_trilinear(&self, input: &[f32], out: &mut [f32]) {
        assert_eq!(self.inputs, 3);
        self.eval_multilinear(input, out);
    }

    /// Multilinear interpolation over all 2^n cube corners.
    pub fn eval_multilinear(&self, input: &[f32], out: &mut [f32]) {
        let n = self.inputs;
        let mut base = 0usize;
        let mut r = [0.0f32; MAX_CHANNELS];
        let mut s = [0usize; MAX_CHANNELS];
        for d in 0..n {
            let (b, rr, ss) = self.locate(d, input[d]);
            base += b;
            r[d] = rr;
            s[d] = ss;
        }
        let o = self.outputs;
        out[..o].iter_mut().for_each(|v| *v = 0.0);
        for corner in 0..(1usize << n) {
            let mut w = 1.0f32;
            let mut off = base;
            for d in 0..n {
                if corner & (1 << d) != 0 {
                    w *= r[d];
                    off += s[d];
                } else {
                    w *= 1.0 - r[d];
                }
            }
            if w == 0.0 {
                continue;
            }
            for (k, v) in out[..o].iter_mut().enumerate() {
                *v += w * self.data[off + k];
            }
        }
    }
}

fn wide_invalid() -> crate::CmsError {
    crate::CmsError::Invalid("invalid wide ICC CLUT layout, coordinates or node values".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f3(p: &[f32], o: &mut [f32]) {
        o[0] = p[0] * 0.5 + p[1] * 0.3 + p[2] * 0.2;
        o[1] = p[0] * p[1];
        o[2] = (p[2] * 3.0).sin() * 0.5 + 0.5;
    }

    #[test]
    fn exact_on_nodes_and_linear_functions() {
        let c = Clut::sample(vec![5, 5, 5], 3, f3);
        let mut o = [0.0; 3];
        c.eval(&[0.25, 0.5, 0.75], &mut o);
        let mut w = [0.0; 3];
        f3(&[0.25, 0.5, 0.75], &mut w);
        for i in 0..3 {
            assert!((o[i] - w[i]).abs() < 1e-6);
        }
        // Linear function is reproduced exactly anywhere.
        c.eval(&[0.13, 0.77, 0.41], &mut o);
        assert!((o[0] - (0.13 * 0.5 + 0.77 * 0.3 + 0.41 * 0.2)).abs() < 1e-6);
    }

    #[test]
    fn four_d_and_multilinear_agree_on_linear() {
        let f = |p: &[f32], o: &mut [f32]| o[0] = 0.1 * p[0] + 0.2 * p[1] + 0.3 * p[2] + 0.4 * p[3];
        let c = Clut::sample(vec![3, 4, 5, 6], 1, f);
        let mut a = [0.0];
        let mut b = [0.0];
        for p in [[0.1f32, 0.9, 0.3, 0.7], [1.0, 0.0, 0.5, 0.5], [0.33, 0.66, 0.99, 0.01]] {
            c.eval(&p, &mut a);
            c.eval_multilinear(&p, &mut b);
            let want = 0.1 * p[0] + 0.2 * p[1] + 0.3 * p[2] + 0.4 * p[3];
            assert!((a[0] - want).abs() < 1e-5 && (b[0] - want).abs() < 1e-5);
        }
    }

    #[test]
    fn one_and_two_d() {
        let c = Clut::sample(vec![3], 2, |p, o| {
            o[0] = p[0];
            o[1] = 1.0 - p[0];
        });
        let mut o = [0.0; 2];
        c.eval(&[0.25], &mut o);
        assert!((o[0] - 0.25).abs() < 1e-6 && (o[1] - 0.75).abs() < 1e-6);
        let c = Clut::sample(vec![2, 2], 1, |p, o| o[0] = p[0] + p[1]);
        c.eval(&[0.5, 0.25], &mut o);
        assert!((o[0] - 0.75).abs() < 1e-6);
    }
}

impl Clut {
 pub(crate) fn controlled_bytes(&self)->crate::control::Result<usize>{
  use crate::control::{Error,invalid};
  if !(1..=MAX_CHANNELS).contains(&self.inputs)||!(1..=MAX_CHANNELS).contains(&self.outputs)||self.grid.len()!=self.inputs||self.strides.len()!=self.inputs{return Err(invalid("CLUT shape"));}
  let mut length=self.outputs;
  for axis in (0..self.inputs).rev(){if *self.strides.get(axis).ok_or(Error::Limit("CLUT stride"))?!=length{return Err(invalid("CLUT strides"));}let n=*self.grid.get(axis).ok_or(Error::Limit("CLUT grid"))?;if n==0{return Err(invalid("zero CLUT grid"));}length=length.checked_mul(n).ok_or(Error::Limit("CLUT product"))?;}
  if self.data.len()!=length||self.wide_data.as_ref().is_some_and(|v|v.len()!=length){return Err(invalid("CLUT node length"));}
  self.data.capacity().checked_mul(4).and_then(|n|n.checked_add(self.wide_data.as_ref().map_or(0,|v|v.capacity().checked_mul(8).unwrap_or(usize::MAX)))).and_then(|n|n.checked_add(self.grid.capacity().checked_mul(std::mem::size_of::<usize>())?)).and_then(|n|n.checked_add(self.strides.capacity().checked_mul(std::mem::size_of::<usize>())?)).ok_or(Error::Limit("CLUT capacity"))
 }
 pub(crate) fn copy_controlled(&self,b:&mut crate::control::Budget<'_>)->crate::control::Result<Self>{
  self.controlled_bytes()?;Ok(Self{inputs:self.inputs,outputs:self.outputs,grid:b.copy(&self.grid)?,data:b.copy(&self.data)?,strides:b.copy(&self.strides)?,wide_data:self.wide_data.as_ref().map(|v|b.copy(v)).transpose()?})
 }
 pub(crate) fn eval_controlled(&self,input:&[f32],out:&mut[f32],b:&mut crate::control::Budget<'_>)->crate::control::Result<()>{
  use crate::control::{Error,invalid};self.controlled_bytes()?;
  if input.len()!=self.inputs||out.len()!=self.outputs||input.iter().any(|v|!v.is_finite()){return Err(invalid("CLUT input/output"));}
  b.work(self.interpolation_taps()?.checked_mul(self.outputs as u64).ok_or(Error::Limit("CLUT evaluation work"))?)?;
  let mut values=[0f32;MAX_CHANNELS];let target=values.get_mut(..self.outputs).ok_or(Error::Limit("CLUT outputs"))?;
  if matches!(self.inputs,1|3|4){self.eval(input,target);}else{
   let n=self.inputs;let mut base=0usize;let mut fractions=[0f32;MAX_CHANNELS];let mut steps=[0usize;MAX_CHANNELS];
   for (axis,&value) in input.iter().enumerate(){let (offset,fraction,step)=self.locate(axis,value);base=base.checked_add(offset).ok_or(Error::Limit("CLUT offset"))?;*fractions.get_mut(axis).ok_or(Error::Limit("CLUT fraction"))?=fraction;*steps.get_mut(axis).ok_or(Error::Limit("CLUT step"))?=step;}
   for corner in 0..(1usize<<n){if corner%64==0{b.poll()?;}let mut weight=1f32;let mut offset=base;
    for axis in 0..n{let fraction=*fractions.get(axis).ok_or(Error::Limit("CLUT fraction"))?;if corner&(1<<axis)!=0{weight*=fraction;offset=offset.checked_add(*steps.get(axis).ok_or(Error::Limit("CLUT step"))?).ok_or(Error::Limit("CLUT offset"))?;}else{weight*=1.0-fraction;}}
    if weight==0.0{continue;}for (channel,value) in target.iter_mut().enumerate(){*value+=weight * *self.data.get(offset.checked_add(channel).ok_or(Error::Limit("CLUT offset"))?).ok_or(Error::Limit("CLUT node"))?;}
   }
  }
  b.poll()?;out.copy_from_slice(target);Ok(())
 }
 pub(crate) fn sample_controlled(grid:&[usize],outputs:usize,b:&mut crate::control::Budget<'_>,mut f:impl FnMut(&[f32],&mut[f32],&mut crate::control::Budget<'_>)->crate::control::Result<()>)->crate::control::Result<Self>{
  use crate::control::{Error,invalid};let inputs=grid.len();if !(1..=MAX_CHANNELS).contains(&inputs)||!(1..=MAX_CHANNELS).contains(&outputs)||grid.contains(&0){return Err(invalid("CLUT sample dimensions"));}
  let total=grid.iter().try_fold(1usize,|n,&v|n.checked_mul(v).ok_or(Error::Limit("CLUT sample product")))?;
  let length=total.checked_mul(outputs).ok_or(Error::Limit("CLUT sample length"))?;
  let bytes=length.checked_mul(4).and_then(|n|n.checked_add(inputs.checked_mul(2*std::mem::size_of::<usize>()+4)?)).ok_or(Error::Limit("CLUT sample storage"))?;
  b.admit(bytes,total as u64)?;let mut data=b.reserve(length)?;let grid=b.copy(grid)?;let mut strides=b.reserve(inputs)?;strides.resize(inputs,0);
  let mut stride=outputs;for axis in (0..inputs).rev(){*strides.get_mut(axis).ok_or(Error::Limit("CLUT stride"))?=stride;stride=stride.checked_mul(*grid.get(axis).ok_or(Error::Limit("CLUT grid"))?).ok_or(Error::Limit("CLUT stride"))?;}
  let mut input=[0f32;MAX_CHANNELS];let mut output=[0f32;MAX_CHANNELS];
  for node in 0..total{b.work(1)?;let mut remainder=node;for axis in (0..inputs).rev(){let n=*grid.get(axis).ok_or(Error::Limit("CLUT grid"))?;let i=remainder%n;remainder/=n;*input.get_mut(axis).ok_or(Error::Limit("CLUT sample input"))?=if n>1{i as f32/(n-1) as f32}else{0.0};}
   let out=output.get_mut(..outputs).ok_or(Error::Limit("CLUT sample output"))?;out.fill(0.0);f(input.get(..inputs).ok_or(Error::Limit("CLUT sample input"))?,out,b)?;b.poll()?;if out.iter().any(|v|!v.is_finite()){return Err(invalid("nonfinite sampled CLUT output"));}data.extend_from_slice(out);
  }Ok(Self{inputs,outputs,grid,data,strides,wide_data:None})
 }
 /// Controlled sampling retains the original node-coordinate and declaration order.
 /// The callback's own work/allocations remain caller-managed.
 pub fn try_sample_with_control(grid:&[usize],outputs:usize,limits:crate::control::Limits,cancelled:&dyn Fn()->bool,mut f:impl FnMut(&[f32],&mut[f32])->crate::control::Result<()>)->crate::control::Result<Self>{
  let mut b=crate::control::Budget::new(limits,cancelled)?;Self::sample_controlled(grid,outputs,&mut b,|input,out,_|f(input,out))
 }
}

#[cfg(test)]
mod controlled_tests {
 use super::*;
 #[test]
 fn controlled_sample_preserves_coordinates_and_node_words(){
  let old=Clut::sample(vec![5,3,4],1,|p,o|o[0]=p[0]*0.5+p[1]*0.25+p[2]*0.125);
  let new=Clut::try_sample_with_control(&[5,3,4],1,crate::control::Limits::default(),&||false,|p,o|{o[0]=p[0]*0.5+p[1]*0.25+p[2]*0.125;Ok(())}).unwrap();assert_eq!(new,old);
 }
 #[test]
 fn controlled_sample_checks_overflow_before_callback_and_cancels_atomically(){
  let called=std::cell::Cell::new(false);assert!(Clut::try_sample_with_control(&[usize::MAX,usize::MAX],1,crate::control::Limits::default(),&||false,|_,_|{called.set(true);Ok(())}).is_err());assert!(!called.get());
  assert!(matches!(Clut::try_sample_with_control(&[256],1,crate::control::Limits::default(),&||true,|_,_|Ok(())),Err(crate::control::Error::Cancelled)));
 }
}

impl Clut {
 pub(crate) fn from_f64_controlled(grid:Vec<usize>,outputs:usize,values:Vec<f64>,b:&mut crate::control::Budget<'_>)->crate::control::Result<Self>{
  use crate::control::{Error,invalid};let inputs=grid.len();if !(1..=MAX_CHANNELS).contains(&inputs)||!(1..=MAX_CHANNELS).contains(&outputs){return Err(invalid("wide CLUT dimensions"));}
  let mut length=outputs;for &n in grid.iter().rev(){if n==0{return Err(invalid("zero CLUT grid"));}length=length.checked_mul(n).ok_or(Error::Limit("wide CLUT product"))?;}
  if values.len()!=length{return Err(invalid("wide CLUT node length"));}
  b.admit(length.checked_mul(4).and_then(|n|n.checked_add(inputs.checked_mul(std::mem::size_of::<usize>())?)).ok_or(Error::Limit("wide CLUT shadow"))?,length as u64)?;
  let mut strides=b.reserve(inputs)?;strides.resize(inputs,0);let mut stride=outputs;for (axis,&n) in grid.iter().enumerate().rev(){*strides.get_mut(axis).ok_or(Error::Limit("wide CLUT stride"))?=stride;stride=stride.checked_mul(n).ok_or(Error::Limit("wide CLUT stride"))?;}
  let mut data=b.reserve(length)?;for chunk in values.chunks(1024){b.work(chunk.len() as u64)?;for &value in chunk{if !value.is_finite(){return Err(invalid("nonfinite wide CLUT node"));}data.push(value as f32);}}
  Ok(Self{inputs,outputs,grid,data,strides,wide_data:Some(values)})
 }
}
