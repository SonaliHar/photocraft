//! UNRUN bounded neighborhood Headless continuation from exact original sources.
//! Harness file Vecs are external; actual readers/replies/writer/native owners
//! retain production charges. This is adapter coverage, not new GIMP evidence.
use super::*;
use photocraft_doc::LayerContent;
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
const CASES: [(&str, &str); 8] = [
    ("operations/gegl-box-blur/operation-gegl-box-blur.xcf", "0dddbdcee491ce0e24aea521ce7c1de4fa0e5304a2caa9021933917a1de63b15"),
    ("box-raster-color-evidence/implicit-hdr-f64.source.xcf", "11c8158edbab33226b51c201ff167a34eeef52c53efeff601cab242ced6abdc6"),
    ("operations/gegl-median-blur/operation-gegl-median-blur.xcf", "31fcb8d83bb6e574a9dddb8b1e4222de1fa2642a34202696c29e3effa6cbe746"),
    ("median-raster-color-evidence/implicit-hdr-f64.source.xcf", "8f8001f3f31d6b9936f659ab18965a8df57f9e403297497f1d339e3539fe4442"),
    ("median-gray-color-evidence/implicit-gray-hdr-f64.source.xcf", "1149bcbe3cadc2075f4c7546a7eeaa0197f07677ce882a6dc75835483cd57cc7"),
    ("operations/gegl-motion-blur-linear/operation-gegl-motion-blur-linear.xcf", "7fd61309e9ad01f397b5127f0b51f2e2fae27f9d81c53c56305f02b6d24413e8"),
    ("operations/gegl-pixelize/operation-gegl-pixelize.xcf", "a4b746de40767cbd07467cad702cda4611c95e614c718164696ef475b2fe1f34"),
    ("gaussian-raster-color-evidence/implicit-hdr-f64.source.xcf", "e9a17b3deb40ecd066f09a0888fe5332cb54c542afae8ee0be8700ac6bc80d46"),
];
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
        let path = std::env::temp_dir().join(format!("photocraft-neighborhood-headless-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn native(h: &Headless) -> (photocraft_raster::backing::NativePlaneDescriptor, String) {
    let doc = &h.session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("genuine native owner") };
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

#[test]
#[ignore = "UNRUN: eight pinned neighborhood domains retain registered authorization/noopredo/native continuation"]
fn eight_neighborhood_sources_use_typed_adoption_and_registered_current_commands_without_fallback() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    for (relative, pin) in CASES {
        let dir = Directory::new();
        let original = std::fs::read(root.join(relative)).unwrap();
        assert_eq!(hash(&original), pin);
        let path = dir.0.join("source.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let identity = h.session.active().unwrap().source.clone();
        assert!(before.accounted().unwrap().is_readonly_point_storage());
        assert!(h.session.active().unwrap().source_read_only);
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(node.enabled);
        let id = node.id.0.to_string();
        let raw = native(&h);
        let project = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(project.value()["filters"]["total"], 1);
        drop(project);
        let inspection = h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        assert_eq!(inspection.value()["operation"]["name"], node.operation.name);
        assert_eq!(inspection.value()["arguments"].as_array().unwrap().len(), node.arguments.len());
        drop(inspection);
        let journal = h.session.journal.len();
        for command in ["layer.setProps", "xcf.filter.set", "xcf.filter.enable"] {
            assert!(
                matches!(h.command_start(command,json!({"unparsed":[[[[[0]]]]]}),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous"))
            );
        }
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny neighborhood current edit".into())));
        assert!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).is_err());
        assert!(h.command_run("layer.setProps", json!({"name":"denied"})).is_err());
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.journal.len(), journal);
        assert!(!h.session.has_jobs());
        // Naming is the existing paid metadata edit; no unsupported blur
        // coefficient domain is invented by this workflow.
        assert_eq!(h.command_run("layer.setProps", json!({"name":"Neighborhood current"})).unwrap(), Value::Null);
        assert_eq!(h.session.active().unwrap().history.past_len(), 1);
        h.command_run("edit.undo", json!({})).unwrap();
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        let edited = h.session.active().unwrap().doc.clone();
        assert_eq!(edited.xcf_state.as_ref().unwrap().filter_stacks, before.xcf_state.as_ref().unwrap().filter_stacks);
        assert_eq!(native(&h), raw);
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
        let disabled = h.session.active().unwrap().doc.clone();
        assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(native(&h), raw);
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&edited));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&disabled));
        let output = dir.0.join("current.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, identity);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        assert!(!current.source_read_only);
        assert_eq!(current.doc.xcf_state, disabled.xcf_state);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(native(&reopened), raw);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        assert_eq!(native(&reopened), raw);
        assert_eq!(reopened.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, node.arguments);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
