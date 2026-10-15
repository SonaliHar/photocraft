//! UNRUN six genuine-source scalar Point continuation drafts. Source and archive
//! Cursor buffers are external harness captures, not claims of paid production
//! buffer ownership. The reader, typed model and native provider remain paid.
use crate::Session;
use photocraft_doc::{LayerContent, xcf_filters::FilterValue};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Case {
    path: &'static str,
    sha: &'static str,
    operation: &'static str,
}
const CASES: [Case; 6] = [
    Case {
        path: "exposure-geometry-evidence/rgb-double_non_linear.negative-x.source.xcf",
        sha: "cc29e9aaae63f97f4f1e01ad49fabc9759b165e9f1496bc70edf728605acf53b",
        operation: "gegl:exposure",
    },
    Case { path: "rgb-clip-hdr-evidence/clip.source.xcf", sha: "33dce24ca99ee3ae8c60b05eb16398498099e7353e7e1698a2efde2e3b3d02a4", operation: "gegl:rgb-clip" },
    Case {
        path: "point-unpremultiply-evidence/unpremultiply-rgb-float_linear.small.source.xcf",
        sha: "21e4faa0915abbf9f0a89ac4c80584392f98849514d8b870ef9db0deb1a9cb79",
        operation: "gegl:unpremultiply",
    },
    Case {
        path: "operations/gegl-mono-mixer/operation-gegl-mono-mixer.xcf",
        sha: "dc7009be42d698b448efd45c6b38a18f80a816a57033a44c031e6fa1d8d5880a",
        operation: "gegl:mono-mixer",
    },
    Case {
        path: "mono-mixer-edits-evidence/mono.source.xcf",
        sha: "32151370e3768e004248e01a59576150ef000572e992d48e52667ff3cb1709b7",
        operation: "gegl:mono-mixer",
    },
    Case {
        path: "operations/gegl-value-invert/operation-gegl-value-invert.xcf",
        sha: "9cd1650e6ddfaba7f3dd8dc9e5ae5ac2cec46b962685973dbe687b6235f6916f",
        operation: "gegl:value-invert",
    },
];
fn imported(name: &str, bytes: &[u8], quota: &BackingQuota) -> photocraft_io::OwnedPointXcfImport {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    match photocraft_io::import_owned_xcf_from_reader_in_quota(
        name,
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        quota.clone(),
        &ctl(),
    )
    .unwrap()
    {
        photocraft_io::OwnedXcfImport::Point(owner) => owner,
        _ => panic!("pinned residual source must select Point without fallback"),
    }
}
fn native(session: &Session) -> (photocraft_raster::backing::NativePlaneDescriptor, String) {
    let doc = &session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("paid native raster") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => digest.update(b),
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    (layer.descriptor(), digest.finalize().iter().map(|v| format!("{v:02x}")).collect())
}

fn source(case: &Case) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let bytes = std::fs::read(root.join(case.path)).unwrap();
    assert_eq!(hash(&bytes), case.sha);
    bytes
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn session(case: &Case, bytes: &[u8], quota: &BackingQuota) -> Session {
    let owner = imported(case.path, bytes, quota);
    let mut session = Session::new();
    session.add_owned_point_xcf_import(&owner, case.path, Some(case.path), &ctl()).unwrap();
    drop(owner); // The published model must retain its actual input/native fees.
    assert!(session.active().unwrap().doc.accounted().unwrap().quota().shares_ledger(quota));
    let state = session.active().unwrap();
    assert!(state.source_read_only);
    assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::xcf());
    assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some(case.path));
    assert!(state.path.is_none());
    let xcf = state.doc.xcf_state.as_ref().unwrap();
    assert_eq!(xcf.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
    assert_eq!(xcf.filter_stacks[0].nodes[0].operation.name, case.operation);
    session
}
fn exact_arguments(session: &Session) -> Option<Value> {
    let node = &session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    if node.arguments.is_empty() {
        return None;
    }
    // All six original packets have flat F32/Bool arguments or no arguments.
    // Serialize their actual words, never a reconstructed coefficient/proxy.
    assert!(
        node.arguments.iter().all(|a| matches!(a.value, FilterValue::Bool(_) | FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))))
    );
    let arguments: Vec<_> = node.arguments.iter().map(|a| json!({"name":a.name,"value":a.value})).collect();
    Some(json!({"filterId":node.id.0.to_string(),"arguments":arguments}))
}

#[test]
#[ignore = "UNRUN: six pinned scalar domains, exact argument no-op, activation/history and paid native continuation"]
fn scalar_sources_continue_through_registered_edits_and_native_reopen_without_payload_changes() {
    for case in &CASES {
        let bytes = source(case);
        let quota = quota();
        let mut session = session(case, &bytes, &quota);
        let initial = session.active().unwrap().doc.clone();
        let source_identity = session.active().unwrap().source.clone();
        let raw = native(&session);
        let node = &initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        let enabled = node.enabled;
        let original_enable = json!({"filterId":id,"enabled":enabled});
        let changed_enable = json!({"filterId":id,"enabled":!enabled});
        let initial_history = session.active().unwrap().history.clone();
        if let Some(params) = exact_arguments(&session) {
            assert_eq!(session.execute("xcf.filter.set", params).unwrap()["updated"], false);
            assert!(session.active().unwrap().doc.shares_state(&initial));
            assert!(session.active().unwrap().history.shares_accounted_history(&initial_history));
        }
        assert_eq!(session.execute("xcf.filter.enable", changed_enable.clone()).unwrap()["updated"], true, "{}", case.path);
        let changed = session.active().unwrap().doc.clone();
        let changed_xcf = changed.xcf_state.as_ref().unwrap();
        assert_eq!(changed_xcf.filter_stacks[0].nodes[0].id, node.id);
        assert_eq!(changed_xcf.filter_stacks[0].nodes[0].arguments, node.arguments);
        assert_eq!(changed_xcf.filter_stacks[0].revision, initial.xcf_state.as_ref().unwrap().filter_stacks[0].revision + 1);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&initial), "exactly one undo for {}", case.path);
        let history = session.active().unwrap().history.clone();
        let revision = session.active().unwrap().revision;
        if let Some(params) = exact_arguments(&session) {
            assert_eq!(session.execute("xcf.filter.set", params).unwrap()["updated"], false);
        }
        assert_eq!(session.execute("xcf.filter.enable", original_enable).unwrap()["updated"], false);
        assert!(session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(session.active().unwrap().revision, revision);
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&changed));
        assert_eq!(session.execute("xcf.filter.enable", changed_enable).unwrap()["updated"], false);
        assert_eq!(native(&session), raw);
        assert_eq!(session.active().unwrap().source, source_identity);
        let current = session.active().unwrap().doc.clone();
        assert_eq!(current.depth, initial.depth);
        assert_eq!(current.icc_profile, initial.icc_profile);
        assert_eq!(current.working_profile_interpretation, initial.working_profile_interpretation);
        assert_eq!(current.source_color_context, initial.source_color_context);
        // Deliberately no with_current_point_storage oracle: legacy opaque Mono
        // and Value schedules are internal None; serialized precision is Canonical.
        assert_eq!(current.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        // Harness-owned buffers; only the real writer/reader/model leases are
        // asserted to use the catalog quota, not these external capture Vecs.
        let saved = photocraft_format::owned_document::save_current_document_archive(&current, Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive, "continued.pcraft", None, &ctl()).unwrap();
        let reopened_state = reopened.active().unwrap();
        assert_eq!(reopened_state.doc.xcf_state, current.xcf_state);
        assert_eq!(reopened_state.doc.depth, initial.depth);
        assert_eq!(reopened_state.doc.icc_profile, initial.icc_profile);
        assert_eq!(reopened_state.doc.source_color_context, initial.source_color_context);
        assert_eq!(reopened_state.doc.working_profile_interpretation, initial.working_profile_interpretation);
        assert!(reopened_state.doc.accounted().unwrap().quota().shares_ledger(&quota));
        assert_eq!(reopened_state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert_eq!(reopened_state.source.as_ref().unwrap().label, "continued.pcraft");
        assert!(reopened_state.source.as_ref().unwrap().path.is_none());
        assert!(!reopened_state.source_read_only);
        assert!(reopened_state.path.is_none(), "persisted XCF paths never become runtime write authority");
        assert_eq!(native(&reopened), raw);
        let reopened_before = reopened.active().unwrap().doc.clone();
        assert_eq!(reopened.execute("xcf.filter.enable", json!({"filterId":id,"enabled":enabled})).unwrap()["updated"], true);
        reopened.execute("edit.undo", json!({})).unwrap();
        assert!(reopened.active().unwrap().doc.shares_state(&reopened_before));
        assert_eq!(native(&reopened), raw);
        assert_eq!(hash(&bytes), case.sha);
        assert_eq!(hash(&source(case)), case.sha);
    }
}

#[test]
#[ignore = "UNRUN: six actual scalar sources, unknown typed argument and cancellation preserve publication, journal and paid owners"]
fn scalar_refusals_leave_native_source_history_and_ledger_unchanged() {
    for case in &CASES {
        let bytes = source(case);
        let quota = quota();
        let mut session = session(case, &bytes, &quota);
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.clone();
        let revision = session.active().unwrap().revision;
        let journal = session.journal.clone();
        let source_identity = session.active().unwrap().source.clone();
        let raw = native(&session); // Pin/cache setup precedes the quota baseline.
        let usage = quota.usage().unwrap();
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let invalid = json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"not-an-original-argument","value":{"Bool":true}}]});
        assert!(session.execute("xcf.filter.set", invalid).is_err());
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        let enable = json!({"filterId":node.id.0.to_string(),"enabled":!node.enabled});
        assert!(crate::accounted_commands::dispatch(&mut session, "xcf.filter.enable", &enable, &control).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.journal, journal);
        assert_eq!(session.active().unwrap().source, source_identity);
        assert_eq!(native(&session), raw);
        assert_eq!(quota.usage().unwrap(), usage);
    }
}
