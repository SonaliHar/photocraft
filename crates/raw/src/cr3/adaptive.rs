//! CRX 0x200 adaptive C-RAW, independently measured from CC0 originals and
//! black-box sensor output. Header extensions carry a shared quantization grid;
//! coefficient entropy and integer 5/3 lifting reuse the legacy implementation.
//! See docs/cr3.md: no third-party decoder source or camera table was copied.
use super::{
    CrxTrack, craw,
    lossless::{Bits, adapt, med},
};
use crate::{Limits, RawError};

fn bad() -> RawError {
    RawError::malformed("invalid CRX adaptive C-RAW stream")
}
fn u16_at(bytes: &[u8], at: usize) -> Result<u16, RawError> {
    Ok(u16::from_be_bytes(bytes.get(at..at.checked_add(2).ok_or_else(bad)?).ok_or_else(bad)?.try_into().map_err(|_| bad())?))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32, RawError> {
    Ok(u32::from_be_bytes(bytes.get(at..at.checked_add(4).ok_or_else(bad)?).ok_or_else(bad)?.try_into().map_err(|_| bad())?))
}
fn marker<'a>(header: &'a [u8], pos: &mut usize, kind: u16, length: u16) -> Result<(usize, &'a [u8]), RawError> {
    if u16_at(header, *pos)? != kind || u16_at(header, pos.checked_add(2).ok_or_else(bad)?)? != length {
        return Err(bad());
    }
    let size = u32_at(header, pos.checked_add(4).ok_or_else(bad)?)? as usize;
    let end = pos.checked_add(4 + usize::from(length)).ok_or_else(bad)?;
    let fields = header.get(pos.checked_add(8).ok_or_else(bad)?..end).ok_or_else(bad)?;
    *pos = end;
    Ok((size, fields))
}
struct Grid {
    width: usize,
    height: usize,
    values: Vec<i32>,
}
impl Grid {
    fn qp(&self, x: usize, y: usize, level: usize) -> Result<u32, RawError> {
        if !(1..=3).contains(&level) || self.width == 0 || self.height == 0 {
            return Err(bad());
        }
        let rows = 1usize << (level - 1);
        let column = x / (8usize >> level);
        if column >= self.width {
            return Err(bad());
        }
        let first = y.checked_mul(rows).ok_or_else(bad)?;
        let mut sum = 0i32;
        for j in 0..rows {
            let row = first.checked_add(j).ok_or_else(bad)?.min(self.height - 1);
            let at = row.checked_mul(self.width).and_then(|v| v.checked_add(column)).ok_or_else(bad)?;
            sum = sum.checked_add(*self.values.get(at).ok_or_else(bad)?).ok_or_else(bad)?;
        }
        // Average indices before exponentiation, rounding down, and replicate
        // the final grid row for an odd-height last decomposition block.
        u32::try_from(sum / rows as i32).map_err(|_| bad())
    }
}
fn qp_rice(bits: &mut Bits<'_>, k: u32) -> Result<u32, RawError> {
    let mut zeros = 0u32;
    while bits.read(1)? == 0 {
        zeros += 1;
        if zeros > 23 {
            return Err(bad());
        }
    }
    if zeros == 23 { bits.read(8) } else { Ok(zeros.checked_shl(k).ok_or_else(bad)? | bits.read(k)?) }
}
fn grid(data: &[u8], width: usize, height: usize) -> Result<Grid, RawError> {
    if width == 0 || height == 0 {
        return Err(bad());
    }
    let mut bits = Bits { bytes: data, pos: 0 };
    let mut k = 0;
    let mut prev = craw::zeros::<i32>(width)?;
    let mut row = craw::zeros::<i32>(width)?;
    let mut values = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let a = if x == 0 { *prev.first().ok_or_else(bad)? } else { *row.get(x - 1).ok_or_else(bad)? };
            let b = *prev.get(x).ok_or_else(bad)?;
            let c = if x == 0 { b } else { *prev.get(x - 1).ok_or_else(bad)? };
            let d = *prev.get((x + 1).min(width - 1)).ok_or_else(bad)?;
            // The QP grid has no MEL flags. Its shorter Rice escape is an
            // independently verified 23-zero prefix and an eight-bit symbol.
            let mapped = qp_rice(&mut bits, k)?;
            let delta = if mapped & 1 == 0 { (mapped / 2) as i32 } else { -((mapped / 2) as i32) - 1 };
            let prediction = if y == 0 { a } else { med(a, b, c) };
            let value = prediction.checked_add(delta).ok_or_else(bad)?;
            if !(0..=51).contains(&value) {
                return Err(bad());
            }
            *row.get_mut(x).ok_or_else(bad)? = value;
            let scale = if y == 0 || x + 1 == width { mapped } else { (mapped / 2).checked_add((d - b).unsigned_abs()).ok_or_else(bad)? };
            k = adapt(k, scale);
        }
        values.try_reserve(width).map_err(|_| RawError::LimitExceeded("CRX quantization grid allocation".into()))?;
        values.extend_from_slice(&row);
        std::mem::swap(&mut row, &mut prev);
    }
    if data.len().saturating_mul(8).saturating_sub(bits.pos) > 7 {
        return Err(bad());
    }
    Ok(Grid { width, height, values })
}
fn step(qp: u32, base: u32, multiplier: u32) -> Result<i32, RawError> {
    // Six phases, doubling every six indices. First truncate the scale to an
    // integer; doing that after multiplication changes HH coefficients.
    if qp > 51 || base > 65535 || multiplier > 65535 {
        return Err(bad());
    }
    let q = qp.checked_add(4).ok_or_else(bad)?;
    let phase = *[40u64, 45, 51, 57, 64, 72].get((q % 6) as usize).ok_or_else(bad)?;
    let scale = phase.checked_shl(q / 6).ok_or_else(bad)? >> 6;
    let result = u64::from(base).checked_add(scale.checked_mul(u64::from(multiplier)).ok_or_else(bad)? >> 3).ok_or_else(bad)?;
    if result == 0 || result > 65535 {
        return Err(bad());
    }
    i32::try_from(result).map_err(|_| bad())
}
struct EncodedBand<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    level: usize,
    base: u32,
    multiplier: u32,
}
fn plane(encoded: &[EncodedBand<'_>], grid: &Grid, width: usize, height: usize, bits: u8) -> Result<Vec<u16>, RawError> {
    let mut bands = Vec::new();
    for (i, b) in encoded.iter().enumerate() {
        let mut band = craw::entropy(b.data, b.width, b.height, i == 0, 1)?;
        for (y, row) in band.values.chunks_exact_mut(b.width).enumerate() {
            for (x, v) in row.iter_mut().enumerate() {
                let step = step(grid.qp(x, y, b.level)?, b.base, b.multiplier)?;
                *v = craw::bounded(i64::from(*v) * i64::from(step))?;
            }
        }
        bands.push(band);
    }
    let mut it = bands.into_iter();
    let mut ll = it.next().ok_or_else(bad)?;
    for _ in 0..3 {
        ll = craw::synth(ll, it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?, false, false)?;
    }
    if it.next().is_some() {
        return Err(bad());
    }
    let ll = craw::crop(&ll, width, height, false, false)?;
    let midpoint = 1i64.checked_shl(u32::from(bits).checked_sub(1).ok_or_else(bad)?).ok_or_else(bad)?;
    let maximum = (1i64.checked_shl(u32::from(bits)).ok_or_else(bad)?) - 1;
    let mut output = Vec::new();
    output.try_reserve_exact(ll.values.len()).map_err(|_| RawError::LimitExceeded("CRX adaptive plane allocation".into()))?;
    for v in ll.values {
        output.push(u16::try_from((i64::from(v) + midpoint).clamp(0, maximum)).map_err(|_| bad())?);
    }
    Ok(output)
}
pub(super) fn samples(track: &CrxTrack<'_>, limits: &Limits) -> Result<Vec<u16>, RawError> {
    if track.version != 0x200 || track.encoding != 0 || track.wavelet_levels != 3 || track.planes != 4 || !(8..=16).contains(&track.bits) {
        return Err(RawError::unsupported("CRX adaptive C-RAW coding variant"));
    }
    let (width, height) = (track.width as usize, track.height as usize);
    if width < 16
        || height < 16
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || track.tile_width != track.width
        || track.tile_height != track.height
    {
        return Err(RawError::unsupported("CRX adaptive C-RAW geometry (single even Bayer tile supported)"));
    }
    limits.check(u64::from(track.width), u64::from(track.height), 2)?;
    let (pw, ph) = (width / 2, height / 2);
    let (qw, qh) = (pw.div_ceil(8), ph.div_ceil(2));
    let batch_size = if cfg!(target_arch = "wasm32") { 1 } else { 2 };
    let peak = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(2)
        .saturating_add((width as u64 + 16).saturating_mul(height as u64 + 16).saturating_mul(6 * batch_size as u64))
        .saturating_add((qw as u64).saturating_mul(qh as u64).saturating_mul(8))
        .saturating_add((qw as u64).saturating_mul(8));
    if peak > limits.max_alloc {
        return Err(RawError::LimitExceeded("CRX adaptive C-RAW working memory".into()));
    }
    let header = track.sample.get(..track.header_size as usize).ok_or_else(bad)?;
    let mut pos = 0;
    let (tile_size, tile) = marker(header, &mut pos, 0xff11, 16)?;
    if u32_at(tile, 0)? != 0x4000 {
        return Err(RawError::unsupported("CRX adaptive tile flags"));
    }
    let qsize = u32_at(tile, 4)? as usize;
    let qpad = usize::from(u16_at(tile, 8)?);
    if qsize == 0 || qpad > 7 || u16_at(tile, 10)? != 0 {
        return Err(bad());
    }
    let mut payload = track.header_size as usize;
    let tile_end = payload.checked_add(tile_size).ok_or_else(bad)?;
    let qend = payload.checked_add(qsize).ok_or_else(bad)?;
    let qdata = track.sample.get(payload..qend).ok_or_else(bad)?;
    payload = qend.checked_add(qpad).ok_or_else(bad)?;
    if tile_end != track.sample.len() || payload > tile_end {
        return Err(bad());
    }
    let ax = craw::axis(pw, false, false)?;
    let ay = craw::axis(ph, false, false)?;
    let mut planes = Vec::new();
    for p in 0..4 {
        let (size, fields) = marker(header, &mut pos, 0xff12, 8)?;
        if u32_at(fields, 0)? != ((p as u32) << 28) | 0x0800_0000 {
            return Err(RawError::unsupported("CRX adaptive plane flags"));
        }
        let plane_end = payload.checked_add(size).ok_or_else(bad)?;
        if plane_end > tile_end {
            return Err(bad());
        }
        let mut bands = Vec::new();
        for b in 0..10 {
            let (size, fields) = marker(header, &mut pos, 0xff13, 16)?;
            let flags = u32_at(fields, 0)?;
            if flags >> 28 != b || flags & 0x0800_0000 != 0 {
                return Err(bad());
            }
            let multiplier = flags & 0x07ff_ffff;
            let base = u32_at(fields, 4)?;
            let padding = usize::from(u16_at(fields, 8)?);
            if padding > 7 || padding >= size || u16_at(fields, 10)? != 0 {
                return Err(bad());
            }
            // Validate quantizer arithmetic before any entropy allocation.
            step(51, base, multiplier)?;
            let level = if b < 4 {
                3
            } else if b < 7 {
                2
            } else {
                1
            };
            let &(lx, hx) = ax.get(level - 1).ok_or_else(bad)?;
            let &(ly, hy) = ay.get(level - 1).ok_or_else(bad)?;
            let (bw, bh) = (if b == 0 || b % 3 == 2 { lx } else { hx }, if b == 0 || b % 3 == 1 { ly } else { hy });
            let end = payload.checked_add(size).ok_or_else(bad)?;
            if end > plane_end {
                return Err(bad());
            }
            let data = track.sample.get(payload..end.checked_sub(padding).ok_or_else(bad)?).ok_or_else(bad)?;
            bands.push(EncodedBand { data, width: bw, height: bh, level, base, multiplier });
            payload = end;
        }
        if payload != plane_end {
            return Err(bad());
        }
        planes.push(bands);
    }
    if payload != tile_end || header.get(pos..).is_none_or(|v| v.len() > 7 || v.iter().any(|b| *b != 0)) {
        return Err(bad());
    }
    let grid = grid(qdata, qw, qh)?;
    let count = width.checked_mul(height).ok_or_else(bad)?;
    let mut output = Vec::new();
    for (batch_index, batch) in planes.chunks(batch_size).enumerate() {
        let decoded = crate::par::map(batch.len(), |i| plane(batch.get(i).ok_or_else(bad)?, &grid, pw, ph, track.bits));
        for (i, result) in decoded.into_iter().enumerate() {
            let p = batch_index.checked_mul(batch_size).and_then(|v| v.checked_add(i)).ok_or_else(bad)?;
            let values = result.map_err(|e| match e {
                RawError::Malformed(m) => RawError::malformed(format!("CRX adaptive plane {p}: {m}")),
                other => other,
            })?;
            if output.is_empty() {
                output = craw::zeros::<u16>(count)?;
            }
            for (y, row) in values.chunks_exact(pw).enumerate() {
                let ty = y.checked_mul(2).and_then(|v| v.checked_add(p >> 1)).ok_or_else(bad)?;
                for (x, &v) in row.iter().enumerate() {
                    let tx = x.checked_mul(2).and_then(|v| v.checked_add(p & 1)).ok_or_else(bad)?;
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
    fn qp_escape_and_truncation_are_bounded() {
        let data = [0, 0, 1, 44, 0x97, 0xff, 0xff, 0xff];
        let g = grid(&data, 29, 1).unwrap();
        assert_eq!(g.values, vec![22; 29]);
        assert!(grid(&[], 4, 2).is_err());
        assert!(grid(&[0; 20], 4, 2).is_err());
        assert!(grid(&[0, 0, 1, 255], 4, 2).is_err());
        assert!(grid(&[data.as_slice(), &[0]].concat(), 29, 1).is_err());
    }
    #[test]
    fn qp_averages_before_scaling_and_extends_the_last_row() {
        let g = Grid { width: 2, height: 3, values: vec![22, 26, 29, 33, 34, 40] };
        assert_eq!(g.qp(3, 0, 1).unwrap(), 22);
        assert_eq!(g.qp(4, 0, 1).unwrap(), 26);
        assert_eq!(g.qp(0, 0, 2).unwrap(), 25);
        assert_eq!(g.qp(2, 0, 2).unwrap(), 29);
        assert_eq!(g.qp(0, 1, 2).unwrap(), 34);
        assert_eq!(g.qp(0, 0, 3).unwrap(), 29);
        assert!(g.qp(8, 0, 1).is_err());
        assert!(g.qp(0, 0, 0).is_err());
    }
    #[test]
    fn adaptive_quantization_truncates_the_scale_before_multiplication() {
        assert_eq!(step(22, 0, 4).unwrap(), 6);
        assert_eq!(step(26, 0, 8).unwrap(), 20);
        assert_eq!(step(29, 0, 16).unwrap(), 56);
        assert_eq!(step(33, 1, 0).unwrap(), 1);
        assert!(step(52, 0, 4).is_err());
        assert!(step(22, 0, 0).is_err());
        assert!(step(51, u32::MAX, u32::MAX).is_err());
    }
}
