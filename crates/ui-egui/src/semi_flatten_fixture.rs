//! Authored RGB Semi-Flatten storage, not a GIMP numerical acceptance oracle.
//! CSS failures, ICC date HOLD and Babl/FMA/F64 rounding limits stay explicit.
#[path = "linear_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod ingress;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod native;
use photocraft_doc::{
    Document, LayerContent, SampleEncoding, SampleType,
    xcf_filters::{ExactNumber, FilterValue},
};
use sha2::{Digest, Sha256};
pub(super) struct Case {
    pub path: std::path::PathBuf,
    pub bytes: Vec<u8>,
    pub native_hash: &'static str,
    pub color: [u32; 4],
    pub encoding: SampleEncoding,
}
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    native::ctl()
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(owner)
}
pub(super) fn import(name: &str, bytes: Vec<u8>) -> photocraft_io::OwnedXcfImport {
    ingress::import(name, bytes)
}
pub(super) fn native_bytes(doc: &Document) -> Vec<u8> {
    native::native(doc)
}
pub(super) fn cases() -> Vec<Case> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-semi-flatten-gimp-20261016");
    [
        (
            "rgb-f32-author-v1/outputs/rgb-f32-background.source.xcf",
            1909,
            "e6d705f52c19ac30c4e3d5f92eefe22ee6baab791ce42fdb879397cb140c61da",
            "2120f5af3b5602c84e98b021240417232307f599815a707f5a2d97d97072daba",
            SampleEncoding::Linear,
            [0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000],
        ),
        (
            "rgb-f32-author-v1/outputs/rgb-f32-background.edit.xcf",
            1909,
            "b12706117bbe1718c4874bfca8a4cdb7964cc4e7e045eff720645e043d022452",
            "2120f5af3b5602c84e98b021240417232307f599815a707f5a2d97d97072daba",
            SampleEncoding::Linear,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3f800000],
        ),
        (
            "rgb-f32-author-v1/outputs/rgb-f32-color-alpha.source.xcf",
            1909,
            "3976c92c72840afc57cacb95a5e120cd68b949a501efa0fa7a8d73db5c2987d9",
            "2120f5af3b5602c84e98b021240417232307f599815a707f5a2d97d97072daba",
            SampleEncoding::Linear,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x0],
        ),
        (
            "rgb-f32-author-v1/outputs/rgb-f32-color-alpha.edit.xcf",
            1909,
            "147035285b053556ac174834eee7d83eca65f7041098cd0d8b4aff3a7dda71cb",
            "2120f5af3b5602c84e98b021240417232307f599815a707f5a2d97d97072daba",
            SampleEncoding::Linear,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3ec00000],
        ),
        (
            "rgb-f64-author-v1/outputs/rgb-f64-background.source.xcf",
            2028,
            "63278093e370c2e7d0bcea850c1fa3ef494f6975b91746ea71594d7b9bec5b71",
            "1a28d027573433ea94f7803b457dd51c0dccab5f16008b610ffadbe45270a4a0",
            SampleEncoding::Linear,
            [0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000],
        ),
        (
            "rgb-f64-author-v1/outputs/rgb-f64-background.edit.xcf",
            2028,
            "41d2ebc1e30203079a129dd2cd7623689207874b9f34967922eeb7fd306507e1",
            "1a28d027573433ea94f7803b457dd51c0dccab5f16008b610ffadbe45270a4a0",
            SampleEncoding::Linear,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3f800000],
        ),
        (
            "rgb-f64-author-v1/outputs/rgb-f64-color-alpha.source.xcf",
            2028,
            "2501ec0a000249bf114158e78d4e634d90fb3e857ad08d7750a16bdc7e938d3a",
            "1a28d027573433ea94f7803b457dd51c0dccab5f16008b610ffadbe45270a4a0",
            SampleEncoding::Linear,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x0],
        ),
        (
            "rgb-f64-author-v1/outputs/rgb-f64-color-alpha.edit.xcf",
            2028,
            "8a31886838d0234448a8d1b500a05efa9d60eb3e98d0193a27c2eddf35219b57",
            "1a28d027573433ea94f7803b457dd51c0dccab5f16008b610ffadbe45270a4a0",
            SampleEncoding::Linear,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3ec00000],
        ),
        (
            "rgb-f32-nonlinear-bytes-author-v1/outputs/rgb-f32-nonlinear-bytes.source.xcf",
            1912,
            "d4debe389177edc01192b09fadb22f5d353b54cd27f7af357698ab5cfb223b89",
            "2120f5af3b5602c84e98b021240417232307f599815a707f5a2d97d97072daba",
            SampleEncoding::NonlinearProfile,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3f800000],
        ),
        (
            "rgb-f32-nonlinear-bytes-author-v1/outputs/rgb-f32-nonlinear-bytes.edit.xcf",
            1912,
            "4953d0168280fc3ba07fcb897a508f6adade89ed9327c5785781c4c8b964de71",
            "2120f5af3b5602c84e98b021240417232307f599815a707f5a2d97d97072daba",
            SampleEncoding::NonlinearProfile,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3ec00000],
        ),
        (
            "rgb-f64-nonlinear-bytes-author-v1/outputs/rgb-f64-nonlinear-bytes.source.xcf",
            2031,
            "8caac66d87e1ea9599c90600709989a87dd33b458beec4611a764971dae9212a",
            "1a28d027573433ea94f7803b457dd51c0dccab5f16008b610ffadbe45270a4a0",
            SampleEncoding::NonlinearProfile,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3f800000],
        ),
        (
            "rgb-f64-nonlinear-bytes-author-v1/outputs/rgb-f64-nonlinear-bytes.edit.xcf",
            2031,
            "e7b4a2e405ccc4804e6e851fdd4bc7d8e1f54c41596489020c7082a5d3a0c45d",
            "1a28d027573433ea94f7803b457dd51c0dccab5f16008b610ffadbe45270a4a0",
            SampleEncoding::NonlinearProfile,
            [0x3f400000, 0x3e800000, 0x3e000000, 0x3ec00000],
        ),
    ]
    .into_iter()
    .map(|(relative, len, hash, native_hash, encoding, color)| {
        let path = root.join(relative);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), len);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        Case { path, bytes, native_hash, encoding, color }
    })
    .collect()
}
pub(super) fn color(doc: &Document) -> &FilterValue {
    &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value
}
pub(super) fn color_bits(value: &FilterValue) -> ([u32; 4], SampleEncoding) {
    let FilterValue::Color { components, encoding, profile: None } = value else { panic!("typed literal") };
    assert_eq!(components.len(), 4);
    let mut out = [0; 4];
    for (word, v) in out.iter_mut().zip(components) {
        let ExactNumber::Float32Bits(bits) = v else { panic!("F32 literal") };
        *word = *bits;
    }
    (out, *encoding)
}
pub(super) fn observed(doc: &Document, case: &Case) {
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(doc.sample_encoding, Some(SampleEncoding::NonlinearProfile));
    assert_eq!(doc.size, photocraft_doc::Size::new(8, 6));
    let source = doc.source_color_context.as_ref().unwrap();
    assert!(source.original_icc.is_none());
    assert_eq!(
        source.source_precision,
        match doc.depth {
            SampleType::F32 => 650,
            SampleType::F64 => 750,
            _ => panic!("exact role"),
        }
    );
    assert!(matches!(&doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
    assert!(doc.icc_profile.is_some());
    let raw = native::native(doc);
    let le: Vec<u8> = raw
        .chunks_exact(doc.depth.bytes())
        .flat_map(|w| {
            if doc.depth == SampleType::F32 {
                u32::from_ne_bytes(w.try_into().unwrap()).to_le_bytes().to_vec()
            } else {
                u64::from_ne_bytes(w.try_into().unwrap()).to_le_bytes().to_vec()
            }
        })
        .collect();
    assert_eq!(format!("{:x}", Sha256::digest(&le)), case.native_hash);
    assert_eq!(le.len(), 8 * 6 * 4 * doc.depth.bytes());
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.name, "gimp:semi-flatten");
    assert_eq!(node.operation.stored_version, "0:0");
    assert_eq!(node.operation.schema_epoch, "xcf22-gimp-semi-flatten-literal-v1");
    let policy = node.composition.as_ref().unwrap();
    assert_eq!([policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator], [0xc8758251u32 as i32; 3]);
    assert_eq!(node.arguments[0].name, "color");
    assert_eq!(color_bits(color(doc)), (case.color, case.encoding));
}
pub(super) fn storage(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(native::native(a), native::native(b));
    let (LayerContent::BackedRaster(x), LayerContent::BackedRaster(y)) = (&a.layers[0].content, &b.layers[0].content) else { panic!("native") };
    assert_eq!(x.format(), y.format());
    assert_eq!(x.default_bytes(), y.default_bytes());
    assert_eq!(x.operative_bounds(), y.operative_bounds());
    let (x, y) = (a.xcf_state.as_ref().unwrap(), b.xcf_state.as_ref().unwrap());
    assert_eq!(x.opaque, y.opaque);
    assert_eq!(x.provenance, y.provenance);
    assert_eq!(x.filter_masks, y.filter_masks);
    assert_eq!(x.items.iter().map(|i| i.item).collect::<Vec<_>>(), y.items.iter().map(|i| i.item).collect::<Vec<_>>());
    assert_eq!(x.filter_stacks[0].owner, y.filter_stacks[0].owner);
    assert_eq!(x.filter_stacks[0].nodes[0].composition, y.filter_stacks[0].nodes[0].composition);
}
pub(super) fn wire(id: &str, value: &FilterValue) -> serde_json::Value {
    serde_json::json!({"filterId":id,"arguments":[{"name":"color","value":value}]})
}
pub(super) fn changed(value: &FilterValue) -> FilterValue {
    let (mut words, encoding) = color_bits(value);
    words[..3].copy_from_slice(&[0.125f32.to_bits(), 0.625f32.to_bits(), 0.375f32.to_bits()]);
    FilterValue::Color { components: words.map(ExactNumber::Float32Bits).to_vec(), encoding, profile: None }
}
