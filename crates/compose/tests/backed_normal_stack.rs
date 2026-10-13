use photocraft_cms::Builtin;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_write_sample_f64};
use photocraft_compose::{
    backed_normal::{BackedNormalError, NormalMask},
    backed_normal_stack::{NormalStackLayer, NormalStackLimits, PreparedBackedNormalStack},
};
use photocraft_doc::{BackedLayer, SampleEncoding, import_compositing::GimpSpace};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
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
#[test]
fn normal_stack_order_mask_offset_and_cow_history_match_independent_equation() {
    let q = quota();
    let rect = Rect::new(-2, 0, 3, 1);
    let (red, rs) = layer(&q, false, &[1.0, 0.0, 0.0, 0.5], Rect::new(-1, 0, 2, 1));
    let (green, gs) = layer(&q, false, &[0.0, 1.0, 0.0, 0.25], Rect::new(0, 0, 3, 1));
    let (mask, ms) = layer(&q, true, &[0.5], Rect::new(-1, 0, 1, 1));
    let old = mask.clone();
    let changed = mask.try_edited_native(Rect::new(0, 0, 1, 1), &0.0_f64.to_ne_bytes(), &control()).unwrap();
    let layers =
        [NormalStackLayer { mask: Some(NormalMask { layer: &changed, enabled: true, outside_coverage: 1.0 }), ..input(&red, 0.5) }, input(&green, 0.8)];
    let stack = PreparedBackedNormalStack::new(
        &layers,
        &q,
        Builtin::Srgb.profile(),
        SampleEncoding::NonlinearProfile,
        GimpSpace::ProfileRgb,
        NormalStackLimits::default(),
        &control(),
    )
    .unwrap();
    let band = stack.render_band(rect, [0.0, 0.0, 1.0, 1.0], &control()).unwrap();
    let expected = [[0.0, 0.0, 1.0, 1.0], [0.125, 0.0, 0.875, 1.0], [0.0, 0.2, 0.8, 1.0], [0.2, 0.2, 0.6, 1.0], [0.0, 0.2, 0.8, 1.0]];
    for (got, want) in band.pixels().iter().zip(expected) {
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 1.0e-14, "{got:?} != {want:?}");
        }
    }
    let mut raw = [0; 8];
    old.checked_view().read_logical_pixel_native_into(0, 0, &mut raw, &control()).unwrap();
    assert_eq!(raw, 0.5_f64.to_ne_bytes());
    changed.checked_view().read_logical_pixel_native_into(0, 0, &mut raw, &control()).unwrap();
    assert_eq!(raw, 0.0_f64.to_ne_bytes());
    assert_eq!(band.rect(), rect);
    drop((band, stack));
    drop((red, green, mask, old, changed, rs, gs, ms));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn failed_later_layer_returns_no_partial_band_and_keeps_source_owners() {
    let q = quota();
    let rect = Rect::new(0, 0, 2, 1);
    let (first, fs) = layer(&q, false, &[1.0, 0.0, 0.0, 0.5], rect);
    let (last, ls) = layer(&q, false, &[0.0, 1.0, 0.0, 0.5], rect);
    ls.fail.store(true, Ordering::Relaxed);
    let layers = [input(&first, 1.0), input(&last, 1.0)];
    let stack = PreparedBackedNormalStack::new(
        &layers,
        &q,
        Builtin::Srgb.profile(),
        SampleEncoding::NonlinearProfile,
        GimpSpace::ProfileRgb,
        NormalStackLimits::default(),
        &control(),
    )
    .unwrap();
    assert!(matches!(stack.render_band(rect, [0.0; 4], &control()), Err(BackedNormalError::View(_))));
    assert!(fs.reads.load(Ordering::Relaxed) > 0);
    assert!(ls.reads.load(Ordering::Relaxed) > 0);
    let mut bytes = [0; 32];
    first.checked_view().read_logical_pixel_native_into(0, 0, &mut bytes, &control()).unwrap();
    assert_eq!(&bytes[24..], &0.5_f64.to_ne_bytes());
    drop(stack);
    drop((first, last, fs, ls));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn unused_hidden_inputs_and_empty_stack_return_checked_backdrop_without_reads() {
    let q = quota();
    let (hidden, hs) = layer(&quota(), false, &[0.5, 0.5, 0.5, 1.0], Rect::new(0, 0, 1, 1));
    hs.fail.store(true, Ordering::Relaxed);
    let layers = [NormalStackLayer { layer: &hidden, visible: false, opacity: f64::NAN, mask: None }];
    for inputs in [&layers[..], &[][..]] {
        let stack = PreparedBackedNormalStack::new(
            inputs,
            &q,
            Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            GimpSpace::ProfileRgb,
            NormalStackLimits::default(),
            &control(),
        )
        .unwrap();
        let band = stack.render_band(Rect::new(-1, -1, 1, 1), [0.2, 0.3, 0.4, 0.5], &control()).unwrap();
        assert_eq!(band.pixels(), &[[0.2, 0.3, 0.4, 0.5]; 4]);
        assert!(stack.render_band(Rect::new(0, 0, 1, 1), [f64::NAN; 4], &control()).is_err());
        assert!(stack.render_band(Rect::new(0, 0, 0, 1), [0.0; 4], &control()).is_err());
        let exhausted = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(stack.render_band(Rect::new(0, 0, 1, 1), [0.0; 4], &exhausted), Err(BackedNormalError::Backing(BackingError::Work))));
        drop((band, stack));
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    assert_eq!(hs.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn metadata_limits_ledger_and_nonfinite_coverage_fail_before_sample_reads() {
    let q = quota();
    let rect = Rect::new(0, 0, 1, 1);
    let (source, store) = layer(&q, false, &[0.25, 0.5, 0.75, 1.0], rect);
    let (mask, mask_store) = layer(&q, true, &[0.5], rect);
    let before = q.usage().unwrap();
    for opacity in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert!(
            PreparedBackedNormalStack::new(
                &[input(&source, opacity)],
                &q,
                Builtin::Srgb.profile(),
                SampleEncoding::NonlinearProfile,
                GimpSpace::ProfileRgb,
                NormalStackLimits::default(),
                &control(),
            )
            .is_err()
        );
        assert_eq!(q.usage().unwrap(), before);
    }
    for outside in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let inputs = [NormalStackLayer { mask: Some(NormalMask { layer: &mask, enabled: true, outside_coverage: outside }), ..input(&source, 1.0) }];
        assert!(
            PreparedBackedNormalStack::new(
                &inputs,
                &q,
                Builtin::Srgb.profile(),
                SampleEncoding::NonlinearProfile,
                GimpSpace::ProfileRgb,
                NormalStackLimits::default(),
                &control()
            )
            .is_err()
        );
        assert_eq!(q.usage().unwrap(), before);
    }
    let inputs = [input(&source, 1.0)];
    let other = quota();
    assert!(matches!(
        PreparedBackedNormalStack::new(
            &inputs,
            &other,
            Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            GimpSpace::ProfileRgb,
            NormalStackLimits::default(),
            &control()
        ),
        Err(BackedNormalError::Invalid("stack persistent ledger mismatch"))
    ));
    let limits = NormalStackLimits { max_layers: 0, ..NormalStackLimits::default() };
    assert!(
        PreparedBackedNormalStack::new(&inputs, &q, Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb, limits, &control())
            .is_err()
    );
    let exhausted = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(
        PreparedBackedNormalStack::new(
            &inputs,
            &q,
            Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            GimpSpace::ProfileRgb,
            NormalStackLimits::default(),
            &exhausted
        ),
        Err(BackedNormalError::Backing(BackingError::Work))
    ));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    assert_eq!(mask_store.reads.load(Ordering::Relaxed), 0);
    drop((source, mask, store, mask_store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn zero_opacity_and_disabled_invalid_mask_do_not_read_faulting_stores() {
    let q = quota();
    let rect = Rect::new(0, 0, 2, 1);
    let (source, store) = layer(&q, false, &[0.25, 0.5, 0.75, 1.0], rect);
    let (mask, mask_store) = layer(&quota(), false, &[0.5, 0.5, 0.5, 1.0], rect);
    store.fail.store(true, Ordering::Relaxed);
    mask_store.fail.store(true, Ordering::Relaxed);
    let inputs = [NormalStackLayer { mask: Some(NormalMask { layer: &mask, enabled: false, outside_coverage: f64::NAN }), ..input(&source, 0.0) }];
    // Input list, caller Profile and construction callback do not escape.
    let stack = {
        let current_profile = Builtin::Srgb.profile().clone();
        let work = |_| Ok(());
        let borrowed = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        PreparedBackedNormalStack::new(
            &inputs,
            &q,
            &current_profile,
            SampleEncoding::NonlinearProfile,
            GimpSpace::ProfileRgb,
            NormalStackLimits::default(),
            &borrowed,
        )
        .unwrap()
    };
    let backdrop = [0.2, 0.4, 0.6, 0.8];
    let output = stack.render_band(rect, backdrop, &control()).unwrap();
    assert_eq!(output.pixels(), &[backdrop; 2]);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    assert_eq!(mask_store.reads.load(Ordering::Relaxed), 0);
    drop((output, stack));
    drop((source, mask, store, mask_store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn empty_gray_backdrop_and_zero_index_budget_are_explicit() {
    let q = quota();
    let stack = PreparedBackedNormalStack::new(
        &[],
        &q,
        Builtin::SGray.profile(),
        SampleEncoding::NonlinearProfile,
        GimpSpace::ProfileRgb,
        NormalStackLimits::default(),
        &control(),
    )
    .unwrap();
    let rect = Rect::new(-1, -1, 1, 1);
    assert!(matches!(stack.render_band(rect, [0.2, 0.3, 0.4, 0.5], &control()), Err(BackedNormalError::Invalid(_))));
    let output = stack.render_band(rect, [0.2, 0.2, 0.2, 0.5], &control()).unwrap();
    assert_eq!(output.pixels(), &[[0.2, 0.2, 0.2, 0.5]; 4]);
    assert!(stack.render_band(Rect::new(0, 0, 257, 257), [0.0; 4], &control()).is_err());
    drop((output, stack));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let zero_index = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 0, index: 0 });
    assert!(matches!(
        PreparedBackedNormalStack::new(
            &[],
            &zero_index,
            Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            GimpSpace::ProfileRgb,
            NormalStackLimits::default(),
            &control()
        ),
        Err(BackedNormalError::Quota(_))
    ));
    assert_eq!(zero_index.usage().unwrap(), QuotaUsage::default());
}

#[test]
#[ignore = "scoped generated 24MP release measurement; no filesystem or RSS qualification"]
fn generated_24mp_masked_two_layer_stack_checks_every_output_and_refunds() {
    let q = quota();
    let bounds = Rect::new(0, 0, 6000, 4000);
    let (bottom, bs) = layer(&q, false, &[0.2, 0.4, 0.6, 0.5], bounds);
    let (top, ts) = layer(&q, false, &[0.7, 0.5, 0.3, 0.25], bounds);
    let (mask, ms) = layer(&q, true, &[0.5], bounds);
    let inputs = [NormalStackLayer { mask: Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 }), ..input(&bottom, 0.5) }, input(&top, 0.8)];
    let stack = PreparedBackedNormalStack::new(
        &inputs,
        &q,
        Builtin::Srgb.profile(),
        SampleEncoding::NonlinearProfile,
        GimpSpace::ProfileRgb,
        NormalStackLimits::default(),
        &control(),
    )
    .unwrap();
    let start = std::time::Instant::now();
    let mut checked = 0_u64;
    let mut sampled_retained = 0;
    for y in (0..4000).step_by(256) {
        for x in (0..6000).step_by(256) {
            let rect = Rect::new(x, y, (x + 256).min(6000), (y + 256).min(4000));
            let band = stack.render_band(rect, [0.0, 0.0, 0.0, 1.0], &control()).unwrap();
            for got in band.pixels() {
                for (actual, expected) in got.iter().zip([0.16, 0.14, 0.12, 1.0]) {
                    assert!((actual - expected).abs() < 1.0e-14, "{got:?}");
                }
                checked += 1;
            }
            drop(band);
            sampled_retained = sampled_retained.max(q.usage().unwrap().resident);
        }
    }
    assert_eq!(checked, 24_000_000);
    eprintln!(
        "generated24MP F64 RGB+F64 Gray mask/two-layer Normal stack: {:?}; sampled retained after bands {sampled_retained}B (not peak/RSS)",
        start.elapsed()
    );
    drop(stack);
    drop((bottom, top, mask, bs, ts, ms));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
