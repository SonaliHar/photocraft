//! Authored transport/native lifecycle only. Producer projection/merged instability,
//! profile dates, scalar Saturation precision, RGB/LCh/Float staging and HDR output
//! remain HOLD; no producer projection oracle is used.
#[path = "linear_box_fixture.rs"]
#[allow(dead_code)]
mod ingress;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
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
    pub args: [u32; 2],
    pub raw: i32,
    pub mask_hash: &'static str,
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
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vibrance-owned-gimp-20261016");
    [
        (
            "f32-author-v1/outputs/f32-neutral-rounding.source.xcf",
            1955,
            "4e239a92719be5b66fb040b9d815295e5fb2cb9744ff55c8cc18980e80ec311c",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x00000000, 0x3f800000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-neutral-rounding.edit.xcf",
            1955,
            "64faa19282abb91fd24d3eaadd73c8c51ac57c192ce092dae767938d060b8272",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x41c80000, 0x3fa00000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-sign-desaturation.source.xcf",
            1955,
            "088fd5b88a1ceb2a1de231bb0cdecbc3f5ab33f8b30af6bc82b82191cc380a27",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0xc2480000, 0x3f800000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-sign-desaturation.edit.xcf",
            1955,
            "95737876c2353188b8ac8e47361b46931343e6560f303ef3560be0ab0cd8bac6",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x42480000, 0x3f000000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-bounded-extremes.source.xcf",
            1955,
            "581627573b92a776bc9dff4f82b8a88d7cbd50df6b4382332cd78832a024e5b5",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0xc2c80000, 0x00000000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-bounded-extremes.edit.xcf",
            1955,
            "0b19695189e32b59dfad2ada8e3ca54bb70863cf8cd6f6bcb29fcefa1651ca48",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x42c80000, 0x41200000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-neutral-rounding.source.xcf",
            2083,
            "120c3e4bf2bb0655b9af3637f6bd9eccc5ca272bf6a960ce86e86a22489fc156",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x00000000, 0x3f800000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-neutral-rounding.edit.xcf",
            2083,
            "6ed2c0ae8cb6f7500bdf0596b7b684e71eb50155d8150ef03080033be99bbe1e",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x41c80000, 0x3fa00000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-sign-desaturation.source.xcf",
            2083,
            "431932e37076996719d54ca620e4235d2dd8580c2b640a821feeb35dffee7daa",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0xc2480000, 0x3f800000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-sign-desaturation.edit.xcf",
            2083,
            "744f79b4826f7cf9d969e30fec4e07059e3d0c767e7980f1ce1707d82c025184",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x42480000, 0x3f000000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-bounded-extremes.source.xcf",
            2083,
            "f2e64ca7e6210f4610c19caa6fc53c509754e491bdc7afc8d3e679fcd583693a",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0xc2c80000, 0x00000000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-bounded-extremes.edit.xcf",
            2083,
            "43fa256526e60c865caab7581564d7a5812787809420de6333efc414c6a64614",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x42c80000, 0x41200000],
            0x2af58251,
        ),
    ]
    .into_iter()
    .map(|(path, len, hash, native_hash, mask_hash, args, raw)| {
        let path = root.join(path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), len);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        Case { path, bytes, native_hash, mask_hash, args, raw: raw as i32 }
    })
    .collect()
}
pub(super) fn args(doc: &Document) -> [u32; 2] {
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.arguments.len(), 2);
    ["vibrance", "saturation"].map(|name| {
        let a = node.arguments.iter().find(|a| a.name == name).unwrap();
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = &a.value else { panic!("exact F32") };
        *bits
    })
}
pub(super) fn wire(id: &str, args: [u32; 2], enabled: Option<bool>) -> serde_json::Value {
    let arguments: Vec<_> =
        ["vibrance", "saturation"].into_iter().zip(args).map(|(name, bits)| serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":bits}}})).collect();
    let mut value = serde_json::json!({"filterId":id,"arguments":arguments});
    if let Some(enabled) = enabled {
        value.as_object_mut().unwrap().insert("enabled".into(), serde_json::json!(enabled));
    }
    value
}
pub(super) fn observed(doc: &Document, case: &Case) {
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(doc.size, photocraft_doc::Size::new(8, 6));
    assert_eq!(doc.sample_encoding, Some(SampleEncoding::NonlinearProfile));
    let source = doc.source_color_context.as_ref().unwrap();
    assert!(source.original_icc.is_none());
    assert_eq!(
        source.source_precision,
        match doc.depth {
            SampleType::F32 => 650,
            SampleType::F64 => 750,
            _ => panic!("exact depth"),
        }
    );
    assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
    assert!(doc.icc_profile.is_some()); // Generated CURRENT profile; no equality to producer ICC.
    let bytes = native::native(doc);
    let le: Vec<u8> = bytes
        .chunks_exact(doc.depth.bytes())
        .flat_map(|word| {
            if doc.depth == SampleType::F32 {
                u32::from_ne_bytes(word.try_into().unwrap()).to_le_bytes().to_vec()
            } else {
                u64::from_ne_bytes(word.try_into().unwrap()).to_le_bytes().to_vec()
            }
        })
        .collect();
    assert_eq!(format!("{:x}", Sha256::digest(&le)), case.native_hash);
    assert_eq!(le.len(), 48 * 4 * doc.depth.bytes());
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.name, "gegl:vibrance");
    assert_eq!(node.operation.stored_version, "0:0");
    assert_eq!(node.operation.schema_epoch, "xcf22-gegl-vibrance-linear-lch-float-v1");
    let policy = node.composition.as_ref().unwrap();
    assert_eq!([policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator], [case.raw; 3]);
    assert_eq!(args(doc), case.args);
    let mask = &doc.xcf_state.as_ref().unwrap().filter_masks[0];
    assert_eq!(mask.owner, node.id);
    assert_eq!(mask.bounds, doc.bounds());
    let raw = mask.surface.try_read_interleaved(mask.bounds).unwrap();
    let le: Vec<u8> = raw
        .chunks_exact(doc.depth.bytes())
        .flat_map(|word| {
            if doc.depth == SampleType::F32 {
                u32::from_ne_bytes(word.try_into().unwrap()).to_le_bytes().to_vec()
            } else {
                u64::from_ne_bytes(word.try_into().unwrap()).to_le_bytes().to_vec()
            }
        })
        .collect();
    assert_eq!(format!("{:x}", Sha256::digest(&le)), case.mask_hash);
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

// Old unit-incompatible HDR and U8 schema files remain explicit negatives.
pub(super) fn old_sources() -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        (
            "corpus/xcf/full/operations/gegl-vibrance/activation-gegl-vibrance.edit.xcf",
            1972,
            "d410dda4b36e15471126315bc8cbd12f59bb000bf13341500e6366ff246ea373",
        ),
        ("corpus/xcf/full/operations/gegl-vibrance/activation-gegl-vibrance.xcf", 1972, "a6cc530fd2f03da085da7feb2951ce1e33998b0e0857c9c9d043af7804db8dab"),
        ("corpus/xcf/full/operations/gegl-vibrance/operation-gegl-vibrance.edit.xcf", 1972, "54d55be3806ebdc022ce52f58139227635f05e2a28536976bdbfedaf513105ee"),
        ("corpus/xcf/full/operations/gegl-vibrance/operation-gegl-vibrance.xcf", 1972, "fb96a323c63d4b7be1f559d0779a647ad077932e81c643ecf45e78adaea76883"),
        ("temp/sol2-vibrance-20261014/v3/packet/default.edit.xcf", 2393, "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/default.xcf", 2393, "8ca257fe0ce1b677d5a39f68d16458d2569276e51bc2e98f7674c0497c80f849"),
        ("temp/sol2-vibrance-20261014/v3/packet/disabled.edit.xcf", 2393, "1eb5041f87ce97c253b178ba775aeee4bfbd9bb7fedf39038745c0f5dc27c2fa"),
        ("temp/sol2-vibrance-20261014/v3/packet/disabled.xcf", 2393, "929aa6ae08b7e3491431d14b662b007082ea8b454b63611dd54cce8a9bc45499"),
        ("temp/sol2-vibrance-20261014/v3/packet/negative.edit.xcf", 2393, "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/negative.xcf", 2393, "7c175d599e2435f432783cdde8f49c36229d2a581b26d4963d3578672cde6ab2"),
        ("temp/sol2-vibrance-20261014/v3/packet/positive.edit.xcf", 2393, "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/positive.xcf", 2393, "5ce751cdc2f2b069753aa708ade31b17ffa68d0812183c186d0bc71e8acc9148"),
        ("temp/sol2-vibrance-20261014/v3/packet/saturation.edit.xcf", 2393, "c49bf10587980d919f4e7bd8de2a2ccc9143b7b1444fc41053997b872fcf0354"),
        ("temp/sol2-vibrance-20261014/v3/packet/saturation.xcf", 2393, "3140c9f9b6e2d163fdd262cd712cf1a7d23b799a743553e26461cc3df9f546aa"),
    ]
    .into_iter()
    .map(|(path, len, hash)| {
        let path = root.join(path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), len);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        (path, bytes)
    })
    .collect()
}
pub(super) fn refuses(name: &str, bytes: Vec<u8>) -> bool {
    ingress::try_import(name, bytes).is_err()
}
