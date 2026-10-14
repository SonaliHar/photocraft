//! The PNG halves of the protocol: the `image` and `mask` parts PhotoCraft sends and the
//! `result.png` it gets back. Amendment 11 fixes the shapes - RGB or RGBA at 8 or 16 bits, and an
//! 8-bit grayscale mask - and nothing is colour-converted on the way.

use std::io::Cursor;

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_engine::ai::AiError;
use png::{BitDepth, ColorType, Decoder, Encoder, Limits};

/// Amendment 11: PNG bodies at most 1 GiB (and no larger than the request's pixel count).
pub const MAX_PNG_BYTES: usize = 1024 * 1024 * 1024;

/// The channel count the protocol allows. Only RGB is in scope for M1 (amendment 4), and the
/// engine already refuses other modes - but the client checks too, because it must not trust them.
pub fn channels(format: PixelFormat) -> Result<usize, AiError> {
    if format.mode != ColorMode::Rgb {
        return Err(AiError::Other(format!("Craft AI Manager fills RGB documents only; {} documents are not supported yet.", mode_name(format.mode))));
    }
    match format.channels() {
        n @ (3 | 4) => Ok(n),
        _ => Err(AiError::Other("The document's channel count is not one Craft AI Manager accepts.".into())),
    }
}

fn mode_name(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Bitmap => "Bitmap",
        ColorMode::Grayscale => "Grayscale",
        ColorMode::Indexed => "Indexed",
        ColorMode::Rgb => "RGB",
        ColorMode::Cmyk => "CMYK",
        ColorMode::Lab => "Lab",
        ColorMode::Multichannel => "Multichannel",
        ColorMode::Duotone => "Duotone",
    }
}

/// 8-bit PNG for an 8-bit document, 16-bit otherwise; 32-bit is clamped (amendment 4).
pub const fn bit_depth(format: PixelFormat) -> u8 {
    match format.sample {
        SampleType::U8 => 8,
        SampleType::U16 | SampleType::F32 => 16,
    }
}

fn oversize() -> AiError {
    AiError::Other("The area to fill is too large for Craft AI Manager.".into())
}

fn not_a_number() -> AiError {
    AiError::Other("The area to fill has a pixel value that is not a number, so it cannot be sent.".into())
}

fn decode_failed(error: png::DecodingError) -> AiError {
    AiError::Other(format!("Craft AI Manager sent a PNG PhotoCraft cannot read ({error})."))
}

fn encode_failed(_: png::EncodingError) -> AiError {
    AiError::Other("PhotoCraft could not package the image for Craft AI Manager.".into())
}

fn check_size(width: u32, height: u32) -> Result<(), AiError> {
    if width == 0 || height == 0 {
        return Err(AiError::Other("The area to fill is empty.".into()));
    }
    Ok(())
}

fn sample_bytes(depth: u8) -> Result<usize, AiError> {
    match depth {
        8 => Ok(1),
        16 => Ok(2),
        _ => Err(AiError::Other("PhotoCraft asked Craft AI Manager for an unsupported bit depth.".into())),
    }
}

/// Raw PNG sample bytes for `samples` samples, checked against the cap before anything allocates.
fn checked_byte_len(samples: usize, depth: u8) -> Result<usize, AiError> {
    let Some(bytes) = samples.checked_mul(sample_bytes(depth)?) else {
        return Err(oversize());
    };
    if bytes > MAX_PNG_BYTES {
        return Err(oversize());
    }
    Ok(bytes)
}

fn expected_samples(width: u32, height: u32, channels: usize) -> Result<usize, AiError> {
    let pixels = u64::from(width) * u64::from(height);
    let Some(samples) = pixels.checked_mul(channels as u64).and_then(|n| usize::try_from(n).ok()) else {
        return Err(oversize());
    };
    Ok(samples)
}

fn color_type(channels: usize) -> Result<ColorType, AiError> {
    match channels {
        3 => Ok(ColorType::Rgb),
        4 => Ok(ColorType::Rgba),
        _ => Err(AiError::Other("The document's channel count is not one Craft AI Manager accepts.".into())),
    }
}

fn bit_depth_of(depth: u8) -> BitDepth {
    if depth == 8 { BitDepth::Eight } else { BitDepth::Sixteen }
}

/// The `image` part: the window as RGB/RGBA PNG, 8- or 16-bit, in the document's colour space.
/// f32 samples are clamped to 0..=1, scaled by 255 or 65535 and rounded (amendment 11).
pub fn encode_image(pixels: &[f32], format: PixelFormat, width: u32, height: u32) -> Result<Vec<u8>, AiError> {
    let channels = channels(format)?;
    let depth = bit_depth(format);
    check_size(width, height)?;
    if pixels.len() != expected_samples(width, height, channels)? {
        return Err(AiError::Other("The AI image does not match its window.".into()));
    }
    if pixels.iter().any(|value| !value.is_finite()) {
        return Err(not_a_number());
    }
    let scale = if depth == 8 { 255.0 } else { 65535.0 };
    let mut raw = vec![0u8; checked_byte_len(pixels.len(), depth)?];
    if depth == 8 {
        for (slot, value) in raw.iter_mut().zip(pixels) {
            *slot = (value.clamp(0.0, 1.0) * scale + 0.5) as u8;
        }
    } else {
        for (slot, value) in raw.as_chunks_mut::<2>().0.iter_mut().zip(pixels) {
            let quantized: u16 = (value.clamp(0.0, 1.0) * scale + 0.5) as u16;
            slot.copy_from_slice(&quantized.to_be_bytes());
        }
    }
    encode_png(&raw, width, height, color_type(channels)?, depth)
}

/// The `mask` part: 8-bit grayscale, 255 fully selected and 0 keep.
pub fn encode_mask(coverage: &[f32], width: u32, height: u32) -> Result<Vec<u8>, AiError> {
    check_size(width, height)?;
    if coverage.len() != expected_samples(width, height, 1)? {
        return Err(AiError::Other("The AI selection does not match its window.".into()));
    }
    if coverage.iter().any(|value| !value.is_finite()) {
        return Err(not_a_number());
    }
    let raw: Vec<u8> = coverage.iter().map(|value| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
    encode_png(&raw, width, height, ColorType::Grayscale, 8)
}

fn encode_png(raw: &[u8], width: u32, height: u32, color: ColorType, depth: u8) -> Result<Vec<u8>, AiError> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, width, height);
    encoder.set_color(color);
    encoder.set_depth(bit_depth_of(depth));
    let mut writer = encoder.write_header().map_err(encode_failed)?;
    writer.write_image_data(raw).map_err(encode_failed)?;
    writer.finish().map_err(encode_failed)?;
    Ok(out)
}

/// `result.png` back to interleaved f32 in the request's format. The size, channels and depth must
/// match what was asked for, or the result cannot be written into the document.
pub fn decode_image(png_bytes: &[u8], format: PixelFormat, width: u32, height: u32) -> Result<Vec<f32>, AiError> {
    let channels = channels(format)?;
    let depth = bit_depth(format);
    check_size(width, height)?;
    let want_color = color_type(channels)?;
    let want_depth = bit_depth_of(depth);
    let raw_len = checked_byte_len(expected_samples(width, height, channels)?, depth)?;
    let mut decoder = Decoder::new_with_limits(Cursor::new(png_bytes), Limits { bytes: raw_len.saturating_add(raw_len / 4).saturating_add(1024) });
    decoder.set_ignore_text_chunk(true);
    let header = decoder.read_header_info().map_err(decode_failed)?;
    if header.width != width || header.height != height {
        return Err(AiError::Other("Craft AI Manager's result image is not the size PhotoCraft asked for.".into()));
    }
    if header.color_type != want_color {
        return Err(AiError::Other("Craft AI Manager's result image has a different number of channels from the image it was given.".into()));
    }
    if header.bit_depth != want_depth {
        return Err(AiError::Other("Craft AI Manager's result image has a different bit depth from the image it was given.".into()));
    }
    let mut reader = decoder.read_info().map_err(decode_failed)?;
    if reader.output_color_type() != (want_color, want_depth) {
        return Err(AiError::Other("Craft AI Manager's result image is not in the format PhotoCraft asked for.".into()));
    }
    if reader.output_buffer_size() != Some(raw_len) {
        return Err(AiError::Other("Craft AI Manager's result image is not the size PhotoCraft asked for.".into()));
    }
    let mut raw = vec![0u8; raw_len];
    reader.next_frame(&mut raw).map_err(decode_failed)?;
    Ok(to_floats(&raw, depth))
}

/// PNG samples are big-endian and unsigned; they normalise to 0..=1 exactly as `photocraft-color` does.
fn to_floats(raw: &[u8], depth: u8) -> Vec<f32> {
    if depth == 8 {
        raw.iter().map(|byte| f32::from(*byte) / 255.0).collect()
    } else {
        raw.as_chunks::<2>().0.iter().map(|bytes| f32::from(u16::from_be_bytes(*bytes)) / 65535.0).collect()
    }
}

/// The largest `result.png` body worth reading for this window: the raw samples plus headroom for
/// compression overhead, never over the protocol's 1 GiB.
pub fn result_body_cap(width: u32, height: u32, format: PixelFormat) -> Result<usize, AiError> {
    let channels = channels(format)?;
    let depth = bit_depth(format);
    let raw_len = checked_byte_len(expected_samples(width, height, channels)?, depth)?;
    Ok(raw_len.saturating_add(raw_len / 4).saturating_add(64 * 1024).min(MAX_PNG_BYTES))
}
