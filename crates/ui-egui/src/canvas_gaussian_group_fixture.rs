//! Shared UNRUN host fixture, no UI dependency. Source/capture buffers are
//! external harness allocations, never production quota or source grants.
use photocraft_doc::{Document, Layer, LayerContent, Rect, item_identity::ItemRef};
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
pub(super) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) fn assets() -> PathBuf {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("composition-matrix");
    let pins: Value = serde_json::from_slice(include_bytes!("../../compose/tests/gimp_gaussian_group_hashes.json")).unwrap();
    for (file, pin) in pins.as_object().unwrap() {
        assert_eq!(hash(&std::fs::read(root.join(file)).unwrap()), pin.as_str().unwrap());
    }
    root
}
pub(super) fn state(root: &Path, phase: &str) -> Value {
    let v: Value = serde_json::from_slice(&std::fs::read(root.join(format!("group-28-group-mask.fresh-{phase}_state.json"))).unwrap()).unwrap();
    assert_eq!(v["size"], serde_json::json!([16, 12]));
    assert_eq!(v["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
    assert!(v["assigned_icc_sha256"].is_null());
    assert_eq!(v["layers"].as_array().unwrap().len(), 4);
    for row in v["layers"].as_array().unwrap() {
        if !row["native"].is_null() {
            blob(root, &row["native"]);
        }
        if !row["mask"].is_null() {
            blob(root, &row["mask"]);
        }
    }
    v
}
fn blob(root: &Path, value: &Value) -> Vec<u8> {
    let bytes = std::fs::read(root.join(value["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), value["sha256"].as_str().unwrap());
    assert_eq!(bytes.len() as u64, value["bytes"].as_u64().unwrap());
    bytes
}
fn bounds(row: &Value) -> Rect {
    let x = row["offset"][0].as_i64().unwrap() as i32;
    let y = row["offset"][1].as_i64().unwrap() as i32;
    Rect::from_xywh(x, y, row["size"][0].as_u64().unwrap() as u32, row["size"][1].as_u64().unwrap() as u32)
}
fn native_order(mut bytes: Vec<u8>) -> Vec<u8> {
    if cfg!(target_endian = "big") {
        for word in bytes.chunks_exact_mut(4) {
            word.reverse();
        }
    }
    bytes
}
fn leaves<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>, depth: usize) {
    assert!(depth <= 1);
    for layer in layers {
        out.push(layer);
        if let LayerContent::Group(g) = &layer.content {
            assert_eq!(depth, 0);
            assert_eq!(g.children.len(), 2);
            leaves(&g.children, out, depth + 1);
        }
    }
}
pub(super) fn observed(root: &Path, doc: &Document, state: &Value) {
    let mut all = Vec::new();
    leaves(&doc.layers, &mut all, 0);
    assert_eq!(all.len(), 4);
    let rows = state["layers"].as_array().unwrap();
    for row in rows {
        let layer = all.iter().copied().find(|l| Some(l.name.as_str()) == row["name"].as_str()).unwrap();
        let r = bounds(row);
        let item = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap();
        assert_eq!(item.raster_bounds, Some(r));
        assert_eq!(item.name, layer.name);
        assert_eq!(item.composition.as_ref().unwrap().source_mode, i32::try_from(row["composition"]["mode"].as_i64().unwrap()).unwrap());
        if row["type"] == "GroupLayer" {
            let LayerContent::Group(g) = &layer.content else { panic!("retained Group.children, no flattened cache") };
            assert_eq!(g.children.len(), 2);
            let names: Vec<_> = g.children.iter().map(|l| l.name.as_str()).collect();
            assert_eq!(names, vec!["Own foreground", "Own group lower"]);
        } else {
            let LayerContent::BackedRaster(b) = &layer.content else { panic!("three genuine native leaves") };
            assert_eq!(b.operative_bounds(), r);
            let mut bytes = vec![0; r.width() as usize * r.height() as usize * 16];
            for coord in r.tiles() {
                let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
                let part = coord.rect().intersect(&r);
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * 16;
                        pin.read_pixel_native_into(x, y, &mut bytes[at..at + 16], &ctl()).unwrap();
                    }
                }
            }
            assert_eq!(bytes, native_order(blob(root, &row["native"])));
        }
        if row["mask"].is_null() {
            assert!(layer.mask.is_none());
        } else {
            let mask = layer.mask.as_ref().unwrap();
            assert!(mask.enabled);
            assert_eq!(mask.surface.try_read_interleaved(r).unwrap(), native_order(blob(root, &row["mask"])));
        }
    }
    let xcf = doc.xcf_state.as_ref().unwrap();
    assert_eq!(xcf.filter_stacks.len(), 1);
    assert_eq!(xcf.filter_masks.len(), 1);
    let stack = &xcf.filter_stacks[0];
    assert_eq!(stack.nodes.len(), 1);
    let node = &stack.nodes[0];
    assert_eq!(node.operation.name, "gegl:gaussian-blur");
    assert!(node.enabled);
    let group = all.iter().copied().find(|l| matches!(l.content, LayerContent::Group(_))).unwrap();
    assert_eq!(stack.owner, ItemRef::Layer(group.id));
    assert_eq!(node.mask, Some(ItemRef::Mask(xcf.filter_masks[0].id)));
    assert_eq!(xcf.filter_masks[0].owner, node.id);
    let mask = &xcf.filter_masks[0].surface;
    assert!(mask.default_bytes().iter().all(|b| *b == 0));
    assert!(mask.tiles().all(|(_, t)| t.bytes().iter().all(|b| *b == 0)));
}
pub(super) fn fingerprint(doc: &Document) -> Vec<String> {
    let mut all = Vec::new();
    leaves(&doc.layers, &mut all, 0);
    all.iter()
        .map(|layer| {
            let mut d = Sha256::new();
            d.update(format!("{:?}", (layer.id, &layer.name, layer.opacity.to_bits(), layer.visible, layer.blend)).as_bytes());
            if let LayerContent::BackedRaster(b) = &layer.content {
                d.update(format!("{:?}", b.descriptor()).as_bytes());
                d.update(b.descriptor().default_bytes());
                for coord in b.operative_bounds().tiles() {
                    let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
                    match pin.encoded() {
                        NativeTileBytes::FullTile(v) | NativeTileBytes::DefaultPixel(v) => d.update(v),
                    }
                }
            }
            if let Some(m) = &layer.mask {
                d.update(format!("{:?}", (m.enabled, m.linked, m.density.to_bits(), m.feather.to_bits(), m.surface.tile_bounds())).as_bytes());
                d.update(m.surface.default_bytes());
                for (c, t) in m.surface.tiles() {
                    d.update(format!("{c:?}").as_bytes());
                    d.update(t.bytes());
                }
            }
            d.finalize().iter().map(|v| format!("{v:02x}")).collect()
        })
        .collect()
}
pub(super) fn retained(before: &Document, current: &Document) {
    assert_eq!(fingerprint(before), fingerprint(current));
    assert_eq!(before.xcf_state, current.xcf_state);
    assert_eq!(before.icc_profile, current.icc_profile);
    assert_eq!(before.source_color_context, current.source_color_context);
    assert_eq!(before.working_profile_interpretation, current.working_profile_interpretation);
    assert_eq!(before.sample_encoding, current.sample_encoding);
}
