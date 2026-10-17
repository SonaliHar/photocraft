//! Ignored UNRUN loaded-word V2 contracts; no producer numerical acceptance.
use super::*;
use photocraft_raster::backing::quota::{QuotaLease, QuotaLimits};
struct Budget {
    q: BackingQuota,
    used: u64,
}
impl PlanBudget for Budget {
    type Lease = QuotaLease;
    fn reserve(&mut self, n: u64) -> Result<QuotaLease, E> {
        self.q.reserve(QuotaKind::Resident, n).map_err(E::Quota)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.used = self.used.checked_add(n).ok_or_else(length)?;
        Ok(())
    }
}
struct Held {
    value: PreparedExposure,
    _fee: QuotaLease,
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
#[ignore = "UNRUN exactly two rawblack endpoint words map effective+0 only in loadedV2; strictoldconstructor refuses; inward/signedzero/overflow remain exact"]
fn raw_words_and_effective_parameters_are_distinct() {
    for black in [0x3dcc_cccd, 0xbdcc_cccd] {
        assert!(Parameters::from_words(0.5f32.to_bits(), black).is_err());
        let p = LoadedParameters::from_words(0.5f32.to_bits(), black).unwrap();
        assert_eq!(p.raw_words(), [0.5f32.to_bits(), black]);
        assert_eq!(p.effective().values(), [0.5, 0.]);
    }
    for black in [0x3dcc_cccc, 0xbdcc_cccc, 0, (-0f32).to_bits(), 0.0625f32.to_bits()] {
        let old = Parameters::from_words(0.5f32.to_bits(), black).unwrap();
        let loaded = LoadedParameters::from_words(0.5f32.to_bits(), black).unwrap();
        assert_eq!(loaded.effective().values().map(f64::to_bits), old.values().map(f64::to_bits));
        assert_eq!(loaded.raw_words()[1], black);
    }
    for black in [0x3dcc_ccce, 0xbdcc_ccce, f32::NAN.to_bits(), f32::INFINITY.to_bits()] {
        assert!(LoadedParameters::from_words(0, black).is_err());
    }
    assert!(LoadedParameters::from_words(f32::MAX.to_bits(), 0x3dcc_cccd).is_err());
    assert_ne!(LOADED_POINT_EPOCH, LOADED_MIXED_EPOCH);
    assert_ne!(EPOCH, LOADED_MIXED_EPOCH);
}
#[test]
#[ignore = "UNRUN loaded stricttwoF32 transport retains sourceorder/rawbits and duplicate/name/type refusal without editingarguments"]
fn transport_is_not_normalization() {
    use photocraft_doc::xcf_filters::ExactNumber;
    let arguments = vec![
        FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0xbdcc_cccd)) },
        FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) },
    ];
    let prior = arguments.clone();
    let p = loaded_parameters(&arguments).unwrap();
    assert_eq!(p.raw_words(), [0.5f32.to_bits(), 0xbdcc_cccd]);
    assert_eq!(arguments, prior);
    assert!(parameters(&arguments).is_err());
    let mut bad = arguments.clone();
    bad[1].name = "black-level".into();
    assert!(loaded_parameters(&bad).is_err());
    let mut bad = arguments.clone();
    bad[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.1f64.to_bits()));
    assert!(loaded_parameters(&bad).is_err());
}
#[test]
#[ignore = "UNRUN samepaidwidekernel/effectivezero reference, neutralanddisabledwidebits and perrequestsamequota cleanup; no rawreplacement or Float boundary"]
fn loaded_effective_zero_reuses_paid_kernel() {
    let q = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 1 << 20 });
    let baseline = q.usage().unwrap();
    for black in [0x3dcc_cccd, 0xbdcc_cccd] {
        let mut b = Budget { q: q.clone(), used: 0 };
        let (value, fee) =
            PreparedExposure::prepare(LoadedParameters::from_words(0.5f32.to_bits(), black).unwrap().effective(), 128, &Interrupt::NONE, &mut b).unwrap();
        let h = Held { value, _fee: fee };
        let p = [0.125, 0.5 + 2f64.powi(-45), 0.75, f64::from_bits(1f64.to_bits() - 1)];
        let curve = photocraft_cms::curve::srgb_trc();
        let expected = [
            curve.eval_inverse64(curve.eval64(p[0]) * 0.5f64.exp2()),
            curve.eval_inverse64(curve.eval64(p[1]) * 0.5f64.exp2()),
            curve.eval_inverse64(curve.eval64(p[2]) * 0.5f64.exp2()),
            p[3],
        ];
        assert_eq!(h.value.evaluate(p, true, false, &q, &ctl()).unwrap().map(f64::to_bits), expected.map(f64::to_bits));
        assert_eq!(h.value.evaluate(p, false, false, &q, &ctl()).unwrap().map(f64::to_bits), p.map(f64::to_bits));
        drop(h);
        assert_eq!(q.usage().unwrap(), baseline);
        let (value, fee) = PreparedExposure::prepare(LoadedParameters::from_words(0, black).unwrap().effective(), 128, &Interrupt::NONE, &mut b).unwrap();
        let h = Held { value, _fee: fee };
        assert_eq!(h.value.evaluate(p, true, false, &q, &ctl()).unwrap().map(f64::to_bits), p.map(f64::to_bits));
        drop(h);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
