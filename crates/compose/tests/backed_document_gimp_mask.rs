#![cfg(feature = "corpus")]
//! Independent GIMP native-plane controls with an explicitly retained synthetic
//! source-model envelope. Real XCF/native codec transfer is a separate IO gate.
use photocraft_compose::{backed_document::PreparedBackedDocument, backed_normal::NormalBandLimits};
use photocraft_doc::{import_compositing::*, item_identity::*, xcf_state::*, *};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{quota::*, *},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("normal-mask-ordinary-scene-evidence/medium")
}
fn read(root: &Path, value: &Value) -> Vec<u8> {
    let b = fs::read(root.join(value["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&b), value["sha256"]);
    b
}
fn bounds(row: &Value) -> Rect {
    let x = row["offset"][0].as_i64().unwrap() as i32;
    let y = row["offset"][1].as_i64().unwrap() as i32;
    Rect::new(x, y, x + row["size"][0].as_i64().unwrap() as i32, y + row["size"][1].as_i64().unwrap() as i32)
}
#[derive(Debug)]
struct Plane {
    bytes: Vec<u8>,
    descriptor: NativePlaneDescriptor,
    _lease: QuotaLease,
}
impl NativePlaneStore for Plane {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        let a = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn fixture(root: &Path, row: &Value) -> (photocraft_doc::accounted_document::AccountedDocument, BackingQuota, photocraft_cms::Profile) {
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 4 << 20, scratch: 0 });
    // Fixed bounded synthetic source/model retention; not a codec/RSS/map census.
    let retention = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mode = if row["model"] == "gray" { ColorMode::Grayscale } else { ColorMode::Rgb };
    let depth = match row["sample_type"].as_str().unwrap() {
        "u16" => SampleType::U16,
        "f32" => SampleType::F32,
        "f64" => SampleType::F64,
        _ => panic!("outside this oracle slice"),
    };
    let mut doc = Document::new("qualified ordinary scene", Size::new(513, 385), mode, depth);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let icc = read(root, &row["assigned_icc"]);
    let profile = photocraft_cms::Profile::parse(&icc).unwrap();
    doc.icc_profile = Some(Arc::new(icc));
    let mut state = XcfState::default();
    state.provenance.format_version = 12;
    for (index, r) in row["layers"].as_array().unwrap().iter().enumerate().rev() {
        let extent = bounds(r);
        let format = doc.pixel_format();
        let source_charge = q.reserve(QuotaKind::Resident, r["native"]["bytes"].as_u64().unwrap() + 256).unwrap();
        let mut bytes = read(root, &r["native"]);
        if cfg!(target_endian = "big") {
            for chunk in bytes.chunks_exact_mut(depth.bytes()) {
                chunk.reverse();
            }
        }
        let descriptor = NativePlaneDescriptor::new(format, extent, &vec![0; format.bytes_per_pixel()]).unwrap();
        let store = Arc::new(Plane { bytes, descriptor, _lease: source_charge });
        let backed = BackedRaster::new(store, q.clone(), 8, &ctl()).unwrap();
        let mut layer = Layer::new(r["name"].as_str().unwrap(), LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &ctl()).unwrap()));
        layer.opacity = (r["opacity"].as_f64().unwrap() / 100.) as f32;
        if index == 0 {
            let mask = &r["mask"];
            let mut maskbytes = read(root, &mask["native"]);
            if cfg!(target_endian = "big") {
                for chunk in maskbytes.chunks_exact_mut(depth.bytes()) {
                    chunk.reverse();
                }
            }
            let maskrect = bounds(mask);
            let surface = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, depth, false), maskrect, &maskbytes).unwrap();
            layer.mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
            state.items.push(ItemState {
                item: ItemRef::Mask(MaskId(1)),
                name: "own mask".into(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: None,
                composition: None,
                mask: Some(MaskState {
                    owner: ItemRef::Layer(layer.id),
                    bounds: maskrect,
                    offset: [maskrect.x0, maskrect.y0],
                    outside_f64_bits: 0.0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: true,
                }),
            });
        }
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(extent),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH.into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: if index == 0 { 1 } else { -1 },
                stored_operator: if index == 0 { 1 } else { -1 },
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                }),
                pass_through: false,
            }),
        });
        doc.layers.push(layer);
    }
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    (photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new(retention), q.clone(), &ctl()).unwrap(), q, profile)
}

fn data(root: &Path) -> Value {
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("gimp_normal_mask_ordinary_scene_hashes.json")).unwrap();
    assert_eq!(hash(&bytes), hashes["medium"]["observations.json"]);
    serde_json::from_slice(&bytes).unwrap()
}
fn native_values(bytes: &[u8], sample: SampleType) -> Vec<f64> {
    bytes
        .chunks_exact(sample.bytes())
        .map(|v| match sample {
            SampleType::U16 => f64::from(u16::from_le_bytes(v.try_into().unwrap())) / 65535.,
            SampleType::F32 => f64::from(f32::from_le_bytes(v.try_into().unwrap())),
            SampleType::F64 => f64::from_le_bytes(v.try_into().unwrap()),
            _ => panic!("outside oracle"),
        })
        .collect()
}
#[test]
fn retained_gimp_medium_ordinary_masks_match_native_and_display_for_rgb_gray_three_depths() {
    let root = root();
    let data = data(&root);
    for row in data["records"].as_array().unwrap().iter().filter(|r| r["id"].as_str().unwrap().ends_with("op1-source") && r["sample_type"] != "u32") {
        let (owner, q, profile) = fixture(&root, row);
        let original_state = owner.document().xcf_state.clone();
        let original_mask = owner.document().layers[1].mask.as_ref().unwrap().surface.clone();
        assert!(
            PreparedBackedDocument::new(owner.document(), &profile, NormalBandLimits::default(), &ctl()).is_err(),
            "raw borrow is not paid mask-source admission"
        );
        let prepared = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
        let rect = Rect::new(0, 0, 128, 64);
        let output = prepared.render_band(rect, &ctl()).unwrap();
        let channels = if row["model"] == "gray" { 1 } else { 3 };
        let sample = owner.document().depth;
        let reference = native_values(&read(&root, &row["projection"]["native"]), sample);
        let display = read(&root, &row["projection"]["nonlinear_double"]);
        let target = if channels == 1 { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let transform = photocraft_cms::Transform::new(&profile, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
        for (index, pixel) in output.pixels().iter().enumerate() {
            let x = index % 128;
            let y = index / 128;
            let offset = (y * 513 + x) * (channels + 1);
            let source = &reference[offset..offset + channels + 1];
            assert!((pixel[3] - source[channels]).abs() <= 1. / 65535., "{} alpha", row["id"]);
            let mut converted = [0.; 3];
            transform.try_eval_f64(&pixel[..channels], &mut converted[..channels]).unwrap();
            for c in 0..channels {
                assert!((pixel[c] * pixel[3] - source[c] * source[channels]).abs() <= 2. / 255., "{} native", row["id"]);
                let b = (offset + c) * 8;
                let expected = f64::from_le_bytes(display[b..b + 8].try_into().unwrap());
                assert!((converted[c] * pixel[3] - expected * source[channels]).abs() <= 2. / 255., "{} display", row["id"]);
            }
        }
        let held = q.usage().unwrap();
        let reduced = prepared.render_reduced(8, 8, &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident, "old+new derived buffers charged concurrently");
        // Independently reduce the GIMP native projection, using rectangular box cells.
        for (i, pixel) in reduced.pixels().iter().enumerate() {
            let x = i % 8;
            let y = i / 8;
            let (x0, x1) = (x * 513 / 8, (x + 1) * 513 / 8);
            let (y0, y1) = (y * 385 / 8, (y + 1) * 385 / 8);
            let mut sum = [0.; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let b = (sy * 513 + sx) * (channels + 1);
                    let alpha = reference[b + channels];
                    sum[3] += alpha;
                    for c in 0..channels {
                        sum[c] += reference[b + c] * alpha;
                    }
                }
            }
            let area = ((x1 - x0) * (y1 - y0)) as f64;
            assert!((f64::from(pixel[3]) - sum[3] / area).abs() <= 1. / 65535.);
            for c in 0..channels {
                assert!((f64::from(pixel[c]) * f64::from(pixel[3]) - sum[c] / area).abs() <= 2. / 255., "{} reduction", row["id"]);
            }
        }
        assert_eq!(owner.document().xcf_state, original_state);
        assert!(Arc::ptr_eq(&original_mask, &owner.document().layers[1].mask.as_ref().unwrap().surface));
        drop(reduced);
        drop(output);
        drop(prepared);
        drop(original_mask);
        drop(owner);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn resident_mask_render_cancellation_quota_and_old_new_frames_are_atomic() {
    let root = root();
    let data = data(&root);
    let row = data["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-f64-op1-source").unwrap();
    let (owner, q, profile) = fixture(&root, row);
    let prepared = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
    let rect = Rect::new(0, 0, 1, 1);
    let old = prepared.render_band(rect, &ctl()).unwrap();
    let old_pixels = old.pixels().to_vec();
    let baseline = q.usage().unwrap();
    let deny = q.reserve(QuotaKind::Resident, (128 << 20) - baseline.resident).unwrap();
    assert!(matches!(prepared.render_band(rect, &ctl()), Err(photocraft_compose::backed_normal::BackedNormalError::Quota(_))));
    drop(deny);
    assert_eq!(q.usage().unwrap(), baseline);
    let steps = std::sync::atomic::AtomicU64::new(0);
    let work = |_| {
        steps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    };
    let next = prepared.render_band(rect, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    assert!(q.usage().unwrap().resident > baseline.resident);
    drop(next);
    assert_eq!(q.usage().unwrap(), baseline);
    for stop in 0..steps.load(std::sync::atomic::Ordering::Relaxed) {
        let step = std::sync::atomic::AtomicU64::new(0);
        let cancel = |_| if step.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == stop { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(prepared.render_band(rect, &ReadControl { interrupt: &Interrupt::NONE, work: &cancel }).is_err());
        assert_eq!(old.pixels(), old_pixels);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    drop(old);
    drop(prepared);
    drop(owner);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn other_source_contexts_and_failed_mask_samples_never_publish_partial_backdrops() {
    let root = root();
    let data = data(&root);
    let row = data["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-f64-op1-source").unwrap();
    let (owner, q, profile) = fixture(&root, row);
    // Every adversarial model copy below has a separate prepaid model envelope.
    for mutation in 0..6 {
        let copy = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
        let mut doc = owner.document().clone();
        match mutation {
            0 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().compatibility_epoch = "unqualified".into(),
            1 => {
                doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().resolved.as_mut().unwrap().operator =
                    GimpOperator::ClipToBackdrop
            }
            2 => doc.layers[1].mask.as_mut().unwrap().density = 0.5,
            3 => doc.layers[1].mask.as_mut().unwrap().enabled = false,
            4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap().offset = [0, 0],
            _ => doc.layers[1].content = LayerContent::Group(Group { children: vec![], expanded: true, artboard: None }),
        }
        let altered = photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new((owner.clone(), copy)), q.clone(), &ctl()).unwrap();
        let before = q.usage().unwrap();
        assert!(PreparedBackedDocument::new_accounted(&altered, &profile, NormalBandLimits::default(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        drop(altered);
    }
    let warm = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
    let band = warm.render_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap();
    drop(band);
    drop(warm);
    let copy = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mask_default = q.reserve(QuotaKind::Resident, 1024).unwrap();
    let mut doc = owner.document().clone();
    doc.layers[1].mask.as_mut().unwrap().surface =
        Arc::new(Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &f64::NAN.to_ne_bytes()).unwrap());
    let altered =
        photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new((owner.clone(), copy, mask_default)), q.clone(), &ctl()).unwrap();
    let prepared = PreparedBackedDocument::new_accounted(&altered, &profile, NormalBandLimits::default(), &ctl()).unwrap();
    let before = q.usage().unwrap();
    assert!(prepared.render_band(Rect::new(0, 0, 1, 1), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(prepared);
    drop(altered);
    drop(owner);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn borrowed_resident_mask_sparse_default_world_offset_and_explicit_outside_match_equation() {
    use photocraft_compose::backed_normal::{PreparedBackedNormal, ResidentNormalMask};
    let root = root();
    let data = data(&root);
    let row = data["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-f64-op1-source").unwrap();
    let (owner, q, profile) = fixture(&root, row);
    let LayerContent::BackedRaster(layer) = &owner.document().layers[1].content else { panic!() };
    let mask_guard = q.reserve(QuotaKind::Resident, 1024).unwrap();
    let surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false), &0x8000u16.to_ne_bytes()).unwrap();
    // A one-cell mask extent inside the negatively offset GIMP source owner.
    let view = photocraft_raster::checked_view::CheckedRasterView::resident_with_control(&surface, Rect::new(0, 0, 1, 1), &ctl()).unwrap();
    let prepared = PreparedBackedNormal::new_with_composite_space(
        layer,
        &profile,
        SampleEncoding::NonlinearProfile,
        GimpSpace::LinearRgb,
        layer.descriptor(),
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap();
    let output = prepared
        .render_band_with_resident_coverage(
            Rect::new(0, 0, 2, 1),
            [0.; 4],
            0.625,
            Some(ResidentNormalMask { view, enabled: true, outside_coverage: 0.25 }),
            &ctl(),
        )
        .unwrap();
    for (x, pixel) in output.pixels().iter().enumerate() {
        let mut bytes = [0; 32];
        layer.checked_view().read_pixel_native_into(x as i32, 0, &mut bytes, &ctl()).unwrap();
        let native: Vec<_> = bytes.as_chunks::<8>().0.iter().map(|b| f64::from_ne_bytes(*b)).collect();
        let coverage = if x == 0 { f64::from(0x8000u16) / 65535. } else { 0.25 };
        assert!((pixel[3] - native[3] * 0.625 * coverage).abs() < 1e-14);
        for c in 0..3 {
            if pixel[3] > 0. {
                assert!((pixel[c] - native[c]).abs() < 1e-13);
            }
        }
    }
    assert_eq!(surface.default_bytes(), 0x8000u16.to_ne_bytes());
    drop(output);
    drop(prepared);
    drop(surface);
    drop(mask_guard);
    drop(owner);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn accounted_mask_preparation_every_poll_is_cancel_safe_and_current_icc_is_required() {
    let root = root();
    let data = data(&root);
    let row = data["records"].as_array().unwrap().iter().find(|r| r["id"] == "gray-f64-op1-source").unwrap();
    let (owner, q, profile) = fixture(&root, row);
    let before = q.usage().unwrap();
    assert!(PreparedBackedDocument::new_accounted(&owner, photocraft_cms::Builtin::SGray.profile(), NormalBandLimits::default(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let steps = std::sync::atomic::AtomicU64::new(0);
    let count = |_| {
        steps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    };
    let prepared =
        PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ReadControl { interrupt: &Interrupt::NONE, work: &count })
            .unwrap();
    drop(prepared);
    assert_eq!(q.usage().unwrap(), before);
    for stop in 0..steps.load(std::sync::atomic::Ordering::Relaxed) {
        let step = std::sync::atomic::AtomicU64::new(0);
        let cancel = |_| if step.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == stop { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(
            PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ReadControl { interrupt: &Interrupt::NONE, work: &cancel })
                .is_err()
        );
        assert_eq!(q.usage().unwrap(), before);
    }
    drop(owner);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
