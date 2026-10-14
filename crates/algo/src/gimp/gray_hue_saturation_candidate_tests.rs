//! Declared-math regression drafts, UNRUN; not qualified GIMP/source equality.
use super::*;
fn params(lightness: f64) -> Parameters {
    let mut p = Parameters { time_bits: Some(0), ranges: [[0; 3]; 7], overlap_bits: 0 };
    p.ranges[0][2] = lightness.to_bits();
    p
}
fn witness(role: InputRole) -> InputProfileWitness {
    InputProfileWitness { role, profile_identity: [7; 32] }
}
fn context(role: InputRole) -> WorkingContext {
    WorkingContext {
        profile_identity: [7; 32],
        encoding: match role {
            InputRole::CallerEstablishedLinearGray => SampleEncoding::Linear,
            InputRole::CallerEstablishedExactGamma2Gray => SampleEncoding::NonlinearProfile,
        },
    }
}
fn prepare(format: PixelFormat, role: InputRole, p: Parameters) -> PreparedGrayHueSaturation {
    PreparedGrayHueSaturation::prepare(
        format,
        context(role),
        p,
        NumericPolicy::DeclaredPerceptualIecSignedLightnessFinalFloat,
        witness(role),
        Limits::default(),
        &Interrupt::NONE,
    )
    .unwrap()
}
fn format(sample: SampleType, alpha: bool) -> PixelFormat {
    PixelFormat { mode: ColorMode::Grayscale, sample, alpha }
}
#[test]
fn explicit_roles_fit_scoped_author_controls_without_claiming_exact_replay() {
    let gamma = InputRole::CallerEstablishedExactGamma2Gray;
    let linear = InputRole::CallerEstablishedLinearGray;
    for (role, l, input, observed) in
        [(gamma, 0.25, 0.125, 0.31569692), (gamma, -0.25, 0.5, 0.36722782), (linear, 0.25, 0.5, 0.60640293), (linear, -0.25, 0.5, 0.2648615)]
    {
        let candidate = prepare(format(SampleType::F32, false), role, params(l));
        assert!((candidate.try_eval_float_gray(input).unwrap() - observed).abs() < 5e-7);
    }
    // Direct nonlinear L changes(.5â†’.625) are a different mathematical recipe.
    let x = prepare(format(SampleType::F32, false), gamma, params(0.25)).try_eval_float_gray(0.5).unwrap();
    assert!((x - 0.625).abs() > 0.005);
}
#[test]
fn roles_profile_and_nonzero_context_are_independent_mandatory_witnesses() {
    let linear = InputRole::CallerEstablishedLinearGray;
    let gamma = InputRole::CallerEstablishedExactGamma2Gray;
    for (ctx, profile) in [
        (context(linear), witness(gamma)),
        (WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [8; 32] }, witness(linear)),
        (WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [0; 32] }, InputProfileWitness { role: linear, profile_identity: [0; 32] }),
    ] {
        assert!(matches!(
            PreparedGrayHueSaturation::prepare(
                format(SampleType::F32, false),
                ctx,
                params(0.),
                NumericPolicy::DeclaredPerceptualIecSignedLightnessFinalFloat,
                profile,
                Limits::default(),
                &Interrupt::NONE
            ),
            Err(Error::ContextMismatch)
        ));
    }
    let mut p = params(0.);
    p.ranges[0][0] = (-0f64).to_bits();
    p.ranges[6][1] = (-0f64).to_bits();
    let candidate = prepare(format(SampleType::F32, false), linear, p);
    assert_eq!(candidate.parameters(), p);
    assert_eq!(candidate.input_profile(), witness(linear));
    p.ranges[1][0] = 0.25f64.to_bits();
    assert!(matches!(
        PreparedGrayHueSaturation::prepare(
            format(SampleType::F32, false),
            context(linear),
            p,
            NumericPolicy::DeclaredPerceptualIecSignedLightnessFinalFloat,
            witness(linear),
            Limits::default(),
            &Interrupt::NONE
        ),
        Err(Error::UnsupportedConfiguration)
    ));
}
#[test]
fn neutral_hue_and_saturation_still_process_declared_float_chain() {
    let role = InputRole::CallerEstablishedLinearGray;
    let fmt = format(SampleType::F64, true);
    let roi = Rect::new(0, 0, 1, 1);
    let input = 0.5f64.to_bits() + 1;
    let alpha = 0xfff8_0000_0000_0042u64;
    let source = Surface::try_from_interleaved(fmt, roi, &[input, alpha].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    let before = source.clone();
    for lane in [0, 1] {
        let mut p = params(0.);
        p.ranges[0][lane] = 0.25f64.to_bits();
        let candidate = prepare(fmt, role, p);
        let output = candidate.apply(&source, roi, context(role), Limits::default(), &Interrupt::NONE).unwrap();
        let bytes = output.try_read_interleaved(roi).unwrap();
        assert_eq!(&bytes[..8], &0.5f64.to_ne_bytes());
        assert_ne!(&bytes[..8], &input.to_ne_bytes());
        assert_eq!(&bytes[8..], &alpha.to_ne_bytes());
    }
    assert_eq!(source, before);
}
#[test]
fn f64_original_unit_admission_precedes_float_and_preserves_unselected_native_state() {
    let role = InputRole::CallerEstablishedExactGamma2Gray;
    let fmt = format(SampleType::F64, true);
    let roi = Rect::new(0, 0, 1, 1);
    let alpha = 0xfff8_0000_0000_0042u64;
    let outside = [0x7ff8_1234_5678_9abcu64, alpha];
    let default = outside.into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
    let mut source = Surface::try_with_default_bytes(fmt, &default).unwrap();
    let bad = f64::from_bits(1f64.to_bits() + 1);
    source.try_write_interleaved_with(roi, &[bad.to_bits(), alpha].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>(), &Interrupt::NONE).unwrap();
    let candidate = prepare(fmt, role, params(0.25));
    let before = source.clone();
    assert!(matches!(candidate.apply(&source, roi, context(role), Limits::default(), &Interrupt::NONE), Err(Error::OutsideUnit)));
    assert_eq!(source, before);
    source.try_write_interleaved_with(roi, &[0.5f64.to_bits(), alpha].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>(), &Interrupt::NONE).unwrap();
    let output = candidate.apply(&source, roi, context(role), Limits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(output.default_bytes(), default);
    assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap(), default);
    assert_eq!(&output.try_read_interleaved(roi).unwrap()[8..], &alpha.to_ne_bytes());
}
#[test]
fn double_math_work_full_tile_cost_quota_and_cancel_are_not_reset() {
    let role = InputRole::CallerEstablishedLinearGray;
    let fmt = format(SampleType::F32, false);
    let roi = Rect::new(-1, 0, 1, 1);
    let source = Surface::try_from_interleaved(fmt, roi, &[0.25f32, 0.75].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    let candidate = prepare(fmt, role, params(-0.25));
    let needs = candidate.requirements_with_interrupt(&source, roi, context(role), &Interrupt::NONE).unwrap();
    assert!(needs.work_units >= 2 * SAMPLE_WORK * 2 + 2 * 65536 * 4);
    let remaining = candidate.remaining_work();
    assert!(matches!(
        candidate.apply(&source, roi, context(role), Limits { work_units: needs.work_units - 1, ..Limits::default() }, &Interrupt::NONE),
        Err(Error::Raster(RasterInputError::LimitExceeded))
    ));
    assert_eq!(candidate.remaining_work(), remaining);
    let before = source.clone();
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 4;
    assert!(matches!(
        candidate.apply(&source, roi, context(role), Limits::default(), &Interrupt::cancel_only(&cancel)),
        Err(Error::Raster(RasterInputError::Cancelled))
    ));
    assert!(candidate.remaining_work() < remaining);
    assert_eq!(source, before);
}
