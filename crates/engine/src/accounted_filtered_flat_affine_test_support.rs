//! Genuine affine source packets; helpers do not stand in for numerical oracles.
#![allow(dead_code, unused_imports)]
#[path = "accounted_filtered_flat_current_test_support.rs"]
mod native;
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod retained;
pub use native::{archive, ctl, same, storage, workspace};
use photocraft_doc::{Document, xcf_filters::FilterNode};
pub use retained::payload;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub struct Case {
    pub name: &'static str,
    pub bytes: usize,
    pub hash: &'static str,
    pub width: u32,
    pub height: u32,
    pub precision: u32,
}
pub const CASES: [Case; 4] = [
    Case {
        name: "f32-8x6-mixed-affine-pair",
        bytes: 15540,
        hash: "a0c8c940d02b6689ce2e0ba8a6beca7de56a7ec78b40d3cdea09bf7bf3e5e640",
        width: 8,
        height: 6,
        precision: 650,
    },
    Case {
        name: "f32-130x65-mixed-affine-pair",
        bytes: 374719,
        hash: "b1c8971957701c795c5bc2d6a0ee905b6a6b2c4f78cab7b7af1dc5174e17a685",
        width: 130,
        height: 65,
        precision: 650,
    },
    Case {
        name: "f64-8x6-mixed-affine-pair",
        bytes: 15636,
        hash: "8c346ccb09200b40e016fc91a6c51746232bbf4e4b26fc59c81117c7f0a4cc78",
        width: 8,
        height: 6,
        precision: 750,
    },
    Case {
        name: "f64-130x65-mixed-affine-pair",
        bytes: 331039,
        hash: "f63d2d224848df7a76a3dc7c5db478a5362a24080acf9395e15436b0de441b46",
        width: 130,
        height: 65,
        precision: 750,
    },
];
pub fn path(c: &Case) -> std::path::PathBuf {
    workspace().join("temp/sol1-filtered-flat-affine-pair-gimp-released-20261016/author-v1/outputs").join(format!("{}.xcf", c.name))
}
pub fn bytes(c: &Case) -> Vec<u8> {
    let b = std::fs::read(path(c)).unwrap();
    assert_eq!(b.len(), c.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), c.hash);
    b
}
pub fn affine(node: &FilterNode) -> bool {
    matches!(node.operation.name.as_str(), "gegl:brightness-contrast" | "gegl:levels")
}
pub fn observed(d: &Document, c: &Case) {
    assert_eq!((d.size.width, d.size.height), (c.width, c.height));
    let x = d.xcf_state.as_ref().unwrap();
    assert_eq!(d.layers.len(), 8);
    assert_eq!(x.filter_stacks.iter().map(|s| s.nodes.len()).sum::<usize>(), 18);
    assert_eq!(x.filter_masks.len(), 18);
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, c.precision);
    let nodes = x.filter_stacks.iter().flat_map(|s| &s.nodes).filter(|n| affine(n)).collect::<Vec<_>>();
    assert_eq!(nodes.len(), 10);
    for node in nodes {
        assert_eq!(node.operation.stored_version, "0:0");
        assert!(node.enabled);
        assert_eq!(node.opacity_f64_bits, 1f64.to_bits());
        if node.operation.name == "gegl:levels" {
            assert_eq!(node.operation.schema_epoch, photocraft_compose::GEGL_LEVELS_MIXED_EPOCH);
            assert_eq!(
                photocraft_compose::gegl_levels_mixed_parameters(&node.arguments).unwrap().raw_words(),
                [0.125f32.to_bits(), 0.875f32.to_bits(), 0.0625f32.to_bits(), 0.9375f32.to_bits()]
            );
        } else {
            assert_eq!(node.operation.schema_epoch, photocraft_compose::BRIGHTNESS_CONTRAST_MIXED_EPOCH);
            assert_eq!(node.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["contrast", "brightness"]);
            assert_eq!(
                photocraft_compose::brightness_contrast_mixed_parameters(&node.arguments).unwrap().raw_words(),
                [(-0.125f32).to_bits(), 0.75f32.to_bits()]
            );
        }
    }
}
pub fn import(c: &Case) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let input = photocraft_io::import_owned_xcf_bytes_in_quota(
        "affine.xcf",
        Arc::new(bytes(c)),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        q,
        &ctl(),
    )
    .unwrap();
    assert!(matches!(input, photocraft_io::OwnedXcfImport::FilteredFlat(_)));
    observed(input.document().document(), c);
    input
}
