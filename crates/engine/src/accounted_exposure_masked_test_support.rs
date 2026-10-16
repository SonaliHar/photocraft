//! Genuine six nonempty scalar-mask sources; all captured blobs are external harness evidence.
#![allow(dead_code, unused_imports)]
#[path = "accounted_filtered_flat_current_test_support.rs"]
mod native;
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
mod planes;
#[path = "accounted_exposure_loaded_test_support.rs"]
mod shared;
use photocraft_doc::{Document, accounted_document::AccountedDocument};
use serde_json::Value;
use sha2::{Digest, Sha256};
pub use shared::{ctl, payload, same, storage, workspace};
use std::sync::Arc;
pub fn cases() -> Vec<Value> {
    let raw = std::fs::read(workspace().join("temp/sol6-masked-exposure-implementation-20261016/source-bindings-v1.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), "3ffde782749306cb739a981eca6e9f1efffb95da1a42ef22ed1c4560b909826c");
    serde_json::from_slice::<Value>(&raw).unwrap()["cases"].as_array().unwrap().clone()
}
pub fn bytes(case: &Value) -> Vec<u8> {
    let raw = std::fs::read(workspace().join(case["source"].as_str().unwrap())).unwrap();
    assert_eq!(raw.len() as u64, case["bytes"].as_u64().unwrap());
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), case["sha256"].as_str().unwrap());
    raw
}
pub fn import(case: &Value) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    let input = photocraft_io::import_owned_xcf_bytes_in_quota("masked.xcf", Arc::new(bytes(case)), &Default::default(), &limits, q, &ctl()).unwrap();
    assert!(matches!(input, photocraft_io::OwnedXcfImport::Point(_)));
    observed(input.document().document(), case);
    input
}
fn blob(case: &Value, value: &Value) -> Vec<u8> {
    let parent = workspace().join(case["capture"].as_str().unwrap());
    let raw = std::fs::read(parent.parent().unwrap().join(value["file"].as_str().unwrap())).unwrap();
    assert_eq!(raw.len() as u64, value["bytes"].as_u64().unwrap());
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), value["sha256"].as_str().unwrap());
    raw
}
pub fn observed(d: &Document, case: &Value) {
    assert_eq!(d.layers.len(), 1);
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision as u64, case["precision"].as_u64().unwrap());
    let raw = std::fs::read(workspace().join(case["capture"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), case["capture_sha256"].as_str().unwrap());
    let x = d.xcf_state.as_ref().unwrap();
    assert_eq!(x.filter_stacks.len(), 1);
    assert_eq!(x.filter_stacks[0].nodes.len(), 1);
    let n = &x.filter_stacks[0].nodes[0];
    assert_eq!(n.operation.schema_epoch, photocraft_compose::EXPOSURE_MASKED_EPOCH);
    assert_eq!(n.composition.as_ref().unwrap().compatibility_epoch, photocraft_compose::EXPOSURE_MASKED_EPOCH);
    assert_eq!(photocraft_compose::masked_exposure_parameters(&n.arguments).unwrap().values(), [0.5, 0.0625]);
    assert_eq!(planes::raw(&d.layers[0]), blob(case, &case["native"]));
    assert_eq!(x.filter_masks[0].surface.try_read_interleaved(d.bounds()).unwrap(), blob(case, &case["mask"]));
    // Logical captured words prove retention only; lower full Source program still proves defaults/padding.
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    // Encoded output is external harness; actual portable ownerwriter and sameq ownedreader pay their real resources.
    let output = std::io::Cursor::new(Vec::new());
    let raw = photocraft_io::accounted_current_native::save_accounted_current_archive(
        owner,
        output,
        photocraft_format::owned_document::OwnedDocumentLimits::default(),
        owner.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner();
    native::open(owner, &raw)
}
