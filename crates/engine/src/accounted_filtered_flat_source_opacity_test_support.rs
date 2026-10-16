//! Deferred Source-opacity contracts; execution remains UNRUN after explicit Source release.
#![allow(dead_code, unused_imports)]
#[path = "accounted_filtered_flat_invert_test_support.rs"]
mod native;
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
mod planes;
pub use native::{archive, ctl, payload, same, storage};
use photocraft_doc::{Document, item_identity::ItemRef};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
pub fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn cases() -> Vec<Value> {
    let b = std::fs::read(workspace().join("temp/sol6-filtered-flat-source-opacity-readiness-20261016/literal-source-cases-v1.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&b)), "d886528519e3b003d7dcdb9dc651c4ffb3a6efcc812bc24f49d97cddf90234db");
    serde_json::from_slice::<Value>(&b).unwrap()["cases"].as_array().unwrap().clone()
}
pub fn path(c: &Value) -> std::path::PathBuf {
    workspace().join(c["source"]["path"].as_str().unwrap())
}
pub fn bytes(c: &Value) -> Vec<u8> {
    let b = std::fs::read(path(c)).unwrap();
    assert_eq!(b.len() as u64, c["source"]["bytes"].as_u64().unwrap());
    assert_eq!(format!("{:x}", Sha256::digest(&b)), c["source"]["sha256"].as_str().unwrap());
    b
}
pub fn import(c: &Value) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let value = photocraft_io::import_owned_xcf_bytes_in_quota(
        "original-opacity.xcf",
        Arc::new(bytes(c)),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        q,
        &ctl(),
    )
    .unwrap();
    assert!(matches!(value, photocraft_io::OwnedXcfImport::FilteredFlat(_)));
    observed(value.document().document(), c);
    value
}
pub fn observed(d: &Document, c: &Value) {
    let capture_bytes = std::fs::read(workspace().join(c["capture"]["path"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&capture_bytes)), c["capture"]["sha256"].as_str().unwrap());
    let capture: Value = serde_json::from_slice(&capture_bytes).unwrap();
    let directory = workspace().join(c["capture"]["path"].as_str().unwrap()).parent().unwrap().to_owned();
    let x = d.xcf_state.as_ref().unwrap();
    assert_eq!(d.layers.len(), 8);
    assert_eq!(x.filter_stacks.iter().map(|s| s.nodes.len()).sum::<usize>(), 12);
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision as u64, c["source_precision"].as_u64().unwrap());
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    for (layer, row) in d.layers.iter().zip(capture["raw"]["roots"].as_array().unwrap().iter().rev()) {
        assert_eq!(layer.name, row["name"].as_str().unwrap());
        let blob = &row["stored_source"];
        let b = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&b)), blob["sha256"].as_str().unwrap());
        assert_eq!(planes::raw(layer), b);
        let stack = x.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id)).unwrap();
        for (node, effect) in stack.nodes.iter().zip(row["effects"].as_array().unwrap()) {
            let blob = &effect["mask"]["stored_native"];
            let b = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&b)), blob["sha256"].as_str().unwrap());
            assert_eq!(x.filter_masks.iter().find(|m| m.owner == node.id).unwrap().surface.try_read_interleaved(d.bounds()).unwrap(), b);
        }
    }
    for r in c["rows"].as_array().unwrap() {
        let layer = &d.layers[r["root_index_bottom_first"].as_u64().unwrap() as usize];
        let stack = x.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id)).unwrap();
        let node = &stack.nodes[r["node_index_raw_public_order"].as_u64().unwrap() as usize];
        assert_eq!(node.operation.name, r["operation"].as_str().unwrap());
        assert_eq!(node.operation.stored_version, r["version"].as_str().unwrap());
        assert_eq!(node.source_display_name.as_deref(), Some(r["label"].as_str().unwrap()));
        assert_eq!(format!("0x{:016x}", node.opacity_f64_bits), r["expected_promoted_f64_hex"].as_str().unwrap());
        assert_eq!(node.enabled, r["enabled"].as_bool().unwrap());
        let identity = format!("33@{}:4", r["opacity_tag33_payload_offset"].as_u64().unwrap());
        let raw = u32::from_str_radix(r["opacity_f32_hex"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap().to_be_bytes();
        assert!(x.opaque.iter().any(|p| p.owner == Some(ItemRef::Layer(layer.id))
            && p.namespace == "gimp/xcf/source-property"
            && p.identity == identity
            && p.bytes.as_slice() == raw));
    }
}
