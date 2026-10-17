//! UNRUN stage-boundary regressions, not a physical CMS/GIMP numerical oracle.
use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

fn context() -> Context {
    Context {
        rgb: WorkingContext { transfer: Transfer::Linear, profile_identity: [7; 32], color_channels: 3, alpha: true },
        lch: LchFloatContext { profile_identity: [7; 32], space_identity: [9; 32] },
    }
}
// A fixed test-only stage witness; it supplies no real profile or source grant.
struct Probe {
    context: Context,
    phase: AtomicU8,
    seen: Cell<Option<[u32; 4]>>,
    expected_chroma: f32,
}
impl Probe {
    fn new(chroma: f32) -> Self {
        Self { context: context(), phase: AtomicU8::new(0), seen: Cell::new(None), expected_chroma: chroma }
    }
}
impl FloatConversions for Probe {
    fn context(&self) -> Context {
        self.context
    }
    fn requirements(&self, _: &Interrupt<'_>) -> Result<ConversionRequirements, E> {
        Ok(ConversionRequirements { retained_bytes: 0, scratch_bytes: 0, preparation_work_units: 0, work_units: 1 })
    }
    fn rgb_to_lch(&self, value: [f32; 4], _: &Interrupt<'_>) -> Result<[f32; 4], E> {
        assert_eq!(self.phase.swap(1, Ordering::Relaxed), 0);
        self.seen.set(Some(value.map(f32::to_bits)));
        Ok([50.0, 40.0, 30.0, value[3]])
    }
    fn lch_to_rgb(&self, value: [f32; 4], _: &Interrupt<'_>) -> Result<[f32; 4], E> {
        assert_eq!(self.phase.swap(2, Ordering::Relaxed), 1);
        assert_eq!(value[..3], [50.0, self.expected_chroma, 30.0]);
        Ok([0.875, 0.125, 0.5, value[3]])
    }
}

#[test]
#[ignore = "UNRUN exact scale-one stage preserves all Float bits but still runs both physical conversion callbacks"]
fn exact_scale_one_preserves_words_without_whole_operation_shortcut() {
    for alpha in [0.0f32, -0.0, 0.375, 1.0, f32::from_bits(1.0f32.to_bits() - 1)] {
        let input = [f32::from_bits(1), -0.0, f32::from_bits(0x3f00_0001), alpha];
        assert_eq!(saturation_linear_float(input, 1.0).unwrap().map(f32::to_bits), input.map(f32::to_bits));
        let probe = Probe::new(40.0);
        let output = evaluate(input, Parameters { vibrance: 0.0, saturation: 1.0 }, context(), &probe, &Interrupt::NONE).unwrap();
        assert_eq!(probe.phase.load(Ordering::Relaxed), 2);
        assert_eq!(probe.seen.get(), Some(input.map(f32::to_bits)));
        assert_eq!(output[..3], [0.875, 0.125, 0.5]);
        assert_eq!(output[3].to_bits(), alpha.to_bits());
    }
}

#[test]
#[ignore = "UNRUN exact f64 scale comparison keeps neighboring value on old formula; noncommuting chroma stage continues"]
fn only_exact_one_is_identity_and_other_stage_arithmetic_stays_existing() {
    let input = [0.00001f32, 0.25, 0.75, -0.0];
    let adjacent = f64::from_bits(1.0f64.to_bits() + 1);
    assert_eq!(adjacent as f32, 1.0);
    for scale in [0.0, 0.5, adjacent, 10.0] {
        let mut expected = [0.0f64; 4];
        PointOperation::Saturation { scale, colorspace: 0 }.try_eval_pixel(&input.map(f64::from), 3, &mut expected).unwrap();
        let expected = expected.map(|value| value as f32);
        let actual = saturation_linear_float(input, scale).unwrap();
        assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
        if scale == adjacent {
            assert_ne!(actual.map(f32::to_bits), input.map(f32::to_bits));
        }
        if scale == 10.0 {
            assert!(actual[..3].iter().any(|value| *value < 0.0 || *value > 1.0));
        }
        let probe = Probe::new(47.5);
        let output = evaluate(input, Parameters { vibrance: 7.5, saturation: scale }, context(), &probe, &Interrupt::NONE).unwrap();
        assert_eq!(probe.seen.get(), Some(expected.map(f32::to_bits)));
        assert_eq!(probe.phase.load(Ordering::Relaxed), 2);
        assert_eq!(output[3].to_bits(), (-0.0f32).to_bits());
    }
}

#[test]
#[ignore = "UNRUN identity stage keeps unit/finite/context refusal and before/between-stage cancellation"]
fn identity_does_not_skip_validation_or_cancellation() {
    let valid = [0.00001f32, 0.25, 0.75, 0.5];
    for scale in [f64::NAN, f64::INFINITY, -0.01, 10.01] {
        assert!(saturation_linear_float(valid, scale).is_err());
    }
    for bad in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        for channel in 0..4 {
            let mut input = valid;
            input[channel] = bad;
            assert!(saturation_linear_float(input, 1.0).is_err());
            let probe = Probe::new(40.0);
            assert!(evaluate(input, Parameters { vibrance: 0.0, saturation: 1.0 }, context(), &probe, &Interrupt::NONE).is_err());
            assert_eq!(probe.phase.load(Ordering::Relaxed), 0);
        }
    }
    let probe = Probe::new(40.0);
    let mut wrong = context();
    wrong.lch.profile_identity = [8; 32];
    assert!(evaluate(valid, Parameters { vibrance: 0.0, saturation: 1.0 }, wrong, &probe, &Interrupt::NONE).is_err());
    assert_eq!(probe.phase.load(Ordering::Relaxed), 0);
    assert_eq!(evaluate(valid, Parameters { vibrance: 0.0, saturation: 1.0 }, context(), &probe, &Interrupt::cancel_only(&|| true)), Err(E::Cancelled));
    assert_eq!(probe.phase.load(Ordering::Relaxed), 0);
    let between = || probe.phase.load(Ordering::Relaxed) == 1;
    assert_eq!(evaluate(valid, Parameters { vibrance: 0.0, saturation: 1.0 }, context(), &probe, &Interrupt::cancel_only(&between)), Err(E::Cancelled));
    assert_eq!(probe.phase.load(Ordering::Relaxed), 1);
    assert_eq!(probe.seen.get(), Some(valid.map(f32::to_bits)));
}
