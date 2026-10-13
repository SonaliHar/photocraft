//! Checked storage access. These APIs never interpret or convert sample bits.
use crate::{PixelFormat, SampleAccessError, SampleType, sample_byte_range};

/// Total encoded size of a sample array, without wrapping input-derived counts.
pub fn checked_sample_byte_len(sample: SampleType, count: usize) -> Result<usize, SampleAccessError> {
    count.checked_mul(sample.bytes()).ok_or(SampleAccessError::OffsetOverflow)
}

/// Total encoded size of packed pixels, including alpha when present.
pub fn checked_pixel_byte_len(format: PixelFormat, pixels: usize) -> Result<usize, SampleAccessError> {
    let samples = pixels.checked_mul(format.channels()).ok_or(SampleAccessError::OffsetOverflow)?;
    checked_sample_byte_len(format.sample, samples)
}

/// Checked channel range in a packed pixel array. Rejects absent channel indices.
pub fn pixel_channel_byte_range(format: PixelFormat, pixel: usize, channel: usize, byte_len: usize) -> Result<std::ops::Range<usize>, SampleAccessError> {
    if channel >= format.channels() {
        return Err(SampleAccessError::OutOfBounds);
    }
    let index = pixel.checked_mul(format.channels()).and_then(|start| start.checked_add(channel)).ok_or(SampleAccessError::OffsetOverflow)?;
    sample_byte_range(format.sample, index, byte_len)
}

/// Borrow exact stored bytes, including floating signed zero and NaN payloads.
/// The bytes use native-endian storage; this is not a serialization conversion.
pub fn try_read_sample_raw(bytes: &[u8], sample: SampleType, index: usize) -> Result<&[u8], SampleAccessError> {
    bytes.get(sample_byte_range(sample, index, bytes.len())?).ok_or(SampleAccessError::OutOfBounds)
}

/// Replace one sample with exactly its encoded width. A failure leaves bytes untouched.
/// No depth conversion or half widening occurs; callers must choose those explicitly.
pub fn try_write_sample_raw(bytes: &mut [u8], sample: SampleType, index: usize, encoded: &[u8]) -> Result<(), SampleAccessError> {
    if encoded.len() != sample.bytes() {
        return Err(SampleAccessError::OutOfBounds);
    }
    let range = sample_byte_range(sample, index, bytes.len())?;
    bytes.get_mut(range).ok_or(SampleAccessError::OutOfBounds)?.copy_from_slice(encoded);
    Ok(())
}

/// Copy a sample within one explicitly shared storage type, with no numerical round trip.
/// Both accesses are checked before the destination is changed. Different depths require
/// a separately chosen conversion API, rather than an implicit narrowing copy.
pub fn try_copy_sample_raw(
    source: &[u8],
    source_index: usize,
    destination: &mut [u8],
    destination_index: usize,
    sample: SampleType,
) -> Result<(), SampleAccessError> {
    let encoded = try_read_sample_raw(source, sample, source_index)?;
    try_write_sample_raw(destination, sample, destination_index, encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_bits_are_copied_without_interpretation() {
        for bits in [0x8000_0000_0000_0000_u64, 1, 0x7ff0_1234_5678_9abc] {
            let source = bits.to_ne_bytes();
            let mut destination = [42; 16];
            try_copy_sample_raw(&source, 0, &mut destination, 1, SampleType::F64).unwrap();
            assert_eq!(try_read_sample_raw(&destination, SampleType::F64, 1).unwrap(), source);
            assert_eq!(&destination[..8], &[42; 8]);
        }
        let source = 16_777_217_u32.to_ne_bytes();
        let mut destination = [0; 4];
        try_copy_sample_raw(&source, 0, &mut destination, 0, SampleType::U32).unwrap();
        assert_eq!(source, destination);
    }

    #[test]
    fn hostile_sizes_channels_and_raw_width_leave_destination_unchanged() {
        assert_eq!(checked_sample_byte_len(SampleType::F64, usize::MAX), Err(SampleAccessError::OffsetOverflow));
        assert_eq!(checked_pixel_byte_len(PixelFormat::RGBA64F, usize::MAX), Err(SampleAccessError::OffsetOverflow));
        assert_eq!(checked_pixel_byte_len(PixelFormat::RGBA64F, 0), Ok(0));
        assert_eq!(pixel_channel_byte_range(PixelFormat::RGBA64F, 1, 3, 64), Ok(56..64));
        assert_eq!(pixel_channel_byte_range(PixelFormat::RGBA64F, 0, 4, 64), Err(SampleAccessError::OutOfBounds));
        let mut destination = [9; 8];
        assert!(try_write_sample_raw(&mut destination, SampleType::F64, 0, &[1; 4]).is_err());
        assert!(try_copy_sample_raw(&[1; 7], 0, &mut destination, 0, SampleType::F64).is_err());
        assert!(try_copy_sample_raw(&[1; 8], 0, &mut destination, usize::MAX, SampleType::F64).is_err());
        assert_eq!(destination, [9; 8]);
    }
}
