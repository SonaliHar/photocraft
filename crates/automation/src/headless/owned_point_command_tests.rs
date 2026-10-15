//! UNRUN actual-source host routing and native continuation. Harness file bytes
//! are external captures; production readers/writers retain their real charges.
use super::*;
use photocraft_doc::LayerContent;
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
        let p = std::env::temp_dir().join(format!("photocraft-point-command-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn original() -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-geometry-evidence");
    assert_eq!(hash(&std::fs::read(root.join("batch-manifest.json")).unwrap()), "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72");
    let bytes = std::fs::read(root.join("brightness-gray-u16_linear.negative-x.author.xcf")).unwrap();
    assert_eq!(hash(&bytes), "819379f87a2d4ff41fffc9ee31741264e5c6ebc9a757dd52a5f14701c9aa555c");
    bytes
}
// Stream exact padded native tiles into a fixed-size digest; no uncharged
// whole-plane materialization, narrowing or derived-display substitution.
fn native(h: &Headless) -> (photocraft_raster::backing::NativePlaneDescriptor, String) {
    let d = &h.session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("actual native owner") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(bytes) => digest.update(bytes),
            NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
        }
    }
    for mask in &d.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    (layer.descriptor(), digest.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn patch(id: &str, bits: u32) -> Value {
    json!({"filterId":id,"arguments":[{"name":"brightness","value":{"Number":{"Float32Bits":bits}}}]})
}

#[test]
#[ignore = "UNRUN: pinned Point original, registered host edits and paid native continuation"]
fn registered_point_props_filter_edit_noop_redo_and_native_continuation() {
    let dir = Directory::new();
    let bytes = original();
    let source = dir.0.join("source.xcf");
    std::fs::write(&source, &bytes).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current(&source, &ctl()).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    let raw = native(&h);
    let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let id = node.id.0.to_string();
    let current = node.arguments.iter().find(|v| v.name == "brightness").unwrap().value.clone();
    let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(old)) = current else {
        panic!("actual F32 source argument")
    };
    let next = if old == 0.125f32.to_bits() { (-0.125f32).to_bits() } else { 0.125f32.to_bits() };
    assert_eq!(h.command_run("layer.setProps", json!({"name":"Point current","opacity":0.5,"visible":true})).unwrap(), Value::Null);
    assert_eq!(h.command_run("xcf.filter.set", patch(&id, next)).unwrap()["updated"], true);
    h.command_run("edit.undo", json!({})).unwrap();
    let revision = h.session.active().unwrap().revision;
    let history = h.session.active().unwrap().history.clone();
    assert_eq!(h.command_run("xcf.filter.set", patch(&id, old)).unwrap()["updated"], false);
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
    h.command_run("edit.redo", json!({})).unwrap();
    assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
    assert_eq!(native(&h), raw);
    let edited = h.session.active().unwrap().doc.clone();
    let target = dir.0.join("continued.pcraft");
    assert!(h.save_owned_native_active(Some(&source), Some("pcraft"), &ctl()).is_err());
    h.save_owned_native_active(Some(&target), Some("pcraft"), &ctl()).unwrap();
    let mut reopened = Headless::trusted_local();
    drop(reopened.open_owned_native_keep_current(&target, &ctl()).unwrap());
    let state = reopened.session.active().unwrap();
    assert!(state.doc.accounted().unwrap().is_readonly_point_storage());
    assert!(!state.source_read_only);
    assert_eq!(state.doc.xcf_state, edited.xcf_state);
    assert_eq!(state.doc.icc_profile, before.icc_profile);
    assert_eq!(state.doc.working_profile_interpretation, before.working_profile_interpretation);
    assert_eq!(native(&reopened), raw);
    assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
}

#[test]
#[ignore = "UNRUN: Point host nonwaiting refusal and retained engine authorization"]
fn point_host_start_and_denied_transport_do_not_publish() {
    let dir = Directory::new();
    let source = dir.0.join("source.xcf");
    std::fs::write(&source, original()).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current(&source, &ctl()).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    let revision = h.session.active().unwrap().revision;
    let journal = h.session.journal.len();
    for id in ["layer.setProps", "xcf.filter.set", "xcf.filter.enable"] {
        assert!(
            matches!(h.command_start(id, json!({"malformed":[[[[[0]]]]]}), false), Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
        );
        assert!(!h.session.has_jobs());
    }
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny actual Point command".into())));
    for (id, params) in [
        ("layer.setProps", json!({"name":"denied"})),
        ("xcf.filter.enable", json!({"filterId":before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string(),"enabled":false})),
    ] {
        assert!(h.command_run(id, params).is_err());
    }
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.journal.len(), journal);
}
