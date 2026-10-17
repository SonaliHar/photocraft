//! UNRUN shared point-band regressions over genuine paid synthetic native backing.
//! Working contexts are explicit mathematical inputs, not stock source authority.
use super::*;
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    quota: BackingQuota,
    reads: AtomicU64,
    fail: AtomicBool,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn fixture(mode: ColorMode, sample: SampleType, bounds: Rect, invalid: bool) -> (BackedLayer, Arc<Store>) {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 4 << 20, scratch: 0 });
    let format = PixelFormat::new(mode, sample, true);
    let bpp = format.bytes_per_pixel();
    let len = bounds.width() as usize * bounds.height() as usize * bpp;
    assert!(len <= 1 << 20);
    let header = std::mem::size_of::<Store>() + 512;
    let mut fee = q.reserve(QuotaKind::Resident, (len * 2 + header) as u64).unwrap();
    let mut default = vec![0u8; bpp];
    for c in 0..format.channels() {
        try_write_sample_f64(&mut default, sample, c, 0.25).unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).unwrap();
    assert!(bytes.capacity() <= len * 2);
    bytes.resize(len, 0);
    for (i, pixel) in bytes.chunks_exact_mut(bpp).enumerate() {
        for c in 0..mode.color_channels() {
            let v = [0.0, 0.25, 0.5, 0.75, 1.0][(i + c) % 5];
            try_write_sample_f64(pixel, sample, c, if sample == SampleType::F64 && v == 0.5 { v + 2f64.powi(-45) } else { v }).unwrap();
        }
        let alpha = [-0.0, 0.125, 0.5 + 2f64.powi(-45), 0.75, 1.0][i % 5];
        try_write_sample_f64(pixel, sample, mode.color_channels(), alpha).unwrap();
        if invalid && i == 0 {
            try_write_sample_f64(pixel, sample, 0, f64::NAN).unwrap();
        }
    }
    fee.reconcile_capacity((bytes.capacity() + header) as u64).unwrap();
    let store = Arc::new(Store { descriptor, bytes, quota: q.clone(), reads: AtomicU64::new(0), fail: AtomicBool::new(false), _fee: fee });
    let raster = BackedRaster::new(store.clone(), q, 2, &ctl()).unwrap();
    (BackedLayer::new(raster, descriptor, &ctl()).unwrap(), store)
}

fn context(layer: &BackedLayer) -> PointWorkingContext {
    PointWorkingContext { format: layer.format(), encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] }
}
fn prepared<'a>(layer: &'a BackedLayer, operation: PointOperation, enabled: bool) -> PreparedBackedPoint<'a> {
    PreparedBackedPoint::new(layer, layer.descriptor(), context(layer), operation, enabled, Default::default(), &ctl()).unwrap()
}
#[test]
fn multiple_operations_native_depths_partial_negative_roi_and_stride_match_surface_contract() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let bounds = Rect::new(-3, 255, 260, 258);
        let (layer, store) = fixture(ColorMode::Rgb, sample, bounds, false);
        // Both temporary oracle Surfaces and their native buffers are paid before
        // allocation; this is a numerical/ROI oracle, never imported source evidence.
        let _oracle = layer.quota().reserve(QuotaKind::Resident, 32 << 20).unwrap();
        let mut source = photocraft_raster::Surface::try_with_default_bytes(layer.format(), layer.default_bytes()).unwrap();
        source.try_write_interleaved_with(bounds, &store.bytes, &Interrupt::NONE).unwrap();
        let roi = Rect::new(-2, 255, 259, 258);
        for operation in [
            PointOperation::Invert,
            PointOperation::Exposure { stops: 0.25, black_level: 0. },
            PointOperation::BrightnessContrast { brightness: 0.125, contrast: 1.125 },
            PointOperation::Levels { input_low: 0., input_high: 1., output_low: 0.125, output_high: 0.875 },
            PointOperation::Sepia { scale: 0.5, srgb: true },
            PointOperation::AlienMap { frequency: [1., 2., 0.5], phase: [0., 15., 30.], keep: [false, false, true] },
        ] {
            let mut working = context(&layer);
            if matches!(operation, PointOperation::Exposure { .. } | PointOperation::BrightnessContrast { .. } | PointOperation::Levels { .. }) {
                working.encoding = SampleEncoding::Linear;
            }
            // The leaf never converts source words: these contexts are caller-declared
            // mathematical working samples, not an assertion about this synthetic ICC.
            let p = PreparedBackedPoint::new(&layer, layer.descriptor(), working, operation, true, Default::default(), &ctl()).unwrap();
            assert_eq!(p.full_extent(), bounds);
            let actual = p.render_band(roi, &ctl()).unwrap();
            let expected = photocraft_algo::gimp::point::try_apply(&source, roi, operation, 64 << 20, u64::MAX, &Interrupt::NONE)
                .unwrap()
                .try_read_interleaved(roi)
                .unwrap();
            assert_eq!(actual.bytes(), expected);
            assert_eq!(actual.rect(), roi);
            assert_eq!(actual.bytes().len(), roi.width() as usize * roi.height() as usize * layer.format().bytes_per_pixel());
            let row = Rect::new(roi.x0, roi.y0, roi.x1, roi.y0 + 1);
            let a = p.render_band(row, &ctl()).unwrap();
            let b = p.render_band(Rect::new(roi.x0, roi.y0 + 1, roi.x1, roi.y1), &ctl()).unwrap();
            assert_eq!([a.bytes(), b.bytes()].concat(), actual.bytes());
        }
        assert_eq!(layer.default_bytes(), store.descriptor.default_bytes());
    }
}
#[test]
fn raw_alpha_alpha_only_coupled_and_disabled_paths_preserve_their_distinct_contracts() {
    let bounds = Rect::new(0, 0, 5, 1);
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F64, bounds, false);
    let alpha = 24;
    let color = prepared(&layer, PointOperation::Invert, true).render_band(bounds, &ctl()).unwrap();
    for (a, b) in store.bytes.as_chunks::<32>().0.iter().zip(color.bytes().as_chunks::<32>().0.iter()) {
        assert_eq!(&a[alpha..], &b[alpha..]);
    }
    let threshold = prepared(&layer, PointOperation::ThresholdAlpha { value: 0.5 }, true).render_band(bounds, &ctl()).unwrap();
    for (a, b) in store.bytes.as_chunks::<32>().0.iter().zip(threshold.bytes().as_chunks::<32>().0.iter()) {
        assert_eq!(&a[..alpha], &b[..alpha]);
    }
    let disabled = prepared(&layer, PointOperation::Invert, false).render_band(bounds, &ctl()).unwrap();
    assert_eq!(disabled.bytes(), store.bytes);
    let (float, _) = fixture(ColorMode::Rgb, SampleType::F32, bounds, false);
    let coupled = prepared(&float, PointOperation::ColorOverlay { color: [0.8, 0.2, 0.1, 0.6] }, true).render_band(bounds, &ctl()).unwrap();
    assert_eq!(coupled.bytes().len(), 80);
    let (invalid, invalid_store) = fixture(ColorMode::Rgb, SampleType::F64, bounds, true);
    assert!(prepared(&invalid, PointOperation::Invert, true).render_band(bounds, &ctl()).is_err());
    assert_eq!(prepared(&invalid, PointOperation::Invert, false).render_band(bounds, &ctl()).unwrap().bytes(), invalid_store.bytes);
    assert_eq!(
        prepared(&invalid, PointOperation::Exposure { stops: 0., black_level: 0. }, true).render_band(bounds, &ctl()).unwrap().bytes(),
        invalid_store.bytes
    );
}
#[test]
fn descriptor_context_format_and_lower_roi_limits_refuse_before_provider_reads() {
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F32, Rect::new(0, 0, 3, 2), false);
    let operation = PointOperation::Invert;
    let mut c = context(&layer);
    c.profile_identity = [0; 32];
    assert!(PreparedBackedPoint::new(&layer, layer.descriptor(), c, operation, true, Default::default(), &ctl()).is_err());
    let wrong = NativePlaneDescriptor::new(layer.format(), Rect::new(0, 0, 1, 1), layer.default_bytes()).unwrap();
    assert!(PreparedBackedPoint::new(&layer, wrong, context(&layer), operation, true, Default::default(), &ctl()).is_err());
    let p = PreparedBackedPoint::new(&layer, layer.descriptor(), context(&layer), operation, true, NormalBandLimits { max_rows: 1, max_pixels: 1 }, &ctl())
        .unwrap();
    for roi in [Rect::EMPTY, Rect::new(-1, 0, 0, 1), layer.operative_bounds()] {
        assert!(p.render_band(roi, &ctl()).is_err());
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    let (gray, _) = fixture(ColorMode::Grayscale, SampleType::F64, Rect::new(0, 0, 1, 1), false);
    assert!(prepared(&gray, PointOperation::ValueInvert, true).render_band(gray.operative_bounds(), &ctl()).is_err());
}
#[test]
fn work_quota_and_midwalk_cancel_keep_output_private_and_charges_live_until_drop() {
    let bounds = Rect::new(0, 0, 16, 1);
    let (layer, _) = fixture(ColorMode::Rgb, SampleType::F64, bounds, false);
    let p = prepared(&layer, PointOperation::Invert, true);
    // Warm source cache so output/scratch rollback is independently observable.
    drop(layer.checked_view().pin_tile(TileCoord::new(0, 0), &ctl()).unwrap());
    let q = layer.quota();
    let before = q.usage().unwrap();
    let req = p.requirements(bounds, &ctl()).unwrap();
    assert!(req.prepaid_work_units >= 2 * 16 * PIXEL_WORK);
    let deny = |_| Err(BackingError::Work);
    assert!(p.render_band(bounds, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let zeroes = AtomicU64::new(0);
    let debit = |n| {
        if n == 0 && zeroes.fetch_add(1, Ordering::Relaxed) > 24 { Err(BackingError::Cancelled) } else { Ok(()) }
    };
    assert!(p.render_band(bounds, &ReadControl { interrupt: &Interrupt::NONE, work: &debit }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident - req.output_resident_bytes + 1).unwrap();
    assert!(p.render_band(bounds, &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let band = p.render_band(bounds, &ctl()).unwrap();
    assert!(q.usage().unwrap().resident > before.resident);
    drop(band);
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
fn gray_native_depths_keep_raw_alpha_without_rgb_or_f32_conversion() {
    for sample in SampleType::ALL {
        let bounds = Rect::new(0, 0, 5, 1);
        let (layer, store) = fixture(ColorMode::Grayscale, sample, bounds, false);
        let output = prepared(&layer, PointOperation::Invert, true).render_band(bounds, &ctl()).unwrap();
        let bpp = layer.format().bytes_per_pixel();
        let alpha = sample.bytes();
        for (input, actual) in store.bytes.chunks_exact(bpp).zip(output.bytes().chunks_exact(bpp)) {
            let mut expected = input.to_vec();
            try_write_sample_f64(&mut expected, sample, 0, 1. - try_read_sample_f64(input, sample, 0).unwrap()).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(&actual[alpha..], &input[alpha..]);
        }
    }
}
