use photocraft_color::{PixelFormat, SampleAccessError, SampleType, sample_byte_range, try_read_sample_f64, try_write_sample_f64, widen_f16_to_f32_bits};

#[test]
fn u32_adjacent_codes_survive_normalized_f64_arithmetic() {
    for code in [0, 1, 16_777_216, 16_777_217, u32::MAX - 1, u32::MAX] {
        let bytes = code.to_ne_bytes();
        let value = try_read_sample_f64(&bytes, SampleType::U32, 0).unwrap();
        let mut out = [0; 4];
        try_write_sample_f64(&mut out, SampleType::U32, 0, value).unwrap();
        assert_eq!(out, bytes);
    }
    assert_eq!(PixelFormat::RGBA32.bytes_per_pixel(), 16);
    assert_eq!(PixelFormat::RGBA64F.bytes_per_pixel(), 32);
}

#[test]
fn f64_reads_preserve_signed_zero_subnormal_hdr_and_nan_payload() {
    for bits in [0x8000_0000_0000_0000, 1, (1.0_f64 + f64::EPSILON).to_bits(), (-4.25_f64).to_bits(), f64::INFINITY.to_bits(), 0x7ff8_1234_5678_9abc] {
        let bytes = bits.to_ne_bytes();
        let value = try_read_sample_f64(&bytes, SampleType::F64, 0).unwrap();
        assert_eq!(value.to_bits(), bits);
        let mut out = [0; 8];
        try_write_sample_f64(&mut out, SampleType::F64, 0, value).unwrap();
        assert_eq!(out, bytes);
    }
}

#[test]
fn hostile_offsets_and_nonfinite_integer_writes_are_errors_without_mutation() {
    assert_eq!(sample_byte_range(SampleType::F64, usize::MAX, 8), Err(SampleAccessError::OffsetOverflow));
    assert!(try_read_sample_f64(&[0; 7], SampleType::F64, 0).is_err());
    let mut bytes = [7; 4];
    assert!(try_write_sample_f64(&mut bytes, SampleType::U32, 0, f64::NAN).is_err());
    assert_eq!(bytes, [7; 4]);
}

#[test]
fn half_widening_is_explicit_and_preserves_binary_origin_information() {
    assert_eq!(widen_f16_to_f32_bits(0x8000), (-0.0_f32).to_bits());
    assert_eq!(widen_f16_to_f32_bits(1), (2.0_f32.powi(-24)).to_bits());
    assert_eq!(widen_f16_to_f32_bits(0x3c00), 1.0_f32.to_bits());
    assert_eq!(widen_f16_to_f32_bits(0x7c00), f32::INFINITY.to_bits());
    assert_eq!(widen_f16_to_f32_bits(0xfe35), 0xffc6_a000);
}
