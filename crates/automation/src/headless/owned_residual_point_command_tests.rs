//! UNRUN residual scalar Headless/CLI continuation through the genuine typed
//! source selector. External fixture file buffers are harness captures; the
//! production reader, paid response and native writer retain actual fees.
use super::*;
use photocraft_doc::{
    LayerContent,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
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
        let p = std::env::temp_dir().join(format!("photocraft-residual-headless-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn source(posterize: bool) -> Vec<u8> {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let (path, pin) = if posterize {
        ("operations/gimp-posterize/operation-gimp-posterize.xcf", "22dd717b5ca0863521f20791705746d104ee6a35fa90806759bb47d96c16b9c3")
    } else {
        assert_eq!(
            hash(&std::fs::read(base.join("desaturate-precision-evidence/SHA256SUMS.json")).unwrap()),
            "cee6be9680c0331b03c5cfb2a3d81758e3da91b7462fdfdbaf40f429762274b9"
        );
        ("desaturate-precision-evidence/rgb-double_non_linear.xcf", "f769510784a2fa4169651703021c958c4409c3ade569abd62f8de10cb8544c38")
    };
    let bytes = std::fs::read(base.join(path)).unwrap();
    assert_eq!(hash(&bytes), pin);
    bytes
}
fn native(h: &Headless) -> (photocraft_raster::backing::NativePlaneDescriptor, String) {
    let doc = &h.session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("real native owner") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
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
fn argument(h: &Headless) -> (&str, &FilterValue) {
    let node = &h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let value = node.arguments.first().unwrap();
    (&value.name, &value.value)
}
fn patch(id: &str, name: &str, value: Value) -> Value {
    json!({"filterId":id,"arguments":[{"name":name,"value":value}]})
}

#[test]
#[ignore = "UNRUN: actual pinned residual originals, Signed32/Enum/Bool edits and paid native continuation"]
fn signed_posterize_and_enum_desaturate_registered_edits_keep_noop_redo_and_native_storage() {
    for posterize in [true, false] {
        let dir = Directory::new();
        let original = source(posterize);
        let path = dir.0.join("original.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_point_storage());
        let raw = native(&h);
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        let (name, old) = argument(&h);
        let name = name.to_owned();
        let old_wire = serde_json::to_value(old).unwrap();
        let next = match old {
            FilterValue::Number(ExactNumber::Signed { width: 32, value }) if posterize => {
                json!({"Number":{"Signed":{"width":32,"value":if *value==5 {7} else {5}}}})
            }
            FilterValue::Enum { domain, value } if !posterize => {
                assert_eq!(domain, "GimpDesaturateMode");
                json!({"Enum":{"domain":domain,"value":if *value==0 {1} else {0}}})
            }
            _ => panic!("independent source must retain expected Signed32/Enum type"),
        };
        assert_eq!(h.command_run("xcf.filter.set", patch(&id, &name, next.clone())).unwrap()["updated"], true);
        h.command_run("edit.undo", json!({})).unwrap();
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        assert_eq!(h.command_run("xcf.filter.set", patch(&id, &name, old_wire)).unwrap()["updated"], false);
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", json!({})).unwrap();
        assert_eq!(serde_json::to_value(argument(&h).1).unwrap(), next);
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
        let edited = h.session.active().unwrap().doc.clone();
        assert_eq!(native(&h), raw);
        let inspection = h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        assert_eq!(inspection.value()["arguments"][0]["state"]["value"], next);
        drop(inspection);
        let destination = dir.0.join("continued.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
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
        assert_eq!(native(&reopened), raw);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}

#[test]
#[ignore = "UNRUN: residual typed owners preserve nonwaiting and engine authorization fences"]
fn residual_nonwaiting_and_authorizer_refusals_never_change_current_owner() {
    for posterize in [true, false] {
        let dir = Directory::new();
        let path = dir.0.join("original.xcf");
        std::fs::write(&path, source(posterize)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let journal = h.session.journal.len();
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        for command in ["layer.setProps", "xcf.filter.set", "xcf.filter.enable"] {
            assert!(
                matches!(h.command_start(command,json!({"malformed":[[[[[0]]]]]}),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
            );
            assert!(!h.session.has_jobs());
        }
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny residual Point edit".into())));
        let (name, value) = argument(&h);
        let exact = patch(&id, name, serde_json::to_value(value).unwrap());
        for (command, params) in
            [("xcf.filter.set", exact), ("xcf.filter.enable", json!({"filterId":id,"enabled":false})), ("layer.setProps", json!({"name":"denied"}))]
        {
            assert!(h.command_run(command, params).is_err());
        }
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.journal.len(), journal);
    }
}
