//! Authored transport/native lifecycle only. Producer projection/merged instability,
//! profile date differences, RGB/LCh boundary rounding and HDR output remain HOLD.
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
    pub args: [u32; 3],
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
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-hue-chroma-owned-gimp-20261016");
    [
        (
            "f32-author-v1/outputs/f32-neutral-to-source.source.xcf",
            1981,
            "0f63702cdd76521303d869ca8ac952103c75a6abf69961555201dc635c615edf",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x00000000, 0x00000000, 0x00000000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-neutral-to-source.edit.xcf",
            1981,
            "9ff2a32b41c705b766789128dc615345b27c332c20aeafa092da5e578d4f55c9",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x42160000, 0xc1480000, 0x41040000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-hue-rounding.source.xcf",
            1981,
            "8d415ba95da8b234895f26ed3e43013491807bdc71131b97705f689837ee7b50",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x42160000, 0xc1480000, 0x41040000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-hue-rounding.edit.xcf",
            1981,
            "7adac66d030a51866ea0055c56fc2e6c25c79278677cc752526fe93eefc7f1a4",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0xc2868000, 0xc1480000, 0x41040000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-bounded-extremes.source.xcf",
            1981,
            "b59dcc15af9cf3eb1c5c8a1e85da8d7db24804ea72d055507cf064e7a6156180",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0xc3340000, 0xc2c80000, 0xc2c80000],
            0x2af58251,
        ),
        (
            "f32-author-v1/outputs/f32-bounded-extremes.edit.xcf",
            1981,
            "6bb46b5917356fc9cc897a681da0f35194d9e332e8e2560e48b6cea3ae4a8555",
            "67a34663709bb48dbe98692e4ef03e8ab61532015043822b08472837c4aa6897",
            "5d89f056865052bcb89c910d2d62872e029fb273c3db03f8968a52a41593c1b5",
            [0x43340000, 0x42c80000, 0x42c80000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-neutral-to-source.source.xcf",
            2109,
            "3514e59d95223345ec4dc73d0c53374af13e25a92ff6b4277a97284528b574f9",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x00000000, 0x00000000, 0x00000000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-neutral-to-source.edit.xcf",
            2109,
            "527f53d962be754627f8f39675dc019cbeba80868546d3ce980fc3cd5d52fe14",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x42160000, 0xc1480000, 0x41040000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-hue-rounding.source.xcf",
            2109,
            "d0002164f32bf0b4d294a58b4704d9dfea9218de14e1c1e403e15df3974cae36",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x42160000, 0xc1480000, 0x41040000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-hue-rounding.edit.xcf",
            2109,
            "723440020b7d91f7785651c5c347514b0af22410ac133e311f0b022a1f6a3b28",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0xc2868000, 0xc1480000, 0x41040000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-bounded-extremes.source.xcf",
            2109,
            "ecbc1e085a72cfc2eef1ccbc4a6d1738727ef68345aa8a80b55fd2db163adadf",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0xc3340000, 0xc2c80000, 0xc2c80000],
            0x2af58251,
        ),
        (
            "f64-author-v1/outputs/f64-bounded-extremes.edit.xcf",
            2109,
            "66798160686b0a6936e752c195f931971e4f3886034c13219efb8d5351bdb992",
            "42d761aacd0be354b515d4c1209a66b68a0c68151ce72da57c85589c7ccd13e1",
            "a1a4f5721c1c4610af7f71078f3a68c330536d679803b0e0507ee8dc10c5dfca",
            [0x43340000, 0x42c80000, 0x42c80000],
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
pub(super) fn args(doc: &Document) -> [u32; 3] {
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.arguments.len(), 3);
    ["hue", "chroma", "lightness"].map(|name| {
        let a = node.arguments.iter().find(|a| a.name == name).unwrap();
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = &a.value else { panic!("exact F32") };
        *bits
    })
}
pub(super) fn wire(id: &str, args: [u32; 3], enabled: Option<bool>) -> serde_json::Value {
    let arguments: Vec<_> = ["hue", "chroma", "lightness"]
        .into_iter()
        .zip(args)
        .map(|(name, bits)| serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":bits}}}))
        .collect();
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
    assert_eq!(node.operation.name, "gegl:hue-chroma");
    assert_eq!(node.operation.stored_version, "0:0");
    assert_eq!(node.operation.schema_epoch, "xcf22-gegl-hue-chroma-lch-float-v1");
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
