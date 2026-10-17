//! UNRUN retained plain CURRENT project discovery. The base fixture passes real
//! synthetic XCF bytes through the owned reader; additional CURRENT metadata is
//! explicitly paid and never presented as original-file lineage or authority.
use super::*;
use crate::Services;
use photocraft_doc::{
    accounted_document::AccountedDocument,
    item_identity::{ItemRef, ItemSetId, ResourceId},
    xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_engine::Session;
use photocraft_raster::backing::BackingError;
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    any::Any,
    io::{Cursor, Write},
    sync::Arc,
};
#[path = "../../xcf/tests/common/mod.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod xcf;

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn imported() -> photocraft_io::OwnedPlainXcfImport {
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 });
    // Pay the bounded input fixture itself before generating its Vec. The real
    // reader creates and retains its own separately charged immutable capture.
    let capture = q.reserve(QuotaKind::Resident, 8192).unwrap();
    let bytes = xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    assert!(bytes.capacity() <= 8192);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 32 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_staged_plain_from_reader_in_quota(
        "synthetic-current.xcf",
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &control(),
    )
    .unwrap();
    drop(bytes);
    drop(capture);
    imported
}
fn app() -> (PhotocraftApp, BackingQuota) {
    let imported = imported();
    let q = imported.document().quota().clone();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "synthetic-current.xcf", Some("authorized/original.xcf"), &control()).unwrap();
    drop(imported);
    (PhotocraftApp::new(session, Services::default()), q)
}
struct CurrentMetadata {
    _original: AccountedDocument,
    _metadata: QuotaLease,
}
fn metadata_app() -> (PhotocraftApp, BackingQuota) {
    let imported = imported();
    let original = imported.document().clone();
    let q = original.quota().clone();
    // This small known CURRENT-only fixture is paid before any test-only clone,
    // Strings, Vecs, resource payload or Arc construction. Its original owner
    // retains all native/source/profile allocations on the same actual quota.
    let metadata = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut document = original.document().clone();
    let layer = document.layers[0].id;
    let state = document.xcf_state.as_mut().unwrap();
    for i in 0..3 {
        state.resources.push(ResourceReference {
            id: ResourceId(100 + i),
            kind: ResourceKind::Pattern,
            name: format!("CURRENT resource {i}"),
            collection: None,
            internal: true,
            locator: Some(format!("never-read://resource/{i}")),
            content_hash: Some("a".repeat(64)),
            owned_bytes: Some(Arc::new(vec![i as u8; 17])),
            resolution: None,
        });
    }
    for i in 0..2 {
        state.item_sets.push(ItemSet {
            id: ItemSetId(200 + i),
            name: format!("CURRENT set {i}"),
            family: ItemFamily::Layer,
            rule: if i == 0 { ItemSetRule::Named } else { ItemSetRule::Glob("CURRENT*".into()) },
            members: vec![ItemRef::Layer(layer)],
        });
    }
    state.selection.selected = vec![ItemRef::Layer(layer)];
    state.selection.active = Some(ItemRef::Layer(layer));
    let retention: Arc<dyn Any + Send + Sync> = Arc::new(CurrentMetadata { _original: original, _metadata: metadata });
    let owner = AccountedDocument::from_owned(document, retention, q.clone(), &control()).unwrap();
    owner.validate_plain_xcf_inspection(&control()).unwrap();
    drop(imported);
    let mut session = Session::new();
    session.add_accounted_document(owner, None, &control()).unwrap();
    (PhotocraftApp::new(session, Services::default()), q)
}
fn request(
    command: &str,
    params: Value,
    id: Value,
    limit: usize,
) -> (ControlRequest, std::sync::mpsc::Receiver<Value>, std::sync::mpsc::Receiver<OwnedFilterInspectWire>) {
    ControlRequest::with_retained_query("engine.execute", json!({"command":command,"params":params}), id, limit)
}
fn inspect(app: &mut PhotocraftApp, params: Value) -> OwnedFilterInspectWire {
    let (req, wake, wire) = request("xcf.inspect", params, json!({"query":"current","sequence":7}), 1 << 20);
    assert!(matches!(crate::control::handle(app, &egui::Context::default(), &req), Outcome::Deferred));
    assert!(wake.try_recv().unwrap().is_null(), "paid carrier is queued before ordinary wake");
    wire.try_recv().unwrap()
}
fn decoded(wire: &OwnedFilterInspectWire) -> Value {
    serde_json::from_slice(wire.bytes()).unwrap()
}
fn denied(outcome: Outcome) {
    let Outcome::Done(value) = outcome else { panic!("expected bounded ordinary refusal") };
    assert_eq!(value["ok"], false);
}

#[test]
#[ignore = "UNRUN: actual synthetic reader -> owned plain UI project query"]
fn real_plain_reader_default_null_and_empty_pages_keep_model_source_and_no_sync() {
    let (mut app, q) = app();
    let old = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.clone();
    let source = app.session.active().unwrap().source.clone();
    let revision = app.session.active().unwrap().revision;
    let view = app.session.active().unwrap().view_revision;
    let journal = app.session.journal.clone();
    assert!(source.is_some());
    assert!(app.session.active().unwrap().source_read_only);
    // sync_views would repair this deliberately missing view; inspection must
    // not enter app.run/job synchronization just to discover current metadata.
    app.ui.views.clear();
    let baseline = q.usage().unwrap();
    for params in [Value::Null, json!({}), json!({"offset":0,"limit":128}), json!({"offset":100000,"limit":1})] {
        let wire = inspect(&mut app, params.clone());
        let result = decoded(&wire);
        assert_eq!(result["ok"], true);
        assert_eq!(result["id"], json!({"query":"current","sequence":7}));
        assert_eq!(result["result"]["format"], "xcf");
        assert_eq!(result["result"]["filters"]["total"], 0);
        let entries = result["result"]["items"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), if params.get("offset").and_then(Value::as_u64) == Some(100000) { 0 } else { 1 });
        assert!(q.usage().unwrap().index > baseline.index);
        drop(wire);
        assert_eq!(q.usage().unwrap(), baseline);
        assert!(app.ui.views.is_empty());
    }
    let state = app.session.active().unwrap();
    assert!(old.shares_state(&state.doc));
    assert!(history.shares_accounted_history(&state.history));
    assert_eq!(state.source, source);
    assert_eq!(state.revision, revision);
    assert_eq!(state.view_revision, view);
    assert_eq!(app.session.journal, journal);
    assert_eq!(state.doc.xcf_state, old.xcf_state);
    assert_eq!(state.doc.source_color_context, old.source_color_context);
    let photocraft_doc::LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!("not actual backed input") };
    let mut pixel = [0; 4];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &control()).unwrap();
    assert_eq!(pixel, [31, 61, 91, 255]);
}

#[test]
#[ignore = "UNRUN: paid CURRENT-only resource/set/selection pages, no original lineage"]
fn paid_current_resource_set_and_selection_pages_are_inert_and_boundary_exact() {
    let (mut app, q) = metadata_app();
    let old = app.session.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    for offset in 0..=3 {
        let wire = inspect(&mut app, json!({"offset":offset,"limit":1}));
        let value = decoded(&wire);
        let result = &value["result"];
        for (page, total) in [("resources", 3), ("savedSets", 2), ("items", 1)] {
            assert_eq!(result[page]["total"], total);
            assert_eq!(result[page]["entries"].as_array().unwrap().len(), usize::from(offset < total));
        }
        assert_eq!(result["selection"]["selected"]["entries"].as_array().unwrap().len(), usize::from(offset == 0));
        if offset < 3 {
            let resource = &result["resources"]["entries"][0];
            assert_eq!(resource["accessGranted"], false);
            assert_eq!(resource["hasLocator"], true);
            assert_eq!(resource["ownedBytes"], 17);
            assert!(resource.get("locator").is_none());
            assert!(resource.get("payload").is_none());
        }
        if offset == 0 {
            assert_eq!(result["savedSets"]["entries"][0]["members"]["entries"].as_array().unwrap().len(), 1);
        }
        if offset == 1 {
            assert!(result["savedSets"]["entries"][0]["members"]["entries"].as_array().unwrap().is_empty());
        }
        drop(wire);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    assert!(old.shares_state(&app.session.active().unwrap().doc));
    assert_eq!(old.xcf_state, app.session.active().unwrap().doc.xcf_state);
}

#[test]
#[ignore = "UNRUN: actual typed anonymous selection guard remains held during project query"]
fn typed_selection_and_redo_are_unchanged_by_plain_project_discovery() {
    let (mut app, q) = app();
    app.session.execute("select.all", json!({})).unwrap();
    let selected = app.session.active().unwrap().doc.clone();
    assert!(selected.selection.is_some());
    app.session.execute("edit.undo", json!({})).unwrap();
    let history = app.session.active().unwrap().history.clone();
    let before = app.session.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    drop(inspect(&mut app, Value::Null));
    assert!(before.shares_state(&app.session.active().unwrap().doc));
    assert!(history.shares_accounted_history(&app.session.active().unwrap().history));
    assert_eq!(q.usage().unwrap(), baseline);
    app.session.execute("edit.redo", json!({})).unwrap();
    assert!(selected.shares_state(&app.session.active().unwrap().doc));
    let baseline = q.usage().unwrap();
    let history = app.session.active().unwrap().history.clone();
    drop(inspect(&mut app, Value::Null));
    assert!(selected.shares_state(&app.session.active().unwrap().doc));
    assert!(history.shares_accounted_history(&app.session.active().unwrap().history));
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
#[ignore = "UNRUN: original project command reaches both authorizers; no query fallback"]
fn original_project_authorizers_are_not_filter_query_aliases_and_denial_has_no_wire() {
    static SESSION_CALLS: AtomicU64 = AtomicU64::new(0);
    fn allow(id: &str, params: &Value) -> photocraft_engine::Result<()> {
        assert_eq!(id, "xcf.inspect");
        assert_eq!(params, &json!({"offset":0,"limit":1}));
        SESSION_CALLS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn deny(id: &str, params: &Value) -> photocraft_engine::Result<()> {
        allow(id, params)?;
        Err(photocraft_engine::EngineError::Other("project denied".into()))
    }
    SESSION_CALLS.store(0, Ordering::Relaxed);
    for gate in 0..3 {
        let (mut app, q) = app();
        let old = app.session.active().unwrap().doc.clone();
        let baseline = q.usage().unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        let seen = calls.clone();
        app.services.automation_command = Some(Box::new(move |id, params| {
            assert_eq!(id, "xcf.inspect");
            assert_eq!(params, &json!({"offset":0,"limit":1}));
            seen.fetch_add(1, Ordering::Relaxed);
            if gate == 0 { Err("workspace denied".into()) } else { Ok(()) }
        }));
        app.services.automation_authorize = Some(if gate == 1 { deny } else { allow });
        let (req, _, wire) = request("xcf.inspect", json!({"offset":0,"limit":1}), json!(2), 1 << 20);
        let outcome = crate::control::handle(&mut app, &egui::Context::default(), &req);
        if gate < 2 {
            denied(outcome);
            assert!(wire.try_recv().is_err());
        } else {
            assert!(matches!(outcome, Outcome::Deferred));
            drop(wire.try_recv().unwrap());
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(app.session.authorize.is_none());
        assert_eq!(q.usage().unwrap(), baseline);
        assert!(old.shares_state(&app.session.active().unwrap().doc));
    }
    assert_eq!(SESSION_CALLS.load(Ordering::Relaxed), 2);
}

#[test]
#[ignore = "UNRUN: plain filter/alias/value-only/hostile pagination refusals"]
fn plain_filter_query_alias_mismatch_and_value_only_transport_do_not_escape() {
    let (mut app, q) = app();
    let old = app.session.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    for params in [
        json!({"command":"xcf.filter.inspect","params":{"filterId":"1"}}),
        json!({"command":"xcf.inspect","id":"xcf.filter.inspect"}),
        json!({"command":"xcf.inspect","params":{"offset":100001}}),
        json!({"command":"xcf.inspect","params":{"limit":0}}),
        json!({"command":"xcf.inspect","params":{"offset":-1}}),
        json!({"command":"xcf.inspect","params":{"unknown":true}}),
        json!({"command":"xcf.inspect","wait":"true"}),
    ] {
        let (req, _, wire) = ControlRequest::with_retained_query("engine.execute", params, Value::Null, 1 << 20);
        denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
        assert!(wire.try_recv().is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let (req, _) = ControlRequest::new("engine.execute", json!({"command":"xcf.inspect"}));
    denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
    assert!(old.shares_state(&app.session.active().unwrap().doc));
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
#[ignore = "UNRUN: real quota/deadline/control failure and disconnected paid replies"]
fn quota_cancel_control_and_receiver_failures_preserve_state_and_refund() {
    let (mut app, q) = app();
    let old = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.clone();
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (32 << 20) - baseline.index).unwrap();
    let full = q.usage().unwrap();
    let (req, _, wire) = request("xcf.inspect", Value::Null, Value::Null, 1 << 20);
    denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
    assert!(wire.try_recv().is_err());
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    let (mut req, _, wire) = request("xcf.inspect", Value::Null, Value::Null, 1 << 20);
    req.deadline = Some(std::time::Instant::now());
    denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
    assert!(wire.try_recv().is_err());
    let failed = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(photocraft_engine::accounted_plain_xcf_inspect::prepare(&app.session, &Value::Null, &failed).is_err());
    let (req, _, wire) = request("xcf.inspect", Value::Null, Value::Null, 32);
    denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
    assert!(wire.try_recv().is_err());
    let (req, _, wire) = request("xcf.inspect", Value::Null, Value::Null, 1 << 20);
    drop(wire);
    denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
    assert_eq!(q.usage().unwrap(), baseline);
    let (req, _, wire) = request("xcf.inspect", Value::Null, Value::Null, 1 << 20);
    assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &req), Outcome::Deferred));
    assert!(q.usage().unwrap().index > baseline.index);
    drop(wire);
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(old.shares_state(&app.session.active().unwrap().doc));
    assert!(history.shares_accounted_history(&app.session.active().unwrap().history));
}

#[test]
#[ignore = "UNRUN: successful and failing native wire writes retain actual owner through flush"]
fn owned_project_wire_keeps_reply_fee_through_write_flush_and_error_drop() {
    struct Sink {
        q: BackingQuota,
        baseline: u64,
        bytes: usize,
        fail: bool,
    }
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            assert!(self.q.usage().unwrap().resident > self.baseline);
            if self.fail {
                return Err(std::io::Error::other("deliberate write refusal"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            assert!(self.q.usage().unwrap().resident > self.baseline);
            Ok(())
        }
    }
    let (mut app, q) = app();
    let baseline = q.usage().unwrap();
    for fail in [false, true] {
        let wire = inspect(&mut app, Value::Null);
        let mut sink = Sink { q: q.clone(), baseline: baseline.resident, bytes: 0, fail };
        if fail {
            assert!(wire.write_line_to(&mut sink).is_err());
        } else {
            wire.write_line_to(&mut sink).unwrap();
            sink.flush().unwrap();
            assert_eq!(sink.bytes, wire.bytes().len() + 1);
        }
        assert!(q.usage().unwrap().resident > baseline.resident);
        drop(wire);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
