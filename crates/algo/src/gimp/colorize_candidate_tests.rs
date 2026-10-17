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
    for (actual, observed) in output.into_iter().zip([0.351_296_75_f32, 0.407_072_75, 0.276_359_7]) {
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
    for (actual, observed) in output.into_iter().zip([0.374_669_9_f32, 0.624_102_65, 0.624_102_65]) {
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

#[test]
fn declared_hsl_authority_keeps_conflicting_scalar_words_independent() {
    let hsl = [0.25f32, 0.75, 0.25].map(f32::to_bits);
    let a = Parameters::from_stored_bits([0.0f32, 0.25, 0.5].map(f32::to_bits), hsl).unwrap();
    let b = Parameters::from_stored_bits([0.75f32, 0.5, -0.5].map(f32::to_bits), hsl).unwrap();
    for role in [TransferRole::CallerEstablishedIecSrgb, TransferRole::CallerEstablishedExactGamma2] {
        let make = |params, policy| {
            PreparedColorize::prepare(PixelFormat::RGBA32F, context(), params, policy, witness(role), weights(), Limits::default(), &Interrupt::NONE).unwrap()
        };
        let scalar_a = make(a, NumericPolicy::DeclaredScalarTripletBinary64FinalFloat);
        let scalar_b = make(b, NumericPolicy::DeclaredScalarTripletBinary64FinalFloat);
        let hsl_a = make(a, NumericPolicy::DeclaredHslTripletBinary64FinalFloat);
        let hsl_b = make(b, NumericPolicy::DeclaredHslTripletBinary64FinalFloat);
        let input = [0.8, 0.4, 0.2];
        assert_ne!(scalar_a.try_eval_float_color(input).unwrap(), scalar_b.try_eval_float_color(input).unwrap());
        assert_eq!(hsl_a.try_eval_float_color(input).unwrap(), hsl_b.try_eval_float_color(input).unwrap());
        assert_ne!(hsl_a.try_eval_float_color(input).unwrap(), scalar_a.try_eval_float_color(input).unwrap());
        assert_eq!(hsl_a.parameters(), a);
        assert_eq!(hsl_b.parameters(), b);
    }
}
#[test]
fn hsl_mid_lightness_matches_explicit_signed_zero_without_raw_neutral_shortcut() {
    let params = Parameters::from_stored_bits([0.25f32, 0., 0.].map(f32::to_bits), [0.25f32, 0., 0.5].map(f32::to_bits)).unwrap();
    let scalar = prepare(params, TransferRole::CallerEstablishedExactGamma2);
    let hsl = PreparedColorize::prepare(
        PixelFormat::RGBA32F,
        context(),
        params,
        NumericPolicy::DeclaredHslTripletBinary64FinalFloat,
        witness(TransferRole::CallerEstablishedExactGamma2),
        weights(),
        Limits::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let input = [0.8, 0.4, 0.2];
    let expected = scalar.try_eval_float_color(input).unwrap();
    assert_eq!(hsl.try_eval_float_color(input).unwrap(), expected);
    assert_ne!(expected, input);
    assert_eq!(hsl.parameters().scalar_bits(), params.scalar_bits());
    assert_eq!(hsl.parameters().stored_hsl_bits(), params.stored_hsl_bits());
    assert_eq!(hsl.sample_work(), scalar.sample_work());
}
#[test]
fn hsl_policy_native_roi_and_cancellation_keep_same_raw_alpha_default_and_quota() {
    let params = Parameters::from_stored_bits([0.75f32, 0.25, 0.5].map(f32::to_bits), [0.25f32, 0.5, 0.25].map(f32::to_bits)).unwrap();
    let p = PreparedColorize::prepare(
        PixelFormat::RGBA32F,
        context(),
        params,
        NumericPolicy::DeclaredHslTripletBinary64FinalFloat,
        witness(TransferRole::CallerEstablishedIecSrgb),
        weights(),
        Limits::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let s = surface(&[[f32::NAN, 0., 0., 0.], [0.8, 0.4, 0.2, f32::from_bits(0x7fc01234)], [0.2, 0.4, 0.6, -0.]]);
    let before = s.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap();
    let out = p.apply(&s, Rect::new(1, 0, 2, 1), context(), Limits::default(), &Interrupt::NONE).unwrap();
    let raw = out.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap();
    assert_eq!(&raw[..16], &before[..16]);
    assert_eq!(&raw[28..], &before[28..]);
    assert_eq!(out.default_bytes(), s.default_bytes());
    let stop = || true;
    let left = p.remaining_work();
    assert!(matches!(
        p.evaluate_color_with_interrupt([0.8, 0.4, 0.2], context(), p.transfer_witness(), &Interrupt::cancel_only(&stop)),
        Err(Error::Raster(RasterInputError::Cancelled))
    ));
    assert_eq!(p.remaining_work(), left);
}

#[test]
fn declared_four_order_replays_contrast_independent_component_authorities() {
    use StoredField::{Color, Hue, Lightness, Saturation};
    let raw = Parameters::from_stored_bits([0.125f32, 0.75, 0.25].map(f32::to_bits), [0.25f32, 0.5, 0.375].map(f32::to_bits)).unwrap();
    assert_eq!(raw.stored_order(), None);
    let cases = [
        ([Hue, Saturation, Lightness, Color], [0.25f32, 0.5, -0.25]),
        ([Color, Hue, Saturation, Lightness], [0.125, 0.75, 0.25]),
        ([Hue, Color, Saturation, Lightness], [0.25, 0.75, 0.25]),
        ([Hue, Saturation, Color, Lightness], [0.25, 0.5, 0.25]),
    ];
    for role in [TransferRole::CallerEstablishedIecSrgb, TransferRole::CallerEstablishedExactGamma2] {
        let mut observed = Vec::new();
        for (order, expected) in cases {
            let ordered = raw.with_order(order).unwrap();
            let p = PreparedColorize::prepare(
                PixelFormat::RGBA32F,
                context(),
                ordered,
                NumericPolicy::DeclaredStoredOrderBinary64FinalFloat,
                witness(role),
                weights(),
                Limits::default(),
                &Interrupt::NONE,
            )
            .unwrap();
            let reference = prepare(Parameters::from_stored_bits(expected.map(f32::to_bits), raw.stored_hsl_bits()).unwrap(), role);
            let actual = p.try_eval_float_color([0.8, 0.4, 0.2]).unwrap();
            assert_eq!(actual, reference.try_eval_float_color([0.8, 0.4, 0.2]).unwrap());
            observed.push(actual);
            assert_eq!(p.parameters().scalar_bits(), raw.scalar_bits());
            assert_eq!(p.parameters().stored_hsl_bits(), raw.stored_hsl_bits());
            assert_eq!(p.parameters().stored_order(), Some(order));
        }
        for i in 0..observed.len() {
            for j in i + 1..observed.len() {
                assert_ne!(observed[i], observed[j]);
            }
        }
    }
}
#[test]
fn stored_order_requires_complete_order_and_is_not_attached_to_legacy_parameters() {
    use StoredField::{Color, Hue, Lightness, Saturation};
    let raw = parameters([0.125, 0.75, 0.25]);
    assert!(raw.with_order([Hue, Hue, Lightness, Color]).is_err());
    assert!(matches!(
        PreparedColorize::prepare(
            PixelFormat::RGBA32F,
            context(),
            raw,
            NumericPolicy::DeclaredStoredOrderBinary64FinalFloat,
            witness(TransferRole::CallerEstablishedIecSrgb),
            weights(),
            Limits::default(),
            &Interrupt::NONE
        ),
        Err(Error::Parameters)
    ));
    let ordered = raw.with_order([Color, Hue, Saturation, Lightness]).unwrap();
    for policy in [NumericPolicy::DeclaredScalarTripletBinary64FinalFloat, NumericPolicy::DeclaredHslTripletBinary64FinalFloat] {
        assert!(matches!(
            PreparedColorize::prepare(
                PixelFormat::RGBA32F,
                context(),
                ordered,
                policy,
                witness(TransferRole::CallerEstablishedIecSrgb),
                weights(),
                Limits::default(),
                &Interrupt::NONE
            ),
            Err(Error::Parameters)
        ));
    }
    assert_eq!(raw, Parameters::from_stored_bits(raw.scalar_bits(), raw.stored_hsl_bits()).unwrap());
    assert_eq!(raw.stored_order(), None);
}
#[test]
fn stored_order_same_pixel_quota_and_cancellation_do_not_reset() {
    let params = parameters([0.125, 0.75, 0.25]).with_order([StoredField::Hue, StoredField::Color, StoredField::Saturation, StoredField::Lightness]).unwrap();
    let limits = Limits { work_units: PREPARATION_WORK + SAMPLE_WORK, ..Limits::default() };
    let p = PreparedColorize::prepare(
        PixelFormat::RGBA32F,
        context(),
        params,
        NumericPolicy::DeclaredStoredOrderBinary64FinalFloat,
        witness(TransferRole::CallerEstablishedIecSrgb),
        weights(),
        limits,
        &Interrupt::NONE,
    )
    .unwrap();
    p.evaluate_color_with_interrupt([0.8, 0.4, 0.2], context(), p.transfer_witness(), &Interrupt::NONE).unwrap();
    assert_eq!(p.remaining_work(), 0);
    assert!(p.evaluate_color_with_interrupt([0.8, 0.4, 0.2], context(), p.transfer_witness(), &Interrupt::NONE).is_err());
    let stop = || true;
    assert!(matches!(
        p.evaluate_color_with_interrupt([0.8, 0.4, 0.2], context(), p.transfer_witness(), &Interrupt::cancel_only(&stop)),
        Err(Error::Raster(RasterInputError::Cancelled))
    ));
    assert_eq!(p.remaining_work(), 0);
}
