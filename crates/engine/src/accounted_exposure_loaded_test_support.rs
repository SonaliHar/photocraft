//! Genuine ordinary endpoint sources; mixed endpoint controls are synthetic CURRENT.
#![allow(dead_code, unused_imports)]
#[path = "accounted_filtered_flat_exposure_test_support.rs"]
mod mixed;
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
mod native;
#[path = "accounted_filtered_flat_current_test_support.rs"]
mod owned;
pub use mixed::{ctl, payload, same, storage, workspace};
use photocraft_doc::{
    Document,
    accounted_document::AccountedDocument,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[derive(Clone, Copy)]
pub struct Case {
    pub source: &'static str,
    pub bytes: usize,
    pub hash: &'static str,
    pub capture: &'static str,
    pub capture_hash: &'static str,
    pub precision: u32,
    pub black: u32,
}
pub const CASES: [Case; 4] = [
    Case {
        source: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f32-exposure-black-positive-endpoint.xcf",
        bytes: 2005,
        hash: "dcf34793cf27896fec4f6696a3fd7d69c13d08002bb72354ef6f7b054e4f3c38",
        capture: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f32-exposure-black-positive-endpoint.case.json",
        capture_hash: "f0a2c069f9083ae8a6287ebe455b59290519e10ace410dfa8fc19dedf97516ed",
        precision: 650,
        black: 0x3dcc_cccd,
    },
    Case {
        source: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f32-exposure-black-negative-endpoint.xcf",
        bytes: 2005,
        hash: "9adb12c808089f13930d60ea3155598ddfe9bfabd2421b131c4664a22cdf6b72",
        capture: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f32-exposure-black-negative-endpoint.case.json",
        capture_hash: "df9ff5822e48fb0b142f52ef00764f83339102ef8d30f129e8a62a15c912fdb1",
        precision: 650,
        black: 0xbdcc_cccd,
    },
    Case {
        source: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f64-exposure-black-positive-endpoint.xcf",
        bytes: 2007,
        hash: "5d834f0df7afd29507c93b83f1391c9ab4f08e56c8db6d6184821bfd720b879c",
        capture: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f64-exposure-black-positive-endpoint.case.json",
        capture_hash: "8c3f47197a383cb41ebdbccfaf262276f3c1f7b50a297f2657969ac82e8d8ee6",
        precision: 750,
        black: 0x3dcc_cccd,
    },
    Case {
        source: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f64-exposure-black-negative-endpoint.xcf",
        bytes: 2007,
        hash: "c4ec2e55dda286741a42995690cc677a4152e65517cdb1b2783c39682f406fd8",
        capture: "temp/sol3-exposure-black-zero-control-gimp-20261016/author-v1/outputs/f64-exposure-black-negative-endpoint.case.json",
        capture_hash: "f65daf8682ba2b234f2795bb352f1576f17f96186aad3955d2d9c58336494fcb",
        precision: 750,
        black: 0xbdcc_cccd,
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
    let value = photocraft_io::import_owned_xcf_bytes_in_quota(
        "endpoint.xcf",
        Arc::new(bytes(case)),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        q,
        &ctl(),
    )
    .unwrap();
    assert!(matches!(value, photocraft_io::OwnedXcfImport::Point(_)));
    observed(value.document().document(), case);
    value
}
pub fn words(node: &FilterNode) -> [u32; 2] {
    let value = |name| match node.arguments.iter().find(|a| a.name == name).unwrap().value {
        FilterValue::Number(ExactNumber::Float32Bits(b)) => b,
        _ => panic!("F32"),
    };
    [value("exposure"), value("black-level")]
}
pub fn observed(d: &Document, case: &Case) {
    let b = std::fs::read(workspace().join(case.capture)).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&b)), case.capture_hash);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    let directory = workspace().join(case.capture).parent().unwrap().to_owned();
    let x = d.xcf_state.as_ref().unwrap();
    assert_eq!((d.layers.len(), x.filter_stacks.len(), x.filter_masks.len()), (1, 1, 1));
    assert_eq!(d.size, photocraft_doc::Size::new(8, 6));
    assert_eq!(d.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(d.depth, if case.precision == 650 { photocraft_doc::SampleType::F32 } else { photocraft_doc::SampleType::F64 });
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(d.icc_profile.as_ref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().to_bytes().as_slice());
    let node = &x.filter_stacks[0].nodes[0];
    assert_eq!(node.operation.schema_epoch, photocraft_compose::EXPOSURE_LOADED_POINT_EPOCH);
    assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, photocraft_compose::EXPOSURE_LOADED_POINT_EPOCH);
    assert_eq!(words(node), [0.5f32.to_bits(), case.black]);
    assert_eq!(node.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["black-level", "exposure"]);
    let row = &v["raw"]["roots"][0];
    let blob = &row["stored_source"];
    let raw = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), blob["sha256"].as_str().unwrap());
    assert_eq!(native::raw(&d.layers[0]), raw);
    let effect = &row["effects"][0];
    let blob = &effect["mask"]["stored_native"];
    let mask = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&mask)), blob["sha256"].as_str().unwrap());
    assert_eq!(x.filter_masks[0].surface.try_read_interleaved(d.bounds()).unwrap(), mask);
    for prop in effect["properties"].as_array().unwrap() {
        let text = prop["hex"].as_str().unwrap();
        let raw = text.as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect::<Vec<_>>();
        let identity = format!("{}@{}:{}", prop["tag"].as_u64().unwrap(), prop["offset"].as_u64().unwrap(), raw.len());
        assert!(x.opaque.iter().any(|r| r.owner == Some(ItemRef::Layer(d.layers[0].id))
            && r.namespace == "gimp/xcf/source-property"
            && r.identity == identity
            && r.bytes.as_slice() == raw));
    }
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let b = owned::encoded(owner);
    owned::open(owner, &b)
}
/// Genuine mixed original retained by real prepaid model copy; not an endpoint GIMP source witness.
pub fn mixed_current(index: usize, black: u32) -> AccountedDocument {
    let input = mixed::import(&mixed::CASES[index]);
    mixed::current(input.document(), |d| {
        for node in d.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().flat_map(|s| s.nodes.iter_mut()).filter(|n| n.operation.name == "gegl:exposure") {
            node.operation.schema_epoch = photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH.into();
            node.composition.as_mut().unwrap().compatibility_epoch = photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH.into();
            node.arguments.iter_mut().find(|a| a.name == "black-level").unwrap().value = FilterValue::Number(ExactNumber::Float32Bits(black));
        }
    })
}
