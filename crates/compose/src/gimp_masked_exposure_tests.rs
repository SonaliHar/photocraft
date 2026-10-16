//! Deferred declared scalar-mask arithmetic and same-ledger controls; no producer numerical oracle.
use super::*;
use photocraft_raster::backing::quota::QuotaLimits;
use std::sync::atomic::{AtomicU64, Ordering};
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
    value: PreparedMaskedExposure,
    _fee: QuotaLease,
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 })
}
fn held(q: &BackingQuota, stops: f32, black: f32) -> Held {
    let mut b = Budget { q: q.clone(), used: 0 };
    let (value, fee) =
        PreparedMaskedExposure::prepare(crate::gimp_exposure::Parameters::from_words(stops.to_bits(), black.to_bits()).unwrap(), 128, &Interrupt::NONE, &mut b)
            .unwrap();
    assert!(b.used > 0);
    Held { value, _fee: fee }
}
fn reference(before: [f64; 4], stops: f64, black: f64, raw: f64) -> [f64; 4] {
    let curve = photocraft_cms::srgb_trc();
    let b = [curve.eval64(before[0]), curve.eval64(before[1]), curve.eval64(before[2])];
    let after = b.map(|v| curve.eval_inverse64(if black == 0.0 { v * stops.exp2() } else { (v - black) / ((-stops).exp2() - black) }));
    let a = after.map(|v| curve.eval64(v));
    let t = f64::from(raw as f32);
    let alpha = before[3] + t * (before[3] - before[3]);
    let mut out = [0.; 4];
    for k in 0..3 {
        let x = if alpha == 0. {
            b[k] + t * (a[k] - b[k])
        } else {
            let bp = b[k] * before[3];
            let ap = a[k] * before[3];
            (bp + t * (ap - bp)) / alpha
        };
        out[k] = f64::from(curve.eval_inverse64(x) as f32);
    }
    out[3] = f64::from(alpha as f32);
    out
}
#[test]
#[ignore = "UNRUN declared full leaf and Linear difference mix at coverage0/1/interior, identity and signedalpha; independent publicTRC reference, not producer arithmetic acceptance"]
fn present_mask_endpoints_never_bypass_mix() {
    let q = q();
    let baseline = q.usage().unwrap();
    for (stops, black) in [(0.5f32, 0.0625f32), (0., 0.)] {
        let h = held(&q, stops, black);
        for before in [[0.125, 0.5 + 2f64.powi(-45), 0.75, -0.], [0.25, 0.5, 0.75, 0.375], [0.125, 0.25, 0.5, 1.]] {
            for raw in [0., -0., 0.375, 1., f64::from_bits(0x3ff0000009b25010)] {
                let _mix = h.value.reserve_runtime(&q, &ctl()).unwrap();
                let actual = h.value.evaluate(before, true, raw, &q, &ctl()).unwrap();
                assert_eq!(actual.map(f64::to_bits), reference(before, f64::from(stops), f64::from(black), raw).map(f64::to_bits));
                if before[3].to_bits() == (-0f64).to_bits() {
                    assert_eq!(actual[3].to_bits(), 0f64.to_bits());
                }
                assert_eq!(h.value.evaluate(before, false, raw, &q, &ctl()).unwrap().map(f64::to_bits), before.map(f64::to_bits));
            }
        }
        drop(h);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN raw coverage checkedFloatpreimage without normalization, strict parameter boundaries, same control/work/cancel/runtimequota failures and exact fee cleanup"]
fn scalar_domain_and_paid_controls() {
    assert_eq!(coverage(f64::from_bits(0x3ff0000009b25010)).unwrap().to_bits(), 1f64.to_bits());
    assert_eq!(coverage(-0.).unwrap().to_bits(), (-0f64).to_bits());
    for v in [-f64::MIN_POSITIVE, -0.1, 1.0001, f64::MAX, f64::NAN, f64::INFINITY] {
        assert!(coverage(v).is_err());
    }
    assert!(crate::gimp_exposure::Parameters::from_words(0, 0x3dcc_cccd).is_err());
    let q = q();
    let h = held(&q, 0.5, 0.0625);
    let stable = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let deny = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(h.value.reserve_runtime(&q, &c).is_err());
    assert!(h.value.evaluate([0.1, 0.2, 0.3, 0.4], true, 0.5, &q, &c).is_err());
    assert!(used.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap(), stable);
    let cancelled = || true;
    let i = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &i, work: &|_| Ok(()) };
    assert!(h.value.evaluate([0.1, 0.2, 0.3, 0.4], true, 0.5, &q, &c).is_err());
    assert_eq!(q.usage().unwrap(), stable);
    let zero = BackingQuota::new(QuotaLimits { resident: 0, index: 0, scratch: 0 });
    assert!(h.value.reserve_runtime(&zero, &ctl()).is_err());
    assert!(h.value.evaluate([0.1, 0.2, 0.3, 0.4], true, 0.5, &zero, &ctl()).is_err());
}
