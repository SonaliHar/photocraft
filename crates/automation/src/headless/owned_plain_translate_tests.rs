//! UNRUN actual pinned implicit plain sources through synchronous authorized
//! Move and writable native continuation. Temporary source/archive files are
//! external harness captures, not accounting or translated-source evidence.
use super::*;
use photocraft_doc::{Document, LayerContent, Rect};
use photocraft_raster::{Interrupt, backing::ReadControl};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn cases() -> Vec<(PathBuf, Value, Vec<u8>)> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("precision-matrix");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "fc1bb8ff0635609a890860725d8449d18c0ffbec1a39b329f70fc23a8611a7d5");
    let data: Value = serde_json::from_slice(&bytes).unwrap();
    let ids = ["rgb-u8_non_linear", "rgb-u16_non_linear", "rgb-float_non_linear", "gray-u8_non_linear", "gray-u16_non_linear", "gray-float_non_linear"];
    let rows: Vec<_> = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| ids.contains(&row["id"].as_str().unwrap()))
        .map(|row| {
            let source = std::fs::read(root.join(row["input"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&source), row["input_sha256"].as_str().unwrap());
            let state = &row["state_before"];
            assert!(state["assigned_icc_sha256"].is_null());
            assert_eq!(state["layers"].as_array().unwrap().len(), 1);
            assert!(state["layers"][0]["filters"].as_array().unwrap().is_empty());
            let plane = &state["layers"][0]["native"];
            let raw = std::fs::read(root.join(plane["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&raw), plane["sha256"].as_str().unwrap());
            assert_eq!(raw.len() as u64, plane["bytes"].as_u64().unwrap());
            (root.clone(), row.clone(), raw)
        })
        .collect();
    assert_eq!(rows.len(), 6);
    rows
}
fn layer(doc: &Document) -> &photocraft_doc::BackedLayer {
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.layers[0].mask.is_none());
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("actual imported native owner") };
    backed
}
fn raw_current(doc: &Document) -> Vec<u8> {
    let backed = layer(doc);
    let rect = backed.operative_bounds();
    let bpp = backed.format().bytes_per_pixel();
    let mut raw = vec![0; rect.width() as usize * rect.height() as usize * bpp];
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let at = ((y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize) * bpp;
            backed.checked_view().read_logical_pixel_native_into(x, y, &mut raw[at..at + bpp], &ctl()).unwrap();
        }
    }
    raw
}
fn retained(before: &Document, current: &Document, dx: i32, dy: i32) {
    let a = layer(before);
    let b = layer(current);
    let old = a.operative_bounds();
    let rect = Rect::new(old.x0 + dx, old.y0 + dy, old.x1 + dx, old.y1 + dy);
    assert_eq!(b.operative_bounds(), rect);
    assert_eq!(b.format(), a.format());
    assert_eq!(b.default_bytes(), a.default_bytes());
    assert_eq!(raw_current(current), raw_current(before));
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.sample_encoding, before.sample_encoding);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    assert_eq!(
        (&current.layers[0].name, current.layers[0].visible, current.layers[0].opacity.to_bits()),
        (&before.layers[0].name, before.layers[0].visible, before.layers[0].opacity.to_bits())
    );
    let mut state = before.xcf_state.as_ref().unwrap().clone();
    state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(before.layers[0].id)).unwrap().raster_bounds = Some(rect);
    assert_eq!(current.xcf_state.as_ref().unwrap(), &state);
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-plain-move-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN:six real implicit plain RGB/Gray NL sources; registered Move rounding/target/gesture/authorization/sync/history and native continuation"]
fn plain_move_headless_preserves_source_samples_and_synchronous_policy() {
    for (root, row, mut raw) in cases() {
        let name = row["input"].as_str().unwrap();
        let original = std::fs::read(root.join(name)).unwrap();
        let dir = Directory::new();
        let path = dir.0.join("original.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let id = before.layers[0].id;
        let source = h.session.active().unwrap().source.clone();
        if cfg!(target_endian = "big") {
            for sample in raw.chunks_exact_mut(before.depth.bytes()) {
                sample.reverse();
            }
        }
        assert_eq!(raw_current(&before), raw);
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        assert!(
            matches!(h.command_start("layer.translate",json!({"layer":id.0,"dx":1}),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
        );
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
        assert!(!h.session.has_jobs());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("plain move denied".into())));
        assert!(h.command_start("layer.translate", json!({"layer":id.0,"dx":1}), true).unwrap_err().to_string().contains("plain move denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(
            h.command_start("layer.translate", json!({"layer":before.layers[0].name,"dx":1.49,"dy":-0.5,"units":"px","coalesce":"own-move"}), true).unwrap(),
            Value::Null
        );
        retained(&before, &h.session.active().unwrap().doc, 1, -1);
        h.command_run("layer.translate", json!({"layer":id.0,"dx":0,"dy":1,"coalesce":"own-move"})).unwrap();
        let moved = h.session.active().unwrap().doc.clone();
        retained(&before, &moved, 1, 0);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        h.command_run("layer.translate", json!({"layer":"nonexistent","dx":0.49,"dy":-0.49,"coalesce":"own-move"})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&moved));
        for params in [json!({"layer":id.0,"dx":1,"target":"mask"}), json!({"layer":id.0,"dx":1,"units":"cm"}), json!({"layer":id.0,"dx":null,"units":"pt"})] {
            assert!(h.command_run("layer.translate", params).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&moved));
        }
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 1);
        assert_eq!(query.value()["filters"]["total"], 0);
        drop(query);
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Translated current.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        assert!(h.session.active().unwrap().source_read_only);
        let saved_name = h.session.active().unwrap().doc.name.clone();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(!current.source_read_only);
        assert_eq!(current.doc.name, saved_name);
        retained(&before, &current.doc, 1, 0);
        reopened.command_run("layer.translate", json!({"dx":-1,"units":"px"})).unwrap();
        retained(&before, &reopened.session.active().unwrap().doc, 0, 0);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(name)).unwrap(), original);
    }
}
