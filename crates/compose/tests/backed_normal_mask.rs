use photocraft_cms::{Builtin, Profile};
use photocraft_color::{PixelFormat, SampleType, try_write_sample_f64};
use photocraft_compose::backed_normal::{BackedNormalError, NormalBandLimits, NormalMask, PreparedBackedNormal};
use photocraft_compose::gimp_composite::{WorkingSpace, encoding, space};
use photocraft_doc::{BackedLayer, ColorMode, Rect, SampleEncoding, import_compositing::GimpOperator};
use photocraft_raster::Interrupt;
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits};
use photocraft_raster::backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl};
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
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 64 << 20, scratch: 64 << 20 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    quota: BackingQuota,
    reads: AtomicU64,
    fail: AtomicBool,
    fail_at: AtomicU64,
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
        if self.fail.load(Ordering::SeqCst) || offset >= self.fail_at.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        let a = offset as usize;
        out.copy_from_slice(self.bytes.get(a..a + out.len()).ok_or(BackingError::Integrity)?);
        Ok(())
    }
}
fn layer(format: PixelFormat, bounds: Rect, value: f64, q: &BackingQuota) -> (BackedLayer, Arc<Store>) {
    let mut pixel = vec![0; format.bytes_per_pixel()];
    for i in 0..format.channels() {
        try_write_sample_f64(&mut pixel, format.sample, i, if format.alpha && i == format.mode.color_channels() { 0.75 } else { value }).unwrap();
    }
    let bytes = pixel.repeat(bounds.width() as usize * bounds.height() as usize);
    let descriptor = NativePlaneDescriptor::new(format, bounds, &pixel).unwrap();
    from_bytes(descriptor, bytes, q)
}
fn from_bytes(descriptor: NativePlaneDescriptor, bytes: Vec<u8>, q: &BackingQuota) -> (BackedLayer, Arc<Store>) {
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    let store = Arc::new(Store {
        descriptor,
        bytes,
        quota: q.clone(),
        reads: AtomicU64::new(0),
        fail: AtomicBool::new(false),
        fail_at: AtomicU64::new(u64::MAX),
        _lease: lease,
    });
    let backed = BackedRaster::new(store.clone(), q.clone(), 1, &ctl()).unwrap();
    (BackedLayer::new(backed, descriptor, &ctl()).unwrap(), store)
}
#[test]
fn disabled_mask_and_zero_opacity_do_not_read_unused_samples() {
    let q = quota();
    let other = quota();
    let (source, ss) = layer(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(0, 0, 3, 2), 0.5, &q);
    let (bad_mask, ms) = layer(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(-1, -1, 1, 1), 0.25, &other);
    ms.fail.store(true, Ordering::SeqCst);
    let p = profile();
    let prepared = PreparedBackedNormal::new(&source, &p, SampleEncoding::NonlinearProfile, source.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    let disabled = NormalMask { layer: &bad_mask, enabled: false, outside_coverage: f64::NAN };
    let bg = vec![[0.25, 0.5, 0.75, 0.125]; 6];
    let actual = prepared.render_band_over_with_coverage(source.operative_bounds(), &bg, 0.5, Some(disabled), &ctl()).unwrap();
    assert_eq!(ms.reads.load(Ordering::SeqCst), 0);
    assert_eq!(actual.pixels(), prepared.render_band_over_with_coverage(source.operative_bounds(), &bg, 0.5, None, &ctl()).unwrap().pixels());
    ss.fail.store(true, Ordering::SeqCst);
    let before = ss.reads.load(Ordering::SeqCst);
    let output = prepared.render_band_over_with_coverage(source.operative_bounds(), &bg, 0.0, Some(disabled), &ctl()).unwrap();
    assert_eq!(output.pixels(), bg);
    assert_eq!(ss.reads.load(Ordering::SeqCst), before);
    assert_eq!(ms.reads.load(Ordering::SeqCst), 0);
    let (enabled_mask, enabled_store) = layer(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), Rect::new(0, 0, 3, 2), 0.25, &q);
    enabled_store.fail.store(true, Ordering::SeqCst);
    let zero = prepared
        .render_band_over_with_coverage(
            source.operative_bounds(),
            &bg,
            0.0,
            Some(NormalMask { layer: &enabled_mask, enabled: true, outside_coverage: 0.0 }),
            &ctl(),
        )
        .unwrap();
    assert_eq!(zero.pixels(), bg);
    assert_eq!(enabled_store.reads.load(Ordering::SeqCst), 0);
    assert_eq!(ss.reads.load(Ordering::SeqCst), before);
    for opacity in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let usage = q.usage().unwrap();
        assert!(prepared.render_band_with_coverage(source.operative_bounds(), [0.0; 4], opacity, None, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), usage);
    }
}
#[test]
fn enabled_mask_layout_ledger_coverage_and_sample_errors_fail_closed() {
    let q = quota();
    let (source, _) = layer(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(0, 0, 2, 1), 0.5, &q);
    let p = profile();
    let prepared = PreparedBackedNormal::new(&source, &p, SampleEncoding::NonlinearProfile, source.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
    for format in [PixelFormat::new(ColorMode::Rgb, SampleType::F64, false), PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true)] {
        let (mask, store) = layer(format, Rect::new(0, 0, 2, 1), 0.25, &q);
        assert!(matches!(
            prepared.render_band_with_coverage(
                source.operative_bounds(),
                [0.0; 4],
                1.0,
                Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 }),
                &ctl()
            ),
            Err(BackedNormalError::Unsupported(_))
        ));
        assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    }
    let (mask, store) = layer(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), Rect::new(0, 0, 2, 1), 0.25, &quota());
    assert!(
        prepared
            .render_band_with_coverage(
                source.operative_bounds(),
                [0.0; 4],
                1.0,
                Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 }),
                &ctl()
            )
            .is_err()
    );
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    for value in [f64::NAN, f64::INFINITY, -0.125, 1.125] {
        let (mask, _) = layer(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), Rect::new(0, 0, 2, 1), 0.25, &q);
        let mask = mask.try_edited_native(Rect::new(1, 0, 2, 1), &value.to_ne_bytes(), &ctl()).unwrap();
        assert!(
            prepared
                .render_band_with_coverage(
                    source.operative_bounds(),
                    [0.0; 4],
                    1.0,
                    Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 }),
                    &ctl()
                )
                .is_err()
        );
    }
}
#[test]
fn late_mask_io_work_cancel_and_output_quota_return_no_band_and_refund() {
    let q = quota();
    {
        let (source, ss) = layer(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(0, 0, 260, 1), 0.5, &q);
        let (mask, ms) = layer(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), Rect::new(0, 0, 260, 1), 0.25, &q);
        ms.fail_at.store(256 * 8, Ordering::SeqCst);
        let before = ss.bytes.clone();
        let p = profile();
        let prepared =
            PreparedBackedNormal::new(&source, &p, SampleEncoding::NonlinearProfile, source.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
        let m = Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 });
        assert!(prepared.render_band_with_coverage(source.operative_bounds(), [0.0; 4], 1.0, m, &ctl()).is_err());
        assert!(ms.reads.load(Ordering::SeqCst) >= 2);
        assert_eq!(ss.bytes, before);
        let baseline = q.usage().unwrap();
        let available = (64 << 20) - baseline.resident;
        let held = q.reserve(QuotaKind::Resident, available).unwrap();
        assert!(prepared.render_band_with_coverage(source.operative_bounds(), [0.0; 4], 1.0, m, &ctl()).is_err());
        drop(held);
        assert_eq!(q.usage().unwrap(), baseline);
        let calls = AtomicU64::new(0);
        let deny = |_: u64| if calls.fetch_add(1, Ordering::SeqCst) > 600 { Err(BackingError::Work) } else { Ok(()) };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
        assert!(prepared.render_band_with_coverage(source.operative_bounds(), [0.0; 4], 1.0, m, &c).is_err());
        // Failed cache fill may evict a derived tile before work denial. It may
        // release resident payload; it cannot retain the unpublished band.
        assert!(q.usage().unwrap().resident <= baseline.resident);
        assert_eq!(q.usage().unwrap().index, baseline.index);
        let polls = AtomicU64::new(0);
        let cancelled = || polls.fetch_add(1, Ordering::SeqCst) > 650;
        let interrupt = Interrupt::cancel_only(&cancelled);
        let c = ReadControl { interrupt: &interrupt, work: &work };
        assert!(prepared.render_band_with_coverage(source.operative_bounds(), [0.0; 4], 1.0, m, &c).is_err());
        assert!(q.usage().unwrap().resident <= baseline.resident);
        assert_eq!(q.usage().unwrap().index, baseline.index);
    }
    assert_eq!(q.usage().unwrap().resident, 0);
    assert_eq!(q.usage().unwrap().index, 0);
}
#[test]
fn current_wide_bits_history_and_nonzero_defaults_are_not_rewritten() {
    for sample in [SampleType::U32, SampleType::F64] {
        let q = quota();
        let (source, store) = layer(PixelFormat::new(ColorMode::Rgb, sample, true), Rect::new(-1, 0, 1, 1), 0.5, &q);
        let history = source.clone();
        let mut raw = source.default_bytes().to_vec();
        if sample == SampleType::U32 {
            raw[..4].copy_from_slice(&16_777_217u32.to_ne_bytes());
        } else {
            raw[..8].copy_from_slice(&(-0.0f64).to_ne_bytes());
        }
        let current = source.try_edited_native(Rect::new(-1, 0, 0, 1), &raw, &ctl()).unwrap();
        let (mask, _) = layer(PixelFormat::new(ColorMode::Grayscale, sample, false), Rect::new(-1, 0, 0, 1), 0.5, &q);
        let p = profile();
        let prepared =
            PreparedBackedNormal::new(&current, &p, SampleEncoding::NonlinearProfile, current.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
        let _output = prepared
            .render_band_with_coverage(Rect::new(-2, 0, 2, 1), [0.0; 4], 0.75, Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.25 }), &ctl())
            .unwrap();
        let mut actual = vec![0; raw.len()];
        current.checked_view().read_logical_pixel_native_into(-1, 0, &mut actual, &ctl()).unwrap();
        assert_eq!(actual, raw);
        history.checked_view().read_logical_pixel_native_into(-1, 0, &mut actual, &ctl()).unwrap();
        assert_eq!(actual, source.default_bytes());
        assert_eq!(store.bytes, source.default_bytes().repeat(2));
        assert_eq!(current.default_bytes(), source.default_bytes());
    }
}
fn profile() -> Profile {
    photocraft_cms::xcf_color::effective_profile_from_definition(Builtin::Srgb.profile(), encoding(SampleEncoding::NonlinearProfile)).unwrap()
}
#[test]
fn independent_negative_mask_extent_and_explicit_outside_match_checked_math() {
    for sample in SampleType::ALL {
        let q = quota();
        let (source, store) = layer(PixelFormat::new(ColorMode::Rgb, sample, true), Rect::new(-2, -1, 3, 2), 0.5, &q);
        let (mask, _) = layer(PixelFormat::new(ColorMode::Grayscale, sample, false), Rect::new(-1, -2, 1, 1), 0.25, &q);
        let original = store.bytes.clone();
        let p = profile();
        let prepared =
            PreparedBackedNormal::new(&source, &p, SampleEncoding::NonlinearProfile, source.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
        let rect = Rect::new(-3, -2, 4, 3);
        let bg = [0.125, 0.25, 0.75, 0.5];
        let output =
            prepared.render_band_with_coverage(rect, bg, 0.5, Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.75 }), &ctl()).unwrap();
        let working = WorkingSpace::new(&p, SampleEncoding::NonlinearProfile, space(SampleEncoding::NonlinearProfile)).unwrap();
        for (i, (x, y)) in (rect.y0..rect.y1).flat_map(|y| (rect.x0..rect.x1).map(move |x| (x, y))).enumerate() {
            let expected = if source.operative_bounds().contains(x, y) {
                let mut s = [0.0; 4];
                source.checked_view().read_logical_pixel_f64_into(x, y, &mut s, &ctl()).unwrap();
                let coverage = if mask.operative_bounds().contains(x, y) {
                    let mut m = [0.0];
                    mask.checked_view().read_logical_pixel_f64_into(x, y, &mut m, &ctl()).unwrap();
                    m[0]
                } else {
                    0.75
                };
                working.normal(bg, s, 0.5 * coverage, GimpOperator::Union).unwrap()
            } else {
                bg
            };
            assert_eq!(output.pixels()[i], expected);
        }
        assert_eq!(store.bytes, original);
    }
}

#[test]
fn all_depths_rgb_gray_match_existing_resident_checked_mask_consumer() {
    use photocraft_doc::{Document, Layer, LayerContent, LayerMask, Size, WorkingProfileInterpretation};
    use photocraft_raster::Surface;
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let q = quota();
            let rect = Rect::new(0, 0, 3, 2);
            let format = PixelFormat::new(mode, sample, true);
            let (source, ss) = layer(format, rect, 0.5, &q);
            let (mask, ms) = layer(PixelFormat::new(ColorMode::Grayscale, sample, false), rect, 0.25, &q);
            let builtin = if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::SGray };
            let p = photocraft_cms::xcf_color::effective_profile_from_definition(builtin.profile(), encoding(SampleEncoding::NonlinearProfile)).unwrap();
            let prepared =
                PreparedBackedNormal::new(&source, &p, SampleEncoding::NonlinearProfile, source.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
            let actual = prepared
                .render_band_with_coverage(rect, [0.0; 4], 0.5, Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 }), &ctl())
                .unwrap();
            let mut doc = Document::new("Resident mask oracle", Size::new(3, 2), mode, sample);
            doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
            let icc = p.to_bytes();
            doc.working_profile_interpretation =
                if mode == ColorMode::Rgb { WorkingProfileInterpretation::implicit_srgb(&icc) } else { WorkingProfileInterpretation::implicit_sgray(&icc) };
            doc.icc_profile = Some(icc);
            let mut owner = Layer::raster("Owner", format);
            owner.content = LayerContent::Raster(Surface::try_from_interleaved(format, rect, &ss.bytes).unwrap());
            owner.opacity = 0.5;
            owner.mask = Some(LayerMask {
                surface: std::sync::Arc::new(Surface::try_from_interleaved(ms.descriptor.format(), rect, &ms.bytes).unwrap()),
                enabled: true,
                linked: false,
                density: 1.0,
                feather: 0.0,
            });
            doc.layers.push(owner);
            let expected = photocraft_compose::gimp_render::try_render(&doc, rect, &mut |_| Ok(())).unwrap();
            assert_eq!(actual.pixels(), expected.px);
        }
    }
}

#[derive(Debug)]
struct Generated {
    descriptor: NativePlaneDescriptor,
}
impl NativePlaneStore for Generated {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        let pixel = self.descriptor.default_bytes();
        for (i, b) in out.iter_mut().enumerate() {
            *b = pixel[((offset + i as u64) % pixel.len() as u64) as usize];
        }
        Ok(())
    }
}
#[test]
#[ignore = "release-only generated 24MP backed mask/opacity consumer"]
fn release_24mp_masked_bands() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let q = quota();
    let bounds = Rect::new(0, 0, 6000, 4000);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, bounds, &[64, 128, 192, 128]).unwrap();
    let source = BackedLayer::new(BackedRaster::new(Arc::new(Generated { descriptor }), q.clone(), 2, &ctl()).unwrap(), descriptor, &ctl()).unwrap();
    let md = NativePlaneDescriptor::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), bounds, &[128]).unwrap();
    let mask = BackedLayer::new(BackedRaster::new(Arc::new(Generated { descriptor: md }), q.clone(), 2, &ctl()).unwrap(), md, &ctl()).unwrap();
    let p = profile();
    let prepared = PreparedBackedNormal::new(&source, &p, SampleEncoding::NonlinearProfile, descriptor, NormalBandLimits::default(), &ctl()).unwrap();
    let expected = WorkingSpace::new(&p, SampleEncoding::NonlinearProfile, space(SampleEncoding::NonlinearProfile))
        .unwrap()
        .normal([0.0; 4], [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 128.0 / 255.0], 0.5 * 128.0 / 255.0, GimpOperator::Union)
        .unwrap();
    let started = std::time::Instant::now();
    let mut visited = 0;
    let mut sampled = 0;
    for y in (0..4000).step_by(256) {
        for x in (0..6000).step_by(256) {
            let rect = Rect::new(x, y, (x + 256).min(6000), (y + 256).min(4000));
            let band = prepared
                .render_band_with_coverage(rect, [0.0; 4], 0.5, Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 }), &ctl())
                .unwrap();
            assert!(band.pixels().iter().all(|p| *p == expected));
            visited += band.pixels().len();
            sampled = sampled.max(q.usage().unwrap().resident);
        }
    }
    assert_eq!(visited, 24_000_000);
    println!(
        "24MP generated RGBA8 + Gray8 mask, opacity0.5, tile-sized bands/two-tile caches: {:?}; sampled retained after returned bands {sampled}B (not transient peak/RSS)",
        started.elapsed()
    );
    drop(prepared);
    drop((source, mask));
    assert_eq!(q.usage().unwrap().resident, 0);
    Ok(())
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires reviewed unpublished Normal-mask analytic original packet"]
fn independently_authored_analytic_normal_union_masks_match_native_projection() {
    use photocraft_doc::import_compositing::GimpSpace;
    use sha2::{Digest, Sha256};
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("explicit reviewed corpus root"));
    let all_hashes: serde_json::Value = serde_json::from_str(include_str!("gimp_normal_mask_analytic_class_hashes.json")).unwrap();
    let mut count = 0;
    for variant in ["gamma", "type3", "type4"] {
        let root = root.join("normal-mask-analytic-class-evidence").join(variant);
        let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
        for (name, expected) in all_hashes[variant].as_object().unwrap() {
            assert_eq!(hash(&fs::read(root.join(name)).unwrap()), expected.as_str().unwrap());
        }
        let data: serde_json::Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
        let read = |value: &serde_json::Value| {
            let bytes = fs::read(root.join(value["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&bytes), value["sha256"].as_str().unwrap());
            assert_eq!(bytes.len() as u64, value["bytes"].as_u64().unwrap());
            bytes
        };
        for row in data["records"].as_array().unwrap() {
            let layers = row["layers"].as_array().unwrap();
            let top = &layers[0];
            if top["spaces"]["composite_mode"] != 1 {
                continue;
            }
            assert_eq!(top["spaces"]["composite_space"], 1);
            let wide = row["id"].as_str().unwrap().contains("f64");
            let sample = if wide { SampleType::F64 } else { SampleType::F32 };
            let gray = row["model"] == "gray";
            let mode = if gray { ColorMode::Grayscale } else { ColorMode::Rgb };
            let format = PixelFormat::new(mode, sample, true);
            let native = |bytes: Vec<u8>| {
                if wide {
                    bytes.as_chunks::<8>().0.iter().flat_map(|p| f64::from_le_bytes(*p).to_ne_bytes()).collect::<Vec<_>>()
                } else {
                    bytes.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect::<Vec<_>>()
                }
            };
            let bounds = |l: &serde_json::Value| {
                Rect::from_xywh(
                    l["offset"][0].as_i64().unwrap() as i32,
                    l["offset"][1].as_i64().unwrap() as i32,
                    l["size"][0].as_u64().unwrap() as u32,
                    l["size"][1].as_u64().unwrap() as u32,
                )
            };
            let q = quota();
            let source = |l: &serde_json::Value, format: PixelFormat| {
                let bytes = native(read(&l["native"]));
                let descriptor = NativePlaneDescriptor::new(format, bounds(l), &vec![0; format.bytes_per_pixel()]).unwrap();
                from_bytes(descriptor, bytes, &q)
            };
            let (bottom, _) = source(&layers[1], format);
            let (owner, os) = source(top, format);
            let (mask, ms) = source(&top["mask"], PixelFormat::new(ColorMode::Grayscale, sample, false));
            let original = os.bytes.clone();
            let originalmask = ms.bytes.clone();
            let p = Profile::parse(&read(&row["assigned_icc"])).unwrap();
            let canvas = Rect::new(0, 0, 16, 12);
            let bottomprepared =
                PreparedBackedNormal::new(&bottom, &p, SampleEncoding::NonlinearProfile, bottom.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
            let bg = bottomprepared.render_band_with_coverage(canvas, [0.0; 4], layers[1]["opacity"].as_f64().unwrap() / 100.0, None, &ctl()).unwrap();
            let prepared = PreparedBackedNormal::new_with_composite_space(
                &owner,
                &p,
                SampleEncoding::NonlinearProfile,
                GimpSpace::LinearRgb,
                owner.descriptor(),
                NormalBandLimits::default(),
                &ctl(),
            )
            .unwrap();
            let out = prepared
                .render_band_over_with_coverage(
                    canvas,
                    bg.pixels(),
                    top["opacity"].as_f64().unwrap() / 100.0,
                    Some(NormalMask { layer: &mask, enabled: true, outside_coverage: 0.0 }),
                    &ctl(),
                )
                .unwrap();
            let reference = native(read(&row["projection"]["native"]));
            let channels = format.channels();
            let mut error = 0.0f64;
            for (i, pixel) in out.pixels().iter().enumerate() {
                let raw = &reference[i * format.bytes_per_pixel()..(i + 1) * format.bytes_per_pixel()];
                let alpha = photocraft_color::try_read_sample_f64(raw, sample, channels - 1).unwrap();
                error = error.max((pixel[3] - alpha).abs());
                for c in 0..mode.color_channels() {
                    let value = photocraft_color::try_read_sample_f64(raw, sample, c).unwrap();
                    if alpha > 0.0 {
                        error = error.max((pixel[c] - value).abs());
                    }
                    error = error.max((pixel[c] * pixel[3] - value * alpha).abs());
                }
            }
            assert!(error <= 2.0 / 255.0, "{}: native error {error}", row["id"]);
            assert_eq!(os.bytes, original);
            assert_eq!(ms.bytes, originalmask);
            count += 1;
        }
    }
    assert!(count > 0);
    println!("qualified exact analytic Normal/Union mask references: {count}");
}
