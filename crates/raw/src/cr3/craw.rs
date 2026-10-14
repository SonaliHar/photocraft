//! Legacy CRX C-RAW: independent entropy reconstruction and reversible 5/3
//! synthesis from public lifting equations and measured CC0 sensor streams.
//! No reference decoder implementation or camera profile was copied.
use super::{
    CrxTrack,
    lossless::{Bits, adapt, marker, med, run},
};
use crate::{Limits, RawError};

const COEFFICIENT_LIMIT: i32 = 1 << 26;
fn bad() -> RawError {
    RawError::malformed("invalid CRX C-RAW sensor stream")
}
pub(super) fn bounded(value: i64) -> Result<i32, RawError> {
    if value.abs() > i64::from(COEFFICIENT_LIMIT) {
        return Err(bad());
    }
    i32::try_from(value).map_err(|_| bad())
}
pub(super) fn zeros<T: Default + Clone>(count: usize) -> Result<Vec<T>, RawError> {
    let mut v = Vec::new();
    v.try_reserve_exact(count).map_err(|_| RawError::LimitExceeded("CRX wavelet allocation".into()))?;
    v.resize(count, T::default());
    Ok(v)
}
pub(super) struct Band {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) values: Vec<i32>,
}
impl Band {
    fn row(&self, y: usize) -> Result<&[i32], RawError> {
        let at = y.checked_mul(self.width).ok_or_else(bad)?;
        self.values.get(at..at.checked_add(self.width).ok_or_else(bad)?).ok_or_else(bad)
    }
}
// The legacy uniform quantizers verified on the public and local camera files.
// Adaptive v0x200 quantization is a separate, explicitly unsupported variant.
fn quantizer(q: u32) -> Result<i32, RawError> {
    match q {
        4 => Ok(1),
        16 => Ok(4),
        22 => Ok(8),
        26 => Ok(12),
        32 => Ok(25),
        _ => Err(RawError::unsupported("CRX C-RAW uniform quantizer")),
    }
}
pub(super) fn entropy(data: &[u8], width: usize, height: usize, low: bool, step: i32) -> Result<Band, RawError> {
    if width == 0 || height == 0 {
        return Err(bad());
    }
    let mut bits = Bits { bytes: data, pos: 0 };
    let mut k = 0;
    let mut index = 0;
    let mut prev = zeros::<i32>(width)?;
    let mut row = zeros::<i32>(width)?;
    let mut prev_k = zeros::<u32>(width)?;
    let mut row_k = zeros::<u32>(width)?;
    let mut values = Vec::new();
    for y in 0..height {
        let mut x = 0;
        let mut interrupted = false;
        while x < width {
            let a = if x == 0 { *prev.first().ok_or_else(bad)? } else { *row.get(x - 1).ok_or_else(bad)? };
            let b = *prev.get(x).ok_or_else(bad)?;
            let c = if x == 0 { b } else { *prev.get(x - 1).ok_or_else(bad)? };
            let d = *prev.get((x + 1).min(width - 1)).ok_or_else(bad)?;
            let flat = x + 1 < width
                && if y == 0 {
                    a == 0
                } else if low {
                    a == b && b == d
                } else {
                    a == 0 && b == 0 && d == 0
                };
            if !interrupted && flat && bits.read(1)? == 1 {
                let n = run(&mut bits, &mut index, width - x)?;
                let end = x.checked_add(n).ok_or_else(bad)?;
                row.get_mut(x..end).ok_or_else(bad)?.fill(a);
                // Runs have zero lookahead context, irrespective of current k.
                row_k.get_mut(x..end).ok_or_else(bad)?.fill(0);
                x = end;
                interrupted = true;
                continue;
            }
            let mapped = bits.rice(k)?;
            let signed = mapped.checked_add(u32::from(!low && (flat || interrupted))).ok_or_else(bad)?;
            let delta = if signed & 1 == 0 { (signed / 2) as i32 } else { -((signed / 2) as i32) - 1 };
            let prediction = if !low {
                0
            } else if y == 0 {
                a
            } else if interrupted {
                b
            } else {
                med(a, b, c)
            };
            *row.get_mut(x).ok_or_else(bad)? = bounded(i64::from(prediction) + i64::from(delta))?;
            let scale = if !low || y == 0 || x + 1 == width { mapped } else { (mapped / 2).checked_add((d - b).unsigned_abs()).ok_or_else(bad)? };
            k = adapt(k, scale);
            if !low && y > 0 && x + 1 < width && *prev_k.get(x + 1).ok_or_else(bad)? > k + 1 {
                k += 1;
            }
            *row_k.get_mut(x).ok_or_else(bad)? = k;
            interrupted = false;
            x += 1;
        }
        // Reserve only after verifying a complete entropy row.
        values.try_reserve(width).map_err(|_| RawError::LimitExceeded("CRX wavelet band allocation".into()))?;
        for &v in &row {
            values.push(bounded(i64::from(v) * i64::from(step))?);
        }
        std::mem::swap(&mut row, &mut prev);
        std::mem::swap(&mut row_k, &mut prev_k);
    }
    if data.len().saturating_mul(8).saturating_sub(bits.pos) > 7 {
        return Err(RawError::malformed("CRX C-RAW band has undecoded entropy bytes"));
    }
    Ok(Band { width, height, values })
}
pub(super) fn axis(mut n: usize, left: bool, right: bool) -> Result<[(usize, usize); 3], RawError> {
    let mut result = [(0, 0); 3];
    for (stage, dims) in result.iter_mut().enumerate() {
        let base = n.div_ceil(2);
        let low = base.checked_add(usize::from(right) * (1 + usize::from(stage < 2 && !base.is_multiple_of(2)))).ok_or_else(bad)?;
        let high = (n / 2).checked_add(usize::from(left)).and_then(|v| v.checked_add(usize::from(right) * (1 + n % 2))).ok_or_else(bad)?;
        *dims = (low, high);
        n = base;
    }
    Ok(result)
}
pub(super) fn crop(band: &Band, width: usize, height: usize, left: bool, top: bool) -> Result<Band, RawError> {
    let extra_x = band.width.checked_sub(width).ok_or_else(bad)?;
    let extra_y = band.height.checked_sub(height).ok_or_else(bad)?;
    let x = usize::from(left).min(extra_x);
    let y = usize::from(top).min(extra_y);
    let count = width.checked_mul(height).ok_or_else(bad)?;
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| RawError::LimitExceeded("CRX wavelet crop allocation".into()))?;
    for j in 0..height {
        values.extend_from_slice(band.row(j + y)?.get(x..x.checked_add(width).ok_or_else(bad)?).ok_or_else(bad)?);
    }
    Ok(Band { width, height, values })
}
// Whole-sample symmetric extension, preserving integer rounding for negative
// coefficients. Tile overlap puts the first high sample before the first low.
fn inverse(low: &[i32], high: &[i32], first_high: bool, out: &mut [i32]) -> Result<(), RawError> {
    let n = low.len().checked_add(high.len()).ok_or_else(bad)?;
    if low.is_empty()
        || high.is_empty()
        || out.len() != n
        || (if first_high { high.len() < low.len() || high.len() - low.len() > 1 } else { low.len() < high.len() || low.len() - high.len() > 1 })
    {
        return Err(bad());
    }
    let shift = usize::from(first_high);
    for (i, &v) in low.iter().enumerate() {
        let a = if first_high { i } else { i.saturating_sub(1) };
        let b = if first_high { i + 1 } else { i };
        let a = i64::from(*high.get(a.min(high.len() - 1)).ok_or_else(bad)?);
        let b = i64::from(*high.get(b.min(high.len() - 1)).ok_or_else(bad)?);
        *out.get_mut(i.checked_mul(2).and_then(|v| v.checked_add(shift)).ok_or_else(bad)?).ok_or_else(bad)? = bounded(i64::from(v) - ((a + b + 2) >> 2))?;
    }
    for (i, &v) in high.iter().enumerate() {
        let a = if first_high { i.saturating_sub(1) } else { i };
        let b = if first_high { i } else { i + 1 };
        let a = i64::from(*out.get(a.min(low.len() - 1) * 2 + shift).ok_or_else(bad)?);
        let b = i64::from(*out.get(b.min(low.len() - 1) * 2 + shift).ok_or_else(bad)?);
        *out.get_mut(i.checked_mul(2).and_then(|v| v.checked_add(1 - shift)).ok_or_else(bad)?).ok_or_else(bad)? = bounded(i64::from(v) + ((a + b) >> 1))?;
    }
    Ok(())
}
pub(super) fn synth(ll: Band, hl: Band, lh: Band, hh: Band, left: bool, top: bool) -> Result<Band, RawError> {
    let lw = lh.width;
    let lhgt = hl.height;
    let hw = hl.width;
    let hhgt = lh.height;
    if (hh.width, hh.height) != (hw, hhgt) {
        return Err(bad());
    }
    let ll = crop(&ll, lw, lhgt, left, top)?;
    let width = lw.checked_add(hw).ok_or_else(bad)?;
    let height = lhgt.checked_add(hhgt).ok_or_else(bad)?;
    let mut horizontal = zeros::<i32>(width.checked_mul(height).ok_or_else(bad)?)?;
    for y in 0..height {
        let at = y.checked_mul(width).ok_or_else(bad)?;
        let out = horizontal.get_mut(at..at.checked_add(width).ok_or_else(bad)?).ok_or_else(bad)?;
        if y < lhgt {
            inverse(ll.row(y)?, hl.row(y)?, left, out)?;
        } else {
            inverse(lh.row(y - lhgt)?, hh.row(y - lhgt)?, left, out)?;
        }
    }
    // Consume/drop all four bands before allocating the vertical result.
    drop((ll, hl, lh, hh));
    let mut values = zeros::<i32>(width.checked_mul(height).ok_or_else(bad)?)?;
    let mut lows = zeros::<i32>(lhgt)?;
    let mut highs = zeros::<i32>(hhgt)?;
    let mut column = zeros::<i32>(height)?;
    for x in 0..width {
        for (y, v) in lows.iter_mut().enumerate() {
            *v = *horizontal.get(y * width + x).ok_or_else(bad)?;
        }
        for (y, v) in highs.iter_mut().enumerate() {
            *v = *horizontal.get((y + lhgt) * width + x).ok_or_else(bad)?;
        }
        inverse(&lows, &highs, top, &mut column)?;
        for (y, &v) in column.iter().enumerate() {
            *values.get_mut(y * width + x).ok_or_else(bad)? = v;
        }
    }
    Ok(Band { width, height, values })
}
struct EncodedBand<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    step: i32,
}
struct Segment<'a> {
    x: usize,
    width: usize,
    height: usize,
    plane: usize,
    left: bool,
    bands: Vec<EncodedBand<'a>>,
}
fn plane(segment: &Segment<'_>, bits: u8) -> Result<Vec<u16>, RawError> {
    let mut bands = Vec::new();
    for (i, b) in segment.bands.iter().enumerate() {
        bands.push(entropy(b.data, b.width, b.height, i == 0, b.step)?);
    }
    let mut it = bands.into_iter();
    let mut ll = it.next().ok_or_else(bad)?;
    for _ in 0..3 {
        ll = synth(ll, it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?, segment.left, false)?;
    }
    if it.next().is_some() {
        return Err(bad());
    }
    let ll = crop(&ll, segment.width, segment.height, segment.left, false)?;
    let midpoint = 1i64.checked_shl(u32::from(bits).checked_sub(1).ok_or_else(bad)?).ok_or_else(bad)?;
    let maximum = (1i64.checked_shl(u32::from(bits)).ok_or_else(bad)?) - 1;
    let mut result = Vec::new();
    result.try_reserve_exact(ll.values.len()).map_err(|_| RawError::LimitExceeded("CRX C-RAW plane allocation".into()))?;
    for v in ll.values {
        result.push(u16::try_from((i64::from(v) + midpoint).clamp(0, maximum)).map_err(|_| bad())?);
    }
    Ok(result)
}
pub(super) fn samples(track: &CrxTrack<'_>, limits: &Limits) -> Result<Vec<u16>, RawError> {
    if track.version != 0x100 || track.encoding != 0 || track.wavelet_levels != 3 || track.planes != 4 || !(8..=16).contains(&track.bits) {
        return Err(RawError::unsupported("CRX C-RAW coding variant"));
    }
    let (width, height, tw, th) = (track.width as usize, track.height as usize, track.tile_width as usize, track.tile_height as usize);
    if [width, height, tw, th].iter().any(|v| *v == 0 || !v.is_multiple_of(2)) {
        return Err(RawError::unsupported("CRX C-RAW Bayer geometry"));
    }
    let cols = width.div_ceil(tw);
    // Vertical and middle-tile overlap have no real-camera oracle yet.
    if cols > 2 || height > th {
        return Err(RawError::unsupported("CRX C-RAW tile layout (one or two horizontal tiles supported)"));
    }
    limits.check(u64::from(track.width), u64::from(track.height), 2)?;
    let batch_size = if cfg!(target_arch = "wasm32") { 1 } else { 2 };
    let peak = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(2)
        .saturating_add((tw.min(width) as u64 + 16).saturating_mul(height as u64 + 16).saturating_mul(6 * batch_size as u64));
    if peak > limits.max_alloc {
        return Err(RawError::LimitExceeded("CRX C-RAW working memory".into()));
    }
    let header = track.sample.get(..track.header_size as usize).ok_or_else(bad)?;
    let mut pos = 0;
    let mut payload = track.header_size as usize;
    let mut segments = Vec::new();
    for tx in 0..cols {
        let (tile_size, flags) = marker(header, &mut pos, 0xff01)?;
        if flags != (tx as u32) << 16 {
            return Err(RawError::unsupported("CRX C-RAW tile flags"));
        }
        let tile_end = payload.checked_add(tile_size).ok_or_else(bad)?;
        let x = tx.checked_mul(tw).ok_or_else(bad)?;
        let pw = width.checked_sub(x).ok_or_else(bad)?.min(tw) / 2;
        let ph = height / 2;
        if pw < 8 || ph < 8 {
            return Err(RawError::unsupported("CRX C-RAW small tile geometry"));
        }
        let ax = axis(pw, tx > 0, tx + 1 < cols)?;
        let ay = axis(ph, false, false)?;
        for p in 0..4 {
            let (size, flags) = marker(header, &mut pos, 0xff02)?;
            if flags != ((p as u32) << 28) | 0x0800_0000 {
                return Err(RawError::unsupported("CRX C-RAW plane flags"));
            }
            let plane_end = payload.checked_add(size).ok_or_else(bad)?;
            if plane_end > tile_end {
                return Err(bad());
            }
            let mut bands = Vec::new();
            for b in 0..10 {
                let (size, flags) = marker(header, &mut pos, 0xff03)?;
                if flags >> 28 != b {
                    return Err(bad());
                }
                let padding = (flags & 0x7ffff) as usize;
                if padding > 7 || padding >= size {
                    return Err(bad());
                }
                let step = quantizer((flags >> 19) & 255)?;
                let depth = if b < 4 {
                    2
                } else if b < 7 {
                    1
                } else {
                    0
                };
                let &(lx, hx) = ax.get(depth).ok_or_else(bad)?;
                let &(ly, hy) = ay.get(depth).ok_or_else(bad)?;
                let bw = if b == 0 || b % 3 == 2 { lx } else { hx };
                let bh = if b == 0 || b % 3 == 1 { ly } else { hy };
                let end = payload.checked_add(size).ok_or_else(bad)?;
                if end > plane_end {
                    return Err(bad());
                }
                let data = track.sample.get(payload..end.checked_sub(padding).ok_or_else(bad)?).ok_or_else(bad)?;
                bands.push(EncodedBand { data, width: bw, height: bh, step });
                payload = end;
            }
            if payload != plane_end {
                return Err(bad());
            }
            segments.push(Segment { x, width: pw, height: ph, plane: p, left: tx > 0, bands });
        }
        if payload != tile_end {
            return Err(bad());
        }
    }
    if payload != track.sample.len() || header.get(pos..).is_none_or(|v| v.len() > 7 || v.iter().any(|b| *b != 0)) {
        return Err(bad());
    }
    let count = width.checked_mul(height).ok_or_else(bad)?;
    let mut output = Vec::new();
    for batch in segments.chunks(batch_size) {
        let decoded = crate::par::map(batch.len(), |i| plane(batch.get(i).ok_or_else(bad)?, track.bits));
        for (segment, result) in batch.iter().zip(decoded) {
            let data = result.map_err(|e| match e {
                RawError::Malformed(m) => RawError::malformed(format!("CRX C-RAW tile x={} plane {}: {m}", segment.x, segment.plane)),
                other => other,
            })?;
            if output.is_empty() {
                output = zeros::<u16>(count)?;
            }
            for (y, row) in data.chunks_exact(segment.width).enumerate() {
                let ty = y.checked_mul(2).and_then(|v| v.checked_add(segment.plane >> 1)).ok_or_else(bad)?;
                for (x, &v) in row.iter().enumerate() {
                    let tx = x.checked_mul(2).and_then(|v| v.checked_add(segment.x)).and_then(|v| v.checked_add(segment.plane & 1)).ok_or_else(bad)?;
                    let at = ty.checked_mul(width).and_then(|v| v.checked_add(tx)).ok_or_else(bad)?;
                    *output.get_mut(at).ok_or_else(bad)? = v;
                }
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifting_rounding_and_boundary_phases() {
        let mut out = [0; 5];
        inverse(&[3, -2, 7], &[-3, 4], false, &mut out).unwrap();
        assert_eq!(out, [4, -2, -2, 5, 5]);
        inverse(&[3, -2], &[-3, 4, -1], true, &mut out).unwrap();
        assert_eq!(out, [0, 3, 4, -3, -4]);
        assert!(inverse(&[], &[], false, &mut []).is_err());
        assert!(inverse(&[0, 0, 0], &[0], false, &mut [0; 4]).is_err());
        assert!(bounded(i64::from(COEFFICIENT_LIMIT) + 1).is_err());
    }
    #[test]
    fn overlap_dimensions_follow_each_level_parity() {
        assert_eq!(axis(1782, false, true).unwrap(), [(893, 892), (447, 447), (224, 224)]);
        assert_eq!(axis(1782, true, false).unwrap(), [(891, 892), (446, 446), (223, 224)]);
        assert_eq!(axis(1572, false, true).unwrap(), [(787, 787), (395, 394), (198, 198)]);
    }
    #[test]
    fn entropy_and_uniform_quantization_reject_unverified_or_broken_streams() {
        assert_eq!(entropy(&[0xfe], 4, 2, false, 25).unwrap().values, [0; 8]);
        assert!(entropy(&[], 4, 2, false, 1).is_err());
        assert!(entropy(&[0xfe, 0], 4, 2, false, 1).is_err());
        assert!(entropy(&[0; 16], 4, 2, false, 1).is_err());
        assert_eq!(quantizer(32).unwrap(), 25);
        assert!(matches!(quantizer(255), Err(RawError::Unsupported(_))));
    }
}
