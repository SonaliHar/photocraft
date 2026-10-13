use photocraft_cms::{Builtin, Profile};
use photocraft_color::{PixelFormat, SampleType, try_write_sample_f64};
use photocraft_compose::backed_normal::{BackedNormalError, NormalBandLimits, PreparedBackedNormal};
use photocraft_compose::gimp_composite::{WorkingSpace, encoding, space};
use photocraft_doc::{
    BackedLayer, ColorMode, Document, Layer, LayerContent, Rect, SampleEncoding, Size, WorkingProfileInterpretation, import_compositing::GimpOperator,
};
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits};
use photocraft_raster::backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl};
use photocraft_raster::{Interrupt, Surface};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
fn work(_: u64) -> Result<(), BackingError> {
    Ok(())
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &work }
}
fn quota(bytes: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: bytes, scratch: bytes, index: bytes })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    _lease: QuotaLease,
    quota: BackingQuota,
    reads: AtomicU64,
    failure: AtomicU64,
    integrity: AtomicBool,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        if offset >= self.failure.load(Ordering::SeqCst) {
            return Err(if self.integrity.load(Ordering::SeqCst) { BackingError::Integrity } else { BackingError::Io });
        }
        let offset = usize::try_from(offset).map_err(|_| BackingError::Integrity)?;
        let end = offset.checked_add(output.len()).ok_or(BackingError::Integrity)?;
        output.copy_from_slice(self.bytes.get(offset..end).ok_or(BackingError::Integrity)?);
        Ok(())
    }
}
fn pixel(format: PixelFormat, colors: f64, alpha: f64) -> Vec<u8> {
    let mut out = vec![0; format.bytes_per_pixel()];
    for channel in 0..format.channels() {
        try_write_sample_f64(&mut out, format.sample, channel, if format.alpha && channel == format.mode.color_channels() { alpha } else { colors }).unwrap();
    }
    out
}
fn setup(format: PixelFormat, bounds: Rect, q: BackingQuota) -> (BackedLayer, Surface, Arc<Store>) {
    let default = pixel(format, 0.125, 0.5);
    let count = bounds.width() as usize * bounds.height() as usize;
    let mut bytes = Vec::new();
    for i in 0..count {
        let mut p = pixel(format, 0.25 + (i % 3) as f64 * 0.25, [0.0, 0.25, 0.5, 1.0][i % 4]);
        if i == 1 && format.sample == SampleType::U32 {
            p[..4].copy_from_slice(&16_777_217_u32.to_ne_bytes());
        }
        if i == 2 && format.sample == SampleType::F64 {
            p[..8].copy_from_slice(&(-0.0_f64).to_ne_bytes());
        }
        if i == 3 && format.sample == SampleType::F64 {
            p[..8].copy_from_slice(&(0.5000000000000001_f64).to_ne_bytes());
        }
        bytes.extend(p);
    }
    let mut resident = Surface::try_with_default_bytes(format, &default).unwrap();
    resident.try_write_interleaved(bounds, &bytes).unwrap();
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let store = Arc::new(Store {
        descriptor,
        bytes,
        _lease: lease,
        quota: q.clone(),
        reads: AtomicU64::new(0),
        failure: AtomicU64::new(u64::MAX),
        integrity: AtomicBool::new(false),
    });
    let layer = BackedLayer::new(BackedRaster::new(store.clone(), q, 2, &ctl()).unwrap(), descriptor, &ctl()).unwrap();
    (layer, resident, store)
}
fn profile(mode: ColorMode, stored: SampleEncoding) -> Profile {
    let builtin = if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::SGray };
    photocraft_cms::xcf_color::effective_profile_from_definition(builtin.profile(), encoding(stored)).unwrap()
}
#[test]
fn native_five_depths_match_actual_resident_checked_normal() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let format = PixelFormat::new(mode, sample, true);
            let bounds = Rect::new(0, 0, 4, 3);
            let (layer, resident, store) = setup(format, bounds, quota(32 << 20));
            let original = store.bytes.clone();
            let current = profile(mode, SampleEncoding::NonlinearProfile);
            let prepared =
                PreparedBackedNormal::new(&layer, &current, SampleEncoding::NonlinearProfile, layer.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
            let output = prepared.render_band(bounds, [0.0; 4], &ctl()).unwrap();
            let mut doc = Document::new("Resident checked Normal oracle", Size::new(4, 3), mode, sample);
            doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
            let icc = current.to_bytes();
            doc.working_profile_interpretation =
                if mode == ColorMode::Rgb { WorkingProfileInterpretation::implicit_srgb(&icc) } else { WorkingProfileInterpretation::implicit_sgray(&icc) };
            doc.icc_profile = Some(icc);
            let mut owner = Layer::raster("Current", format);
            owner.content = LayerContent::Raster(resident);
            doc.layers.push(owner);
            let oracle = photocraft_compose::gimp_render::try_render(&doc, bounds, &mut |_| Ok(())).unwrap();
            assert_eq!(output.pixels(), oracle.px);
            assert_eq!(store.bytes, original);
            assert_eq!(output.resident_bytes(), output.pixels().len() as u64 * 32);
            if sample == SampleType::U32 {
                assert_eq!(output.pixels()[1][0], 16_777_217.0 / u32::MAX as f64);
            }
            if sample == SampleType::F64 {
                assert_eq!(output.pixels()[2][0].to_bits(), (-0.0_f64).to_bits());
                assert_eq!(output.pixels()[3][0].to_bits(), 0.5000000000000001_f64.to_bits());
            }
        }
    }
}
#[test]
fn varying_backdrop_negative_extent_and_current_transfer_match_resident_reads() {
    for stored in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile, SampleEncoding::Perceptual] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let format = PixelFormat::new(mode, SampleType::F64, true);
            let bounds = Rect::new(-1, -1, 3, 2);
            let (layer, resident, store) = setup(format, bounds, quota(32 << 20));
            let original = store.bytes.clone();
            let current = profile(mode, stored);
            let prepared = PreparedBackedNormal::new(&layer, &current, stored, layer.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
            let area = Rect::new(-2, -2, 4, 3);
            let backdrop: Vec<_> = (0..30)
                .map(|i| {
                    [0.2, if mode == ColorMode::Rgb { 0.3 } else { 0.2 }, if mode == ColorMode::Rgb { 0.4 } else { 0.2 }, if i % 2 == 0 { 0.75 } else { 1.0 }]
                })
                .collect();
            let output = prepared.render_band_over(area, &backdrop, &ctl()).unwrap();
            let ws = WorkingSpace::new(&current, stored, space(stored)).unwrap();
            let mut index = 0;
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let expected = if bounds.contains(x, y) {
                        let mut p = [0.0; 4];
                        resident.try_read_pixel_f64(x, y, &mut p[..format.channels()]).unwrap();
                        let src = if mode == ColorMode::Grayscale { [p[0], p[0], p[0], p[1]] } else { p };
                        ws.normal(backdrop[index], src, 1.0, GimpOperator::Union).unwrap()
                    } else {
                        backdrop[index]
                    };
                    assert_eq!(output.pixels()[index], expected);
                    index += 1;
                }
            }
            assert_eq!(store.bytes, original); // hidden alpha-zero samples and signedzero remain raw
            assert!(prepared.render_band_over(area, &backdrop[..29], &ctl()).is_err());
        }
    }
}
#[test]
fn output_quota_denial_precedes_source_read_and_concurrent_outputs_refund() {
    let q = quota(1 << 20);
    let (layer, _, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), q.clone());
    let p = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    let prepared = PreparedBackedNormal::new(&layer, &p, SampleEncoding::NonlinearProfile, layer.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(prepared.render_band(Rect::new(0, 0, 256, 256), [0.0; 4], &ctl()), Err(BackedNormalError::Quota(_))));
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    assert_eq!(q.usage().unwrap(), before);
    let a = prepared.render_band(Rect::new(0, 0, 2, 1), [0.0; 4], &ctl()).unwrap();
    let b = prepared.render_band(Rect::new(0, 0, 2, 1), [0.0; 4], &ctl()).unwrap();
    let live = q.usage().unwrap().resident;
    assert_eq!(a.resident_bytes(), 64);
    drop(a);
    assert_eq!(q.usage().unwrap().resident, live - 64);
    drop(b);
    drop(prepared);
    drop((layer, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn late_io_work_and_cancel_never_return_partial_band() {
    let q = quota(8 << 20);
    let (layer, _, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 257, 1), q.clone());
    let p = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    let prepared = PreparedBackedNormal::new(&layer, &p, SampleEncoding::NonlinearProfile, layer.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    store.failure.store(1024, Ordering::SeqCst);
    assert!(matches!(prepared.render_band(layer.operative_bounds(), [0.0; 4], &ctl()), Err(BackedNormalError::View(_))));
    assert_eq!(layer, layer.clone());
    store.integrity.store(true, Ordering::SeqCst);
    assert!(matches!(
        prepared.render_band(layer.operative_bounds(), [0.0; 4], &ctl()),
        Err(BackedNormalError::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Integrity)))
    ));
    store.integrity.store(false, Ordering::SeqCst);
    store.failure.store(u64::MAX, Ordering::SeqCst);
    let completed = prepared.render_band(layer.operative_bounds(), [0.0; 4], &ctl()).unwrap();
    drop(completed);
    let before = q.usage().unwrap();
    let count = AtomicU64::new(0);
    let meter = |_: u64| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    drop(prepared.render_band(layer.operative_bounds(), [0.0; 4], &ReadControl { interrupt: &Interrupt::NONE, work: &meter }).unwrap());
    let last = count.load(Ordering::SeqCst);
    count.store(0, Ordering::SeqCst);
    let denied = |_: u64| if count.fetch_add(1, Ordering::SeqCst) + 1 == last { Err(BackingError::Work) } else { Ok(()) };
    assert!(matches!(
        prepared.render_band(layer.operative_bounds(), [0.0; 4], &ReadControl { interrupt: &Interrupt::NONE, work: &denied }),
        Err(BackedNormalError::Backing(BackingError::Work))
    ));
    assert_eq!(q.usage().unwrap(), before);
    count.store(0, Ordering::SeqCst);
    let cancelled = AtomicBool::new(false);
    let cancelling = |_: u64| {
        if count.fetch_add(1, Ordering::SeqCst) + 1 == last {
            cancelled.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let cancel = || cancelled.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&cancel);
    assert!(matches!(
        prepared.render_band(layer.operative_bounds(), [0.0; 4], &ReadControl { interrupt: &interrupt, work: &cancelling }),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), before);
    drop(prepared);
    drop((layer, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn malformed_samples_profiles_bounds_and_bands_are_named_errors() {
    let q = quota(32 << 20);
    let (mut layer, _, _) = setup(PixelFormat::RGBA64F, Rect::new(0, 0, 1, 1), q.clone());
    let current = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    let wrong = profile(ColorMode::Grayscale, SampleEncoding::NonlinearProfile);
    assert!(matches!(
        PreparedBackedNormal::new(&layer, &wrong, SampleEncoding::NonlinearProfile, layer.descriptor(), NormalBandLimits::default(), &ctl()),
        Err(BackedNormalError::Invalid(_))
    ));
    assert!(PreparedBackedNormal::new(&layer, &current, SampleEncoding::Linear, layer.descriptor(), NormalBandLimits::default(), &ctl()).is_err());
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA64F, Rect::new(0, 0, 2, 1), &[0; 32]).unwrap();
    assert!(PreparedBackedNormal::new(&layer, &current, SampleEncoding::NonlinearProfile, descriptor, NormalBandLimits::default(), &ctl()).is_err());
    let past = layer.clone();
    for (channel, value) in [0, 3].into_iter().flat_map(|channel| [f64::NAN, f64::INFINITY, -0.01, 1.01].map(|value| (channel, value))) {
        let mut raw = pixel(PixelFormat::RGBA64F, 0.5, 1.0);
        raw[channel * 8..(channel + 1) * 8].copy_from_slice(&value.to_ne_bytes());
        layer.try_write_native(layer.operative_bounds(), &raw, &ctl()).unwrap();
        let prepared =
            PreparedBackedNormal::new(&layer, &current, SampleEncoding::NonlinearProfile, layer.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
        assert!(matches!(prepared.render_band(layer.operative_bounds(), [0.0; 4], &ctl()), Err(BackedNormalError::Unsupported(_))));
    }
    assert_ne!(past, layer);
    let prepared =
        PreparedBackedNormal::new(&past, &current, SampleEncoding::NonlinearProfile, past.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    assert!(prepared.render_band(Rect::new(0, 0, 1, 257), [0.0; 4], &ctl()).is_err());
    assert!(prepared.render_band(past.operative_bounds(), [1.1; 4], &ctl()).is_err());
    drop(prepared);
    drop((layer, past));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn inactive_profile_tables_cannot_hide_preparation_allocation() {
    let q = quota(8 << 20);
    let (layer, _, _) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), q.clone());
    let mut current = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    current.gray_trc = Some(photocraft_cms::curve::Curve::Table64(vec![0.0, 0.5, 1.0]));
    let before = q.usage().unwrap();
    assert!(matches!(
        PreparedBackedNormal::new(&layer, &current, SampleEncoding::NonlinearProfile, layer.descriptor(), NormalBandLimits::default(), &ctl()),
        Err(BackedNormalError::Unsupported("analytic profile curves only"))
    ));
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
fn current_history_native_defaults_and_bands_own_separate_leases() {
    let q = quota(32 << 20);
    let (past, resident, store) = setup(PixelFormat::RGBA64F, Rect::new(-1, -1, 3, 2), q.clone());
    let original = store.bytes.clone();
    let default = past.default_bytes().to_vec();
    let raw = pixel(PixelFormat::RGBA64F, 0.5000000000000001, 0.75);
    let current = past.try_edited_native(Rect::new(0, 0, 1, 1), &raw, &ctl()).unwrap();
    let p = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    let before = PreparedBackedNormal::new(&past, &p, SampleEncoding::NonlinearProfile, past.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    let after = PreparedBackedNormal::new(&current, &p, SampleEncoding::NonlinearProfile, current.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    let area = past.operative_bounds();
    let a = before.render_band(area, [0.0; 4], &ctl()).unwrap();
    let b = after.render_band(area, [0.0; 4], &ctl()).unwrap();
    assert_ne!(a.pixels(), b.pixels());
    assert_eq!(b.pixels()[5][0].to_bits(), 0.5000000000000001_f64.to_bits());
    assert_eq!(past.default_bytes(), default);
    assert_eq!(current.default_bytes(), default);
    assert_eq!(store.bytes, original);
    assert_eq!(resident.try_read_interleaved(area).unwrap(), original);
    assert_ne!(past, current);
    assert_eq!(past.clone(), past);
    drop((before, after));
    drop((past, current, store));
    assert_eq!(q.usage().unwrap().resident, a.resident_bytes() + b.resident_bytes());
    drop(a);
    assert_eq!(q.usage().unwrap().resident, b.resident_bytes());
    drop(b);
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[derive(Debug)]
struct ConstantStore {
    descriptor: NativePlaneDescriptor,
}
impl NativePlaneStore for ConstantStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Integrity);
        }
        let pixel = self.descriptor.default_bytes();
        for (i, byte) in output.iter_mut().enumerate() {
            *byte = pixel[((offset + i as u64) % pixel.len() as u64) as usize];
        }
        Ok(())
    }
}
#[test]
#[ignore = "release-only 24MP generated-native backing consumer measurement"]
fn release_24mp_bounded_backed_normal() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let q = quota(8 << 20);
    let bounds = Rect::new(0, 0, 6000, 4000);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, bounds, &[64, 128, 192, 128]).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(ConstantStore { descriptor }), q.clone(), 2, &ctl()).unwrap(), descriptor, &ctl()).unwrap();
    let profile = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    let prepared = PreparedBackedNormal::new(&layer, &profile, SampleEncoding::NonlinearProfile, descriptor, NormalBandLimits::default(), &ctl()).unwrap();
    let expected = [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 128.0 / 255.0];
    let started = std::time::Instant::now();
    let mut visited = 0_u64;
    let mut peak = q.usage().unwrap().resident;
    for y in (0..4000).step_by(256) {
        for x in (0..6000).step_by(256) {
            let rect = Rect::new(x, y, (x + 256).min(6000), (y + 256).min(4000));
            let band = prepared.render_band(rect, [0.0; 4], &ctl()).unwrap();
            assert!(band.pixels().iter().all(|pixel| *pixel == expected));
            visited += band.pixels().len() as u64;
            peak = peak.max(q.usage().unwrap().resident);
        }
    }
    assert_eq!(visited, 24_000_000);
    println!(
        "generated native 24MP RGBA8 / two-tile cache / exact output checked: {:?}; sampled retained usage after returned bands={}B (excludes allocation-envelope transients)",
        started.elapsed(),
        peak
    );
    drop(prepared);
    drop(layer);
    assert_eq!(q.usage().unwrap().resident, 0);
    Ok(())
}

#[test]
fn explicit_cross_space_native_matrix_matches_resident_union_and_preserves_state() {
    use photocraft_doc::import_compositing::GimpSpace;
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            for stored in [SampleEncoding::NonlinearProfile, SampleEncoding::Linear, SampleEncoding::Perceptual] {
                for composite in [GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpSpace::PerceptualRgb] {
                    let q = quota(32 << 20);
                    let format = PixelFormat::new(mode, sample, true);
                    let bounds = Rect::new(-1, -1, 3, 2);
                    let (layer, resident, store) = setup(format, bounds, q.clone());
                    let original = store.bytes.clone();
                    let default = layer.default_bytes().to_vec();
                    let history = layer.clone();
                    let current = profile(mode, stored);
                    let prepared = PreparedBackedNormal::new_with_composite_space(
                        &layer,
                        &current,
                        stored,
                        composite,
                        layer.descriptor(),
                        NormalBandLimits::default(),
                        &ctl(),
                    )
                    .unwrap();
                    let oracle = WorkingSpace::new(&current, stored, composite).unwrap();
                    let area = Rect::new(-2, -2, 4, 3);
                    let back: Vec<_> = (0..30)
                        .map(|i| {
                            [
                                0.2,
                                if mode == ColorMode::Rgb { 0.3 } else { 0.2 },
                                if mode == ColorMode::Rgb { 0.4 } else { 0.2 },
                                if i % 2 == 0 { 0.75 } else { 1.0 },
                            ]
                        })
                        .collect();
                    let band = prepared.render_band_over(area, &back, &ctl()).unwrap();
                    for y in area.y0..area.y1 {
                        for x in area.x0..area.x1 {
                            let index = ((y - area.y0) * area.width() as i32 + x - area.x0) as usize;
                            let expected = if bounds.contains(x, y) {
                                let mut native = vec![0.0; format.channels()];
                                resident.try_read_pixel_f64(x, y, &mut native).unwrap();
                                let source = if mode == ColorMode::Rgb {
                                    [native[0], native[1], native[2], native[3]]
                                } else {
                                    [native[0], native[0], native[0], native[1]]
                                };
                                oracle.normal(back[index], source, 1.0, GimpOperator::Union).unwrap()
                            } else {
                                back[index]
                            };
                            assert_eq!(band.pixels()[index], expected, "{mode:?}/{sample:?}/{stored:?}/{composite:?}");
                        }
                    }
                    assert_eq!(store.bytes, original);
                    assert_eq!(layer.default_bytes(), default);
                    assert_eq!(layer, history);
                    drop((band, prepared));
                    drop((layer, history, store));
                    assert_eq!(q.usage().unwrap().resident, 0);
                }
            }
        }
    }
}

#[test]
fn explicit_cross_space_is_observable_and_preparation_is_prepaid() {
    use photocraft_doc::import_compositing::GimpSpace;
    let q = quota(32 << 20);
    let (layer, _, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 4, 3), q.clone());
    let current = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    let same = PreparedBackedNormal::new(&layer, &current, SampleEncoding::NonlinearProfile, layer.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    let cross = PreparedBackedNormal::new_with_composite_space(
        &layer,
        &current,
        SampleEncoding::NonlinearProfile,
        GimpSpace::LinearRgb,
        layer.descriptor(),
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap();
    let a = same.render_band(layer.operative_bounds(), [0.9, 0.1, 0.3, 1.0], &ctl()).unwrap();
    let b = cross.render_band(layer.operative_bounds(), [0.9, 0.1, 0.3, 1.0], &ctl()).unwrap();
    assert!(a.pixels().iter().zip(b.pixels()).any(|(a, b)| a != b));
    let before = q.usage().unwrap();
    let reads_before = store.reads.load(Ordering::SeqCst);
    let rest = q.reserve(QuotaKind::Resident, (32 << 20) - before.resident - 4096).unwrap();
    assert!(matches!(
        PreparedBackedNormal::new_with_composite_space(
            &layer,
            &current,
            SampleEncoding::NonlinearProfile,
            GimpSpace::LinearRgb,
            layer.descriptor(),
            NormalBandLimits::default(),
            &ctl()
        ),
        Err(BackedNormalError::Quota(_))
    ));
    assert_eq!(store.reads.load(Ordering::SeqCst), reads_before);
    drop(rest);
    assert_eq!(q.usage().unwrap(), before);
    assert!(matches!(
        PreparedBackedNormal::new_with_composite_space(
            &layer,
            &current,
            SampleEncoding::NonlinearProfile,
            GimpSpace::Lab,
            layer.descriptor(),
            NormalBandLimits::default(),
            &ctl()
        ),
        Err(BackedNormalError::Unsupported(_))
    ));
    drop((a, b, cross, same));
    drop((layer, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn cross_space_preparation_failure_refunds_retained_profile_before_publication() {
    use photocraft_doc::import_compositing::GimpSpace;
    let q = quota(2 << 20);
    let (layer, _, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), q.clone());
    let mut current = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    current.description = "original procedural description ".repeat(256);
    let baseline = q.usage().unwrap();
    for stop in [2, 3] {
        let calls = AtomicU64::new(0);
        let deny = |_: u64| if calls.fetch_add(1, Ordering::SeqCst) + 1 == stop { Err(BackingError::Work) } else { Ok(()) };
        assert!(matches!(
            PreparedBackedNormal::new_with_composite_space(
                &layer,
                &current,
                SampleEncoding::NonlinearProfile,
                GimpSpace::LinearRgb,
                layer.descriptor(),
                NormalBandLimits::default(),
                &ReadControl { interrupt: &Interrupt::NONE, work: &deny }
            ),
            Err(BackedNormalError::Backing(BackingError::Work))
        ));
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let cancelled = AtomicBool::new(false);
    let calls = AtomicU64::new(0);
    let work = |_: u64| {
        if calls.fetch_add(1, Ordering::SeqCst) == 2 {
            cancelled.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let cancel = || cancelled.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&cancel);
    assert!(matches!(
        PreparedBackedNormal::new_with_composite_space(
            &layer,
            &current,
            SampleEncoding::NonlinearProfile,
            GimpSpace::LinearRgb,
            layer.descriptor(),
            NormalBandLimits::default(),
            &ReadControl { interrupt: &interrupt, work: &work }
        ),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    let prepared = PreparedBackedNormal::new_with_composite_space(
        &layer,
        &current,
        SampleEncoding::NonlinearProfile,
        GimpSpace::LinearRgb,
        layer.descriptor(),
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap();
    assert!(q.usage().unwrap().resident - baseline.resident > current.description.len() as u64);
    drop(prepared);
    assert_eq!(q.usage().unwrap(), baseline);
    drop((layer, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
#[ignore = "release-only 24MP explicit cross-space generated-native measurement"]
fn release_24mp_explicit_cross_space() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    use photocraft_doc::import_compositing::GimpSpace;
    let q = quota(8 << 20);
    let bounds = Rect::new(0, 0, 6000, 4000);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, bounds, &[64, 128, 192, 128]).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(ConstantStore { descriptor }), q.clone(), 2, &ctl()).unwrap(), descriptor, &ctl()).unwrap();
    let current = profile(ColorMode::Rgb, SampleEncoding::NonlinearProfile);
    let prepared = PreparedBackedNormal::new_with_composite_space(
        &layer,
        &current,
        SampleEncoding::NonlinearProfile,
        GimpSpace::LinearRgb,
        descriptor,
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap();
    let backdrop = [0.2, 0.1, 0.8, 0.7];
    let expected = WorkingSpace::new(&current, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb)
        .unwrap()
        .normal(backdrop, [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 128.0 / 255.0], 1.0, GimpOperator::Union)
        .unwrap();
    let start = std::time::Instant::now();
    let mut pixels = 0_u64;
    let mut sampled = q.usage().unwrap().resident;
    for y in (0..4000).step_by(256) {
        for x in (0..6000).step_by(256) {
            let band = prepared.render_band(Rect::new(x, y, (x + 256).min(6000), (y + 256).min(4000)), backdrop, &ctl()).unwrap();
            assert!(band.pixels().iter().all(|pixel| *pixel == expected));
            pixels += band.pixels().len() as u64;
            sampled = sampled.max(q.usage().unwrap().resident);
        }
    }
    assert_eq!(pixels, 24_000_000);
    println!(
        "generated 24MP NL-to-Linear Normal/Union exact checked: {:?}; sampled retained after bands={}B (not transient peak/RSS)",
        start.elapsed(),
        sampled
    );
    drop(prepared);
    drop(layer);
    assert_eq!(q.usage().unwrap().resident, 0);
    Ok(())
}
