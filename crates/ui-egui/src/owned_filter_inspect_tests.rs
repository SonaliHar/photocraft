//! UNRUN retained UI/control inspection drafts over independently saved, SHA-pinned originals.
//! No fabricated current metadata is used as original-XCF qualification.
use super::*;
use crate::{PhotocraftApp, Services};
use photocraft_engine::Session;
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits};
use photocraft_xcf::{BudgetKind, DecodeOptions, Encoding, Model, ResourceLimits, Sample};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
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
struct Evidence {
    imported: photocraft_io::OwnedXcfImport,
    bytes: Arc<Vec<u8>>,
    quota: BackingQuota,
    name: String,
    _capture: QuotaLease,
}
fn evidence(histogram: bool) -> Evidence {
    let (packet, pin, operation) = if histogram {
        ("threshold-precision-evidence", "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142", "gimp:threshold")
    } else {
        ("alpha-threshold-precision-evidence", "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263", "gimp:threshold-alpha")
    };
    let sample = Sample::F64;
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires pinned full-XCF evidence corpus")).join(packet);
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), pin);
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof_bytes = std::fs::read(root.join("fresh-verification.json")).unwrap();
    assert_eq!(hash(&proof_bytes), manifest["fresh-verification.json"].as_str().unwrap());
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
                .is_some_and(|effect| effect.operation == operation && effect.state.enabled == Some(true))
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

fn session(e: &Evidence) -> Session {
    let mut s = Session::new();
    match &e.imported {
        photocraft_io::OwnedXcfImport::ThresholdAlpha(v) => {
            s.add_owned_threshold_alpha_xcf_import(v, &e.name, Some("authorized/original.xcf"), &ctl()).unwrap();
        }
        photocraft_io::OwnedXcfImport::HistogramThreshold(v) => {
            s.add_owned_histogram_threshold_xcf_import(v, &e.name, Some("authorized/original.xcf"), &ctl()).unwrap();
        }
        _ => panic!("not a supported source class"),
    }
    s
}
fn request(s: &Session) -> Value {
    json!({"filterId":s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}

fn outer(s: &Session) -> Value {
    json!({"command":"xcf.filter.inspect","params":request(s),"wait":true})
}
fn app(histogram: bool) -> (PhotocraftApp, Evidence) {
    let e = evidence(histogram);
    let s = session(&e);
    (PhotocraftApp::new(s, Services::default()), e)
}
fn denied(outcome: Outcome) {
    let Outcome::Done(value) = outcome else { panic!("expected ordinary bounded error") };
    assert_eq!(value["ok"], false);
}
#[test]
#[ignore = "UNRUN: pinned originals; no UI/runtime or TCP execution"]
fn real_owned_control_reply_is_queued_before_null_and_held_through_write_flush() {
    for histogram in [false, true] {
        let (mut app, e) = app(histogram);
        let snapshot = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.clone();
        let rev = app.session.active().unwrap().revision;
        let view = app.session.active().unwrap().view_revision;
        let source = app.session.active().unwrap().source.clone();
        let journal = app.session.journal.clone();
        let p = outer(&app.session);
        let baseline = e.quota.usage().unwrap();
        let (req, wake, wire) = ControlRequest::with_retained_query("engine.execute", p, json!({"request":"owned-inspect","sequence":7}), 1 << 20);
        assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &req), Outcome::Deferred));
        assert!(wake.try_recv().unwrap().is_null());
        let wire = wire.try_recv().unwrap();
        let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
        assert_eq!(value["id"], json!({"request":"owned-inspect","sequence":7}));
        assert_eq!(value["ok"], true);
        assert_eq!(value["result"]["filterId"], request(&app.session)["filterId"]);
        assert_eq!(value["result"]["argumentCount"], if histogram { 3 } else { 1 });
        let held = e.quota.usage().unwrap();
        assert!(held.index > baseline.index);
        assert!(held.resident > baseline.resident);
        struct Sink {
            q: BackingQuota,
            index: u64,
            written: usize,
            flushed: bool,
        }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                assert!(self.q.usage().unwrap().index > self.index);
                self.written += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                assert!(self.q.usage().unwrap().index > self.index);
                self.flushed = true;
                Ok(())
            }
        }
        let mut sink = Sink { q: e.quota.clone(), index: baseline.index, written: 0, flushed: false };
        wire.write_line_to(&mut sink).unwrap();
        sink.flush().unwrap();
        assert_eq!(sink.written, wire.bytes().len() + 1);
        assert!(sink.flushed);
        drop(wire);
        assert_eq!(e.quota.usage().unwrap(), baseline);
        let state = app.session.active().unwrap();
        assert!(state.doc.shares_state(&snapshot));
        assert!(state.history.shares_accounted_history(&history));
        assert_eq!(state.revision, rev);
        assert_eq!(state.view_revision, view);
        assert_eq!(state.source, source);
        assert_eq!(app.session.journal, journal);
        assert_eq!(state.doc.icc_profile, snapshot.icc_profile);
        assert_eq!(state.doc.xcf_state.as_ref().unwrap().filter_masks, snapshot.xcf_state.as_ref().unwrap().filter_masks);
    }
}
#[test]
#[ignore = "UNRUN: requires pinned originals"]
fn both_command_authorizers_receive_original_query_and_denial_does_not_queue_wire() {
    fn deny_session(id: &str, p: &Value) -> photocraft_engine::Result<()> {
        assert_eq!(id, "xcf.filter.inspect");
        assert!(p.get("filterId").is_some());
        Err(photocraft_engine::EngineError::Other("session inspection denied".into()))
    }
    for gate in 0..2 {
        let (mut app, e) = app(true);
        let p = outer(&app.session);
        let exact = p["params"].clone();
        let calls = Arc::new(AtomicU64::new(0));
        let seen = calls.clone();
        app.services.automation_command = Some(Box::new(move |id, p| {
            assert_eq!(id, "xcf.filter.inspect");
            assert_eq!(p, &exact);
            seen.fetch_add(1, Ordering::Relaxed);
            if gate == 0 { Err("workspace inspection denied".into()) } else { Ok(()) }
        }));
        if gate == 1 {
            app.services.automation_authorize = Some(deny_session);
        }
        let before = app.session.active().unwrap().doc.clone();
        let baseline = e.quota.usage().unwrap();
        let (req, _wake, wire) = ControlRequest::with_retained_query("engine.execute", p, json!(1), 1 << 20);
        denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(wire.try_recv().is_err());
        assert!(app.session.authorize.is_none());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(e.quota.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN: requires pinned originals"]
fn legacy_value_only_raw_session_and_query_batches_never_escape_paid_carrier() {
    let (mut app, e) = app(true);
    let before = app.session.active().unwrap().doc.clone();
    let baseline = e.quota.usage().unwrap();
    let (req, _) = ControlRequest::new("engine.execute", outer(&app.session));
    denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
    assert!(app.session.execute("xcf.filter.inspect", request(&app.session)).is_err());
    for p in [
        json!([outer(&app.session)]),
        json!({"command":"xcf.filter.inspect","params":request(&app.session),"batch":[{}]}),
        json!({"command":"xcf.filter.inspect","params":request(&app.session),"wait":"true"}),
    ] {
        let (req, _, wire) = ControlRequest::with_retained_query("engine.execute", p, Value::Null, 1 << 20);
        denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
        assert!(wire.try_recv().is_err());
    }
    assert!(app.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(e.quota.usage().unwrap(), baseline);
}
#[test]
#[ignore = "UNRUN: requires pinned originals"]
fn hostile_rpc_ids_limits_deadline_and_disconnected_transport_refund_without_publication() {
    let (mut app, e) = app(true);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.clone();
    let baseline = e.quota.usage().unwrap();
    let mut deep = Value::Null;
    for _ in 0..10 {
        deep = json!([deep]);
    }
    let auth_calls = Arc::new(AtomicU64::new(0));
    let seen = auth_calls.clone();
    app.services.automation_command = Some(Box::new(move |_, _| {
        seen.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }));
    for (index, (id, limit)) in
        [(deep, 1 << 20), (Value::Array(vec![Value::Null; 4097]), 1 << 20), (json!("x".repeat((1 << 20) + 1)), 1 << 20), (Value::Null, 32)]
            .into_iter()
            .enumerate()
    {
        let (req, _, wire) = ControlRequest::with_retained_query("engine.execute", outer(&app.session), id, limit);
        let outcome = crate::control::handle(&mut app, &egui::Context::default(), &req);
        if index < 3 {
            let Outcome::Done(value) = outcome else { panic!("rejected ID must return bounded static error") };
            assert_eq!(value.get("id"), Some(&Value::Null), "explicit null, not an absent/reinjected rejected ID");
            assert_eq!(value["ok"], false);
            assert_eq!(auth_calls.load(Ordering::Relaxed), 0, "ID refusal precedes authorizers");
        } else {
            denied(outcome);
        }
        assert!(wire.try_recv().is_err());
        assert_eq!(e.quota.usage().unwrap(), baseline);
    }
    let (mut req, _, wire) = ControlRequest::with_retained_query("engine.execute", outer(&app.session), Value::Null, 1 << 20);
    req.deadline = Some(std::time::Instant::now());
    let Outcome::Done(value) = crate::control::handle(&mut app, &egui::Context::default(), &req) else { panic!("deadline prepass must return static error") };
    assert_eq!(value.get("id"), Some(&Value::Null));
    assert_eq!(value["ok"], false);
    assert_eq!(auth_calls.load(Ordering::Relaxed), 1, "expired ID/control prepass does not run another authorizer");
    assert!(wire.try_recv().is_err());
    assert_eq!(e.quota.usage().unwrap(), baseline);
    let (req, _, wire) = ControlRequest::with_retained_query("engine.execute", outer(&app.session), Value::Null, 1 << 20);
    drop(wire);
    denied(crate::control::handle(&mut app, &egui::Context::default(), &req));
    assert_eq!(e.quota.usage().unwrap(), baseline);
    // Timeout after enqueue: the receiver owns the charge until it is dropped.
    let (req, wake, wire) = ControlRequest::with_retained_query("engine.execute", outer(&app.session), Value::Null, 1 << 20);
    assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &req), Outcome::Deferred));
    assert!(wake.try_recv().unwrap().is_null());
    assert!(e.quota.usage().unwrap().index > baseline.index);
    drop(wire);
    assert_eq!(e.quota.usage().unwrap(), baseline);
    assert!(app.session.active().unwrap().doc.shares_state(&before));
    assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
}
