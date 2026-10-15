//! UNRUN configured stock continuation through real selected readers and the
//! existing Headless/CLI command seam. Test JSON/file captures are external
//! harness allocations, not assertions of product writer/provider accounting.
use super::*;
use photocraft_doc::{
    LayerContent,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_raster::{
    Interrupt,
    backing::{NativePlaneDescriptor, ReadControl},
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-configured-headless-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn source(kind: usize) -> Vec<u8> {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let (path, pin) = match kind {
        0 => {
            assert_eq!(
                hash(&std::fs::read(base.join("color-balance-evidence/SHA256SUMS.json")).unwrap()),
                "2f2ec198f19fb27e34f5d5e6f6b37b270f3249db06f768247481abf3dcd93833"
            );
            ("color-balance-evidence/rgb-double_non_linear.author.xcf", "8f750218080d918b1c11686f46303e7b7710dd184c5fe8258135756eb3863f1d")
        }
        1 => ("curve-config-matrix/levels-trc1-clamps00.xcf", "3e71bb5ffd4a63db51e739bca98f607330ae824c3750db1f68ca3bec1a8a31d4"),
        _ => ("curve-config-matrix/free256-rgb-u8.xcf", "2b92c7013049ccef9618985b7b9af3e6a491b92e15c7946fe736010074d83ce4"),
    };
    let bytes = std::fs::read(base.join(path)).unwrap();
    assert_eq!(hash(&bytes), pin);
    bytes
}
fn native(h: &Headless) -> (NativePlaneDescriptor, String) {
    let doc = &h.session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("actual native owner") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coordinate in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coordinate, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(bytes) | NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
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
// Exact bounded wire used by the resident configuration command. Conversion is
// test-side expected data, not reconstruction from public GIMP proxy values.
fn wire(value: &FilterValue, depth: usize) -> Value {
    assert!(depth <= 2);
    match value {
        FilterValue::Number(ExactNumber::Float64Bits(bits)) => json!({"Number":{"Float64Bits":format!("0x{bits:016x}")}}),
        FilterValue::Config(fields) => {
            assert!(fields.len() <= 5);
            json!({"Config":fields.iter().map(|field|json!({"name":field.name,"value":wire(&field.value,depth+1)})).collect::<Vec<_>>()})
        }
        FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } => {
            assert!(*free);
            assert!(points_f64_bits.is_empty());
            assert!(point_types.is_empty());
            assert_eq!(samples_f64_bits.len(), 256);
            json!({"Curve":{"free":true,"points_f64_bits":[],"point_types":[],"samples_f64_bits":samples_f64_bits.iter().map(|bits|format!("0x{bits:016x}")).collect::<Vec<_>>()}})
        }
        _ => panic!("only established configured types"),
    }
}
fn config(h: &Headless) -> &FilterValue {
    &h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|v| v.name == "config").unwrap().value
}
fn patch(id: &str, config: Value) -> Value {
    json!({"filterId":id,"arguments":[{"name":"config","value":config}]})
}
fn changed(kind: usize, mut value: Value) -> Value {
    match kind {
        0 => {
            let slot = &mut value["Config"][0]["value"]["Config"][0]["value"]["Number"]["Float64Bits"];
            *slot = json!(if slot.as_str() == Some("0x3fc0000000000000") { "0xbfc0000000000000" } else { "0x3fc0000000000000" });
            value["Config"][0]["value"]["Config"][1]["value"]["Number"]["Float64Bits"] = json!("0x8000000000000000");
        }
        1 => {
            let slot = &mut value["Config"][0]["value"]["Config"][2]["value"]["Number"]["Float64Bits"];
            *slot = json!(if slot.as_str() == Some("0x3ff4000000000000") { "0x3fe8000000000000" } else { "0x3ff4000000000000" });
        }
        _ => {
            let samples = &mut value["Config"][0]["value"]["Curve"]["samples_f64_bits"];
            samples[128] = json!(if samples[128].as_str() == Some("0x3fd0000000000000") { "0x3fe8000000000000" } else { "0x3fd0000000000000" });
            samples[0] = json!("0x8000000000000000");
        }
    }
    value
}

#[test]
#[ignore = "UNRUN: three independently saved configured sources, canonical wire edits and native continuation"]
fn configured_exact_hex_edits_one_undo_noop_redo_and_native_continuation_preserve_backing() {
    for kind in 0..3 {
        let dir = Directory::new();
        let original = source(kind);
        let source_path = dir.0.join("original.xcf");
        std::fs::write(&source_path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&source_path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_point_storage());
        let quota = before.accounted().unwrap().quota().clone();
        let raw = native(&h);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let old = wire(config(&h), 0);
        let next = changed(kind, old.clone());
        assert_ne!(old, next);
        assert_eq!(h.command_run("xcf.filter.set", patch(&id, next.clone())).unwrap()["updated"], true);
        assert_eq!(h.session.active().unwrap().history.past_len(), 1);
        assert_eq!(wire(config(&h), 0), next);
        let reply = h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        assert!(reply.quota().shares_ledger(&quota));
        let row = reply.value()["arguments"].as_array().unwrap().iter().find(|v| v["name"] == "config").unwrap();
        assert_eq!(row["state"]["value"], next);
        assert_eq!(row["state"]["structuredBits"], "canonical hexadecimal64");
        drop(reply);
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        assert_eq!(h.command_run("xcf.filter.set", patch(&id, old)).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert_eq!(wire(config(&h), 0), next);
        let edited = h.session.active().unwrap().doc.clone();
        assert!(edited.accounted().unwrap().quota().shares_ledger(&quota));
        assert_eq!(edited.layers[0].content, before.layers[0].content);
        assert_eq!(native(&h), raw);
        assert!(h.save_owned_native_active(Some(&source_path), Some("pcraft"), &ctl()).is_err());
        let destination = dir.0.join("configured.pcraft");
        h.save_owned_native_active(Some(&destination), Some("pcraft"), &ctl()).unwrap();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&destination, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        assert!(!current.source_read_only);
        assert_eq!(current.doc.xcf_state, edited.xcf_state);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(wire(config(&reopened), 0), next);
        assert_eq!(native(&reopened), raw);
        let enabled = edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled;
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":!enabled})).unwrap()["updated"], true);
        assert_eq!(std::fs::read(source_path).unwrap(), original);
    }
}

#[test]
#[ignore = "UNRUN: configured source host refusal, authorization and malformed nested wire"]
fn configured_start_authorizer_and_closed_wire_refusals_preserve_snapshot_and_journal() {
    for kind in 0..3 {
        let dir = Directory::new();
        let path = dir.0.join("original.xcf");
        std::fs::write(&path, source(kind)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let journal = h.session.journal.len();
        let revision = h.session.active().unwrap().revision;
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let old = wire(config(&h), 0);
        for command in ["layer.setProps", "xcf.filter.set", "xcf.filter.enable"] {
            assert!(matches!(h.command_start(command,patch(&id,old.clone()),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous")));
            assert!(!h.session.has_jobs());
        }
        let mut malformed = old.clone();
        malformed["Config"][0]["name"] = json!("unrecognized-selector");
        assert!(h.command_run("xcf.filter.set", patch(&id, malformed)).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny configured edit".into())));
        assert!(h.command_run("xcf.filter.set", patch(&id, changed(kind, old))).is_err());
        assert!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
    }
}
