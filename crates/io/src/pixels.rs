//! Conversions between PSD planar big-endian channel data and interleaved
//! native-endian surface bytes.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Surface;

/// The common RGB8 import path: assemble planar channels in the final tiles, avoiding a
/// full-image interleaved allocation and its subsequent tile copy. Other formats keep the
/// general conversion path below. Coordinates may be negative or not tile-aligned.
pub(crate) fn rgba8_surface(planes: &[Option<&[u8]>], rect: Rect) -> Option<Surface> {
    let size = rect.size();
    let (width, height) = (usize::try_from(size.width).ok()?, usize::try_from(size.height).ok()?);
    let n = width.checked_mul(height)?;
    let red = planes.first().copied().flatten()?;
    let green = planes.get(1).copied().flatten()?;
    let blue = planes.get(2).copied().flatten()?;
    let alpha = planes.get(3).copied().flatten();
    if red.len() != n || green.len() != n || blue.len() != n || alpha.is_some_and(|p| p.len() != n) {
        return None;
    }
    let mut surface = Surface::new(photocraft_color::PixelFormat::RGBA8);
    let tiles = surface.take_tiles(rect);
    let count = tiles.len();
    let step = bands(count).first().map_or(1, |b| b.len()).max(1);
    let mut iter = tiles.into_iter();
    let mut jobs = Vec::new();
    while !iter.as_slice().is_empty() {
        jobs.push(iter.by_ref().take(step).collect::<Vec<_>>());
    }
    let ready = par_map(jobs, |tiles| {
        let mut populated = Vec::with_capacity(tiles.len());
        for (coord, tile) in tiles {
            let tr = coord.rect().intersect(&rect);
            let mut tile = std::sync::Arc::unwrap_or_clone(tile);
            let data = tile.bytes_mut();
            for y in tr.y0..tr.y1 {
                let src =
                    usize::try_from(y.saturating_sub(rect.y0)).ok()?.checked_mul(width)?.checked_add(usize::try_from(tr.x0.saturating_sub(rect.x0)).ok()?)?;
                let span = tr.width() as usize;
                let dst = ((y - coord.rect().y0) as usize * TILE_SIZE as usize + (tr.x0 - coord.rect().x0) as usize) * 4;
                let row = data.get_mut(dst..dst.checked_add(span.checked_mul(4)?)?)?;
                let end = src.checked_add(span)?;
                let (r, g, b) = (red.get(src..end)?, green.get(src..end)?, blue.get(src..end)?);
                if let Some(a) = alpha.and_then(|a| a.get(src..src + span)) {
                    for ((((dst, r), g), b), a) in row.chunks_exact_mut(4).zip(r).zip(g).zip(b).zip(a) {
                        dst.copy_from_slice(&[*r, *g, *b, *a]);
                    }
                } else {
                    for (((dst, r), g), b) in row.chunks_exact_mut(4).zip(r).zip(g).zip(b) {
                        dst.copy_from_slice(&[*r, *g, *b, 255]);
                    }
                }
            }
            // Preserve nonzero RGB under zero alpha, just like Surface::prune does.
            if tile.bytes().iter().any(|&v| v != 0) {
                populated.push((coord, std::sync::Arc::new(tile)));
            }
        }
        Some(populated)
    })
    .into_iter()
    .collect::<Option<Vec<_>>>()?;
    surface.put_tiles(ready.into_iter().flatten());
    Some(surface)
}

/// Assemble supported PSD channel planes directly in final native-endian tiles. RGB8 keeps
/// its specialized packing loop; other depths/models avoid the full-image interleaved buffer.
/// Missing channels use the same zero/opaque fills as `interleave`; malformed planes fall back.
pub(crate) fn planar_surface(planes: &[Option<&[u8]>], rect: Rect, format: PixelFormat) -> Option<Surface> {
    if !format.alpha || planes.len() != format.channels() || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Cmyk | ColorMode::Lab) {
        return None;
    }
    if format == PixelFormat::RGBA8
        && let Some(surface) = rgba8_surface(planes, rect)
    {
        return Some(surface);
    }
    let size = rect.size();
    let (width, height) = (usize::try_from(size.width).ok()?, usize::try_from(size.height).ok()?);
    let bps = format.sample.bytes();
    let channels = format.channels();
    let bpp = channels.checked_mul(bps)?;
    let expected = width.checked_mul(height)?.checked_mul(bps)?;
    if planes.len() != channels || planes.iter().all(Option::is_none) || planes.iter().flatten().any(|p| p.len() != expected) {
        return None;
    }
    let alpha = max_sample(format.sample);
    let zero = zero_sample(format.sample);
    let mut surface = Surface::new(format);
    let tiles = surface.take_tiles(rect);
    let step = bands(tiles.len()).first().map_or(1, |b| b.len()).max(1);
    let mut iter = tiles.into_iter();
    let mut jobs = Vec::new();
    while !iter.as_slice().is_empty() {
        jobs.push(iter.by_ref().take(step).collect::<Vec<_>>());
    }
    let ready = par_map(jobs, |tiles| {
        let mut populated = Vec::with_capacity(tiles.len());
        for (coord, tile) in tiles {
            let tile_rect = coord.rect();
            let tr = tile_rect.intersect(&rect);
            let mut tile = std::sync::Arc::unwrap_or_clone(tile);
            for y in tr.y0..tr.y1 {
                let src = usize::try_from(y.saturating_sub(rect.y0))
                    .ok()?
                    .checked_mul(width)?
                    .checked_add(usize::try_from(tr.x0.saturating_sub(rect.x0)).ok()?)?
                    .checked_mul(bps)?;
                let span = usize::try_from(tr.width()).ok()?;
                let dst = usize::try_from(y.saturating_sub(tile_rect.y0))
                    .ok()?
                    .checked_mul(TILE_SIZE as usize)?
                    .checked_add(usize::try_from(tr.x0.saturating_sub(tile_rect.x0)).ok()?)?
                    .checked_mul(bpp)?;
                let row = tile.bytes_mut().get_mut(dst..dst.checked_add(span.checked_mul(bpp)?)?)?;
                let end = src.checked_add(span.checked_mul(bps)?)?;
                for (channel, plane) in planes.iter().enumerate() {
                    if let Some(plane) = plane {
                        let input = plane.get(src..end)?;
                        for (pixel, sample) in row.chunks_exact_mut(bpp).zip(input.chunks_exact(bps)) {
                            let out = pixel.get_mut(channel * bps..(channel + 1) * bps)?;
                            out.copy_from_slice(sample);
                            if cfg!(target_endian = "little") {
                                out.reverse();
                            }
                            if format.mode == ColorMode::Cmyk && channel + 1 < channels {
                                invert_sample(out, format.sample);
                            }
                        }
                    } else {
                        // Missing CMYK planes are zero ink; inversion applies only to samples
                        // present in the PSD, exactly as in the existing interleave path.
                        let fill = if channel + 1 == channels { &alpha } else { &zero };
                        for pixel in row.chunks_exact_mut(bpp) {
                            pixel.get_mut(channel * bps..(channel + 1) * bps)?.copy_from_slice(fill);
                        }
                    }
                }
                if format.mode == ColorMode::Lab && format.sample == SampleType::U16 {
                    lab16_chroma(row, channels, true);
                }
            }
            // Byte comparison preserves hidden color, signed zero and NaN payloads, matching
            // Surface::prune against the all-zero default pixel rather than testing alpha.
            if tile.bytes().iter().any(|&v| v != 0) {
                populated.push((coord, std::sync::Arc::new(tile)));
            }
        }
        Some(populated)
    })
    .into_iter()
    .collect::<Option<Vec<_>>>()?;
    surface.put_tiles(ready.into_iter().flatten());
    Some(surface)
}

/// PSD depth (bits) for a sample type.
pub fn psd_depth(s: SampleType) -> u16 {
    match s {
        SampleType::U8 => 8,
        SampleType::U16 => 16,
        SampleType::F32 => 32,
    }
}

/// Sample type for a PSD depth (1-bit is expanded to 8).
pub fn sample_for_depth(depth: u16) -> SampleType {
    match depth {
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => SampleType::U8,
    }
}

fn invert_sample(bytes: &mut [u8], s: SampleType) {
    match s {
        SampleType::U8 => bytes[0] = 255 - bytes[0],
        SampleType::U16 => {
            let v = u16::from_ne_bytes([bytes[0], bytes[1]]);
            bytes.copy_from_slice(&(65535 - v).to_ne_bytes());
        }
        SampleType::F32 => {
            let v = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            bytes.copy_from_slice(&(1.0 - v).to_ne_bytes());
        }
    }
}

/// Interleaves planes (big-endian samples, `n` pixels each). `None` planes
/// are filled with `fill[c]` (a native-endian encoded sample). Channels whose
/// `invert[c]` is set are inverted (PSD stores CMYK as `max - ink`).
pub fn interleave(planes: &[Option<&[u8]>], fill: &[Vec<u8>], n: usize, s: SampleType, invert: &[bool]) -> Vec<u8> {
    let bps = s.bytes();
    let ch = planes.len();
    let row = ch * bps;
    let mut out = vec![0u8; n * row];
    // Workers own disjoint output slices. Temporary per-worker images doubled the live
    // interleaved storage and required another full-image copy after every import.
    let bs = bands(n);
    let step = bs.first().map_or(1, |b| b.len()).max(1);
    if row == 0 {
        return out;
    }
    let chunks: Vec<_> = out.chunks_mut(step * row).enumerate().collect();
    par_map(chunks, |(band, part)| {
        for (i, p) in (band * step..band * step + part.len() / row).enumerate() {
            for (c, plane) in planes.iter().enumerate() {
                let dst = &mut part[(i * ch + c) * bps..(i * ch + c + 1) * bps];
                match plane {
                    Some(pl) if pl.len() >= (p + 1) * bps => {
                        let src = &pl[p * bps..(p + 1) * bps];
                        for k in 0..bps {
                            dst[k] = src[bps - 1 - k];
                        }
                        if cfg!(target_endian = "big") {
                            dst.copy_from_slice(src);
                        }
                        if invert[c] {
                            invert_sample(dst, s);
                        }
                    }
                    _ => dst.copy_from_slice(&fill[c]),
                }
            }
        }
    });
    out
}

/// Splits interleaved native-endian bytes into big-endian planes.
pub fn deinterleave(bytes: &[u8], ch: usize, s: SampleType, invert: &[bool]) -> Vec<Vec<u8>> {
    let bps = s.bytes();
    let n = bytes.len() / (ch * bps).max(1);
    if s == SampleType::U8 {
        // Fast path: one plane per thread, no per-sample copies.
        return par_map((0..ch).collect(), |c| {
            let it = bytes.iter().skip(c).step_by(ch).take(n);
            if invert[c] { it.map(|v| 255 - v).collect() } else { it.copied().collect() }
        });
    }
    // One band of rows per core, appended in band order into pre-grown planes.
    let row = ch * bps;
    let bs = bands(n);
    let parts: Vec<Vec<Vec<u8>>> = par_map(bs.clone(), |band| {
        let mut planes = vec![Vec::with_capacity(band.len() * bps); ch];
        let mut tmp = [0u8; 4];
        for p in band {
            for (c, plane) in planes.iter_mut().enumerate() {
                let src = &bytes[p * row + c * bps..p * row + (c + 1) * bps];
                tmp[..bps].copy_from_slice(src);
                if invert[c] {
                    invert_sample(&mut tmp[..bps], s);
                }
                if cfg!(target_endian = "big") {
                    plane.extend_from_slice(&tmp[..bps]);
                } else {
                    plane.extend(tmp[..bps].iter().rev());
                }
            }
        }
        planes
    });
    let mut planes: Vec<Vec<u8>> = (0..ch).map(|_| Vec::with_capacity(n * bps)).collect();
    for part in parts {
        for (c, plane) in planes.iter_mut().enumerate() {
            plane.extend_from_slice(&part[c]);
        }
    }
    planes
}

/// Native-endian encoding of the maximum sample value.
pub fn max_sample(s: SampleType) -> Vec<u8> {
    match s {
        SampleType::U8 => vec![255],
        SampleType::U16 => 65535u16.to_ne_bytes().to_vec(),
        SampleType::F32 => 1.0f32.to_ne_bytes().to_vec(),
    }
}

/// Top of Photoshop's 16-bit Lab a*/b* scale: it stores `32768 + 256·a` (0..65280 spans −128..127,
/// fitted on psd-tools `4x4_16bit_lab`: the merged image holds a stop's `LbCl` a* = 52.29 as 46154),
/// while documents keep a*/b* on the 8-bit scale `(a + 128) / 255` at every depth.
pub const LAB16_CHROMA_MAX: f32 = 65280.0;

/// Rescales the a*/b* channels (1 and 2) of interleaved native-endian 16-bit Lab pixels with
/// `ch` channels: PSD → document when `to_doc`, else document → PSD. Round-trips exactly.
pub fn lab16_chroma(bytes: &mut [u8], ch: usize, to_doc: bool) {
    if ch < 3 {
        return;
    }
    let k = if to_doc { 65535.0 / f64::from(LAB16_CHROMA_MAX) } else { f64::from(LAB16_CHROMA_MAX) / 65535.0 };
    for px in bytes.chunks_exact_mut(ch * 2) {
        for c in 1..3 {
            let v = u16::from_ne_bytes([px[2 * c], px[2 * c + 1]]);
            let w = (f64::from(v) * k).round().min(65535.0) as u16;
            px[2 * c..2 * c + 2].copy_from_slice(&w.to_ne_bytes());
        }
    }
}

/// Native-endian encoding of zero.
pub fn zero_sample(s: SampleType) -> Vec<u8> {
    vec![0; s.bytes()]
}

/// Quantizes a normalized value to big-endian bytes.
pub fn encode_be(v: f32, s: SampleType, out: &mut Vec<u8>) {
    match s {
        SampleType::U8 => out.push((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8),
        SampleType::U16 => out.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes()),
        SampleType::F32 => out.extend_from_slice(&v.to_be_bytes()),
    }
}

/// Decodes a big-endian sample at `index` to a normalized value.
#[cfg(test)]
pub fn decode_be(bytes: &[u8], index: usize, s: SampleType) -> f32 {
    match s {
        SampleType::U8 => f32::from(bytes[index]) / 255.0,
        SampleType::U16 => f32::from(u16::from_be_bytes([bytes[index * 2], bytes[index * 2 + 1]])) / 65535.0,
        SampleType::F32 => {
            let o = index * 4;
            f32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        }
    }
}

/// Photoshop stores the merged image of a transparent document matted
/// against white: `m = c·a + w·(1 − a)` per stored channel, where `w` is the
/// model's white. These convert between straight and matted values.
pub fn matte(c: f32, a: f32, white: f32) -> f32 {
    c * a + white * (1.0 - a)
}

/// Inverse of [`matte`] (returns `white` where alpha is 0).
/// Keep HDR and negative samples; integer encoders own normalized-range clipping.
pub fn unmatte(m: f32, a: f32, white: f32) -> f32 {
    if a <= 0.0 { white } else { (m - white * (1.0 - a)) / a }
}

/// `items.map(f)` on scoped threads (one per item; callers pass a handful of channels or
/// bands), in order; sequential on wasm or for a single item.
pub fn par_map<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    if cfg!(target_arch = "wasm32") || items.len() < 2 {
        return items.into_iter().map(f).collect();
    }
    std::thread::scope(|sc| {
        let f = &f;
        let hs: Vec<_> = items.into_iter().map(|t| sc.spawn(move || f(t))).collect();
        // Join every worker first (an unjoined panicked thread would make the scope panic),
        // then hand a worker's panic to the caller exactly as the sequential path would, so
        // the shell's last-resort guard reports it instead of the scope aborting the join.
        let joined: Vec<_> = hs.into_iter().map(|h| h.join()).collect();
        joined.into_iter().map(|r| r.unwrap_or_else(|p| std::panic::resume_unwind(p))).collect()
    })
}

/// `0..n` split into about one band per core.
pub fn bands(n: usize) -> Vec<std::ops::Range<usize>> {
    let k = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |v| v.get()).clamp(1, 32) };
    let step = n.div_ceil(k).max(1);
    (0..n).step_by(step).map(|a| a..(a + step).min(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba8_tiles_match_planar_samples_across_negative_edges() {
        let rect = Rect::new(-3, -2, 270, 260);
        let n = rect.width() as usize * rect.height() as usize;
        let channels: Vec<Vec<u8>> = (0..4).map(|c| (0..n).map(|i| if c == 3 { 0 } else { (i.wrapping_mul(37) + c * 29) as u8 }).collect()).collect();
        let planes: Vec<_> = channels.iter().map(|p| Some(p.as_slice())).collect();
        let surface = rgba8_surface(&planes, rect).expect("valid planes");
        let expected: Vec<_> = (0..n).flat_map(|i| channels.iter().map(move |c| c[i])).collect();
        assert_eq!(surface.to_interleaved(rect), expected);
        assert!(surface.tile_count() > 0, "nonzero RGB beneath zero alpha must survive");
        let planes = [Some(channels[0].as_slice()), Some(channels[1].as_slice()), Some(channels[2].as_slice()), None];
        let opaque = rgba8_surface(&planes, rect).expect("missing alpha is opaque");
        assert!(opaque.to_interleaved(rect).chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn rgba8_tiles_prune_zero_data_and_reject_short_planes() {
        let rect = Rect::new(-1, 3, 258, 261);
        let zeros = vec![0; rect.width() as usize * rect.height() as usize];
        let planes = [Some(zeros.as_slice()); 4];
        assert_eq!(rgba8_surface(&planes, rect).expect("valid zeros").tile_count(), 0);
        let short = [Some(&zeros[..zeros.len() - 1]); 4];
        assert!(rgba8_surface(&short, rect).is_none());
        assert!(rgba8_surface(&[None; 4], rect).is_none());
    }

    #[test]
    fn direct_tiles_match_general_path_at_every_depth_and_supported_model() {
        let rect = Rect::new(-3, -2, 270, 260);
        let n = rect.width() as usize * rect.height() as usize;
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat::new(mode, sample, true);
                let channels = format.channels();
                let values: Vec<Vec<u8>> = (0..channels)
                    .map(|c| {
                        (0..n)
                            .flat_map(|i| match sample {
                                SampleType::U8 => vec![(i.wrapping_mul(37) + c * 29) as u8],
                                SampleType::U16 => ((i.wrapping_mul(257) + c * 1999) as u16).to_be_bytes().to_vec(),
                                SampleType::F32 => ((i % 257) as f32 / 128.0 - 0.3 + c as f32 * 0.07).to_be_bytes().to_vec(),
                            })
                            .collect()
                    })
                    .collect();
                // Full planes, missing alpha, and missing color (plus alpha when another color exists).
                for missing in 0..3 {
                    let mut planes: Vec<_> = values.iter().map(|v| Some(v.as_slice())).collect();
                    if missing == 1 || (missing == 2 && channels > 2) {
                        planes[channels - 1] = None;
                    }
                    if missing >= 2 {
                        planes[0] = None;
                    }
                    let fill: Vec<_> = (0..channels).map(|c| if c + 1 == channels { max_sample(sample) } else { zero_sample(sample) }).collect();
                    let invert: Vec<_> = (0..channels).map(|c| mode == ColorMode::Cmyk && c + 1 < channels).collect();
                    let mut expected = interleave(&planes, &fill, n, sample, &invert);
                    if mode == ColorMode::Lab && sample == SampleType::U16 {
                        lab16_chroma(&mut expected, channels, true);
                    }
                    let mut reference = Surface::from_interleaved(format, rect, &expected);
                    reference.prune();
                    let direct = planar_surface(&planes, rect, format).expect("supported exact planes");
                    assert_eq!(direct.to_interleaved(rect), expected, "{format:?}, missing {missing}");
                    assert_eq!(direct.tile_count(), reference.tile_count(), "{format:?}, missing {missing}");
                    for (coord, tile) in reference.tiles() {
                        assert_eq!(direct.tile(*coord).unwrap().bytes(), tile.bytes(), "{format:?}, missing {missing}, {coord:?}: includes padding");
                    }
                }
                let zeros = vec![0; n * sample.bytes()];
                let planes = vec![Some(zeros.as_slice()); channels];
                let direct = planar_surface(&planes, rect, format).unwrap();
                if mode != ColorMode::Cmyk {
                    assert_eq!(direct.tile_count(), 0, "{format:?}");
                }
                let mut short = planes.clone();
                short[0] = Some(&zeros[..zeros.len() - 1]);
                assert!(planar_surface(&short, rect, format).is_none());
                let long = vec![0; zeros.len() + 1];
                short[0] = Some(&long);
                assert!(planar_surface(&short, rect, format).is_none());
            }
        }
    }

    #[test]
    fn direct_float_tiles_preserve_ieee_bits_and_hidden_color() {
        let rect = Rect::new(-1, 255, 4, 256);
        let bits = [0x8000_0000u32, 0x7fc0_1234, 0x7f80_0000, 0xff80_0000, 0x4080_0000];
        let color: Vec<_> = bits.into_iter().flat_map(u32::to_be_bytes).collect();
        let alpha = vec![0; bits.len() * 4];
        let planes = [Some(color.as_slice()), Some(color.as_slice()), Some(color.as_slice()), Some(alpha.as_slice())];
        let direct = planar_surface(&planes, rect, PixelFormat::RGBA32F).unwrap();
        let expected = interleave(&planes, &vec![zero_sample(SampleType::F32); 4], bits.len(), SampleType::F32, &[false; 4]);
        assert_eq!(direct.to_interleaved(rect), expected);
        assert!(direct.tile_count() > 0, "hidden IEEE bit patterns cannot be pruned by alpha");
        assert!(planar_surface(&planes[..3], rect, PixelFormat::RGBA32F).is_none());
        assert!(planar_surface(&[None; 4], rect, PixelFormat::RGBA32F).is_none());
        assert!(planar_surface(&planes, rect, PixelFormat::new(ColorMode::Lab, SampleType::F32, false)).is_none());
    }

    #[test]
    fn banded_shuffles_match_a_sequential_reference() {
        fn seq_interleave(planes: &[Option<&[u8]>], fill: &[Vec<u8>], n: usize, s: SampleType, invert: &[bool]) -> Vec<u8> {
            let bps = s.bytes();
            let ch = planes.len();
            let mut out = vec![0u8; n * ch * bps];
            for (c, plane) in planes.iter().enumerate() {
                for i in 0..n {
                    let dst = &mut out[(i * ch + c) * bps..(i * ch + c + 1) * bps];
                    match plane {
                        Some(p) if p.len() >= (i + 1) * bps => {
                            let src = &p[i * bps..(i + 1) * bps];
                            for k in 0..bps {
                                dst[k] = src[bps - 1 - k];
                            }
                            if invert[c] {
                                invert_sample(dst, s);
                            }
                        }
                        _ => dst.copy_from_slice(&fill[c]),
                    }
                }
            }
            out
        }
        fn seq_deinterleave(bytes: &[u8], ch: usize, s: SampleType, invert: &[bool]) -> Vec<Vec<u8>> {
            let bps = s.bytes();
            let n = bytes.len() / (ch * bps).max(1);
            let mut planes = vec![Vec::with_capacity(n * bps); ch];
            let mut tmp = [0u8; 4];
            for i in 0..n {
                for (c, plane) in planes.iter_mut().enumerate() {
                    let src = &bytes[(i * ch + c) * bps..(i * ch + c + 1) * bps];
                    tmp[..bps].copy_from_slice(src);
                    if invert[c] {
                        invert_sample(&mut tmp[..bps], s);
                    }
                    plane.extend(tmp[..bps].iter().rev());
                }
            }
            planes
        }
        // Odd pixel counts cross band boundaries; a missing plane exercises the fill path.
        for n in [1usize, 7, 64, 1000] {
            for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let bps = s.bytes();
                let planes: Vec<Vec<u8>> = (0..4).map(|c| (0..n * bps).map(|i| (i * 13 + c * 5) as u8).collect()).collect();
                let mut refs: Vec<Option<&[u8]>> = planes.iter().map(|p| Some(&p[..])).collect();
                refs[2] = None;
                let fill: Vec<Vec<u8>> = (0..4).map(|c| vec![c as u8 * 40; bps]).collect();
                let invert = [false, true, false, true];
                let got = interleave(&refs, &fill, n, s, &invert);
                let want = seq_interleave(&refs, &fill, n, s, &invert);
                assert_eq!(got, want, "interleave n={n}");
                let planes_back = deinterleave(&got, 4, s, &invert);
                let want_back = seq_deinterleave(&got, 4, s, &invert);
                assert_eq!(planes_back, want_back, "deinterleave n={n}");
            }
        }
    }

    #[test]
    fn interleave_roundtrip_all_depths() {
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bps = s.bytes();
            let planes: Vec<Vec<u8>> = (0..3).map(|c| (0..4 * bps).map(|i| (i * 7 + c * 3) as u8).collect()).collect();
            let refs: Vec<Option<&[u8]>> = planes.iter().map(|p| Some(&p[..])).collect();
            let fill = vec![zero_sample(s); 3];
            let inv = [false, true, false];
            let il = interleave(&refs, &fill, 4, s, &inv);
            let back = deinterleave(&il, 3, s, &inv);
            if s == SampleType::F32 {
                // 1 - (1 - v) may not be exact for floats; only check non-inverted planes.
                assert_eq!(back[0], planes[0]);
                assert_eq!(back[2], planes[2]);
            } else {
                assert_eq!(back, planes);
            }
        }
    }

    #[test]
    fn missing_planes_use_fill() {
        let il = interleave(&[None, Some(&[5, 6])], &[vec![9], vec![0]], 2, SampleType::U8, &[false, false]);
        assert_eq!(il, vec![9, 5, 9, 6]);
    }

    #[test]
    fn lab16_chroma_scale_round_trips() {
        // Two pixels of three channels: L is untouched, a*/b* rescale, and every PSD value
        // comes back exactly.
        let mut all = Vec::new();
        for v in 0..=u16::MAX {
            all.extend_from_slice(&v.to_ne_bytes());
            all.extend_from_slice(&v.to_ne_bytes());
            all.extend_from_slice(&v.to_ne_bytes());
        }
        let orig = all.clone();
        lab16_chroma(&mut all, 3, true);
        let px = |b: &[u8], i: usize, c: usize| u16::from_ne_bytes([b[(i * 3 + c) * 2], b[(i * 3 + c) * 2 + 1]]);
        // Neutral (32768 in PSD) is 128 / 255 of the document scale; 65280 (a* = 127) is the top.
        assert_eq!(px(&all, 32768, 0), 32768);
        assert_eq!(px(&all, 32768, 1), (32768.0f64 * 65535.0 / 65280.0).round() as u16);
        assert_eq!(px(&all, 65280, 2), 65535);
        lab16_chroma(&mut all, 3, false);
        for i in 0..=65280usize {
            assert_eq!(px(&all, i, 1), px(&orig, i, 1), "{i}");
        }
        // Short or odd buffers are left alone.
        let mut short = vec![1u8, 2, 3];
        lab16_chroma(&mut short, 3, true);
        lab16_chroma(&mut short, 1, true);
        assert_eq!(short, [1, 2, 3]);
    }

    #[test]
    fn be_codec() {
        let mut v = Vec::new();
        encode_be(1.0, SampleType::U16, &mut v);
        assert_eq!(v, vec![0xff, 0xff]);
        assert_eq!(decode_be(&v, 0, SampleType::U16), 1.0);
        let mut f = Vec::new();
        encode_be(2.5, SampleType::F32, &mut f);
        assert_eq!(decode_be(&f, 0, SampleType::F32), 2.5);
    }
}
