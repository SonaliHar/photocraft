//! UNRUN24 actual isolated Normal groups through authorized Headless typed
//! source/native services. Harness files are external captures, not fee proof.
use super::*;
use photocraft_doc::{Document, Layer, LayerContent, SampleEncoding};
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn blob(root: &std::path::Path, value: &Value) {
    let b = std::fs::read(root.join(value["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&b), value["sha256"].as_str().unwrap());
    assert_eq!(b.len() as u64, value["bytes"].as_u64().unwrap());
}
fn original_layer(root: &std::path::Path, row: &Value, depth: usize) {
    assert!(depth <= 1);
    blob(root, &row["native"]);
    if !row["mask"].is_null() {
        blob(root, &row["mask"]["native"]);
    }
    if let Some(children) = row["children"].as_array() {
        assert_eq!(children.len(), 2);
        for child in children {
            original_layer(root, child, depth + 1);
        }
    }
}
fn cases() -> Vec<(PathBuf, Value)> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("accounted-normal-group-evidence");
    let b = std::fs::read(root.join("observations.json")).unwrap();
    let pins: Value = serde_json::from_str(include_str!("../../../compose/tests/gimp_accounted_group_hashes.json")).unwrap();
    assert_eq!(hash(&b), pins["observations.json"].as_str().unwrap());
    let data: Value = serde_json::from_slice(&b).unwrap();
    let rows = data["records"].as_array().unwrap();
    assert_eq!(rows.len(), 24);
    rows.iter()
        .map(|row| {
            blob(&root, &row["source"]);
            assert_eq!(row["layers"].as_array().unwrap().len(), 2);
            for layer in row["layers"].as_array().unwrap() {
                original_layer(&root, layer, 0);
            }
            blob(&root, &row["projection"]["native"]);
            (root.clone(), row.clone())
        })
        .collect()
}
fn tree_words(doc: &Document) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<String>, depth: usize) {
        assert!(depth <= 1);
        for layer in layers {
            let mut d = Sha256::new();
            d.update(format!("{:?}", (layer.id, &layer.name, layer.blend, layer.visible, layer.opacity.to_bits(), layer.fill_opacity.to_bits())).as_bytes());
            match &layer.content {
                LayerContent::Group(group) => {
                    assert_eq!(depth, 0);
                    assert_eq!(group.children.len(), 2);
                    d.update(format!("group{:?}", (group.expanded, &group.artboard)).as_bytes());
                }
                LayerContent::BackedRaster(backed) => {
                    d.update(format!("{:?}", backed.descriptor()).as_bytes());
                    d.update(backed.descriptor().default_bytes());
                    for coord in backed.descriptor().bounds().tiles() {
                        let pin = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
                        match pin.encoded() {
                            NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => d.update(b),
                        }
                    }
                }
                _ => panic!("genuine Group.children/backed native source, never cached group Raster"),
            }
            if let Some(mask) = &layer.mask {
                d.update(
                    format!(
                        "{:?}",
                        (mask.surface.format(), mask.surface.tile_bounds(), mask.enabled, mask.linked, mask.density.to_bits(), mask.feather.to_bits())
                    )
                    .as_bytes(),
                );
                d.update(mask.surface.default_bytes());
                for (coord, tile) in mask.surface.tiles() {
                    d.update(format!("{coord:?}").as_bytes());
                    d.update(tile.bytes());
                }
            }
            out.push(d.finalize().iter().map(|v| format!("{v:02x}")).collect());
            if let LayerContent::Group(group) = &layer.content {
                walk(&group.children, out, depth + 1);
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.layers, &mut out, 0);
    assert_eq!(out.len(), 4);
    out
}
fn observed_tree(root: &std::path::Path, doc: &Document, row: &Value) {
    fn walk(root: &std::path::Path, doc: &Document, actual: &[Layer], rows: &[Value]) {
        assert_eq!(actual.len(), rows.len());
        for (layer, source) in actual.iter().zip(rows.iter().rev()) {
            assert_eq!(layer.name, source["name"].as_str().unwrap());
            assert_eq!(layer.visible, source["visible"].as_bool().unwrap());
            assert_eq!(layer.opacity.to_bits(), ((source["opacity"].as_f64().unwrap() / 100.) as f32).to_bits());
            let x = source["offset"][0].as_i64().unwrap() as i32;
            let y = source["offset"][1].as_i64().unwrap() as i32;
            let w = source["size"][0].as_i64().unwrap() as i32;
            let h = source["size"][1].as_i64().unwrap() as i32;
            let rect = photocraft_doc::Rect::new(x, y, x + w, y + h);
            let item = doc.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(layer.id)).unwrap();
            assert_eq!(item.raster_bounds, Some(rect));
            if let Some(children) = source.get("children") {
                let LayerContent::Group(group) = &layer.content else { panic!("captured group must retain children, never cached native plane") };
                walk(root, doc, &group.children, children.as_array().unwrap());
            } else {
                let LayerContent::BackedRaster(backed) = &layer.content else { panic!("native leaf") };
                assert_eq!(backed.descriptor().bounds(), rect);
                let view = backed.checked_view();
                let bpp = view.format().bytes_per_pixel();
                let mut actual_bytes = vec![0; w as usize * h as usize * bpp];
                for yy in 0..h {
                    for xx in 0..w {
                        let at = (yy as usize * w as usize + xx as usize) * bpp;
                        view.read_logical_pixel_native_into(x + xx, y + yy, &mut actual_bytes[at..at + bpp], &ctl()).unwrap();
                    }
                }
                let mut expected = std::fs::read(root.join(source["native"]["file"].as_str().unwrap())).unwrap();
                if cfg!(target_endian = "big") {
                    for sample in expected.chunks_exact_mut(doc.depth.bytes()) {
                        sample.reverse();
                    }
                }
                assert_eq!(actual_bytes, expected);
            }
            if source["mask"].is_null() {
                assert!(layer.mask.is_none());
            } else {
                let mask = layer.mask.as_ref().unwrap();
                assert_eq!(mask.enabled, source["mask"]["enabled"].as_bool().unwrap());
                let mut expected = std::fs::read(root.join(source["mask"]["native"]["file"].as_str().unwrap())).unwrap();
                if cfg!(target_endian = "big") {
                    for sample in expected.chunks_exact_mut(doc.depth.bytes()) {
                        sample.reverse();
                    }
                }
                assert_eq!(mask.surface.try_read_interleaved(rect).unwrap(), expected);
            }
        }
    }
    walk(root, doc, &doc.layers, row["layers"].as_array().unwrap());
}

fn retained(before: &Document, current: &Document) {
    assert_eq!(tree_words(before), tree_words(current));
    assert_eq!(current.xcf_state, before.xcf_state);
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    assert_eq!(current.sample_encoding, before.sample_encoding);
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-owned-group-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN:24 pinned isolated Normal groups use typed Headless source/native routes, tree/profile/source protection and paid title continuation"]
fn isolated_groups_use_authorized_source_and_native_continuation_without_flattening() {
    for (root, row) in cases() {
        let original = std::fs::read(root.join(row["source"]["file"].as_str().unwrap())).unwrap();
        let dir = Directory::new();
        let path = dir.0.join("source.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(photocraft_raster::backing::BackingError::Work) };
        assert!(h.open_owned_xcf_keep_current(&path, &denied).is_err());
        assert!(h.session.active().is_none());
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        before.accounted().unwrap().validate_assigned_isolated_normal_group_storage(&ctl()).unwrap();
        assert!(h.session.active().unwrap().source_read_only);
        assert_eq!(tree_words(&before).len(), 4);
        observed_tree(&root, &before, &row);
        assert_eq!(before.sample_encoding, Some(SampleEncoding::NonlinearProfile));
        assert_eq!(hash(before.icc_profile.as_deref().unwrap()), row["assigned_icc"]["sha256"].as_str().unwrap());
        assert_eq!(before.icc_profile.as_deref().unwrap().len() as u64, row["assigned_icc"]["bytes"].as_u64().unwrap());
        assert_eq!(before.source_color_context.as_ref().unwrap().original_icc, before.icc_profile);
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        let masks = before.layers.iter().filter(|layer| layer.mask.is_some()).count();
        assert_eq!(query.value()["items"]["total"], 4 + masks);
        assert_eq!(query.value()["filters"]["total"], 0);
        drop(query);
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("group denied".into())));
        let error = match h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => e,
            Ok(_) => panic!("actual paid query must honor authorizer"),
        };
        assert!(error.to_string().contains("group denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.command_run("select.all", json!({})).is_err());
        assert!(h.command_run("xcf.filter.enable", json!({"filterId":"1","enabled":false})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Isolated current.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_ne!(saved.name, before.name);
        retained(&before, &saved);
        assert_eq!(h.session.active().unwrap().source, source);
        assert!(h.session.active().unwrap().source_read_only);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let state = reopened.session.active().unwrap();
        assert!(!state.source_read_only);
        assert_eq!(state.doc.name, saved.name);
        retained(&before, &state.doc);
        state.doc.accounted().unwrap().validate_assigned_isolated_normal_group_storage(&ctl()).unwrap();
        let query = reopened.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 4 + masks);
        assert_eq!(query.value()["filters"]["total"], 0);
        drop(query);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(row["source"]["file"].as_str().unwrap())).unwrap(), original);
    }
}
