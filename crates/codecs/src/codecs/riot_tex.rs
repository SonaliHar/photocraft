//! Riot TEX container (RitoTex's public 12-byte specification).
//! Mips are stored smallest first. Block compression uses the CPU Rust port of
//! Intel's encoder; dimensions and buffers are validated before calling it.

use crate::options::alloc_zeroed;
use crate::{ChannelLayout, CodecError, EncodeOptions, Format, Image, Limits, SampleType};
use block_compression::{BC7Settings, CompressionVariant, decode::decompress_blocks_as_rgba8, encode::compress_rgba8};

/// Riot's on-disk pixel encodings. BC5 stores red/green only; BC1 stores RGB only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TexCompression {
    Bc1,
    Bc3,
    Bc5,
    Bc7,
    #[default]
    Bgra8,
}

impl TexCompression {
    pub const ALL: [Self; 5] = [Self::Bc1, Self::Bc3, Self::Bc5, Self::Bc7, Self::Bgra8];
    pub fn code(self) -> u8 {
        match self {
            Self::Bc1 => 10,
            Self::Bc3 => 12,
            Self::Bc7 => 13,
            Self::Bc5 => 14,
            Self::Bgra8 => 20,
        }
    }
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            10 => Some(Self::Bc1),
            12 => Some(Self::Bc3),
            13 => Some(Self::Bc7),
            14 => Some(Self::Bc5),
            20 => Some(Self::Bgra8),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Bc1 => "BC1 (RGB)",
            Self::Bc3 => "BC3 (RGBA)",
            Self::Bc5 => "BC5 (RG)",
            Self::Bc7 => "BC7 (RGBA)",
            Self::Bgra8 => "BGRA8 (lossless)",
        }
    }
    fn variant(self) -> Option<CompressionVariant> {
        match self {
            Self::Bc1 => Some(CompressionVariant::BC1),
            Self::Bc3 => Some(CompressionVariant::BC3),
            Self::Bc5 => Some(CompressionVariant::BC5),
            Self::Bc7 => Some(CompressionVariant::BC7(BC7Settings::alpha_basic())),
            Self::Bgra8 => None,
        }
    }
    fn size(self, w: u32, h: u32) -> Result<usize, CodecError> {
        let n = if self == Self::Bgra8 {
            u64::from(w) * u64::from(h) * 4
        } else {
            u64::from(w.div_ceil(4)) * u64::from(h.div_ceil(4)) * if self == Self::Bc1 { 8 } else { 16 }
        };
        usize::try_from(n).map_err(|_| bad("texture size exceeds address space"))
    }
}

/// Header settings, reusable by Save without guessing from decoded colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexInfo {
    pub width: u32,
    pub height: u32,
    pub compression: TexCompression,
    pub mipmaps: bool,
}

fn bad(message: &str) -> CodecError {
    CodecError::malformed(Format::RiotTex, message)
}

pub fn info(bytes: &[u8]) -> Result<TexInfo, CodecError> {
    let h: &[u8; 12] = bytes.get(..12).and_then(|s| s.try_into().ok()).ok_or_else(|| bad("truncated TEX header"))?;
    if !h.starts_with(b"TEX\0") {
        return Err(bad("invalid TEX signature"));
    }
    let width = u32::from(u16::from_le_bytes([h[4], h[5]]));
    let height = u32::from(u16::from_le_bytes([h[6], h[7]]));
    if width == 0 || height == 0 {
        return Err(bad("zero texture dimensions"));
    }
    let compression = TexCompression::from_code(h[9]).ok_or_else(|| CodecError::unsupported(Format::RiotTex, format!("TEX encoding 0x{:02X}", h[9])))?;
    if h[11] > 1 {
        return Err(bad("invalid mipmap flag"));
    }
    Ok(TexInfo { width, height, compression, mipmaps: h[11] != 0 })
}

fn levels(w: u32, h: u32, mipmaps: bool) -> Vec<(u32, u32)> {
    let (mut w, mut h) = (w, h);
    let mut out = vec![(w, h)];
    while mipmaps && (w > 1 || h > 1) {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        out.push((w, h));
    }
    out
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let i = info(bytes)?;
    limits.check(i.width, i.height, ChannelLayout::Rgba, SampleType::U8)?;
    let sizes = levels(i.width, i.height, i.mipmaps).into_iter().map(|(w, h)| i.compression.size(w, h)).collect::<Result<Vec<_>, _>>()?;
    let total = sizes.iter().try_fold(12usize, |a, b| a.checked_add(*b)).ok_or_else(|| bad("texture length overflow"))?;
    if bytes.len() != total {
        return Err(bad("TEX payload length does not match dimensions and mip chain"));
    }
    let base_size = i.compression.size(i.width, i.height)?;
    let payload = bytes.get(total - base_size..).ok_or_else(|| bad("missing base level"))?;
    let mut rgba = alloc_zeroed(i.width as usize * i.height as usize * 4)?;
    if let Some(variant) = i.compression.variant() {
        let bw = i.width.div_ceil(4) as usize;
        let block_size = if i.compression == TexCompression::Bc1 { 8 } else { 16 };
        for (idx, block) in payload.chunks_exact(block_size).enumerate() {
            let mut tile = [0u8; 64];
            decompress_blocks_as_rgba8(variant, 4, 4, block, &mut tile);
            if i.compression == TexCompression::Bc5 {
                for pixel in tile.as_chunks_mut::<4>().0 {
                    pixel[2] = 0;
                    pixel[3] = 255;
                }
            }
            let (x0, y0) = ((idx % bw) * 4, (idx / bw) * 4);
            for (row, source) in tile.chunks_exact(16).enumerate().take((i.height as usize - y0).min(4)) {
                let n = (i.width as usize - x0).min(4) * 4;
                let start = ((y0 + row) * i.width as usize + x0) * 4;
                let dest = rgba.get_mut(start..start + n).ok_or_else(|| bad("invalid block destination"))?;
                dest.copy_from_slice(source.get(..n).ok_or_else(|| bad("invalid block row"))?);
            }
        }
    } else {
        for (src, dst) in payload.as_chunks::<4>().0.iter().zip(rgba.as_chunks_mut::<4>().0) {
            *dst = [src[2], src[1], src[0], src[3]];
        }
    }
    Image::from_u8(i.width, i.height, ChannelLayout::Rgba, rgba)
}

pub(crate) fn encode(image: &Image, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 || w > 65535 || h > 65535 {
        return Err(CodecError::encode(Format::RiotTex, "TEX dimensions must be 1..65535"));
    }
    let compression = opts.tex_compression;
    if compression != TexCompression::Bgra8 && (!w.is_multiple_of(4) || !h.is_multiple_of(4)) {
        return Err(CodecError::encode(Format::RiotTex, "BC textures require dimensions divisible by 4; use BGRA8 for other dimensions"));
    }
    Limits::default().check(w, h, ChannelLayout::Rgba, SampleType::U8)?;
    let rgba = image.convert(ChannelLayout::Rgba, SampleType::U8);
    if compression == TexCompression::Bc1 && rgba.has_translucency() {
        return Err(CodecError::encode(Format::RiotTex, "BC1 cannot retain this alpha channel; choose BC3, BC7 or BGRA8"));
    }
    let dims = levels(w, h, opts.tex_mipmaps);
    let mut encoded = Vec::new();
    let mut pixels = rgba.into_data();
    for (index, &(mw, mh)) in dims.iter().enumerate() {
        if index > 0 {
            let &(pw, ph) = dims.get(index - 1).ok_or_else(|| bad("missing parent mip"))?;
            pixels = downsample(&pixels, pw, ph, mw, mh)?;
        }
        let mut mip = alloc_zeroed(compression.size(mw, mh)?)?;
        if let Some(variant) = compression.variant() {
            // Each block is edge-padded, including the 2x2 and 1x1 tail mips.
            let bw = mw.div_ceil(4) as usize;
            let bs = if compression == TexCompression::Bc1 { 8 } else { 16 };
            let encode_block = |(idx, block): (usize, &mut [u8])| -> Result<(), CodecError> {
                let mut tile = [0u8; 64];
                let (x0, y0) = ((idx % bw) * 4, (idx / bw) * 4);
                for (p, dst) in tile.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let x = (x0 + p % 4).min(mw as usize - 1);
                    let y = (y0 + p / 4).min(mh as usize - 1);
                    let offset = (y * mw as usize + x) * 4;
                    dst.copy_from_slice(pixels.get(offset..offset + 4).ok_or_else(|| bad("invalid source pixel"))?);
                }
                compress_rgba8(variant, &tile, block, 4, 4, 16);
                Ok(())
            };
            #[cfg(not(target_arch = "wasm32"))]
            {
                use rayon::prelude::*;
                mip.par_chunks_mut(bs).enumerate().try_for_each(encode_block)?;
            }
            #[cfg(target_arch = "wasm32")]
            {
                mip.chunks_mut(bs).enumerate().try_for_each(encode_block)?;
            }
        } else {
            for (src, dst) in pixels.as_chunks::<4>().0.iter().zip(mip.as_chunks_mut::<4>().0) {
                *dst = [src[2], src[1], src[0], src[3]];
            }
        }
        encoded.push(mip);
    }
    let total = encoded.iter().try_fold(12usize, |a, b| a.checked_add(b.len())).ok_or_else(|| bad("encoded size overflow"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| CodecError::LimitExceeded("cannot allocate TEX output".into()))?;
    out.extend_from_slice(b"TEX\0");
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.extend_from_slice(&[1, compression.code(), 0, u8::from(opts.tex_mipmaps)]);
    for mip in encoded.into_iter().rev() {
        out.extend_from_slice(&mip);
    }
    Ok(out)
}

// Filter each channel independently: texture alpha and RG channels can contain game data.
fn downsample(src: &[u8], pw: u32, ph: u32, w: u32, h: u32) -> Result<Vec<u8>, CodecError> {
    let mut out = alloc_zeroed(w as usize * h as usize * 4)?;
    for (idx, pixel) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (idx % w as usize, idx / w as usize);
        let (x0, x1) = (x * pw as usize / w as usize, (x + 1) * pw as usize / w as usize);
        let (y0, y1) = (y * ph as usize / h as usize, (y + 1) * ph as usize / h as usize);
        let mut sums = [0u32; 4];
        let mut count = 0;
        for sy in y0..y1 {
            for sx in x0..x1 {
                let offset = (sy * pw as usize + sx) * 4;
                let p = src.get(offset..offset + 4).ok_or_else(|| bad("invalid mip source"))?;
                for (s, v) in sums.iter_mut().zip(p) {
                    *s += u32::from(*v);
                }
                count += 1;
            }
        }
        if count == 0 {
            return Err(bad("empty mip footprint"));
        }
        for (dst, sum) in pixel.iter_mut().zip(sums) {
            *dst = ((sum + count / 2) / count) as u8;
        }
    }
    Ok(out)
}
