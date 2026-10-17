//! Genuine eight authored masked-mixed sources; raw captures are custody evidence, never a numerical oracle.
#![allow(dead_code, unused_imports)]
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_current_test_support.rs"]
mod native;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod old;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
mod planes;
pub use old::{ctl, payload, same, storage, workspace};
use photocraft_doc::{
    Document,
    accounted_document::AccountedDocument,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterValue},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
pub fn cases() -> Vec<Value> {
    let raw = std::fs::read(workspace().join("temp/root-mixed-mask-research-20261016/literal-index-v1.json")).unwrap();
    assert!(raw.len() < 256 << 10);
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), "4d64c41a6460a57e96673e04fdfd79d62183f0808d9ffe1e7db0a34ec442d43e");
    serde_json::from_slice::<Value>(&raw).unwrap()["cases"].as_array().unwrap().clone()
}
fn bound(b: &Value) -> Vec<u8> {
    let raw = std::fs::read(b["path"].as_str().unwrap()).unwrap();
    assert_eq!(raw.len() as u64, b["bytes"].as_u64().unwrap());
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), b["sha256"].as_str().unwrap());
    raw
}
pub fn path(c: &Value) -> PathBuf {
    PathBuf::from(c["source"]["path"].as_str().unwrap())
}
pub fn bytes(c: &Value) -> Vec<u8> {
    bound(&c["source"])
}
fn span(packet: &[u8], b: &Value) -> Vec<u8> {
    let at = usize::try_from(b["offset"].as_u64().unwrap()).unwrap();
    let len = usize::try_from(b["bytes"].as_u64().unwrap()).unwrap();
    let v = packet.get(at..at.checked_add(len).unwrap()).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(v)), b["sha256"].as_str().unwrap());
    v.to_vec()
}
pub fn import(c: &Value) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    let i = photocraft_io::import_owned_xcf_bytes_in_quota("masked-mixed.xcf", Arc::new(bytes(c)), &Default::default(), &limits, q, &ctl()).unwrap();
    assert!(matches!(i, photocraft_io::OwnedXcfImport::MaskedFilteredFlat(_)));
    observed(i.document().document(), c);
    i
}
fn props(d: &Document, owner: ItemRef, rows: &Value, source: &[u8]) {
    for p in rows.as_array().unwrap() {
        let hex = p["hex"].as_str().unwrap();
        let b: Vec<u8> = hex.as_bytes().as_chunks::<2>().0.iter().map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap()).collect();
        let at = usize::try_from(p["offset"].as_u64().unwrap()).unwrap();
        assert_eq!(source.get(at..at.checked_add(b.len()).unwrap()).unwrap(), b);
        let identity = format!("{}@{}:{}", p["tag"].as_u64().unwrap(), at, b.len());
        let record = d
            .xcf_state
            .as_ref()
            .unwrap()
            .opaque
            .iter()
            .find(|r| r.owner == Some(owner) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
            .unwrap();
        assert_eq!(record.bytes.as_slice(), b);
    }
}
pub fn observed(d: &Document, c: &Value) {
    let source = bytes(c);
    let _capture = bound(&c["capture"]);
    let packet = bound(&c["packet"]);
    let ctx = d.source_color_context.as_ref().unwrap();
    assert!(ctx.original_icc.is_none());
    assert_eq!(ctx.source_precision as u64, c["source_precision"].as_u64().unwrap());
    let roots = c["raw"]["roots"].as_array().unwrap();
    assert_eq!(d.layers.len(), roots.len());
    let state = d.xcf_state.as_ref().unwrap();
    for (layer, row) in d.layers.iter().zip(roots.iter().rev()) {
        assert_eq!(layer.name, row["name"].as_str().unwrap());
        assert_eq!(planes::raw(layer), span(&packet, &row["stored_source"]));
        props(d, ItemRef::Layer(layer.id), &row["properties"], &source);
        let effects = row["canonical_effects"].as_array().unwrap();
        let stack = state.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id));
        if effects.is_empty() {
            assert!(stack.is_none_or(|s| s.nodes.is_empty()));
            continue;
        }
        let stack = stack.unwrap();
        assert_eq!(stack.nodes.len(), effects.len());
        for (n, e) in stack.nodes.iter().zip(effects) {
            assert_eq!(n.operation.name, e["operation"].as_str().unwrap());
            assert_eq!(n.operation.stored_version, e["version"].as_str().unwrap());
            props(d, ItemRef::Layer(layer.id), &e["properties"], &source);
            let mask = state.filter_masks.iter().find(|m| m.owner == n.id).unwrap();
            props(d, ItemRef::Mask(mask.id), &e["stored_mask"]["properties"], &source);
            let raw = span(&packet, &e["stored_mask"]["stored_native"]);
            assert_eq!(mask.surface.try_read_interleaved(d.bounds()).unwrap(), raw);
            let nonempty = raw.iter().any(|b| *b != 0);
            let epoch = match (n.operation.name.as_str(), nonempty) {
                ("gegl:exposure", true) => photocraft_compose::EXPOSURE_MASKED_MIXED_EPOCH,
                ("gegl:invert-linear", true) => photocraft_compose::INVERT_LINEAR_MASKED_MIXED_EPOCH,
                ("gegl:invert-gamma", true) => photocraft_compose::INVERT_GAMMA_MASKED_MIXED_EPOCH,
                ("gegl:brightness-contrast", true) => photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_MIXED_EPOCH,
                ("gegl:levels", true) => photocraft_compose::GEGL_LEVELS_MASKED_MIXED_EPOCH,
                ("gegl:exposure", false) => photocraft_compose::EXPOSURE_MIXED_EPOCH,
                ("gegl:invert-linear", false) => photocraft_compose::INVERT_LINEAR_MIXED_EPOCH,
                ("gegl:invert-gamma", false) => photocraft_compose::INVERT_GAMMA_MIXED_EPOCH,
                ("gegl:brightness-contrast", false) => photocraft_compose::BRIGHTNESS_CONTRAST_MIXED_EPOCH,
                ("gegl:levels", false) => photocraft_compose::GEGL_LEVELS_MIXED_EPOCH,
                _ => panic!("bound scalar"),
            };
            assert_eq!(n.operation.schema_epoch, epoch);
            assert_eq!(n.composition.as_ref().unwrap().compatibility_epoch, epoch);
            let args: Vec<_> = e["properties"].as_array().unwrap().iter().filter_map(|p| p.get("argument_observation")).collect();
            assert_eq!(n.arguments.len(), args.len());
            for (a, b) in n.arguments.iter().zip(args) {
                assert_eq!(a.name, b["name"].as_str().unwrap());
                assert_eq!(a.value, FilterValue::Number(ExactNumber::Float32Bits(u32::from_str_radix(b["word_hex"].as_str().unwrap(), 16).unwrap())));
            }
        }
    }
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let raw = photocraft_io::accounted_current_native::save_accounted_current_archive(
        owner,
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        owner.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner();
    let archive = native::open(owner, &raw);
    assert_eq!(archive.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::MaskedFilteredFlatPoint));
    archive
}

struct CopyGuard {
    _source: AccountedDocument,
    _index: photocraft_raster::backing::quota::QuotaLease,
    _resident: photocraft_raster::backing::quota::QuotaLease,
}
/// External synthetic CURRENT only; actual source owner and conservative whole-copy fees precede clone.
pub fn current(owner: &AccountedDocument, change: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = owner.quota().clone();
    let index = q.reserve(photocraft_raster::backing::quota::QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = owner.document().clone();
    change(&mut d);
    AccountedDocument::new_readonly_masked_filtered_flat_point_current_storage(
        d,
        Arc::new(CopyGuard { _source: owner.clone(), _index: index, _resident: resident }),
        q,
        &ctl(),
    )
    .unwrap()
}
