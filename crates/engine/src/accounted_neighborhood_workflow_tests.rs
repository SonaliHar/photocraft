//! UNRUN eight genuine-source neighborhood continuation drafts. Source and archive
//! Cursor buffers are external harness captures, not claims of paid production
//! buffer ownership. The reader, typed model and native provider remain paid.
use crate::Session;
use crate::accounted_native_open::is_current_point_candidate;
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
const CASES: [(&str, &str, &str); 8] = [
    ("operations/gegl-box-blur/operation-gegl-box-blur.xcf", "0dddbdcee491ce0e24aea521ce7c1de4fa0e5304a2caa9021933917a1de63b15", "gegl:box-blur"),
    ("box-raster-color-evidence/implicit-hdr-f64.source.xcf", "11c8158edbab33226b51c201ff167a34eeef52c53efeff601cab242ced6abdc6", "gegl:box-blur"),
    ("operations/gegl-median-blur/operation-gegl-median-blur.xcf", "31fcb8d83bb6e574a9dddb8b1e4222de1fa2642a34202696c29e3effa6cbe746", "gegl:median-blur"),
    ("median-raster-color-evidence/implicit-hdr-f64.source.xcf", "8f8001f3f31d6b9936f659ab18965a8df57f9e403297497f1d339e3539fe4442", "gegl:median-blur"),
    ("median-gray-color-evidence/implicit-gray-hdr-f64.source.xcf", "1149bcbe3cadc2075f4c7546a7eeaa0197f07677ce882a6dc75835483cd57cc7", "gegl:median-blur"),
    (
        "operations/gegl-motion-blur-linear/operation-gegl-motion-blur-linear.xcf",
        "7fd61309e9ad01f397b5127f0b51f2e2fae27f9d81c53c56305f02b6d24413e8",
        "gegl:motion-blur-linear",
    ),
    ("operations/gegl-pixelize/operation-gegl-pixelize.xcf", "a4b746de40767cbd07467cad702cda4611c95e614c718164696ef475b2fe1f34", "gegl:pixelize"),
    ("gaussian-raster-color-evidence/implicit-hdr-f64.source.xcf", "e9a17b3deb40ecd066f09a0888fe5332cb54c542afae8ee0be8700ac6bc80d46", "gegl:gaussian-blur"),
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

fn source(path: &str, pin: &str) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let bytes = std::fs::read(root.join(path)).unwrap();
    assert_eq!(hash(&bytes), pin);
    bytes
}
fn session(path: &str, bytes: &[u8]) -> Session {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let imported = imported(path, bytes, &q);
    let mut s = Session::new();
    s.add_owned_point_xcf_import(&imported, path, Some(path), &ctl()).unwrap();
    drop(imported);
    assert!(s.active().unwrap().doc.quota().unwrap().shares_ledger(&q));
    s
}
fn exact_arguments(s: &Session) -> Value {
    use photocraft_doc::xcf_filters::ExactNumber;
    let node = &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let args: Vec<_> = node
        .arguments
        .iter()
        .map(|a| {
            let value = match &a.value {
                FilterValue::Number(ExactNumber::Float64Bits(bits)) => json!({"Number":{"Float64Bits":format!("0x{bits:016x}")}}),
                other => serde_json::to_value(other).unwrap(),
            };
            json!({"name":a.name,"value":value})
        })
        .collect();
    json!({"filterId":node.id.0.to_string(),"arguments":args,"enabled":node.enabled})
}
#[test]
#[ignore = "UNRUN: eight genuine neighborhood domains, exact args/activation no-op, rename undo/redo and paid native continuation"]
fn original_neighborhood_sources_publish_only_after_full_current_validation_and_continue_natively() {
    for (path, pin, operation) in CASES {
        let bytes = source(path, pin);
        let mut s = session(path, &bytes);
        let initial = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let xcf = initial.xcf_state.as_ref().unwrap();
        let node = &xcf.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, operation);
        assert!(node.enabled);
        assert!(is_current_point_candidate(&initial, &ctl()).unwrap());
        let id = node.id.0.to_string();
        let original_wire = exact_arguments(&s);
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        assert_eq!(s.execute("xcf.filter.set", original_wire.clone()).unwrap()["updated"], false);
        assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(s.active().unwrap().doc.shares_state(&initial));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        let source_identity = s.active().unwrap().source.clone();
        s.execute("layer.setProps", json!({"name":"Current neighborhood continuation"})).unwrap();
        let changed = s.active().unwrap().doc.clone();
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&initial));
        let history = s.active().unwrap().history.clone();
        assert_eq!(s.execute("xcf.filter.set", original_wire).unwrap()["updated"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&changed));
        assert_eq!(native(&s), raw);
        assert_eq!(changed.depth, initial.depth);
        assert_eq!(changed.mode, initial.mode);
        assert_eq!(changed.icc_profile, initial.icc_profile);
        assert_eq!(changed.source_color_context, initial.source_color_context);
        assert_eq!(changed.working_profile_interpretation, initial.working_profile_interpretation);
        assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks[0], xcf.filter_stacks[0]);
        assert_eq!(changed.xcf_state.as_ref().unwrap().filter_masks, xcf.filter_masks);
        assert_eq!(changed.xcf_state.as_ref().unwrap().provenance, xcf.provenance);
        assert_eq!(s.active().unwrap().source, source_identity);
        assert!(s.active().unwrap().source_read_only);
        assert_eq!(xcf.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        // External capture Vecs do not purport to carry production quota proof.
        let q = changed.quota().unwrap().clone();
        let saved = photocraft_format::owned_document::save_current_document_archive(&changed, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive, "neighborhood.pcraft", None, &ctl()).unwrap();
        let current = reopened.active().unwrap();
        assert_eq!(current.doc.xcf_state, changed.xcf_state);
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        assert!(current.doc.quota().unwrap().shares_ledger(&q));
        assert_eq!(current.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert!(current.source.as_ref().unwrap().path.is_none());
        assert!(!current.source_read_only);
        assert_eq!(native(&reopened), raw);
        assert_eq!(reopened.execute("xcf.filter.set", exact_arguments(&reopened)).unwrap()["updated"], false);
        assert_eq!(hash(&bytes), pin);
    }
}
#[test]
#[ignore = "UNRUN: eight neighborhood domains refuse wrong arguments, stack edits and cancellation without publication"]
fn neighborhood_refusal_cannot_fallback_or_mutate_current_model() {
    for (path, pin, _) in CASES {
        let bytes = source(path, pin);
        let mut s = session(path, &bytes);
        let before = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        let journal = s.journal.clone();
        let q = before.quota().unwrap().clone();
        let usage = q.usage().unwrap();
        for (command, params) in [
            ("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"not-an-original-argument","value":{"Bool":true}}]})),
            ("xcf.filter.move", json!({"filterId":id,"index":0})),
            ("xcf.filter.remove", json!({"filterId":id})),
        ] {
            assert!(s.execute(command, params).is_err(), "{path}");
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(s.journal, journal);
        }
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(crate::accounted_commands::dispatch(&mut s, "xcf.filter.enable", &json!({"filterId":id,"enabled":true}), &control).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(native(&s), raw);
        assert_eq!(q.usage().unwrap(), usage);
    }
}

#[test]
#[ignore = "UNRUN: eight original neighborhood domains, disabled history/noopredo/native continuation/re-enable with exact raw conservation"]
fn disabled_neighborhood_lifecycle_retains_current_words_and_real_source_owners() {
    for (path, pin, _) in CASES {
        let bytes = source(path, pin);
        let mut s = session(path, &bytes);
        let enabled = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let xcf = enabled.xcf_state.as_ref().unwrap();
        let node = &xcf.filter_stacks[0].nodes[0];
        assert!(node.enabled);
        let id = node.id.0.to_string();
        let source_identity = s.active().unwrap().source.clone();
        let revision = s.active().unwrap().revision;
        assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap(), json!({"filterId":id,"updated":true}));
        let disabled = s.active().unwrap().doc.clone();
        let next = disabled.xcf_state.as_ref().unwrap();
        let changed = &next.filter_stacks[0].nodes[0];
        assert!(!changed.enabled);
        assert_eq!(changed.id, node.id);
        assert_eq!(changed.operation, node.operation);
        assert_eq!(changed.arguments, node.arguments);
        assert_eq!(changed.opacity_f64_bits, node.opacity_f64_bits);
        assert_eq!(changed.mask, node.mask);
        assert_eq!(changed.composition, node.composition);
        assert_eq!(changed.auxiliary, node.auxiliary);
        assert_eq!(changed.source_clipping, node.source_clipping);
        assert_eq!(changed.source_region, node.source_region);
        assert_eq!(next.filter_stacks[0].owner, xcf.filter_stacks[0].owner);
        assert_eq!(next.filter_stacks[0].revision, xcf.filter_stacks[0].revision + 1);
        assert_eq!(next.filter_masks, xcf.filter_masks);
        assert_eq!(next.items, xcf.items);
        assert_eq!(next.opaque, xcf.opaque);
        assert_eq!(next.provenance, xcf.provenance);
        assert_eq!(next.current_filter_output_precision, xcf.current_filter_output_precision);
        assert_eq!(disabled.depth, enabled.depth);
        assert_eq!(disabled.mode, enabled.mode);
        assert_eq!(disabled.icc_profile, enabled.icc_profile);
        assert_eq!(disabled.source_color_context, enabled.source_color_context);
        assert_eq!(disabled.working_profile_interpretation, enabled.working_profile_interpretation);
        assert_eq!(native(&s), raw);
        assert_eq!(s.active().unwrap().source, source_identity);
        assert!(s.active().unwrap().source_read_only);
        assert_eq!(s.active().unwrap().revision, revision + 1);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&enabled));
        let history = s.active().unwrap().history.clone();
        let before_noop = s.active().unwrap().revision;
        assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert_eq!(s.execute("xcf.filter.set", exact_arguments(&s)).unwrap()["updated"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, before_noop);
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&disabled));
        let history = s.active().unwrap().history.clone();
        assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(s.execute("xcf.filter.set", exact_arguments(&s)).unwrap()["updated"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        // Source/archive Vecs remain external captures, not fabricated buffer leases.
        let q = disabled.quota().unwrap().clone();
        let saved = photocraft_format::owned_document::save_current_document_archive(&disabled, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive, "disabled-neighborhood.pcraft", None, &ctl()).unwrap();
        let reopened_before = reopened.active().unwrap().doc.clone();
        assert_eq!(reopened_before.xcf_state, disabled.xcf_state);
        assert!(reopened_before.accounted().unwrap().is_readonly_point_storage());
        assert!(reopened_before.quota().unwrap().shares_ledger(&q));
        assert_eq!(reopened_before.depth, disabled.depth);
        assert_eq!(reopened_before.icc_profile, disabled.icc_profile);
        assert_eq!(reopened_before.source_color_context, disabled.source_color_context);
        assert_eq!(reopened_before.working_profile_interpretation, disabled.working_profile_interpretation);
        assert_eq!(native(&reopened), raw);
        let state = reopened.active().unwrap();
        assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert!(state.source.as_ref().unwrap().path.is_none());
        assert!(!state.source_read_only);
        assert!(state.path.is_none());
        assert_eq!(reopened.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        assert!(reopened.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(native(&reopened), raw);
        assert!(reopened.undo_with_control(&ctl()).unwrap());
        assert!(reopened.active().unwrap().doc.shares_state(&reopened_before));
        assert_eq!(hash(&bytes), pin);
        assert_eq!(hash(&source(path, pin)), pin);
    }
}

#[test]
#[ignore = "UNRUN: eight disabled neighborhood domains preserve model/history/journal/native owners on cancelled or quota-denied re-enable"]
fn disabled_neighborhood_failed_reenable_preserves_every_publication_owner() {
    for (path, pin, _) in CASES {
        let bytes = source(path, pin);
        let mut s = session(path, &bytes);
        let id = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        let journal = s.journal.clone();
        let source_identity = s.active().unwrap().source.clone();
        let q = before.quota().unwrap().clone();
        let usage = q.usage().unwrap();
        let params = json!({"filterId":id,"enabled":true});
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(crate::accounted_commands::dispatch(&mut s, "xcf.filter.enable", &params, &control).is_err());
        let fee = q.reserve(photocraft_raster::backing::quota::QuotaKind::Index, (32 << 20) - q.usage().unwrap().index).unwrap();
        assert!(s.execute("xcf.filter.enable", params).is_err());
        drop(fee);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.journal, journal);
        assert_eq!(s.active().unwrap().source, source_identity);
        assert_eq!(native(&s), raw);
        assert_eq!(q.usage().unwrap(), usage);
    }
}
