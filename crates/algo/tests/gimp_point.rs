use photocraft_algo::gimp::{
    point::{PointOperation, try_apply},
    registry::{WorkingEncoding, point_member},
};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn point_hdr_equations_preserve_alpha_outside_and_history_exactly() {
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[2.25, -0.5, 0.123456789012345, -0.0]).unwrap();
    source.try_write_pixel_f64(1, 0, &[f64::from_bits(0x7ff8000000000042), 0.0, 1.0, -0.0]).unwrap();
    let history = source.clone();
    let original = source.try_read_interleaved(Rect::new(0, 0, 2, 1)).unwrap();
    let result = try_apply(
        &source,
        Rect::new(0, 0, 1, 1),
        PointOperation::Exposure { stops: 1.0, black_level: 0.0 },
        MAX_CHECKED_RASTER_BYTES,
        u64::MAX,
        &Interrupt::NONE,
    )
    .unwrap();
    let mut pixel = [0.0; 4];
    result.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
    assert_eq!(pixel[..3], [4.5, -1.0, 0.24691357802469]);
    assert_eq!(pixel[3].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(result.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap(), original[32..]);
    assert_eq!(history.try_read_interleaved(Rect::new(0, 0, 2, 1)).unwrap(), original);
    let result = try_apply(
        &source,
        Rect::new(0, 0, 1, 1),
        PointOperation::BrightnessContrast { brightness: 0.25, contrast: 2.0 },
        MAX_CHECKED_RASTER_BYTES,
        u64::MAX,
        &Interrupt::NONE,
    )
    .unwrap();
    result.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
    assert_eq!(pixel[0], 4.25);
}

#[test]
fn exact_identity_and_limits_cancellation_are_transactional() {
    let source = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &[0xff; 32]).unwrap();
    let area = Rect::new(0, 0, 32, 32);
    let identity = PointOperation::Exposure { stops: 0.0, black_level: 0.0 };
    assert_eq!(try_apply(&source, area, identity, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).unwrap().default_bytes(), source.default_bytes());
    assert_eq!(try_apply(&source, area, identity, 1, 0, &Interrupt::NONE).unwrap_err(), RasterInputError::LimitExceeded);
    let source = Surface::new(PixelFormat::RGBA64F);
    let count = AtomicUsize::new(0);
    let poll = || count.fetch_add(1, Ordering::Relaxed) > 5;
    let cancel = Interrupt::cancel_only(&poll);
    assert_eq!(try_apply(&source, area, PointOperation::Invert, MAX_CHECKED_RASTER_BYTES, u64::MAX, &cancel).unwrap_err(), RasterInputError::Cancelled);
    assert_eq!(source.tile_count(), 0);
    assert_eq!(try_apply(&source, area, PointOperation::Invert, 1, u64::MAX, &Interrupt::NONE).unwrap_err(), RasterInputError::LimitExceeded);
}

#[test]
fn registry_rejects_version_unknown_duplicate_and_invalid_black_level_domain() {
    let (member, _) = point_member("gegl:exposure", Some("1:0"), &[("exposure", 0.125), ("black-level", 0.0)]).unwrap();
    assert_eq!(member.encoding, WorkingEncoding::Linear);
    for (version, args) in [
        (None, vec![("exposure", 0.0), ("black-level", 0.0)]),
        (Some("0:0"), vec![("exposure", 0.0), ("black-level", 0.0)]),
        (Some("1:0"), vec![("exposure", 0.0), ("exposure", 0.0)]),
        (Some("1:0"), vec![("exposure", 0.0), ("black-level", 0.125)]),
        (Some("1:0"), vec![("exposure", 4.0), ("black-level", 0.0625)]),
    ] {
        assert!(point_member("gegl:exposure", version, &args).is_err());
    }
    assert!(point_member("gegl:invert-linear", Some("0:0"), &[("unknown", 0.0)]).is_err());
}

#[test]
fn point_u32_adjacent_codes_and_nonfinite_rejection_preserve_source() {
    let area = Rect::new(0, 0, 2, 1);
    let codes = [0x80000000_u32, 0x80000001, 7, 11, 0x80000001, 0x80000002, 9, 13];
    let raw: Vec<u8> = codes.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let source = Surface::try_from_interleaved(PixelFormat::RGBA32, area, &raw).unwrap();
    let result = try_apply(&source, area, PointOperation::Invert, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).unwrap();
    let expected: Vec<u8> = codes.iter().enumerate().flat_map(|(i, v)| if i % 4 == 3 { *v } else { u32::MAX - *v }.to_ne_bytes()).collect();
    assert_eq!(result.try_read_interleaved(area).unwrap(), expected);
    assert_eq!(source.try_read_interleaved(area).unwrap(), raw);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[f64::NAN, 0.0, 1.0, 1.0]).unwrap();
    let before = source.try_read_interleaved(area).unwrap();
    assert_eq!(
        try_apply(&source, area, PointOperation::Invert, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).unwrap_err(),
        RasterInputError::InvalidSample
    );
    assert_eq!(source.try_read_interleaved(area).unwrap(), before);
}

#[test]
fn ordered_pixel_evaluation_is_bounded_atomic_and_keeps_alpha_bits() {
    let alpha = f64::from_bits(0x7ff8000000000042);
    let mut out = [9.0; 4];
    PointOperation::Invert.try_eval_pixel(&[2.0, -1.0, 0.125, alpha], 3, &mut out).unwrap();
    assert_eq!(out[..3], [-1.0, 2.0, 0.875]);
    assert_eq!(out[3].to_bits(), alpha.to_bits());
    let before = out;
    assert!(PointOperation::Invert.try_eval_pixel(&[1.0, f64::NAN, 0.0, 1.0], 3, &mut out).is_err());
    assert_eq!(out.map(f64::to_bits), before.map(f64::to_bits));
    let mut large = [0.0; 7];
    assert!(PointOperation::Invert.try_eval_pixel(&[0.0; 7], 6, &mut large).is_err());
    let mut gray = [0.0; 2];
    PointOperation::Exposure { stops: 1.0, black_level: 0.0 }.try_eval_pixel(&[1.25, -0.0], 1, &mut gray).unwrap();
    assert_eq!(gray[0], 2.5);
    assert_eq!(gray[1].to_bits(), (-0.0_f64).to_bits());
}

#[test]
fn affine_levels_hdr_domains_identity_and_atomic_errors() {
    let op = PointOperation::Levels { input_low: 0.25, input_high: 0.75, output_low: -1.0, output_high: 2.0 };
    let mut output = [0.0; 4];
    op.try_eval_pixel(&[0.0, 0.5, 1.5, -0.0], 3, &mut output).unwrap();
    assert_eq!(output[..3], [-2.5, 0.5, 6.5]);
    assert_eq!(output[3].to_bits(), (-0.0_f64).to_bits());
    for (low, high) in [(1.0, 1.0), (2.0, 1.0), (f64::NEG_INFINITY, 1.0), (-f64::MAX, f64::MAX)] {
        let op = PointOperation::Levels { input_low: low, input_high: high, output_low: 0.0, output_high: 1.0 };
        let before = output;
        assert!(op.try_eval_pixel(&[0.0, 0.5, 1.0, 1.0], 3, &mut output).is_err());
        assert_eq!(output, before);
    }
    let identity = PointOperation::Levels { input_low: -2.0, input_high: 3.0, output_low: -2.0, output_high: 3.0 };
    let input = [f64::from_bits(0x7ff8000000000042), -0.0, 1.0, -0.0];
    identity.try_eval_pixel(&input, 3, &mut output).unwrap();
    assert_eq!(output.map(f64::to_bits), input.map(f64::to_bits));
    assert!(point_member("gegl:levels", Some("0:0"), &[("in-low", 0.0), ("in-high", 1.0), ("out-low", 0.0), ("out-high", 1.0)]).is_ok());
}

#[test]
fn posterize_unit_domain_integer_levels_alpha_and_atomic_errors() {
    let (member, evaluator) = point_member("gimp:posterize", Some("0:0"), &[("levels", 3.0)]).unwrap();
    assert_eq!(member.encoding, WorkingEncoding::Nonlinear);
    let alpha = f64::from_bits(0x7ff8000000000042);
    let mut output = [9.0; 4];
    evaluator.try_eval_pixel(&[0.0, 0.25, 1.0, alpha], 3, &mut output).unwrap();
    assert_eq!(output[..3], [0.0, 0.5, 1.0]);
    assert_eq!(output[3].to_bits(), alpha.to_bits());
    for levels in [1.0, 257.0, 3.5, f64::NAN, f64::INFINITY] {
        assert!(point_member("gimp:posterize", Some("0:0"), &[("levels", levels)]).is_err());
    }
    assert!(point_member("gimp:posterize", Some("1:0"), &[("levels", 3.0)]).is_err());
    assert!(point_member("gegl:posterize", Some("0:0"), &[("levels", 3.0)]).is_err());
    for value in [-0.001, 1.001, f64::NAN, f64::INFINITY] {
        let before = output.map(f64::to_bits);
        assert!(evaluator.try_eval_pixel(&[0.0, value, 0.5, alpha], 3, &mut output).is_err());
        assert_eq!(output.map(f64::to_bits), before);
    }
}

#[test]
fn value_invert_coupled_unit_rgb_and_alpha_are_atomic() {
    let (member, evaluator) = point_member("gegl:value-invert", Some("0:0"), &[]).unwrap();
    assert_eq!(member.encoding, WorkingEncoding::Nonlinear);
    let alpha = f64::from_bits(0x7ff8000000000042);
    let mut output = [9.0; 4];
    evaluator.try_eval_pixel(&[0.5, 0.25, 0.75, alpha], 3, &mut output).unwrap();
    assert_eq!(output[..3], [1.0 / 6.0, 1.0 / 12.0, 0.25]);
    assert_eq!(output[3].to_bits(), alpha.to_bits());
    for (input, expected) in [([0.0; 3], [1.0; 3]), ([1.0; 3], [0.0; 3]), ([1.0, 0.5, 0.0], [0.0; 3]), ([0.25; 3], [0.75; 3])] {
        evaluator.try_eval_pixel(&[input[0], input[1], input[2], -0.0], 3, &mut output).unwrap();
        assert_eq!(output[..3], expected);
        assert_eq!(output[3].to_bits(), (-0.0f64).to_bits());
    }
    for value in [-0.001, 1.001, f64::NAN, f64::INFINITY] {
        let before = output.map(f64::to_bits);
        assert!(evaluator.try_eval_pixel(&[0.5, value, 0.25, alpha], 3, &mut output).is_err());
        assert_eq!(output.map(f64::to_bits), before);
    }
    assert!(point_member("gegl:value-invert", Some("1:0"), &[]).is_err());
    assert!(point_member("gegl:value-invert", Some("0:0"), &[("unexpected", 0.0)]).is_err());
    let mut gray = [7.0; 2];
    assert!(evaluator.try_eval_pixel(&[0.5, 1.0], 1, &mut gray).is_err());
    assert_eq!(gray, [7.0; 2]);
    evaluator.try_eval_pixel(&[f64::from_bits(1), 0.0, 0.0, alpha], 3, &mut output).unwrap();
    assert_eq!(output[..3], [1.0, 0.0, 0.0]);
}

#[test]
fn value_invert_surface_preserves_raw_alpha_roi_and_failed_source() {
    let area = Rect::new(0, 0, 2, 1);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[0.5, 0.25, 0.75, f64::from_bits(0x7ff8000000000042)]).unwrap();
    source.try_write_pixel_f64(1, 0, &[2.0, -0.0, 0.0, -0.0]).unwrap();
    let before = source.try_read_interleaved(area).unwrap();
    let result = try_apply(&source, Rect::new(0, 0, 1, 1), PointOperation::ValueInvert, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).unwrap();
    let after = result.try_read_interleaved(area).unwrap();
    assert_eq!(after[24..], before[24..]);
    let mut values = [0.0; 4];
    result.try_read_pixel_f64(0, 0, &mut values).unwrap();
    assert_eq!(values[..3], [1.0 / 6.0, 1.0 / 12.0, 0.25]);
    assert!(try_apply(&source, area, PointOperation::ValueInvert, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).is_err());
    assert_eq!(source.try_read_interleaved(area).unwrap(), before);
}

#[test]
fn mono_mixer_coupled_linear_rgb_closed_weights_and_atomic_alpha() {
    let args = [("red", 0.5), ("green", 0.25), ("blue", 0.25), ("preserve-luminosity", 0.0)];
    let (member, operation) = point_member("gegl:mono-mixer", Some("0:0"), &args).unwrap();
    assert_eq!(member.encoding, WorkingEncoding::Linear);
    let alpha = f64::from_bits(0x7ff8000000000042);
    let mut output = [9.0; 4];
    operation.try_eval_pixel(&[0.5, 0.25, 0.75, alpha], 3, &mut output).unwrap();
    assert_eq!(output[..3], [0.5; 3]);
    assert_eq!(output[3].to_bits(), alpha.to_bits());
    for value in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
        let before = output.map(f64::to_bits);
        assert!(operation.try_eval_pixel(&[value, 0.25, 0.75, alpha], 3, &mut output).is_err());
        assert_eq!(output.map(f64::to_bits), before);
    }
    for weights in [(-0.1, 0.0, 0.0), (0.6, 0.6, 0.0), (f64::INFINITY, 0.0, 0.0)] {
        assert!(PointOperation::MonoMixer { red: weights.0, green: weights.1, blue: weights.2 }.validate().is_err());
    }
    let mut changed = args;
    changed[3].1 = 1.0;
    assert!(point_member("gegl:mono-mixer", Some("0:0"), &changed).is_err());
    let mut gray = [0.0; 2];
    assert!(operation.try_eval_pixel(&[0.5, 1.0], 1, &mut gray).is_err());
}
#[test]
fn rgb_clip_finite_hdr_exact_alpha_and_closed_boolean_domains() {
    let args = [("clip-low", 0.0), ("low-limit", 0.1), ("clip-high", 1.0), ("high-limit", 0.8)];
    let (member, operation) = point_member("gegl:rgb-clip", Some("0:0"), &args).unwrap();
    assert_eq!(member.encoding, WorkingEncoding::Linear);
    let alpha = f64::from_bits(0x7ff8000000000042);
    let mut output = [9.0; 4];
    operation.try_eval_pixel(&[-0.25, -0.0, 2.0, alpha], 3, &mut output).unwrap();
    assert_eq!(output[..3], [-0.25, -0.0, 0.8]);
    assert_eq!(output[1].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(output[3].to_bits(), alpha.to_bits());
    for (slot, value) in [(0, 0.5), (2, 0.0), (1, -0.1), (3, 1.1), (1, f64::NAN), (3, f64::INFINITY)] {
        let mut invalid = args;
        invalid[slot].1 = value;
        assert!(point_member("gegl:rgb-clip", Some("0:0"), &invalid).is_err());
    }
    let mut reversed = args;
    reversed[1].1 = 0.9;
    assert!(point_member("gegl:rgb-clip", Some("0:0"), &reversed).is_err());
    let before = output.map(f64::to_bits);
    assert!(operation.try_eval_pixel(&[0.5, f64::NAN, 2.0, alpha], 3, &mut output).is_err());
    assert_eq!(output.map(f64::to_bits), before);
    assert!(operation.try_eval_pixel(&[0.5, alpha], 1, &mut [0.0; 2]).is_err());
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[-0.25, -0.0, 2.0, alpha]).unwrap();
    source.try_write_pixel_f64(1, 0, &[4.0, 3.0, 2.0, -0.0]).unwrap();
    let area = Rect::new(0, 0, 2, 1);
    let original = source.try_read_interleaved(area).unwrap();
    let changed = try_apply(&source, Rect::new(0, 0, 1, 1), operation, MAX_CHECKED_RASTER_BYTES, u64::MAX, &Interrupt::NONE).unwrap();
    assert_eq!(changed.try_read_interleaved(area).unwrap()[24..], original[24..]);
    assert_eq!(source.try_read_interleaved(area).unwrap(), original);
    assert!(try_apply(&source, area, operation, 0, u64::MAX, &Interrupt::NONE).is_err());
}
