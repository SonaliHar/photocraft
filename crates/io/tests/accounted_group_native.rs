#![cfg(all(windows, feature = "corpus"))]
//! Actual XCF -> TEST-only exact leaf capture -> owned native archive -> group
//! rendering. Default eager import routing is deliberately not changed here.
#[path = "common/accounted_raster_capture.rs"]
mod capture;
use photocraft_compose::{backed_document::PreparedBackedDocument, backed_normal::NormalBandLimits};
use photocraft_doc::{Layer, LayerContent, LayerId, LayerMask, item_identity::ItemRef, xcf_state::XcfState};
use photocraft_format::owned_document::OwnedDocumentLimits;
use photocraft_geom::Rect;
use photocraft_io::{accounted_native::*, xcf_source::ServiceAuthorizedScratchDirectory};
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl, quota::*},
};
use photocraft_xcf::{Budget, ResourceLimits, ResourcePolicy};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-group-native-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let removed = fs::remove_dir_all(&self.0);
        // A failed assertion can unwind before earlier-declared backing owners
        // release Windows handles. Preserve that original failure, not a second
        // destructor panic; successful cases assert empty scratch explicitly.
        if !std::thread::panicking() {
            assert!(removed.is_ok(), "temporary group archive cleanup: {removed:?}");
        }
    }
}
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("accounted-normal-group-evidence")
}
fn read(root: &Path, row: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    bytes
}
fn records(root: &Path) -> Value {
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let pins: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_accounted_group_hashes.json")).unwrap();
    assert_eq!(hash(&bytes), pins["observations.json"]);
    serde_json::from_slice(&bytes).unwrap()
}
fn bounds(row: &Value) -> Rect {
    let x = row["offset"][0].as_i64().unwrap() as i32;
    let y = row["offset"][1].as_i64().unwrap() as i32;
    Rect::new(x, y, x + row["size"][0].as_i64().unwrap() as i32, y + row["size"][1].as_i64().unwrap() as i32)
}
fn authored<'a>(rows: &'a [Value], name: &str) -> &'a Value {
    rows.iter().find(|row| row["name"] == name).unwrap()
}
fn masks(layers: &[Layer], out: &mut Vec<(LayerId, Option<LayerMask>)>) {
    for layer in layers {
        out.push((layer.id, layer.mask.clone()));
        if let LayerContent::Group(group) = &layer.content {
            masks(&group.children, out);
        }
    }
}
fn find(layers: &[Layer], id: LayerId) -> &Layer {
    for layer in layers {
        if layer.id == id {
            return layer;
        }
        if let LayerContent::Group(group) = &layer.content
            && group.children.iter().any(|l| l.id == id)
        {
            return find(&group.children, id);
        }
    }
    panic!("missing captured layer")
}
fn capture_layers(
    layers: &mut [Layer],
    state: &XcfState,
    rows: &[Value],
    root: &Path,
    service: &ServiceAuthorizedScratchDirectory<'_, '_>,
    q: &BackingQuota,
) -> usize {
    let mut captured = 0;
    for layer in layers {
        let row = authored(rows, &layer.name);
        let canonical = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).unwrap();
        assert_eq!(canonical.raster_bounds, Some(bounds(row)), "actual canonical extent for {}", layer.name);
        assert_eq!(layer.visible, row["visible"].as_bool().unwrap());
        assert_eq!(layer.opacity, (row["opacity"].as_f64().unwrap() / 100.) as f32);
        if let Some(mask) = &layer.mask {
            let original = &row["mask"];
            assert!(!original.is_null());
            let expected = read(root, &original["native"]);
            assert_eq!(mask.surface.try_read_interleaved(bounds(original)).unwrap(), expected, "{} original mask native", layer.name);
            assert_eq!(mask.enabled, original["enabled"].as_bool().unwrap());
        } else {
            assert!(row["mask"].is_null());
        }
        match &mut layer.content {
            LayerContent::Group(group) => {
                assert!(group.artboard.is_none());
                assert_eq!(group.children.len(), 2);
                captured += capture_layers(&mut group.children, state, row["children"].as_array().unwrap(), root, service, q);
            }
            LayerContent::Raster(surface) => {
                let extent = canonical.raster_bounds.unwrap();
                let expected = read(root, &row["native"]);
                assert_eq!(surface.try_read_interleaved(extent).unwrap(), expected, "{} exact original leaf native", layer.name);
                let backed = capture::capture(surface, extent, service, q, &ctl()).unwrap();
                assert_eq!(backed.descriptor().default_bytes(), surface.default_bytes());
                // Verify every captured native pixel, not only the first row.
                let bpp = surface.format().bytes_per_pixel();
                let mut raw = vec![0; bpp];
                for y in extent.y0..extent.y1 {
                    for x in extent.x0..extent.x1 {
                        backed.checked_view().read_logical_pixel_native_into(x, y, &mut raw, &ctl()).unwrap();
                        let offset = (((y - extent.y0) as usize) * extent.width() as usize + (x - extent.x0) as usize) * bpp;
                        assert_eq!(raw, &expected[offset..offset + bpp]);
                    }
                }
                layer.content = LayerContent::BackedRaster(backed);
                captured += 1;
            }
            _ => panic!("unqualified actual source kind"),
        }
    }
    captured
}
fn check_projection(
    owner: &photocraft_doc::accounted_document::AccountedDocument,
    profile: &photocraft_cms::Profile,
    root: &Path,
    row: &Value,
) -> photocraft_compose::backed_normal_stack::OwnedNormalStackBand {
    let prepared = PreparedBackedDocument::new_accounted(owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    let band = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
    let native = read(root, &row["projection"]["native"]);
    let display = read(root, &row["projection"]["nonlinear_double"]);
    let channels = owner.document().mode.color_channels();
    let depth = owner.document().depth;
    let target = if channels == 1 { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
    let transform = photocraft_cms::Transform::new(profile, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
    for (i, pixel) in band.pixels().iter().enumerate() {
        let offset = i * (channels + 1);
        let alpha = photocraft_color::try_read_sample_f64(&native, depth, offset + channels).unwrap();
        assert!((pixel[3] - alpha).abs() <= 2. / 65535., "{} alpha{i}", row["id"]);
        let mut converted = [0.; 3];
        transform.try_eval_f64(&pixel[..channels], &mut converted[..channels]).unwrap();
        for c in 0..channels {
            let expected = photocraft_color::try_read_sample_f64(&native, depth, offset + c).unwrap();
            assert!((pixel[c] * pixel[3] - expected * alpha).abs() <= 2. / 255., "{} native{i}/{c}", row["id"]);
            let b = (offset + c) * 8;
            let expected = f64::from_le_bytes(display[b..b + 8].try_into().unwrap());
            assert!((converted[c] * pixel[3] - expected * alpha).abs() <= 2. / 255., "{} display{i}/{c}", row["id"]);
        }
    }
    band
}

// Reopened exact leaves and the current canonical/source fields remain separate
// from projected reference data throughout every paid history state.
fn verify_reopened(layers: &[Layer], state: &XcfState, rows: &[Value], root: &Path) {
    for layer in layers {
        let row = authored(rows, &layer.name);
        let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).unwrap();
        assert_eq!(item.raster_bounds, Some(bounds(row)));
        match &layer.content {
            LayerContent::Group(group) => {
                assert_eq!(group.children.len(), 2);
                verify_reopened(&group.children, state, row["children"].as_array().unwrap(), root);
            }
            LayerContent::BackedRaster(backed) => {
                let extent = bounds(row);
                let expected = read(root, &row["native"]);
                let bpp = backed.format().bytes_per_pixel();
                let mut actual = vec![0; bpp];
                for y in extent.y0..extent.y1 {
                    for x in extent.x0..extent.x1 {
                        backed.checked_view().read_logical_pixel_native_into(x, y, &mut actual, &ctl()).unwrap();
                        let offset = (((y - extent.y0) as usize) * extent.width() as usize + (x - extent.x0) as usize) * bpp;
                        assert_eq!(actual, &expected[offset..offset + bpp], "{} exact reopened leaf", layer.name);
                    }
                }
            }
            _ => panic!("reopened group leaf erased"),
        }
    }
}
fn compare_native_tree(before: &[Layer], after: &[Layer]) {
    assert_eq!(before.len(), after.len());
    for (before, after) in before.iter().zip(after) {
        assert_eq!(before.id, after.id);
        assert_eq!(before.name, after.name);
        assert_eq!(before.visible, after.visible);
        assert_eq!(before.opacity.to_bits(), after.opacity.to_bits());
        assert_eq!(before.mask, after.mask);
        match (&before.content, &after.content) {
            (LayerContent::Group(before), LayerContent::Group(after)) => {
                assert_eq!(before.artboard, after.artboard);
                compare_native_tree(&before.children, &after.children);
            }
            (LayerContent::BackedRaster(before), LayerContent::BackedRaster(after)) => {
                assert_eq!(before.descriptor(), after.descriptor());
                assert!(before.try_content_equal(after, &ctl()).unwrap());
            }
            _ => panic!("native group tree changed kind"),
        }
    }
}
fn check_history(
    owner: &photocraft_doc::accounted_document::AccountedDocument,
    profile: &photocraft_cms::Profile,
    root: &Path,
    row: &Value,
    records: &[Value],
) {
    use photocraft_doc::accounted_document::{AccountedEdit, AccountedHistory};
    let id = row["id"].as_str().unwrap();
    let Some(prefix) = id.strip_suffix("-visible-mask") else { return };
    let disabled_id = format!("{prefix}-visible-disabled-mask");
    let hidden_id = format!("{prefix}-hidden-mask");
    let disabled = records.iter().find(|r| r["id"] == disabled_id).unwrap();
    let hidden = records.iter().find(|r| r["id"] == hidden_id).unwrap();
    let layer = owner.document().layers[1].id;
    let original_state = owner.document().xcf_state.as_ref().unwrap();
    let original_mask = owner.document().layers[1].mask.as_ref().unwrap();
    let mut history = AccountedHistory::new(owner.clone(), 4, &ctl()).unwrap();
    let old = check_projection(history.current().unwrap(), profile, root, row);
    history.try_commit(AccountedEdit::MaskEnabled { layer, enabled: false }, &ctl()).unwrap();
    let current = check_projection(history.current().unwrap(), profile, root, disabled);
    let mut expected_state = original_state.clone();
    expected_state.items.iter_mut().filter_map(|i| i.mask.as_mut()).find(|m| m.owner == ItemRef::Layer(layer)).unwrap().enabled = false;
    let current_owner = history.current().unwrap();
    assert_eq!(current_owner.document().xcf_state.as_ref().unwrap(), &expected_state);
    let current_mask = current_owner.document().layers[1].mask.as_ref().unwrap();
    assert!(!current_mask.enabled);
    assert!(std::sync::Arc::ptr_eq(&current_mask.surface, &original_mask.surface));
    assert!(original_mask.enabled);
    verify_reopened(&current_owner.document().layers, &expected_state, row["layers"].as_array().unwrap(), root);
    history.try_commit(AccountedEdit::LayerMetadata { layer, name: None, visible: Some(false), opacity: None }, &ctl()).unwrap();
    expected_state.items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().visible = false;
    assert_eq!(history.current().unwrap().document().xcf_state.as_ref().unwrap(), &expected_state);
    let hidden_band = check_projection(history.current().unwrap(), profile, root, hidden);
    assert!(history.undo(&ctl()).unwrap());
    let restored_disabled = check_projection(history.current().unwrap(), profile, root, disabled);
    assert_eq!(restored_disabled.pixels(), current.pixels());
    assert!(history.undo(&ctl()).unwrap());
    assert!(history.current().unwrap().shares_model(owner));
    assert_eq!(history.current().unwrap().document().xcf_state.as_ref().unwrap(), original_state);
    let restored = check_projection(history.current().unwrap(), profile, root, row);
    assert_eq!(restored.pixels(), old.pixels());
    assert!(history.redo(&ctl()).unwrap());
    let redone_disabled = check_projection(history.current().unwrap(), profile, root, disabled);
    assert_eq!(redone_disabled.pixels(), current.pixels());
    assert!(history.redo(&ctl()).unwrap());
    let redone_hidden = check_projection(history.current().unwrap(), profile, root, hidden);
    assert_eq!(redone_hidden.pixels(), hidden_band.pixels());
    let final_owner = history.current().unwrap();
    assert_eq!(final_owner.document().xcf_state.as_ref().unwrap(), &expected_state);
    assert_eq!(final_owner.document().icc_profile, owner.document().icc_profile);
    assert_eq!(final_owner.document().source_color_context, owner.document().source_color_context);
    assert_eq!(final_owner.document().working_profile_interpretation, owner.document().working_profile_interpretation);
    assert_eq!(final_owner.document().sample_encoding, owner.document().sample_encoding);
    assert!(std::sync::Arc::ptr_eq(&final_owner.document().layers[1].mask.as_ref().unwrap().surface, &original_mask.surface));
    verify_reopened(&final_owner.document().layers, &expected_state, row["layers"].as_array().unwrap(), root);
}

#[test]
#[ignore = "requires reviewed unpublished original group corpus; conversion is test-only"]
fn twenty_four_actual_group_archives_preserve_tree_native_current_source_and_renderer_lifetime() {
    let root = root();
    let data = records(&root);
    let rows = data["records"].as_array().unwrap();
    assert_eq!(rows.len(), 24);
    for row in rows {
        let source = read(&root, &row["source"]);
        let source_path = root.join(row["source"]["file"].as_str().unwrap());
        let mut doc = photocraft_io::xcf_import::import("original independent Normal group", &source, &Interrupt::NONE).unwrap().document;
        assert!(matches!(doc.layers[1].content, LayerContent::Group(_)), "never substitute observed group cached raster");
        if doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(doc.layers[1].id)).unwrap().raster_bounds.is_none() {
            let parsed = photocraft_xcf::decode(&source, &Default::default()).unwrap();
            eprintln!(
                "source {}: version={} precision={} encoding={:?} model={:?}",
                row["id"], parsed.version, parsed.precision, parsed.encoding, parsed.model
            );
            for (i, layer) in parsed.layers.iter().enumerate() {
                eprintln!(
                    "layer{i}: group={} path={:?} model={:?} alpha={} unsupported={:?} opacity={} tuple={:?} plane={:?}",
                    layer.group,
                    layer.path,
                    layer.model,
                    layer.alpha,
                    layer.unsupported,
                    layer.opacity,
                    (layer.blend_space, layer.composite_space, layer.composite_mode),
                    layer.plane.as_ref().map(|p| (p.width, p.height, p.channels, p.sample))
                );
                if let Some(mask) = &layer.mask {
                    eprintln!(
                        "layer{i} mask: size={:?} offset={:?} model={:?} alpha={} plane={:?}",
                        (mask.width, mask.height),
                        mask.offset,
                        mask.model,
                        mask.alpha,
                        mask.plane.as_ref().map(|p| (p.width, p.height, p.channels, p.sample))
                    );
                }
            }
        }
        let original_state = doc.xcf_state.clone();
        let original_profile = doc.icc_profile.clone();
        let original_context = doc.source_color_context.clone();
        let original_interpretation = doc.working_profile_interpretation.clone();
        let original_encoding = doc.sample_encoding;
        let mut original_masks = Vec::new();
        masks(&doc.layers, &mut original_masks);
        let temp = Temp::new();
        let scratch = temp.0.join("scratch");
        fs::create_dir(&scratch).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, scratch: 256 << 20, index: 16 << 20 });
        {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            assert_eq!(capture_layers(&mut doc.layers, doc.xcf_state.as_ref().unwrap(), row["layers"].as_array().unwrap(), &root, &service, &q), 3);
        }
        assert_eq!(doc.xcf_state, original_state);
        assert_eq!(doc.icc_profile, original_profile);
        assert_eq!(doc.source_color_context, original_context);
        for (id, mask) in &original_masks {
            let layer = find(&doc.layers, *id);
            assert_eq!(&layer.mask, mask);
            if let (Some(after), Some(before)) = (&layer.mask, mask) {
                assert!(std::sync::Arc::ptr_eq(&after.surface, &before.surface));
            }
        }
        let path = temp.0.join("current.pcraft");
        let write = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &ctl()).unwrap();
        save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &ctl(), |_, _| Ok(())).unwrap();
        let saved_hash = hash(&fs::read(&path).unwrap());
        let usage = q.usage().unwrap();
        let gates = AtomicUsize::new(0);
        assert!(
            save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &ctl(), |_, _| {
                if gates.fetch_add(1, Ordering::SeqCst) == 0 { Ok(()) } else { Err(BackingError::Cancelled.into()) }
            })
            .is_err()
        );
        assert_eq!(hash(&fs::read(&path).unwrap()), saved_hash);
        assert_eq!(q.usage().unwrap(), usage);
        let protected = temp.0.join("original.xcf");
        fs::write(&protected, &source).unwrap();
        let protect = ServiceAuthorizedNativeWritePath::from_platform_authorized(protected.clone(), &q, &ctl()).unwrap();
        assert!(save_accounted_native_path(&doc, &protect, OwnedDocumentLimits::default(), &q, &ctl(), |_, _| Ok(())).is_err());
        assert_eq!(fs::read(protected).unwrap(), source);
        drop(protect);
        let read = ServiceAuthorizedNativeReadPath::from_platform_authorized(path.clone(), &q, &ctl()).unwrap();
        let loaded = {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            open_accounted_native(&read, &service, OwnedDocumentLimits::default(), q.clone(), &ctl()).unwrap()
        };
        assert_eq!(loaded.document().xcf_state, original_state);
        assert_eq!(loaded.document().icc_profile, original_profile);
        assert_eq!(loaded.document().source_color_context, original_context);
        assert_eq!(loaded.document().working_profile_interpretation, original_interpretation);
        assert_eq!(loaded.document().sample_encoding, original_encoding);
        compare_native_tree(&doc.layers, &loaded.document().layers);
        for (id, mask) in &original_masks {
            assert_eq!(&find(&loaded.document().layers, *id).mask, mask);
        }
        drop(doc);
        drop(read);
        drop(write);
        fs::remove_file(path).unwrap();
        let owner = loaded.into_accounted_document(&ctl()).unwrap();
        verify_reopened(&owner.document().layers, owner.document().xcf_state.as_ref().unwrap(), row["layers"].as_array().unwrap(), &root);
        let profile = photocraft_cms::Profile::parse(owner.document().icc_profile.as_ref().unwrap()).unwrap();
        check_history(&owner, &profile, &root, row, rows);
        let old = check_projection(&owner, &profile, &root, row);
        let p = PreparedBackedDocument::new_accounted(&owner, &profile, NormalBandLimits::default(), &ctl()).unwrap();
        let baseline = q.usage().unwrap();
        let calls = AtomicUsize::new(0);
        let work = |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        let band = p.render_band(Rect::new(0, 0, 4, 3), &control).unwrap();
        drop(band);
        let last = calls.load(Ordering::Relaxed);
        calls.store(0, Ordering::Relaxed);
        let work = |_| {
            if calls.fetch_add(1, Ordering::Relaxed) + 1 == last { Err(BackingError::Cancelled) } else { Ok(()) }
        };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(p.render_band(Rect::new(0, 0, 4, 3), &control).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        let pressure = q.reserve(QuotaKind::Resident, (256 << 20) - baseline.resident - 16).unwrap();
        let pressured = q.usage().unwrap();
        assert!(p.render_band(Rect::new(0, 0, 4, 3), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), pressured);
        drop(pressure);
        drop(p);
        assert!(PreparedBackedDocument::new_accounted(&owner, photocraft_cms::Builtin::Srgb.profile(), NormalBandLimits::default(), &ctl()).is_err());
        let previous = owner.clone();
        let id = owner.document().layers[1].id;
        let renamed = owner
            .try_edit(
                photocraft_doc::accounted_document::AccountedEdit::LayerMetadata {
                    layer: id,
                    name: Some("current paid group name"),
                    visible: None,
                    opacity: None,
                },
                &ctl(),
            )
            .unwrap();
        assert_eq!(previous.document().xcf_state, original_state);
        assert_eq!(renamed.document().layers[1].name, "current paid group name");
        let current = check_projection(&renamed, &profile, &root, row);
        assert_eq!(current.pixels(), old.pixels());
        assert_eq!(hash(&fs::read(source_path).unwrap()), row["source"]["sha256"]);
        drop(current);
        drop(old);
        drop(renamed);
        drop(previous);
        drop(owner);
        drop(original_masks);
        drop(original_state);
        drop(original_profile);
        drop(original_context);
        drop(original_interpretation);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        assert_eq!(fs::read_dir(scratch).unwrap().count(), 0);
    }
}
