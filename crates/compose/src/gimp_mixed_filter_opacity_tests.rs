//! Ignored UNRUN declared-opacity arithmetic/custody contracts, not producer or display acceptance.
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
    value: PreparedMix,
    _fee: QuotaLease,
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 1 << 20 })
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn held(q: &BackingQuota) -> Held {
    let mut b = Budget { q: q.clone(), used: 0 };
    let (value, fee) = PreparedMix::prepare(128, &Interrupt::NONE, &mut b).unwrap();
    Held { value, _fee: fee }
}
fn reference(b: [f64; 4], a: [f64; 4], t: f64) -> [f64; 4] {
    if t == 0. {
        return b.map(|v| f64::from(v as f32));
    }
    let curve = photocraft_cms::srgb_trc();
    let bb = [curve.eval64(b[0]), curve.eval64(b[1]), curve.eval64(b[2])];
    let aa = [curve.eval64(a[0]), curve.eval64(a[1]), curve.eval64(a[2])];
    let alpha = b[3] + t * (a[3] - b[3]);
    let mut r = [0.; 4];
    for c in 0..3 {
        let v = if alpha == 0. {
            bb[c] + t * (aa[c] - bb[c])
        } else {
            let x = bb[c] * b[3];
            let y = aa[c] * a[3];
            (x + t * (y - x)) / alpha
        };
        r[c] = f64::from(curve.eval_inverse64(v) as f32);
    }
    r[3] = f64::from(alpha as f32);
    r
}
#[test]
#[ignore = "UNRUN exact declared difference/premul/alpha0hidden/Float schedule with finiteHDR, arbitraryF64 amounts, changedalpha including unobservedsource0toafterpositive; no GIMP numerical oracle"]
fn partial_reference_and_alpha_zero_hidden_rgb() {
    let q = q();
    let h = held(&q);
    for (b, a) in [
        ([-0.125, 1.25, 0.5, -0.], [0.75, -0.25, 2., 0.]),
        ([0.125, 0.25, 0.75, 0.], [0.75, 0.5, 0.25, 1.]),
        ([0.25, 0.5, 0.75, 0.125], [0.5, 0.75, 0.25, 1.]),
        ([0.5 + 2f64.powi(-45), 0.25, 0.75, f64::from_bits(1f64.to_bits() - 1)], [1.25, -0.125, 0.25, 1.]),
    ] {
        for t in [0.375, f64::from_bits(0.375f64.to_bits() + 1), f64::MIN_POSITIVE, f64::from_bits(1f64.to_bits() - 1)] {
            let _runtime = h.value.reserve_runtime(&q, &ctl()).unwrap();
            let out = h.value.apply(b, a, t.to_bits(), &ctl()).unwrap();
            assert_eq!(out.map(f64::to_bits), reference(b, a, t).map(f64::to_bits));
        }
    }
    let _runtime = h.value.reserve_runtime(&q, &ctl()).unwrap();
    let p = h.value.apply([0.125, 0.25, 0.5, -0.], [0.75, 0.5, 0.25, -0.], 0.375f64.to_bits(), &ctl()).unwrap();
    assert_eq!(p[3].to_bits(), 0f64.to_bits());
    assert_ne!(p[..3], [0.; 3]);
}
#[test]
#[ignore = "UNRUN signedzero opacity endpoints retain before Floatalpha/lowbit rounding AFTER valid fullafter, reject1helper/NaN/overflow atomically and preserve derivedHDR no clamp"]
fn endpoint_and_invalid_domains() {
    let q = q();
    let h = held(&q);
    let b = [0.5 + 2f64.powi(-45), -0.125, 1.25, -0.];
    let a = [0.25, 0.75, 0.5, 1.];
    let _runtime = h.value.reserve_runtime(&q, &ctl()).unwrap();
    for t in [0f64, -0f64] {
        assert_eq!(h.value.apply(b, a, t.to_bits(), &ctl()).unwrap().map(f64::to_bits), b.map(|v| f64::from(v as f32).to_bits()));
    }
    for bits in [1f64.to_bits(), f64::NAN.to_bits(), f64::INFINITY.to_bits(), (-0.1f64).to_bits(), 1.01f64.to_bits()] {
        assert!(h.value.apply(b, a, bits, &ctl()).is_err());
    }
    for p in [[f64::MAX, 0., 0., 1.], [f64::NAN, 0., 0., 1.], [0., 0., 0., 1.01]] {
        assert!(h.value.apply(p, a, 0.375f64.to_bits(), &ctl()).is_err());
    }
    assert!(h.value.apply(b, [f64::NAN, 0., 0., 1.], 0f64.to_bits(), &ctl()).is_err());
    assert!(h.value.apply([f64::MAX, 0., 0., 1.], a, 0f64.to_bits(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN genuine sameq extra preparation+1024 mix runtime overlap with leaf fee, conservative cumulative work and typed Work/cancel/refund; no borrowed old scratch allowance"]
fn same_ledger_overlap_and_control() {
    let q = q();
    let base = q.usage().unwrap();
    let h = held(&q);
    let retained = q.usage().unwrap();
    assert!(retained.resident > base.resident);
    let leaf = q.reserve(QuotaKind::Resident, 512).unwrap();
    let with_leaf = q.usage().unwrap();
    {
        let _mix = h.value.reserve_runtime(&q, &ctl()).unwrap();
        assert_eq!(q.usage().unwrap().resident, with_leaf.resident + PIXEL_SCRATCH_BYTES);
        for (t, expected) in [(0f64, ZERO_WORK), (0.375, PARTIAL_WORK)] {
            let used = AtomicU64::new(0);
            let count = |n| {
                used.fetch_add(n, Ordering::Relaxed);
                Ok(())
            };
            let c = ReadControl { interrupt: &Interrupt::NONE, work: &count };
            h.value.apply([0.25, 0.5, 0.75, -0.], [0.75, 0.25, 0.5, 1.], t.to_bits(), &c).unwrap();
            assert_eq!(used.load(Ordering::Relaxed), expected);
        }
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(h.value.apply([0.25, 0.5, 0.75, 1.], [0.75, 0.25, 0.5, 1.], 0.375f64.to_bits(), &denied), Err(E::Backing(BackingError::Work))));
        let cancel = || true;
        let i = Interrupt::cancel_only(&cancel);
        let c = ReadControl { interrupt: &i, work: &|_| Ok(()) };
        assert!(matches!(h.value.apply([0.25, 0.5, 0.75, 1.], [0.75, 0.25, 0.5, 1.], 0.375f64.to_bits(), &c), Err(E::Backing(BackingError::Cancelled))));
    }
    assert_eq!(q.usage().unwrap(), with_leaf);
    let available = (1u64 << 20) - q.usage().unwrap().resident;
    let blocker = q.reserve(QuotaKind::Resident, available - 512).unwrap();
    assert!(matches!(h.value.reserve_runtime(&q, &ctl()), Err(E::Quota(_))));
    drop(blocker);
    assert_eq!(q.usage().unwrap(), with_leaf);
    drop(leaf);
    assert_eq!(q.usage().unwrap(), retained);
    drop(h);
    assert_eq!(q.usage().unwrap(), base);
}
