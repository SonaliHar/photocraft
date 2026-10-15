//! UNRUN64 representative authored assigned source domains from256 pinned
//! records. Translation/disabled-link states are synthetic CURRENT controls,
//! never independently GIMP-authored linkage or translated-source evidence.
use super::owned_analytic_geometry_command_tests::{cases, ctl, operators, profiles};
use super::*;
use photocraft_doc::{Document, LayerContent, Rect, item_identity::ItemRef};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn backed(layer: &photocraft_doc::Layer) -> &photocraft_doc::BackedLayer {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("actual native leaf") };
    b
}
fn native(layer: &photocraft_doc::Layer) -> Vec<u8> {
    let b = backed(layer);
    let r = b.operative_bounds();
    let n = b.format().bytes_per_pixel();
    let mut out = vec![0; r.width() as usize * r.height() as usize * n];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * n;
            b.checked_view().read_logical_pixel_native_into(x, y, &mut out[at..at + n], &ctl()).unwrap();
        }
    }
    out
}
fn foreground(doc: &Document) -> photocraft_doc::LayerId {
    assert_eq!(doc.layers.len(), 2);
    doc.layers.iter().find(|l| l.mask.is_some()).unwrap().id
}
fn retained(before: &Document, current: &Document, dx: i32, dy: i32) {
    let id = foreground(before);
    let old = before.layer(id).unwrap();
    let new = current.layer(id).unwrap();
    let a = backed(old);
    let b = backed(new);
    let r = a.operative_bounds();
    let moved = Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy);
    assert_eq!(b.operative_bounds(), moved);
    assert_eq!(b.format(), a.format());
    assert_eq!(b.default_bytes(), a.default_bytes());
    assert_eq!(native(old), native(new));
    let ma = old.mask.as_ref().unwrap();
    let mb = new.mask.as_ref().unwrap();
    assert!(ma.linked && mb.linked);
    assert_eq!(ma.enabled, mb.enabled);
    assert_eq!(ma.surface.format(), mb.surface.format());
    assert_eq!(ma.surface.default_bytes(), mb.surface.default_bytes());
    assert_eq!(ma.surface.try_read_interleaved(r).unwrap(), mb.surface.try_read_interleaved(moved).unwrap());
    assert_eq!((ma.density.to_bits(), ma.feather.to_bits()), (mb.density.to_bits(), mb.feather.to_bits()));
    for layer in &before.layers {
        let after = current.layer(layer.id).unwrap();
        assert_eq!(
            (&layer.name, layer.visible, layer.opacity.to_bits(), layer.fill_opacity.to_bits()),
            (&after.name, after.visible, after.opacity.to_bits(), after.fill_opacity.to_bits())
        );
        if layer.id != id {
            assert_eq!(backed(layer).descriptor(), backed(after).descriptor());
            assert_eq!(native(layer), native(after));
        }
    }
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.sample_encoding, before.sample_encoding);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let mut state = before.xcf_state.as_ref().unwrap().clone();
    for item in &mut state.items {
        if item.item == ItemRef::Layer(id) {
            item.raster_bounds = Some(moved);
        }
        if let Some(mask) = item.mask.as_mut().filter(|m| m.owner == ItemRef::Layer(id)) {
            item.raster_bounds = Some(moved);
            mask.bounds = moved;
            mask.offset = [moved.x0, moved.y0];
        }
    }
    assert_eq!(current.xcf_state.as_ref().unwrap(), &state);
}
fn literals(root: &std::path::Path, row: &Value, doc: &Document) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    for layer in row["layers"].as_array().unwrap() {
        for blob in std::iter::once(&layer["native"]).chain(if layer["mask"].is_null() { None } else { Some(&layer["mask"]["native"]) }) {
            let mut bytes = std::fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
            let hash = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
            assert_eq!(hash, blob["sha256"].as_str().unwrap());
            assert_eq!(bytes.len() as u64, blob["bytes"].as_u64().unwrap());
            if cfg!(target_endian = "big") {
                for sample in bytes.chunks_exact_mut(doc.depth.bytes()) {
                    sample.reverse();
                }
            }
            result.push(bytes);
        }
    }
    assert_eq!(result.len(), 3);
    result
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-assigned-move-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN:64 representative source domains; enabled/disabled linked CURRENT Move, exact source/native/profile/mask custody, sync/auth/history/native continuation"]
fn linked_assigned_move_headless_keeps_source_and_current_native_custody() {
    let selected: Vec<_> = cases().into_iter().filter(|(root, row)| root.ends_with("left-top") && row["id"].as_str().unwrap().ends_with("-source")).collect();
    assert_eq!(selected.len(), 64);
    for (root, row) in selected {
        for disable in [false, true] {
            let name = row["source_file"].as_str().unwrap();
            let original = std::fs::read(root.join(name)).unwrap();
            let dir = Directory::new();
            let path = dir.0.join("source.xcf");
            std::fs::write(&path, &original).unwrap();
            let mut h = Headless::trusted_local();
            drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
            let initial = h.session.active().unwrap().doc.clone();
            let source = h.session.active().unwrap().source.clone();
            let id = foreground(&initial);
            assert!(initial.layer(id).unwrap().mask.as_ref().unwrap().linked, "actual retained CURRENT policy flag");
            assert!(initial.layer(id).unwrap().mask.as_ref().unwrap().enabled);
            profiles(&initial, &row);
            operators(&initial, &row);
            let raw = literals(&root, &row, &initial);
            assert_eq!(native(initial.layer(id).unwrap()), raw[0]);
            let bg = initial.layers.iter().find(|l| l.id != id).unwrap();
            assert_eq!(native(bg), raw[2]);
            assert_eq!(
                initial.layer(id).unwrap().mask.as_ref().unwrap().surface.try_read_interleaved(backed(initial.layer(id).unwrap()).operative_bounds()).unwrap(),
                raw[1]
            );
            let backdrop = bg.id;
            let mask_id = initial
                .xcf_state
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find_map(|i| match (i.item, i.mask.as_ref()) {
                    (ItemRef::Mask(mid), Some(m)) if m.owner == ItemRef::Layer(id) => Some(mid),
                    _ => None,
                })
                .unwrap();
            if disable {
                let journal = h.session.journal.len();
                h.command_run("xcf.mask.enabled.set", json!({"maskId":mask_id.0.to_string(),"enabled":false})).unwrap();
                assert_eq!(h.session.journal.len(), journal);
            }
            let before = h.session.active().unwrap().doc.clone();
            let revision = h.session.active().unwrap().revision;
            let journal = h.session.journal.len();
            assert!(
                matches!(h.command_start("layer.translate",json!({"layer":id.0,"dx":1}),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
            );
            assert_eq!(h.session.active().unwrap().revision, revision);
            assert_eq!(h.session.journal.len(), journal);
            assert!(!h.session.has_jobs());
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("assigned move denied".into())));
            assert!(h.command_start("layer.translate", json!({"layer":id.0,"dx":1}), true).unwrap_err().to_string().contains("assigned move denied"));
            h.session.authorize = None;
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            let past = h.session.active().unwrap().history.past_len();
            assert_eq!(
                h.command_start("layer.translate", json!({"layer":before.layer(id).unwrap().name,"dx":1.49,"dy":-0.5,"coalesce":"assigned-move"}), true)
                    .unwrap(),
                Value::Null
            );
            retained(&before, &h.session.active().unwrap().doc, 1, -1);
            h.command_run("layer.translate", json!({"layer":id.0,"target":"mask","dy":1,"coalesce":"assigned-move"})).unwrap();
            let moved = h.session.active().unwrap().doc.clone();
            retained(&before, &moved, 1, 0);
            profiles(&moved, &row);
            assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
            assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Move"));
            h.command_run("edit.undo", json!({})).unwrap();
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            let history = h.session.active().unwrap().history.clone();
            let revision = h.session.active().unwrap().revision;
            h.command_run("layer.translate", json!({"layer":"absent","dx":0.49,"coalesce":"assigned-move"})).unwrap();
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.active().unwrap().revision, revision);
            h.command_run("edit.redo", json!({})).unwrap();
            assert!(h.session.active().unwrap().doc.shares_state(&moved));
            for params in
                [json!({"layer":backdrop.0,"dx":1}), json!({"layer":id.0,"dx":1,"target":"quickMask"}), json!({"layer":id.0,"dx":1,"target":{"channel":0}})]
            {
                assert!(h.command_run("layer.translate", params).is_err());
                assert!(h.session.active().unwrap().doc.shares_state(&moved));
            }
            let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
            assert_eq!(query.value()["items"]["total"], 3);
            assert_eq!(query.value()["filters"]["total"], 0);
            drop(query);
            assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
            let output = dir.0.join("Assigned moved.pcraft");
            h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
            let saved_name = h.session.active().unwrap().doc.name.clone();
            assert_eq!(h.session.active().unwrap().source, source);
            assert!(h.session.active().unwrap().source_read_only);
            let mut reopened = Headless::trusted_local();
            drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
            let current = reopened.session.active().unwrap();
            assert!(!current.source_read_only);
            assert_eq!(current.doc.name, saved_name);
            retained(&before, &current.doc, 1, 0);
            profiles(&current.doc, &row);
            reopened.command_run("layer.translate", json!({"layer":id.0,"dx":-1})).unwrap();
            retained(&before, &reopened.session.active().unwrap().doc, 0, 0);
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert_eq!(std::fs::read(root.join(name)).unwrap(), original);
        }
    }
}
