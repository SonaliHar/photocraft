//! UNRUN numeric/preparation contracts only; no source or GIMP equality claim.
use super::*;
use photocraft_raster::backing::quota::{QuotaLease, QuotaLimits};
struct Budget {
    q: BackingQuota,
    used: u64,
    limit: u64,
}
impl PlanBudget for Budget {
    type Lease = QuotaLease;
    fn reserve(&mut self, n: u64) -> Result<QuotaLease, E> {
        self.q.reserve(QuotaKind::Resident, n).map_err(E::Quota)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.used = self.used.checked_add(n).ok_or_else(length)?;
        if self.used > self.limit { Err(E::Backing(BackingError::Work)) } else { Ok(()) }
    }
}
struct Held {
    value: PreparedExposure,
    _fee: QuotaLease,
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 1 << 20 })
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn held(q: &BackingQuota, stops: f32, black: f32) -> Held {
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let (value, fee) = PreparedExposure::prepare(Parameters::from_words(stops.to_bits(), black.to_bits()).unwrap(), 128, &Interrupt::NONE, &mut b).unwrap();
    Held { value, _fee: fee }
}
#[test]
#[ignore = "UNRUN exact F32 parameters and promoted black-bound/scale overflow, not source/epoch admission; fixed wide numeric recipe only"]
fn stored_parameters_are_exact_and_checked() {
    let p = Parameters::from_words(0.5f32.to_bits(), 0.0625f32.to_bits()).unwrap();
    assert_eq!(p.values(), [0.5, 0.0625]);
    assert!(Parameters::from_words(0.5f32.to_bits(), 0.1f32.to_bits()).is_err());
    assert!(Parameters::from_words(0.5f32.to_bits(), (-0.1f32).to_bits()).is_err());
    for (s, b) in [(f32::NAN, 0.), (f32::INFINITY, 0.), (f32::MAX, 0.), (0., f32::NAN), (0., 0.125)] {
        assert!(Parameters::from_words(s.to_bits(), b.to_bits()).is_err());
    }
    let args = [
        FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.0625f32.to_bits())) },
        FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) },
    ];
    assert_eq!(parameters(&args).unwrap(), p);
    let mut bad = args.clone();
    bad[1].name = "black-level".into();
    assert!(parameters(&bad).is_err());
    let p = Parameters::from_words((-0f32).to_bits(), (-0f32).to_bits()).unwrap();
    assert_eq!(p.values().map(f64::to_bits), [(-0f64).to_bits(); 2]);
}
#[test]
#[ignore = "UNRUN independent public transfer and exact existing scalar f64 order, strictunit vs derivedfinite, wide alpha/identity/disabled and overflow refusal; no ExposureFloat boundary"]
fn singleton_wide_math_and_additive_derived_input() {
    let q = q();
    let h = held(&q, 0.5, 0.0625);
    let curve = photocraft_cms::srgb_trc();
    for p in [[0., 0.25, 1., -0.], [0.125, 0.5, 0.75, 0.375], [0.5 + 2f64.powi(-40), 0.25, 0.75, f64::from_bits(1f64.to_bits() - 1)], [-0.125, 1.25, 0.75, 0.5]]
    {
        let derived = p[..3].iter().any(|v| !(0.0..=1.0).contains(v));
        if derived {
            assert!(h.value.evaluate(p, true, false, &q, &ctl()).is_err());
        }
        let mut expected = p;
        for i in 0..3 {
            let linear = curve.eval64(p[i]);
            let operated = (linear - 0.0625) / ((-0.5f64).exp2() - 0.0625);
            expected[i] = curve.eval_inverse64(operated);
        }
        let actual = h.value.evaluate(p, true, derived, &q, &ctl()).unwrap();
        assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        assert_eq!(actual[3].to_bits(), p[3].to_bits());
        assert_eq!(h.value.evaluate(p, false, derived, &q, &ctl()).unwrap().map(f64::to_bits), p.map(f64::to_bits));
    }
    let p = [0.5 + 2f64.powi(-40), -0., 0.75, f64::from_bits(1f64.to_bits() - 1)];
    let neutral = held(&q, -0., 0.);
    assert_eq!(neutral.value.evaluate(p, true, false, &q, &ctl()).unwrap().map(f64::to_bits), p.map(f64::to_bits));
    let gain = held(&q, 1., 0.);
    let p = [f64::MAX, 0.5, 0.25, 1.];
    assert!(gain.value.evaluate(p, true, true, &q, &ctl()).is_err());
    for p in [[f64::NAN, 0., 0., 1.], [0., 0., 0., 1.01], [0., 0., 0., f64::INFINITY]] {
        assert!(h.value.evaluate(p, false, true, &q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN real same-ledger prospective owner/scratch fees and typed Work/cancel/quota refunds; prepared owner retained through evaluation, no local CPU allowance"]
fn actual_preparation_and_runtime_custody() {
    let q = q();
    let base = q.usage().unwrap();
    let p = Parameters::from_words(0.5f32.to_bits(), 0.0625f32.to_bits()).unwrap();
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let (value, fee) = PreparedExposure::prepare(p, 128, &Interrupt::NONE, &mut b).unwrap();
    let h = Held { value, _fee: fee };
    assert!(q.usage().unwrap().resident > base.resident);
    let work = b.used;
    assert!(work > 0);
    let baseline = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(h.value.evaluate([0.5; 4], true, false, &q, &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let cancel = || true;
    let i = Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &i, work: &|_| Ok(()) };
    assert!(matches!(h.value.evaluate([0.5; 4], true, false, &q, &c), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    let remaining = (1 << 20) - baseline.resident;
    let full = q.reserve(QuotaKind::Resident, remaining).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(h.value.evaluate([0.5; 4], true, false, &q, &ctl()), Err(E::Quota(_))));
    assert_eq!(q.usage().unwrap(), before);
    drop(full);
    drop(h);
    assert_eq!(q.usage().unwrap(), base);
    let mut denied = Budget { q: q.clone(), used: 0, limit: work - 1 };
    assert!(PreparedExposure::prepare(p, 128, &Interrupt::NONE, &mut denied).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    assert!(PreparedExposure::prepare(p, 128, &i, &mut b).is_err());
    assert_eq!(q.usage().unwrap(), base);
}
