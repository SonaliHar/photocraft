//! JPEG 2000 (ISO/IEC 15444-1): the JP2 file format and the raw J2K/J2C codestreams, plus
//! the Part 2 JPX/JPF and Part 6 JPM wrappers, which carry the same single codestream.
//!
//! Built on the pure-Rust `j2k` crate (no C, no build script), so it compiles for wasm with
//! the rest of L0.
//!
//! Reading: the codestream's SIZ marker gives dimensions, component count, precision and
//! signedness; the JP2 `colr` box gives the colour model — an enumerated method-1 box, or an
//! embedded ICC profile, which we keep — and `cdef` marks the opacity channel. Samples come
//! back as one plane per component, which are interleaved into the layout the components
//! describe: a third-party CMYK or CMYK+alpha stream opens as such, because its colour space
//! says so, while one that only says "ICC" is read as gray or RGB by its component count.
//!
//! Writing: one codestream, plus the signature and header boxes for the file wrappers.
//! [`EncodeOptions::jp2_quality`] of 100 — the default, and what every caller that has not
//! asked for a smaller file gets — selects the reversible 5/3 integer wavelet, which is exact:
//! the reason to pick JPEG 2000. Anything lower uses the irreversible 9/7 wavelet with a
//! bits-per-pixel rate target, which is ordinary Part 1 (not HTJ2K), so the result stays a plain
//! `.jp2` that any Part 1 reader opens. The irreversible path carries Part 1's colour transform,
//! which is defined for three colour components, so it is used only for gray and RGB; `caps`
//! keeps CMYK and Lab out of the family, and they are converted to RGB with the usual warning.

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, Limits};

use j2k::{
    J2kChannelAssociation, J2kChannelDefinition, J2kChannelType, J2kColorSpec, J2kDecoder, J2kEncodeValidation, J2kFileBoxMetadata, J2kFileColorSpec,
    J2kFileWrapOptions, J2kLosslessEncodeOptions, J2kLosslessSamples, J2kLossyEncodeOptions, J2kLossySamples, J2kRateTarget, encode_j2k_lossless,
    encode_j2k_lossy, wrap_j2k_codestream,
};
use j2k_core::Colorspace;

/// The format reported in errors: every JPEG 2000 wrapper shares one decoder and encoder, so
/// a message that names the family reads better than one naming the extension that was lost.
const F: Format = Format::Jp2;

/// JPEG 2000 Part 1 stores integers of 1..=38 bits; more than 16 does not fit our sample
/// types, and no Part 1 writer produces it.
const MAX_SUPPORTED_BITS: u8 = 16;

/// Decode a JP2/JPH file or a raw J2K/J2C codestream.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let header = J2kDecoder::inspect_support(bytes).map_err(|e| CodecError::malformed(F, format!("JPEG 2000 header: {e}")))?;
    let info = &header.info;
    let (width, height) = info.dimensions;
    let layout = layout_for(info.colorspace, info.components)?;
    let sample = sample_for(info.bit_depth)?;

    limits.check(width, height, layout, sample)?;
    // A codestream can beat the raw samples by orders of magnitude on flat content, so the
    // bomb guard has to be generous (as it is for WebP): a 65 536:1 bound still rejects a
    // header-only file that claims gigabytes, while `Limits::check` above bounds the buffer.
    Limits::check_plausible(
        u64::from(width) * u64::from(height) * (layout.channels() * usize::from(sample.bytes())) as u64,
        bytes.len(),
        1 << 16,
    )?;

    let mut decoder = J2kDecoder::new(bytes).map_err(|e| CodecError::malformed(F, format!("JPEG 2000: {e}")))?;
    let planes = decoder.decode_components().map_err(|e| CodecError::malformed(F, format!("JPEG 2000 decode: {e}")))?;
    let channels = layout.channels();
    if planes.planes().len() < channels {
        return Err(CodecError::malformed(F, format!("JPEG 2000 decode produced {} of {channels} components", planes.planes().len())));
    }

    let data = interleave(planes.planes(), width, height, layout, sample)?;
    Ok(Image::from_raw(width, height, layout, sample, data)?.with_meta(Metadata::default()).with_icc(icc_profile(&header)))
}

/// Encode as a JP2 file (the `.jp2`/`.jpx`/`.jpf`/`.jpm` wrappers all write this container:
/// they differ only in the Part they belong to, which we do not use).
pub fn encode(image: &Image, format: Format, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let codestream = codestream(image, format, plan, opts)?;
    wrap(image, plan, &codestream)
}

/// Encode as a raw J2K/J2C codestream: the same bitstream without the JP2 boxes.
pub fn encode_j2k(image: &Image, format: Format, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    codestream(image, format, plan, opts)
}

/// The colour model a component count and `colr` box describe. The extra component beyond
/// a colour model is the opacity channel, which is where `cdef` points at it.
fn layout_for(colorspace: Colorspace, components: u16) -> Result<ChannelLayout, CodecError> {
    let layout = match (colorspace, components) {
        // An ICC-coloured stream does not name its model, so the component count decides.
        (Colorspace::IccTagged, 1) | (Colorspace::Grayscale, 1) | (Colorspace::SGray, 1) => ChannelLayout::Gray,
        (Colorspace::IccTagged, 2) | (Colorspace::Grayscale, 2) | (Colorspace::SGray, 2) => ChannelLayout::GrayA,
        (Colorspace::Cmyk, 4) | (Colorspace::Ycck, 4) => ChannelLayout::Cmyk,
        (Colorspace::Cmyk, 5) | (Colorspace::Ycck, 5) => ChannelLayout::CmykA,
        (_, 1) => ChannelLayout::Gray,
        (_, 2) => ChannelLayout::GrayA,
        (_, 3) => ChannelLayout::Rgb,
        (_, 4) => ChannelLayout::Rgba,
        (_, 5) => ChannelLayout::CmykA,
        (colorspace, n) => {
            return Err(CodecError::unsupported(F, format!("{n}-component JPEG 2000 images ({colorspace:?}) cannot be mapped to a channel layout")));
        }
    };
    Ok(layout)
}

/// 8-bit components decode as U8, wider ones as U16 (a 12-bit stream widens without loss).
fn sample_for(bit_depth: u8) -> Result<SampleType, CodecError> {
    match bit_depth {
        0 => Err(CodecError::malformed(F, "JPEG 2000 SIZ marker declares zero-bit components")),
        1..=8 => Ok(SampleType::U8),
        9..=MAX_SUPPORTED_BITS => Ok(SampleType::U16),
        bits => Err(CodecError::unsupported(F, format!("JPEG 2000 images of {bits} bits per sample are not supported (16 at most)"))),
    }
}

/// The ICC profile of the first method-2 `colr` box, which the decoder reads back as the
/// document's profile.
fn icc_profile(header: &j2k::J2kSupportInfo) -> Option<Vec<u8>> {
    header.file_metadata.as_ref()?.color_specs.iter().find_map(|spec| match spec {
        J2kColorSpec::IccProfile { profile } => Some(profile.clone()),
        _ => None,
    })
}

/// One decoded component, with the arithmetic that turns its raw samples into 0..1.
struct Plane<'a> {
    samples: &'a [f32],
    /// What is added before dividing: zero for an unsigned component, the signed maximum for
    /// a signed one, whose range is lifted to 0..1 rather than clamped into it.
    bias: f32,
    /// `2^bits - 1`, or twice the signed magnitude; never zero.
    divisor: f32,
}

impl<'a> Plane<'a> {
    fn new(plane: &'a j2k::J2kComponentPlane<'a>) -> Self {
        let (bias, divisor) = sample_range(plane.bit_depth(), plane.signed());
        Self { samples: plane.samples(), bias, divisor }
    }

    /// One sample as a 0..1 position. A non-finite value (a decoder bug or a damaged file)
    /// reads as black rather than propagating.
    fn at(&self, index: usize) -> f32 {
        let raw = self.samples.get(index).copied().unwrap_or(0.0);
        if raw.is_finite() { ((raw + self.bias) / self.divisor).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// Interleave the first `layout.channels()` component planes into our native-endian samples.
/// The planes are per component, so this handles every layout the family can describe without
/// asking the codec for an interleaved format that has no gray+alpha or four-colour variant.
fn interleave(
    planes: &[j2k::J2kComponentPlane<'_>],
    width: u32,
    height: u32,
    layout: ChannelLayout,
    sample: SampleType,
) -> Result<Vec<u8>, CodecError> {
    let channels = layout.channels();
    let step = sample.bytes();
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(channels))
        .and_then(|n| n.checked_mul(step))
        .ok_or_else(|| CodecError::InvalidImage("JPEG 2000 image size overflows usize".into()))?;
    let mut bytes = crate::options::alloc_zeroed(pixels)?;

    // A component's precision does not change inside the loop, so read it once up front.
    let mut sources: Vec<Plane<'_>> = Vec::with_capacity(channels);
    for channel in 0..channels {
        let plane = planes.get(channel).ok_or_else(|| CodecError::malformed(F, "JPEG 2000 decode produced too few components"))?;
        sources.push(Plane::new(plane));
    }

    let count = width as usize * height as usize;
    match sample {
        SampleType::U8 => {
            let full = f32::from(u8::MAX);
            for pixel in 0..count {
                let base = pixel * channels;
                for (channel, plane) in sources.iter().enumerate() {
                    if let Some(slot) = bytes.get_mut(base + channel) {
                        *slot = (plane.at(pixel) * full).round() as u8;
                    }
                }
            }
        }
        SampleType::U16 => {
            let full = f32::from(u16::MAX);
            for pixel in 0..count {
                let base = pixel * channels * step;
                for (channel, plane) in sources.iter().enumerate() {
                    let at = base + channel * step;
                    let value = (plane.at(pixel) * full).round() as u16;
                    if let Some(slot) = bytes.get_mut(at..at + 2) {
                        slot.copy_from_slice(&value.to_ne_bytes());
                    }
                }
            }
        }
        // `sample_for` never returns a float type; the branch keeps the match total.
        SampleType::F16 | SampleType::F32 => return Err(CodecError::unsupported(F, "JPEG 2000 float sample types are not produced by this decoder")),
    }
    Ok(bytes)
}

/// The offset and divisor that scale one component's raw samples into 0..1. A 1-, 4- or
/// 12-bit component is scaled by its own maximum, not by 255 or 65535, so an 8-bit file from a
/// 12-bit source still lands on the full range.
fn sample_range(bit_depth: u8, signed: bool) -> (f32, f32) {
    let bits = u32::from(bit_depth).clamp(1, 24);
    if signed {
        let max = ((1u64 << (bits - 1)) - 1) as f32;
        if max <= 0.0 { (0.0, 1.0) } else { (max, 2.0 * max) }
    } else {
        let max = ((1u64 << bits) - 1) as f32;
        if max <= 0.0 { (0.0, 1.0) } else { (0.0, max) }
    }
}

/// Encode `image` per `plan` into a raw codestream.
fn codestream(image: &Image, format: Format, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = image.converted(plan.layout, plan.sample);
    let (width, height) = img.dimensions();
    let components = u16::try_from(plan.layout.channels()).map_err(|_| CodecError::encode(format, "too many components for JPEG 2000"))?;
    let bit_depth = bits_for(plan.sample, format)?;
    // No quality asked for means the reversible path; see `EncodeOptions::jp2_quality`.
    let quality = opts.jp2_quality.unwrap_or(100).clamp(1, 100);

    // Quality 100 is the reversible path; so is any layout whose colour transform the
    // irreversible path would get wrong, which today means anything but the enumerated models.
    if quality >= 100 || !lossy_transform_is_safe(plan.layout) {
        return lossless(img.as_ref(), format, width, height, components, bit_depth);
    }
    lossy(img.as_ref(), format, width, height, components, bit_depth, quality)
}

/// The irreversible 9/7 path applies Part 1's colour transform, which is defined for exactly
/// three colour components. That is sound for gray, RGB and RGB+alpha (the transform touches
/// the first three components and the fourth is coded independently) but would corrupt inks, so
/// `caps` keeps CMYK out of the family entirely and this is the belt to that braces.
fn lossy_transform_is_safe(layout: ChannelLayout) -> bool {
    layout.is_gray() || layout.is_rgb()
}

fn lossless(
    image: &Image,
    format: Format,
    width: u32,
    height: u32,
    components: u16,
    bit_depth: u8,
) -> Result<Vec<u8>, CodecError> {
    let samples = J2kLosslessSamples::new(image.data(), width, height, components, bit_depth, false)
        .map_err(|e| CodecError::encode(format, format!("JPEG 2000 samples: {e}")))?;
    let options = J2kLosslessEncodeOptions::default().with_validation(J2kEncodeValidation::CpuRoundTrip);
    encode_j2k_lossless(samples, &options).map(|encoded| encoded.codestream).map_err(|e| CodecError::encode(format, format!("JPEG 2000 lossless encode: {e}")))
}

fn lossy(
    image: &Image,
    format: Format,
    width: u32,
    height: u32,
    components: u16,
    bit_depth: u8,
    quality: u8,
) -> Result<Vec<u8>, CodecError> {
    let samples = J2kLossySamples::new(image.data(), width, height, components, bit_depth, false)
        .map_err(|e| CodecError::encode(format, format!("JPEG 2000 samples: {e}")))?;
    // JPEG 2000 has no quality scale of its own, so quality picks a rate the way JPEG's
    // quantisation tables pick one: a curve, near-lossless at the top and a thumbnail at the
    // bottom, as a fraction of the uncompressed size so the scale means the same at any depth.
    let raw_bits_per_pixel = f64::from(components) * f64::from(bit_depth);
    let q = f64::from(quality.clamp(1, 99)) / 100.0;
    let bits_per_pixel = raw_bits_per_pixel * (0.02 + 0.98 * q * q);
    let options = J2kLossyEncodeOptions::default()
        .with_rate_target(Some(J2kRateTarget::BitsPerPixel(bits_per_pixel)))
        .with_validation(J2kEncodeValidation::CpuRoundTrip);
    encode_j2k_lossy(samples, &options).map(|encoded| encoded.codestream).map_err(|e| CodecError::encode(format, format!("JPEG 2000 lossy encode: {e}")))
}

/// The significant bits per component of a planned image: 8 or 16. Float plans never reach
/// here (`caps` keeps them at U16), but a direct caller could ask.
fn bits_for(sample: SampleType, format: Format) -> Result<u8, CodecError> {
    match sample {
        SampleType::U8 => Ok(8),
        SampleType::U16 | SampleType::F16 => Ok(16),
        SampleType::F32 => Err(CodecError::unsupported(format, "32-bit float images are not stored by JPEG 2000; they are written at 16 bits")),
    }
}

/// Wrap a codestream in the JP2 boxes: signature, `ftyp`, then a header box holding the
/// colour specification, the alpha channel's `cdef` entry and the ICC profile when asked for.
fn wrap(image: &Image, plan: Plan, codestream: &[u8]) -> Result<Vec<u8>, CodecError> {
    let layout = plan.layout;
    let color: J2kFileColorSpec<'_> = match image.icc.as_deref().filter(|icc| !icc.is_empty()) {
        // A method-2 `colr` box: the ICC profile describes the samples, so no enumerated
        // colour space is written beside it.
        Some(icc) => J2kFileColorSpec::IccProfile(icc),
        // Greyscale plus opacity is two components, which no enumerated colour space covers,
        // so name the colour model (sGray) and let `cdef` mark the second channel.
        _ if layout == ChannelLayout::GrayA => J2kFileColorSpec::Enumerated(Colorspace::SGray),
        _ => J2kFileColorSpec::Infer,
    };
    let defs = channel_definitions(layout);

    let options = J2kFileWrapOptions::jp2().with_color(color).with_metadata(J2kFileBoxMetadata { palette: None, component_mappings: &[], channel_definitions: &defs });
    wrap_j2k_codestream(codestream, options).map_err(|e| CodecError::encode(F, format!("JPEG 2000 wrapping: {e}")))
}

/// The `cdef` box entries for a layout with an alpha channel: one per component, as readers
/// require, each colour channel bound to its own one-based index and the last one declared
/// opacity for the whole image. A layout without alpha needs none — the channels are colour.
fn channel_definitions(layout: ChannelLayout) -> Vec<J2kChannelDefinition> {
    let color = layout.color_channels();
    if !layout.has_alpha() {
        return Vec::new();
    }
    let mut defs: Vec<J2kChannelDefinition> = (0..color)
        .filter_map(|index| {
            u16::try_from(index).ok().and_then(|association| u16::try_from(association + 1).ok()).map(|association| J2kChannelDefinition {
                channel_index: association - 1,
                channel_type: J2kChannelType::Color,
                association: J2kChannelAssociation::Color { index: association },
            })
        })
        .collect();
    if let Some(alpha) = u16::try_from(color).ok() {
        defs.push(J2kChannelDefinition { channel_index: alpha, channel_type: J2kChannelType::Opacity, association: J2kChannelAssociation::WholeImage });
    }
    defs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic header, not a decodable image: enough bytes to identify the wrappers.
    fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
        let mut v = 0x14u32.to_be_bytes().to_vec();
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(brand);
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(brand);
        v
    }

    #[test]
    fn jp2_brand_is_detected() {
        assert_eq!(crate::detect(&ftyp(b"jp2 ")), Some(Format::Jp2));
    }

    #[test]
    fn raw_codestream_is_detected() {
        assert_eq!(crate::detect(&[0xFF, 0x4F, 0xFF, 0x51, 0, 0]), Some(Format::J2k));
    }

    #[test]
    fn component_counts_map_to_layouts() {
        assert_eq!(layout_for(Colorspace::SRgb, 3).unwrap(), ChannelLayout::Rgb);
        assert_eq!(layout_for(Colorspace::SRgb, 4).unwrap(), ChannelLayout::Rgba);
        assert_eq!(layout_for(Colorspace::SGray, 1).unwrap(), ChannelLayout::Gray);
        assert_eq!(layout_for(Colorspace::SGray, 2).unwrap(), ChannelLayout::GrayA);
        assert_eq!(layout_for(Colorspace::Cmyk, 4).unwrap(), ChannelLayout::Cmyk);
        assert_eq!(layout_for(Colorspace::Cmyk, 5).unwrap(), ChannelLayout::CmykA);
        assert_eq!(layout_for(Colorspace::IccTagged, 1).unwrap(), ChannelLayout::Gray);
        assert_eq!(layout_for(Colorspace::IccTagged, 3).unwrap(), ChannelLayout::Rgb);
    }

    #[test]
    fn component_counts_outside_the_map_are_unsupported_not_guessed() {
        assert!(layout_for(Colorspace::SRgb, 0).is_err());
        assert!(layout_for(Colorspace::SRgb, 6).is_err());
    }

    #[test]
    fn precisions_pick_the_narrower_sample_type() {
        assert_eq!(sample_for(1).unwrap(), SampleType::U8);
        assert_eq!(sample_for(8).unwrap(), SampleType::U8);
        assert_eq!(sample_for(9).unwrap(), SampleType::U16);
        assert_eq!(sample_for(16).unwrap(), SampleType::U16);
        assert!(sample_for(0).is_err());
        assert!(sample_for(17).is_err());
    }

    #[test]
    fn planes_scale_by_their_own_precision() {
        let scaled = |raw: f32, bits: u8, signed: bool| {
            let (bias, divisor) = sample_range(bits, signed);
            if raw.is_finite() { ((raw + bias) / divisor).clamp(0.0, 1.0) } else { 0.0 }
        };
        assert_eq!(scaled(255.0, 8, false), 1.0);
        assert_eq!(scaled(15.0, 4, false), 1.0);
        assert_eq!(scaled(7.0, 3, false), 1.0);
        assert_eq!(scaled(32767.0, 15, false), 1.0);
        assert_eq!(scaled(65535.0, 16, false), 1.0);
        // Signed: the range lifts into 0..1, so the extremes are the ends.
        assert_eq!(scaled(0.0, 16, true), 0.5);
        assert_eq!(scaled(32767.0, 16, true), 1.0);
        // Hostile samples never escape 0..1, and a non-finite one reads as black.
        assert_eq!(scaled(-5.0, 8, false), 0.0);
        assert_eq!(scaled(f32::NAN, 8, false), 0.0);
        assert_eq!(scaled(f32::INFINITY, 8, false), 0.0);
        assert_eq!(scaled(f32::NEG_INFINITY, 8, false), 0.0);
        assert_eq!(scaled(1.0e9, 8, false), 1.0);
        // A zero- or absurd-precision component divides by one instead of by zero.
        assert_eq!(sample_range(0, false), (0.0, 1.0));
        assert_eq!(sample_range(1, true), (0.0, 1.0));
    }
}