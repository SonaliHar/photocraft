//! UNRUN native Threshold Alpha band/ownership/cancellation drafts.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 4 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    quota: BackingQuota,
    reads: AtomicU64,
    fail: AtomicBool,
    _lease: QuotaLease,
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
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        let begin = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = begin.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(begin..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn fixture(mode: ColorMode, sample: SampleType, bounds: Rect, invalid: bool) -> (BackedLayer, Arc<Store>) {
    let format = PixelFormat::new(mode, sample, true);
    let mut default = vec![0; format.bytes_per_pixel()];
    for c in 0..format.channels() {
        try_write_sample_f64(&mut default, sample, c, 0.25).unwrap();
    }
    let count = bounds.width() as usize * bounds.height() as usize;
    let mut bytes = Vec::new();
    for i in 0..count {
        let mut p = default.clone();
        for c in 0..mode.color_channels() {
            let color = if i % 3 == 0 && sample == SampleType::F64 {
                -0.0
            } else if sample == SampleType::F64 {
                0.5000000000000001
            } else {
                0.375
            };
            try_write_sample_f64(&mut p, sample, c, color).unwrap();
        }
        let alpha = [0.0, 0.5, 0.5 + 2_f64.powi(-52), 0.5 + 2_f64.powi(-24), 1.0][i % 5];
        try_write_sample_f64(&mut p, sample, mode.color_channels(), alpha).unwrap();
        if invalid && i == count / 2 {
            try_write_sample_f64(&mut p, sample, 0, f64::NAN).unwrap();
        }
        bytes.extend_from_slice(&p);
    }
    let q = quota();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let lease = q.reserve(QuotaKind::Resident, (bytes.capacity() + std::mem::size_of::<Store>() + 128) as u64).unwrap();
    let store = Arc::new(Store { descriptor, bytes, quota: q.clone(), reads: AtomicU64::new(0), fail: AtomicBool::new(false), _lease: lease });
    let raster = BackedRaster::new(store.clone(), q, 2, &ctl()).unwrap();
    (BackedLayer::new(raster, descriptor, &ctl()).unwrap(), store)
}

#[test]
fn six_runtime_domains_cross_tiles_preserve_raw_colors_and_match_original_alpha_kernel() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            let bounds = Rect::new(-3, 255, 260, 258);
            let (layer, store) = fixture(mode, sample, bounds, false);
            let operation = ThresholdAlpha { value: 0.5 };
            let prepared = PreparedBackedThresholdAlpha::new(&layer, layer.descriptor(), operation, NormalBandLimits::default(), &ctl()).unwrap();
            let band = prepared.render_band(bounds, &ctl()).unwrap();
            assert_eq!(band.rect(), bounds);
            assert_eq!(band.format(), layer.format());
            assert!(band.resident_bytes() >= band.bytes().len() as u64);
            for (raw, output) in store.bytes.chunks_exact(layer.format().bytes_per_pixel()).zip(band.bytes().chunks_exact(layer.format().bytes_per_pixel())) {
                let mut input = [0.; 4];
                for (c, value) in input.iter_mut().enumerate().take(layer.format().channels()) {
                    *value = try_read_sample_f64(raw, sample, c).unwrap();
                }
                let mut expected = [0.; 4];
                operation.try_eval_pixel(&input[..layer.format().channels()], mode.color_channels(), &mut expected[..layer.format().channels()]).unwrap();
                let color_bytes = mode.color_channels() * sample.bytes();
                assert_eq!(&output[..color_bytes], &raw[..color_bytes]);
                assert_eq!(try_read_sample_f64(output, sample, mode.color_channels()).unwrap().to_bits(), expected[mode.color_channels()].to_bits());
            }
            let mut source_pixel = vec![0; layer.format().bytes_per_pixel()];
            layer.checked_view().read_logical_pixel_native_into(bounds.x0, bounds.y0, &mut source_pixel, &ctl()).unwrap();
            assert_eq!(source_pixel, store.bytes[..source_pixel.len()]);
        }
    }
}

#[test]
fn roi_limits_descriptor_and_unqualified_runtime_formats_refuse_before_store_reads() {
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F32, Rect::new(-2, -1, 6, 3), false);
    let operation = ThresholdAlpha { value: 0.5 };
    let max = NormalBandLimits::default();
    for limits in [
        NormalBandLimits { max_rows: 0, ..max },
        NormalBandLimits { max_pixels: max.max_pixels + 1, ..max },
        NormalBandLimits { max_rows: max.max_rows + 1, ..max },
    ] {
        assert!(PreparedBackedThresholdAlpha::new(&layer, layer.descriptor(), operation, limits, &ctl()).is_err());
    }
    let wrong = NativePlaneDescriptor::new(layer.format(), Rect::new(0, 0, 8, 4), layer.default_bytes()).unwrap();
    assert!(PreparedBackedThresholdAlpha::new(&layer, wrong, operation, max, &ctl()).is_err());
    assert!(PreparedBackedThresholdAlpha::new(&layer, layer.descriptor(), ThresholdAlpha { value: f64::NAN }, max, &ctl()).is_err());
    let prepared = PreparedBackedThresholdAlpha::new(&layer, layer.descriptor(), operation, max, &ctl()).unwrap();
    for rect in [Rect::new(-3, -1, 6, 3), Rect::new(-2, -2, 6, 3), Rect::new(0, 0, 0, 1)] {
        assert!(prepared.render_band(rect, &ctl()).is_err());
    }
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    for sample in [SampleType::U8, SampleType::U32] {
        let (other, source) = fixture(ColorMode::Rgb, sample, Rect::new(0, 0, 8, 2), false);
        assert!(matches!(PreparedBackedThresholdAlpha::new(&other, other.descriptor(), operation, max, &ctl()), Err(BackedPointError::Unsupported(_))));
        assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn partial_band_and_cow_override_use_current_owned_words_without_changing_original() {
    let bounds = Rect::new(-2, -1, 6, 3);
    let (original, store) = fixture(ColorMode::Grayscale, SampleType::F64, bounds, false);
    let mut replacement = vec![0; original.format().bytes_per_pixel()];
    try_write_sample_f64(&mut replacement, SampleType::F64, 0, -0.0).unwrap();
    try_write_sample_f64(&mut replacement, SampleType::F64, 1, 0.5000000000000001).unwrap();
    let edited = original.try_edited_native(Rect::new(0, 0, 1, 1), &replacement, &ctl()).unwrap();
    let prepared = PreparedBackedThresholdAlpha::new(&edited, edited.descriptor(), ThresholdAlpha { value: 0.5 }, NormalBandLimits::default(), &ctl()).unwrap();
    let band = prepared.render_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap();
    assert_eq!(&band.bytes()[..8], &(-0.0f64).to_ne_bytes());
    assert_eq!(&band.bytes()[8..], &0.0f64.to_ne_bytes());
    assert_eq!(store.bytes.len(), bounds.width() as usize * bounds.height() as usize * original.format().bytes_per_pixel());
    let mut current = replacement.clone();
    edited.checked_view().read_logical_pixel_native_into(0, 0, &mut current, &ctl()).unwrap();
    assert_eq!(current, replacement);
    assert_ne!(original, edited);
}

#[test]
fn nonunit_sample_io_quota_work_and_late_cancellation_never_return_partial_bands() {
    let bounds = Rect::new(0, 0, 8, 4);
    let (invalid, _) = fixture(ColorMode::Rgb, SampleType::F32, bounds, true);
    let prepared =
        PreparedBackedThresholdAlpha::new(&invalid, invalid.descriptor(), ThresholdAlpha { value: 0.5 }, NormalBandLimits::default(), &ctl()).unwrap();
    assert!(matches!(prepared.render_band(bounds, &ctl()), Err(BackedPointError::Operation(_))));
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F64, bounds, false);
    let prepared = PreparedBackedThresholdAlpha::new(&layer, layer.descriptor(), ThresholdAlpha { value: 0.5 }, NormalBandLimits::default(), &ctl()).unwrap();
    store.fail.store(true, Ordering::SeqCst);
    assert!(prepared.render_band(bounds, &ctl()).is_err());
    store.fail.store(false, Ordering::SeqCst);
    let full = prepared.render_band(bounds, &ctl()).unwrap();
    drop(full); // Warm cache is derived/retained; price comparisons use this baseline.
    let baseline = layer.quota().usage().unwrap();
    for threshold in [0, 1, 100, 2048] {
        let used = AtomicU64::new(0);
        let work = |n| if used.fetch_add(n, Ordering::SeqCst).saturating_add(n) > threshold { Err(BackingError::Work) } else { Ok(()) };
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(prepared.render_band(bounds, &denied).is_err());
        assert_eq!(layer.quota().usage().unwrap(), baseline);
    }
    let calls = AtomicU64::new(0);
    let cancel = || calls.fetch_add(1, Ordering::SeqCst) > 150;
    let interrupt = Interrupt::cancel_only(&cancel);
    let late = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(prepared.render_band(bounds, &late).is_err());
    assert_eq!(layer.quota().usage().unwrap(), baseline);
    let held = layer.quota().reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(matches!(prepared.render_band(bounds, &ctl()), Err(BackedPointError::Backing(BackingError::Quota))));
    drop(held);
    assert_eq!(layer.quota().usage().unwrap(), baseline);
}

#[test]
fn retained_bands_keep_their_real_charges_after_prepared_layer_and_source_drop() {
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F64, Rect::new(0, 0, 8, 4), false);
    let q = layer.quota().clone();
    let prepared = PreparedBackedThresholdAlpha::new(&layer, layer.descriptor(), ThresholdAlpha { value: 0.5 }, NormalBandLimits::default(), &ctl()).unwrap();
    let first = prepared.render_band(Rect::new(0, 0, 8, 2), &ctl()).unwrap();
    let second = prepared.render_band(Rect::new(0, 2, 8, 4), &ctl()).unwrap();
    let fee = first.resident_bytes() + second.resident_bytes();
    drop(prepared);
    drop(layer);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage { resident: fee, index: 0, scratch: 0 });
    assert_eq!(first.bytes().len(), 8 * 2 * 4 * 8);
    drop(first);
    assert_eq!(q.usage().unwrap().resident, second.resident_bytes());
    drop(second);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[path = "backed_point_filter_disabled_tests.rs"]
mod disabled;
