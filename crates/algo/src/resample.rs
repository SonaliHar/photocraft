//! High-quality separable resampling of surfaces (Image Size).

use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Surface, Interrupt, RasterInputError, MAX_CHECKED_RASTER_BYTES, checked_region_byte_len};
use serde::{Deserialize, Serialize};

use crate::image::{premultiply, unpremultiply};

/// Resampling method (Photoshop's Image Size options).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Resample {
    Nearest,
    Bilinear,
    #[default]
    Bicubic,
    Lanczos,
    /// Lanczos followed by a light sharpening when enlarging.
    PreserveDetails,
}

impl Resample {
    pub(crate) fn support(self) -> f64 {
        match self {
            Resample::Nearest => 0.5,
            Resample::Bilinear => 1.0,
            Resample::Bicubic => 2.0,
            Resample::Lanczos | Resample::PreserveDetails => 3.0,
        }
    }
    pub(crate) fn weight(self, x: f64) -> f64 {
        let x = x.abs();
        match self {
            Resample::Nearest => {
                if x <= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Resample::Bilinear => (1.0 - x).max(0.0),
            Resample::Bicubic => {
                // Keys cubic, a = -0.5 (Catmull-Rom).
                let a = -0.5;
                if x < 1.0 {
                    (a + 2.0) * x * x * x - (a + 3.0) * x * x + 1.0
                } else if x < 2.0 {
                    a * x * x * x - 5.0 * a * x * x + 8.0 * a * x - 4.0 * a
                } else {
                    0.0
                }
            }
            Resample::Lanczos | Resample::PreserveDetails => {
                if x < 1e-9 {
                    1.0
                } else if x < 3.0 {
                    let px = std::f64::consts::PI * x;
                    3.0 * px.sin() * (px / 3.0).sin() / (px * px)
                } else {
                    0.0
                }
            }
        }
    }
}

/// Per-output-sample source window and normalized weights.
struct Taps {
    start: i32,
    weights: Vec<f32>,
    /// Total weight of taps outside the source's content, which all read the default pixel.
    outside: f32,
}

/// Taps for mapping output coordinate `o` (in `[dst0, dst1)`) to the source
/// via `u = (o + 0.5) / scale - 0.5`. With `edge` (`[first, last]` source index), taps outside it
/// read the nearest sample inside it: the image repeats its edge instead of fading out.
/// Taps outside `content` (`[first, last]`) all read the default pixel, so they are folded into
/// `outside` instead of being read: the read window never grows past the content, however small
/// the scale (#713).
fn taps(dst0: i32, dst1: i32, scale: f64, filter: Resample, edge: Option<(i32, i32)>, content: (i32, i32)) -> Vec<Taps> {
    let stretch = if scale < 1.0 { 1.0 / scale } else { 1.0 };
    let support = filter.support() * stretch;
    let clamp = |i: i32| edge.map_or(i, |(a, b)| i.clamp(a, b));
    (dst0..dst1)
        .map(|o| {
            let u = (o as f64 + 0.5) / scale - 0.5;
            if filter == Resample::Nearest {
                let i = clamp((u + 0.5).floor() as i32);
                return if (content.0..=content.1).contains(&i) {
                    Taps { start: i, weights: vec![1.0], outside: 0.0 }
                } else {
                    Taps { start: content.0, weights: Vec::new(), outside: 1.0 }
                };
            }
            let lo = (u - support).ceil() as i32;
            let hi = (u + support).floor() as i32;
            let mut w: Vec<f64> = (lo..=hi).map(|i| filter.weight((i as f64 - u) / stretch)).collect();
            let s: f64 = w.iter().sum();
            if s.abs() > 1e-12 {
                for v in &mut w {
                    *v /= s;
                }
            }
            // Fold the weights of taps beyond the edge onto the edge sample.
            let start = clamp(lo);
            let mut folded = vec![0.0f32; (clamp(hi) - start + 1).max(1) as usize];
            for (i, v) in (lo..=hi).zip(w) {
                if let Some(slot) = folded.get_mut((clamp(i) - start) as usize) {
                    *slot += v as f32;
                }
            }
            // Keep the part inside the content; the rest reads the default pixel.
            let (a, b) = (start.max(content.0), (start + folded.len() as i32 - 1).min(content.1));
            if a > b {
                return Taps { start: content.0, weights: Vec::new(), outside: folded.iter().sum() };
            }
            let (i0, i1) = ((a - start) as usize, (b - start) as usize + 1);
            let outside = folded.get(..i0).unwrap_or(&[]).iter().chain(folded.get(i1..).unwrap_or(&[])).sum();
            Taps { start: a, weights: folded.get(i0..i1).unwrap_or(&[]).to_vec(), outside }
        })
        .collect()
}

fn resize_poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn resize_vec<T: Clone>(length: usize, value: T, control: &Interrupt<'_>) -> Result<Vec<T>, RasterInputError> {
    let bytes = length.checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)?;
    if bytes as u64 > MAX_CHECKED_RASTER_BYTES { return Err(RasterInputError::LimitExceeded); }
    let mut result = Vec::new();
    result.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    if result.capacity() != length || result.capacity().checked_mul(std::mem::size_of::<T>()).is_none_or(|size| size as u64 > MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    while result.len() < length { resize_poll(control)?; result.resize(result.len().saturating_add(8192).min(length), value.clone()); }
    Ok(result)
}
fn resize_count(width: i32, height: i32, channels: usize) -> Result<usize, RasterInputError> {
    usize::try_from(width).ok().and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|value| value.checked_mul(channels)).ok_or(RasterInputError::InvalidBounds)
}
fn tap_coordinate(value: f64) -> Result<i32, RasterInputError> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) { return Err(RasterInputError::InvalidBounds); }
    Ok(value as i32)
}
fn taps_with_interrupt(dst0: i32, dst1: i32, scale: f64, filter: Resample, edge: Option<(i32, i32)>, content: (i32, i32), control: &Interrupt<'_>, retained_limit: u64) -> Result<Vec<Taps>, RasterInputError> {
    resize_poll(control)?;
    if !scale.is_finite() || scale <= 0.0 || content.0 > content.1 || edge.is_some_and(|(a,b)| a > b) { return Err(RasterInputError::InvalidBounds); }
    let stretch = if scale < 1.0 { 1.0 / scale } else { 1.0 };
    let support = filter.support() * stretch;
    if !support.is_finite() { return Err(RasterInputError::InvalidBounds); }
    let clamp = |i: i32| edge.map_or(i, |(a,b)| i.clamp(a,b));
    let length = usize::try_from(i64::from(dst1) - i64::from(dst0)).map_err(|_| RasterInputError::InvalidBounds)?;
    if (length as u64).checked_mul(std::mem::size_of::<Taps>() as u64).is_none_or(|size| size > retained_limit) { return Err(RasterInputError::LimitExceeded); }
    let mut result = Vec::new();
    result.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    let mut retained = (result.capacity() as u64).checked_mul(std::mem::size_of::<Taps>() as u64).ok_or(RasterInputError::LimitExceeded)?;
    if retained > retained_limit { return Err(RasterInputError::LimitExceeded); }
    for o in dst0..dst1 {
        resize_poll(control)?;
        let u = (f64::from(o) + 0.5) / scale - 0.5;
        let tap = if filter == Resample::Nearest {
            let i = clamp(tap_coordinate((u + 0.5).floor())?);
            if (content.0..=content.1).contains(&i) { Taps { start: i, weights: resize_vec(1,1.0,control)?, outside: 0.0 } }
            else { Taps { start: content.0, weights: Vec::new(), outside: 1.0 } }
        } else {
            let lo = tap_coordinate((u-support).ceil())?; let hi = tap_coordinate((u+support).floor())?;
            let count = usize::try_from(i64::from(hi)-i64::from(lo)+1).map_err(|_| RasterInputError::InvalidBounds)?;
            let transient = (count as u64).checked_mul(12).and_then(|v| v.checked_add(retained)).ok_or(RasterInputError::LimitExceeded)?;
            if transient > retained_limit { return Err(RasterInputError::LimitExceeded); }
            let mut w = resize_vec(count,0.0f64,control)?;
            for (index,v) in w.iter_mut().enumerate() { resize_poll(control)?; let i = i64::from(lo) + index as i64; *v = filter.weight((i as f64-u)/stretch); }
            let mut sum = 0.0f64; for v in &w { resize_poll(control)?; sum += *v; }
            if sum.abs() > 1e-12 { for v in &mut w { resize_poll(control)?; *v /= sum; } }
            let start = clamp(lo);
            let folded_len = usize::try_from((i64::from(clamp(hi))-i64::from(start)+1).max(1)).map_err(|_| RasterInputError::InvalidBounds)?;
            let mut folded = resize_vec(folded_len,0.0f32,control)?;
            for (index,v) in w.into_iter().enumerate() { resize_poll(control)?; let i = i32::try_from(i64::from(lo)+index as i64).map_err(|_| RasterInputError::InvalidBounds)?;
                let slot = usize::try_from(i64::from(clamp(i))-i64::from(start)).map_err(|_| RasterInputError::InvalidBounds)?;
                if let Some(value) = folded.get_mut(slot) { *value += v as f32; }
            }
            let a = start.max(content.0);
            let end = i64::from(start) + i64::try_from(folded.len()).map_err(|_| RasterInputError::InvalidBounds)? - 1;
            let b = end.min(i64::from(content.1));
            if i64::from(a) > b { Taps { start: content.0, weights: Vec::new(), outside: { let mut sum=0.0f32; for v in &folded { resize_poll(control)?; sum += *v; } sum } } }
            else {
                let i0 = usize::try_from(i64::from(a)-i64::from(start)).map_err(|_| RasterInputError::InvalidBounds)?;
                let i1 = usize::try_from(b-i64::from(start)+1).map_err(|_| RasterInputError::InvalidBounds)?;
                let mut outside = 0.0f32; for v in folded.get(..i0).ok_or(RasterInputError::LengthMismatch)?.iter().chain(folded.get(i1..).ok_or(RasterInputError::LengthMismatch)?) { resize_poll(control)?; outside += *v; }
                let values = folded.get(i0..i1).ok_or(RasterInputError::LengthMismatch)?;
                let mut weights = resize_vec(values.len(),0.0f32,control)?;
                for (value,input) in weights.iter_mut().zip(values) { resize_poll(control)?; *value = *input; }
                Taps { start: a, weights, outside }
            }
        };
        retained = retained.checked_add((tap.weights.capacity() as u64).checked_mul(4).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        if retained > retained_limit { return Err(RasterInputError::LimitExceeded); }
        result.push(tap);
    }
    Ok(result)
}
fn read_resize_rows(s: &Surface, r: Rect, control: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
    let raw = s.try_read_interleaved_with(r,control)?;
    let mut values = resize_vec(resize_count(i32::try_from(r.width()).map_err(|_| RasterInputError::InvalidBounds)?,i32::try_from(r.height()).map_err(|_| RasterInputError::InvalidBounds)?,s.channels())?,0.0f32,control)?;
    for (index,v) in values.iter_mut().enumerate() { if index % 1024 == 0 { resize_poll(control)?; } *v = photocraft_color::read_sample(&raw,s.format().sample,index); }
    Ok(values)
}
fn write_resize_rows(s: &mut Surface, r: Rect, values: &[f32], control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if values.len() != resize_count(i32::try_from(r.width()).map_err(|_| RasterInputError::InvalidBounds)?,i32::try_from(r.height()).map_err(|_| RasterInputError::InvalidBounds)?,s.channels())? { return Err(RasterInputError::LengthMismatch); }
    let mut raw = resize_vec(checked_region_byte_len(r,s.format())?,0u8,control)?;
    for (index,&v) in values.iter().enumerate() { if index % 1024 == 0 { resize_poll(control)?; } photocraft_color::write_sample(&mut raw,s.format().sample,index,v); }
    s.try_write_interleaved_with(r,&raw,control)
}

/// Output rectangle for `r` scaled by `(sx, sy)` about the origin.
pub fn scaled_rect(r: Rect, sx: f64, sy: f64) -> Rect {
    if r.is_empty() {
        return Rect::EMPTY;
    }
    Rect::new((r.x0 as f64 * sx).floor() as i32, (r.y0 as f64 * sy).floor() as i32, (r.x1 as f64 * sx).ceil() as i32, (r.y1 as f64 * sy).ceil() as i32)
}

/// Resizes a surface by `(sx, sy)` about the document origin. The surface's
/// default pixel is kept (masks keep their "reveal all" background).
pub fn resize_surface(s: &Surface, sx: f64, sy: f64, filter: Resample) -> Surface {
    resize(s, sx, sy, filter, None)
}

/// [`resize_surface`] for a layer, mask or channel of a document whose canvas is `canvas`
/// (Image Size): at the canvas edges the pixels repeat outwards, so content that reaches an edge
/// keeps it (an opaque layer stays opaque to the border) instead of fading into the default pixel.
/// Pixels beyond the canvas count as the surface's own, and inside the canvas nothing changes.
pub fn resize_surface_in_canvas(s: &Surface, sx: f64, sy: f64, filter: Resample, canvas: Rect) -> Surface {
    let area = s.content_bounds().union(&canvas);
    resize(s, sx, sy, filter, (!area.is_empty()).then_some(area))
}

/// Samples one source read may hold (64 MB of `f32`); larger windows are read in row chunks.
const READ_BUDGET: usize = 1 << 24;

// Preserve tap order and edge weights while exposing the common channel counts to LLVM.
fn horizontal_fixed<const N: usize>(row: &[f32], out: &mut [f32], taps: &[Taps], x_lo: i32, def: &[f32]) {
    let pixels = row.as_chunks::<N>().0;
    for (t, d) in taps.iter().zip(out.as_chunks_mut::<N>().0) {
        let mut acc = [0.0f32; N];
        for (k, w) in t.weights.iter().enumerate() {
            if let Some(sp) = pixels.get((t.start + k as i32 - x_lo) as usize) {
                for c in 0..N {
                    acc[c] += sp[c] * w;
                }
            }
        }
        for (c, v) in acc.iter_mut().enumerate() {
            *v += def.get(c).copied().unwrap_or(0.0) * t.outside;
        }
        *d = acc;
    }
}

fn horizontal_dynamic(row: &[f32], out: &mut [f32], taps: &[Taps], x_lo: i32, def: &[f32], n: usize) {
    for (t, d) in taps.iter().zip(out.chunks_exact_mut(n)) {
        for (k, w) in t.weights.iter().enumerate() {
            let sx_ = (t.start + k as i32 - x_lo) as usize;
            if let Some(sp) = row.get(sx_ * n..(sx_ + 1) * n) {
                for c in 0..n {
                    d[c] += sp[c] * w;
                }
            }
        }
        for c in 0..n {
            d[c] += def[c] * t.outside;
        }
    }
}

fn resize(s: &Surface, sx: f64, sy: f64, filter: Resample, edge: Option<Rect>) -> Surface {
    resize_with_budget(s, sx, sy, filter, edge, READ_BUDGET)
}

fn resize_with_budget(s: &Surface, sx: f64, sy: f64, filter: Resample, edge: Option<Rect>, budget: usize) -> Surface {
    let fmt = s.format();
    let mut out = Surface::with_default(fmt, &s.default_pixel());
    let src = s.content_bounds();
    let dst = scaled_rect(src, sx, sy);
    if dst.is_empty() {
        return out;
    }
    let n = fmt.channels();
    let alpha = fmt.alpha;
    // With `edge`, every tap is clamped into the edge area, which holds the content.
    let area = edge.unwrap_or(src);
    let hx = taps(dst.x0, dst.x1, sx, filter, edge.map(|r| (r.x0, r.x1 - 1)), (area.x0, area.x1 - 1));
    let vy = taps(dst.y0, dst.y1, sy, filter, edge.map(|r| (r.y0, r.y1 - 1)), (area.y0, area.y1 - 1));
    let x_lo = hx.iter().filter(|t| !t.weights.is_empty()).map(|t| t.start).min().unwrap_or(area.x0);
    let x_hi = hx.iter().map(|t| t.start + t.weights.len() as i32).max().unwrap_or(area.x0).max(x_lo);
    let mut def = s.default_pixel();
    premultiply(&mut def, n, alpha);
    let dw = dst.width() as usize;
    const BAND: usize = 64;
    let bands: Vec<(usize, usize)> = (0..vy.len()).step_by(BAND).map(|b| (b, (b + BAND).min(vy.len()))).collect();
    // Horizontal pass of one source row (`row` spans `x_lo..x_hi`) into `dw` samples.
    let horizontal = |row: &[f32], out: &mut [f32]| match n {
        1 => horizontal_fixed::<1>(row, out, &hx, x_lo, &def),
        2 => horizontal_fixed::<2>(row, out, &hx, x_lo, &def),
        3 => horizontal_fixed::<3>(row, out, &hx, x_lo, &def),
        4 => horizontal_fixed::<4>(row, out, &hx, x_lo, &def),
        5 => horizontal_fixed::<5>(row, out, &hx, x_lo, &def),
        _ => horizontal_dynamic(row, out, &hx, x_lo, &def, n),
    };
    // A source row of nothing but the default pixel, after the horizontal pass.
    let def_row = {
        let row: Vec<f32> = def.iter().copied().cycle().take((x_hi - x_lo) as usize * n).collect();
        let mut out = vec![0.0f32; dw * n];
        horizontal(&row, &mut out);
        out
    };
    // At most one tile row per read anyway, so this always fits an `i32`.
    let rows_per_read = (budget / ((x_hi - x_lo).max(1) as usize * n).max(dw * n)).clamp(1, TILE_SIZE as usize) as i32;
    let run = |&(b0, b1): &(usize, usize)| -> (Rect, Vec<f32>) {
        let band = vy.get(b0..b1).unwrap_or(&[]);
        let y_lo = band.iter().filter(|t| !t.weights.is_empty()).map(|t| t.start).min().unwrap_or(area.y0);
        let y_hi = band.iter().map(|t| t.start + t.weights.len() as i32).max().unwrap_or(area.y0).max(y_lo);
        let mut res = vec![0.0f32; band.len() * dw * n];
        // Source rows stream through in bounded reads and go straight into the output rows that
        // use them, so memory is the read budget plus this band's output, whatever the scale or
        // the source size (#713). In ascending row order, as one read of the whole window would.
        let add = |res: &mut [f32], y: i32, row: &[f32]| {
            for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
                if let Some(w) = t.weights.get((y - t.start) as usize).filter(|_| y >= t.start) {
                    for (dv, sv) in d.iter_mut().zip(row) {
                        *dv += sv * w;
                    }
                }
            }
        };
        let mut rows = Vec::new();
        let mut hrows = Vec::new();
        let mut y = y_lo;
        while y < y_hi {
            let y1 = ((y.div_euclid(TILE_SIZE) + 1) * TILE_SIZE).min(y_hi);
            if s.has_tiles_in(Rect::new(x_lo, y, x_hi, y1)) {
                let mut c0 = y;
                while c0 < y1 {
                    let c1 = (c0 + rows_per_read).min(y1);
                    let read = Rect::new(x_lo, c0, x_hi, c1);
                    s.read_region_into(read, &mut rows);
                    premultiply(&mut rows, n, alpha);
                    hrows.clear();
                    hrows.resize((c1 - c0) as usize * dw * n, 0.0);
                    let rw = read.width() as usize * n;
                    for (row, out) in rows.chunks_exact(rw.max(1)).zip(hrows.chunks_exact_mut(dw * n)) {
                        horizontal(row, out);
                    }
                    for (r, h) in (c0..c1).zip(hrows.chunks_exact(dw * n)) {
                        add(&mut res, r, h);
                    }
                    c0 = c1;
                }
            } else {
                // Nothing stored in this tile row: every row in it is the default row, so each
                // output row takes it once, with the summed weight of those rows.
                for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
                    let (a, b) = ((y - t.start).max(0) as usize, (y1 - t.start).max(0) as usize);
                    let w: f32 = t.weights.get(a.min(t.weights.len())..b.min(t.weights.len())).unwrap_or(&[]).iter().sum();
                    if w != 0.0 {
                        for (dv, sv) in d.iter_mut().zip(&def_row) {
                            *dv += sv * w;
                        }
                    }
                }
            }
            y = y1;
        }
        for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
            for (dv, sv) in d.iter_mut().zip(&def_row) {
                *dv += sv * t.outside;
            }
        }
        if alpha {
            for px in res.chunks_exact_mut(n) {
                px[n - 1] = px[n - 1].clamp(0.0, 1.0);
            }
        }
        unpremultiply(&mut res, n, alpha);
        (Rect::new(dst.x0, dst.y0 + b0 as i32, dst.x1, dst.y0 + b1 as i32), res)
    };
    // Bands go into the output a group at a time, so the `f32` rows held at once are bounded by
    // the group, not by the whole destination (#1544).
    #[cfg(not(target_arch = "wasm32"))]
    let group = rayon::current_num_threads().max(1) * 2;
    #[cfg(target_arch = "wasm32")]
    let group = 1;
    for chunk in bands.chunks(group) {
        #[cfg(not(target_arch = "wasm32"))]
        let parts: Vec<(Rect, Vec<f32>)> = {
            use rayon::prelude::*;
            chunk.par_iter().map(run).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let parts: Vec<(Rect, Vec<f32>)> = chunk.iter().map(run).collect();
        for (r, d) in parts {
            out.write_region(r, &d);
        }
    }
    if filter == Resample::PreserveDetails && (sx > 1.0 || sy > 1.0) {
        let p = crate::FilterParams::UnsharpMask { amount: 30.0, radius: 0.6 * sx.max(sy) as f32, threshold: 0.0 };
        let area = crate::output_area(&p, out.content_bounds(), dst, None);
        out = crate::apply(&out, &p, area, dst, None);
    }
    out.prune();
    out
}

/// Controlled legacy resampling without canvas-edge repetition (warp pre-reduction).
pub fn try_resize_surface_with_interrupt(s: &Surface, sx: f64, sy: f64, filter: Resample, control: &Interrupt<'_>) -> Result<Surface,RasterInputError> {
    try_resize_with_budget(s,sx,sy,filter,None,READ_BUDGET,control)
}
/// Controlled legacy f32 resampling, with the same taps/edge/band arithmetic.
pub fn try_resize_surface_in_canvas_with_interrupt(s: &Surface, sx: f64, sy: f64, filter: Resample, canvas: Rect, control: &Interrupt<'_>) -> Result<Surface,RasterInputError> {
    resize_poll(control)?;
    let area = s.try_content_bounds_with_interrupt(control)?.union(&canvas);
    try_resize_with_budget(s,sx,sy,filter,(!area.is_empty()).then_some(area),READ_BUDGET,control)
}
fn try_resize_with_budget(s: &Surface, sx: f64, sy: f64, filter: Resample, edge: Option<Rect>, budget: usize, control: &Interrupt<'_>) -> Result<Surface,RasterInputError> {
    resize_poll(control)?;
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 { return Err(RasterInputError::InvalidBounds); }
    s.try_validate_tile_origins_with(control)?;
    let fmt = s.format();
    let mut default = resize_vec(fmt.bytes_per_pixel(),0u8,control)?;
    for (index,value) in s.default_pixel().into_iter().enumerate() { photocraft_color::write_sample(&mut default,fmt.sample,index,value); }
    let mut out = Surface::try_with_default_bytes(fmt,&default)?;
    let src = s.try_content_bounds_with_interrupt(control)?;
    checked_region_byte_len(src,fmt)?;
    let dst = if src.is_empty() { Rect::EMPTY } else { Rect::new(tap_coordinate((f64::from(src.x0)*sx).floor())?,tap_coordinate((f64::from(src.y0)*sy).floor())?,tap_coordinate((f64::from(src.x1)*sx).ceil())?,tap_coordinate((f64::from(src.y1)*sy).ceil())?) };
    checked_region_byte_len(dst,fmt)?;
    if dst.is_empty() {
        return Ok(out);
    }
    let n = fmt.channels();
    let alpha = fmt.alpha;
    // With `edge`, every tap is clamped into the edge area, which holds the content.
    let area = edge.unwrap_or(src);
    let mut source_retained = 0u64;
    for (_,tile) in s.tiles() { resize_poll(control)?; source_retained = source_retained.checked_add(tile.bytes().len() as u64 + 512).ok_or(RasterInputError::LimitExceeded)?; }
    let tap_allowance = MAX_CHECKED_RASTER_BYTES.checked_sub(source_retained).ok_or(RasterInputError::LimitExceeded)?;
    let hx = taps_with_interrupt(dst.x0, dst.x1, sx, filter, edge.map(|r| (r.x0, r.x1 - 1)), (area.x0, area.x1 - 1),control,tap_allowance)?;
    let hx_bytes = hx.iter().try_fold((hx.capacity() as u64)*(std::mem::size_of::<Taps>() as u64), |total,tap| total.checked_add((tap.weights.capacity() as u64)*4).ok_or(RasterInputError::LimitExceeded))?;
    let vy = taps_with_interrupt(dst.y0, dst.y1, sy, filter, edge.map(|r| (r.y0, r.y1 - 1)), (area.y0, area.y1 - 1),control,tap_allowance.checked_sub(hx_bytes).ok_or(RasterInputError::LimitExceeded)?)?;
    let x_lo = hx.iter().filter(|t| !t.weights.is_empty()).map(|t| t.start).min().unwrap_or(area.x0);
    let x_hi = hx.iter().try_fold(area.x0,|maximum,t| { let end = t.start.checked_add(i32::try_from(t.weights.len()).map_err(|_|RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?; Ok::<_,RasterInputError>(maximum.max(end)) })?.max(x_lo);
    let mut def = s.default_pixel();
    premultiply(&mut def, n, alpha);
    let dw = dst.width() as usize;
    const BAND: usize = 64;
    
    // Horizontal pass of one source row (`row` spans `x_lo..x_hi`) into `dw` samples.
    let horizontal = |row: &[f32], out: &mut [f32]| -> Result<(),RasterInputError> {
        for (tap,d) in hx.iter().zip(out.chunks_exact_mut(n)) {
            resize_poll(control)?;
            for (k,w) in tap.weights.iter().enumerate() {
                if k % 1024 == 0 { resize_poll(control)?; }
                let index = usize::try_from(i64::from(tap.start)+k as i64-i64::from(x_lo)).map_err(|_|RasterInputError::InvalidBounds)?;
                let start = index.checked_mul(n).ok_or(RasterInputError::LimitExceeded)?;
                if let Some(sp) = row.get(start..start.checked_add(n).ok_or(RasterInputError::LimitExceeded)?) {
                    for (dv,sv) in d.iter_mut().zip(sp) { *dv += sv * w; }
                }
            }
            for (index,dv) in d.iter_mut().enumerate() { *dv += def.get(index).copied().ok_or(RasterInputError::LengthMismatch)? * tap.outside; }
        }
        Ok(())
    };
    let rows_per_read = (budget / ((x_hi - x_lo).max(1) as usize * n).max(dw * n)).clamp(1, TILE_SIZE as usize) as i32;
    #[cfg(not(target_arch = "wasm32"))]
    let group = rayon::current_num_threads().max(1) * 2;
    #[cfg(target_arch = "wasm32")]
    let group = 1;
    let mut retained = source_retained;
    for tap in hx.iter().chain(&vy) { retained = retained.checked_add((tap.weights.capacity() as u64)*4 + std::mem::size_of::<Taps>() as u64).ok_or(RasterInputError::LimitExceeded)?; }
    let tile_span = |a:i32,b:i32| -> Result<u64,RasterInputError> { u64::try_from(i64::from((b-1).div_euclid(TILE_SIZE))-i64::from(a.div_euclid(TILE_SIZE))+1).map_err(|_|RasterInputError::InvalidBounds) };
    let output_tiles = tile_span(dst.x0,dst.x1)?.checked_mul(tile_span(dst.y0,dst.y1)?).ok_or(RasterInputError::LimitExceeded)?;
    let output_charge = output_tiles.checked_mul((TILE_SIZE as u64)*(TILE_SIZE as u64)*(fmt.bytes_per_pixel() as u64)+512).ok_or(RasterInputError::LimitExceeded)?;
    let band_bytes = (dw as u64).checked_mul(n as u64).and_then(|v|v.checked_mul(BAND as u64)).and_then(|v|v.checked_mul(24)).ok_or(RasterInputError::LimitExceeded)?;
    let read_samples = resize_count(x_hi-x_lo,rows_per_read,n)? as u64;
    let read_bytes = read_samples.checked_mul(16).ok_or(RasterInputError::LimitExceeded)?;
    let group_peak = band_bytes.checked_add(read_bytes).and_then(|v|v.checked_mul(group as u64)).ok_or(RasterInputError::LimitExceeded)?;
    if retained.checked_add(output_charge).and_then(|v|v.checked_add(group_peak)).is_none_or(|v|v > MAX_CHECKED_RASTER_BYTES) { return Err(RasterInputError::LimitExceeded); }
    // A source row of nothing but the default pixel, after the horizontal pass.
    let def_row = {
        let mut row = resize_vec(resize_count(x_hi-x_lo,1,n)?,0.0f32,control)?;
        for (index,value) in row.iter_mut().enumerate() { if index % 1024 == 0 { resize_poll(control)?; } *value = def.get(index%n).copied().ok_or(RasterInputError::LengthMismatch)?; }
        let mut out = resize_vec(dw.checked_mul(n).ok_or(RasterInputError::LimitExceeded)?,0.0f32,control)?;
        horizontal(&row, &mut out)?;
        out
    };
    // At most one tile row per read anyway, so this always fits an `i32`.

    let run = |&(b0, b1): &(usize, usize)| -> Result<(Rect, Vec<f32>),RasterInputError> {
        resize_poll(control)?;
        let band = vy.get(b0..b1).unwrap_or(&[]);
        let y_lo = band.iter().filter(|t| !t.weights.is_empty()).map(|t| t.start).min().unwrap_or(area.y0);
        let y_hi = band.iter().try_fold(area.y0,|maximum,t| { let end = t.start.checked_add(i32::try_from(t.weights.len()).map_err(|_|RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?; Ok::<_,RasterInputError>(maximum.max(end)) })?.max(y_lo);
        let mut res = resize_vec(band.len().checked_mul(dw).and_then(|v|v.checked_mul(n)).ok_or(RasterInputError::LimitExceeded)?,0.0f32,control)?;
        // Source rows stream through in bounded reads and go straight into the output rows that
        // use them, so memory is the read budget plus this band's output, whatever the scale or
        // the source size (#713). In ascending row order, as one read of the whole window would.
        let add = |res: &mut [f32], y: i32, row: &[f32]| -> Result<(),RasterInputError> {
            for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
                resize_poll(control)?;
                if let Some(w) = t.weights.get(usize::try_from(i64::from(y)-i64::from(t.start)).unwrap_or(usize::MAX)).filter(|_| y >= t.start) {
                    for (index,(dv, sv)) in d.iter_mut().zip(row).enumerate() {
                        if index % 1024 == 0 { resize_poll(control)?; }
                        *dv += sv * w;
                    }
                }
            }
            Ok(())
        };
        
        
        let mut y = y_lo;
        while y < y_hi {
            resize_poll(control)?;
            let y1 = i32::try_from((i64::from(y).div_euclid(i64::from(TILE_SIZE))+1)*i64::from(TILE_SIZE)).map_err(|_|RasterInputError::InvalidBounds)?.min(y_hi);
            if s.has_tiles_in(Rect::new(x_lo, y, x_hi, y1)) {
                let mut c0 = y;
                while c0 < y1 {
                    resize_poll(control)?;
                    let c1 = c0.checked_add(rows_per_read).ok_or(RasterInputError::InvalidBounds)?.min(y1);
                    let read = Rect::new(x_lo, c0, x_hi, c1);
                    let mut rows = read_resize_rows(s,read,control)?;
                    for part in rows.chunks_mut(1024*n) { resize_poll(control)?; premultiply(part,n,alpha); }
                    let mut hrows = resize_vec(usize::try_from(c1-c0).ok().and_then(|v|v.checked_mul(dw)).and_then(|v|v.checked_mul(n)).ok_or(RasterInputError::LimitExceeded)?,0.0f32,control)?;
                    let rw = read.width() as usize * n;
                    for (row, out) in rows.chunks_exact(rw.max(1)).zip(hrows.chunks_exact_mut(dw * n)) {
                        resize_poll(control)?;
                        horizontal(row, out)?;
                    }
                    for (r, h) in (c0..c1).zip(hrows.chunks_exact(dw * n)) {
                        add(&mut res, r, h)?;
                    }
                    c0 = c1;
                }
            } else {
                // Nothing stored in this tile row: every row in it is the default row, so each
                // output row takes it once, with the summed weight of those rows.
                for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
                    resize_poll(control)?;
                    let (a, b) = (usize::try_from((i64::from(y)-i64::from(t.start)).max(0)).map_err(|_|RasterInputError::InvalidBounds)?, usize::try_from((i64::from(y1)-i64::from(t.start)).max(0)).map_err(|_|RasterInputError::InvalidBounds)?);
                    let mut w=0.0f32; for value in t.weights.get(a.min(t.weights.len())..b.min(t.weights.len())).unwrap_or(&[]) { resize_poll(control)?; w += *value; }
                    if w != 0.0 {
                        for (index,(dv, sv)) in d.iter_mut().zip(&def_row).enumerate() {
                            if index % 1024 == 0 { resize_poll(control)?; }
                            *dv += sv * w;
                        }
                    }
                }
            }
            y = y1;
        }
        for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
            for (index,(dv, sv)) in d.iter_mut().zip(&def_row).enumerate() {
                if index % 1024 == 0 { resize_poll(control)?; }
                *dv += sv * t.outside;
            }
        }
        if alpha {
            for (index,px) in res.chunks_exact_mut(n).enumerate() {
                if index % 1024 == 0 { resize_poll(control)?; }
                px[n - 1] = px[n - 1].clamp(0.0, 1.0);
            }
        }
        for part in res.chunks_mut(1024*n) { resize_poll(control)?; unpremultiply(part,n,alpha); }
        Ok((Rect::new(dst.x0, dst.y0 + b0 as i32, dst.x1, dst.y0 + b1 as i32), res))
    };
    // Bands go into the output a group at a time, so the `f32` rows held at once are bounded by
    // the group, not by the whole destination (#1544).
    let mut first = 0usize;
    while first < vy.len() {
        resize_poll(control)?;
        let count = vy.len().saturating_sub(first).div_ceil(BAND).min(group);
        let mut slots = resize_vec(count,None,control)?;
        #[cfg(not(target_arch = "wasm32"))]
        { use rayon::prelude::*; slots.par_iter_mut().enumerate().for_each(|(index,slot)| {
            let b0 = first + index * BAND; *slot = Some(run(&(b0,(b0+BAND).min(vy.len())))); }); }
        #[cfg(target_arch = "wasm32")]
        for (index,slot) in slots.iter_mut().enumerate() { let b0 = first + index * BAND; *slot = Some(run(&(b0,(b0+BAND).min(vy.len())))); }
        for slot in slots { resize_poll(control)?; let (r,d) = slot.ok_or(RasterInputError::LengthMismatch)??; write_resize_rows(&mut out,r,&d,control)?; }
        first = first.checked_add(count.checked_mul(BAND).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    }
    if filter == Resample::PreserveDetails && (sx > 1.0 || sy > 1.0) {
        let p = crate::FilterParams::UnsharpMask { amount: 30.0, radius: 0.6 * sx.max(sy) as f32, threshold: 0.0 };
        let area = crate::output_area(&p, out.try_content_bounds_with_interrupt(control)?, dst, None);
        out = crate::apply_tiled_with(&out,&p,area,dst,None,crate::auto_tile(&p,dst),None,control).ok_or_else(|| if control.check().is_err() { RasterInputError::Cancelled } else { RasterInputError::AllocationFailed })?;
    }
    out.try_prune_with(control)?;
    resize_poll(control)?;
    Ok(out)
}

/// Moves a surface by whole pixels (default pixel kept).
pub fn translate_surface(s: &Surface, dx: i32, dy: i32) -> Surface {
    s.translated(dx, dy, s.content_bounds())
}

/// Keeps only the pixels inside `keep` (others become the default pixel).
pub fn crop_surface(s: &Surface, keep: Rect) -> Surface {
    let mut out = s.empty_like();
    let r = s.content_bounds().intersect(&keep);
    if !r.is_empty() {
        out.write_interleaved(r, &s.to_interleaved(r));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    fn ramp(sample: SampleType) -> Surface {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        let r = Rect::new(0, 0, 64, 32);
        let v: Vec<f32> = (0..32).flat_map(|_| (0..64).flat_map(|x| [x as f32 / 63.0, 0.5, 1.0 - x as f32 / 63.0, 1.0])).collect();
        s.write_region(r, &v);
        s
    }

    #[test]
    fn controlled_legacy_resize_matches_native_quantization_and_cancel_refuses() {
        for sample in [SampleType::U8,SampleType::U16,SampleType::F32] {
            let source=ramp(sample); let before=source.clone(); let canvas=Rect::new(0,0,64,32);
            for filter in [Resample::Nearest,Resample::Bilinear,Resample::Bicubic,Resample::Lanczos,Resample::PreserveDetails] {
                for (sx,sy) in [(0.17,0.43),(1.0,1.0),(1.7,2.3)] {
                    let old=resize_surface_in_canvas(&source,sx,sy,filter,canvas);
                    let controlled=try_resize_surface_in_canvas_with_interrupt(&source,sx,sy,filter,canvas,&Interrupt::NONE).unwrap();
                    assert_eq!(controlled,old,"{sample:?} {filter:?} {sx} {sy}");
                }
            }
            let cancel=||true;
            assert_eq!(try_resize_surface_in_canvas_with_interrupt(&source,2.0,2.0,Resample::Bicubic,canvas,&Interrupt::cancel_only(&cancel)),Err(RasterInputError::Cancelled));
            for scale in [0.0,-1.0,f64::NAN,f64::INFINITY,1e100] {
                assert!(try_resize_surface_in_canvas_with_interrupt(&source,scale,1.0,Resample::Bicubic,canvas,&Interrupt::NONE).is_err());
            }
            assert_eq!(source,before);
        }
    }
    #[test]
    fn fixed_channel_rows_match_dynamic_resampling() {
        fn check<const N: usize>() {
            for scale in [0.12, 0.5, 1.0, 2.3] {
                for filter in [Resample::Nearest, Resample::Bilinear, Resample::Bicubic, Resample::Lanczos] {
                    for clamp in [None, Some((-3, 31))] {
                        let ts = taps(-5, 41, scale, filter, clamp, (-3, 31));
                        let row: Vec<f32> = (0..35 * N).map(|i| ((i * 73) % 251) as f32 / 250.0).collect();
                        let def: Vec<f32> = (0..N).map(|i| i as f32 / 7.0).collect();
                        let mut expected = vec![0.0; ts.len() * N];
                        let mut actual = expected.clone();
                        horizontal_dynamic(&row, &mut expected, &ts, -3, &def, N);
                        horizontal_fixed::<N>(&row, &mut actual, &ts, -3, &def);
                        assert_eq!(actual, expected, "N={N} scale={scale} filter={filter:?} clamp={clamp:?}");
                    }
                }
            }
        }
        check::<1>();
        check::<2>();
        check::<3>();
        check::<4>();
        check::<5>();
    }

    #[test]
    fn identity_scale_is_lossless() {
        for f in [Resample::Nearest, Resample::Bilinear, Resample::Bicubic, Resample::Lanczos] {
            let s = ramp(SampleType::U16);
            let o = resize_surface(&s, 1.0, 1.0, f);
            let r = Rect::new(0, 0, 64, 32);
            let (a, b) = (s.read_region(r), o.read_region(r));
            let d = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
            assert!(d < 1e-4, "{f:?} {d}");
        }
    }

    #[test]
    fn half_and_double_sizes() {
        for f in [Resample::Nearest, Resample::Bilinear, Resample::Bicubic, Resample::Lanczos, Resample::PreserveDetails] {
            let s = ramp(SampleType::F32);
            let half = resize_surface(&s, 0.5, 0.5, f);
            assert_eq!(half.content_bounds(), Rect::new(0, 0, 32, 16), "{f:?}");
            let p = half.pixel(16, 8);
            assert!((p[1] - 0.5).abs() < 0.02 && (p[3] - 1.0).abs() < 1e-3, "{f:?} {p:?}");
            let dbl = resize_surface(&s, 2.0, 2.0, f);
            assert_eq!(dbl.content_bounds(), Rect::new(0, 0, 128, 64), "{f:?}");
            // Mid-gray channel and the ramp's monotonicity are preserved.
            let row: Vec<f32> = (8..120).map(|x| dbl.pixel(x, 30)[0]).collect();
            assert!(row.windows(2).all(|w| w[1] >= w[0] - 0.02), "{f:?}");
        }
    }

    #[test]
    fn downscale_averages_checkerboard() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        for y in 0..64 {
            for x in 0..64 {
                let v = ((x + y) % 2) as f32;
                s.write_pixel(x, y, &[v, v, v, 1.0]);
            }
        }
        let o = resize_surface(&s, 0.25, 0.25, Resample::Bicubic);
        let p = o.pixel(8, 8);
        assert!((p[0] - 0.5).abs() < 0.05, "{p:?}");
    }

    #[test]
    fn transparent_edges_do_not_darken() {
        let mut s = Surface::new(PixelFormat::RGBA32F);
        s.fill_rect(Rect::new(10, 10, 20, 20), &[1.0, 0.0, 0.0, 1.0]);
        let o = resize_surface(&s, 1.7, 1.7, Resample::Lanczos);
        let edge = o.pixel(17, 25);
        assert!(edge[3] > 0.0 && (edge[0] - 1.0).abs() < 0.05 && edge[1].abs() < 0.05, "{edge:?}");
    }

    /// Image Size of a layer that fills the canvas: its edges stay opaque and keep their colour,
    /// up or down, at every depth; a layer away from the edges keeps its soft border.
    #[test]
    fn canvas_edges_repeat_instead_of_fading() {
        let canvas = Rect::new(0, 0, 30, 20);
        let colour = [0.2, 0.6, 1.0, 1.0];
        let filters = [Resample::Nearest, Resample::Bilinear, Resample::Bicubic, Resample::Lanczos, Resample::PreserveDetails];
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut s = Surface::new(fmt);
            s.fill_rect(canvas, &colour);
            for f in filters {
                for k in [2.0, 1.37, 0.5] {
                    let o = resize_surface_in_canvas(&s, k, k, f, canvas);
                    let dst = scaled_rect(canvas, k, k);
                    assert_eq!(o.content_bounds(), dst, "{sample:?} {f:?} {k}");
                    for (x, y) in [(0, 0), (dst.x1 - 1, 0), (0, dst.y1 - 1), (dst.x1 - 1, dst.y1 - 1), (dst.x1 / 2, 0), (0, dst.y1 / 2)] {
                        let p = o.pixel(x, y);
                        let off = p.iter().zip(colour).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                        assert!(off < 0.01, "{sample:?} {f:?} {k} ({x},{y}): {p:?}");
                    }
                }
            }
            // Inside the canvas the default pixel is still what lies around a layer.
            let mut inner = Surface::new(fmt);
            inner.fill_rect(Rect::new(10, 5, 20, 15), &[1.0, 0.0, 0.0, 1.0]);
            let r = Rect::new(0, 0, 60, 40);
            let a = resize_surface_in_canvas(&inner, 1.7, 1.7, Resample::Lanczos, canvas);
            assert_eq!(a.read_region(r), resize_surface(&inner, 1.7, 1.7, Resample::Lanczos).read_region(r), "{sample:?}");
        }
    }

    /// Two tiles in opposite corners of a 100000 px square: the content bounds are huge but
    /// almost nothing is stored. Shrinking it to one pixel must neither allocate a window
    /// proportional to the scale (#713) nor read the empty middle.
    #[test]
    fn tiny_target_of_a_huge_sparse_source() {
        let mut s = Surface::new(PixelFormat::RGBA32F);
        s.fill_rect(Rect::new(0, 0, 256, 256), &[1.0, 1.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(99_840, 99_840, 100_096, 100_096), &[1.0, 1.0, 1.0, 1.0]);
        let k = 1.0 / 100_096.0;
        let o = resize_surface(&s, k, k, Resample::Bicubic);
        assert_eq!(o.content_bounds(), Rect::new(0, 0, 1, 1));
        let p = o.pixel(0, 0);
        // Two 256² tiles out of 100096²: nearly transparent, and white where it isn't.
        assert!(p[3] >= 0.0 && p[3] < 1e-3 && p.iter().all(|v| v.is_finite()), "{p:?}");
        // Squashed to one row but kept wide: the whole height feeds one band of output rows,
        // which must not hold a full-height intermediate.
        let o = resize_surface(&s, 1000.0 / 100_096.0, k, Resample::Bicubic);
        let b = o.content_bounds();
        assert!(b.height() == 1 && (1000..=1001).contains(&b.width()), "{b:?}");
        assert!(o.pixel(0, 0)[3] > 0.0 && o.pixel(500, 0)[3] == 0.0, "{:?} {:?}", o.pixel(0, 0), o.pixel(500, 0));
    }

    /// Tile rows with nothing stored are not read; they count as rows of the default pixel,
    /// exactly as reading them would (and within rounding for a non-zero default).
    #[test]
    fn skipped_empty_rows_match_reading_them() {
        for (fmt, def, tol) in [(PixelFormat::RGBA32F, vec![0.0, 0.0, 0.0, 0.0], 0.0), (PixelFormat::GRAY8, vec![1.0], 1e-6)] {
            let mut sparse = Surface::with_default(fmt, &def);
            sparse.fill_rect(Rect::new(0, 0, 300, 40), &vec![0.5; def.len()]);
            sparse.fill_rect(Rect::new(100, 1500, 400, 1540), &vec![0.25; def.len()]);
            // The same pixels with the empty rows stored as tiles of the default pixel.
            let mut dense = sparse.clone();
            dense.fill_rect(Rect::new(0, 40, 400, 1500), &def);
            assert!(dense.tile_count() > sparse.tile_count());
            for f in [Resample::Bilinear, Resample::Bicubic, Resample::Lanczos] {
                for (kx, ky) in [(0.013, 0.013), (0.37, 0.37), (1.0, 0.01)] {
                    let (a, b) = (resize_surface(&sparse, kx, ky, f), resize_surface(&dense, kx, ky, f));
                    let r = a.content_bounds().union(&b.content_bounds());
                    let d = a.read_region(r).iter().zip(b.read_region(r)).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
                    assert!(d <= tol, "{fmt:?} {f:?} {kx} {ky}: {d}");
                }
            }
        }
    }

    /// Reading the source in row chunks gives exactly what one read of the whole window gives.
    #[test]
    fn chunked_reads_match_a_single_read() {
        let mut s = ramp(SampleType::F32);
        s.fill_rect(Rect::new(600, 700, 640, 720), &[0.3, 0.9, 0.1, 0.5]);
        let mut mask = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
        mask.fill_rect(Rect::new(0, 0, 40, 30), &[0.0]);
        mask.fill_rect(Rect::new(500, 900, 530, 940), &[0.25]);
        for src in [&s, &mask] {
            for f in [Resample::Nearest, Resample::Bilinear, Resample::Bicubic, Resample::Lanczos] {
                for k in [0.013, 0.37, 1.6] {
                    let whole = resize_with_budget(src, k, k, f, None, usize::MAX);
                    let chunked = resize_with_budget(src, k, k, f, None, 1);
                    let r = whole.content_bounds().union(&chunked.content_bounds());
                    assert_eq!(whole.read_region(r), chunked.read_region(r), "{f:?} {k}");
                }
            }
        }
    }

    #[test]
    fn masks_keep_default_pixel() {
        let fmt = PixelFormat::GRAY8;
        let mut m = Surface::with_default(fmt, &[1.0]);
        m.fill_rect(Rect::new(0, 0, 10, 10), &[0.0]);
        let o = resize_surface(&m, 2.0, 2.0, Resample::Bilinear);
        assert_eq!(o.default_pixel(), vec![1.0]);
        assert!(o.pixel(5, 5)[0] < 0.01);
    }

    #[test]
    fn translate_and_crop() {
        let s = ramp(SampleType::U8);
        let t = translate_surface(&s, 5, -3);
        assert_eq!(t.pixel(15, 7), s.pixel(10, 10));
        let c = crop_surface(&s, Rect::new(0, 0, 10, 10));
        assert_eq!(c.content_bounds(), Rect::new(0, 0, 10, 10));
    }

    /// #1114: nearest neighbor reduction must sample single source pixels, never average.
    #[test]
    fn nearest_neighbor_reduction_does_not_average() {
        let mut s = Surface::new(PixelFormat::GRAY8);
        for y in 0..8 {
            for x in 0..8 {
                if (x + y) % 2 == 0 {
                    s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[1.0]);
                }
            }
        }
        let reduced = resize_surface(&s, 0.5, 0.5, Resample::Nearest);
        for y in 0..4 {
            for x in 0..4 {
                let p = reduced.pixel(x, y)[0];
                assert!(p == 0.0 || p == 1.0, "nearest produced an averaged value {p} at ({x}, {y})");
            }
        }
    }
}
