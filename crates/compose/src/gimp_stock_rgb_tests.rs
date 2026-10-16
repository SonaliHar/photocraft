//! Ignored UNRUN exact declared stock RGB arithmetic and paid numeric contracts, no GIMP/display acceptance.
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
    value: PreparedStockRgb,
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
    let (value, fee) = PreparedStockRgb::prepare(p, 128, &Interrupt::NONE, &mut b).unwrap();
    Held { value, _fee: fee }
}
fn temp(a: f32, b: f32) -> Parameters {
    Parameters::ColorTemperature(TemperatureParameters::from_words(a.to_bits(), b.to_bits()).unwrap())
}
fn sepia(scale: f32, srgb: bool) -> Parameters {
    Parameters::Sepia(SepiaParameters::from_words(scale.to_bits(), srgb).unwrap())
}
fn alien(keep: [bool; 3]) -> Parameters {
    Parameters::AlienRgb(AlienRgbParameters::from_words([1f32, 2.5, 0.].map(f32::to_bits), [0f32, 91., 360.].map(f32::to_bits), keep).unwrap())
}
fn arg(n: &str, v: f32) -> FilterArgument {
    FilterArgument { name: n.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
fn bool_arg(n: &str, v: bool) -> FilterArgument {
    FilterArgument { name: n.into(), value: FilterValue::Bool(v) }
}
fn alien_args() -> Vec<FilterArgument> {
    vec![
        FilterArgument { name: "color-model".into(), value: FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 0 } },
        arg("cpn-1-frequency", 1.),
        arg("cpn-2-frequency", 2.5),
        arg("cpn-3-frequency", 0.),
        arg("cpn-1-phaseshift", 0.),
        arg("cpn-2-phaseshift", 91.),
        arg("cpn-3-phaseshift", 360.),
        bool_arg("cpn-1-keep", false),
        bool_arg("cpn-2-keep", true),
        bool_arg("cpn-3-keep", false),
    ]
}
// Public transfer curve + old public operation for coefficient custody only.
// These reference allocations/constructions are external harness, not quota proof.
pub(super) fn reference(p: Parameters, input: [f64; 4]) -> [f64; 4] {
    let curve = photocraft_cms::srgb_trc();
    let mut working = input;
    if p.working() == GimpSpace::LinearRgb {
        for x in &mut working[..3] {
            *x = curve.eval64(*x);
        }
    }
    let mut out = working;
    match p {
        Parameters::ColorTemperature(_) => {
            let mut gains = [0.; 4];
            p.operation().prepare().unwrap().try_eval_pixel(&[1.; 4], 3, &mut gains).unwrap();
            for c in 0..3 {
                out[c] = f64::from((working[c] as f32) * (gains[c] as f32));
            }
        }
        Parameters::Sepia(p) => {
            let rows = [[0x3ec9374c, 0x3f44dd2f, 0x3e418937], [0x3eb2b021, 0x3f2f9db2, 0x3e2c0831], [0x3e8b4396, 0x3f08b439, 0x3e0624dd]];
            let rgb = [working[0] as f32, working[1] as f32, working[2] as f32];
            for (i, row) in rows.into_iter().enumerate() {
                let transformed = rgb.into_iter().zip(row).map(|(v, w)| v * f32::from_bits(w)).sum::<f32>();
                out[i] = f64::from(rgb[i] * (1. - p.scale() as f32) + transformed * p.scale() as f32);
            }
        }
        Parameters::AlienRgb(p) => {
            let (frequency, phase) = p.values();
            for c in 0..3 {
                if !p.keep()[c] {
                    let angle = (2. * f64::from(working[c] as f32) - 1.) * std::f64::consts::PI * frequency[c] + phase[c].to_radians();
                    out[c] = f64::from(((1. + angle.sin()) * 0.5) as f32);
                }
            }
        }
    }
    if p.working() == GimpSpace::LinearRgb {
        for x in &mut out[..3] {
            *x = curve.eval_inverse64(*x);
        }
    }
    out
}
#[test]
#[ignore = "UNRUN cheap typed schemas/raworder/F32BoolEnum guards; no hidden operative interpolation or HSL alias"]
fn exact_cheap_typed_words_and_models() {
    let args = [arg("intended-temperature", 4000.), arg("original-temperature", 6500.)];
    let before = args.clone();
    let p = color_temperature_parameters(&args).unwrap();
    assert_eq!(p.values(), [6500., 4000.]);
    assert_eq!(args, before);
    assert!(TemperatureParameters::from_words(1000f32.to_bits(), 12000f32.to_bits()).is_ok());
    for v in [999f32, 12001., f32::NAN, f32::INFINITY] {
        assert!(TemperatureParameters::from_words(v.to_bits(), 6500f32.to_bits()).is_err());
    }
    let args = [bool_arg("srgb", false), arg("scale", -0.)];
    let p = sepia_parameters(&args).unwrap();
    assert_eq!(p.raw_scale_word(), (-0f32).to_bits());
    assert!(!p.srgb());
    assert!(sepia_parameters(&[arg("scale", 0.5), arg("srgb", 1.)]).is_err());
    let args = alien_args();
    let before = args.clone();
    let p = alien_map_rgb_parameters(&args).unwrap();
    assert_eq!(p.keep(), [false, true, false]);
    assert_eq!(args, before);
    for fault in 0..6 {
        let mut args = alien_args();
        match fault {
            0 => args[0].value = FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 1 },
            1 => args[0].value = FilterValue::Enum { domain: "foreign".into(), value: 0 },
            2 => args[1].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits())),
            3 => args[7] = arg("cpn-1-keep", 1.),
            4 => args[2].name = args[1].name.clone(),
            _ => {
                args.pop();
            }
        }
        assert!(alien_map_rgb_parameters(&args).is_err());
    }
    assert!(AlienRgbParameters::from_words([21f32.to_bits(); 3], [0; 3], [true; 3]).is_err());
    assert!(AlienRgbParameters::from_words([0; 3], [361f32.to_bits(); 3], [true; 3]).is_err());
    assert_ne!(Kind::ColorTemperature.epoch(), Kind::Sepia.epoch());
    assert_ne!(Kind::Sepia.epoch(), Kind::AlienRgb.epoch());
}
#[test]
#[ignore = "UNRUN publiccurve/oldgain Float arithmetic reference, transformed strictfirst vs derived, no equalKelvin/scale0/allkeep shortcut, widealpha and atomictyped finite refusal"]
fn exact_schedules_and_identity_settings() {
    let q = q();
    for p in [
        temp(6500., 4000.),
        temp(6500., 6500.),
        sepia(0., true),
        sepia(0., false),
        sepia(0.625, true),
        sepia(0.625, false),
        alien([false, true, false]),
        alien([true; 3]),
    ] {
        let h = held(&q, p);
        for input in [[0.125, 0.5 + 2f64.powi(-45), 0.875, -0.], [-0.125, 1.25, 0.75, f64::from_bits(1f64.to_bits() - 1)]] {
            let derived = input[..3].iter().any(|v| !(0.0..=1.0).contains(v));
            let out = h.value.evaluate(input, true, derived, &q, &ctl()).unwrap();
            assert_eq!(out.map(f64::to_bits), reference(p, input).map(f64::to_bits));
            assert_eq!(out[3].to_bits(), input[3].to_bits());
            assert_eq!(h.value.evaluate(input, false, derived, &q, &ctl()).unwrap().map(f64::to_bits), input.map(f64::to_bits));
            if derived {
                assert!(h.value.evaluate(input, true, false, &q, &ctl()).is_err());
            }
        }
    }
    let p = alien([true; 3]);
    let h = held(&q, p);
    let input = [f64::MAX, -f64::MAX, -0., -0.];
    assert_eq!(h.value.evaluate(input, true, true, &q, &ctl()).unwrap().map(f64::to_bits), input.map(f64::to_bits));
    for p in [temp(6500., 4000.), sepia(0., true), alien([false; 3])] {
        let h = held(&q, p);
        assert!(h.value.evaluate([f64::MAX, 0., 0., 1.], true, true, &q, &ctl()).is_err());
    }
    // Exactly the released first-lane policy: old strict receives actual working samples.
    let h = held(&q, temp(6500., 4000.));
    let input = [1.0 + f64::EPSILON, 0.5, 0.5, 1.];
    assert!(h.value.evaluate(input, true, false, &q, &ctl()).is_err());
    assert!(h.value.evaluate(input, true, true, &q, &ctl()).is_ok());
}
#[test]
#[ignore = "UNRUN real preparation32768gain debit/refund, activework bykind, disabled256, overlapping512runtime and latework/cancel/quotacleanup"]
fn actual_fees_work_and_cleanup() {
    let q = q();
    let base = q.usage().unwrap();
    for (p, active) in [(temp(6500., 6500.), 1536), (sepia(0., false), 1920), (sepia(0., true), 1152), (alien([true; 3]), 1920)] {
        let h = held(&q, p);
        let retained = q.usage().unwrap();
        for (enabled, expected) in [(true, active), (false, 256)] {
            let used = AtomicU64::new(0);
            let count = |n| {
                used.fetch_add(n, Ordering::Relaxed);
                Ok(())
            };
            h.value.evaluate([0.125, 0.5, 0.875, -0.], enabled, false, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
            assert_eq!(used.load(Ordering::Relaxed), expected);
            assert_eq!(q.usage().unwrap(), retained);
            let used = AtomicU64::new(0);
            let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > expected - 1 { Err(BackingError::Work) } else { Ok(()) };
            assert!(matches!(
                h.value.evaluate([0.125, 0.5, 0.875, 1.], enabled, false, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }),
                Err(E::Backing(BackingError::Work))
            ));
            assert_eq!(q.usage().unwrap(), retained);
        }
        let used = q.usage().unwrap().resident;
        let fill = q.reserve(QuotaKind::Resident, (1 << 20) - used - 256).unwrap();
        assert!(matches!(h.value.evaluate([0.125, 0.5, 0.875, 1.], true, false, &q, &ctl()), Err(E::Quota(_))));
        drop(fill);
        let cancel = || true;
        let i = Interrupt::cancel_only(&cancel);
        assert!(matches!(
            h.value.evaluate([0.125, 0.5, 0.875, 1.], true, false, &q, &ReadControl { interrupt: &i, work: &|_| Ok(()) }),
            Err(E::Backing(BackingError::Cancelled))
        ));
        drop(h);
        assert_eq!(q.usage().unwrap(), base);
    }
    let mut b = Budget { q: q.clone(), used: 0, limit: 32767 };
    assert!(matches!(PreparedStockRgb::prepare(temp(6500., 4000.), 128, &Interrupt::NONE, &mut b), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), base);
}
