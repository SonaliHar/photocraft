//! Deferred declared arithmetic and ownership contracts; no producer pixel oracle.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
use photocraft_raster::backing::{
    BackingError,
    quota::{QuotaKind, QuotaLimits},
};
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
        self.used = self.used.checked_add(n).ok_or(E::Invalid("testwork"))?;
        Ok(())
    }
}
struct Held {
    value: Prepared,
    _fee: QuotaLease,
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 })
}
fn held(q: &BackingQuota, p: Parameters) -> Held {
    let mut b = Budget { q: q.clone(), used: 0 };
    let (value, fee) = Prepared::prepare(p, 128, &Interrupt::NONE, &mut b).unwrap();
    assert!(b.used > 0);
    Held { value, _fee: fee }
}
fn arg(n: &str, v: f32) -> FilterArgument {
    FilterArgument { name: n.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
fn args(k: Kind) -> Vec<FilterArgument> {
    match k {
        Kind::Exposure => vec![arg("exposure", 0.5), arg("black-level", 0.0625)],
        Kind::BrightnessContrast => vec![arg("contrast", 0.75), arg("brightness", -0.125)],
        Kind::GeglLevels => vec![arg("in-low", 0.125), arg("in-high", 0.875), arg("out-low", 0.0625), arg("out-high", 0.9375)],
        _ => vec![],
    }
}
const KINDS: [Kind; 5] = [Kind::Exposure, Kind::InvertLinear, Kind::InvertGamma, Kind::BrightnessContrast, Kind::GeglLevels];
// Public TRC and old public scalar kernel, followed by independently written
// full masked difference-alpha policy. Reference allocations are external harness.
pub(crate) fn reference(p: Parameters, before: [f64; 4], raw: f64) -> [f64; 4] {
    use photocraft_algo::gimp::point::PointOperation;
    let trc = photocraft_cms::srgb_trc();
    let linear = p.kind() != Kind::InvertGamma;
    let mut working = before;
    if linear {
        for v in &mut working[..3] {
            *v = trc.eval64(*v);
        }
    }
    let op = match p {
        Parameters::Exposure(p) => {
            let [stops, black_level] = p.values();
            PointOperation::Exposure { stops, black_level }
        }
        Parameters::Scalar(p) => match p {
            crate::MaskedScalarParameters::InvertLinear | crate::MaskedScalarParameters::InvertGamma => PointOperation::Invert,
            crate::MaskedScalarParameters::BrightnessContrast(p) => {
                let [brightness, contrast] = p.values();
                PointOperation::BrightnessContrast { brightness, contrast }
            }
            crate::MaskedScalarParameters::GeglLevels(p) => {
                let [input_low, input_high, output_low, output_high] = p.values();
                PointOperation::Levels { input_low, input_high, output_low, output_high }
            }
        },
    };
    let mut after = [0.; 4];
    op.prepare().unwrap().try_eval_pixel(&working, 3, &mut after).unwrap();
    if linear {
        for v in &mut after[..3] {
            *v = trc.eval_inverse64(*v);
        }
    }
    let t = f64::from(raw as f32);
    let alpha = before[3] + t * (after[3] - before[3]);
    let mut out = [0.; 4];
    for k in 0..3 {
        let b = trc.eval64(before[k]);
        let a = trc.eval64(after[k]);
        let v = if alpha == 0. {
            b + t * (a - b)
        } else {
            let bp = b * before[3];
            let ap = a * after[3];
            (bp + t * (ap - bp)) / alpha
        };
        out[k] = f64::from(trc.eval_inverse64(v) as f32);
    }
    out[3] = f64::from(alpha as f32);
    out
}
#[test]
#[ignore = "UNRUN distinct masked-mixed epochs exact five raw schemas and no ordinary Point aliases"]
fn five_distinct_exact_shapes() {
    for kind in KINDS {
        let a = args(kind);
        let before = a.clone();
        let p = parameters(kind, &a).unwrap();
        assert_eq!(p.kind(), kind);
        assert_eq!(a, before);
        assert!(kind.epoch().contains("-masked-mixed-wide-v1"));
        assert!(!crate::gimp_masked_scalar::claims_epoch(kind.epoch()));
        let mut bad = a;
        bad.push(arg("unknown", 0.));
        assert!(parameters(kind, &bad).is_err());
    }
    assert!(parameters(Kind::Exposure, &[arg("exposure", 0.), arg("black-level", f32::from_bits(0x3dcccccd))]).is_err());
}
#[test]
#[ignore = "UNRUN five full masked leaf+mix references at local zero/one/identity and derived HDR; strict old unit APIs unchanged"]
fn strict_derived_full_endpoint_and_identity_math() {
    let q = quota();
    for kind in KINDS {
        let p = parameters(kind, &args(kind)).unwrap();
        let h = held(&q, p);
        let runtime = h.value.reserve_runtime(&q, &ctl()).unwrap();
        for before in [[0.125, 0.5, 0.875, -0.], [0.125, 0.5, 0.875, f64::from_bits(1f64.to_bits() - 1)], [-0.25, 0.5, 1.25, 0.375]] {
            for raw in [0., -0., 0.375, 1., f64::from_bits(0x3ff0000009b25010), f64::from_bits(1)] {
                let derived = before[0] < 0.;
                let actual = h.value.evaluate(before, true, derived, raw, &q, &ctl()).unwrap();
                assert_eq!(actual.map(f64::to_bits), reference(p, before, raw).map(f64::to_bits));
                assert_eq!(h.value.evaluate(before, false, derived, raw, &q, &ctl()).unwrap().map(f64::to_bits), before.map(f64::to_bits));
                if derived {
                    assert!(h.value.evaluate(before, true, false, raw, &q, &ctl()).is_err());
                }
            }
        }
        drop(runtime);
    }
    for p in [
        Parameters::Exposure(crate::gimp_exposure::Parameters::from_words(0, 0).unwrap()),
        Parameters::Scalar(crate::MaskedScalarParameters::BrightnessContrast(crate::BrightnessContrastParameters::from_words(0, 1f32.to_bits()).unwrap())),
        Parameters::Scalar(crate::MaskedScalarParameters::GeglLevels(crate::GeglLevelsParameters::from_words([0, 1f32.to_bits(), 0, 1f32.to_bits()]).unwrap())),
    ] {
        let h = held(&q, p);
        let _runtime = h.value.reserve_runtime(&q, &ctl()).unwrap();
        let input = [0.123456789, 0.5, 0.875, -0.];
        for raw in [0., 1.] {
            assert_eq!(h.value.evaluate(input, true, false, raw, &q, &ctl()).unwrap().map(f64::to_bits), reference(p, input, raw).map(f64::to_bits));
        }
    }
}
#[test]
#[ignore = "UNRUN same-q independent mix/leaf overlap and cumulative typed work/cancel/invalid finite domain cleanup"]
fn runtime_and_controls_are_not_preparation_credit() {
    let q = quota();
    for kind in KINDS {
        let h = held(&q, parameters(kind, &args(kind)).unwrap());
        let base = q.usage().unwrap();
        {
            let _mix = h.value.reserve_runtime(&q, &ctl()).unwrap();
            let counted = AtomicU64::new(0);
            let c = ReadControl {
                interrupt: &Interrupt::NONE,
                work: &|n| {
                    counted.fetch_add(n, Ordering::Relaxed);
                    Ok(())
                },
            };
            h.value.evaluate([0.125, 0.5, 0.875, 0.375], true, false, 0.375, &q, &c).unwrap();
            assert_eq!(counted.load(Ordering::Relaxed), if kind == Kind::InvertGamma { 2816 } else { 3584 });
            let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
            assert!(matches!(h.value.evaluate([0.125, 0.5, 0.875, 0.375], true, false, 0., &q, &deny), Err(E::Backing(BackingError::Work))));
            for p in [[f64::NAN, 0., 0., 1.], [f64::MAX, 0., 0., 1.], [0., 0., 0., 1.25]] {
                assert!(h.value.evaluate(p, true, true, 0.375, &q, &ctl()).is_err());
            }
            let stop = || true;
            let i = Interrupt::cancel_only(&stop);
            assert!(h.value.evaluate([0.125, 0.5, 0.875, 0.375], false, false, 0., &q, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
        }
        assert_eq!(q.usage().unwrap(), base);
        {
            // Preparation's retained peak is not runtime credit: leave room
            // for mix1024 plus only511 leaf bytes on the same actual ledger.
            let filler = q.reserve(QuotaKind::Resident, (1u64 << 20) - base.resident - 1024 - 511).unwrap();
            let mix = h.value.reserve_runtime(&q, &ctl()).unwrap();
            let input = [0.125, 0.5, 0.875, 0.375];
            assert!(matches!(h.value.evaluate(input, true, false, 0., &q, &ctl()), Err(E::Quota(_))));
            assert_eq!(h.value.evaluate(input, false, false, 0., &q, &ctl()).unwrap().map(f64::to_bits), input.map(f64::to_bits));
            drop(mix);
            drop(filler);
        }
        assert_eq!(q.usage().unwrap(), base);
    }
}
