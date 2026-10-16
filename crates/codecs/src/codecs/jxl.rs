//! JPEG XL, read and written.
//!
//! Both directions live in the optional `photocraft-jxl` crate (jxl-oxide, a pure-Rust decoder of
//! the whole specification, and jixel, a pure-Rust encoder), enabled by this crate's `jxl`
//! feature. Without it, `.jxl` files are still detected and opening or writing one is a
//! [`CodecError::Unsupported`] error, never a panic.
//!
//! Writing is lossy (VarDCT) or lossless (Modular) by [`EncodeOptions::jxl_lossless`], with the
//! ICC profile and the EXIF and XMP (in the container) when the options embed them. Every frame of
//! an animation comes from [`decode_frames`], and a JPEG can be recompressed losslessly and rebuilt
//! byte for byte ([`recompress_jpeg`], [`reconstruct_jpeg`]).
//!
//! JPEG XL records orientation in the codestream header, not in EXIF, so the decoder applies it
//! and the EXIF Orientation is rewritten to 1: a file's EXIF tag only mirrors what the header
//! already says, and applying it as well would turn the picture twice. With
//! `keep_orientation` the upright pixels are turned back to how they were coded.

use crate::Format;
use crate::codecs::gif_frames::{AnimationFrame, MAX_GIF_FRAMES};
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::Image;
use crate::options::{DecodeOptions, EncodeOptions, Limits};

const F: Format = Format::Jxl;

/// The reason given when this build has no JPEG XL decoder.
#[cfg(not(feature = "jxl"))]
pub(crate) const NOT_IN_BUILD: &str = "JPEG XL support isn't included in this build of PhotoCraft";

#[cfg(not(feature = "jxl"))]
pub(crate) fn decode(_bytes: &[u8], _limits: &Limits, _keep_orientation: bool) -> Result<Image, CodecError> {
    Err(CodecError::unsupported(F, NOT_IN_BUILD))
}

#[cfg(not(feature = "jxl"))]
pub(crate) fn decode_frames(_bytes: &[u8], _opts: &DecodeOptions) -> Result<Vec<AnimationFrame>, CodecError> {
    let _ = MAX_GIF_FRAMES;
    Err(CodecError::unsupported(F, NOT_IN_BUILD))
}

#[cfg(not(feature = "jxl"))]
pub(crate) fn encode(_image: &Image, _plan: Plan, _opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::unsupported(F, NOT_IN_BUILD))
}

#[cfg(not(feature = "jxl"))]
pub(crate) fn recompress_jpeg(_jpeg: &[u8], _limits: &Limits) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::unsupported(F, NOT_IN_BUILD))
}

#[cfg(not(feature = "jxl"))]
pub(crate) fn reconstruct_jpeg(_bytes: &[u8], _limits: &Limits) -> Result<Option<Vec<u8>>, CodecError> {
    Err(CodecError::unsupported(F, NOT_IN_BUILD))
}

#[cfg(feature = "jxl")]
fn err(e: photocraft_jxl::Error) -> CodecError {
    match e {
        photocraft_jxl::Error::Unsupported(what) => CodecError::unsupported(F, what),
        photocraft_jxl::Error::Limit(what) => CodecError::LimitExceeded(what),
        photocraft_jxl::Error::Malformed(what) => CodecError::malformed(F, what),
    }
}

#[cfg(feature = "jxl")]
fn layout(l: photocraft_jxl::Layout) -> crate::image::ChannelLayout {
    use crate::image::ChannelLayout as L;
    match l {
        photocraft_jxl::Layout::Gray => L::Gray,
        photocraft_jxl::Layout::GrayA => L::GrayA,
        photocraft_jxl::Layout::Rgb => L::Rgb,
        photocraft_jxl::Layout::Rgba => L::Rgba,
    }
}

/// Our sample type for the decoder's: 16-bit floats come back as F16, everything else as the
/// decoder hands it out.
#[cfg(feature = "jxl")]
fn sample(s: photocraft_jxl::Sample, float: bool, bits: u32) -> crate::image::SampleType {
    use crate::image::SampleType as S;
    match s {
        photocraft_jxl::Sample::U8 => S::U8,
        photocraft_jxl::Sample::U16 => S::U16,
        photocraft_jxl::Sample::F32 if float && bits <= 16 => S::F16,
        photocraft_jxl::Sample::F32 => S::F32,
    }
}

/// The orientation that undoes `o` (EXIF numbering): 6 and 8 are each other's inverse, the
/// rest are their own.
#[cfg(feature = "jxl")]
fn inverse(o: u8) -> u16 {
    match o {
        6 => 8,
        8 => 6,
        o => u16::from(o),
    }
}

#[cfg(feature = "jxl")]
pub(crate) fn decode(bytes: &[u8], limits: &Limits, keep_orientation: bool) -> Result<Image, CodecError> {
    use crate::image::{DecodeWarning, Metadata, SampleType};

    // The header alone: the declared size is checked before any pixel is decoded. Orientations
    // 5–8 swap width and height, so both the coded and the upright size must fit, and float and
    // premultiplied-alpha images are rendered through a 32-bit float working buffer, so that
    // size is what the budget must hold.
    let decoder = photocraft_jxl::Decoder::open(bytes, limits.alloc_usize()).map_err(err)?;
    let info = decoder.info();
    let s = sample(info.sample, info.float, info.bits_per_sample);
    let working = if info.float || info.premultiplied { SampleType::F32 } else { s };
    limits.check(info.width, info.height, layout(info.layout), working)?;
    limits.check(info.coded_width, info.coded_height, layout(info.layout), working)?;

    let mut d = decoder.decode().map_err(err)?;
    let data = std::mem::take(&mut d.data);
    let mut img = image_of(&d, data)?;
    img.icc = d.icc;
    img.meta = Metadata {
        exif: d.exif.map(|e| if keep_orientation { e } else { crate::orientation::upright_exif(&e).into_owned() }),
        xmp: d.xmp.map(|x| if keep_orientation { x } else { crate::orientation::upright_xmp(&x).into_owned() }),
        ..Default::default()
    };
    // An animation opens at its first frame. A complete file says how many it held; a file cut
    // off after its first frame (or inside a later one) only that there were more.
    if d.frames > 1 {
        img.warnings.push(DecodeWarning::MoreFrames { total: Some(d.frames) });
    } else if d.animated && !d.complete {
        img.warnings.push(DecodeWarning::MoreFrames { total: None });
    }
    if keep_orientation && d.orientation != 1 {
        // Turning the pixels back as coded; the metadata stays as the file has it (`oriented`
        // would rewrite its Orientation to 1, which is right only for upright pixels).
        let (icc, meta, warnings) = (img.icc.take(), std::mem::take(&mut img.meta), std::mem::take(&mut img.warnings));
        img = img.oriented(inverse(d.orientation))?;
        img.icc = icc;
        img.meta = meta;
        img.warnings = warnings;
    }
    Ok(img)
}

/// The image of one decoded frame laid out as `d` says. Trusts the decoded output's shape, not
/// the header's.
#[cfg(feature = "jxl")]
fn image_of(d: &photocraft_jxl::Decoded, data: Vec<u8>) -> Result<Image, CodecError> {
    let (l, s) = (layout(d.layout), sample(d.sample, d.float, d.bits_per_sample));
    if s == crate::image::SampleType::F16 {
        let floats: Vec<f32> = data.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect();
        Image::from_normalized(d.width, d.height, l, s, &floats)
    } else {
        Image::from_raw(d.width, d.height, l, s, data)
    }
}

/// Every frame, upright, each a full picture with the file's profile; the first also carries the
/// file's metadata. The size and every frame's bytes together are checked against `opts.limits`.
#[cfg(feature = "jxl")]
pub(crate) fn decode_frames(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<AnimationFrame>, CodecError> {
    let limits = &opts.limits;
    let decoder = photocraft_jxl::Decoder::open(bytes, limits.alloc_usize()).map_err(err)?;
    let info = decoder.info();
    let working = if info.float || info.premultiplied { crate::image::SampleType::F32 } else { sample(info.sample, info.float, info.bits_per_sample) };
    limits.check(info.width, info.height, layout(info.layout), working)?;
    let anim = decoder.decode_frames(MAX_GIF_FRAMES).map_err(err)?;
    let first_meta = crate::image::Metadata {
        exif: anim.first.exif.as_deref().map(|e| crate::orientation::upright_exif(e).into_owned()),
        xmp: anim.first.xmp.as_deref().map(|x| crate::orientation::upright_xmp(x).into_owned()),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(anim.frames.len());
    for (k, f) in anim.frames.into_iter().enumerate() {
        let mut img = image_of(&anim.first, f.data)?;
        img.icc = anim.first.icc.clone();
        if k == 0 {
            img.meta = first_meta.clone();
            // A file cut off inside the animation gives the frames before the cut, as GIF does.
            if !anim.first.complete {
                img.warnings.push(crate::image::DecodeWarning::Truncated { format: F });
            }
        }
        out.push(AnimationFrame { image: img, delay_ms: f.duration_ms });
    }
    Ok(out)
}

#[cfg(feature = "jxl")]
fn input_layout(l: crate::image::ChannelLayout) -> Result<photocraft_jxl::Layout, CodecError> {
    use crate::image::ChannelLayout as L;
    match l {
        L::Gray => Ok(photocraft_jxl::Layout::Gray),
        L::GrayA => Ok(photocraft_jxl::Layout::GrayA),
        L::Rgb => Ok(photocraft_jxl::Layout::Rgb),
        L::Rgba => Ok(photocraft_jxl::Layout::Rgba),
        _ => Err(CodecError::unsupported(F, "JPEG XL export writes grayscale and RGB, with optional alpha")),
    }
}

/// Encodes `image` converted to `plan`: lossy or lossless with the options' quality, effort and
/// progression, the ICC profile when `opts.embed_icc`, and EXIF and XMP (upright, with the
/// resolution) when `opts.embed_metadata`.
#[cfg(feature = "jxl")]
pub(crate) fn encode(image: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    use crate::image::SampleType as S;
    let source_layout = image.layout();
    let image = image.converted(plan.layout, plan.sample);
    let sample = match image.sample_type() {
        S::U8 => photocraft_jxl::InputSample::U8,
        S::U16 => photocraft_jxl::InputSample::U16,
        S::F32 => photocraft_jxl::InputSample::F32,
        S::F16 => return Err(CodecError::encode(F, "16-bit float is written as 32-bit float; the plan should have converted it")),
    };
    let layout = input_layout(image.layout())?;
    // A profile of another colour model than the samples (the plan turned gray into RGB) is not
    // the samples' profile: leave it out (`fidelity_warnings_with` reports it dropped).
    let icc = image.icc.as_deref().filter(|_| opts.embed_icc && source_layout.same_model(plan.layout));
    let exif = opts.embed_metadata.then(|| image.meta.exif.as_deref().map(|e| crate::resolution::export_exif(e, image.meta.dpi))).flatten();
    let xmp = opts.embed_metadata.then(|| image.meta.xmp.as_deref().map(|x| crate::resolution::export_xmp(x, image.meta.dpi))).flatten();
    let input = photocraft_jxl::Input {
        width: image.width(),
        height: image.height(),
        layout,
        sample,
        data: image.data(),
        icc,
        exif: exif.as_deref(),
        xmp: xmp.as_deref().map(str::as_bytes),
    };
    let settings = photocraft_jxl::Settings {
        lossless: opts.jxl_lossless,
        quality: f32::from(opts.jxl_quality.clamp(1, 100)),
        effort: opts.jxl_effort.clamp(1, 9),
        progressive: opts.jxl_progressive,
    };
    photocraft_jxl::encode(&input, &settings).map_err(|e| match e {
        photocraft_jxl::Error::Limit(what) => CodecError::LimitExceeded(what),
        photocraft_jxl::Error::Unsupported(what) => CodecError::unsupported(F, what),
        photocraft_jxl::Error::Malformed(what) => CodecError::encode(F, what),
    })
}

#[cfg(feature = "jxl")]
pub(crate) fn recompress_jpeg(jpeg: &[u8], limits: &Limits) -> Result<Vec<u8>, CodecError> {
    photocraft_jxl::recompress_jpeg(jpeg, limits.alloc_usize()).map_err(|e| match e {
        photocraft_jxl::Error::Limit(what) => CodecError::LimitExceeded(what),
        photocraft_jxl::Error::Unsupported(what) => CodecError::unsupported(Format::Jpeg, what),
        photocraft_jxl::Error::Malformed(what) => CodecError::malformed(Format::Jpeg, what),
    })
}

#[cfg(feature = "jxl")]
pub(crate) fn reconstruct_jpeg(bytes: &[u8], limits: &Limits) -> Result<Option<Vec<u8>>, CodecError> {
    photocraft_jxl::reconstruct_jpeg(bytes, &photocraft_jxl::Options { alloc_limit: limits.alloc_usize() }).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARE: &[u8] = &[0xFF, 0x0A, 0x30, 0x54];
    const CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n\0\0\0\x14ftypjxl \0\0\0\0jxl ";

    #[cfg(not(feature = "jxl"))]
    #[test]
    fn without_the_feature_jxl_is_a_clear_unsupported_error() {
        for header in [BARE, CONTAINER] {
            assert_eq!(crate::detect(header), Some(F), "still recognised");
            let r = crate::decode(header);
            assert!(matches!(&r, Err(CodecError::Unsupported { format: Format::Jxl, reason }) if reason.contains("isn't included in this build")), "{r:?}");
        }
        assert!(!crate::caps(F).read);
    }

    #[cfg(feature = "jxl")]
    #[test]
    fn with_the_feature_jxl_is_readable_and_a_bare_header_is_malformed() {
        assert!(crate::caps(F).read);
        for header in [BARE, CONTAINER] {
            let r = crate::decode(header);
            assert!(matches!(&r, Err(CodecError::Malformed { format: Format::Jxl, .. })), "{r:?}");
        }
        assert!(matches!(crate::decode_as(F, b""), Err(CodecError::Malformed { .. })));
    }

    #[cfg(feature = "jxl")]
    #[test]
    fn inverse_orientation_undoes_each_one() {
        for o in 1..=8u8 {
            let img = crate::image::Image::from_u8(3, 2, crate::image::ChannelLayout::Gray, vec![1, 2, 3, 4, 5, 6]).unwrap();
            let back = img.clone().oriented(u16::from(o)).unwrap().oriented(inverse(o)).unwrap();
            assert_eq!(back.data(), img.data(), "orientation {o}");
        }
    }
}
