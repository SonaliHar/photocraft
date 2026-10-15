//! UNRUN original Gaussian registered Headless workflow. Harness files remain
//! external captures; real readers/writer/response owners supply actual fees.
use super::*;
use photocraft_doc::{Document, LayerContent};
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
        let path = std::env::temp_dir().join(format!("photocraft-gaussian-original-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn native(doc: &Document) -> (photocraft_raster::backing::NativePlaneDescriptor, Vec<u8>, String) {
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("actual native layer") };
    let r = layer.operative_bounds();
    let mut bytes = vec![0; r.width() as usize * r.height() as usize * 4];
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in r.tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => digest.update(b),
        }
        let part = coord.rect().intersect(&r);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * 4;
                pin.read_pixel_native_into(x, y, &mut bytes[at..at + 4], &ctl()).unwrap();
            }
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    (layer.descriptor(), bytes, digest.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
#[test]
#[ignore = "UNRUN: pinned original Gaussian source/edit actual authorization/synchronous wire/undo/noopredo/disabled CURRENT native continuation"]
fn original_gaussian_headless_commands_preserve_source_and_current_native_custody() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: Value = serde_json::from_slice(include_bytes!("../../../compose/tests/gimp_gaussian_hashes.json")).unwrap();
    for (path, pin) in hashes.as_object().unwrap() {
        assert_eq!(hash(&std::fs::read(root.join(path)).unwrap()), pin.as_str().unwrap());
    }
    for (file, initial, other) in [("filter-owner-layer.xcf", 2.25f32, 3.75f32), ("filter-owner-layer.edit.xcf", 3.75f32, 2.25f32)] {
        let dir = Directory::new();
        let original = std::fs::read(root.join(file)).unwrap();
        let path = dir.0.join("original.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let identity = h.session.active().unwrap().source.clone();
        assert!(h.session.active().unwrap().source_read_only);
        let raw = native(&before);
        assert_eq!(raw.1, std::fs::read(root.join("filter-owner-layer.source.0.native.bin")).unwrap());
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let patch = |value: f32| json!({"filterId":id,"arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":value.to_bits()}}}]});
        let query = h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gegl:gaussian-blur");
        drop(query);
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        assert!(matches!(h.command_start("xcf.filter.set",patch(other),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous")));
        assert!(!h.session.has_jobs());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("Gaussian denied".into())));
        assert!(h.command_run("xcf.filter.set", patch(other)).unwrap_err().to_string().contains("Gaussian denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal, journal);
        assert_eq!(h.command_run("xcf.filter.set", patch(other)).unwrap()["updated"], true);
        let edited = h.session.active().unwrap().doc.clone();
        assert_eq!(native(&edited), raw);
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        assert_eq!(h.command_run("xcf.filter.set", patch(initial)).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&edited));
        // Disabled is a CURRENT edit, not independent disabled-source evidence.
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
        let disabled = h.session.active().unwrap().doc.clone();
        assert_eq!(native(&disabled), raw);
        let output = dir.0.join("Gaussian current.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, identity);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.xcf_state, disabled.xcf_state);
        assert_eq!(native(current), raw);
        assert_eq!(current.icc_profile, before.icc_profile);
        assert_eq!(current.source_color_context, before.source_color_context);
        assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        assert_eq!(reopened.command_run("xcf.filter.set", patch(initial)).unwrap()["updated"], true);
        assert_eq!(native(&reopened.session.active().unwrap().doc), raw);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), original);
    }
}
