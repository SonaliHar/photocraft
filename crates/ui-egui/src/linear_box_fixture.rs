//! Actual implicit Linear sources. Native/profile provenance and derived display are separate.
//! Full Surface reference allocations are external harness, not budget evidence.
//! GIMP effective/Babl profile byte roles are not equated or normalized.
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
    use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    // Nonlinear sRGB is only the known basis; samples and result remain Linear.
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("native") };
    let literal = native::Literal { bounds: backed.operative_bounds(), raw: native::native(doc), sample: doc.depth };
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let FilterValue::Number(ExactNumber::Signed { width: 32, value: radius }) = node.arguments[0].value else { panic!("radius") };
    // Existing full Surface kernel runs anew over independently shifted raw
    // samples. Never translate a previously filtered bitmap: global256 resets
    // are observable for unaligned moves. All reference buffers are external.
    let mut linear = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Rgb, SampleType::F64, true));
    for y in literal.bounds.y0..literal.bounds.y1 {
        for x in literal.bounds.x0..literal.bounds.x1 {
            let p = native::sample(&literal, x, y);
            linear.try_write_pixel_f64(x, y, &p).unwrap();
        }
    }
    let extent = if node.enabled { photocraft_algo::gimp::neighborhood::box_blur_extent(literal.bounds, radius as u32).unwrap() } else { literal.bounds };
    let filtered = node.enabled.then(|| {
        photocraft_algo::gimp::neighborhood::try_box_blur(
            &linear,
            extent,
            literal.bounds,
            radius as u32,
            Default::default(),
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap()
    });
    let normal = photocraft_compose::gimp_composite::WorkingSpace::new(profile, photocraft_doc::SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let opacity = if doc.layers[0].visible { f64::from(doc.layers[0].opacity) } else { 0.0 };
    let mut px = Vec::new();
    for y in 0..doc.size.height as i32 {
        for x in 0..doc.size.width as i32 {
            let p = if extent.contains(x, y) {
                if let Some(filtered) = &filtered {
                    let mut p = [0.; 4];
                    filtered.try_read_pixel_f64(x, y, &mut p).unwrap();
                    p
                } else {
                    native::sample(&literal, x, y)
                }
            } else {
                [0.; 4]
            };
            px.push(normal.normal([0.; 4], p, opacity, GimpOperator::Union).unwrap().map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: doc.bounds(), px }
}

pub(super) fn cases() -> Vec<(std::path::PathBuf, Vec<u8>, i32)> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol4-linear-box-gimp-20261016");
    [
        ("f32-author-v1", "implicit-f32-linear-hdr-r4.xcf", 313818, 4, "0e33a73c4fe0c61ea989ae378433d365a92620273208f8c66196b7d30ce9261e"),
        ("f32-author-v1", "implicit-f32-linear-hdr-r5.xcf", 313818, 5, "56231c40bcb120e8d5b9b3693a8e74c4b7269a108e659770055fea7be03b13aa"),
        ("f64-author-v1", "implicit-f64-linear-hdr-r4.xcf", 206721, 4, "06129bd442bcbdfce2f821ba700cbada76f5303ff86656a7526ce1a1a46c2fc7"),
        ("f64-author-v1", "implicit-f64-linear-hdr-r5.xcf", 206721, 5, "136e581599fd6bdf9321fdc88561a1774ff1964ee2553a4cf4a06a678135e8b1"),
    ]
    .into_iter()
    .map(|(run, name, len, radius, hash)| {
        let path = root.join(run).join("outputs").join(name);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), len);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        (path, bytes, radius)
    })
    .collect()
}
pub(super) fn import(name: &str, bytes: Vec<u8>) -> photocraft_io::OwnedXcfImport {
    try_import(name, bytes).unwrap()
}
pub(super) fn try_import(name: &str, bytes: Vec<u8>) -> Result<photocraft_io::OwnedXcfImport, photocraft_io::IoError> {
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
pub(super) fn observed(doc: &Document) {
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::Linear));
    let source = doc.source_color_context.as_ref().unwrap();
    assert!(source.original_icc.is_none());
    assert!(doc.icc_profile.is_some()); // Generated CURRENT effective Linear profile, not assigned ICC.
    let (precision, width, hash) = match doc.depth {
        SampleType::F32 => (600, 4, "76320fc394402bd023260ecd0ed09a64780b1012106811e9a139c00edd898f4f"),
        SampleType::F64 => (700, 8, "15cea9161eeb5ebbd9d0aa4ffaf29215c1ebb22b448986a0c1283a261f9a91fc"),
        _ => panic!("exact observed Linear role"),
    };
    assert_eq!(source.source_precision, precision);
    let raw = native::native(doc);
    let little: Vec<u8> = raw
        .chunks_exact(width)
        .flat_map(|w| {
            if width == 4 {
                u32::from_ne_bytes(w.try_into().unwrap()).to_le_bytes().to_vec()
            } else {
                u64::from_ne_bytes(w.try_into().unwrap()).to_le_bytes().to_vec()
            }
        })
        .collect();
    assert_eq!(little.len(), 257 * 259 * 4 * width);
    assert_eq!(format!("{:x}", Sha256::digest(&little)), hash);
    if width == 4 {
        for (offset, bits) in [(412800, 0xbe000000u32), (412820, 0x3fa00000), (412840, 0x80000000), (416912, 0x3f800008), (416932, 0x8d800000)] {
            assert_eq!(u32::from_le_bytes(little[offset..offset + 4].try_into().unwrap()), bits);
        }
    } else {
        for (offset, bits) in [
            (825600, 0xbfc0000000000000u64),
            (825640, 0x3ff4000000000000),
            (825680, 0x8000000000000000),
            (833824, 0x3ff0000000001000),
            (833864, 0xb690000000000000),
        ] {
            assert_eq!(u64::from_le_bytes(little[offset..offset + 8].try_into().unwrap()), bits);
        }
    }
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.schema_epoch, "xcf22-observed-box-geometry-v1");
    let policy = node.composition.as_ref().unwrap();
    assert_eq!([policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator], [0x30038251; 3]);
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

// Deliberately malformed copy of a pinned source, never an authored witness.
// The source still matches Box/Linear routing; its radius payload exceeds policy.
pub(super) fn malformed_radius(bytes: &[u8], radius: i32) -> Vec<u8> {
    let mut needle = vec![0, 0, 0, 7, b'r', b'a', b'd', b'i', b'u', b's', 0, 0, 0, 0, 1];
    needle.extend_from_slice(&radius.to_be_bytes());
    let offsets: Vec<_> = bytes.windows(needle.len()).enumerate().filter_map(|(i, w)| (w == needle).then_some(i)).collect();
    assert_eq!(offsets.len(), 1);
    let mut bad = bytes.to_vec();
    let end = offsets[0] + needle.len();
    bad[end - 4..end].copy_from_slice(&1001i32.to_be_bytes());
    bad
}
