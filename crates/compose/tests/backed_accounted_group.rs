#![cfg(feature = "corpus")]
//! Independent original GIMP isolated-group planes under explicit synthetic ownership.
//! Real importer/native codec integration remains a distinct gate.
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
        .join("accounted-normal-group-evidence")
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

const EPOCH: &str = "own-accounted-isolated-normal-group-v1";
fn layer(root: &Path, r: &Value, doc: &Document, q: &BackingQuota, state: &mut XcfState, parent: Option<LayerId>, mask_id: &mut u64) -> Layer {
    let extent = bounds(r);
    let mut result = if let Some(children) = r.get("children") {
        let mut group = Layer::group(r["name"].as_str().unwrap(), Vec::new());
        let nested: Vec<_> = children.as_array().unwrap().iter().rev().map(|child| layer(root, child, doc, q, state, Some(group.id), mask_id)).collect();
        if let LayerContent::Group(content) = &mut group.content {
            content.children = nested;
        }
        group
    } else {
        let charge = q.reserve(QuotaKind::Resident, r["native"]["bytes"].as_u64().unwrap() + 256).unwrap();
        let mut bytes = read(root, &r["native"]);
        if cfg!(target_endian = "big") {
            for sample in bytes.chunks_exact_mut(doc.depth.bytes()) {
                sample.reverse();
            }
        }
        let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), extent, &vec![0; doc.pixel_format().bytes_per_pixel()]).unwrap();
        let store = Arc::new(Plane { bytes, descriptor, _lease: charge });
        let raster = BackedRaster::new(store, q.clone(), 8, &ctl()).unwrap();
        Layer::new(r["name"].as_str().unwrap(), LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()))
    };
    result.blend = BlendMode::Normal;
    result.visible = r["visible"].as_bool().unwrap();
    result.opacity = (r["opacity"].as_f64().unwrap() / 100.) as f32;
    if !r["mask"].is_null() {
        let mask = &r["mask"];
        let rect = bounds(mask);
        let mut bytes = read(root, &mask["native"]);
        if cfg!(target_endian = "big") {
            for sample in bytes.chunks_exact_mut(doc.depth.bytes()) {
                sample.reverse();
            }
        }
        let enabled = mask["enabled"].as_bool().unwrap();
        result.mask = Some(LayerMask {
            surface: Arc::new(Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, doc.depth, false), rect, &bytes).unwrap()),
            enabled,
            linked: true,
            density: 1.,
            feather: 0.,
        });
        *mask_id += 1;
        state.items.push(ItemState {
            item: ItemRef::Mask(MaskId(*mask_id)),
            name: "Own group mask".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(rect),
            composition: None,
            mask: Some(MaskState {
                owner: ItemRef::Layer(result.id),
                bounds: rect,
                offset: [rect.x0, rect.y0],
                outside_f64_bits: 0f64.to_bits(),
                enabled,
                shown: false,
                editing: false,
                linked: true,
            }),
        });
    }
    state.items.push(ItemState {
        item: ItemRef::Layer(result.id),
        name: result.name.clone(),
        custom_name: false,
        visible: result.visible,
        locks: ItemLocks::default(),
        parent: parent.map(ItemRef::Layer),
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(extent),
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: EPOCH.into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 0,
            stored_composite_space: 1,
            stored_operator: 1,
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
            pass_through: false,
        }),
    });
    result
}
fn fixture(root: &Path, row: &Value) -> (photocraft_doc::accounted_document::AccountedDocument, BackingQuota, photocraft_cms::Profile) {
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 4 << 20, scratch: 0 });
    // Explicit bounded synthetic guard, not a claim to census opaque map nodes.
    let guard = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mode = if row["model"] == "gray" { ColorMode::Grayscale } else { ColorMode::Rgb };
    let depth = match row["sample_type"].as_str().unwrap() {
        "u16" => SampleType::U16,
        "f32" => SampleType::F32,
        "f64" => SampleType::F64,
        _ => panic!("oracle depth"),
    };
    let mut doc = Document::new("Own isolated Normal group", Size::new(17, 13), mode, depth);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let raw = read(root, &row["assigned_icc"]);
    let profile = photocraft_cms::Profile::parse(&raw).unwrap();
    doc.icc_profile = Some(Arc::new(raw));
    doc.source_color_context = Some(SourceColorContext {
        original_icc: doc.icc_profile.clone(),
        source_precision: match depth {
            SampleType::U16 => 250,
            SampleType::F32 => 650,
            SampleType::F64 => 750,
            _ => panic!("oracle depth"),
        },
    });
    let mut state = XcfState::default();
    // Retain the recorded source metadata in this synthetic Legacy owner;
    // this does not invoke or qualify an imported Source factory.
    let source = read(root, &row["source"]);
    state.provenance.format = "xcf".into();
    state.provenance.format_version = if source.starts_with(b"gimp xcf v013\0") {
        13
    } else {
        assert!(source.starts_with(b"gimp xcf v012\0"));
        12
    };
    let mut next = 0;
    doc.layers = row["layers"].as_array().unwrap().iter().rev().map(|r| layer(root, r, &doc, &q, &mut state, None, &mut next)).collect();
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    let owner = photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new(guard), q.clone(), &ctl()).unwrap();
    (owner, q, profile)
}
fn data(root: &Path) -> Value {
    let raw = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("gimp_accounted_group_hashes.json")).unwrap();
    assert_eq!(hash(&raw), hashes["observations.json"]);
    serde_json::from_slice(&raw).unwrap()
}
fn values(raw: &[u8], depth: SampleType) -> Vec<f64> {
    raw.chunks_exact(depth.bytes())
        .map(|p| match depth {
            SampleType::U16 => f64::from(u16::from_le_bytes(p.try_into().unwrap())) / 65535.,
            SampleType::F32 => f64::from(f32::from_le_bytes(p.try_into().unwrap())),
            SampleType::F64 => f64::from_le_bytes(p.try_into().unwrap()),
            _ => panic!("oracle"),
        })
        .collect()
}
#[test]
fn retained_isolated_normal_groups_match_independent_native_projection() {
    let root = root();
    let data = data(&root);
    for row in data["records"].as_array().unwrap() {
        let (owner, q, profile) = fixture(&root, row);
        let prepared = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
        let band = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
        let reference = values(&read(&root, &row["projection"]["native"]), owner.document().depth);
        let channels = if row["model"] == "gray" { 1 } else { 3 };
        let display = read(&root, &row["projection"]["nonlinear_double"]);
        let target = if channels == 1 { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let transform = photocraft_cms::Transform::new(&profile, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
        for (i, pixel) in band.pixels().iter().enumerate() {
            let expected = &reference[i * (channels + 1)..(i + 1) * (channels + 1)];
            assert!((pixel[3] - expected[channels]).abs() <= 2. / 65535., "{} alpha {} {}", row["id"], pixel[3], expected[channels]);
            let mut converted = [0.; 3];
            transform.try_eval_f64(&pixel[..channels], &mut converted[..channels]).unwrap();
            for c in 0..channels {
                assert!((pixel[c] * pixel[3] - expected[c] * expected[channels]).abs() <= 2. / 255., "{} pixel{i} c{c}", row["id"]);
                let offset = (i * (channels + 1) + c) * 8;
                let expected_display = f64::from_le_bytes(display[offset..offset + 8].try_into().unwrap());
                assert!((converted[c] * pixel[3] - expected_display * expected[channels]).abs() <= 2. / 255., "{} display{i} c{c}", row["id"]);
            }
        }
        let held = q.usage().unwrap();
        let reduced = prepared.render_reduced(3, 2, &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident);
        for (i, pixel) in reduced.pixels().iter().enumerate() {
            let x = i % 3;
            let y = i / 3;
            let mut total = [0.; 4];
            for sy in y * 13 / 2..(y + 1) * 13 / 2 {
                for sx in x * 17 / 3..(x + 1) * 17 / 3 {
                    let offset = (sy * 17 + sx) * (channels + 1);
                    let alpha = reference[offset + channels];
                    total[3] += alpha;
                    for c in 0..3 {
                        total[c] += reference[offset + if channels == 1 { 0 } else { c }] * alpha;
                    }
                }
            }
            let area = (((x + 1) * 17 / 3 - x * 17 / 3) * ((y + 1) * 13 / 2 - y * 13 / 2)) as f64;
            assert!((f64::from(pixel[3]) - total[3] / area).abs() <= 2. / 65535.);
            for c in 0..3 {
                assert!((f64::from(pixel[c]) * f64::from(pixel[3]) - total[c] / area).abs() <= 2. / 255.);
            }
        }
        drop(reduced);
        drop(band);
        drop(prepared);
        drop(owner);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn isolated_groups_keep_old_outputs_and_history_across_every_cancel_and_quota_denial() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = root();
    let data = data(&root);
    let row = data["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-f64-visible-mask").unwrap();
    let (owner, q, profile) = fixture(&root, row);
    let previous = owner.clone();
    let prepared = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
    let old = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
    let old_pixels = old.pixels().to_vec();
    let baseline = q.usage().unwrap();
    let count = AtomicUsize::new(0);
    let work = |_| {
        count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let next = prepared.render_band(Rect::new(0, 0, 4, 3), &control).unwrap();
    drop(next);
    let fences = count.load(Ordering::Relaxed);
    assert!(fences > 50);
    for stop in 1..=fences {
        count.store(0, Ordering::Relaxed);
        let work = |_| {
            count.fetch_add(1, Ordering::Relaxed);
            if count.load(Ordering::Relaxed) == stop { Err(BackingError::Cancelled) } else { Ok(()) }
        };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(prepared.render_band(Rect::new(0, 0, 4, 3), &control).is_err(), "cancel fence{stop}");
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(old.pixels(), old_pixels);
        assert!(owner.shares_model(&previous));
    }
    let pressure = q.reserve(QuotaKind::Resident, (128 << 20) - baseline.resident - 16).unwrap();
    let before = q.usage().unwrap();
    assert!(prepared.render_band(Rect::new(0, 0, 4, 3), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(pressure);
    let count = AtomicUsize::new(0);
    let work = |_| {
        count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let p = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &control).unwrap();
    drop(p);
    let fences = count.load(Ordering::Relaxed);
    for stop in 1..=fences {
        count.store(0, Ordering::Relaxed);
        let work = |_| {
            count.fetch_add(1, Ordering::Relaxed);
            if count.load(Ordering::Relaxed) == stop { Err(BackingError::Cancelled) } else { Ok(()) }
        };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &control).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    assert!(PreparedBackedDocument::new_accounted(&owner, photocraft_cms::Builtin::Srgb.profile(), NormalBandLimits::default(), &ctl()).is_err());
    drop(old);
    drop(prepared);
    drop(owner);
    drop(previous);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn unqualified_group_contexts_and_invalid_mask_samples_refuse_atomically() {
    let root = root();
    let data = data(&root);
    let row = data["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-f64-visible-mask").unwrap();
    let (owner, q, profile) = fixture(&root, row);
    for change in 0..6 {
        let copy = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
        let mut doc = owner.document().clone();
        match change {
            0 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().compatibility_epoch = "unknown-group".into(),
            1 => doc.layers[1].blend = BlendMode::PassThrough,
            2 => doc.layers[1].mask.as_mut().unwrap().density = 0.5,
            3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap().outside_f64_bits = 0.5f64.to_bits(),
            4 => {
                let LayerContent::Group(group) = &mut doc.layers[1].content else { panic!() };
                group.children[0].blend = BlendMode::Multiply;
            }
            _ => {
                let LayerContent::Group(group) = &mut doc.layers[1].content else { panic!() };
                group.children[0].content = LayerContent::Group(Group { children: vec![], expanded: true, artboard: None });
            }
        }
        let altered = photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new((owner.clone(), copy)), q.clone(), &ctl()).unwrap();
        let before = q.usage().unwrap();
        assert!(PreparedBackedDocument::new_accounted(&altered, &profile, NormalBandLimits::default(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        drop(altered);
    }
    let p = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
    let warm = p.render_band(owner.document().bounds(), &ctl()).unwrap();
    drop(warm);
    drop(p);
    let copy = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let source = q.reserve(QuotaKind::Resident, 1024).unwrap();
    let mut doc = owner.document().clone();
    doc.layers[1].mask.as_mut().unwrap().surface =
        Arc::new(Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &f64::NAN.to_ne_bytes()).unwrap());
    let altered = photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new((owner.clone(), copy, source)), q.clone(), &ctl()).unwrap();
    let before = q.usage().unwrap();
    assert!(PreparedBackedDocument::new_accounted(&altered, &profile, NormalBandLimits::default(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(altered);
    drop(owner);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn typed_group_mask_and_visibility_history_matches_independent_controls_and_retains_original_surface() {
    use photocraft_doc::accounted_document::{AccountedEdit, AccountedHistory};
    let root = root();
    let data = data(&root);
    let records = data["records"].as_array().unwrap();
    let enabled = records.iter().find(|r| r["id"] == "rgb-f64-visible-mask").unwrap();
    let disabled = records.iter().find(|r| r["id"] == "rgb-f64-visible-disabled-mask").unwrap();
    let hidden = records.iter().find(|r| r["id"] == "rgb-f64-hidden-mask").unwrap();
    let (original, q, profile) = fixture(&root, enabled);
    let id = original.document().layers[1].id;
    let mut history = AccountedHistory::new(original.clone(), 4, &ctl()).unwrap();
    let render = |owner: &photocraft_doc::accounted_document::AccountedDocument, row: &Value| {
        let p = PreparedBackedDocument::new_accounted(owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
        let band = p.render_band(owner.document().bounds(), &ctl()).unwrap();
        let reference = values(&read(&root, &row["projection"]["native"]), SampleType::F64);
        for (index, pixel) in band.pixels().iter().enumerate() {
            let expected = &reference[index * 4..index * 4 + 4];
            assert!((pixel[3] - expected[3]).abs() <= 2. / 65535.);
            for c in 0..3 {
                assert!((pixel[c] * pixel[3] - expected[c] * expected[3]).abs() <= 2. / 255.);
            }
        }
        band
    };
    let old = render(history.current().unwrap(), enabled);
    history.try_commit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &ctl()).unwrap();
    let current = render(history.current().unwrap(), disabled);
    let current_mask = &history.current().unwrap().document().layers[1].mask.as_ref().unwrap().surface;
    let original_mask = &original.document().layers[1].mask.as_ref().unwrap().surface;
    assert!(Arc::ptr_eq(current_mask, original_mask));
    assert!(original.document().layers[1].mask.as_ref().unwrap().enabled);
    assert!(!history.current().unwrap().document().layers[1].mask.as_ref().unwrap().enabled);
    history.try_commit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: Some(false), opacity: None }, &ctl()).unwrap();
    let hidden_output = render(history.current().unwrap(), hidden);
    assert!(history.undo(&ctl()).unwrap());
    let restored_disabled = render(history.current().unwrap(), disabled);
    assert_eq!(restored_disabled.pixels(), current.pixels());
    assert!(history.undo(&ctl()).unwrap());
    let restored = render(history.current().unwrap(), enabled);
    assert_eq!(restored.pixels(), old.pixels());
    assert!(history.current().unwrap().shares_model(&original));
    assert!(history.redo(&ctl()).unwrap());
    drop(restored);
    drop(restored_disabled);
    drop(hidden_output);
    drop(current);
    drop(old);
    drop(history);
    drop(original);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
