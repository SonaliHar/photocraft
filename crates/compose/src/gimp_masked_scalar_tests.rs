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
    value: PreparedMaskedScalar,
    _fee: QuotaLease,
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 })
}
fn held(q: &BackingQuota, p: Parameters) -> Held {
    let mut b = Budget { q: q.clone(), used: 0 };
    let (value, fee) = PreparedMaskedScalar::prepare(p, 128, &Interrupt::NONE, &mut b).unwrap();
    assert!(b.used > 0);
    Held { value, _fee: fee }
}
fn bc(b: f32, c: f32) -> Parameters {
    Parameters::BrightnessContrast(crate::BrightnessContrastParameters::from_words(b.to_bits(), c.to_bits()).unwrap())
}
fn levels(v: [f32; 4]) -> Parameters {
    Parameters::GeglLevels(crate::GeglLevelsParameters::from_words(v.map(f32::to_bits)).unwrap())
}
fn arg(name: &str, value: f32) -> FilterArgument {
    FilterArgument {
        name: name.into(),
        value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(value.to_bits())),
    }
}
fn reference(p: Parameters, before: [f64; 4], raw: f64) -> [f64; 4] {
    let curve = photocraft_cms::srgb_trc();
    let mut after = before;
    for k in 0..3 {
        let v = if p.kind().linear() { curve.eval64(before[k]) } else { before[k] };
        let v = match p {
            Parameters::InvertLinear | Parameters::InvertGamma => 1. - v,
            Parameters::BrightnessContrast(p) => {
                let [b, c] = p.values();
                if b == 0. && c == 1. { v } else { (v - 0.5) * c + 0.5 + b }
            }
            Parameters::GeglLevels(p) => {
                let [il, ih, ol, oh] = p.values();
                if il == ol && ih == oh { v } else { (v - il) / (ih - il) * (oh - ol) + ol }
            }
        };
        after[k] = if p.kind().linear() { curve.eval_inverse64(v) } else { v };
    }
    let b = [curve.eval64(before[0]), curve.eval64(before[1]), curve.eval64(before[2])];
    let a = [curve.eval64(after[0]), curve.eval64(after[1]), curve.eval64(after[2])];
    let t = f64::from(raw as f32);
    let alpha = before[3] + t * (after[3] - before[3]);
    let mut out = [0.; 4];
    for k in 0..3 {
        let v = if alpha == 0. {
            b[k] + t * (a[k] - b[k])
        } else {
            let bp = b[k] * before[3];
            let ap = a[k] * after[3];
            (bp + t * (ap - bp)) / alpha
        };
        out[k] = f64::from(curve.eval_inverse64(v) as f32);
    }
    out[3] = f64::from(alpha as f32);
    out
}
#[test]
#[ignore = "UNRUN distinctmasked0:0exact0/0/2/4storedF32 schemas/order, existing constructorbounds andcrossroletypedaliases refuse"]
fn exact_masked_scalar_schemas() {
    for k in [Kind::InvertLinear, Kind::InvertGamma] {
        assert_eq!(parameters(k, &[]).unwrap().kind(), k);
        assert!(parameters(k, &[arg("unknown", 0.)]).is_err());
        assert_eq!(k.version(), "0:0");
    }
    let a = [arg("contrast", 0.75), arg("brightness", -0.125)];
    let old = a.clone();
    let Parameters::BrightnessContrast(p) = parameters(Kind::BrightnessContrast, &a).unwrap() else { panic!("kind") };
    assert_eq!(p.raw_words(), [(-0.125f32).to_bits(), 0.75f32.to_bits()]);
    assert_eq!(a, old);
    let a = [arg("out-high", 1.), arg("in-high", 0.875), arg("out-low", 0.), arg("in-low", 0.125)];
    let Parameters::GeglLevels(p) = parameters(Kind::GeglLevels, &a).unwrap() else { panic!("kind") };
    assert_eq!(p.values(), [0.125, 0.875, 0., 1.]);
    assert!(parameters(Kind::BrightnessContrast, &[arg("brightness", 0.), arg("brightness", 1.)]).is_err());
    assert!(parameters(Kind::GeglLevels, &a[..3]).is_err());
    let bad = FilterArgument {
        name: "brightness".into(),
        value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float64Bits(0f64.to_bits())),
    };
    assert!(parameters(Kind::BrightnessContrast, &[bad, arg("contrast", 1.)]).is_err());
    assert!(crate::BrightnessContrastParameters::from_words(4f32.to_bits(), 1f32.to_bits()).is_err());
    assert!(crate::GeglLevelsParameters::from_words([1f32, 0., 0., 1.].map(f32::to_bits)).is_err());
    assert_ne!(Kind::InvertLinear.epoch(), crate::INVERT_LINEAR_MIXED_EPOCH);
    assert_ne!(Kind::GeglLevels.epoch(), crate::GEGL_LEVELS_MIXED_EPOCH);
    assert_ne!(Kind::InvertLinear.epoch(), Kind::InvertGamma.epoch());
}
#[test]
#[ignore = "UNRUN fullleaf+unchangedLinearMaskmix atalllocal0/1/fraction/preimage/tiny endpoints andaffineidentity, hiddenRGB/signedalpha/no nativeidentity or GammaLinear alias"]
fn full_leaf_and_mix_endpoints() {
    let q = q();
    let baseline = q.usage().unwrap();
    for p in [
        Parameters::InvertLinear,
        Parameters::InvertGamma,
        bc(-0.125, 0.75),
        bc(-0., 1.),
        levels([0.125, 0.875, 0., 1.]),
        levels([0., 1., 0., 1.]),
        levels([0.125, 0.875, 0.125, 0.875]),
    ] {
        let h = held(&q, p);
        for before in [[0.125, 0.5 + 2f64.powi(-45), 0.875, -0.], [0.125, 0.5, 0.875, 0.375], [0.125, 0.5, 0.875, f64::from_bits(1f64.to_bits() - 1)]] {
            for raw in [0., -0., 0.375, 1., f64::from_bits(0x3ff0000009b25010), f64::from_bits(1)] {
                let _mix = h.value.reserve_runtime(&q, &ctl()).unwrap();
                let out = h.value.evaluate(before, true, raw, &q, &ctl()).unwrap();
                assert_eq!(out.map(f64::to_bits), reference(p, before, raw).map(f64::to_bits));
                assert!(out.iter().all(|v| f64::from(*v as f32).to_bits() == v.to_bits()));
                if before[3].to_bits() == (-0f64).to_bits() && raw.to_bits() == 0 {
                    assert_eq!(out[3].to_bits(), 0f64.to_bits());
                }
                assert_eq!(h.value.evaluate(before, false, raw, &q, &ctl()).unwrap().map(f64::to_bits), before.map(f64::to_bits));
            }
        }
        drop(h);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let before = [0.125, 0.5, 0.875, 1.];
    assert_ne!(reference(Parameters::InvertLinear, before, 1.)[..3], reference(Parameters::InvertGamma, before, 1.)[..3]);
    let identity = bc(0., 1.);
    let before = [0.125, 0.5 + 2f64.powi(-45), 0.875, 1.];
    assert_ne!(reference(identity, before, 0.).map(f64::to_bits), before.map(f64::to_bits));
}
#[test]
#[ignore = "UNRUN fullworst active3584/2816 evenmask0/1/identity, disabled128, actual1024+512concurrent sameq/control/refund andtypedinvaliddomains"]
fn real_work_and_overlapping_fees() {
    let q = q();
    let base = q.usage().unwrap();
    for p in [Parameters::InvertLinear, Parameters::InvertGamma, bc(0., 1.), levels([0., 1., 0., 1.])] {
        let h = held(&q, p);
        let retained = q.usage().unwrap();
        for enabled in [true, false] {
            for raw in [0., 0.375, 1.] {
                let mix = h.value.reserve_runtime(&q, &ctl()).unwrap();
                let during = q.usage().unwrap();
                assert_eq!(during.resident - retained.resident, 1024);
                let units = AtomicU64::new(0);
                let peak = AtomicU64::new(0);
                let count = |n| {
                    units.fetch_add(n, Ordering::Relaxed);
                    peak.fetch_max(q.usage().unwrap().resident, Ordering::Relaxed);
                    Ok(())
                };
                h.value.evaluate([0.125, 0.5, 0.875, 1.], enabled, raw, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
                assert_eq!(units.load(Ordering::Relaxed), if enabled { if p.kind().linear() { 3584 } else { 2816 } } else { 128 });
                if enabled {
                    assert!(peak.load(Ordering::Relaxed) >= during.resident + 512);
                }
                drop(mix);
                assert_eq!(q.usage().unwrap(), retained);
            }
        }
        let mix = h.value.reserve_runtime(&q, &ctl()).unwrap();
        let during = q.usage().unwrap();
        let fill = q.reserve(QuotaKind::Resident, (1 << 20) - during.resident - 256).unwrap();
        assert!(matches!(h.value.evaluate([0.125, 0.5, 0.875, 1.], true, 0., &q, &ctl()), Err(E::Quota(_))));
        drop(fill);
        let deny = |_| Err(BackingError::Work);
        assert!(matches!(
            h.value.evaluate([0.125, 0.5, 0.875, 1.], true, 1., &q, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }),
            Err(E::Backing(BackingError::Work))
        ));
        let cancelled = || true;
        let i = Interrupt::cancel_only(&cancelled);
        assert!(matches!(
            h.value.evaluate([0.125, 0.5, 0.875, 1.], true, 1., &q, &ReadControl { interrupt: &i, work: &|_| Ok(()) }),
            Err(E::Backing(BackingError::Cancelled))
        ));
        for (raw, before) in
            [(f64::NAN, [0.125, 0.5, 0.875, 1.]), (-0.1, [0.125, 0.5, 0.875, 1.]), (1.001, [0.125, 0.5, 0.875, 1.]), (0.5, [f64::MAX, 0., 0., 1.])]
        {
            assert!(h.value.evaluate(before, true, raw, &q, &ctl()).is_err());
        }
        assert_eq!(q.usage().unwrap(), during);
        drop(mix);
        drop(h);
        assert_eq!(q.usage().unwrap(), base);
    }
}
