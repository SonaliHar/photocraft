//! Four independently saved assigned Gaussian originals. Named "unit" still
//! has five mild HDR RGB words; the HDR palette is bound separately. No clamp,
//! producer ICC equality waiver or independent derived-display claim.
#[path = "canvas_assigned_box_fixture.rs"]
#[allow(dead_code)]
mod original;
use photocraft_doc::{
    Document,
    xcf_filters::{ExactNumber, FilterValue},
};
use std::path::{Path, PathBuf};
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    original::ctl()
}
pub(super) fn exact(a: &Document, b: &Document) {
    original::exact(a, b)
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    original::archive(owner)
}
fn native_hash(path: &Path) -> &'static str {
    if path.file_name().unwrap().to_str().unwrap().starts_with("assigned-hdr-") {
        "77cacaa145ed0a5a07fc80c71fb093483c5eb684df0433a5aa01756dc275d302"
    } else {
        "3102cfa5ba9eb6cf718b767257d474fc865e0b5fa93e8b3622adeebd7fefaadf"
    }
}
pub(super) fn cases() -> Vec<(PathBuf, Vec<u8>)> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("gaussian-raster-color-evidence");
    [
        ("assigned-unit-f32.source", "832fdd77cdd8f9e9ac1c9a2a7de1ce52128c91f4f17f4018083aca65faee8f6a"),
        ("assigned-unit-f32.edit", "a79bfce1ec95b36ea7d4ccb6e39e705fb5872c844c8e41c7f15b14bbf1c9a2cd"),
        ("assigned-hdr-f32.source", "ce2b5b6339a7edd4b657b1cb85100673c33aa1f13fb25acbaf4c7929dc4ca1be"),
        ("assigned-hdr-f32.edit", "aade08703fbfde31c57bb3d11115c4e82c82ad05fe68df9c13acd68fe43e8be7"),
    ]
    .into_iter()
    .map(|(stem, hash)| {
        let path = root.join(format!("{stem}.xcf"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(original::hash(&bytes), hash);
        let native = std::fs::read(root.join(format!("{stem}.reopened.layer.buffer-extent.native.none.bin"))).unwrap();
        assert_eq!(native.len(), 3072);
        assert_eq!(original::hash(&native), native_hash(&path));
        (path, bytes)
    })
    .collect()
}
pub(super) fn observed(path: &Path, doc: &Document) {
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_deref().unwrap();
    assert_eq!(original.len(), 640);
    assert_eq!(original::hash(original), "abae4941f28e3558cb648775352364b2978e07a3b49e78228836b98a4ac05f46");
    for (offset, gamma) in [(520, 104858i32), (560, 150733), (600, 183501)] {
        assert_eq!(&original[offset..offset + 4], b"para");
        assert_eq!(u16::from_be_bytes(original[offset + 8..offset + 10].try_into().unwrap()), 4);
        let words: Vec<_> = original[offset + 12..offset + 40].chunks_exact(4).map(|b| i32::from_be_bytes(b.try_into().unwrap())).collect();
        assert_eq!(words, [gamma, 65536, 0, 65536, 0, 0, 0]);
    }
    let bytes = original::native(doc);
    assert_eq!(original::hash(&bytes), native_hash(path));
    for (i, word) in bytes.chunks_exact(4).enumerate() {
        let value = f32::from_bits(u32::from_le_bytes(word.try_into().unwrap()));
        assert!(value.is_finite());
        if i % 4 == 3 {
            assert!((0.0..=1.0).contains(&value));
        }
    }
    if native_hash(path).starts_with("3102") {
        original::observed(doc);
    }
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.name, "gegl:gaussian-blur");
    assert_eq!(node.operation.stored_version, "0:0");
    assert_eq!(node.operation.schema_epoch, "xcf22-rgb-alpha-finite-gaussian-v1");
    assert_eq!(node.arguments.len(), 5);
    assert!(node.enabled);
    assert_eq!(node.opacity_f64_bits, 1.0f64.to_bits());
    assert_eq!(node.source_clipping, Some(0));
    assert_eq!(node.source_region, Some(0));
    let edited = path.file_name().unwrap().to_str().unwrap().ends_with(".edit.xcf");
    let value = |name: &str| &node.arguments.iter().find(|a| a.name == name).unwrap().value;
    let (x, y) = if edited { (3.75f32, 0.8f32) } else { (2.25f32, 1.3f32) };
    assert_eq!(value("std-dev-x"), &FilterValue::Number(ExactNumber::Float32Bits(x.to_bits())));
    assert_eq!(value("std-dev-y"), &FilterValue::Number(ExactNumber::Float32Bits(y.to_bits())));
    assert_eq!(value("filter"), &FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: if edited { 1 } else { 0 } });
    assert_eq!(value("abyss-policy"), &FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 });
    assert_eq!(value("clip-extent"), &FilterValue::Bool(true));
    let c = node.composition.as_ref().unwrap();
    assert_eq!(c.source_mode, 62);
    assert!(!c.legacy);
    assert!(!c.pass_through);
    assert!(c.resolved.is_none());
    assert_eq!([c.stored_blend_space, c.stored_composite_space, c.stored_operator], [-1474592175; 3]);
}
