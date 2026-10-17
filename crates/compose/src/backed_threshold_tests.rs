//! UNRUN typed nonlinear RGB native Threshold leaf tests. Synthetic source
//! storage is genuinely charged; no source/schema/profile enrollment is asserted.
use super::*;
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
fn prepare<'a>(layer: &'a BackedLayer, operation: Threshold) -> PreparedBackedThreshold<'a> {
    PreparedBackedThreshold::new(layer, layer.descriptor(), SampleEncoding::NonlinearProfile, operation, Default::default(), &ctl()).unwrap()
}
#[test]
fn all_histogram_channels_match_existing_math_and_preserve_native_alpha_across_tiles() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let bounds = Rect::new(-3, 255, 260, 258);
        let (layer, store) = fixture(ColorMode::Rgb, sample, bounds, false);
        let bpp = layer.format().bytes_per_pixel();
        let alpha = 3 * sample.bytes();
        let original = store.bytes.clone();
        for channel in 0..=5 {
            let operation = Threshold { channel, low: 0.25, high: 0.75 };
            let prepared = prepare(&layer, operation);
            let band = prepared.render_band(bounds, &ctl()).unwrap();
            assert_eq!(band.rect(), bounds);
            assert_eq!(band.format(), layer.format());
            for (input, output) in store.bytes.chunks_exact(bpp).zip(band.bytes().chunks_exact(bpp)) {
                let values = std::array::from_fn::<_, 4, _>(|c| try_read_sample_f64(input, sample, c).unwrap());
                let mut expected = [0.; 4];
                operation.try_eval_pixel(&values, 3, &mut expected).unwrap();
                for (c, expected_value) in expected.iter().enumerate().take(3) {
                    let actual = try_read_sample_f64(output, sample, c).unwrap();
                    assert_eq!(actual, *expected_value);
                    assert!(actual == 0. || actual == 1.);
                }
                assert_eq!(&output[alpha..], &input[alpha..], "F64 alpha lowbits and signed zero stay raw");
            }
        }
        assert_eq!(store.bytes, original);
        assert_eq!(layer.default_bytes(), store.descriptor.default_bytes());
    }
}
#[test]
fn inclusive_float_histogram_boundaries_reversed_ranges_and_disabled_identity() {
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F64, Rect::new(0, 0, 5, 1), false);
    for (channel, low, high) in [(1, 0.5, 0.5), (4, 0.5, 0.5), (0, 0.75, 0.25)] {
        let p = prepare(&layer, Threshold { channel, low, high });
        let band = p.render_band(layer.operative_bounds(), &ctl()).unwrap();
        for (input, output) in store.bytes.as_chunks::<32>().0.iter().zip(band.bytes().as_chunks::<32>().0.iter()) {
            let values = std::array::from_fn::<_, 4, _>(|c| try_read_sample_f64(input, SampleType::F64, c).unwrap());
            let mut expected = [0.; 4];
            Threshold { channel, low, high }.try_eval_pixel(&values, 3, &mut expected).unwrap();
            assert_eq!(try_read_sample_f64(output, SampleType::F64, 0).unwrap(), expected[0]);
            assert_eq!(&output[24..], &input[24..]);
        }
    }
    let disabled = PreparedBackedThreshold::new_with_enabled(
        &layer,
        layer.descriptor(),
        SampleEncoding::NonlinearProfile,
        Threshold { channel: 0, low: 1., high: 0. },
        false,
        Default::default(),
        &ctl(),
    )
    .unwrap();
    assert_eq!(disabled.render_band(layer.operative_bounds(), &ctl()).unwrap().bytes(), store.bytes);
}
#[test]
fn constructor_domain_descriptor_operation_and_roi_limits_refuse_before_reads() {
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F32, Rect::new(0, 0, 2, 2), false);
    let op = Threshold { channel: 0, low: 0.25, high: 0.75 };
    for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
        assert!(PreparedBackedThreshold::new(&layer, layer.descriptor(), encoding, op, Default::default(), &ctl()).is_err());
    }
    for op in [Threshold { channel: 6, ..op }, Threshold { low: f64::NAN, ..op }] {
        assert!(PreparedBackedThreshold::new(&layer, layer.descriptor(), SampleEncoding::NonlinearProfile, op, Default::default(), &ctl()).is_err());
    }
    let wrong = NativePlaneDescriptor::new(layer.format(), Rect::new(0, 0, 1, 1), layer.default_bytes()).unwrap();
    assert!(PreparedBackedThreshold::new(&layer, wrong, SampleEncoding::NonlinearProfile, op, Default::default(), &ctl()).is_err());
    let p =
        PreparedBackedThreshold::new(&layer, layer.descriptor(), SampleEncoding::NonlinearProfile, op, NormalBandLimits { max_rows: 1, max_pixels: 1 }, &ctl())
            .unwrap();
    for rect in [Rect::EMPTY, Rect::new(-1, 0, 0, 1), layer.operative_bounds()] {
        assert!(p.render_band(rect, &ctl()).is_err());
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    for (mode, sample) in [(ColorMode::Grayscale, SampleType::F64), (ColorMode::Rgb, SampleType::U8), (ColorMode::Rgb, SampleType::U32)] {
        let (other, _) = fixture(mode, sample, Rect::new(0, 0, 1, 1), false);
        assert!(PreparedBackedThreshold::new(&other, other.descriptor(), SampleEncoding::NonlinearProfile, op, Default::default(), &ctl()).is_err());
    }
}
#[test]
fn invalid_native_and_disabled_invalid_native_io_cancel_work_and_quota_fail_privately() {
    for enabled in [false, true] {
        let (layer, _) = fixture(ColorMode::Rgb, SampleType::F64, Rect::new(0, 0, 2, 1), true);
        let p = PreparedBackedThreshold::new_with_enabled(
            &layer,
            layer.descriptor(),
            SampleEncoding::NonlinearProfile,
            Threshold { channel: 0, low: 0., high: 1. },
            enabled,
            Default::default(),
            &ctl(),
        )
        .unwrap();
        assert!(p.render_band(layer.operative_bounds(), &ctl()).is_err());
    }
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F64, Rect::new(0, 0, 2, 1), false);
    let p = prepare(&layer, Threshold { channel: 0, low: 0., high: 1. });
    let baseline = layer.quota().usage().unwrap();
    store.fail.store(true, Ordering::Relaxed);
    assert!(p.render_band(layer.operative_bounds(), &ctl()).is_err());
    assert_eq!(layer.quota().usage().unwrap(), baseline);
    store.fail.store(false, Ordering::Relaxed);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(p.render_band(layer.operative_bounds(), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    let deny = |_| Err(BackingError::Work);
    assert!(p.render_band(layer.operative_bounds(), &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    let hold = layer.quota().reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(p.render_band(layer.operative_bounds(), &ctl()).is_err());
    drop(hold);
    assert_eq!(layer.quota().usage().unwrap(), baseline);
}
#[test]
fn retained_outputs_remain_charged_and_subregions_match_full_rows() {
    let (layer, store) = fixture(ColorMode::Rgb, SampleType::F64, Rect::new(0, 0, 8, 2), false);
    let p = prepare(&layer, Threshold { channel: 5, low: 0.25, high: 0.75 });
    let full = p.render_band(layer.operative_bounds(), &ctl()).unwrap();
    let baseline = layer.quota().usage().unwrap();
    let row = p.render_band(Rect::new(0, 1, 8, 2), &ctl()).unwrap();
    assert_eq!(row.bytes(), full.bytes().get(8 * 32..).unwrap());
    assert!(layer.quota().usage().unwrap().resident >= baseline.resident + row.resident_bytes());
    drop(row);
    assert_eq!(layer.quota().usage().unwrap(), baseline);
    assert!(!store.bytes.is_empty());
}
