//! Genuine sixteen nonempty authored mask sources. Captured bytes are external harness evidence, not a numerical output oracle.
#![allow(dead_code, unused_imports)]
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_current_test_support.rs"]
mod native;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
mod planes;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_exposure_loaded_test_support.rs"]
mod shared;
use photocraft_doc::{Document, accounted_document::AccountedDocument};
use serde_json::Value;
use sha2::{Digest, Sha256};
pub use shared::{ctl, payload, same, storage, workspace};
use std::{path::PathBuf, sync::Arc};
fn index() -> Value {
    let raw = std::fs::read(workspace().join("temp/sol1-four-masked-point-literal-bindings-20261016/literal-index-v1.json")).unwrap();
    assert!(raw.len() < 128 << 10);
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), "f21b945a5957539a5d645fa87a87a8012bf00de2a32cdb12a3d0b3520716c017");
    serde_json::from_slice(&raw).unwrap()
}
pub fn cases() -> Vec<Value> {
    index()["nonempty_cases"].as_array().unwrap().clone()
}
pub fn none_baselines() -> Vec<Value> {
    index()["old_none_baselines"].as_array().unwrap().clone()
}
pub fn path(case: &Value) -> PathBuf {
    PathBuf::from(case["source"]["path"].as_str().unwrap())
}
fn bound(binding: &Value) -> Vec<u8> {
    let raw = std::fs::read(binding["path"].as_str().unwrap()).unwrap();
    assert_eq!(raw.len() as u64, binding["bytes"].as_u64().unwrap());
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), binding["sha256"].as_str().unwrap());
    raw
}
pub fn bytes(case: &Value) -> Vec<u8> {
    bound(&case["source"])
}
fn span(packet: &[u8], binding: &Value) -> Vec<u8> {
    let at = usize::try_from(binding["offset"].as_u64().unwrap()).unwrap();
    let len = usize::try_from(binding["bytes"].as_u64().unwrap()).unwrap();
    let raw = packet.get(at..at.checked_add(len).unwrap()).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(raw)), binding["sha256"].as_str().unwrap());
    raw.to_vec()
}
pub fn import(case: &Value) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    let input = photocraft_io::import_owned_xcf_bytes_in_quota("masked-stock.xcf", Arc::new(bytes(case)), &Default::default(), &limits, q, &ctl()).unwrap();
    assert!(matches!(input, photocraft_io::OwnedXcfImport::Point(_)));
    observed(input.document().document(), case);
    input
}
pub fn observed(d: &Document, case: &Value) {
    let _capture = bound(&case["capture"]);
    let packet = bound(&case["packet"]);
    let source = bytes(case);
    assert_eq!(d.layers.len(), 1);
    let context = d.source_color_context.as_ref().unwrap();
    assert!(context.original_icc.is_none());
    assert_eq!(context.source_precision as u64, case["source_precision"].as_u64().unwrap());
    let x = d.xcf_state.as_ref().unwrap();
    assert_eq!(x.filter_stacks.len(), 1);
    assert_eq!(x.filter_stacks[0].nodes.len(), 1);
    let n = &x.filter_stacks[0].nodes[0];
    assert_eq!(n.operation.name, case["operation"].as_str().unwrap());
    let kind = match n.operation.name.as_str() {
        "gegl:invert-linear" => photocraft_compose::MaskedScalarKind::InvertLinear,
        "gegl:invert-gamma" => photocraft_compose::MaskedScalarKind::InvertGamma,
        "gegl:brightness-contrast" => photocraft_compose::MaskedScalarKind::BrightnessContrast,
        "gegl:levels" => photocraft_compose::MaskedScalarKind::GeglLevels,
        _ => panic!("unexpected bound operation"),
    };
    assert_eq!(n.operation.schema_epoch, kind.epoch());
    assert_eq!(n.composition.as_ref().unwrap().compatibility_epoch, kind.epoch());
    photocraft_compose::masked_scalar_parameters(kind, &n.arguments).unwrap();
    let args = case["arguments"].as_array().unwrap();
    assert_eq!(n.arguments.len(), args.len());
    for (a, expected) in n.arguments.iter().zip(args) {
        assert_eq!(a.name, expected["name"].as_str().unwrap());
        let word = u32::from_str_radix(expected["word_be_hex"].as_str().unwrap(), 16).unwrap();
        assert_eq!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(word)));
    }
    assert_eq!(planes::raw(&d.layers[0]), span(&packet, &case["raw_source_native"]));
    assert_eq!(x.filter_masks.len(), 1);
    assert_eq!(x.filter_masks[0].surface.try_read_interleaved(d.bounds()).unwrap(), span(&packet, &case["mask"]["raw_native"]));
    // Property offsets are payload starts. Raw records remain independent of declared operation semantics.
    for row in case["envelope_tag_payloadOffset_order_hex"].as_array().unwrap() {
        let tag = row[0].as_u64().unwrap();
        let at = usize::try_from(row[1].as_u64().unwrap()).unwrap();
        let hex = row[3].as_str().unwrap();
        let expected: Vec<u8> = hex.as_bytes().as_chunks::<2>().0.iter().map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap()).collect();
        assert_eq!(source.get(at..at.checked_add(expected.len()).unwrap()).unwrap(), expected);
        let identity = format!("{tag}@{at}:{}", expected.len());
        assert!(x.opaque.iter().any(|r| r.namespace == "gimp/xcf/source-property" && r.identity == identity && r.bytes.as_slice() == expected.as_slice()));
    }
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    // Encoded Vec is external harness; portable proof/writer and owned reader retain their actual receipts.
    let raw = photocraft_io::accounted_current_native::save_accounted_current_archive(
        owner,
        std::io::Cursor::new(Vec::new()),
        photocraft_format::owned_document::OwnedDocumentLimits::default(),
        owner.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner();
    native::open(owner, &raw)
}
