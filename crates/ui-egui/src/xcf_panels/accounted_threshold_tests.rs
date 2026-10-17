//! UNRUN ignored UI workflows over independently saved, SHA-pinned originals.
//! No fabricated current metadata is used as original-XCF qualification.
use super::*;
use crate::Services;
use photocraft_doc::{
    LayerContent,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_engine::{Session, document_snapshot::DocumentSnapshot};
use photocraft_format::{
    LoadOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{load_document_from_owned_archive, save_current_document_archive},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use photocraft_xcf::{BudgetKind, DecodeOptions, Encoding, Model, ResourceLimits, Sample};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Evidence {
    imported: photocraft_io::OwnedXcfImport,
    bytes: Arc<Vec<u8>>,
    quota: BackingQuota,
    name: String,
    _capture: QuotaLease,
}
fn evidence(sample: Sample) -> Evidence {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires pinned full-XCF evidence corpus"))
        .join("threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof_bytes = std::fs::read(root.join("fresh-verification.json")).unwrap();
    assert_eq!(hash(&proof_bytes), "0e9a4c5cc20585efe0b9007a2ad983e22c846f85d7b61e0b437b166cab394f64");
    let proof: serde_json::Value = serde_json::from_slice(&proof_bytes).unwrap();
    let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits =
        ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)]).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        assert!(name.len() <= 4096 && !name.contains(['/', '\\']));
        let path = root.join(&name);
        let length = std::fs::metadata(&path).unwrap().len();
        assert!(length > 0 && length <= 16 << 20);
        let capture = quota.reserve(QuotaKind::Resident, length.checked_mul(2).unwrap() + 4096).unwrap();
        let bytes = Arc::new(std::fs::read(path).unwrap());
        assert!(bytes.capacity() as u64 <= 2 * length);
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let raw = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
        // Choose the independently authored RGB requested-depth nonlinear state whose exact
        // source class is admitted by the existing qualified importer.
        if raw.model != Model::Rgb
            || raw.sample != sample
            || raw.encoding != Encoding::NonlinearProfile
            || raw.precision
                != match sample {
                    Sample::U16 => 250,
                    Sample::F32 => 650,
                    Sample::F64 => 750,
                    _ => panic!("closed sample"),
                }
            || !raw
                .layers
                .first()
                .and_then(|layer| layer.effects.first())
                .is_some_and(|effect| effect.operation == "gimp:threshold" && effect.state.enabled == Some(true))
        {
            continue;
        }
        let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
            &name,
            &mut std::io::Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &DecodeOptions::default(),
            &limits,
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(imported.document().quota().shares_ledger(&quota));
        assert_eq!(limits.get(BudgetKind::Scratch), 0);
        return Evidence { imported, bytes, quota, name, _capture: capture };
    }
    panic!("pinned packet has no supported RGB requested-depth nonlinear Histogram Threshold state")
}

fn app(sample: Sample, services: Services) -> (PhotocraftApp, Evidence) {
    let evidence = evidence(sample);
    let photocraft_io::OwnedXcfImport::HistogramThreshold(imported) = &evidence.imported else { panic!("selected original did not retain threshold family") };
    let mut session = Session::new();
    session.add_owned_histogram_threshold_xcf_import(imported, &evidence.name, Some("authorized/original.xcf"), &ctl()).unwrap();
    (PhotocraftApp::new(session, services), evidence)
}
fn node(app: &PhotocraftApp) -> &photocraft_doc::xcf_filters::FilterNode {
    &app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
}
fn number(app: &PhotocraftApp, name: &str) -> u32 {
    let argument = node(app).arguments.iter().find(|a| a.name == name).unwrap();
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = argument.value else { panic!("not exact F32") };
    bits
}
fn channel(app: &PhotocraftApp) -> i32 {
    let argument = node(app).arguments.iter().find(|a| a.name == "channel").unwrap();
    let FilterValue::Enum { domain, value } = &argument.value else { panic!("not histogram enum") };
    assert_eq!(domain, "GimpHistogramChannel");
    *value
}
fn patch(app: &PhotocraftApp, name: &str, value: Value) -> Value {
    json!({"filterId":node(app).id.0.to_string(),"arguments":[{"name":name,"value":value}]})
}
fn set_number(app: &PhotocraftApp, name: &str, bits: u32) -> Value {
    patch(app, name, json!({"Number":{"Float32Bits":bits}}))
}
fn set_channel(app: &PhotocraftApp, value: i32) -> Value {
    patch(app, "channel", json!({"Enum":{"domain":"GimpHistogramChannel","value":value}}))
}
fn stack_revision(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].revision
}
// Full storage observation is deliberately separate from current canvas output.
// The pinned closed class has one layer tile and at most one resident mask tile.
type NativeMasks = Vec<(photocraft_raster::backing::NativePlaneDescriptor, Option<(photocraft_geom::TileCoord, Vec<u8>)>)>;
#[derive(Debug, PartialEq, Eq)]
struct RawStorage {
    layer: photocraft_raster::backing::NativePlaneDescriptor,
    layer_full_tile: bool,
    layer_bytes: Vec<u8>,
    masks: NativeMasks,
}
#[derive(Debug)]
struct NativeSnapshot {
    storage: RawStorage,
    // Copied payload/metadata drop before their genuine same-ledger derivative fee.
    _lease: QuotaLease,
}
impl PartialEq for NativeSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.storage == other.storage
    }
}
impl Eq for NativeSnapshot {}
fn native(snapshot: &DocumentSnapshot) -> NativeSnapshot {
    use photocraft_raster::{backing::NativePlaneDescriptor, checked_view::NativeTileBytes};
    let LayerContent::BackedRaster(layer) = &snapshot.layers[0].content else { panic!("source is not native backed") };
    assert_eq!(snapshot.layers.len(), 1);
    assert_eq!(layer.operative_bounds(), photocraft_geom::Rect::new(0, 0, 8, 6));
    let masks = &snapshot.xcf_state.as_ref().unwrap().filter_masks;
    assert!(masks.len() <= 1);
    let full_layer = 256usize.checked_mul(256).unwrap().checked_mul(layer.format().bytes_per_pixel()).unwrap();
    let mut payload = full_layer;
    for mask in masks {
        assert!(mask.surface.tile_count() <= 1);
        payload = payload.checked_add(256 * 256 * mask.surface.format().bytes_per_pixel()).unwrap();
    }
    // Double actual maximum payload admits allocator excess; 4096 prices the
    // bounded descriptor/tuple/vector headers. No native data is narrowed.
    let admitted = payload.checked_mul(2).unwrap().checked_add(4096).unwrap() as u64;
    let mut fee = layer.quota().reserve(QuotaKind::Resident, admitted).unwrap();
    fn copy(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.try_reserve_exact(bytes.len()).unwrap();
        assert!(out.capacity() <= bytes.len().checked_mul(2).unwrap());
        for chunk in bytes.chunks(64 * 1024) {
            ctl().check(chunk.len() as u64).unwrap();
            out.extend_from_slice(chunk);
        }
        out
    }
    let pin = layer.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &ctl()).unwrap();
    let (layer_full_tile, layer_bytes) = match pin.encoded() {
        NativeTileBytes::FullTile(bytes) => {
            assert_eq!(bytes.len(), full_layer);
            (true, copy(bytes))
        }
        NativeTileBytes::DefaultPixel(_) => panic!("backed authoritative tile was not full native storage"),
    };
    drop(pin);
    let mut copied_masks = Vec::new();
    copied_masks.try_reserve_exact(masks.len()).unwrap();
    assert!(copied_masks.capacity() <= 2 * masks.len());
    let mut actual = layer_bytes.capacity() + 4096;
    for mask in masks {
        ctl().check(1).unwrap();
        let descriptor = NativePlaneDescriptor::new(mask.surface.format(), mask.bounds, mask.surface.default_bytes()).unwrap();
        let mut tile = None;
        for (coord, bytes) in mask.surface.tiles() {
            assert_eq!(*coord, photocraft_geom::TileCoord::new(0, 0));
            assert_eq!(bytes.bytes().len(), 256 * 256 * mask.surface.format().bytes_per_pixel());
            let copied = copy(bytes.bytes());
            actual = actual.checked_add(copied.capacity()).unwrap();
            tile = Some((*coord, copied));
        }
        copied_masks.push((descriptor, tile));
    }
    assert!(actual as u64 <= admitted);
    fee.reconcile_capacity(actual as u64).unwrap();
    NativeSnapshot { storage: RawStorage { layer: layer.descriptor(), layer_full_tile, layer_bytes, masks: copied_masks }, _lease: fee }
}
fn scene(snapshot: &DocumentSnapshot) -> Vec<[u64; 4]> {
    let owner = snapshot.accounted().unwrap();
    let prepared =
        photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(owner, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl())
            .unwrap();
    prepared.render_band(snapshot.bounds(), &ctl()).unwrap().pixels().iter().map(|p| p.map(f64::to_bits)).collect()
}

fn resident_scene(bytes: &[u8], current: &PhotocraftApp) -> Vec<[u64; 4]> {
    let imported = photocraft_io::import("pinned-reference.xcf", bytes).unwrap();
    let id = imported.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
    let mut reference = Session::new();
    reference.add_document(imported.document, None).unwrap();
    reference
        .execute(
            "xcf.filter.set",
            json!({"filterId":id.0.to_string(),"enabled":node(current).enabled,"arguments":[
                {"name":"low","value":{"Number":{"Float32Bits":number(current,"low")}}},
                {"name":"high","value":{"Number":{"Float32Bits":number(current,"high")}}},
                {"name":"channel","value":{"Enum":{"domain":"GimpHistogramChannel","value":channel(current)}}}
            ]}),
        )
        .unwrap();
    photocraft_compose::gimp_render::try_render(&reference.active().unwrap().doc, reference.active().unwrap().doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .map(|p| p.map(f64::to_bits))
        .collect()
}
#[test]
#[ignore = "UNRUN: requires SHA-pinned independent Threshold originals; shared native/browser UI code, no desktop services"]
fn panel_controls_and_exact_low_high_channel_activation_wire_keep_native_owners() {
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    for sample in [Sample::U16, Sample::F32, Sample::F64] {
        let (app, evidence) = app(sample, Services::default());
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(480., 700.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        assert!(!h.get_by_label("Histogram channel").accesskit_node().is_disabled());
        assert!(h.query_all_by_role(egui::accesskit::Role::SpinButton).filter(|n| !n.accesskit_node().is_disabled()).count() >= 2);
        let original = h.state().session.active().unwrap().doc.clone();
        let raw = native(&original);
        let source = h.state().session.active().unwrap().source.clone();
        let low = number(h.state(), "low");
        let high = number(h.state(), "high");
        let old_channel = channel(h.state());
        let edits = [
            set_number(h.state(), "low", if low == 0.25f32.to_bits() { 0.75f32.to_bits() } else { 0.25f32.to_bits() }),
            set_number(h.state(), "high", if high == 0.75f32.to_bits() { 0.25f32.to_bits() } else { 0.75f32.to_bits() }),
            set_channel(h.state(), (old_channel + 1) % 6),
        ];
        for (i, request) in edits.into_iter().enumerate() {
            let before = h.state().session.active().unwrap().doc.clone();
            let revision = h.state().session.active().unwrap().revision;
            let stack = stack_revision(h.state());
            run_checked(h.state_mut(), &before, revision, "xcf.filter.set", request);
            let app = h.state();
            assert_eq!(app.session.active().unwrap().history.past_len(), i + 1);
            assert_eq!(app.session.active().unwrap().revision, revision + 1);
            assert_eq!(stack_revision(app), stack + 1);
            assert_eq!(native(&app.session.active().unwrap().doc), raw);
            assert_eq!(scene(&app.session.active().unwrap().doc), resident_scene(&evidence.bytes, app));
            assert_eq!(app.session.active().unwrap().doc.icc_profile, original.icc_profile);
            assert_eq!(app.session.active().unwrap().doc.source_color_context, original.source_color_context);
        }
        let before = h.state().session.active().unwrap().doc.clone();
        let revision = h.state().session.active().unwrap().revision;
        let id = node(h.state()).id.0.to_string();
        run_checked(h.state_mut(), &before, revision, "xcf.filter.enable", json!({"filterId":id,"enabled":false}));
        assert!(!node(h.state()).enabled);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 4);
        assert_eq!(native(&h.state().session.active().unwrap().doc), raw);
        assert_eq!(scene(&h.state().session.active().unwrap().doc), resident_scene(&evidence.bytes, h.state()));
        h.state_mut().run("edit.undo", json!({})).unwrap();
        let before_noop = h.state().session.active().unwrap().doc.clone();
        let revision = h.state().session.active().unwrap().revision;
        let id = node(h.state()).id.0.to_string();
        assert_eq!(h.state_mut().run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(h.state().session.active().unwrap().doc.shares_state(&before_noop));
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        assert!(h.state().session.active().unwrap().history.can_redo());
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert!(!node(h.state()).enabled);
        assert_eq!(h.state().session.active().unwrap().source, source);
        assert!(h.state().session.active().unwrap().source_read_only);
        assert!(h.state().session.active().unwrap().doc.quota().unwrap().shares_ledger(&evidence.quota));
    }
}
#[test]
#[ignore = "UNRUN: requires SHA-pinned independent Threshold originals"]
fn compound_parameter_wire_is_one_undo_and_stale_or_malformed_actions_preserve_state() {
    let (mut app, evidence) = app(Sample::F64, Services::default());
    let original = app.session.active().unwrap().doc.clone();
    let raw = native(&original);
    let revision = app.session.active().unwrap().revision;
    let id = node(&app).id.0.to_string();
    let low = if number(&app, "low") == 0.25f32.to_bits() { 0.75f32.to_bits() } else { 0.25f32.to_bits() };
    app.run(
        "xcf.filter.set",
        json!({"filterId":id,"arguments":[
        {"name":"high","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}},
        {"name":"channel","value":{"Enum":{"domain":"GimpHistogramChannel","value":4}}},
        {"name":"low","value":{"Number":{"Float32Bits":low}}}
    ],"enabled":false}),
    )
    .unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), 1);
    assert_eq!(stack_revision(&app), original.xcf_state.as_ref().unwrap().filter_stacks[0].revision + 1);
    let changed = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.past_len();
    let stale = set_number(&app, "low", 0.5f32.to_bits());
    run_checked(&mut app, &original, revision, "xcf.filter.set", stale);
    assert_eq!(app.ui.status, "xcf.properties.stale-target");
    assert!(app.session.active().unwrap().doc.shares_state(&changed));
    for request in [
        patch(&app, "low", json!({"Number":{"Float64Bits":"0x0000000000000000"}})),
        patch(&app, "low", json!({"Number":{"Float32Bits":f32::NAN.to_bits()}})),
        patch(&app, "channel", json!({"Enum":{"domain":"wrong","value":4}})),
        set_channel(&app, 6),
    ] {
        assert!(app.run("xcf.filter.set", request).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        assert_eq!(app.session.active().unwrap().history.past_len(), count);
        assert_eq!(native(&app.session.active().unwrap().doc), raw);
    }
    assert_eq!(scene(&changed), resident_scene(&evidence.bytes, &app));
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.shares_state(&original));
    app.run("edit.redo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.shares_state(&changed));
}
#[derive(Debug)]
struct SavedArchive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for SavedArchive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
#[test]
#[ignore = "UNRUN: requires SHA-pinned independently saved Histogram Threshold precision evidence"]
fn registered_edited_ui_native_save_reopen_preserves_current_node_source_guard_and_raw_words() {
    let captured: Rc<RefCell<Option<SavedArchive>>> = Rc::new(RefCell::new(None));
    let writer = captured.clone();
    let writes = Rc::new(RefCell::new(0usize));
    let seen = writes.clone();
    let (mut app, evidence) = app(
        Sample::F64,
        Services {
            accounted_native_save: Some(Box::new(move |snapshot, path, protection, c| {
                assert_eq!(path, "copy.pcraft");
                assert!(protection.sources.iter().any(|p| p == "authorized/original.xcf"));
                let quota = snapshot.quota().unwrap().clone();
                let mut fee = quota.reserve(QuotaKind::Resident, 16 << 20).map_err(|e| e.to_string())?;
                let bytes = save_current_document_archive(snapshot, std::io::Cursor::new(Vec::new()), Default::default(), &quota, c)
                    .map_err(|e| e.to_string())?
                    .into_inner();
                if bytes.capacity() as u64 + 128 > fee.bytes() {
                    return Err("native test output exceeded its prepaid envelope".into());
                }
                fee.reconcile_capacity(bytes.capacity() as u64 + 128).map_err(|e| e.to_string())?;
                *writer.borrow_mut() = Some(SavedArchive { bytes, quota, _lease: fee });
                *seen.borrow_mut() += 1;
                Ok(())
            })),
            native_save: Some(Box::new(|_, _, _| panic!("resident writer reached"))),
            ..Default::default()
        },
    );
    let raw = native(&app.session.active().unwrap().doc);
    let next = if number(&app, "low") == 0.75f32.to_bits() { 0.25f32.to_bits() } else { 0.75f32.to_bits() };
    let mut request = set_number(&app, "low", next);
    request.as_object_mut().unwrap().insert("enabled".into(), Value::Bool(false));
    request["arguments"].as_array_mut().unwrap().extend([
        json!({"name":"high","value":{"Number":{"Float32Bits":0.875f32.to_bits()}}}),
        json!({"name":"channel","value":{"Enum":{"domain":"GimpHistogramChannel","value":5}}}),
    ]);
    app.run("xcf.filter.set", request).unwrap();
    let current = app.session.active().unwrap().doc.clone();
    let display = scene(&current);
    assert_eq!(native(&current), raw);
    let history = app.session.active().unwrap().history.past_len();
    assert!(crate::menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({"path":"authorized/original.xcf"})).is_err());
    assert_eq!(*writes.borrow(), 0);
    assert!(app.session.active().unwrap().doc.shares_state(&current));
    crate::menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({"path":"copy.pcraft"})).unwrap();
    assert_eq!(*writes.borrow(), 1);
    assert_eq!(native(&app.session.active().unwrap().doc), raw);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(app.session.active().unwrap().source_read_only);
    assert_eq!(app.session.active().unwrap().source.as_ref().unwrap().path.as_deref(), Some("authorized/original.xcf"));
    let saved = Arc::new(captured.borrow_mut().take().unwrap());
    let archive = load_document_from_owned_archive(saved, &LoadOptions::default(), Default::default(), evidence.quota.clone(), &ctl()).unwrap();
    let pending = Rc::new(RefCell::new(Some(archive)));
    let take = pending.clone();
    let mut reopened = PhotocraftApp::new(
        Session::new(),
        Services {
            owned_native_open: Some(Box::new(move |path, catalog, c| {
                assert_eq!(path, "copy.pcraft");
                assert!(catalog.is_none());
                c.check(1).map_err(|e| e.to_string())?;
                take.borrow_mut().take().ok_or("native archive already consumed".into())
            })),
            ..Default::default()
        },
    );
    crate::menus::invoke(&mut reopened, &egui::Context::default(), "file.open", json!({"path":"copy.pcraft","nativeProfilePolicy":"keepCurrent"})).unwrap();
    let state = reopened.session.active().unwrap();
    assert!(state.doc.accounted().unwrap().is_readonly_threshold_storage());
    assert!(state.doc.quota().unwrap().shares_ledger(&evidence.quota));
    assert_eq!(number(&reopened, "low"), next);
    assert_eq!(number(&reopened, "high"), 0.875f32.to_bits());
    assert_eq!(channel(&reopened), 5);
    assert_eq!(node(&reopened).arguments, current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
    assert!(!node(&reopened).enabled);
    assert_eq!(native(&state.doc), raw);
    assert_eq!(scene(&state.doc), display);
    assert_eq!(state.doc.source_color_context, current.source_color_context);
    assert_eq!(state.doc.xcf_state.as_ref().unwrap().provenance, current.xcf_state.as_ref().unwrap().provenance);
    assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
    assert!(!state.source_read_only);
}

#[test]
#[ignore = "UNRUN: pinned owned original plus synthetic CURRENT lock archive; no GIMP-authored lock claim"]
fn native_current_lock_controls_disable_content_and_visibility_only_blocks_activation_changes() {
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    for lock in 0..4 {
        let evidence = evidence(Sample::F64);
        let owner = evidence.imported.document();
        let q = evidence.quota.clone();
        let model = photocraft_io::retained_native_input_bytes(
            owner.document(),
            photocraft_format::PreparationLimits { resident_bytes: 64 << 20, ..Default::default() },
            &Interrupt::NONE,
        )
        .unwrap();
        let metadata = q.reserve(QuotaKind::Index, (2 * model + (1 << 20)) as u64).unwrap();
        let mut doc = owner.document().clone();
        let layer = doc.layers[0].id;
        match lock {
            0 => doc.layers[0].locks.all = true,
            1 => doc.layers[0].locks.pixels = true,
            2 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.content = true,
            _ => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.visibility = true,
        }
        let fee = q.reserve(QuotaKind::Resident, (8 << 20) + 256).unwrap();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(8 << 20).unwrap();
        assert!(bytes.capacity() <= 8 << 20);
        let bytes = save_current_document_archive(&doc, std::io::Cursor::new(bytes), Default::default(), &q, &ctl()).unwrap().into_inner();
        assert!(bytes.capacity() <= 8 << 20);
        let saved = Arc::new(SavedArchive { bytes, quota: q.clone(), _lease: fee });
        drop(doc);
        drop(metadata);
        let archive = load_document_from_owned_archive(saved, &LoadOptions::default(), Default::default(), q.clone(), &ctl()).unwrap();
        let mut session = Session::new();
        session.add_owned_native_threshold_archive(archive, "locked-current.pcraft", None, &ctl()).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(480., 700.))
            .build_ui_state(move |ui, app| filter_properties(app, ui, layer), PhotocraftApp::new(session, Services::default()));
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        assert_eq!(h.get_by_label("Histogram channel").accesskit_node().is_disabled(), lock < 3);
        let original = h.state().session.active().unwrap().doc.clone();
        let raw = native(&original);
        let revision = h.state().session.active().unwrap().revision;
        let low = number(h.state(), "low");
        let request = set_number(h.state(), "low", if low == 0.25f32.to_bits() { 0.75f32.to_bits() } else { 0.25f32.to_bits() });
        if lock < 3 {
            assert!(h.state_mut().run("xcf.filter.set", request).is_err());
            let request = json!({"filterId":node(h.state()).id.0.to_string(),"enabled":!node(h.state()).enabled});
            assert!(h.state_mut().run("xcf.filter.enable", request).is_err());
            assert!(h.state().session.active().unwrap().doc.shares_state(&original));
            assert_eq!(h.state().session.active().unwrap().revision, revision);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
        } else {
            assert_eq!(h.state_mut().run("xcf.filter.set", request).unwrap()["updated"], true);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            let original = h.state().session.active().unwrap().doc.clone();
            let enabled = node(h.state()).enabled;
            let id = node(h.state()).id.0.to_string();
            assert_eq!(h.state_mut().run("xcf.filter.enable", json!({"filterId":id,"enabled":enabled})).unwrap()["updated"], false);
            assert!(h.state().session.active().unwrap().history.can_redo());
            assert!(h.state_mut().run("xcf.filter.enable", json!({"filterId":id,"enabled":!enabled})).is_err());
            assert!(h.state().session.active().unwrap().doc.shares_state(&original));
            assert!(h.state().session.active().unwrap().history.can_redo());
            h.state_mut().run("edit.redo", json!({})).unwrap();
        }
        assert_eq!(native(&h.state().session.active().unwrap().doc), raw);
        assert_eq!(h.state().session.active().unwrap().doc.source_color_context, original.source_color_context);
        assert_eq!(h.state().session.active().unwrap().doc.icc_profile, original.icc_profile);
    }
}
