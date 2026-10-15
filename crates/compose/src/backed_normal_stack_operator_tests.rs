//! UNRUN stack integration drafts. Custom input-store allocations are external
//! harness storage; actual backing/cache/output/metadata fees use the real quota.
use super::*;
use photocraft_cms::Builtin;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_write_sample_f64};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 0, index: 1 << 20 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: AtomicU64,
    fail: AtomicBool,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(output.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        for (i, b) in output.iter_mut().enumerate() {
            *b = self.descriptor.default_bytes()[((offset + i as u64) % self.descriptor.format().bytes_per_pixel() as u64) as usize];
        }
        Ok(())
    }
}
fn layer(q: &BackingQuota, gray: bool, values: &[f64], rect: Rect) -> (BackedLayer, Arc<Store>) {
    let format = PixelFormat::new(if gray { ColorMode::Grayscale } else { ColorMode::Rgb }, SampleType::F64, !gray);
    let mut bytes = vec![0; format.bytes_per_pixel()];
    for (i, v) in values.iter().enumerate() {
        try_write_sample_f64(&mut bytes, SampleType::F64, i, *v).unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, rect, &bytes).unwrap();
    let store = Arc::new(Store { descriptor, reads: AtomicU64::new(0), fail: AtomicBool::new(false) });
    let raster = BackedRaster::new(store.clone(), q.clone(), 1, &control()).unwrap();
    (BackedLayer::new(raster, descriptor, &control()).unwrap(), store)
}
fn input(layer: &BackedLayer, opacity: f64) -> NormalStackLayer<'_> {
    NormalStackLayer { layer, visible: true, opacity, mask: None }
}

fn op<'a>(layer: &'a BackedLayer, operator: GimpOperator, opacity: f64) -> NormalStackOperatorLayer<'a> {
    NormalStackOperatorLayer { input: ResidentNormalStackLayer { input: input(layer, opacity), mask: None }, operator }
}
fn prepared<'a>(layers: &[NormalStackOperatorLayer<'a>], q: &BackingQuota) -> PreparedBackedNormalStack<'a> {
    PreparedBackedNormalStack::new_with_operators(
        layers,
        q,
        Builtin::Srgb.profile(),
        SampleEncoding::NonlinearProfile,
        GimpSpace::ProfileRgb,
        Default::default(),
        &control(),
    )
    .unwrap()
}
#[test]
fn four_operators_follow_independent_alpha_and_color_equations_and_old_union_matches() {
    let q = quota();
    let rect = Rect::new(0, 0, 2, 1);
    let (red, _) = layer(&q, false, &[1., 0., 0., 0.5], rect);
    let background = [0., 0., 1., 0.75];
    for (operator, expected) in [
        (GimpOperator::Union, [4. / 13., 0., 9. / 13., 13. / 16.]),
        (GimpOperator::ClipToBackdrop, [0.25, 0., 0.75, 0.75]),
        (GimpOperator::ClipToLayer, [1., 0., 0., 0.25]),
        (GimpOperator::Intersection, [1., 0., 0., 0.1875]),
    ] {
        let stack = prepared(&[op(&red, operator, 0.5)], &q);
        let band = stack.render_band(rect, background, &control()).unwrap();
        for pixel in band.pixels() {
            for (got, want) in pixel.iter().zip(expected) {
                assert!((got - want).abs() < 1e-14);
            }
        }
        if operator == GimpOperator::Union {
            let old = PreparedBackedNormalStack::new(
                &[input(&red, 0.5)],
                &q,
                Builtin::Srgb.profile(),
                SampleEncoding::NonlinearProfile,
                GimpSpace::ProfileRgb,
                Default::default(),
                &control(),
            )
            .unwrap();
            assert_eq!(band.pixels(), old.render_band(rect, background, &control()).unwrap().pixels());
        }
    }
}
#[test]
fn zero_opacity_zero_mask_and_outside_source_clear_only_clipping_layer_operators() {
    let q = quota();
    let rect = Rect::new(-1, 0, 2, 1);
    let (red, _) = layer(&q, false, &[1., 0., 0., 0.5], Rect::new(0, 0, 1, 1));
    let (mask, _) = layer(&q, true, &[0.], rect);
    let background = [0., 0., 1., 0.75];
    for operator in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
        for use_mask in [false, true] {
            let mut entry = op(&red, operator, if use_mask { 1. } else { 0. });
            if use_mask {
                entry.input.input.mask = Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0. });
            }
            let band = prepared(&[entry], &q).render_band(rect, background, &control()).unwrap();
            let expected = if matches!(operator, GimpOperator::ClipToLayer | GimpOperator::Intersection) { [0.; 4] } else { background };
            assert_eq!(band.pixels(), &[expected; 3]);
        }
    }
    let mut hidden = op(&red, GimpOperator::Intersection, f64::NAN);
    hidden.input.input.visible = false;
    assert_eq!(prepared(&[hidden], &q).render_band(rect, background, &control()).unwrap().pixels(), &[background; 3]);
}
#[test]
fn operator_order_preserves_prior_layer_then_applies_zero_coverage_clear() {
    let q = quota();
    let rect = Rect::new(0, 0, 1, 1);
    let (red, _) = layer(&q, false, &[1., 0., 0., 0.5], rect);
    let (green, _) = layer(&q, false, &[0., 1., 0., 0.5], rect);
    let union = op(&red, GimpOperator::Union, 1.);
    let clear = op(&green, GimpOperator::Intersection, 0.);
    assert_eq!(prepared(&[union, clear], &q).render_band(rect, [0.; 4], &control()).unwrap().pixels(), &[[0.; 4]]);
    assert_eq!(prepared(&[clear, union], &q).render_band(rect, [0.; 4], &control()).unwrap().pixels(), &[[1., 0., 0., 0.5]]);
}
#[test]
fn same_ledger_metadata_denial_and_late_cancel_leave_no_partial_output() {
    let q = quota();
    let rect = Rect::new(0, 0, 2, 1);
    let (red, rs) = layer(&q, false, &[1., 0., 0., 0.5], rect);
    let (green, gs) = layer(&q, false, &[0., 1., 0., 0.5], rect);
    let entries = [op(&red, GimpOperator::Union, 1.), op(&green, GimpOperator::ClipToLayer, 1.)];
    let before = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(
        PreparedBackedNormalStack::new_with_operators(
            &entries,
            &q,
            Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            GimpSpace::ProfileRgb,
            Default::default(),
            &denied
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(rs.reads.load(Ordering::Relaxed), 0);
    let foreign = quota();
    assert!(
        PreparedBackedNormalStack::new_with_operators(
            &entries,
            &foreign,
            Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            GimpSpace::ProfileRgb,
            Default::default(),
            &control()
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), before);
    let stack = prepared(&entries, &q);
    // Warm actual source caches first; private failed output must not affect them.
    drop(stack.render_band(rect, [0.; 4], &control()).unwrap());
    let stable = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let work = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    drop(stack.render_band(rect, [0.; 4], &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap());
    let total = used.load(Ordering::Relaxed);
    assert!(total > 1);
    let used = AtomicU64::new(0);
    let work = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total / 2 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(matches!(
        stack.render_band(rect, [0.; 4], &ReadControl { interrupt: &Interrupt::NONE, work: &work }),
        Err(BackedNormalError::Backing(BackingError::Cancelled)) | Err(BackedNormalError::View(_))
    ));
    assert_eq!(q.usage().unwrap(), stable);
    assert!(gs.reads.load(Ordering::Relaxed) > 0);
    drop(stack);
    drop((red, green, rs, gs));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
