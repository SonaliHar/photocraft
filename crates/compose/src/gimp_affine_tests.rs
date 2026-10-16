//! Ignored UNRUN exact declared affine arithmetic and paid numeric contracts, no GIMP/display acceptance.
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
    value: PreparedAffine,
    _fee: QuotaLease,
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 1 << 20 })
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn held(q: &BackingQuota, p: Parameters) -> Held {
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let (value, fee) = PreparedAffine::prepare(p, 128, &Interrupt::NONE, &mut b).unwrap();
    Held { value, _fee: fee }
}
fn bc(b: f32, c: f32) -> Parameters {
    Parameters::BrightnessContrast(BrightnessContrastParameters::from_words(b.to_bits(), c.to_bits()).unwrap())
}
fn levels(v: [f32; 4]) -> Parameters {
    Parameters::GeglLevels(GeglLevelsParameters::from_words(v.map(f32::to_bits)).unwrap())
}
fn arg(n: &str, v: f32) -> FilterArgument {
    FilterArgument { name: n.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
fn reference(p: Parameters, input: [f64; 4]) -> [f64; 4] {
    if p.identity() {
        return input;
    }
    let curve = photocraft_cms::srgb_trc();
    let mut out = input;
    for i in 0..3 {
        let x = curve.eval64(input[i]);
        let y = match p {
            Parameters::BrightnessContrast(p) => {
                let [b, c] = p.values();
                (x - 0.5) * c + 0.5 + b
            }
            Parameters::GeglLevels(p) => {
                let [il, ih, ol, oh] = p.values();
                if il == ol && ih == oh { x } else { (x - il) / (ih - il) * (oh - ol) + ol }
            }
        };
        out[i] = curve.eval_inverse64(y);
    }
    out
}
#[test]
#[ignore = "UNRUN separate kinds/epochs exactF32 names/count/order/words/full oldbounds includingnegativecontrast and reversalLevels, invalidtuples/typedaliases refuse without normalization"]
fn schemas_and_words_are_exact() {
    let a = [arg("contrast", 0.75), arg("brightness", -0.125)];
    let before = a.clone();
    let p = brightness_contrast_parameters(&a).unwrap();
    assert_eq!(p.raw_words(), [(-0.125f32).to_bits(), 0.75f32.to_bits()]);
    assert_eq!(a, before);
    let a = [arg("out-high", 0.9375), arg("in-low", 0.125), arg("out-low", 0.0625), arg("in-high", 0.875)];
    assert_eq!(gegl_levels_parameters(&a).unwrap().values(), [0.125, 0.875, 0.0625, 0.9375]);
    assert_ne!(Kind::BrightnessContrast.epoch(), Kind::GeglLevels.epoch());
    assert_eq!(Kind::GeglLevels.operation_name(), "gegl:levels");
    assert!(BrightnessContrastParameters::from_words((-3f32).to_bits(), (-5f32).to_bits()).is_ok());
    assert!(BrightnessContrastParameters::from_words(3f32.to_bits(), 5f32.to_bits()).is_ok());
    for (b, c) in [(3.01f32, 1.), (0., 5.01), (f32::NAN, 1.), (0., f32::INFINITY)] {
        assert!(BrightnessContrastParameters::from_words(b.to_bits(), c.to_bits()).is_err());
    }
    for v in [[0., 0., 0., 1.], [1., 0., 0., 1.], [f32::NAN, 1., 0., 1.], [0., 1., 0., f32::INFINITY]] {
        assert!(GeglLevelsParameters::from_words(v.map(f32::to_bits)).is_err());
    }
    assert!(GeglLevelsParameters::from_words([0f32, 1., 1., 0.].map(f32::to_bits)).is_ok());
    assert!(brightness_contrast_parameters(&[arg("brightness", 0.), arg("brightness", 1.)]).is_err());
    assert!(gegl_levels_parameters(&a[..3]).is_err());
    let bad = FilterArgument { name: "brightness".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0f64.to_bits())) };
    assert!(brightness_contrast_parameters(&[bad, arg("contrast", 1.)]).is_err());
}
#[test]
#[ignore = "UNRUN existing affinef64 scalar/Lineartransfers/widealpha, strictfirst versusfiniteHDRderived, literalidentityversusnonliteralLevels lowercopy+transfer, noownFloat or neutralcancellation"]
fn wide_scalar_and_identity_distinctions() {
    let q = q();
    for p in [
        bc(-0.125, 0.75),
        bc(0.25, -2.),
        levels([0.125, 0.875, 0.0625, 0.9375]),
        levels([0.125, 0.875, 1., 0.]),
        levels([0.125, 0.875, 0.125, 0.875]),
        bc(-0., 1.),
        levels([0., 1., -0., 1.]),
    ] {
        let h = held(&q, p);
        for input in [[0., 0.25, 1., -0.], [0.125, 0.5 + 2f64.powi(-45), 0.875, f64::from_bits(1f64.to_bits() - 1)], [-0.125, 1.25, 0.75, 0.375]] {
            let derived = input[..3].iter().any(|v| !(0.0..=1.0).contains(v));
            let actual = h.value.evaluate(input, true, derived, &q, &ctl()).unwrap();
            assert_eq!(actual.map(f64::to_bits), reference(p, input).map(f64::to_bits));
            assert_eq!(actual[3].to_bits(), input[3].to_bits());
            assert_eq!(h.value.evaluate(input, false, derived, &q, &ctl()).unwrap().map(f64::to_bits), input.map(f64::to_bits));
            if derived {
                assert!(h.value.evaluate(input, true, false, &q, &ctl()).is_err());
            }
        }
    }
    let h = held(&q, bc(0., 2.));
    assert!(h.value.evaluate([f64::MAX, 0., 0., 1.], true, true, &q, &ctl()).is_err());
    let h = held(&q, levels([0., f32::from_bits(1), 0., f32::MAX]));
    assert!(h.value.evaluate([f64::MAX, 0., 0., 1.], true, true, &q, &ctl()).is_err());
    for input in [[f64::NAN, 0., 0., 1.], [0., 0., 0., -0.1], [0., 0., 0., f64::INFINITY]] {
        assert!(h.value.evaluate(input, false, true, &q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN actualsameq construction/runtime512 and1536active/256disabledliteralidentity, other equalLevels transferwork retained, Work/cancel/quota refund andpayload-before-fee retention"]
fn costs_and_lifetimes_are_real() {
    let q = q();
    let base = q.usage().unwrap();
    for p in [bc(-0.125, 0.75), levels([0.125, 0.875, 0.125, 0.875]), bc(0., 1.), levels([0., 1., 0., 1.])] {
        let h = held(&q, p);
        let retained = q.usage().unwrap();
        assert!(retained.resident > base.resident);
        for enabled in [true, false] {
            let used = AtomicU64::new(0);
            let count = |n| {
                used.fetch_add(n, Ordering::Relaxed);
                Ok(())
            };
            let c = ReadControl { interrupt: &Interrupt::NONE, work: &count };
            h.value.evaluate([0.125, 0.5, 0.875, -0.], enabled, false, &q, &c).unwrap();
            assert_eq!(used.load(Ordering::Relaxed), if enabled && !p.identity() { 1536 } else { 256 });
            assert_eq!(q.usage().unwrap(), retained);
        }
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(h.value.evaluate([0.125, 0.5, 0.875, 1.], true, false, &q, &denied), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), retained);
        let cancel = || true;
        let i = Interrupt::cancel_only(&cancel);
        assert!(matches!(
            h.value.evaluate([0.125, 0.5, 0.875, 1.], true, false, &q, &ReadControl { interrupt: &i, work: &|_| Ok(()) }),
            Err(E::Backing(BackingError::Cancelled))
        ));
        assert_eq!(q.usage().unwrap(), retained);
        let available = (1u64 << 20) - q.usage().unwrap().resident;
        let blocked = q.reserve(QuotaKind::Resident, available - 256).unwrap();
        assert!(matches!(h.value.evaluate([0.125, 0.5, 0.875, 1.], true, false, &q, &ctl()), Err(E::Quota(_))));
        drop(blocked);
        assert_eq!(q.usage().unwrap(), retained);
        drop(h);
        assert_eq!(q.usage().unwrap(), base);
    }
    let mut b = Budget { q: q.clone(), used: 0, limit: 0 };
    assert!(PreparedAffine::prepare(bc(0., 1.), 128, &Interrupt::NONE, &mut b).is_err());
    assert_eq!(q.usage().unwrap(), base);
}
