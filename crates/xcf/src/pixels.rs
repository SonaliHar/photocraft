use crate::{reader::Reader, *};
use flate2::{Decompress, FlushDecompress, Status};
pub(crate) fn plane(r: &mut Reader<'_, '_>, p: usize, w: u32, h: u32, channels: usize, sample: Sample, compression: Compression) -> Result<Plane, XcfError> {
    r.enter(p)?;
    let rw = r.u32()?;
    let rh = r.u32()?;
    let bpp = r.u32()? as usize;
    let expected_bpp = channels.checked_mul(sample.source_bytes()).ok_or_else(|| r.error(ErrorKind::Structure, "pixel stride overflow"))?;
    if rw != w || rh != h || bpp != expected_bpp {
        return Err(r.error(ErrorKind::Structure, "hierarchy dimensions/stride mismatch"));
    }
    let level = r.hierarchy_level()?.ok_or_else(|| r.error(ErrorKind::Structure, "missing pixel level"))?;
    r.enter(level)?;
    if r.u32()? != w || r.u32()? != h {
        return Err(r.error(ErrorKind::Structure, "level dimensions mismatch"));
    }
    let cols = w.div_ceil(64);
    let rows = h.div_ceil(64);
    let count = (cols as usize).checked_mul(rows as usize).ok_or_else(|| r.error(ErrorKind::Limit, "tile count overflow"))?;
    if count > r.limits.visits {
        return Err(r.error(ErrorKind::Limit, "tile work limit"));
    }
    let tiles = r.pointers(count)?;
    if tiles.len() != count {
        return Err(r.error(ErrorKind::Structure, "tile count mismatch"));
    }
    let out_sample = if sample == Sample::F16 { Sample::F32 } else { sample };
    let out_bpp = channels * out_sample.source_bytes();
    let len = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(out_bpp)).ok_or_else(|| r.error(ErrorKind::Limit, "plane byte overflow"))?;
    let mut bytes = r.vec(len)?;
    // One bounded tile scratch allocation, reused and charged along with the final plane.
    let max_tile = 64usize * 64 * bpp;
    let mut scratch = r.vec(max_tile)?;
    if compression == Compression::Zlib {
        r.charge(128 * 1024)?;
    } // zlib window/state and bounded decoder scratch
    for (i, tp) in tiles.into_iter().enumerate() {
        r.enter(tp)?;
        let tx = (i as u32) % cols;
        let ty = (i as u32) / cols;
        let tw = (w - tx * 64).min(64);
        let th = (h - ty * 64).min(64);
        let pixels = (tw * th) as usize;
        let size = pixels * bpp;
        let tile = scratch.get_mut(..size).ok_or_else(|| r.error(ErrorKind::Structure, "tile scratch bounds"))?;
        match compression {
            Compression::None => tile.copy_from_slice(r.take(size)?),
            Compression::Rle => {
                for c in 0..bpp {
                    let mut written = 0;
                    while written < pixels {
                        r.poll()?;
                        let op = r.take(1)?.first().copied().ok_or_else(|| r.error(ErrorKind::Truncated, "RLE opcode"))?;
                        let repeat = op <= 127;
                        let run = if op == 127 || op == 128 {
                            let a = r.take(2)?;
                            u16::from_be_bytes(a.try_into().map_err(|_| r.error(ErrorKind::Truncated, "RLE length"))?) as usize
                        } else if repeat {
                            usize::from(op) + 1
                        } else {
                            256 - usize::from(op)
                        };
                        if run == 0 || run > pixels - written {
                            return Err(r.error(ErrorKind::Compression, "zero/oversized RLE run"));
                        }
                        let data = r.take(if repeat { 1 } else { run })?;
                        for j in 0..run {
                            let v = data.get(if repeat { 0 } else { j }).copied().ok_or_else(|| r.error(ErrorKind::Truncated, "RLE literal"))?;
                            let at = (written + j) * bpp + c;
                            let output = tile.get_mut(at).ok_or_else(|| r.error(ErrorKind::Structure, "RLE output bounds"))?;
                            *output = v;
                        }
                        written += run;
                    }
                }
            }
            Compression::Zlib => {
                let mut z = Decompress::new(true);
                loop {
                    r.poll()?;
                    let input = r.bytes.get(r.pos..).ok_or_else(|| r.error(ErrorKind::Truncated, "zlib input"))?;
                    let input = input.get(..input.len().min(4096)).ok_or_else(|| r.error(ErrorKind::Truncated, "zlib chunk"))?;
                    let out = usize::try_from(z.total_out()).map_err(|_| r.error(ErrorKind::Compression, "zlib output size"))?;
                    let dest = tile.get_mut(out..).ok_or_else(|| r.error(ErrorKind::Compression, "zlib exceeds tile size"))?;
                    let before_in = z.total_in();
                    let before_out = z.total_out();
                    let status = z.decompress(input, dest, FlushDecompress::None).map_err(|_| r.error(ErrorKind::Compression, "invalid zlib stream"))?;
                    let consumed = usize::try_from(z.total_in() - before_in).map_err(|_| r.error(ErrorKind::Compression, "zlib size"))?;
                    r.pos = r.pos.checked_add(consumed).ok_or_else(|| r.error(ErrorKind::Structure, "zlib offset overflow"))?;
                    if status == Status::StreamEnd {
                        break;
                    }
                    if consumed == 0 && z.total_out() == before_out {
                        return Err(r.error(ErrorKind::Compression, "truncated/oversized zlib tile"));
                    }
                    if z.total_out() > size as u64 {
                        return Err(r.error(ErrorKind::Compression, "zlib output exceeds tile size"));
                    }
                }
                if z.total_out() != size as u64 {
                    return Err(r.error(ErrorKind::Compression, "zlib tile has wrong decoded size"));
                }
            }
        }
        for y in 0..th as usize {
            for x in 0..tw as usize {
                for c in 0..channels {
                    let src = ((y * tw as usize + x) * channels + c) * sample.source_bytes();
                    let dst = ((((ty * 64) as usize + y) * w as usize + (tx * 64) as usize + x) * channels + c) * out_sample.source_bytes();
                    let input = tile.get(src..src + sample.source_bytes()).ok_or_else(|| r.error(ErrorKind::Structure, "sample input bounds"))?;
                    let output = bytes.get_mut(dst..dst + out_sample.source_bytes()).ok_or_else(|| r.error(ErrorKind::Structure, "sample output bounds"))?;
                    match sample {
                        Sample::U8 => output.copy_from_slice(input),
                        Sample::U16 => {
                            let a: [u8; 2] = input.try_into().map_err(|_| r.error(ErrorKind::Structure, "u16 sample"))?;
                            output.copy_from_slice(&u16::from_be_bytes(a).to_ne_bytes());
                        }
                        Sample::F16 => {
                            let a: [u8; 2] = input.try_into().map_err(|_| r.error(ErrorKind::Structure, "half sample"))?;
                            let f = half::f16::from_bits(u16::from_be_bytes(a)).to_f32();
                            if !f.is_finite() {
                                return Err(r.error(ErrorKind::Unsupported, "nonfinite F16 sample"));
                            }
                            output.copy_from_slice(&f.to_ne_bytes());
                        }
                        Sample::F32 => {
                            let a: [u8; 4] = input.try_into().map_err(|_| r.error(ErrorKind::Structure, "float sample"))?;
                            let f = f32::from_be_bytes(a);
                            if !f.is_finite() {
                                return Err(r.error(ErrorKind::Unsupported, "nonfinite F32 sample"));
                            }
                            output.copy_from_slice(&f.to_ne_bytes());
                        }
                    }
                }
            }
        }
    }
    Ok(Plane { width: w, height: h, channels, sample: out_sample, bytes })
}
