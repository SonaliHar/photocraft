//! Actual authored assigned-F64 sources; raw storage and derived display differ.
#[path = "observed_box_move_fixture.rs"]
#[allow(dead_code)]
mod current;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod native;
use photocraft_doc::{Document, LayerContent, SampleType};
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
pub(super) fn cases() -> Vec<(std::path::PathBuf, Vec<u8>, i32)> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol4-assigned-f64-box-gimp-20261016/author-v1/outputs");
    [(4, "8fca79d49c1993c2c9408b158f5d0767e41f1f48365631e2075b2e7c6e4f3ecb"), (5, "f8fa56b3de1c8a9a6d1339d4f937fb39233ffc9d939f7fa74b355d1113b27363")]
        .into_iter()
        .map(|(radius, hash)| {
            let path = root.join(format!("assigned-f64-hdr-nl-r{radius}.xcf"));
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(bytes.len(), 86036);
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
            (path, bytes, radius)
        })
        .collect()
}
pub(super) fn import(name: &str, bytes: Vec<u8>) -> photocraft_io::OwnedXcfImport {
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
    photocraft_io::import_owned_xcf_bytes_in_quota(name, std::sync::Arc::new(bytes), &options, &limits, q, &ctl()).unwrap()
}
pub(super) fn observed(doc: &Document) {
    assert_eq!(doc.depth, SampleType::F64);
    assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, 750);
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap();
    assert_eq!(original.len(), 640);
    assert_eq!(format!("{:x}", Sha256::digest(original.as_slice())), "abae4941f28e3558cb648775352364b2978e07a3b49e78228836b98a4ac05f46");
    let words = native::native(doc);
    assert_eq!(words.len(), 2130016);
    let little: Vec<u8> = words.chunks_exact(8).flat_map(|w| u64::from_ne_bytes(w.try_into().unwrap()).to_le_bytes()).collect();
    assert_eq!(format!("{:x}", Sha256::digest(&little)), "a72a5c127254c9c6310ca8d4da3c9e4b0b15db50d7602e6e6cd4fef6fda08cfe");
    for (offset, bits) in [
        (825600, 0xbfc0000000000000u64),
        (825640, 0x3ff4000000000000),
        (825680, 0x8000000000000000),
        (833824, 0x3ff0000000001000),
        (833864, 0xb690000000000000),
    ] {
        assert_eq!(u64::from_le_bytes(little[offset..offset + 8].try_into().unwrap()), bits);
    }
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.schema_epoch, "xcf22-observed-box-geometry-v1");
    let p = node.composition.as_ref().unwrap();
    assert_eq!([p.stored_blend_space, p.stored_composite_space, p.stored_operator], [1654948433; 3]);
}
pub(super) fn native_roles(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(native::native(a), native::native(b));
    let (LayerContent::BackedRaster(x), LayerContent::BackedRaster(y)) = (&a.layers[0].content, &b.layers[0].content) else { panic!("native") };
    assert_eq!(x.default_bytes(), y.default_bytes());
    assert_eq!(x.operative_bounds(), y.operative_bounds());
    let (x, y) = (a.xcf_state.as_ref().unwrap(), b.xcf_state.as_ref().unwrap());
    assert_eq!(x.opaque, y.opaque);
    assert_eq!(x.provenance, y.provenance);
    assert_eq!(x.filter_masks, y.filter_masks);
    assert_eq!(x.items, y.items);
    assert_eq!(x.filter_stacks[0].owner, y.filter_stacks[0].owner);
    assert_eq!(x.filter_stacks[0].nodes[0].composition, y.filter_stacks[0].nodes[0].composition);
}
