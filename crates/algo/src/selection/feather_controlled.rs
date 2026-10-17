//! Controlled copy of the existing feather kernel, not a different blur approximation.
//! The direct/cosine fit, run shortcuts, pass order and rounding residue are retained.
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
const TERMS: usize = 5;
const DIRECT_MAX_R: usize = 12;
const FIT_TOL: f64 = 0.3 / 255.0;
const MAX_SIGMA: f32 = 100_000.0;
const STRIP: usize = 64;
fn poll(ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    ctl.check().map_err(|_| RasterInputError::Cancelled)
}
fn tick(ctl: &Interrupt<'_>, i: usize) -> Result<(), RasterInputError> {
    if i.is_multiple_of(1024) {
        poll(ctl)?;
    }
    Ok(())
}
struct Memory {
    remaining: usize,
}
fn vec_of<T: Clone>(n: usize, value: T, ctl: &Interrupt<'_>, memory: &mut Memory) -> Result<Vec<T>, RasterInputError> {
    poll(ctl)?;
    let bytes = n.checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)?;
    if bytes > memory.remaining {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| RasterInputError::AllocationFailed)?;
    let actual = out.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)?;
    memory.remaining = memory.remaining.checked_sub(actual).ok_or(RasterInputError::LimitExceeded)?;
    while out.len() < n {
        poll(ctl)?;
        out.resize(out.len().saturating_add(1024).min(n), value.clone());
    }
    Ok(out)
}
#[derive(Clone, Copy)]
struct Win {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}
impl Win {
    fn w(self) -> usize {
        self.x1 - self.x0
    }
    fn h(self) -> usize {
        self.y1 - self.y0
    }
}
struct Rows<'a> {
    tmp: &'a [f32],
    gw: usize,
    bh: usize,
    off: usize,
    gh: usize,
}
impl Rows<'_> {
    fn get(&self, yy: usize, c0: usize, sw: usize) -> Option<&[f32]> {
        let r = yy.checked_sub(self.off)?;
        if r >= self.bh {
            return None;
        }
        let start = r.checked_mul(self.gw)?.checked_add(c0)?;
        self.tmp.get(start..start.checked_add(sw)?)
    }
}
fn add_row(acc: &mut [f64], rows: &Rows<'_>, yy: usize, c0: usize, k: f64) {
    if k == 0.0 {
        return;
    }
    if let Some(row) = rows.get(yy, c0, acc.len()) {
        for (a, v) in acc.iter_mut().zip(row) {
            *a += k * f64::from(*v);
        }
    }
}
struct CosKernel {
    r: usize,
    a0: f64,
    a: [f64; TERMS],
    /// `e^{-iω}`: shifts the window by one sample.
    rot: [(f64, f64); TERMS],
    /// `e^{-iωr}` (leaving sample) and `e^{iωr}` (entering sample).
    out_w: [(f64, f64); TERMS],
    in_w: [(f64, f64); TERMS],
    /// `Σ_{|j|≤r} cos(ωj)`: a window full of ones.
    full: [f64; TERMS],
    omega: [f64; TERMS],
}

// Keep the bounded inline representation; boxing adds a new allocation and changes its controlled storage schedule.
#[allow(clippy::large_enum_variant)]
enum Kernel {
    /// Taps for `-r..=r`.
    Direct(Vec<f32>),
    Cos(CosKernel),
}

#[allow(clippy::needless_range_loop)]
fn solve(mut a: [[f64; TERMS + 1]; TERMS + 1], mut b: [f64; TERMS + 1]) -> Option<[f64; TERMS + 1]> {
    let n = TERMS + 1;
    for c in 0..n {
        let p = (c..n).max_by(|i, j| a[*i][c].abs().total_cmp(&a[*j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for i in c + 1..n {
            let f = a[i][c] / a[c][c];
            for j in c..n {
                a[i][j] -= f * a[c][j];
            }
            b[i] -= f * b[c];
        }
    }
    let mut x = [0.0; TERMS + 1];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|j| a[i][j] * x[j]).sum();
        x[i] = (b[i] - s) / a[i][i];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}
impl Kernel {
    fn new(sigma: f64, r: usize, n: usize, ctl: &Interrupt<'_>, memory: &mut Memory) -> Result<Self, RasterInputError> {
        let g = |j: f64| (-(j * j) / (2.0 * sigma * sigma)).exp();
        let mut sum = 0.0;
        for j in 1..=r {
            tick(ctl, j)?;
            sum += g(j as f64);
        }
        let total = 1.0 + 2.0 * sum;
        let jr = r.min(n.saturating_sub(1));
        if jr == 0 {
            return Ok(Self::Direct(vec_of(1, (1.0 / total) as f32, ctl, memory)?));
        }
        let mut taps = vec_of(jr + 1, 0.0f64, ctl, memory)?;
        for (j, t) in taps.iter_mut().enumerate() {
            tick(ctl, j)?;
            *t = g(j as f64) / total;
        }
        let direct = |memory: &mut Memory| -> Result<Self, RasterInputError> {
            let mut out = vec_of(2 * jr + 1, 0.0f32, ctl, memory)?;
            for (i, t) in out.iter_mut().enumerate() {
                tick(ctl, i)?;
                *t = taps[i.abs_diff(jr)] as f32;
            }
            Ok(Self::Direct(out))
        };
        if jr <= DIRECT_MAX_R {
            return direct(memory);
        }
        let l = (jr + 1) as f64;
        let omega: [f64; TERMS] = std::array::from_fn(|k| std::f64::consts::PI * (k + 1) as f64 / l);
        let basis = |q: usize, j: usize| if q == 0 { 1.0 } else { (omega[q - 1] * j as f64).cos() };
        let wt = |j: usize| if j == 0 { 1.0 } else { 2.0 };
        let mut a = [[0.0; TERMS + 1]; TERMS + 1];
        let mut b = [0.0; TERMS + 1];
        for (j, t) in taps.iter().enumerate() {
            tick(ctl, j)?;
            let phi: [f64; TERMS + 1] = std::array::from_fn(|q| basis(q, j));
            for p in 0..=TERMS {
                b[p] += wt(j) * phi[p] * t;
                for q in 0..=TERMS {
                    a[p][q] += wt(j) * phi[p] * phi[q];
                }
            }
        }
        let Some(c) = solve(a, b) else { return direct(memory) };
        let mut err = 0.0;
        for (j, t) in taps.iter().enumerate() {
            tick(ctl, j)?;
            err += wt(j) * ((0..=TERMS).map(|q| c[q] * basis(q, j)).sum::<f64>() - t).abs();
        }
        let err = err / 2.0;
        if !err.is_finite() || err > FIT_TOL {
            return direct(memory);
        }
        let mut full = [0.0; TERMS];
        for k in 0..TERMS {
            let mut sum = 0.0;
            for j in 1..=jr {
                tick(ctl, j)?;
                sum += (omega[k] * j as f64).cos();
            }
            full[k] = 1.0 + 2.0 * sum;
        }
        let jf = jr as f64;
        Ok(Self::Cos(CosKernel {
            r: jr,
            a0: c[0],
            a: std::array::from_fn(|k| c[k + 1]),
            rot: std::array::from_fn(|k| (omega[k].cos(), -omega[k].sin())),
            out_w: std::array::from_fn(|k| ((omega[k] * jf).cos(), -(omega[k] * jf).sin())),
            in_w: std::array::from_fn(|k| ((omega[k] * jf).cos(), (omega[k] * jf).sin())),
            full,
            omega,
        }))
    }
    /// Convolves one line (zero outside it) into `dst`.
    fn line(&self, src: &[f32], dst: &mut [f32], runs: &mut [usize], ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
        let n = src.len().min(dst.len());
        match self {
            Kernel::Direct(taps) => {
                let r = taps.len() / 2;
                for part in dst[..n].chunks_mut(1024) {
                    poll(ctl)?;
                    part.fill(0.0);
                }
                for (i, k) in taps.iter().enumerate() {
                    tick(ctl, i)?;
                    // Output x reads src[x + i - r].
                    let (d0, s0) = if i >= r { (0, i - r) } else { (r - i, 0) };
                    if d0 >= n || s0 >= n {
                        continue;
                    }
                    let len = (n - d0).min(n - s0);
                    for (j, (d, s)) in dst[d0..d0 + len].iter_mut().zip(&src[s0..s0 + len]).enumerate() {
                        tick(ctl, j)?;
                        *d += k * s;
                    }
                }
            }
            Kernel::Cos(k) => k.line(&src[..n], &mut dst[..n], runs, ctl)?,
        }
        Ok(())
    }

    /// The vertical pass over columns `c0..c0 + sw` (zero outside the window).
    fn strip(&self, rows: &Rows, c0: usize, sw: usize, ctl: &Interrupt<'_>, memory: &mut Memory) -> Result<Vec<f32>, RasterInputError> {
        match self {
            Kernel::Direct(taps) => {
                let r = taps.len() / 2;
                let mut out = vec_of(rows.gh * sw, 0.0f32, ctl, memory)?;
                if sw == 0 {
                    return Ok(out);
                }
                for (yy, dst) in out.chunks_exact_mut(sw).enumerate() {
                    poll(ctl)?;
                    for (i, k) in taps.iter().enumerate() {
                        tick(ctl, i)?;
                        let Some(src) = (yy + i).checked_sub(r).and_then(|y| rows.get(y, c0, sw)) else { continue };
                        for (d, s) in dst.iter_mut().zip(src) {
                            *d += k * s;
                        }
                    }
                }
                Ok(out)
            }
            Kernel::Cos(k) => k.strip(rows, c0, sw, ctl, memory),
        }
    }
}

impl CosKernel {
    #[allow(clippy::needless_range_loop)] // `x` and `k` index several arrays in lockstep
    fn line(&self, src: &[f32], dst: &mut [f32], runs: &mut [usize], ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
        let n = src.len();
        let r = self.r;
        // runs[i]: end (exclusive) of the constant run containing i.
        if runs.len() != n {
            return Err(RasterInputError::LengthMismatch);
        }
        for part in runs.chunks_mut(1024) {
            poll(ctl)?;
            part.fill(n);
        }
        for i in (0..n.saturating_sub(1)).rev() {
            tick(ctl, i)?;
            runs[i] = if src[i] == src[i + 1] { runs[i + 1] } else { i + 1 };
        }
        let at = |i: usize| f64::from(src.get(i).copied().unwrap_or(0.0));
        // Window sums for the last position handled (valid when `have`).
        let mut have = false;
        let mut sb = 0.0f64;
        let mut s = [(0.0f64, 0.0f64); TERMS];
        for x in 0..n {
            tick(ctl, x)?;
            let lo = x.saturating_sub(r);
            let hi = (x + r + 1).min(n);
            let c = src[lo];
            if runs[lo] >= hi && (c == 0.0 || (x >= r && x + r < n)) {
                // The whole window is one value (zeros may run off the line; ones may not).
                dst[x] = c;
                let cf = f64::from(c);
                sb = cf * (2 * r + 1) as f64;
                for (sk, fk) in s.iter_mut().zip(&self.full) {
                    *sk = (cf * fk, 0.0);
                }
                have = true;
                continue;
            }
            if have {
                // Slide from x - 1: drop src[x - 1 - r], take src[x + r].
                let gone = if x > r { at(x - 1 - r) } else { 0.0 };
                let come = at(x + r);
                sb += come - gone;
                for k in 0..TERMS {
                    let (re, im) = (s[k].0 - gone * self.out_w[k].0, s[k].1 - gone * self.out_w[k].1);
                    let (cr, ci) = self.rot[k];
                    s[k] = (re * cr - im * ci + come * self.in_w[k].0, re * ci + im * cr + come * self.in_w[k].1);
                }
            } else {
                // Direct sums over the in-line part of the window.
                sb = 0.0;
                s = [(0.0, 0.0); TERMS];
                for (i, v) in src.iter().enumerate().take(hi).skip(lo) {
                    tick(ctl, i)?;
                    let v = f64::from(*v);
                    if v == 0.0 {
                        continue;
                    }
                    let j = i as f64 - x as f64;
                    sb += v;
                    for (sk, w) in s.iter_mut().zip(&self.omega) {
                        sk.0 += v * (w * j).cos();
                        sk.1 += v * (w * j).sin();
                    }
                }
                have = true;
            }
            let v = self.a0 * sb + s.iter().zip(&self.a).map(|(sk, a)| a * sk.0).sum::<f64>();
            dst[x] = v as f32;
        }
        Ok(())
    }
}

impl CosKernel {
    /// [`CosKernel::line`] down columns `c0..c0 + sw`, all columns of a row at once.
    #[allow(clippy::needless_range_loop)] // `k` indexes several term arrays in lockstep
    fn strip(&self, rows: &Rows, c0: usize, sw: usize, ctl: &Interrupt<'_>, memory: &mut Memory) -> Result<Vec<f32>, RasterInputError> {
        let gh = rows.gh;
        let r = self.r;
        let mut out = vec_of(gh * sw, 0.0f32, ctl, memory)?;
        if sw == 0 {
            return Ok(out);
        }
        // Rows whose strip segment is one value, and the runs of equal such rows.
        let mut flag = vec_of(gh, None, ctl, memory)?;
        for (yy, item) in flag.iter_mut().enumerate() {
            tick(ctl, yy)?;
            *item = match rows.get(yy, c0, sw) {
                None => Some(0.0),
                Some(s) => {
                    let c = s.first().copied().unwrap_or(0.0);
                    s.iter().all(|v| *v == c).then_some(c)
                }
            };
        }
        let mut runs = vec_of(gh, gh, ctl, memory)?;
        for i in (0..gh.saturating_sub(1)).rev() {
            tick(ctl, i)?;
            runs[i] = match (flag[i], flag[i + 1]) {
                (Some(a), Some(b)) if a == b => runs[i + 1],
                _ => i + 1,
            };
        }
        let zeros = vec_of(sw, 0.0f32, ctl, memory)?;
        let mut sb = vec_of(sw, 0.0f64, ctl, memory)?;
        let mut re = vec_of(TERMS, Vec::new(), ctl, memory)?;
        for row in &mut re {
            *row = vec_of(sw, 0.0f64, ctl, memory)?;
        }
        let mut im = vec_of(TERMS, Vec::new(), ctl, memory)?;
        for row in &mut im {
            *row = vec_of(sw, 0.0f64, ctl, memory)?;
        }
        let mut acc = vec_of(sw, 0.0f64, ctl, memory)?;
        let mut have = false;
        for (yy, dst) in out.chunks_exact_mut(sw).enumerate() {
            poll(ctl)?;
            let lo = yy.saturating_sub(r);
            let hi = (yy + r + 1).min(gh);
            // The whole window is one value (zero rows may run off the window; others may not).
            if let Some(c) = flag[lo]
                && runs[lo] >= hi
                && (c == 0.0 || (yy >= r && yy + r < gh))
            {
                dst.fill(c);
                let cf = f64::from(c);
                sb.fill(cf * (2 * r + 1) as f64);
                for k in 0..TERMS {
                    re[k].fill(cf * self.full[k]);
                    im[k].fill(0.0);
                }
                have = true;
                continue;
            }
            if have {
                // Slide from yy - 1: drop row yy - 1 - r, take row yy + r.
                let gone = if yy > r { rows.get(yy - 1 - r, c0, sw) } else { None }.unwrap_or(&zeros);
                let come = rows.get(yy + r, c0, sw).unwrap_or(&zeros);
                for ((s, g), c) in sb.iter_mut().zip(gone).zip(come) {
                    *s += f64::from(*c) - f64::from(*g);
                }
                for k in 0..TERMS {
                    let (cr, ci) = self.rot[k];
                    let (o0, o1) = self.out_w[k];
                    let (i0, i1) = self.in_w[k];
                    for (((sr, si), g), c) in re[k].iter_mut().zip(im[k].iter_mut()).zip(gone).zip(come) {
                        let (g, c) = (f64::from(*g), f64::from(*c));
                        let (a, b) = (*sr - g * o0, *si - g * o1);
                        *sr = a * cr - b * ci + c * i0;
                        *si = a * ci + b * cr + c * i1;
                    }
                }
            } else {
                // Direct sums over the in-window rows.
                sb.fill(0.0);
                for k in 0..TERMS {
                    re[k].fill(0.0);
                    im[k].fill(0.0);
                }
                for i in lo..hi {
                    tick(ctl, i)?;
                    let Some(row) = rows.get(i, c0, sw) else { continue };
                    let j = i as f64 - yy as f64;
                    add_row(&mut sb, rows, i, c0, 1.0);
                    for k in 0..TERMS {
                        let (cs, sn) = ((self.omega[k] * j).cos(), (self.omega[k] * j).sin());
                        for ((sr, si), v) in re[k].iter_mut().zip(im[k].iter_mut()).zip(row) {
                            *sr += f64::from(*v) * cs;
                            *si += f64::from(*v) * sn;
                        }
                    }
                }
                have = true;
            }
            for (a, s) in acc.iter_mut().zip(&sb) {
                *a = self.a0 * s;
            }
            for k in 0..TERMS {
                for (a, s) in acc.iter_mut().zip(&re[k]) {
                    *a += self.a[k] * s;
                }
            }
            for (d, a) in dst.iter_mut().zip(&acc) {
                *d = *a as f32;
            }
        }
        Ok(out)
    }
}

/// Same numerical passes as `selection::feather`, with checked ordinary coverage input.
/// The byte envelope includes borrowed input and all cumulative allocated capacities
/// (a conservative bound on simultaneous residency); it is not a process RSS budget.
pub(super) fn try_feather(m: &[f32], w: usize, h: usize, radius: f32, ctl: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
    poll(ctl)?;
    let n = w.checked_mul(h).ok_or(RasterInputError::InvalidBounds)?;
    if n != m.len() {
        return Err(RasterInputError::LengthMismatch);
    }
    if !radius.is_finite() {
        return Err(RasterInputError::InvalidSample);
    }
    let sigma = if radius == 0.0 { 0.0 } else { radius.clamp(0.0, MAX_SIGMA) };
    let r = (sigma * 3.0).ceil() as usize;
    let active = sigma >= 0.1 && n != 0;
    let bytes = if active {
        n.checked_mul(48)
            .and_then(|n| n.checked_add(r.checked_mul(256)?))
            .and_then(|n| n.checked_add(w.max(h).checked_mul(64)?))
            .and_then(|n| n.checked_add(65_536))
    } else {
        n.checked_mul(8)
    }
    .ok_or(RasterInputError::LimitExceeded)?;
    if u64::try_from(bytes).map_err(|_| RasterInputError::LimitExceeded)? > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    // Includes the worst direct fallback, the cosine fit and line initialization.
    let work = n
        .checked_mul(if active { r.checked_mul(4).and_then(|v| v.checked_add(128)).ok_or(RasterInputError::LimitExceeded)? } else { 2 })
        .and_then(|v| v.checked_add(if active { r.checked_mul(512)? } else { 0 }))
        .ok_or(RasterInputError::LimitExceeded)?;
    if u64::try_from(work).map_err(|_| RasterInputError::LimitExceeded)? > 1u64 << 32 {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut memory = Memory { remaining: bytes.checked_sub(n.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)? };
    let mut bounds = None::<Win>;
    for (i, v) in m.iter().enumerate() {
        tick(ctl, i)?;
        if !v.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        if *v != 0.0 {
            let x = i % w;
            let y = i / w;
            bounds = Some(match bounds {
                None => Win { x0: x, x1: x + 1, y0: y, y1: y + 1 },
                Some(b) => Win { x0: b.x0.min(x), x1: b.x1.max(x + 1), y0: b.y0.min(y), y1: b.y1.max(y + 1) },
            });
        }
    }
    if !active || bounds.is_none() {
        let mut out = vec_of(n, 0.0, ctl, &mut memory)?;
        for (a, b) in out.chunks_mut(1024).zip(m.chunks(1024)) {
            poll(ctl)?;
            a.copy_from_slice(b);
        }
        return Ok(out);
    }
    let b = bounds.ok_or(RasterInputError::InvalidBounds)?;
    let win = Win { x0: b.x0.saturating_sub(r), x1: b.x1.saturating_add(r).min(w), y0: b.y0.saturating_sub(r), y1: b.y1.saturating_add(r).min(h) };
    let (gw, gh) = (win.w(), win.h());
    let kh = Kernel::new(f64::from(sigma), r, gw, ctl, &mut memory)?;
    let kv = Kernel::new(f64::from(sigma), r, gh, ctl, &mut memory)?;
    let mut tmp = vec_of(b.h() * gw, 0.0f32, ctl, &mut memory)?;
    let mut runs = vec_of(gw, gw, ctl, &mut memory)?;
    for (y, row) in tmp.chunks_exact_mut(gw).enumerate() {
        poll(ctl)?;
        let o = (b.y0 + y) * w + win.x0;
        let src = m.get(o..o + gw).ok_or(RasterInputError::LengthMismatch)?;
        kh.line(src, row, &mut runs, ctl)?;
        for (i, v) in row.iter().enumerate() {
            tick(ctl, i)?;
            if !v.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
        }
    }
    let rows = Rows { tmp: &tmp, gw, bh: b.h(), off: b.y0 - win.y0, gh };
    let count = gw.div_ceil(STRIP);
    let mut strips = vec_of(count, Vec::new(), ctl, &mut memory)?;
    for (i, out) in strips.iter_mut().enumerate() {
        poll(ctl)?;
        let c0 = i * STRIP;
        *out = kv.strip(&rows, c0, STRIP.min(gw - c0), ctl, &mut memory)?;
    }
    drop(tmp);
    let mut out = vec_of(n, 0.0f32, ctl, &mut memory)?;
    for (y, row) in out.chunks_exact_mut(w).enumerate() {
        poll(ctl)?;
        if y < win.y0 || y >= win.y1 {
            continue;
        }
        let yy = y - win.y0;
        for (i, strip) in strips.iter().enumerate() {
            tick(ctl, i)?;
            let c0 = i * STRIP;
            let sw = STRIP.min(gw - c0);
            let src = strip.get(yy * sw..(yy + 1) * sw).ok_or(RasterInputError::LengthMismatch)?;
            let dst = row.get_mut(win.x0 + c0..win.x0 + c0 + sw).ok_or(RasterInputError::InvalidBounds)?;
            for (d, v) in dst.iter_mut().zip(src) {
                if !v.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                *d = if v.abs() < 1e-7 { 0.0 } else { *v };
            }
        }
    }
    poll(ctl)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_legacy_passes_and_edge_windows() {
        for (w, h) in [(1, 1), (1, 65), (65, 1), (17, 23), (129, 131)] {
            let m: Vec<f32> = (0..w * h).map(|i| if i % 7 == 0 { 0.0 } else { ((i * 37) % 101) as f32 / 100.0 }).collect();
            for radius in [0.0, 0.19, 0.2, 1.0, 8.0, 25.0, 75.0] {
                let old = crate::selection::feather(&m, w, h, radius);
                let controlled = try_feather(&m, w, h, radius, &Interrupt::NONE).unwrap();
                assert_eq!(old.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), controlled.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
    }
    #[test]
    fn constant_sparse_and_raw_identity() {
        for m in [vec![0.0; 65 * 33], vec![1.0; 65 * 33], {
            let mut m = vec![0.0; 65 * 33];
            m[0] = 1.0;
            m[65 * 33 - 1] = 0.25;
            m
        }] {
            for radius in [0.0, 1.0, 30.0] {
                let old = crate::selection::feather(&m, 65, 33, radius);
                let new = try_feather(&m, 65, 33, radius, &Interrupt::NONE).unwrap();
                assert_eq!(old.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), new.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
        let raw = [-0.0, 0.0];
        assert_eq!(
            try_feather(&raw, 2, 1, 0.0, &Interrupt::NONE).unwrap().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            raw.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }
    #[test]
    fn cancellation_during_preparation_and_passes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let m: Vec<f32> = (0..129 * 131).map(|i| if i % 3 == 0 { 1.0 } else { 0.25 }).collect();
        let before = m.clone();
        for after in [0, 8, 64, 128, 256] {
            let checks = AtomicUsize::new(0);
            let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= after;
            assert_eq!(try_feather(&m, 129, 131, 25.0, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled));
            assert_eq!(m, before);
        }
    }
    #[test]
    fn malformed_and_cancelled_do_not_publish() {
        assert_eq!(try_feather(&[f32::NAN], 1, 1, 2.0, &Interrupt::NONE), Err(RasterInputError::InvalidSample));
        assert_eq!(try_feather(&[1.0], 2, 1, 2.0, &Interrupt::NONE), Err(RasterInputError::LengthMismatch));
        assert_eq!(try_feather(&[], usize::MAX, 2, 2.0, &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
    }
}
