//! Raw native alpha gate matching the upstream adjustment `map_rgb` skip.
//! No source alpha word is normalized, validated as finite, or rewritten.
use photocraft_color::{PixelFormat, SampleType};
pub(crate) fn skips(pixel: &[u8], format: PixelFormat) -> std::result::Result<bool, String> {
    if pixel.len() != format.bytes_per_pixel() {
        return Err("native adjustment requires one exact pixel".into());
    }
    if !format.alpha {
        return Ok(false);
    }
    let size = format.sample.bytes();
    let start = format.mode.color_channels().checked_mul(size).ok_or_else(|| "native alpha offset overflow".to_string())?;
    let end = start.checked_add(size).ok_or_else(|| "native alpha offset overflow".to_string())?;
    let alpha = pixel.get(start..end).ok_or_else(|| "native alpha sample width".to_string())?;
    match format.sample {
        SampleType::U32 => Ok(u32::from_ne_bytes(alpha.try_into().map_err(|_| "native U32 alpha width".to_string())?) == 0),
        SampleType::F64 => Ok(f64::from_ne_bytes(alpha.try_into().map_err(|_| "native F64 alpha width".to_string())?) <= 0.0),
        _ => Err("native adjustment alpha gate supports U32/F64 only".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;
    #[test]
    fn alpha_predicate_matches_nonpositive_rule_without_rejecting_nan() {
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        for (word, skip) in [
            ((-0.0f64).to_bits(), true),
            (0.0f64.to_bits(), true),
            ((-1.0f64).to_bits(), true),
            (f64::NEG_INFINITY.to_bits(), true),
            (0x7ff8_0000_0000_0042, false),
            (f64::INFINITY.to_bits(), false),
            (1u64, false),
        ] {
            let mut pixel = vec![0; 32];
            pixel[24..].copy_from_slice(&word.to_ne_bytes());
            let before = pixel.clone();
            assert_eq!(skips(&pixel, format).unwrap(), skip);
            assert_eq!(pixel, before);
        }
        assert!(skips(&[0; 31], format).is_err());
    }
}
