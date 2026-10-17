//! Hash-bound authored flat blend packets. All file/native buffers are external
//! harness captures, not production quota evidence or source qualification.
use photocraft_doc::{Document, LayerContent, Rect, item_identity::ItemRef};
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
pub(super) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) struct Case {
    pub file: String,
    pub state: Value,
    #[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
    pub png: String,
}
pub(super) fn cases() -> (PathBuf, Vec<Case>) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("composition-matrix");
    let mut cases = Vec::new();
    for raw in [
        include_bytes!("../../compose/tests/gimp_basic_blends_hashes.json").as_slice(),
        include_bytes!("../../compose/tests/gimp_multiply_hashes.json").as_slice(),
        include_bytes!("../../compose/tests/gimp_multiply_spaces_hashes.json").as_slice(),
    ] {
        let pins: Value = serde_json::from_slice(raw).unwrap();
        for (file, pin) in pins.as_object().unwrap() {
            assert_eq!(hash(&std::fs::read(root.join(file)).unwrap()), pin.as_str().unwrap());
            if file.starts_with("composite-space-lab") {
                continue;
            }
            if !file.ends_with(".xcf") {
                continue;
            }
            let (stem, phase) = if let Some(stem) = file.strip_suffix(".edit.xcf") { (stem, "edit") } else { (file.strip_suffix(".xcf").unwrap(), "source") };
            let state: Value = serde_json::from_slice(&std::fs::read(root.join(format!("{stem}.fresh-{phase}_state.json"))).unwrap()).unwrap();
            assert_eq!(state["size"], serde_json::json!([16, 12]));
            assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
            assert!(state["assigned_icc_sha256"].is_null());
            assert_eq!(state["layers"].as_array().unwrap().len(), 2);
            for row in state["layers"].as_array().unwrap() {
                blob(&root, &row["native"]);
                assert!(row["mask"].is_null());
                assert!(row["filters"].as_array().unwrap().is_empty());
            }
            cases.push(Case { file: file.clone(), state, png: format!("{stem}.fresh-{phase}.ref.png") });
        }
    }
    assert_eq!(cases.len(), 50);
    (root, cases)
}
fn blob(root: &Path, pin: &Value) -> Vec<u8> {
    let mut bytes = std::fs::read(root.join(pin["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), pin["sha256"].as_str().unwrap());
    assert_eq!(bytes.len() as u64, pin["bytes"].as_u64().unwrap());
    assert_eq!(pin["byte_order"], "little");
    if cfg!(target_endian = "big") {
        for word in bytes.as_chunks_mut::<4>().0 {
            word.reverse();
        }
    }
    bytes
}
pub(super) fn observed(root: &Path, doc: &Document, case: &Case) {
    // Independent existing eager source mapping supplies the literal canonical
    // policy witness; public GIMP Auto getters are not raw stored tuple aliases.
    // This external harness document is not a render or resource authority.
    let input = std::fs::read(root.join(&case.file)).unwrap();
    let eager = photocraft_io::xcf_import::import(&case.file, &input, &Interrupt::NONE).unwrap().document;
    assert_eq!(doc.layers.len(), 2);
    let xcf = doc.xcf_state.as_ref().unwrap();
    assert_eq!(xcf.items.len(), 2);
    assert!(xcf.filter_stacks.is_empty());
    assert!(xcf.filter_masks.is_empty());
    for ((layer, original), row) in doc.layers.iter().zip(&eager.layers).zip(case.state["layers"].as_array().unwrap().iter().rev()) {
        assert_eq!(layer.name, row["name"].as_str().unwrap());
        assert!(layer.visible);
        assert!(layer.mask.is_none());
        let r = Rect::from_xywh(
            row["offset"][0].as_i64().unwrap() as i32,
            row["offset"][1].as_i64().unwrap() as i32,
            row["size"][0].as_u64().unwrap() as u32,
            row["size"][1].as_u64().unwrap() as u32,
        );
        let item = xcf.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap();
        let original_item = eager.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(original.id)).unwrap();
        assert_eq!(item.composition, original_item.composition);
        assert_eq!(item.raster_bounds, Some(r));
        assert_eq!(item.name, layer.name);
        assert_eq!(item.composition.as_ref().unwrap().source_mode as i64, row["composition"]["mode"].as_i64().unwrap());
        let LayerContent::BackedRaster(b) = &layer.content else { panic!("two unflattened native leaves") };
        assert_eq!(b.operative_bounds(), r);
        let mut native = vec![0; r.width() as usize * r.height() as usize * 16];
        for coord in r.tiles() {
            let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
            let part = coord.rect().intersect(&r);
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * 16;
                    pin.read_pixel_native_into(x, y, &mut native[at..at + 16], &ctl()).unwrap();
                }
            }
        }
        assert_eq!(native, blob(root, &row["native"]));
    }
}
fn fingerprint(doc: &Document) -> String {
    let mut h = Sha256::new();
    for layer in &doc.layers {
        h.update(format!("{:?}", (layer.id, &layer.name, layer.opacity.to_bits(), layer.visible, layer.blend)).as_bytes());
        let LayerContent::BackedRaster(b) = &layer.content else { panic!("native") };
        h.update(format!("{:?}", b.descriptor()).as_bytes());
        h.update(b.descriptor().default_bytes());
        for coord in b.operative_bounds().tiles() {
            let p = b.checked_view().pin_tile(coord, &ctl()).unwrap();
            h.update(format!("{coord:?}").as_bytes());
            match p.encoded() {
                NativeTileBytes::FullTile(v) | NativeTileBytes::DefaultPixel(v) => h.update(v),
            }
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn retained(before: &Document, after: &Document) {
    assert_eq!(fingerprint(before), fingerprint(after));
    assert_eq!(before.xcf_state, after.xcf_state);
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.source_color_context, after.source_color_context);
    assert_eq!(before.working_profile_interpretation, after.working_profile_interpretation);
    assert_eq!(before.sample_encoding, after.sample_encoding);
}
