//! One-dimensional tone curves (ICC `curv` and `para` tag types) and their inverses.

/// A tone reproduction curve.
///
/// Gamma and parametric curves are evaluated analytically, including outside `[0, 1]` (negative
/// inputs mirror, values above 1 extend the formula), so unbounded float transforms keep HDR
/// values. Sampled tables clamp their input to `[0, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Identity,
    /// `y = x^g`.
    Gamma(f64),
    /// ICC `parametricCurveType` function `kind` (0–4) with parameters `[g, a, b, c, d, e, f]`
    /// (unused ones are 0).
    Parametric {
        kind: u16,
        p: [f64; 7],
    },
    /// Samples at evenly spaced inputs over `[0, 1]`, normalised outputs.
    Table(Vec<f32>),
    /// Parsed ICC integer table normalized in f64, without first rounding entries to f32.
    Table64(Vec<f64>),
}

/// The sRGB transfer function (IEC 61966-2.1) as ICC parametric type 3.
pub fn srgb_trc() -> Curve {
    Curve::Parametric { kind: 3, p: [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045, 0.0, 0.0] }
}

impl Curve {
    /// Number of parameters stored for a parametric function type.
    pub fn param_count(kind: u16) -> usize {
        match kind {
            0 => 1,
            1 => 3,
            2 => 4,
            3 => 5,
            _ => 7,
        }
    }

    pub fn is_identity(&self) -> bool {
        match self {
            Curve::Identity => true,
            Curve::Gamma(g) => (*g - 1.0).abs() < 1e-9,
            Curve::Parametric { kind: 0, p } => (p[0] - 1.0).abs() < 1e-9,
            Curve::Table(t) => {
                let n = t.len();
                n < 2 || t.iter().enumerate().all(|(i, v)| (*v as f64 - i as f64 / (n - 1) as f64).abs() < 1.0 / 65535.0)
            }
            Curve::Table64(t) => {
                let n = t.len();
                n < 2 || t.iter().enumerate().all(|(i, v)| (*v - i as f64 / (n - 1) as f64).abs() < 1.0 / 65535.0)
            }
            _ => false,
        }
    }

    /// Evaluates the curve at `x` (f64 reference path).
    pub fn eval64(&self, x: f64) -> f64 {
        match self {
            Curve::Identity => x,
            Curve::Gamma(g) => {
                if *g == 1.0 && x.is_finite() {
                    x
                } else {
                    mirror(x, |v| v.powf(*g))
                }
            }
            Curve::Parametric { kind, p } => para(*kind, p, x),
            Curve::Table(t) => table64(t, x),
            Curve::Table64(t) => table64(t, x),
        }
    }

    #[inline]
    pub fn eval(&self, x: f32) -> f32 {
        match self {
            Curve::Identity => x,
            Curve::Table(t) => table(t, x),
            _ => self.eval64(x as f64) as f32,
        }
    }

    /// Evaluates the inverse curve at `y`.
    pub fn eval_inverse64(&self, y: f64) -> f64 {
        match self {
            Curve::Identity => y,
            Curve::Gamma(g) => {
                if (*g == 1.0 && y.is_finite()) || g.abs() < 1e-12 {
                    y
                } else {
                    mirror(y, |v| v.powf(1.0 / g))
                }
            }
            Curve::Parametric { kind, p } => para_inverse(*kind, p, y).unwrap_or_else(|| numeric_inverse(|x| para(*kind, p, x), y)),
            Curve::Table(t) => table_inverse64(t, y),
            Curve::Table64(t) => table_inverse64(t, y),
        }
    }

    #[inline]
    pub fn eval_inverse(&self, y: f32) -> f32 {
        match self {
            Curve::Identity => y,
            Curve::Table(t) => table_inverse(t, y),
            _ => self.eval_inverse64(y as f64) as f32,
        }
    }

    /// Samples `n` evenly spaced points over `[0, 1]`.
    pub fn sample(&self, n: usize) -> Vec<f32> {
        (0..n).map(|i| self.eval64(i as f64 / (n - 1) as f64) as f32).collect()
    }

    /// `true` when the curve never decreases over `[0, 1]`.
    pub fn is_monotonic(&self) -> bool {
        let s = self.sample(1024);
        s.windows(2).all(|w| w[1] >= w[0] - 1e-6) || s.windows(2).all(|w| w[1] <= w[0] + 1e-6)
    }
}

#[inline]
fn mirror(x: f64, f: impl Fn(f64) -> f64) -> f64 {
    if x.is_sign_negative() { -f(-x) } else { f(x) }
}

fn pw(base: f64, g: f64) -> f64 {
    if base <= 0.0 { 0.0 } else { base.powf(g) }
}

fn para(kind: u16, p: &[f64; 7], x: f64) -> f64 {
    let [g, a, b, c, d, e, f] = *p;
    match kind {
        0 => {
            if g == 1.0 && x.is_finite() {
                x
            } else {
                mirror(x, |v| v.powf(g))
            }
        }
        1 => {
            if a != 0.0 && x >= -b / a {
                pw(a * x + b, g)
            } else {
                0.0
            }
        }
        2 => {
            if a != 0.0 && x >= -b / a {
                pw(a * x + b, g) + c
            } else {
                c
            }
        }
        3 => {
            if x < 0.0 && b == 0.0 && d == 0.0 {
                // Pure gamma written as type 3: mirror negatives.
                -para(kind, p, -x)
            } else if x >= d {
                pw(a * x + b, g)
            } else {
                c * x
            }
        }
        _ => {
            if x >= d {
                pw(a * x + b, g) + e
            } else {
                c * x + f
            }
        }
    }
}

fn para_inverse(kind: u16, p: &[f64; 7], y: f64) -> Option<f64> {
    let [g, a, b, c, d, e, f] = *p;
    if g.abs() < 1e-12 {
        return None;
    }
    let root = |v: f64| if v <= 0.0 { 0.0 } else { v.powf(1.0 / g) };
    Some(match kind {
        0 => {
            if g == 1.0 && y.is_finite() {
                y
            } else {
                mirror(y, |v| v.powf(1.0 / g))
            }
        }
        1 => {
            if a == 0.0 {
                return None;
            }
            if y <= 0.0 { -b / a } else { (root(y) - b) / a }
        }
        2 => {
            if a == 0.0 {
                return None;
            }
            if y <= c { -b / a } else { (root(y - c) - b) / a }
        }
        3 => {
            if a == 0.0 {
                return None;
            }
            if y < 0.0 && b == 0.0 && d == 0.0 {
                return para_inverse(kind, p, -y).map(|v| -v);
            }
            let yd = pw(a * d + b, g);
            if y >= yd {
                (root(y) - b) / a
            } else if c != 0.0 {
                y / c
            } else {
                0.0
            }
        }
        _ => {
            if a == 0.0 {
                return None;
            }
            let yd = pw(a * d + b, g) + e;
            if y >= yd {
                (root(y - e) - b) / a
            } else if c != 0.0 {
                (y - f) / c
            } else {
                0.0
            }
        }
    })
}

/// Bisection inverse of an increasing function on `[0, 1]`.
fn numeric_inverse(f: impl Fn(f64) -> f64, y: f64) -> f64 {
    let (f0, f1) = (f(0.0), f(1.0));
    let increasing = f1 >= f0;
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        let v = f(mid);
        if (v < y) == increasing {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[inline]
fn table(t: &[f32], x: f32) -> f32 {
    let n = t.len();
    match n {
        0 => x,
        1 => t[0],
        _ => {
            let pos = x.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (pos as usize).min(n - 2);
            let f = pos - i as f32;
            t[i] + (t[i + 1] - t[i]) * f
        }
    }
}

// ICC table entries have their encoded precision; the sample and interpolation must not
// inherit the f32 precision of the entry storage when evaluating a wide source.
fn table64<T: Copy + Into<f64>>(t: &[T], x: f64) -> f64 {
    let Some(&first) = t.first() else { return x };
    if t.len() == 1 {
        return first.into();
    }
    if !x.is_finite() {
        return x;
    }
    let pos = x.clamp(0.0, 1.0) * (t.len() - 1) as f64;
    let i = (pos as usize).min(t.len() - 2);
    let (Some(&a), Some(&b)) = (t.get(i), t.get(i + 1)) else { return x };
    a.into() + (b.into() - a.into()) * (pos - i as f64)
}

fn table_inverse64<T: Copy + Into<f64>>(t: &[T], y: f64) -> f64 {
    let (Some(&first), Some(&last)) = (t.first(), t.last()) else { return y };
    if t.len() < 2 || !y.is_finite() {
        return y;
    }
    let (first, last) = (first.into(), last.into());
    let increasing = last >= first;
    let (low, high) = if increasing { (first, last) } else { (last, first) };
    if y <= low || y >= high {
        let start = if y <= low { increasing } else { !increasing };
        let edge = if start { first } else { last };
        let i = if start {
            t.iter().position(|v| ((*v).into() - edge).abs() > 1e-7).map_or(t.len() - 1, |i| i.saturating_sub(1))
        } else {
            t.iter().rposition(|v| ((*v).into() - edge).abs() > 1e-7).map_or(0, |i| i + 1)
        };
        return i as f64 / (t.len() - 1) as f64;
    }
    let (mut lo, mut hi) = (0, t.len() - 1);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let Some(&v) = t.get(mid) else { return y };
        if (v.into() < y) == increasing {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (Some(&a), Some(&b)) = (t.get(lo), t.get(hi)) else { return y };
    let (a, b) = (a.into(), b.into());
    let fraction = if b != a { (y - a) / (b - a) } else { 0.5 };
    (lo as f64 + fraction) / (t.len() - 1) as f64
}

/// Exact inverse of a piecewise-linear table (binary search), for increasing or decreasing tables.
fn table_inverse(t: &[f32], y: f32) -> f32 {
    let n = t.len();
    if n < 2 {
        return y;
    }
    let increasing = t[n - 1] >= t[0];
    let (lo_v, hi_v) = if increasing { (t[0], t[n - 1]) } else { (t[n - 1], t[0]) };
    if y <= lo_v {
        // Flat start: choose the last index still at the lowest value.
        return if increasing { edge_index(t, true) } else { edge_index(t, false) };
    }
    if y >= hi_v {
        return if increasing { edge_index(t, false) } else { edge_index(t, true) };
    }
    let (mut lo, mut hi) = (0usize, n - 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if (t[mid] < y) == increasing {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (a, b) = (t[lo], t[hi]);
    let f = if (b - a).abs() > 1e-12 { (y - a) / (b - a) } else { 0.5 };
    (lo as f32 + f) / (n - 1) as f32
}

/// For a flat run at the table start (`start = true`) the last index of the run, or for a flat
/// run at the end the first index of it, as a normalised input.
fn edge_index(t: &[f32], start: bool) -> f32 {
    let n = t.len();
    if start {
        let v = t[0];
        let i = t.iter().position(|x| (*x - v).abs() > 1e-7).map_or(n - 1, |i| i - 1);
        i as f32 / (n - 1) as f32
    } else {
        let v = t[n - 1];
        let i = t.iter().rposition(|x| (*x - v).abs() > 1e-7).map_or(0, |i| i + 1);
        i as f32 / (n - 1) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_matches_formula() {
        let c = srgb_trc();
        for i in 0..=255 {
            let v = i as f64 / 255.0;
            let want = if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            assert!((c.eval64(v) - want).abs() < 1e-12);
            assert!((c.eval_inverse64(want) - v).abs() < 1e-9);
        }
    }

    #[test]
    fn parametric_inverses() {
        let curves = [
            Curve::Parametric { kind: 0, p: [2.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
            Curve::Parametric { kind: 1, p: [2.0, 0.9, 0.1, 0.0, 0.0, 0.0, 0.0] },
            Curve::Parametric { kind: 2, p: [2.0, 0.9, 0.1, 0.05, 0.0, 0.0, 0.0] },
            srgb_trc(),
            Curve::Parametric { kind: 4, p: [2.4, 0.9, 0.05, 0.08, 0.04, 0.01, 0.002] },
            Curve::Gamma(1.8),
        ];
        for c in &curves {
            for i in 1..100 {
                let x = i as f64 / 100.0;
                let y = c.eval64(x);
                assert!((c.eval_inverse64(y) - x).abs() < 1e-6, "{c:?} at {x}");
            }
        }
    }

    #[test]
    fn table_inverse_exact_on_knots_and_between() {
        let t = Curve::Gamma(2.2).sample(1024);
        let c = Curve::Table(t);
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            let back = c.eval_inverse(c.eval(x));
            assert!((back - x).abs() < 1e-4, "{x} -> {back}");
        }
        let dec = Curve::Table(vec![1.0, 0.5, 0.0]);
        assert!((dec.eval_inverse(0.25) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn hdr_and_negative_extension() {
        let c = srgb_trc();
        assert!(c.eval64(2.0) > 1.0);
        assert!((c.eval_inverse64(c.eval64(2.0)) - 2.0).abs() < 1e-9);
        let g = Curve::Gamma(2.2);
        assert!((g.eval64(-0.5) + 0.5f64.powf(2.2)).abs() < 1e-12);
    }

    #[test]
    fn exact_one_preserves_finite_extended_domain_bits() {
        let curves = [Curve::Gamma(1.0), Curve::Parametric { kind: 0, p: [1.0, 7.0, -3.0, 11.0, 0.5, 9.0, -2.0] }];
        let mut values = vec![
            -f64::MAX,
            -1e300,
            -65504.0,
            -17.25,
            -2.0,
            -1.0,
            -0.125,
            -f64::MIN_POSITIVE,
            -f64::from_bits(1),
            -0.0,
            0.0,
            f64::from_bits(1),
            f64::MIN_POSITIVE,
            0.125,
            1.0,
            2.0,
            17.25,
            65504.0,
            1e300,
            f64::MAX,
        ];
        for sign in [0, 1_u64 << 63] {
            for exponent in [0, 1, 2, 256, 1022, 1023, 1024, 1536, 2045, 2046] {
                for mantissa in [0, 1, 0x4000, 0x8_0000_0000_0000, 0xf_ffff_ffff_ffff] {
                    values.push(f64::from_bits(sign | (exponent << 52) | mantissa));
                }
            }
        }
        for x in values {
            for curve in &curves {
                assert_eq!(curve.eval64(x).to_bits(), x.to_bits());
                assert_eq!(curve.eval_inverse64(x).to_bits(), x.to_bits());
                let y = x as f32;
                if y.is_finite() {
                    assert_eq!(curve.eval(y).to_bits(), y.to_bits());
                    assert_eq!(curve.eval_inverse(y).to_bits(), y.to_bits());
                }
            }
        }
    }

    #[test]
    fn exact_one_keeps_nonfinite_expression_policy() {
        for curve in [Curve::Gamma(1.0), Curve::Parametric { kind: 0, p: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }] {
            for x in [f64::INFINITY, f64::NEG_INFINITY] {
                assert_eq!(curve.eval64(x).to_bits(), mirror(x, |v| v.powf(1.0)).to_bits());
                assert_eq!(curve.eval_inverse64(x).to_bits(), mirror(x, |v| v.powf(1.0)).to_bits());
            }
            for x in [f64::NAN, f64::from_bits(0x7ff8_0000_0000_0042), f64::from_bits(0xfff8_0000_0000_0042)] {
                assert!(curve.eval64(x).is_nan());
                assert!(curve.eval_inverse64(x).is_nan());
            }
        }
    }

    #[test]
    fn near_one_curves_keep_analytic_behavior() {
        for g in [1.0 - 1e-10, 1.0 + 1e-10, 1.8, 2.2] {
            for curve in [Curve::Gamma(g), Curve::Parametric { kind: 0, p: [g, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }] {
                for x in [-2.0_f64, -0.5, -0.0, 0.0, 0.125, 0.5, 1.0, 2.0] {
                    assert_eq!(curve.eval64(x).to_bits(), mirror(x, |v| v.powf(g)).to_bits());
                    assert_eq!(curve.eval_inverse64(x).to_bits(), mirror(x, |v| v.powf(1.0 / g)).to_bits());
                }
            }
        }
        for curve in [Curve::Table(vec![0.0, 1.0]), Curve::Table64(vec![0.0, 1.0])] {
            assert_eq!(curve.eval64(-2.0), 0.0);
            assert_eq!(curve.eval64(2.0), 1.0);
        }
        let threshold = Curve::Parametric { kind: 1, p: [1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0] };
        assert_eq!(threshold.eval64(-2.0), 0.0);
        assert_eq!(threshold.eval_inverse64(-2.0), 0.0);
    }
}

impl Curve {
 pub(crate) fn controlled_bytes(&self)->crate::control::Result<usize>{
  let bytes=match self{Self::Table(v)=>v.capacity().checked_mul(4),Self::Table64(v)=>v.capacity().checked_mul(8),_=>Some(0)};
  bytes.ok_or(crate::control::Error::Limit("curve capacity"))
 }
 pub(crate) fn controlled_work(&self)->u64{match self{Self::Table(v)=>v.len() as u64+64,Self::Table64(v)=>v.len() as u64+64,_=>128}}
 pub(crate) fn copy_controlled(&self,b:&mut crate::control::Budget<'_>)->crate::control::Result<Self>{
  b.poll()?;Ok(match self{Self::Identity=>Self::Identity,Self::Gamma(g)=>Self::Gamma(*g),Self::Parametric{kind,p}=>Self::Parametric{kind:*kind,p:*p},Self::Table(v)=>Self::Table(b.copy(v)?),Self::Table64(v)=>Self::Table64(b.copy(v)?)})
 }
 pub(crate) fn eval_inverse_controlled(&self,y:f32,b:&mut crate::control::Budget<'_>)->crate::control::Result<f32>{
  use crate::control::{Error,Result};
  fn edge<T:Copy+Into<f64>>(t:&[T],start:bool,b:&mut crate::control::Budget<'_>)->Result<usize>{
   let n=t.len();let reference=if start{t.first()}else{t.last()}.ok_or(Error::Limit("empty curve edge"))?;let value=(*reference).into();
   if start{for (i,v) in t.iter().enumerate(){if i%256==0{b.poll()?;}if ((*v).into()-value).abs()>1e-7{return Ok(i.saturating_sub(1));}}Ok(n-1)}
   else{for (i,v) in t.iter().enumerate().rev(){if i%256==0{b.poll()?;}if ((*v).into()-value).abs()>1e-7{return Ok(i+1);}}Ok(0)}
  }
  fn edge32(t:&[f32],start:bool,b:&mut crate::control::Budget<'_>)->Result<usize>{
   let n=t.len();let value=*if start{t.first()}else{t.last()}.ok_or(Error::Limit("empty curve edge"))?;
   if start{for (i,&v) in t.iter().enumerate(){if i%256==0{b.poll()?;}if (v-value).abs()>1e-7{return Ok(i.saturating_sub(1));}}Ok(n-1)}
   else{for (i,&v) in t.iter().enumerate().rev(){if i%256==0{b.poll()?;}if (v-value).abs()>1e-7{return Ok(i+1);}}Ok(0)}
  }
  b.work(self.controlled_work())?;
  let value=match self{
   Self::Table(t)=>{let n=t.len();if n<2{y}else{let first=*t.first().ok_or(Error::Limit("curve first"))?;let last=*t.last().ok_or(Error::Limit("curve last"))?;let inc=last>=first;let (low,high)=if inc{(first,last)}else{(last,first)};
    if y<=low{edge32(t,inc,b)? as f32/(n-1) as f32}else if y>=high{edge32(t,!inc,b)? as f32/(n-1) as f32}else{let(mut lo,mut hi)=(0,n-1);while hi-lo>1{b.poll()?;let mid=(lo+hi)/2;if (*t.get(mid).ok_or(Error::Limit("curve midpoint"))?<y)==inc{lo=mid}else{hi=mid}}let a=*t.get(lo).ok_or(Error::Limit("curve low"))?;let z=*t.get(hi).ok_or(Error::Limit("curve high"))?;let f=if (z-a).abs()>1e-12{(y-a)/(z-a)}else{0.5};(lo as f32+f)/(n-1) as f32}
   }},
   Self::Table64(t)=>{let n=t.len();let y=f64::from(y);if n<2||!y.is_finite(){y as f32}else{let first=*t.first().ok_or(Error::Limit("curve first"))?;let last=*t.last().ok_or(Error::Limit("curve last"))?;let inc=last>=first;let(low,high)=if inc{(first,last)}else{(last,first)};
    let v=if y<=low||y>=high{let start=if y<=low{inc}else{!inc};edge(t,start,b)? as f64/(n-1) as f64}else{let(mut lo,mut hi)=(0,n-1);while hi-lo>1{b.poll()?;let mid=lo+(hi-lo)/2;if (*t.get(mid).ok_or(Error::Limit("curve midpoint"))?<y)==inc{lo=mid}else{hi=mid}}let a=*t.get(lo).ok_or(Error::Limit("curve low"))?;let z=*t.get(hi).ok_or(Error::Limit("curve high"))?;let f=if z!=a{(y-a)/(z-a)}else{0.5};(lo as f64+f)/(n-1) as f64};v as f32
   }},_=>self.eval_inverse(y)
  };b.poll()?;Ok(value)
 }
}
impl Curve {
 pub(crate) fn equals_controlled(&self,other:&Self,b:&mut crate::control::Budget<'_>)->crate::control::Result<bool>{
  fn eq<T:PartialEq>(a:&[T],z:&[T],b:&mut crate::control::Budget<'_>)->crate::control::Result<bool>{if a.len()!=z.len(){return Ok(false)}for(x,y)in a.chunks(1024).zip(z.chunks(1024)){b.work(x.len() as u64)?;if x!=y{return Ok(false)}}Ok(true)}
  b.poll()?;match(self,other){(Self::Identity,Self::Identity)=>Ok(true),(Self::Gamma(a),Self::Gamma(z))=>Ok(a==z),(Self::Parametric{kind:a,p:x},Self::Parametric{kind:z,p:y})=>Ok(a==z&&x==y),(Self::Table(a),Self::Table(z))=>eq(a,z,b),(Self::Table64(a),Self::Table64(z))=>eq(a,z,b),_=>Ok(false)}
 }
 pub(crate) fn identity_controlled(&self,b:&mut crate::control::Budget<'_>)->crate::control::Result<bool>{
  b.poll()?;match self {Self::Table(t)=>{let n=t.len();if n<2{return Ok(true)}for(i,&v)in t.iter().enumerate(){b.work(1)?;if !((f64::from(v)-i as f64/(n-1) as f64).abs()<1.0/65535.0){return Ok(false)}}Ok(true)},Self::Table64(t)=>{let n=t.len();if n<2{return Ok(true)}for(i,&v)in t.iter().enumerate(){b.work(1)?;if !((v-i as f64/(n-1) as f64).abs()<1.0/65535.0){return Ok(false)}}Ok(true)},_=>Ok(self.is_identity())}
 }
 pub(crate) fn inverse64_controlled(&self,y:f64,b:&mut crate::control::Budget<'_>)->crate::control::Result<f64>{
  fn table<T:Copy+Into<f64>>(t:&[T],y:f64,b:&mut crate::control::Budget<'_>)->crate::control::Result<f64>{
   use crate::control::invalid;b.work((t.len() as u64).checked_add(128).ok_or(crate::control::Error::Limit("inverse curve work"))?)?;let(Some(&first),Some(&last))=(t.first(),t.last())else{return Ok(y)};if t.len()<2||!y.is_finite(){return Ok(y)}let(first,last)=(first.into(),last.into());let increasing=last>=first;let(low,high)=if increasing{(first,last)}else{(last,first)};
   if y<=low||y>=high{let start=if y<=low{increasing}else{!increasing};let edge=if start{first}else{last};let mut found=None;if start{for(i,v)in t.iter().enumerate(){if i.is_multiple_of(256){b.poll()?;}if ((*v).into()-edge).abs()>1e-7{found=Some(i.saturating_sub(1));break;}}}else{for(i,v)in t.iter().enumerate().rev(){if i.is_multiple_of(256){b.poll()?;}if ((*v).into()-edge).abs()>1e-7{found=Some(i+1);break;}}}return Ok(found.unwrap_or(if start{t.len()-1}else{0}) as f64/(t.len()-1) as f64)}
   let(mut lo,mut hi)=(0,t.len()-1);while hi-lo>1{b.poll()?;let mid=lo+(hi-lo)/2;let v=(*t.get(mid).ok_or_else(||invalid("inverse curve midpoint"))?).into();if (v<y)==increasing{lo=mid}else{hi=mid}}let a=(*t.get(lo).ok_or_else(||invalid("inverse curve low"))?).into();let z=(*t.get(hi).ok_or_else(||invalid("inverse curve high"))?).into();let fraction=if z!=a{(y-a)/(z-a)}else{0.5};Ok((lo as f64+fraction)/(t.len()-1) as f64)
  }
  b.poll()?;let value=match self{Self::Table(t)=>table(t,y,b)?,Self::Table64(t)=>table(t,y,b)?,_=>{b.work(128)?;self.eval_inverse64(y)}};b.poll()?;Ok(value)
 }
}
