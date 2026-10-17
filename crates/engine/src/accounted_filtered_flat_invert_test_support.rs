//! Authentic mixed invert author bindings; storage observations only.
//! Import workflows are deferred until explicit IO admission; no numerical oracle.
#![allow(dead_code, unused_imports)]
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod native_fixture;
#[path = "accounted_filtered_flat_current_test_support.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod owned;
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod retained;
pub use owned::{archive, ctl, current, encoded, same, storage, unmarked_archive, workspace};
use photocraft_doc::{
    Document,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterValue},
};
pub use retained::payload;
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[derive(Clone, Copy)]
pub struct Case {
    pub source: &'static str,
    pub bytes: usize,
    pub hash: &'static str,
    pub capture: &'static str,
    pub capture_bytes: usize,
    pub capture_hash: &'static str,
    pub precision: u32,
    pub width: u32,
    pub height: u32,
}
pub const CASES: [Case; 4] = [
    Case {
        source: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-8x6-mixed-inverts.xcf",
        bytes: 13247,
        hash: "8948b71cbb257c700ce05145e1695d4cca599f291a7277ac101a232ad54acb46",
        capture: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-8x6-mixed-inverts.case.json",
        capture_bytes: 158805,
        capture_hash: "4cffc8140cb35720cafda6c71cb2866d53422903b5ee426169d6af010c17f1c2",
        precision: 650,
        width: 8,
        height: 6,
    },
    Case {
        source: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-130x65-mixed-inverts.xcf",
        bytes: 371994,
        hash: "cb9e5bb196a17f04acbc755e2c21df26e4478a791a22f469e8b799b744ec7aa2",
        capture: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f32-130x65-mixed-inverts.case.json",
        capture_bytes: 160197,
        capture_hash: "1b9331db984dc4ddbae54b156a5f8ff804958d024564437fcc3747d3318615ff",
        precision: 650,
        width: 130,
        height: 65,
    },
    Case {
        source: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-8x6-mixed-inverts.xcf",
        bytes: 13319,
        hash: "54aff2884dea1e361b1d5d6765fb51a9366a4c39b02ac842b2c71ed46ec68c08",
        capture: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-8x6-mixed-inverts.case.json",
        capture_bytes: 158969,
        capture_hash: "63943e30e92f0faf1672104fe2b19162536ee40853589c24e515223e222032e1",
        precision: 750,
        width: 8,
        height: 6,
    },
    Case {
        source: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-130x65-mixed-inverts.xcf",
        bytes: 328098,
        hash: "e6c830205356fb140e6a0b8df68dd5b6602fc669391c4c43c7156e0b46c7d8b3",
        capture: "temp/sol3-filtered-flat-invert-gimp-20261016/author-v1/outputs/f64-130x65-mixed-inverts.case.json",
        capture_bytes: 160301,
        capture_hash: "6c38fa63da38132675537f568ddec958c2b80dad60bd09bc20fe07f14f17d324",
        precision: 750,
        width: 130,
        height: 65,
    },
];
pub fn bytes(case: &Case) -> Vec<u8> {
    let b = std::fs::read(workspace().join(case.source)).unwrap();
    assert_eq!(b.len(), case.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), case.hash);
    b
}
pub fn import(case: &Case) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 64 << 20;
    options.limits.input_bytes = 16 << 20;
    let value = photocraft_io::import_owned_xcf_bytes_in_quota("mixed-inverts.xcf", Arc::new(bytes(case)), &options, &limits, q, &ctl()).unwrap();
    assert!(matches!(value, photocraft_io::OwnedXcfImport::FilteredFlat(_)));
    observed(value.document().document(), case);
    value
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes().as_chunks::<2>().0.iter().map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
}
fn scalar(raw: &[u8]) -> (String, u32) {
    let n = u32::from_be_bytes(raw[..4].try_into().unwrap()) as usize;
    let name = std::str::from_utf8(&raw[4..4 + n - 1]).unwrap().to_owned();
    assert_eq!(raw[4 + n - 1], 0);
    assert_eq!(u32::from_be_bytes(raw[4 + n..8 + n].try_into().unwrap()), 3);
    let bits = u32::from_be_bytes(raw[8 + n..12 + n].try_into().unwrap());
    assert_eq!(raw.len(), 12 + n);
    (name, bits)
}
pub fn observed(d: &Document, case: &Case) {
    let capture = std::fs::read(workspace().join(case.capture)).unwrap();
    assert_eq!(capture.len(), case.capture_bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&capture)), case.capture_hash);
    let value: serde_json::Value = serde_json::from_slice(&capture).unwrap();
    let directory = workspace().join(case.capture).parent().unwrap().to_owned();
    let x = d.xcf_state.as_ref().unwrap();
    assert_eq!(d.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(d.depth, if case.precision == 650 { photocraft_doc::SampleType::F32 } else { photocraft_doc::SampleType::F64 });
    assert_eq!(d.size, photocraft_doc::Size::new(case.width, case.height));
    assert_eq!(d.layers.len(), 8);
    assert_eq!(x.filter_stacks.len(), 7);
    assert_eq!(x.filter_masks.len(), 15);
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
    assert_eq!(d.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    assert_eq!(d.icc_profile.as_ref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().to_bytes().as_slice());
    let operations: [&[&str]; 8] = [
        &[],
        &["gegl:invert-linear"],
        &["gegl:invert-gamma"],
        &["gegl:exposure", "gegl:invert-linear"],
        &["gegl:invert-linear", "gegl:exposure"],
        &["gegl:exposure", "gegl:invert-gamma"],
        &["gegl:invert-gamma", "gegl:exposure"],
        &["gegl:vibrance", "gegl:hue-chroma", "gimp:semi-flatten", "gegl:invert-gamma", "gegl:invert-linear"],
    ];
    for (index, (layer, row)) in d.layers.iter().zip(value["raw"]["roots"].as_array().unwrap().iter().rev()).enumerate() {
        assert_eq!(layer.name, format!("Own root {index}"));
        let blob = &row["stored_source"];
        let b = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&b)), blob["sha256"].as_str().unwrap());
        assert_eq!(native_fixture::raw(layer), b);
        let stack = x.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id));
        if index == 0 {
            assert!(stack.is_none());
            continue;
        }
        let stack = stack.unwrap();
        assert_eq!(stack.nodes.iter().map(|n| n.operation.name.as_str()).collect::<Vec<_>>(), operations[index]);
        for (node, effect) in stack.nodes.iter().zip(row["effects"].as_array().unwrap()) {
            assert_eq!(node.operation.name, effect["strings"][2].as_str().unwrap());
            assert_eq!(node.operation.stored_version, effect["strings"][3].as_str().unwrap());
            for p in effect["properties"].as_array().unwrap() {
                let raw = hex(p["hex"].as_str().unwrap());
                let identity = format!("{}@{}:{}", p["tag"].as_u64().unwrap(), p["offset"].as_u64().unwrap(), raw.len());
                let record = x
                    .opaque
                    .iter()
                    .find(|r| r.owner == Some(ItemRef::Layer(layer.id)) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
                    .unwrap();
                assert_eq!(record.bytes.as_slice(), raw);
            }
            if matches!(node.operation.name.as_str(), "gegl:invert-linear" | "gegl:invert-gamma") {
                let epoch = if node.operation.name == "gegl:invert-linear" {
                    photocraft_compose::INVERT_LINEAR_MIXED_EPOCH
                } else {
                    photocraft_compose::INVERT_GAMMA_MIXED_EPOCH
                };
                assert_eq!(node.operation.stored_version, "0:0");
                assert_eq!(node.operation.schema_epoch, epoch);
                assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, epoch);
                assert!(node.arguments.is_empty());
                assert!(effect["properties"].as_array().unwrap().iter().all(|p| p["tag"] != 45));
            }
            if node.operation.name == "gegl:exposure" {
                assert_eq!(node.operation.stored_version, "1:0");
                assert_eq!(node.operation.schema_epoch, photocraft_compose::EXPOSURE_MIXED_EPOCH);
                assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, photocraft_compose::EXPOSURE_MIXED_EPOCH);
                let expected = effect["properties"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|p| p["tag"] == 45)
                    .map(|p| scalar(&hex(p["hex"].as_str().unwrap())))
                    .collect::<Vec<_>>();
                assert_eq!(expected.len(), 2);
                for ((name, bits), a) in expected.iter().zip(&node.arguments) {
                    assert_eq!(name, &a.name);
                    assert_eq!(a.value, FilterValue::Number(ExactNumber::Float32Bits(*bits)));
                    assert_eq!(
                        *bits,
                        match name.as_str() {
                            "exposure" => 0.5f32.to_bits(),
                            "black-level" => 0.0625f32.to_bits(),
                            _ => panic!("unknown"),
                        }
                    );
                }
            }
            let blob = &effect["mask"]["stored_native"];
            let b = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&b)), blob["sha256"].as_str().unwrap());
            let mask = x.filter_masks.iter().find(|m| m.owner == node.id).unwrap();
            assert_eq!(mask.surface.try_read_interleaved(d.bounds()).unwrap(), b);
        }
    }
}

pub fn old_point_baseline(index: usize) -> photocraft_io::OwnedXcfImport {
    let (path, length, hash) = match index {
        0 => (
            "corpus/xcf/full/point-invert-linear-evidence/invert-linear-rgb-float_non_linear.small.source.xcf",
            2081,
            "8004eaeedd9eca95ac35a1ce8074ac1f52be37eb65492f2dbe62bc3977ea6296",
        ),
        1 => (
            "corpus/xcf/full/point-invert-gamma-evidence/invert-gamma-rgb-float_non_linear.small.source.xcf",
            2079,
            "a6d580568cc2beb4dda6508b922924f948223442144c1b2dc99bfd13593f4f45",
        ),
        _ => panic!("baseline"),
    };
    let raw = std::fs::read(workspace().join(path)).unwrap();
    assert_eq!(raw.len(), length);
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), hash);
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 64 << 20;
    options.limits.input_bytes = 16 << 20;
    let value = photocraft_io::import_owned_xcf_bytes_in_quota("old-invert.xcf", Arc::new(raw), &options, &limits, q, &ctl()).unwrap();
    assert!(matches!(value, photocraft_io::OwnedXcfImport::Point(_)));
    value
}

struct DerivedGuard {
    _owner: photocraft_doc::accounted_document::AccountedDocument,
    _index: photocraft_raster::backing::quota::QuotaLease,
    _resident: photocraft_raster::backing::quota::QuotaLease,
}
/// Synthetic CURRENT copy only; actual source/native custody and prospective fees retained.
pub fn derived_current(
    owner: &photocraft_doc::accounted_document::AccountedDocument,
    f: impl FnOnce(&mut Document),
) -> photocraft_doc::accounted_document::AccountedDocument {
    use photocraft_raster::backing::quota::QuotaKind;
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut doc = owner.document().clone();
    f(&mut doc);
    photocraft_doc::accounted_document::AccountedDocument::new_readonly_filtered_flat_point_current_storage(
        doc,
        Arc::new(DerivedGuard { _owner: owner.clone(), _index: index, _resident: resident }),
        q,
        &ctl(),
    )
    .unwrap()
}
