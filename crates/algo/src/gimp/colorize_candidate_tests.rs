// Implementation cohort drafts: UNRUN; residual fixtures are not exact GIMP claims.
use super::*;
fn context() -> WorkingContext {
    WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [9; 32] }
}
fn witness(role: TransferRole) -> TransferWitness {
    TransferWitness { role, profile_identity: [9; 32] }
}
fn weights() -> YWeights {
    YWeights::from_bits([0.2224884033203125f64, 0.7169036865234375, 0.06060791015625].map(f64::to_bits)).unwrap()
}
fn parameters(values: [f32; 3]) -> Parameters {
    Parameters::from_stored_bits(values.map(f32::to_bits), [0.1f32, 0.2, 0.3].map(f32::to_bits)).unwrap()
}
fn prepare(params: Parameters, role: TransferRole) -> PreparedColorize {
    PreparedColorize::prepare(
        PixelFormat::RGBA32F,
        context(),
        params,
        NumericPolicy::DeclaredScalarTripletBinary64FinalFloat,
        witness(role),
        weights(),
        Limits::default(),
        &Interrupt::NONE,
    )
    .unwrap()
}
fn surface(values: &[[f32; 4]]) -> Surface {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    let bytes: Vec<_> = values.iter().flatten().flat_map(|v| v.to_ne_bytes()).collect();
    s.try_write_interleaved_with(Rect::new(0, 0, values.len() as i32, 1), &bytes, &Interrupt::NONE).unwrap();
    s
}
#[test]
fn gamma2_sealed_stored_negative_l_fixture_is_residual_only() {
    // Sealed Sol6 analysis f5c3f884, actual source/target2 edit. Original independent
    // HSL storage thirdword3e6ecc4c is not converted into scalar lightness.
    let params = Parameters::from_stored_bits([0x3e7b79e8, 0x3ebcf03f, 0xbf0899da], [0x3e7b79e8, 0x3ebcf03f, 0x3e6ecc4c]).unwrap();
    let p = prepare(params, TransferRole::CallerEstablishedExactGamma2);
    let output = p.try_eval_float_color([0.8, 0.4, 0.2]).unwrap();
    for (actual, observed) in output.into_iter().zip([0.3512967526912689f32, 0.4070727527141571, 0.2763597071170807]) {
        assert!((actual - observed).abs() <= 0.000001);
    }
}
#[test]
fn independent_hsl_and_signed_scalar_words_remain_exact_under_explicit_scalar_authority() {
    let bits = [(-0.0f32).to_bits(), 0.5f32.to_bits(), (-0.0f32).to_bits()];
    let a = Parameters::from_stored_bits(bits, [0.1f32, 0.2, 0.3].map(f32::to_bits)).unwrap();
    let b = Parameters::from_stored_bits(bits, [0.8f32, 0.9, 0.7].map(f32::to_bits)).unwrap();
    assert_ne!(a, b);
    assert_eq!(a.scalar_bits(), bits);
    assert_eq!(a.stored_hsl_bits(), [0.1f32, 0.2, 0.3].map(f32::to_bits));
    let p = prepare(a, TransferRole::CallerEstablishedIecSrgb);
    let q = prepare(b, TransferRole::CallerEstablishedIecSrgb);
    assert_eq!(p.try_eval_float_color([0.2, 0.4, 0.6]).unwrap(), q.try_eval_float_color([0.2, 0.4, 0.6]).unwrap());
}
#[test]
fn role_and_supplied_y_not_profile_derived_and_scalar_context_fenced() {
    let a = prepare(parameters([0.2, 0.5, 0.]), TransferRole::CallerEstablishedIecSrgb);
    let b = prepare(parameters([0.2, 0.5, 0.]), TransferRole::CallerEstablishedExactGamma2);
    let rgb = [0.8, 0.4, 0.2];
    assert_ne!(a.try_eval_float_color(rgb).unwrap(), b.try_eval_float_color(rgb).unwrap());
    let other = YWeights::from_bits([0.25f64, 0.649993896484375, 0.100006103515625].map(f64::to_bits)).unwrap();
    let c =
        PreparedColorize::prepare(PixelFormat::RGBA32F, context(), a.parameters(), a.policy(), a.input_profile(), other, Limits::default(), &Interrupt::NONE)
            .unwrap();
    assert_ne!(a.try_eval_float_color(rgb).unwrap(), c.try_eval_float_color(rgb).unwrap());
    assert!(a.evaluate_color_with_interrupt(rgb, context(), witness(TransferRole::CallerEstablishedExactGamma2), &Interrupt::NONE).is_err());
}
#[test]
fn color_only_roi_preserves_raw_alpha_default_outside_and_preflight_unit_refuses() {
    let s = surface(&[[f32::NAN, 0., 0., 0.], [0.8, 0.4, 0.2, f32::from_bits(0x7fc01234)], [0.2, 0.4, 0.6, 0.5]]);
    let before = s.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap();
    let p = prepare(parameters([0.2, 0.5, -0.3]), TransferRole::CallerEstablishedExactGamma2);
    let out = p.apply(&s, Rect::new(1, 0, 2, 1), context(), Limits::default(), &Interrupt::NONE).unwrap();
    let bytes = out.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap();
    assert_eq!(&bytes[..16], &before[..16]);
    assert_eq!(&bytes[32..], &before[32..]);
    assert_eq!(&bytes[28..32], &before[28..32]);
    assert_eq!(out.default_bytes(), s.default_bytes());
    assert_eq!(s.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap(), before);
    assert!(p.apply(&s, Rect::new(0, 0, 1, 1), context(), Limits::default(), &Interrupt::NONE).is_err());
}
#[test]
fn malformed_scalars_weights_roles_and_formats_refuse() {
    for values in [[f32::NAN, 0., 0.], [1.1, 0., 0.], [0., -0.1, 0.], [0., 0., -1.1]] {
        assert!(Parameters::from_stored_bits(values.map(f32::to_bits), [0; 3]).is_err());
    }
    for values in [[0.2f64, 0.2, 0.2], [f64::NAN, 0., 1.], [-0.1, 0.1, 1.]] {
        assert!(YWeights::from_bits(values.map(f64::to_bits)).is_err());
    }
    let params = parameters([0., 0., 0.]);
    let mut zero = witness(TransferRole::CallerEstablishedIecSrgb);
    zero.profile_identity = [0; 32];
    assert!(
        PreparedColorize::prepare(
            PixelFormat::RGBA32F,
            context(),
            params,
            NumericPolicy::DeclaredScalarTripletBinary64FinalFloat,
            zero,
            weights(),
            Limits::default(),
            &Interrupt::NONE
        )
        .is_err()
    );
    assert!(
        PreparedColorize::prepare(
            PixelFormat::new(ColorMode::Rgb, SampleType::F64, true),
            context(),
            params,
            NumericPolicy::DeclaredScalarTripletBinary64FinalFloat,
            witness(TransferRole::CallerEstablishedIecSrgb),
            weights(),
            Limits::default(),
            &Interrupt::NONE
        )
        .is_err()
    );
}
#[test]
fn full_native_tile_low_work_and_cancellation_do_not_reset_quota() {
    let s = surface(&[[0.8, 0.4, 0.2, 1.]]);
    let p = prepare(parameters([0.2, 0.5, 0.]), TransferRole::CallerEstablishedExactGamma2);
    let roi = Rect::new(0, 0, 1, 1);
    let req = p.requirements_with_interrupt(&s, roi, context(), &Interrupt::NONE).unwrap();
    assert!(req.work_units >= 2 * 65536 * 16);
    let budget = Limits { resident_bytes: Limits::default().resident_bytes, work_units: PREPARATION_WORK + req.work_units };
    let p = PreparedColorize::prepare(PixelFormat::RGBA32F, context(), p.parameters(), p.policy(), p.input_profile(), p.weights(), budget, &Interrupt::NONE)
        .unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 4;
    assert!(p.apply(&s, roi, context(), budget, &Interrupt::cancel_only(&cancelled)).is_err());
    assert_eq!(p.remaining_work(), 0);
    assert!(p.apply(&s, roi, context(), budget, &Interrupt::NONE).is_err());
}

#[test]
fn first_iec_packet_default_scalars_are_residual_fixture_not_raw_neutral_identity() {
    // Sealedanalysis2fc7cf44 retains strict implicit ICC and proxy-field HOLDs.
    let params = Parameters::from_stored_bits([0.5f32, 0.5, 0.].map(f32::to_bits), [0.5f32; 3].map(f32::to_bits)).unwrap();
    let p = prepare(params, TransferRole::CallerEstablishedIecSrgb);
    let input = [0.8, 0.4, 0.2];
    let output = p.try_eval_float_color(input).unwrap();
    assert_ne!(output, input);
    for (actual, observed) in output.into_iter().zip([0.3746699094772339f32, 0.6241026520729065, 0.6241026520729065]) {
        assert!((actual - observed).abs() <= 0.000001);
    }
}

#[test]
fn reserved_roi_initialization_polls_after_each_bounded_chunk() {
    let length = 3 * 64 * 1024;
    let mut raw = Vec::new();
    raw.try_reserve_exact(length).unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 1;
    assert!(matches!(initialize_with_interrupt(&mut raw, length, &Interrupt::cancel_only(&cancelled)), Err(Error::Raster(RasterInputError::Cancelled))));
    assert_eq!(raw.len(), 64 * 1024);
    assert!(raw.iter().all(|v| *v == 0));
}
