//! Actual independently authored GIMP activity inputs. No derived acceptance claim.
#[path = "observed_box_move_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod current;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod native;
use photocraft_doc::{Document, SampleType, item_identity::ItemRef};
use sha2::{Digest, Sha256};
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    native::ctl()
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(owner)
}
pub(super) fn exact(a: &Document, b: &Document, dx: i32, dy: i32) {
    current::exact(a, b, dx, dy)
}
pub(super) fn reference(doc: &Document) -> photocraft_compose::Buffer {
    current::reference(doc)
}
pub(super) struct Case {
    pub id: String,
    pub path: std::path::PathBuf,
    pub bytes: Vec<u8>,
    pub visible: bool,
    pub opacity: u32,
    pub enabled: bool,
    pub assigned: bool,
    pub sample: SampleType,
    tag6: (usize, u32),
    tag33: (usize, u32),
    native_hash: String,
    native_bytes: usize,
}
pub(super) fn cases() -> Vec<Case> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Runtime-only local evidence dependency: ignored tests still compile in a
    // checkout without these unpublished research inputs.
    let text = std::fs::read_to_string(root.join("temp/sol6-observed-box-source-activity-host-20261016/source-bindings-v1.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(text.as_bytes())), "7cac734567839b02ef75ebf7157e2adafd46080ecb6d4e2c56e8143d38de1b69");
    let input: serde_json::Value = serde_json::from_str(&text).unwrap();
    let rows = input["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 15);
    rows.iter()
        .map(|row| {
            let path = root.join(row["path"].as_str().unwrap());
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(bytes.len() as u64, row["bytes"].as_u64().unwrap());
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), row["sha256"].as_str().unwrap());
            let tag = |name: &str| {
                let offset = usize::try_from(row[name]["offset"].as_u64().unwrap()).unwrap();
                let bits = u32::from_str_radix(row[name]["hex"].as_str().unwrap(), 16).unwrap();
                assert_eq!(&bytes[offset..offset + 4], &bits.to_be_bytes());
                (offset, bits)
            };
            let stored = std::fs::read(root.join(row["native"]["path"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&stored)), row["native"]["sha256"].as_str().unwrap());
            Case {
                id: row["id"].as_str().unwrap().into(),
                visible: row["visible"].as_bool().unwrap(),
                opacity: u32::try_from(row["opacity_bits"].as_u64().unwrap()).unwrap(),
                enabled: row["enabled"].as_bool().unwrap(),
                assigned: row["role"].as_str().unwrap() == "assigned-f32-nl",
                sample: if row["sample_bytes"] == 8 { SampleType::F64 } else { SampleType::F32 },
                tag6: tag("tag6"),
                tag33: tag("tag33"),
                native_hash: row["native"]["sha256"].as_str().unwrap().into(),
                native_bytes: stored.len(),
                path,
                bytes,
            }
        })
        .collect()
}
pub(super) fn import(case: &Case) -> photocraft_io::OwnedXcfImport {
    import_bytes(&case.id, case.bytes.clone()).unwrap()
}
pub(super) fn import_bytes(name: &str, bytes: Vec<u8>) -> Result<photocraft_io::OwnedXcfImport, photocraft_io::IoError> {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 32 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 64 << 20;
    options.limits.input_bytes = 16 << 20;
    photocraft_io::import_owned_xcf_bytes_in_quota(name, std::sync::Arc::new(bytes), &options, &limits, q, &ctl())
}
pub(super) fn observed(case: &Case, doc: &Document) {
    assert_eq!(doc.depth, case.sample);
    assert_eq!(doc.layers.len(), 1);
    let layer = &doc.layers[0];
    assert_eq!(layer.visible, case.visible);
    assert_eq!(layer.opacity.to_bits(), case.opacity);
    let state = doc.xcf_state.as_ref().unwrap();
    assert!(state.provenance.source_hash.is_some());
    assert_eq!(state.filter_stacks[0].nodes[0].enabled, case.enabled);
    assert_eq!(state.filter_stacks[0].nodes[0].operation.schema_epoch, "xcf22-observed-box-geometry-v1");
    for (tag, (offset, bits)) in [(6, case.tag6), (33, case.tag33)] {
        let identity = format!("{tag}@{offset}:4");
        let records: Vec<_> = state
            .opaque
            .iter()
            .filter(|r| r.owner == Some(ItemRef::Layer(layer.id)) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].bytes.as_slice(), bits.to_be_bytes());
    }
    // Tag33 is authoritative F32 opacity. Tag6 is independently retained legacy
    // byte evidence (0.375 authored alongside 95/255), never substituted for it.
    assert_eq!(case.tag33.1, case.opacity);
    if case.opacity == 0.375f32.to_bits() {
        assert_eq!(case.tag6.1, 95);
        assert_ne!((95f32 / 255.).to_bits(), case.opacity);
    }
    let words = native::native(doc);
    let sample_bytes = case.sample.bytes();
    let little: Vec<u8> = words
        .chunks_exact(sample_bytes)
        .flat_map(|word| {
            if sample_bytes == 4 {
                u32::from_ne_bytes(word.try_into().unwrap()).to_le_bytes().to_vec()
            } else {
                u64::from_ne_bytes(word.try_into().unwrap()).to_le_bytes().to_vec()
            }
        })
        .collect();
    assert_eq!(little.len(), case.native_bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&little)), case.native_hash);
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_ref();
    if case.assigned {
        let icc = original.unwrap();
        assert_eq!(icc.len(), 640);
        assert_eq!(format!("{:x}", Sha256::digest(icc.as_slice())), "abae4941f28e3558cb648775352364b2978e07a3b49e78228836b98a4ac05f46");
    } else {
        assert!(original.is_none());
    }
    assert!(doc.icc_profile.is_some()); // generated CURRENT, not a producer ICC equality claim
}
pub(super) fn malformed_inactive(case: &Case) -> Vec<u8> {
    assert!(!case.visible || case.opacity == 0);
    let mut bytes = case.bytes.clone();
    bytes[case.tag33.0..case.tag33.0 + 4].copy_from_slice(&f32::NAN.to_bits().to_be_bytes());
    bytes
}
