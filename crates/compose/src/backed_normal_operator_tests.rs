//! UNRUN generated leaf regressions; original GIMP source enrollment is separate.
use super::*;
use photocraft_cms::Builtin;
use photocraft_color::{PixelFormat, SampleType, try_write_sample_f64};
use photocraft_raster::{
    Interrupt, Surface,
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
    q: BackingQuota,
    reads: Arc<AtomicU64>,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let at = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
struct Fixture {
    layer: BackedLayer,
    store: Arc<Store>,
    q: BackingQuota,
    reads: Arc<AtomicU64>,
    _metadata: QuotaLease,
}
fn fixture(mode: ColorMode, sample: SampleType) -> Fixture {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 4 << 20, scratch: 0 });
    // Real prospective storage for this fixed generated fixture and oracle buffers.
    let metadata = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let format = PixelFormat::new(mode, sample, true);
    let bounds = Rect::new(-2, 1, 1, 3);
    let fee = q.reserve(QuotaKind::Resident, 4096).unwrap();
    let mut bytes = vec![0; 6 * format.bytes_per_pixel()];
    for (i, p) in bytes.chunks_exact_mut(format.bytes_per_pixel()).enumerate() {
        for channel in 0..mode.color_channels() {
            try_write_sample_f64(p, sample, channel, 0.125 + (i % 3) as f64 * 0.25).unwrap();
        }
        try_write_sample_f64(p, sample, mode.color_channels(), if i % 3 == 0 { 0.0 } else { 0.625 }).unwrap();
    }
    let mut default = vec![0; format.bytes_per_pixel()];
    for channel in 0..format.channels() {
        try_write_sample_f64(&mut default, sample, channel, 0.75).unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let store = Arc::new(Store { descriptor, bytes, q: q.clone(), reads: reads.clone(), _fee: fee });
    let raster = BackedRaster::new(store.clone(), q.clone(), 1, &ctl()).unwrap();
    let layer = BackedLayer::new(raster, descriptor, &ctl()).unwrap();
    Fixture { layer, store, q, reads, _metadata: metadata }
}
fn profile(mode: ColorMode) -> &'static Profile {
    if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() }
}
fn prepared<'a>(f: &'a Fixture, op: GimpOperator) -> PreparedBackedNormal<'a> {
    PreparedBackedNormal::new_with_composite_operator(
        &f.layer,
        profile(f.layer.format().mode),
        SampleEncoding::NonlinearProfile,
        GimpSpace::LinearRgb,
        op,
        f.layer.descriptor(),
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap()
}
fn operators() -> [GimpOperator; 4] {
    [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection]
}
#[test]
fn all_operators_offset_extents_mask_zero_and_transparent_backdrops_reuse_existing_math() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let f = fixture(mode, sample);
            let before = f.store.bytes.clone();
            let rect = Rect::new(-3, 0, 2, 4);
            let mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, false), &[0.5]);
            let maskbounds = Rect::new(-1, 1, 1, 3);
            let view = photocraft_raster::checked_view::CheckedRasterView::resident_with_control(&mask, maskbounds, &ctl()).unwrap();
            let mask = Some(ResidentNormalMask { view, enabled: true, outside_coverage: 0.0 });
            let oracle = WorkingSpace::new(profile(mode), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
            for op in operators() {
                let p = prepared(&f, op);
                // Nonzero RGB under zero backdrop alpha is deliberate: don't substitute an
                // all-zero backdrop optimization or erase inert colors on retained paths.
                let backdrop: Vec<_> = (0..20).map(|i| [0.25, 0.25, 0.25, if i % 3 == 0 { 0.0 } else { 0.5 }]).collect();
                let band = p.render_band_over_with_resident_coverage(rect, &backdrop, 0.375, mask, &ctl()).unwrap();
                for (y, row) in band.pixels().chunks_exact(5).enumerate() {
                    for (x, actual) in row.iter().enumerate() {
                        let (x, y) = (rect.x0 + x as i32, rect.y0 + y as i32);
                        let index = ((y - rect.y0) * 5 + x - rect.x0) as usize;
                        let (mut source, mut coverage) = ([0.; 4], 0.0);
                        if f.layer.operative_bounds().contains(x, y) {
                            let mut native = [0.; 4];
                            let channels = f.layer.format().channels();
                            f.layer.checked_view().read_logical_pixel_f64_into(x, y, &mut native[..channels], &ctl()).unwrap();
                            source = if mode == ColorMode::Grayscale { [native[0], native[0], native[0], native[1]] } else { native };
                            if maskbounds.contains(x, y) {
                                let mut m = [0.];
                                view.read_logical_pixel_f64_into(x, y, &mut m, &ctl()).unwrap();
                                coverage = 0.375 * m[0];
                            }
                        }
                        let expected = oracle.normal(backdrop[index], source, coverage, op).unwrap();
                        assert_eq!(*actual, expected, "{mode:?}/{sample:?}/{op:?} at {x},{y}");
                    }
                }
            }
            assert_eq!(f.store.bytes, before, "original native source never changes");
        }
    }
}
#[test]
fn visible_zero_opacity_never_reads_source_and_old_union_constructors_keep_exact_result() {
    let f = fixture(ColorMode::Rgb, SampleType::F64);
    let rect = Rect::new(-3, 0, 2, 4);
    let bg = [0.125, 0.25, 0.5, 0.0];
    let oracle = WorkingSpace::new(profile(ColorMode::Rgb), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
    for op in operators() {
        let p = prepared(&f, op);
        let band = p.render_band_with_coverage(rect, bg, 0.0, None, &ctl()).unwrap();
        let expected = oracle.normal(bg, [0.; 4], 0.0, op).unwrap();
        assert!(band.pixels().iter().all(|v| *v == expected));
    }
    assert_eq!(f.reads.load(Ordering::Relaxed), 0);
    let old = PreparedBackedNormal::new_with_composite_space(
        &f.layer,
        profile(ColorMode::Rgb),
        SampleEncoding::NonlinearProfile,
        GimpSpace::LinearRgb,
        f.layer.descriptor(),
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap();
    let explicit = prepared(&f, GimpOperator::Union);
    assert_eq!(old.render_band(rect, bg, &ctl()).unwrap().pixels(), explicit.render_band(rect, bg, &ctl()).unwrap().pixels());
}
#[test]
fn actual_parent_work_quota_and_interrupt_failures_refund_private_preparation_and_bands() {
    let f = fixture(ColorMode::Rgb, SampleType::F64);
    let before = f.q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n >= 200000 { Err(BackingError::Work) } else { Ok(()) } };
    assert!(matches!(
        PreparedBackedNormal::new_with_composite_operator(
            &f.layer,
            profile(ColorMode::Rgb),
            SampleEncoding::NonlinearProfile,
            GimpSpace::LinearRgb,
            GimpOperator::Intersection,
            f.layer.descriptor(),
            NormalBandLimits::default(),
            &denied
        ),
        Err(BackedNormalError::Backing(BackingError::Work))
    ));
    assert_eq!(f.q.usage().unwrap(), before);
    // Arm after real profile/header leases are admitted, but permit the outer
    // post-work poll and final pre/post polls (three total). Cancellation on the
    // fourth armed poll therefore requires the actual WorkingSpace caller token;
    // the old internal Interrupt::NONE path would succeed instead.
    let stop = AtomicBool::new(false);
    let armed_polls = AtomicU64::new(0);
    let cancelled = || stop.load(Ordering::Relaxed) && armed_polls.fetch_add(1, Ordering::Relaxed) >= 3;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let strings = profile(ColorMode::Rgb).description.len() + profile(ColorMode::Rgb).copyright.len() + 1;
    let work = |n| {
        if n == strings as u64 {
            stop.store(true, Ordering::Relaxed);
        }
        Ok(())
    };
    let c = ReadControl { interrupt: &interrupt, work: &work };
    assert!(matches!(
        PreparedBackedNormal::new_with_composite_operator(
            &f.layer,
            profile(ColorMode::Rgb),
            SampleEncoding::NonlinearProfile,
            GimpSpace::LinearRgb,
            GimpOperator::ClipToLayer,
            f.layer.descriptor(),
            NormalBandLimits::default(),
            &c
        ),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert!(stop.load(Ordering::Relaxed));
    assert!(armed_polls.load(Ordering::Relaxed) >= 4, "actual WorkingSpace interrupt polls must exceed the outer-only path");
    assert_eq!(f.q.usage().unwrap(), before);
    let p = prepared(&f, GimpOperator::Intersection);
    let before = f.q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n == 32 { Err(BackingError::Work) } else { Ok(()) } };
    assert!(matches!(p.render_band(Rect::new(-3, 0, 2, 4), [0.5; 4], &denied), Err(BackedNormalError::Backing(BackingError::Work))));
    assert_eq!(f.q.usage().unwrap(), before);
    let blocker = f.q.reserve(QuotaKind::Resident, (64 << 20) - before.resident - 1).unwrap();
    let tight = f.q.usage().unwrap();
    assert!(matches!(p.render_band(Rect::new(-3, 0, 2, 4), [0.5; 4], &ctl()), Err(BackedNormalError::Quota(_))));
    assert_eq!(f.q.usage().unwrap(), tight);
    drop(blocker);
    assert_eq!(f.q.usage().unwrap(), before);
}
