//! UNRUN ignored integration drafts over independently saved, hash-bound
//! Histogram Threshold originals. Positive owners come only from the real
//! selected importer/owned native reader, never resident fixture adoption.
use super::*;
use crate::{DocumentSnapshot, accounted_native_save::PreparedAccountedNativeSave};
use photocraft_doc::{LayerContent, SampleType};
use photocraft_format::owned_document::{OwnedDocumentArchive, OwnedDocumentLimits, save_current_document_archive};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
};
use photocraft_xcf::{DecodeOptions, ResourceLimits};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
const CASES: [(&str, SampleType); 3] =
    [("rgb-u16_non_linear.channel0", SampleType::U16), ("rgb-float_non_linear.channel0", SampleType::F32), ("rgb-double_non_linear.channel0", SampleType::F64)];
struct Fixture {
    imported: photocraft_io::OwnedXcfImport,
    encoded: Arc<Vec<u8>>,
    expected_native: Vec<u8>,
    name: String,
    quota: BackingQuota,
    _capture: QuotaLease,
}
impl Fixture {
    fn histogram(&self) -> &photocraft_io::OwnedHistogramThresholdXcfImport {
        match &self.imported {
            photocraft_io::OwnedXcfImport::HistogramThreshold(v) => v,
            _ => panic!("selected original did not produce the Histogram Threshold class"),
        }
    }
}
fn corpus() -> (PathBuf, Value, Value) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires independently saved local full-XCF corpus"))
        .join("threshold-precision-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&bytes), "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let proof = std::fs::read(root.join("fresh-verification.json")).unwrap();
    assert_eq!(hash(&proof), manifest["fresh-verification.json"].as_str().unwrap());
    (root, manifest, serde_json::from_slice(&proof).unwrap())
}
fn limits() -> ResourceLimits {
    ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)]).unwrap()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn capture(root: &std::path::Path, name: &str, q: &BackingQuota) -> (Arc<Vec<u8>>, QuotaLease) {
    assert!(name.len() <= 4096 && !name.contains(['/', '\\']));
    let n = std::fs::metadata(root.join(name)).unwrap().len();
    assert!(n > 0 && n <= 16 << 20);
    let fee = q.reserve(QuotaKind::Resident, n.checked_mul(2).unwrap() + 65536).unwrap();
    let bytes = Arc::new(std::fs::read(root.join(name)).unwrap());
    assert!(bytes.capacity() as u64 <= 2 * n);
    (bytes, fee)
}
fn fixture(id: &str, sample: SampleType) -> Fixture {
    let (root, manifest, proof) = corpus();
    let row = proof["checks"].as_array().unwrap().iter().find(|v| v["id"].as_str() == Some(id)).unwrap();
    assert_eq!(row["native_exact"], true);
    let name = format!("{id}.xcf");
    let q = quota();
    let (encoded, fee) = capture(&root, &name, &q);
    assert_eq!(hash(&encoded), manifest[&name].as_str().unwrap());
    let native_name = row["fresh"]["layers"][0]["native"]["file"].as_str().unwrap();
    assert!(!native_name.contains(['/', '\\']));
    assert!(std::fs::metadata(root.join(native_name)).unwrap().len() <= 32768);
    let expected_native = std::fs::read(root.join(native_name)).unwrap();
    assert_eq!(hash(&expected_native), manifest[native_name].as_str().unwrap());
    assert!(expected_native.capacity() <= 32768);
    let imported = photocraft_io::import_owned_xcf_bytes_in_quota(&name, encoded.clone(), &DecodeOptions::default(), &limits(), q.clone(), &ctl()).unwrap();
    assert!(matches!(&imported, photocraft_io::OwnedXcfImport::HistogramThreshold(_)));
    assert!(imported.document().quota().shares_ledger(&q));
    assert!(imported.document().is_readonly_threshold_storage());
    let doc = imported.document().document();
    assert_eq!(doc.depth, sample);
    assert_eq!(doc.bounds(), photocraft_geom::Rect::new(0, 0, 8, 6));
    assert!(matches!(doc.layers[0].content, LayerContent::BackedRaster(_)));
    Fixture { imported, encoded, expected_native, name, quota: q, _capture: fee }
}
fn session(f: &Fixture) -> Session {
    let mut s = Session::new();
    s.add_owned_histogram_threshold_xcf_import(f.histogram(), &f.name, Some("authorized/original.xcf"), &ctl()).unwrap();
    s
}
fn operative(snapshot: &DocumentSnapshot) -> Vec<u8> {
    let LayerContent::BackedRaster(layer) = &snapshot.layers[0].content else { panic!("not genuine backed storage") };
    let mut bytes = vec![0; 48 * layer.format().bytes_per_pixel()];
    let bpp = layer.format().bytes_per_pixel();
    for y in 0..6 {
        for x in 0..8 {
            let at = (y * 8 + x) * bpp;
            layer.checked_view().read_logical_pixel_native_into(x as i32, y as i32, &mut bytes[at..at + bpp], &ctl()).unwrap();
        }
    }
    bytes
}
#[derive(Debug, PartialEq, Eq)]
struct RawStorage {
    layer: photocraft_raster::backing::NativePlaneDescriptor,
    layer_full_tile: bool,
    layer_bytes: Vec<u8>,
    masks: Vec<(photocraft_raster::backing::NativePlaneDescriptor, Option<(photocraft_geom::TileCoord, Vec<u8>)>)>,
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
    prepared.render_band(snapshot.bounds(), &ctl()).unwrap().pixels().iter().map(|v| v.map(f64::to_bits)).collect()
}
struct SavedBytes {
    bytes: Vec<u8>,
    _fee: QuotaLease,
}
fn save_as(s: &mut Session, path: &str, q: &BackingQuota) -> SavedBytes {
    let prepared = PreparedAccountedNativeSave::prepare(s, path, &ctl()).unwrap();
    prepared
        .write_and_publish(s, &ctl(), |snapshot, _| {
            // A bounded memory destination only; this makes no filesystem alias claim.
            let fee = q.reserve(QuotaKind::Resident, (8 << 20) + 256).unwrap();
            let bytes = save_current_document_archive(snapshot, Cursor::new(Vec::new()), Default::default(), q, &ctl())
                .map_err(|e| EngineError::Other(e.to_string()))?
                .into_inner();
            let mut result = SavedBytes { bytes, _fee: fee };
            assert!(result.bytes.capacity() <= 8 << 20);
            result._fee.reconcile_capacity(result.bytes.capacity() as u64 + 256).unwrap();
            Ok(result)
        })
        .unwrap()
}
fn load(bytes: &SavedBytes, q: &BackingQuota) -> OwnedDocumentArchive {
    let mut reader = Cursor::new(bytes.bytes.as_slice());
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut reader, bytes.bytes.len() as u64, OwnedDocumentLimits::default(), q.clone(), &ctl())
        .unwrap()
}
#[test]
#[ignore = "UNRUN: requires pinned independently authored Histogram Threshold originals"]
fn original_selected_family_genuine_retention_profile_native_words_and_source_identity() {
    for (id, sample) in CASES {
        let f = fixture(id, sample);
        let weak = Arc::downgrade(&f.encoded);
        let s = session(&f);
        let state = s.active().unwrap();
        assert_eq!(operative(&state.doc), f.expected_native);
        let raw = native(&state.doc);
        assert!(state.doc.accounted().unwrap().is_readonly_threshold_storage());
        assert!(state.doc.quota().unwrap().shares_ledger(&f.quota));
        assert!(state.source_read_only);
        assert!(state.path.is_none());
        assert_eq!(state.source.as_ref().unwrap().label, f.name);
        assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("authorized/original.xcf"));
        assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::xcf());
        assert_eq!(state.doc.source_color_context.as_ref().unwrap().original_icc, None);
        assert!(!state.history.can_undo());
        assert_eq!(state.doc.icc_profile, f.histogram().document().document().icc_profile);
        assert_eq!(native(&state.doc), raw);
        assert_eq!(scene(&state.doc).len(), 48);
        let q = f.quota.clone();
        drop(f);
        assert!(weak.upgrade().is_some());
        drop(s);
        assert!(weak.upgrade().is_none());
        drop(raw);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
#[ignore = "UNRUN: requires pinned independently authored Histogram Threshold originals"]
fn actual_native_save_reader_reopen_title_source_profile_and_readonly_continuation() {
    for (id, sample) in CASES {
        let f = fixture(id, sample);
        let mut s = session(&f);
        let original = s.active().unwrap().doc.clone();
        let source = s.active().unwrap().source.clone();
        let raw = native(&original);
        let display = scene(&original);
        let saved = save_as(&mut s, "saved-threshold.pcraft", &f.quota);
        assert_eq!(s.active().unwrap().doc.name, "saved-threshold.pcraft");
        assert_eq!(s.active().unwrap().source, source);
        assert!(s.active().unwrap().source_read_only);
        assert_eq!(native(&s.active().unwrap().doc), raw);
        let mut reopened = Session::new();
        reopened.add_owned_native_threshold_archive(load(&saved, &f.quota), "actual-input.pcraft", Some("authorized/actual-input.pcraft"), &ctl()).unwrap();
        let state = reopened.active().unwrap();
        assert_eq!(state.doc.name, "saved-threshold.pcraft");
        assert_eq!(state.source.as_ref().unwrap().label, "actual-input.pcraft");
        assert_eq!(state.path.as_deref(), Some("authorized/actual-input.pcraft"));
        assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert!(!state.source_read_only);
        assert_eq!(state.doc.icc_profile, original.icc_profile);
        assert_eq!(state.doc.working_profile_interpretation, original.working_profile_interpretation);
        assert_eq!(state.doc.source_color_context, original.source_color_context);
        assert_eq!(state.doc.xcf_state, original.xcf_state);
        assert_eq!(native(&state.doc), raw);
        assert_eq!(scene(&state.doc), display);
        let continued = save_as(&mut reopened, "continued-threshold.pcraft", &f.quota);
        assert_eq!(reopened.active().unwrap().doc.name, "continued-threshold.pcraft");
        assert!(!reopened.undo_with_control(&ctl()).unwrap());
        assert!(!reopened.redo_with_control(&ctl()).unwrap());
        assert_eq!(reopened.active().unwrap().doc.name, "continued-threshold.pcraft");
        assert_eq!(native(&reopened.active().unwrap().doc), raw);
        let mut last = Session::new();
        last.add_owned_native_threshold_archive(load(&continued, &f.quota), "last-input.pcraft", None, &ctl()).unwrap();
        assert_eq!(last.active().unwrap().doc.name, "continued-threshold.pcraft");
        assert_eq!(native(&last.active().unwrap().doc), raw);
    }
}
#[test]
#[ignore = "UNRUN: requires pinned independently authored Histogram Threshold originals"]
fn generic_reopen_and_mutations_refuse_without_fallback_or_changed_history() {
    let f = fixture(CASES[1].0, SampleType::F32);
    let mut s = session(&f);
    let saved = save_as(&mut s, "strict.pcraft", &f.quota);
    let mut generic = Session::new();
    assert!(
        generic.add_owned_native_archive(load(&saved, &f.quota), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "strict.pcraft", None, &ctl()).is_err()
    );
    assert!(generic.documents().is_empty());
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let revision = s.active().unwrap().revision;
    let raw = native(&before);
    let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    for (command, params) in [
        ("xcf.filter.set", json!({"filterId":filter,"arguments":before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments})),
        ("xcf.filter.enable", json!({"filterId":filter,"enabled":false})),
        ("edit.clear", json!({})),
        ("edit.fill", json!({"contents":"white"})),
        ("layer.setProps", json!({"name":"not-admitted"})),
    ] {
        assert!(s.execute(command, params).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert!(history.shares_accounted_history(&s.active().unwrap().history));
        assert_eq!(s.journal, journal);
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(native(&s.active().unwrap().doc), raw);
    }
    let (root, manifest, _) = corpus();
    let name = "rgb-u16_linear.channel0.xcf";
    let (bytes, _capture) = capture(&root, name, &f.quota);
    assert_eq!(hash(&bytes), manifest[name].as_str().unwrap());
    assert!(photocraft_io::import_owned_xcf_bytes_in_quota(name, bytes, &DecodeOptions::default(), &limits(), f.quota.clone(), &ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
}
#[test]
#[ignore = "UNRUN: requires pinned independently authored Histogram Threshold originals"]
fn duplicate_cancel_work_and_real_quota_failures_preserve_existing_tab_and_fees() {
    let f = fixture(CASES[2].0, SampleType::F64);
    let mut s = session(&f);
    let _warm = native(&s.active().unwrap().doc);
    let _render = scene(&s.active().unwrap().doc);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let view = s.active().unwrap().accounted_view.as_ref().unwrap().clone();
    let journal = s.journal.clone();
    let revision = s.active().unwrap().revision;
    assert!(s.add_owned_histogram_threshold_xcf_import(f.histogram(), "duplicate", None, &ctl()).is_err());
    let baseline = f.quota.usage().unwrap();
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let deny = |_| Err(BackingError::Work);
    for c in [ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }, ReadControl { interrupt: &Interrupt::NONE, work: &deny }] {
        assert!(s.add_owned_histogram_threshold_xcf_import(f.histogram(), "denied", None, &c).is_err());
        assert_eq!(f.quota.usage().unwrap(), baseline);
    }
    let saved = save_as(&mut s, "failure-input.pcraft", &f.quota);
    let after_save = s.active().unwrap().doc.clone();
    let after_history = s.active().unwrap().history.clone();
    let stable = f.quota.usage().unwrap();
    let archive = load(&saved, &f.quota);
    let baseline = f.quota.usage().unwrap();
    let held = f.quota.reserve(QuotaKind::Index, (32 << 20) - baseline.index).unwrap();
    assert!(s.add_owned_native_threshold_archive(archive, "quota.pcraft", None, &ctl()).is_err());
    drop(held);
    assert_eq!(f.quota.usage().unwrap(), stable);
    assert!(s.active().unwrap().doc.shares_state(&after_save));
    assert!(s.active().unwrap().history.shares_accounted_history(&after_history));
    assert_eq!(s.journal, journal);
    assert_eq!(s.active().unwrap().revision, revision);
    assert!(s.active().unwrap().accounted_view.as_ref().unwrap().target().active == view.target().active);
    assert!(history.current().is_ok());
    assert_eq!(before.id, s.active().unwrap().doc.id);
    // Immediate and late native handoff failure must leave an unrelated existing tab intact.
    for limit in [0u64, 4, 32, 256] {
        let stable = f.quota.usage().unwrap();
        let archive = load(&saved, &f.quota);
        let current = s.active().unwrap().doc.clone();
        let used = AtomicU64::new(0);
        let charge = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > limit { Err(BackingError::Cancelled) } else { Ok(()) };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
        assert!(s.add_owned_native_threshold_archive(archive, "cancel.pcraft", None, &c).is_err());
        assert!(current.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.documents().len(), 1);
        assert_eq!(f.quota.usage().unwrap(), stable);
    }
}
