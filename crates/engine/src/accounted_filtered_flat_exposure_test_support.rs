//! Actual mixed Exposure8x6 source bindings, independent transport/native checks only.
#![allow(dead_code, unused_imports)]
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
mod native;
#[path = "accounted_filtered_flat_current_test_support.rs"]
mod owned;
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
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
}
pub const CASES: [Case; 2] = [
    Case {
        source: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f32-plain-exposure-ordered-point.xcf",
        bytes: 8051,
        hash: "b41ff7782734b803494e43a0274f0789757aa67902e335933455e4c58179ca2c",
        capture: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f32-plain-exposure-ordered-point.case.json",
        capture_bytes: 93477,
        capture_hash: "d8a67ff8232c162ed1ac04727ff4e642f8b30c36435e788d8b8318782d71cf7e",
        precision: 650,
    },
    Case {
        source: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f64-plain-exposure-ordered-point.xcf",
        bytes: 8113,
        hash: "ffa38184286dfa0ef1462223bb55e217822bed5595b350af0ff7ce786c71f015",
        capture: "temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs/f64-plain-exposure-ordered-point.case.json",
        capture_bytes: 93565,
        capture_hash: "a30284ae72d28587c5f8591fdd7f9136d7059bcb98f1932f331a6e2981e12df9",
        precision: 750,
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
    let value = photocraft_io::import_owned_xcf_bytes_in_quota("exposure.xcf", Arc::new(bytes(case)), &options, &limits, q, &ctl()).unwrap();
    assert!(matches!(value, photocraft_io::OwnedXcfImport::FilteredFlat(_)));
    observed(value.document().document(), case);
    value
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
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
    assert_eq!(d.size, photocraft_doc::Size::new(8, 6));
    assert_eq!(d.layers.len(), 4);
    assert_eq!(x.filter_stacks.len(), 3);
    assert_eq!(x.filter_masks.len(), 9);
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
    assert_eq!(d.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    assert_eq!(d.icc_profile.as_ref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().to_bytes().as_slice());
    let operations: [&[&str]; 4] = [
        &[],
        &["gegl:exposure"],
        &["gegl:vibrance", "gegl:hue-chroma", "gimp:semi-flatten", "gegl:exposure"],
        &["gegl:exposure", "gimp:semi-flatten", "gegl:hue-chroma", "gegl:vibrance"],
    ];
    for (index, (layer, row)) in d.layers.iter().zip(value["raw"]["roots"].as_array().unwrap().iter().rev()).enumerate() {
        assert_eq!(layer.name, format!("Own root {index}"));
        let blob = &row["stored_source"];
        let b = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&b)), blob["sha256"].as_str().unwrap());
        assert_eq!(native::raw(layer), b);
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
            "corpus/xcf/full/exposure-black-level-precision-evidence/rgb-float_non_linear.source.xcf",
            2135,
            "11f0242cedd1e70854903f0d85868ae8d173666f6e6192d17616790d0b97bfa0",
        ),
        1 => (
            "corpus/xcf/full/exposure-black-level-precision-evidence/rgb-double_non_linear.source.xcf",
            2120,
            "7dcfe0d9a2da768a15f6da08a46b7d82977bfb01cd370359b5bce962348ca637",
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
    let value = photocraft_io::import_owned_xcf_bytes_in_quota("old-exposure.xcf", Arc::new(raw), &options, &limits, q, &ctl()).unwrap();
    assert!(matches!(value, photocraft_io::OwnedXcfImport::Point(_)));
    value
}

pub fn exposure_pair(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_doc::accounted_document::AccountedDocument {
    // Explicitly synthetic CURRENT native control, retained genuine packet/provider.
    // Existing current() preholds Index8MiB+Resident16MiB before model copy.
    current(owner, |d| {
        let root = d.layers.last().unwrap().id;
        d.layers.retain(|l| l.id == root);
        let x = d.xcf_state.as_mut().unwrap();
        x.filter_stacks.retain(|st| st.owner == ItemRef::Layer(root));
        let st = &mut x.filter_stacks[0];
        assert_eq!(st.nodes[0].operation.name, "gegl:exposure");
        assert_eq!(st.nodes[1].operation.name, "gimp:semi-flatten");
        st.nodes.truncate(2);
        let masks = st.nodes.iter().map(|n| n.mask.unwrap()).collect::<Vec<_>>();
        x.filter_masks.retain(|m| masks.contains(&ItemRef::Mask(m.id)));
        x.items.retain(|i| i.item == ItemRef::Layer(root) || masks.contains(&i.item));
        x.opaque.retain(|r| r.owner.is_none() || r.owner == Some(ItemRef::Layer(root)) || r.owner.is_some_and(|i| masks.contains(&i)));
        x.selection.selected.retain(|i| *i == ItemRef::Layer(root));
        x.selection.active = Some(ItemRef::Layer(root));
    })
}
