//! CRX version 0x100 lossless Bayer reconstruction.
//!
//! Sources: public CMP1/marker documentation, Canon US20160323602A1 and
//! JP2017192077A (MED, signed mapping, adaptive Rice and run detection),
//! ITU-T T.87 (MELCODE). Adaptive thresholds, upper-row lookahead, boundary
//! cases and padding units were independently measured against CC0 camera
//! bytes and DNGLab 0.8.0's released Windows binary output. No decoder source
//! was read. See docs/cr3.md for the measurements and precise support scope.
use super::CrxTrack;
use crate::{Limits, RawError};

fn bad() -> RawError {
    RawError::malformed("invalid CRX lossless sensor stream")
}
fn be16(b: &[u8], p: usize) -> Result<u16, RawError> {
    Ok(u16::from_be_bytes(b.get(p..p.checked_add(2).ok_or_else(bad)?).ok_or_else(bad)?.try_into().map_err(|_| bad())?))
}
fn be32(b: &[u8], p: usize) -> Result<u32, RawError> {
    Ok(u32::from_be_bytes(b.get(p..p.checked_add(4).ok_or_else(bad)?).ok_or_else(bad)?.try_into().map_err(|_| bad())?))
}
pub(super) struct Bits<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) pos: usize,
}
impl Bits<'_> {
    pub(super) fn read(&mut self, n: u32) -> Result<u32, RawError> {
        if n > 21 {
            return Err(bad());
        }
        let mut value = 0;
        for _ in 0..n {
            let byte = self.bytes.get(self.pos / 8).ok_or_else(bad)?;
            value = (value << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos = self.pos.checked_add(1).ok_or_else(bad)?;
        }
        Ok(value)
    }
    pub(super) fn rice(&mut self, k: u32) -> Result<u32, RawError> {
        let mut zeros = 0u32;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 41 {
                return Err(bad());
            }
        }
        if zeros == 41 { self.read(21) } else { Ok((zeros.checked_shl(k).ok_or_else(bad)?) | self.read(k)?) }
    }
}
const J: [u32; 32] = [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15];
pub(super) fn run(bits: &mut Bits<'_>, index: &mut usize, remaining: usize) -> Result<usize, RawError> {
    let mut length = 1usize;
    loop {
        let j = *J.get(*index).ok_or_else(bad)?;
        if bits.read(1)? == 1 {
            length = length.checked_add(1usize.checked_shl(j).ok_or_else(bad)?).ok_or_else(bad)?;
            if length > remaining {
                return Ok(remaining);
            }
            *index = (*index + 1).min(J.len() - 1);
            if length == remaining {
                return Ok(remaining);
            }
        } else {
            length = length.checked_add(bits.read(j)? as usize).ok_or_else(bad)?;
            *index = index.saturating_sub(1);
            return if length <= remaining { Ok(length) } else { Err(bad()) };
        }
    }
}
pub(super) fn med(a: i32, b: i32, c: i32) -> i32 {
    if c >= a.max(b) {
        a.min(b)
    } else if c <= a.min(b) {
        a.max(b)
    } else {
        a + b - c
    }
}
pub(super) fn adapt(k: u32, value: u32) -> u32 {
    let scaled = value >> k;
    (k as i32 + i32::from(scaled > 2) + i32::from(scaled > 5) - i32::from(value < (1 << k.saturating_sub(1)))).clamp(0, 20) as u32
}
fn plane(src: &[u8], width: usize, height: usize, bits: u8) -> Result<Vec<u16>, RawError> {
    let mut reader = Bits { bytes: src, pos: 0 };
    let mut k = 0;
    let mut index = 0usize;
    let alloc = || {
        let mut row = Vec::new();
        row.try_reserve_exact(width).map_err(|_| RawError::LimitExceeded("CRX row allocation".into()))?;
        row.resize(width, 0i32);
        Ok::<_, RawError>(row)
    };
    let mut prev = alloc()?;
    let mut row = alloc()?;
    let mut output = Vec::new();
    let midpoint = 1i32.checked_shl(u32::from(bits).saturating_sub(1)).ok_or_else(bad)?;
    let maximum = (1i32.checked_shl(u32::from(bits)).ok_or_else(bad)?) - 1;
    for y in 0..height {
        let mut x = 0;
        let mut interrupted = false;
        while x < width {
            let a = if x == 0 { *prev.first().ok_or_else(bad)? } else { *row.get(x - 1).ok_or_else(bad)? };
            let b = *prev.get(x).ok_or_else(bad)?;
            let c = if x == 0 { b } else { *prev.get(x - 1).ok_or_else(bad)? };
            let d = *prev.get((x + 1).min(width - 1)).ok_or_else(bad)?;
            let flat = x + 1 < width && if y == 0 { a == 0 } else { a == b && b == d };
            let detected = !interrupted && flat && reader.read(1)? == 1;
            if detected {
                let n = run(&mut reader, &mut index, width - x)?;
                row.get_mut(x..x.checked_add(n).ok_or_else(bad)?).ok_or_else(bad)?.fill(a);
                x += n;
                interrupted = true;
                continue;
            }
            let mapped = reader.rice(k)?;
            let delta = if mapped & 1 == 0 { (mapped / 2) as i32 } else { -((mapped / 2) as i32) - 1 };
            let prediction = if y == 0 {
                a
            } else if interrupted {
                b
            } else {
                med(a, b, c)
            };
            let value = prediction.checked_add(delta).ok_or_else(bad)?;
            if value.checked_add(midpoint).is_none_or(|v| v < 0 || v > maximum) {
                return Err(RawError::malformed(format!("CRX sample outside declared precision at {x},{y}")));
            }
            *row.get_mut(x).ok_or_else(bad)? = value;
            let scale = if y == 0 || x + 1 == width { mapped } else { mapped / 2 + (d - b).unsigned_abs() };
            k = adapt(k, scale);
            interrupted = false;
            x += 1;
        }
        // Grow only after decoding a valid row; malformed tiny files cannot reserve
        // their entire declared image before any entropy has been verified.
        output.try_reserve(width).map_err(|_| RawError::LimitExceeded("CRX plane allocation".into()))?;
        for &value in &row {
            output.push(u16::try_from(value.checked_add(midpoint).ok_or_else(bad)?).map_err(|_| bad())?);
        }
        std::mem::swap(&mut prev, &mut row);
    }
    if reader.bytes.len().saturating_mul(8).saturating_sub(reader.pos) > 7 {
        return Err(RawError::malformed("CRX plane has undecoded entropy bytes"));
    }
    Ok(output)
}
struct Segment<'a> {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    plane: usize,
    data: &'a [u8],
}
pub(super) fn marker(header: &[u8], pos: &mut usize, kind: u16) -> Result<(usize, u32), RawError> {
    if be16(header, *pos)? != kind || be16(header, pos.checked_add(2).ok_or_else(bad)?)? != 8 {
        return Err(bad());
    }
    let size = be32(header, pos.checked_add(4).ok_or_else(bad)?)? as usize;
    let flags = be32(header, pos.checked_add(8).ok_or_else(bad)?)?;
    *pos = pos.checked_add(12).ok_or_else(bad)?;
    Ok((size, flags))
}
pub(super) fn samples(track: &CrxTrack<'_>, limits: &Limits) -> Result<Vec<u16>, RawError> {
    if track.version != 0x100 || track.encoding != 0 || track.wavelet_levels != 0 || track.planes != 4 {
        return Err(RawError::unsupported("CRX coding variant (only version 0x100 lossless Bayer is decoded)"));
    }
    let width = track.width as usize;
    let height = track.height as usize;
    let tw = track.tile_width as usize;
    let th = track.tile_height as usize;
    if !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || !tw.is_multiple_of(2)
        || !th.is_multiple_of(2)
        || width == 0
        || height == 0
        || tw == 0
        || th == 0
    {
        return Err(RawError::unsupported("CRX rounded Bayer plane geometry"));
    }
    limits.check(u64::from(track.width), u64::from(track.height), 2)?;
    // Bound concurrency to two independent planes on native. Keeping every
    // decoded plane at once would unnecessarily double full-sensor memory;
    // wasm uses one plane because its address space is more constrained.
    let batch_size = if cfg!(target_arch = "wasm32") { 1 } else { 2 };
    let peak = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(2)
        .saturating_add((tw as u64).saturating_mul(th as u64).saturating_mul(batch_size as u64))
        .saturating_add((tw as u64).saturating_mul(8).saturating_mul(batch_size as u64));
    if peak > limits.max_alloc {
        return Err(RawError::LimitExceeded("CRX sensor working memory".into()));
    }
    let cols = width.div_ceil(tw);
    let rows = height.div_ceil(th);
    if cols.checked_mul(rows).is_none_or(|n| n > 16) {
        return Err(RawError::unsupported("CRX tile count"));
    }
    let header = track.sample.get(..track.header_size as usize).ok_or_else(bad)?;
    let mut pos = 0;
    let mut payload = track.header_size as usize;
    let mut segments = Vec::new();
    for ty in 0..rows {
        for tx in 0..cols {
            let (tile_size, tile_flags) = marker(header, &mut pos, 0xff01)?;
            if tile_flags != ((ty * cols + tx) as u32) << 16 {
                return Err(RawError::unsupported("CRX tile header flags"));
            }
            let tile_end = payload.checked_add(tile_size).ok_or_else(bad)?;
            let w = (width - tx * tw).min(tw);
            let h = (height - ty * th).min(th);
            if !w.is_multiple_of(2) || !h.is_multiple_of(2) {
                return Err(RawError::unsupported("CRX partial tile geometry"));
            }
            for p in 0..4usize {
                let (size, flags) = marker(header, &mut pos, 0xff02)?;
                if flags != ((p as u32) << 28) | 0x0800_0000 {
                    return Err(RawError::unsupported("CRX plane header flags"));
                }
                let (subsize, subflags) = marker(header, &mut pos, 0xff03)?;
                let padding = (subflags & 0x7ffff) as usize;
                if size != subsize || subflags >> 19 != 4 || padding > 7 || padding > size {
                    return Err(RawError::unsupported("CRX lossless subband flags"));
                }
                let end = payload.checked_add(size).ok_or_else(bad)?;
                if end > tile_end {
                    return Err(bad());
                }
                let data = track.sample.get(payload..end - padding).ok_or_else(bad)?;
                segments.push(Segment { x: tx * tw, y: ty * th, width: w / 2, height: h / 2, plane: p, data });
                payload = end;
            }
            if payload != tile_end {
                return Err(bad());
            }
        }
    }
    if payload != track.sample.len() || header.get(pos..).is_none_or(|b| b.len() > 7 || b.iter().any(|v| *v != 0)) {
        return Err(bad());
    }
    let count = width.checked_mul(height).ok_or_else(bad)?;
    let mut output = Vec::new();
    for batch in segments.chunks(batch_size) {
        let decoded = crate::par::map(batch.len(), |i| {
            let segment = batch.get(i).ok_or_else(bad)?;
            plane(segment.data, segment.width, segment.height, track.bits).map_err(|e| match e {
                RawError::Malformed(message) => RawError::malformed(format!("CRX tile at {},{} plane {}: {message}", segment.x, segment.y, segment.plane)),
                other => other,
            })
        });
        for (segment, data) in batch.iter().zip(decoded) {
            let data = data?;
            if output.is_empty() {
                output.try_reserve_exact(count).map_err(|_| RawError::LimitExceeded("CRX sensor allocation".into()))?;
                output.resize(count, 0);
            }
            let dx = segment.plane & 1;
            let dy = segment.plane >> 1;
            for (y, row) in data.chunks_exact(segment.width).enumerate() {
                let target_y = segment.y.checked_add(y.checked_mul(2).ok_or_else(bad)?).and_then(|v| v.checked_add(dy)).ok_or_else(bad)?;
                for (x, value) in row.iter().enumerate() {
                    let target_x = segment.x.checked_add(x.checked_mul(2).ok_or_else(bad)?).and_then(|v| v.checked_add(dx)).ok_or_else(bad)?;
                    let at = target_y.checked_mul(width).and_then(|v| v.checked_add(target_x)).ok_or_else(bad)?;
                    *output.get_mut(at).ok_or_else(bad)? = *value;
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
    fn exact_row_end_advances_mel_state_but_overshoot_does_not() {
        let mut state = 3;
        assert_eq!(run(&mut Bits { bytes: &[0x80], pos: 0 }, &mut state, 2).unwrap(), 2);
        assert_eq!(state, 4);
        assert_eq!(run(&mut Bits { bytes: &[0x80], pos: 0 }, &mut state, 2).unwrap(), 2);
        assert_eq!(state, 4);
    }
    #[test]
    fn flat_rows_truncation_and_declared_precision() {
        assert_eq!(plane(&[0xfe], 4, 2, 14).unwrap(), vec![8192; 8]);
        assert!(plane(&[], 4, 2, 14).is_err());
        // A unary prefix exceeding the 41-zero escape must be rejected.
        assert!(plane(&[0; 16], 4, 2, 14).is_err());
        assert!(plane(&[0xfe, 0], 4, 2, 14).is_err());
    }
}
