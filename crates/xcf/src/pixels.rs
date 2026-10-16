use crate::{reader::Reader, *};
use flate2::{Decompress, FlushDecompress, Status};
pub(crate) fn plane<S: PlaneSink>(
    r: &mut Reader<'_, '_>,
    p: u64,
    descriptor: PlaneDescriptor,
    name: &str,
    compression: Compression,
    sink: &mut S,
) -> Result<S::Plane, XcfError> {
    sink.validate_plane_descriptor(&descriptor,&mut PlaneSinkControl{reader:r})?;
    let PlaneDescriptor { width: w, height: h, channels, source_sample: sample, .. } = descriptor;
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
    sink.begin(&descriptor, &mut PlaneSinkControl { reader: r })?;
    let coverage_policy=sink.mask_coverage_policy(&descriptor);
    if coverage_policy==MaskCoveragePolicy::CheckedF32Preimage && descriptor.role!=PlaneRole::Mask{return Err(r.error(ErrorKind::Unsupported,"checked coverage requires exact Mask descriptor"));}
    // One bounded tile scratch allocation, reused and charged along with the final plane.
    let max_tile = 64usize * 64 * bpp.max(out_bpp);
    // This is resident memory, not owned disk scratch (which wasm forbids).
    let mut scratch = r.vec(max_tile)?;
    let _decoder = if compression == Compression::Zlib {
        r.poll()?;
        if r.charged.checked_add(128 * 1024).is_none_or(|total| total > r.limits.resident_bytes) {
            return Err(r.error(ErrorKind::Limit, "lowered zlib resident state budget"));
        }
        r.budget.map(|budget| budget.reserve(BudgetKind::Decoder, 128 * 1024, "zlib tile decoder")).transpose()?
    } else {
        None
    }; // live zlib window/state lease, released with this plane decode
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
            Compression::None => {
                for chunk in tile.chunks_mut(65536) {
                    r.poll()?;
                    r.read_at(r.pos, chunk)?;
                    r.pos = r.pos.checked_add(chunk.len() as u64).ok_or_else(|| r.error(ErrorKind::Structure, "raw tile offset overflow"))?;
                }
            }
            Compression::Rle => {
                for c in 0..bpp {
                    let mut written = 0;
                    while written < pixels {
                        r.poll()?;
                        let op = r.take(1)?.first().copied().ok_or_else(|| r.error(ErrorKind::Truncated, "RLE opcode"))?;
                        let repeat = op <= 127;
                        let run = if op == 127 || op == 128 {
                            let a = r.take(2)?;
                            u16::from_be_bytes(a.as_ref().try_into().map_err(|_| r.error(ErrorKind::Truncated, "RLE length"))?) as usize
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
                    let start = r.pos;
                    let available = usize::try_from((r.len() - start).min(4096)).map_err(|_| r.error(ErrorKind::Limit, "zlib window"))?;
                    let input = r.take(available)?;
                    r.pos = start;
                    let out = usize::try_from(z.total_out()).map_err(|_| r.error(ErrorKind::Compression, "zlib output size"))?;
                    let end = out.saturating_add(4096).min(tile.len());
                    let dest = tile.get_mut(out..end).ok_or_else(|| r.error(ErrorKind::Compression, "zlib exceeds tile size"))?;
                    let before_in = z.total_in();
                    let before_out = z.total_out();
                    let status = z.decompress(&input, dest, FlushDecompress::None).map_err(|_| r.error(ErrorKind::Compression, "invalid zlib stream"))?;
                    let consumed = usize::try_from(z.total_in() - before_in).map_err(|_| r.error(ErrorKind::Compression, "zlib size"))?;
                    r.pos = r.pos.checked_add(consumed as u64).ok_or_else(|| r.error(ErrorKind::Structure, "zlib offset overflow"))?;
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
        // Reverse conversion permits exact F16 widening in the same bounded
        // tile allocation; fixed stack bytes prevent overlapping borrows.
        for unit in (0..pixels * channels).rev() {
            if unit.is_multiple_of(4096) {
                r.poll()?;
                if let Some(budget) = r.budget {
                    budget.consume_work(((pixels * channels) - unit).min(4096) as u64, "sample decode")?;
                }
            }
            let src = unit * sample.source_bytes();
            let dst = unit * out_sample.source_bytes();
            let mut value_bytes = [0u8; 8];
            let source = scratch.get(src..src + sample.source_bytes()).ok_or_else(|| r.error(ErrorKind::Structure, "sample input bounds"))?;
            value_bytes.get_mut(..sample.source_bytes()).ok_or_else(|| r.error(ErrorKind::Structure, "sample width"))?.copy_from_slice(source);
            let input = value_bytes.get(..sample.source_bytes()).ok_or_else(|| r.error(ErrorKind::Structure, "sample width"))?;
            let output = scratch.get_mut(dst..dst + out_sample.source_bytes()).ok_or_else(|| r.error(ErrorKind::Structure, "sample output bounds"))?;
            match sample {
                Sample::U8 => output.copy_from_slice(input),
                Sample::U16 => {
                    let a: [u8; 2] = input.try_into().map_err(|_| r.error(ErrorKind::Structure, "u16 sample"))?;
                    output.copy_from_slice(&u16::from_be_bytes(a).to_ne_bytes());
                }
                Sample::U32 => {
                    let a: [u8; 4] = input.try_into().map_err(|_| r.error(ErrorKind::Structure, "u32 sample"))?;
                    output.copy_from_slice(&u32::from_be_bytes(a).to_ne_bytes());
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
                Sample::F64 => {
                    let a: [u8; 8] = input.try_into().map_err(|_| r.error(ErrorKind::Structure, "double sample"))?;
                    let f = f64::from_be_bytes(a);
                    if !f.is_finite() {
                        return Err(r.error(ErrorKind::Unsupported, "nonfinite F64 sample"));
                    }
                    output.copy_from_slice(&f.to_ne_bytes());
                }
            }
        }
        let decoded = scratch.get(..pixels * out_bpp).ok_or_else(|| r.error(ErrorKind::Structure, "native tile extent"))?;
        if (descriptor.role != PlaneRole::Raster || descriptor.alpha) && matches!(out_sample, Sample::F32 | Sample::F64) {
            if coverage_policy==MaskCoveragePolicy::CheckedF32Preimage{
                let extra=(pixels as u64).checked_mul((out_sample.source_bytes()as u64).checked_add(16).ok_or_else(||r.error(ErrorKind::Limit,"checked coverage work"))?).ok_or_else(||r.error(ErrorKind::Limit,"checked coverage work"))?;
                r.poll()?;if let Some(b)=r.budget{b.consume_work(extra,"checked preplanned mask coverage decode")?;}
            }
            let width = out_sample.source_bytes();
            let coverage = if descriptor.role != PlaneRole::Raster { 0 } else { (channels - 1) * width };
            for (i, pixel) in decoded.chunks_exact(out_bpp).enumerate() {
                if i.is_multiple_of(64) {
                    r.poll()?;
                }
                let bytes = pixel.get(coverage..coverage + width).ok_or_else(|| r.error(ErrorKind::Structure, "coverage bounds"))?;
                let value = if out_sample == Sample::F64 {
                    f64::from_ne_bytes(bytes.try_into().map_err(|_| r.error(ErrorKind::Structure, "coverage sample"))?)
                } else {
                    f64::from(f32::from_ne_bytes(bytes.try_into().map_err(|_| r.error(ErrorKind::Structure, "coverage sample"))?))
                };
                if !value.is_finite() || if coverage_policy==MaskCoveragePolicy::CheckedF32Preimage{
                    let converted=value as f32;value<0.||!converted.is_finite()||!(0.0..=1.0).contains(&converted)
                }else{(descriptor.role != PlaneRole::Raster || !r.options.allow_finite_drawable_alpha) && !(0.0..=1.0).contains(&value)} {
                    return Err(r.error(ErrorKind::Unsupported, format!("{}: alpha/mask coverage outside 0..1", name.chars().take(80).collect::<String>())));
                }
            }
        }
        r.poll()?;
        sink.tile(tx * 64, ty * 64, tw, th, decoded, &mut PlaneSinkControl { reader: r })?;
    }
    sink.finish(&mut PlaneSinkControl { reader: r })
}
