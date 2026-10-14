//! Synthetic candidate policy drafts, UNRUN; no GIMP output/source qualification.
use super::*;
fn context() -> WorkingContext {
    WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] }
}
fn prepare(format: PixelFormat, brightness: f64, contrast: f64) -> PreparedBrightnessContrast {
    PreparedBrightnessContrast::prepare(format, context(), Parameters { brightness, contrast }, Limits::default(), &Interrupt::NONE).unwrap()
}
#[test]
fn declared_analytic_brightness_precedes_contrast_and_finite_hdr_is_unclipped() {
    let prepared = prepare(PixelFormat::RGBA32F, 0.25, 0.25);
    let x = 0.25f32;
    // Independently spell both orders; the coupled case rejects their alias.
    let decoded = ((f64::from(x) + 0.055) / 1.055).powf(2.4);
    let half = 0.125;
    let slope = (1.25 * std::f64::consts::PI / 4.).tan();
    let bright = decoded + (1. - decoded) * half;
    let y = (bright - 0.5) * slope + 0.5;
    let expected = (if y <= 0.0031308 { 12.92 * y } else { 1.055 * y.powf(1. / 2.4) - 0.055 }) as f32;
    assert_eq!(prepared.try_eval_float_color(x).unwrap().to_bits(), expected.to_bits());
    let before = (decoded - 0.5) * slope + 0.5;
    let wrong = before + (1. - before) * half;
    let wrong = (if wrong <= 0.0031308 { 12.92 * wrong } else { 1.055 * wrong.powf(1. / 2.4) - 0.055 }) as f32;
    assert_ne!(expected.to_bits(), wrong.to_bits());
    let contrast = prepare(PixelFormat::RGBA32F, 0., 0.25);
    assert!(contrast.try_eval_float_color(0.).unwrap() < 0.);
    assert!(contrast.try_eval_float_color(1.).unwrap() > 1.);
    assert!(contrast.try_eval_float_color(-0.25).unwrap().is_finite());
    assert!(contrast.try_eval_float_color(f32::NAN).is_err());
}
#[test]
fn float_and_double_storage_preserve_raw_alpha_default_and_unselected_nonfinite_words() {
    for sample in [SampleType::F32, SampleType::F64] {
        let format = PixelFormat { mode: ColorMode::Grayscale, sample, alpha: true };
        let (outside, color, alpha) = if sample == SampleType::F64 {
            (0x7ff8_1234_5678_9abcu64.to_ne_bytes().to_vec(), (0.25f64.to_bits() + 1).to_ne_bytes().to_vec(), 0xfff8_0000_0000_0042u64.to_ne_bytes().to_vec())
        } else {
            (0x7fc0_1234u32.to_ne_bytes().to_vec(), 0.25f32.to_ne_bytes().to_vec(), 0xffc0_0042u32.to_ne_bytes().to_vec())
        };
        let default = [outside.clone(), alpha.clone()].concat();
        let mut source = Surface::try_with_default_bytes(format, &default).unwrap();
        let extent = Rect::new(-1, 0, 1, 1);
        let roi = Rect::new(0, 0, 1, 1);
        let raw = [outside.clone(), alpha.clone(), color, alpha.clone()].concat();
        source.try_write_interleaved_with(extent, &raw, &Interrupt::NONE).unwrap();
        let before = source.clone();
        let prepared = prepare(format, 0.25, 0.25);
        let expected = prepared.try_eval_float_color(0.25).unwrap();
        let output = prepared.apply(&source, roi, context(), Limits::default(), &Interrupt::NONE).unwrap();
        let encoded = if sample == SampleType::F64 { f64::from(expected).to_ne_bytes().to_vec() } else { expected.to_ne_bytes().to_vec() };
        assert_eq!(output.try_read_interleaved(roi).unwrap(), [encoded, alpha].concat());
        assert_eq!(output.default_bytes(), default);
        assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap(), source.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap());
        assert_eq!(source, before);
    }
}
#[test]
fn closed_parameters_context_and_preallocation_quota_refuse_without_changing_source() {
    let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
    for parameters in [
        Parameters { brightness: 0., contrast: 1. },
        Parameters { brightness: 0., contrast: -1. },
        Parameters { brightness: f64::NAN, contrast: 0. },
        Parameters { brightness: 1.01, contrast: 0. },
    ] {
        assert!(PreparedBrightnessContrast::prepare(format, context(), parameters, Limits::default(), &Interrupt::NONE).is_err());
    }
    assert!(
        PreparedBrightnessContrast::prepare(
            format,
            WorkingContext { profile_identity: [0; 32], ..context() },
            Parameters { brightness: 0., contrast: 0. },
            Limits::default(),
            &Interrupt::NONE
        )
        .is_err()
    );
    assert!(
        PreparedBrightnessContrast::prepare(
            format,
            WorkingContext { encoding: SampleEncoding::Linear, ..context() },
            Parameters { brightness: 0., contrast: 0. },
            Limits::default(),
            &Interrupt::NONE
        )
        .is_err()
    );
    let roi = Rect::new(0, 0, 1, 1);
    let source = Surface::try_with_default_bytes(format, &0.25f64.to_ne_bytes()).unwrap();
    let before = source.clone();
    let prepared = prepare(format, 0.25, 0.25);
    assert!(matches!(
        prepared.apply(&source, roi, context(), Limits { resident_bytes: 1, work_units: 0 }, &Interrupt::NONE),
        Err(Error::Raster(RasterInputError::LimitExceeded))
    ));
    assert_eq!(source, before);
}
#[test]
fn cancellation_after_prepaid_work_has_no_refund_or_partial_publication() {
    let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: false };
    let roi = Rect::new(0, 0, 1, 1);
    let source = Surface::try_from_interleaved(format, roi, &0.25f32.to_ne_bytes()).unwrap();
    let before = source.clone();
    let prepared = prepare(format, 0.25, 0.25);
    let initial = prepared.remaining_work();
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
    assert!(matches!(
        prepared.apply(&source, roi, context(), Limits::default(), &Interrupt::cancel_only(&cancelled)),
        Err(Error::Raster(RasterInputError::Cancelled))
    ));
    assert!(prepared.remaining_work() < initial);
    assert_eq!(source, before);
}

#[test]
fn single_pixel_work_admits_complete_native_tile_initialization_before_allocation() {
    let format = PixelFormat::RGBA32F;
    let roi = Rect::new(0, 0, 1, 1);
    let default = [0.25f32, 0.25, 0.25, 1.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
    let source = Surface::try_with_default_bytes(format, &default).unwrap();
    let before = source.clone();
    let prepared = prepare(format, 0.25, 0.25);
    let initial = prepared.remaining_work();
    let needs = prepared.requirements_with_interrupt(&source, roi, context(), &Interrupt::NONE).unwrap();
    assert!(needs.work_units >= 2 * 65536 * 16);
    assert!(matches!(
        prepared.apply(&source, roi, context(), Limits { work_units: 5000, ..Limits::default() }, &Interrupt::NONE),
        Err(Error::Raster(RasterInputError::LimitExceeded))
    ));
    assert_eq!(prepared.remaining_work(), initial);
    assert_eq!(source, before);
}
