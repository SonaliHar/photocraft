//! The JPEG XL encoder: a thin, panic-guarded wrapper around `jixel` (pure Rust, BSD-3-Clause OR
//! Apache-2.0, a port of libjxl-tiny): lossy VarDCT and lossless Modular, 8 and 16-bit integer
//! and 32-bit float samples, grayscale and RGB with alpha, an embedded ICC profile, and EXIF and
//! XMP boxes in the container. [`recompress_jpeg`] turns a JPEG file into a JPEG XL file from
//! which the original JPEG can be rebuilt byte for byte ([`crate::reconstruct_jpeg`]).
//!
//! Like the decoder, this never panics: every call into jixel runs under `catch_unwind`, and a
//! panic inside it becomes [`Error::Malformed`].

use crate::{Error, Layout};

/// What a panic inside jixel becomes.
const ENCODER_FAILED: &str = "the JPEG XL encoder failed on this image";

/// Runs `f`, turning a jixel panic into [`Error::Malformed`] with [`ENCODER_FAILED`].
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(Error::Malformed(ENCODER_FAILED.into())))
}

/// Storage type of the samples handed to [`encode`], native-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSample {
    U8,
    U16,
    /// Nominal range `[0, 1]`; HDR values above it are kept.
    F32,
}

impl InputSample {
    pub const fn bytes(self) -> usize {
        match self {
            InputSample::U8 => 1,
            InputSample::U16 => 2,
            InputSample::F32 => 4,
        }
    }
}

/// An image to encode: interleaved samples, row-major, no padding, upright, straight alpha.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    pub sample: InputSample,
    /// `width * height * layout.channels() * sample.bytes()` bytes, native-endian samples.
    pub data: &'a [u8],
    /// The profile the samples are in (`None`: sRGB, or gray with the sRGB curve).
    pub icc: Option<&'a [u8]>,
    /// EXIF as a TIFF structure (no `Exif\0\0` prefix).
    pub exif: Option<&'a [u8]>,
    /// The XMP packet.
    pub xmp: Option<&'a [u8]>,
}

/// How to encode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Lossless Modular: every sample comes back exactly. `quality` is then ignored.
    pub lossless: bool,
    /// Lossy quality 0–100 on the familiar JPEG-like scale (90 is visually lossless, distance 1).
    pub quality: f32,
    /// Encoder effort 1–9 (as libjxl's `-e`): higher is slower and smaller.
    pub effort: u8,
    /// Progressive: a lossy file shows a coarse picture first and refines; a lossless one holds
    /// a low-resolution preview that refines to the exact image.
    pub progressive: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { lossless: true, quality: 90.0, effort: DEFAULT_EFFORT, progressive: false }
    }
}

/// The effort [`Settings::default`] uses: jixel's own default speed.
pub const DEFAULT_EFFORT: u8 = 4;

/// The encoder speed an effort (1–9, clamped) selects.
fn speed(effort: u8) -> jixel::Speed {
    use jixel::Speed as S;
    match effort.clamp(1, 9) {
        1 => S::UltraFast,
        2 => S::Fastest,
        3 | 4 => S::Fast,
        5 | 6 => S::Medium,
        7 | 8 => S::Slow,
        _ => S::ExtraSlow,
    }
}

/// The lossy distance a quality (0–100) selects. jixel's own mapping asserts on NaN, so the
/// quality is made finite first.
fn distance(quality: f32) -> f32 {
    let q = if quality.is_finite() { quality.clamp(0.0, 100.0) } else { 90.0 };
    jixel::distance_from_quality(q)
}

fn encode_err(e: jixel::EncodeError) -> Error {
    use jixel::EncodeError as E;
    match e {
        E::DimensionTooLarge { .. } | E::AllocationFailed { .. } => Error::Limit(e.to_string()),
        E::UnsupportedAlphaBitDepth(_) | E::IccProfileNotSupported => Error::Unsupported(e.to_string()),
        e => Error::Malformed(e.to_string()),
    }
}

/// Native-endian bytes as samples of `T`; `None` when the length is not a whole number of them.
fn samples<T, const N: usize>(bytes: &[u8], from: fn([u8; N]) -> T) -> Option<Vec<T>> {
    let (chunks, rest) = bytes.as_chunks::<N>();
    rest.is_empty().then(|| chunks.iter().map(|c| from(*c)).collect())
}

/// Float gray goes in as RGB(A) with the gray copied to the three colour channels where jixel
/// has no gray entry point that keeps it exact: gray + alpha has none at all, and its gray-only
/// one is lossy even when asked for lossless. Only without a profile (a gray one would not fit).
fn gray_to_rgb(v: &[f32], alpha: bool) -> Vec<f32> {
    if alpha { v.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect() } else { v.iter().flat_map(|&g| [g, g, g]).collect() }
}

/// Encodes `input` as a JPEG XL file: a bare codestream, or the container when there is EXIF or
/// XMP to carry.
pub fn encode(input: &Input<'_>, settings: &Settings) -> Result<Vec<u8>, Error> {
    let (w, h) = (input.width as usize, input.height as usize);
    if w == 0 || h == 0 {
        return Err(Error::Malformed(format!("cannot encode an empty {w}x{h} image")));
    }
    let expected = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(input.layout.channels()))
        .and_then(|n| n.checked_mul(input.sample.bytes()))
        .ok_or_else(|| Error::Limit(format!("a {w}x{h} image does not fit in memory")))?;
    if input.data.len() != expected {
        return Err(Error::Malformed(format!("{} bytes of samples for a {w}x{h} image that needs {expected}", input.data.len())));
    }
    let float_gray_as_rgb = input.sample == InputSample::F32 && (input.layout == Layout::GrayA || (input.layout == Layout::Gray && settings.lossless));
    if float_gray_as_rgb && input.icc.is_some() {
        return Err(Error::Unsupported("lossless float grayscale, or float grayscale with alpha, with a gray profile".into()));
    }
    let mut config = jixel::EncodeConfig {
        lossless: settings.lossless,
        distance: distance(settings.quality),
        speed: speed(settings.effort),
        progressive: settings.progressive,
        icc_profile: input.icc.map(<[u8]>::to_vec),
        exif: input.exif.map(<[u8]>::to_vec),
        xmp: input.xmp.map(<[u8]>::to_vec),
        ..Default::default()
    };
    let first = encode_with(input, &config, float_gray_as_rgb);
    // jixel 0.4.2's slower lossy transform search panics on a few small sizes (12x9 and 15x9 at
    // effort 5 and up). The faster search has no such case: try it once before giving up.
    match first {
        Err(Error::Malformed(m)) if m == ENCODER_FAILED && !settings.lossless && speed(settings.effort) != jixel::Speed::Fast => {
            config.speed = jixel::Speed::Fast;
            encode_with(input, &config, float_gray_as_rgb)
        }
        r => r,
    }
}

/// [`encode`] with the settings turned into jixel's.
fn encode_with(input: &Input<'_>, config: &jixel::EncodeConfig, float_gray_as_rgb: bool) -> Result<Vec<u8>, Error> {
    let (w, h, config) = (input.width as usize, input.height as usize, config);
    let bad = || Error::Malformed("sample bytes are not whole samples".into());
    guarded(|| {
        let out = match (input.sample, input.layout) {
            (InputSample::U8, Layout::Gray) => jixel::encode_image_gray(input.data, w, h, config),
            (InputSample::U8, Layout::GrayA) => jixel::encode_image_gray_alpha(input.data, w, h, config),
            (InputSample::U8, Layout::Rgb) => jixel::encode_image(input.data, w, h, config),
            (InputSample::U8, Layout::Rgba) => jixel::encode_image_with_alpha(input.data, w, h, config),
            (InputSample::U16, layout) => {
                let v = samples(input.data, u16::from_ne_bytes).ok_or_else(bad)?;
                match layout {
                    Layout::Gray => jixel::encode_image_gray_16bit(&v, w, h, config),
                    Layout::GrayA => jixel::encode_image_gray_alpha_16bit(&v, w, h, config),
                    Layout::Rgb => jixel::encode_image_16bit(&v, w, h, config),
                    Layout::Rgba => jixel::encode_image_with_alpha_16bit(&v, w, h, config),
                }
            }
            (InputSample::F32, layout) => {
                let v = samples(input.data, f32::from_ne_bytes).ok_or_else(bad)?;
                if v.iter().any(|s| !s.is_finite()) {
                    return Err(Error::Malformed("float samples must be finite".into()));
                }
                match layout {
                    Layout::Gray if float_gray_as_rgb => jixel::encode_image_f32(&gray_to_rgb(&v, false), w, h, config),
                    Layout::Gray => jixel::encode_image_gray_f32(&v, w, h, config),
                    Layout::GrayA => jixel::encode_image_with_alpha_f32(&gray_to_rgb(&v, true), w, h, config),
                    Layout::Rgb => jixel::encode_image_f32(&v, w, h, config),
                    Layout::Rgba => jixel::encode_image_with_alpha_f32(&v, w, h, config),
                }
            }
        };
        out.map_err(encode_err)
    })
}

/// The frame size and component count of a JPEG, from its first SOF marker: `None` when there is
/// none before the scan data or the file ends first.
fn jpeg_frame(jpeg: &[u8]) -> Option<(u32, u32, u32)> {
    let mut at = 2usize;
    loop {
        // Fill bytes (0xFF) may come before a marker.
        while jpeg.get(at) == Some(&0xFF) && jpeg.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if jpeg.get(at) != Some(&0xFF) {
            return None;
        }
        let marker = *jpeg.get(at + 1)?;
        let len = usize::from(u16::from_be_bytes([*jpeg.get(at + 2)?, *jpeg.get(at + 3)?]));
        match marker {
            // SOF0–SOF15 except DHT (C4), JPG (C8) and DAC (CC).
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let h = u32::from(u16::from_be_bytes([*jpeg.get(at + 5)?, *jpeg.get(at + 6)?]));
                let w = u32::from(u16::from_be_bytes([*jpeg.get(at + 7)?, *jpeg.get(at + 8)?]));
                return Some((w, h, u32::from(*jpeg.get(at + 9)?)));
            }
            0xDA | 0xD9 => return None,
            _ => at = at.checked_add(2)?.checked_add(len)?,
        }
    }
}

/// Recompresses a JPEG file losslessly: the JPEG's own DCT coefficients are stored (about 20%
/// smaller), with the data to rebuild the original file byte for byte
/// ([`crate::reconstruct_jpeg`]). The pixels decode exactly as the JPEG's do.
///
/// The encoder holds every coefficient of the image, sized by the JPEG's header: a header that
/// asks for more than `alloc_limit` bytes of them is [`Error::Limit`], checked before any
/// allocation (a failed allocation would abort, which `catch_unwind` cannot stop).
pub fn recompress_jpeg(jpeg: &[u8], alloc_limit: usize) -> Result<Vec<u8>, Error> {
    if !jpeg.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(Error::Malformed("not a JPEG file".into()));
    }
    let (w, h, components) = jpeg_frame(jpeg).ok_or_else(|| Error::Malformed("the JPEG has no frame header".into()))?;
    if w == 0 || h == 0 || components == 0 {
        return Err(Error::Malformed(format!("a {w}x{h} JPEG with {components} components")));
    }
    // 16-bit coefficients, padded to whole 8x8 blocks, and as much again for the encoder's own
    // planes: a generous estimate, so a file that fits never fails late.
    let padded = |v: u32| u64::from(v.div_ceil(8)) * 8;
    let need = padded(w) * padded(h) * u64::from(components) * 2 * 2;
    if need > alloc_limit as u64 {
        return Err(Error::Limit(format!("a {w}x{h} JPEG needs about {need} bytes, more than the {alloc_limit} allowed")));
    }
    guarded(|| jixel::encode_jpeg_lossless(jpeg).map_err(encode_err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, Sample, decode};

    const W: u32 = 37;
    const H: u32 = 23;

    /// A smooth gradient with a sharp edge, `channels` interleaved samples per pixel, as `sample`.
    fn picture(layout: Layout, sample: InputSample) -> Vec<u8> {
        let ch = layout.channels();
        let mut out = Vec::new();
        for y in 0..H {
            for x in 0..W {
                for c in 0..ch {
                    let v = ((x * 7 + y * 3 + c as u32 * 50) % 256) as f32 / 255.0;
                    let v = if c + 1 == ch && layout.has_alpha() { if x < W / 2 { 1.0 } else { y as f32 / H as f32 } } else { v };
                    match sample {
                        InputSample::U8 => out.push((v * 255.0).round() as u8),
                        InputSample::U16 => out.extend(((v * 65535.0).round() as u16).to_ne_bytes()),
                        InputSample::F32 => out.extend(v.to_ne_bytes()),
                    }
                }
            }
        }
        out
    }

    fn input(layout: Layout, sample: InputSample, data: &[u8]) -> Input<'_> {
        Input { width: W, height: H, layout, sample, data, icc: None, exif: None, xmp: None }
    }

    fn opts() -> Options {
        Options { alloc_limit: 1 << 28 }
    }

    const LAYOUTS: [Layout; 4] = [Layout::Gray, Layout::GrayA, Layout::Rgb, Layout::Rgba];

    /// Lossless files decode to exactly the samples encoded, for every depth and layout.
    #[test]
    fn lossless_round_trips_every_depth_and_layout() {
        for sample in [InputSample::U8, InputSample::U16, InputSample::F32] {
            for layout in LAYOUTS {
                let data = picture(layout, sample);
                let jxl = encode(&input(layout, sample, &data), &Settings::default()).unwrap_or_else(|e| panic!("{sample:?} {layout:?}: {e}"));
                let d = decode(&jxl, &opts()).unwrap_or_else(|e| panic!("{sample:?} {layout:?}: {e}"));
                assert_eq!((d.width, d.height), (W, H));
                let want_sample = match sample {
                    InputSample::U8 => Sample::U8,
                    InputSample::U16 => Sample::U16,
                    InputSample::F32 => Sample::F32,
                };
                assert_eq!(d.sample, want_sample, "{sample:?} {layout:?}");
                // Float gray goes in as RGB(A), the gray copied to the three colour channels.
                if sample == InputSample::F32 && matches!(layout, Layout::Gray | Layout::GrayA) {
                    let alpha = layout == Layout::GrayA;
                    assert_eq!(d.layout, if alpha { Layout::Rgba } else { Layout::Rgb });
                    let want: Vec<u8> = data
                        .as_chunks::<4>()
                        .0
                        .chunks(layout.channels())
                        .flat_map(|px| if alpha { vec![px[0], px[0], px[0], px[1]] } else { vec![px[0]; 3] })
                        .flatten()
                        .collect();
                    assert!(d.data == want, "{layout:?}: lossless float gray differs");
                    continue;
                }
                assert_eq!(d.layout, layout, "{sample:?}");
                assert!(d.data == data, "{sample:?} {layout:?}: lossless samples differ");
            }
        }
    }

    /// PSNR in dB of 8-bit samples.
    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        let mse = a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum::<f64>() / a.len() as f64;
        if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
    }

    /// Lossy files are close to the original, and lower quality makes smaller files.
    #[test]
    fn lossy_is_close_and_quality_trades_size() {
        // A smooth picture: the sawtooth of `picture` is a worst case for any lossy coder.
        let data: Vec<u8> = (0..H).flat_map(|y| (0..W).flat_map(move |x| [(x * 255 / W) as u8, (y * 255 / H) as u8, ((x + y) * 4) as u8])).collect();
        let at = |quality: f32| encode(&input(Layout::Rgb, InputSample::U8, &data), &Settings { lossless: false, quality, ..Default::default() }).unwrap();
        let (high, low) = (at(95.0), at(40.0));
        assert!(low.len() < high.len(), "{} vs {}", low.len(), high.len());
        let d = decode(&high, &opts()).unwrap();
        assert!(psnr(&d.data, &data) > 30.0, "{}", psnr(&d.data, &data));
        // A quality that is not a number still encodes (at the default), never panics.
        assert!(at(f32::NAN).len() > 10);
    }

    #[test]
    fn every_effort_and_progressive_decodes() {
        let data = picture(Layout::Rgba, InputSample::U8);
        for effort in [0, 1, 4, 7, 9, 200] {
            for lossless in [true, false] {
                for progressive in [false, true] {
                    let s = Settings { lossless, quality: 85.0, effort, progressive };
                    let jxl = encode(&input(Layout::Rgba, InputSample::U8, &data), &s).unwrap_or_else(|e| panic!("{s:?}: {e}"));
                    let d = decode(&jxl, &opts()).unwrap_or_else(|e| panic!("{s:?}: {e}"));
                    assert_eq!((d.width, d.height, d.layout), (W, H, Layout::Rgba));
                    if lossless {
                        assert!(d.data == data, "{s:?}");
                    }
                }
            }
        }
    }

    /// EXIF and XMP go into the container and come back verbatim.
    #[test]
    fn exif_and_xmp_survive() {
        let data = picture(Layout::Rgb, InputSample::U8);
        let exif = b"II*\0\x08\0\0\0\0\0\0\0\0\0".to_vec();
        let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"/></x:xmpmeta>"#.to_vec();
        let i = Input { exif: Some(&exif), xmp: Some(&xmp), ..input(Layout::Rgb, InputSample::U8, &data) };
        for lossless in [true, false] {
            let jxl = encode(&i, &Settings { lossless, ..Default::default() }).unwrap();
            assert!(jxl.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n"), "metadata needs the container");
            let d = decode(&jxl, &opts()).unwrap();
            assert_eq!(d.exif.as_deref(), Some(&exif[..]));
            assert_eq!(d.xmp.as_deref().map(str::as_bytes), Some(&xmp[..]));
        }
        // Without metadata the output is a bare codestream.
        assert!(encode(&input(Layout::Rgb, InputSample::U8, &data), &Settings::default()).unwrap().starts_with(&[0xFF, 0x0A]));
    }

    /// Sizes where jixel 0.4.2's slower lossy search panics still encode (at the faster one).
    #[test]
    fn small_sizes_encode_lossy_at_every_effort() {
        for (w, h) in [(12, 9), (15, 9), (1, 1), (3, 17), (65, 33)] {
            let data: Vec<u8> = (0..w * h * 3).map(|i| (i * 37 % 251) as u8).collect();
            let i = Input { width: w, height: h, ..input(Layout::Rgb, InputSample::U8, &data) };
            for effort in 1..=9 {
                let jxl = encode(&i, &Settings { lossless: false, effort, ..Default::default() }).unwrap_or_else(|e| panic!("{w}x{h} effort {effort}: {e}"));
                assert_eq!(decode(&jxl, &opts()).unwrap().width, w);
            }
        }
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let data = picture(Layout::Rgb, InputSample::U8);
        let short = &data[..data.len() - 1];
        assert!(encode(&input(Layout::Rgb, InputSample::U8, short), &Settings::default()).is_err());
        assert!(encode(&Input { width: 0, ..input(Layout::Rgb, InputSample::U8, &data) }, &Settings::default()).is_err());
        let mut nan = picture(Layout::Rgb, InputSample::F32);
        nan[..4].copy_from_slice(&f32::NAN.to_ne_bytes());
        assert!(encode(&input(Layout::Rgb, InputSample::F32, &nan), &Settings::default()).is_err());
        let huge = Input { width: u32::MAX, height: u32::MAX, ..input(Layout::Rgb, InputSample::U8, &data) };
        assert!(encode(&huge, &Settings::default()).is_err());
        assert!(recompress_jpeg(b"not a jpeg", 1 << 30).is_err());
        assert!(recompress_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0], 1 << 30).is_err(), "a truncated JPEG");
        // A header that claims 65535x65535 is refused before anything is allocated.
        let huge = [0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0xFF, 0xFF, 0xFF, 0xFF, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF, 0xD9];
        assert!(matches!(recompress_jpeg(&huge, 1 << 30), Err(Error::Limit(_))));
        assert_eq!(jpeg_frame(&huge), Some((65535, 65535, 3)));
    }
}
