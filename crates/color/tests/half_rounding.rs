use photocraft_color::{try_round_f32_to_f16_bits as round, widen_f16_to_f32_bits as widen};
#[test]
fn all_finite_half_values_round_trip_exactly_including_signed_zero() {
    for bits in 0..=u16::MAX {
        if bits & 0x7c00 != 0x7c00 {
            assert_eq!(round(f32::from_bits(widen(bits))).unwrap(), bits);
        }
    }
}
#[test]
fn every_finite_positive_midpoint_rounds_to_even_with_adjacent_float_controls() {
    for bits in 0..0x7bffu16 {
        let a = f32::from_bits(widen(bits));
        let b = f32::from_bits(widen(bits + 1));
        let midpoint = (a + b) * 0.5;
        let expected = if bits & 1 == 0 { bits } else { bits + 1 };
        assert_eq!(round(midpoint).unwrap(), expected);
        assert_eq!(round(f32::from_bits(midpoint.to_bits() - 1)).unwrap(), bits);
        assert_eq!(round(f32::from_bits(midpoint.to_bits() + 1)).unwrap(), bits + 1);
        assert_eq!(round(-midpoint).unwrap(), expected | 0x8000);
    }
}
#[test]
fn nonfinite_and_finite_overflow_reject_instead_of_clamping() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 65520., -65520., f32::MAX] {
        assert!(round(value).is_err());
    }
    assert_eq!(round(65519.).unwrap(), 0x7bff);
    assert_eq!(round(-65519.).unwrap(), 0xfbff);
}
