//! Ignored UNRUN numeric drafts, no source/root/producer authority.
use super::*;
use photocraft_raster::backing::quota::{QuotaLease, QuotaLimits};
use std::sync::atomic::{AtomicU64, Ordering};
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
    value: PreparedInvert,
    _fee: QuotaLease,
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 1 << 20 })
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn held(q: &BackingQuota, kind: Kind) -> Held {
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let (value, fee) = PreparedInvert::prepare(kind, 128, &Interrupt::NONE, &mut b).unwrap();
    Held { value, _fee: fee }
}
#[test]
#[ignore = "UNRUN distinct kind/name/epoch/version and independent exact public curve-plus-subtract references, wide incomingalpha/signedHDR/no implicit Float; not mixed/source admission"]
fn distinct_interpretations_keep_wide_arithmetic() {
    assert_eq!(Kind::Linear.operation_name(), "gegl:invert-linear");
    assert_eq!(Kind::Gamma.operation_name(), "gegl:invert-gamma");
    assert_ne!(Kind::Linear.epoch(), Kind::Gamma.epoch());
    assert_eq!(Kind::Linear.version(), "0:0");
    assert_eq!(Kind::Gamma.version(), "0:0");
    let q = q();
    let curve = photocraft_cms::curve::srgb_trc();
    let linear = held(&q, Kind::Linear);
    let gamma = held(&q, Kind::Gamma);
    for p in [[0., 0.25, 1., -0.], [0.125, 0.5, 0.75, 0.375], [0.5 + 2f64.powi(-40), 0.25, 0.75, f64::from_bits(1f64.to_bits() - 1)], [-0.125, 1.25, 0.75, 0.5]]
    {
        let derived = p[..3].iter().any(|v| !(0.0..=1.0).contains(v));
        let mut expected_l = p;
        let mut expected_g = p;
        for i in 0..3 {
            expected_l[i] = curve.eval_inverse64(1. - curve.eval64(p[i]));
            expected_g[i] = 1. - p[i];
        }
        let l = linear.value.evaluate(p, true, derived, &q, &ctl()).unwrap();
        let g = gamma.value.evaluate(p, true, derived, &q, &ctl()).unwrap();
        assert_eq!(l.map(f64::to_bits), expected_l.map(f64::to_bits));
        assert_eq!(g.map(f64::to_bits), expected_g.map(f64::to_bits));
        assert_eq!(l[3].to_bits(), p[3].to_bits());
        assert_eq!(g[3].to_bits(), p[3].to_bits());
        if p[0] == 0.125 {
            assert_ne!(l.map(f64::to_bits), g.map(f64::to_bits));
        }
        for h in [&linear, &gamma] {
            assert_eq!(h.value.evaluate(p, false, derived, &q, &ctl()).unwrap().map(f64::to_bits), p.map(f64::to_bits));
            if derived {
                assert!(h.value.evaluate(p, true, false, &q, &ctl()).is_err());
            }
        }
    }
    // Finite wide Gamma output is not artificially restricted to F32. A future
    // enabled stack bridge has its own independent checked narrowing/refusal.
    let p = [f64::MAX, 0., 1., f64::from_bits(1f64.to_bits() - 1)];
    assert_eq!(gamma.value.evaluate(p, true, true, &q, &ctl()).unwrap(), [1. - f64::MAX, 1., 0., p[3]]);
    assert!(linear.value.evaluate(p, true, true, &q, &ctl()).is_err());
    for p in [[f64::NAN, 0., 0., 1.], [0., 0., 0., 1.01], [0., 0., 0., f64::NEG_INFINITY]] {
        for h in [&linear, &gamma] {
            assert!(h.value.evaluate(p, false, true, &q, &ctl()).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN per-kind actual transfer cost and full active processing at alpha0/1/neutralRGB, no neutral/doubleinvert cancellation; disabled remains fully validated and paid"]
fn cost_tracks_real_kind_and_activity() {
    let q = q();
    for (kind, active) in [(Kind::Linear, 1536), (Kind::Gamma, 768)] {
        let h = held(&q, kind);
        for alpha in [-0., 1.] {
            for enabled in [true, false] {
                let used = AtomicU64::new(0);
                let count = |n| {
                    used.fetch_add(n, Ordering::Relaxed);
                    Ok(())
                };
                let c = ReadControl { interrupt: &Interrupt::NONE, work: &count };
                let first = h.value.evaluate([0.5, 0.5, 0.5, alpha], enabled, false, &q, &c).unwrap();
                assert_eq!(used.load(Ordering::Relaxed), if enabled { active } else { 256 });
                h.value.evaluate(first, enabled, true, &q, &c).unwrap();
                assert_eq!(used.load(Ordering::Relaxed), 2 * if enabled { active } else { 256 });
            }
        }
    }
}
#[test]
#[ignore = "UNRUN actual same-ledger constructor/scratch admission, payload-before-fee holder and typed Work/cancel/quota error refund; no reset/localCPU allowance"]
fn paid_preparation_and_runtime_cleanup() {
    for kind in [Kind::Linear, Kind::Gamma] {
        let q = q();
        let base = q.usage().unwrap();
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        let (value, fee) = PreparedInvert::prepare(kind, 128, &Interrupt::NONE, &mut b).unwrap();
        let h = Held { value, _fee: fee };
        let work = b.used;
        assert!(work > 0);
        let baseline = q.usage().unwrap();
        assert!(baseline.resident > base.resident);
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(h.value.evaluate([0.5; 4], true, false, &q, &denied), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), baseline);
        let cancel = || true;
        let i = Interrupt::cancel_only(&cancel);
        let c = ReadControl { interrupt: &i, work: &|_| Ok(()) };
        assert!(matches!(h.value.evaluate([0.5; 4], true, false, &q, &c), Err(E::Backing(BackingError::Cancelled))));
        assert_eq!(q.usage().unwrap(), baseline);
        let full = q.reserve(QuotaKind::Resident, (1 << 20) - baseline.resident).unwrap();
        let before = q.usage().unwrap();
        assert!(matches!(h.value.evaluate([0.5; 4], true, false, &q, &ctl()), Err(E::Quota(_))));
        assert_eq!(q.usage().unwrap(), before);
        drop(full);
        drop(h);
        assert_eq!(q.usage().unwrap(), base);
        let mut b = Budget { q: q.clone(), used: 0, limit: work - 1 };
        assert!(PreparedInvert::prepare(kind, 128, &Interrupt::NONE, &mut b).is_err());
        assert_eq!(q.usage().unwrap(), base);
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(PreparedInvert::prepare(kind, 128, &i, &mut b).is_err());
        assert_eq!(q.usage().unwrap(), base);
    }
}
