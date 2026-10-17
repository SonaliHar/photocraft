//! Actual implicit Gray NL sources. Native/profile provenance and derived display are separate.
//! Full Surface reference allocations are external harness, not budget evidence.
//! Producer F32 projection differences and profile date HOLDs remain explicit.
//! GIMP effective/Babl profile byte roles are not equated or normalized.
#[path = "observed_box_move_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod current;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
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
// External oracle storage only: this bound is per logical buffer, not whole peak or quota.
fn bulk_gray_linear_reference(bounds: photocraft_doc::Rect, values: &[f64]) -> photocraft_raster::Surface {
    let format = photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Rgb, SampleType::F64, true);
    let bytes = photocraft_raster::checked_region_byte_len(bounds, format).unwrap();
    assert!(bounds.width() <= 1024 && bounds.height() <= 1024 && bytes <= 16 << 20);
    assert_eq!(values.len().checked_mul(std::mem::size_of::<f64>()), Some(bytes));
    let mut linear = photocraft_raster::Surface::new(format);
    linear.try_write_region_f64(bounds, values).unwrap();
    linear
}
#[test]
fn bulk_gray_box_reference_preserves_cross_tile_raw_words_and_padding() {
    let bounds = photocraft_doc::Rect::new(-1, 255, 2, 258);
    let format = photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Rgb, SampleType::F64, true);
    let mut old = photocraft_raster::Surface::new(format);
    let mut values = Vec::new();
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let gray = if x == -1 {
                -0.0
            } else if x == 0 {
                f64::from_bits(0x3fd0000000000001)
            } else {
                1.125
            };
            let alpha = if y == 256 { -0.0 } else { 0.75 };
            let pixel = [gray, gray, gray, alpha];
            old.try_write_pixel_f64(x, y, &pixel).unwrap();
            values.extend_from_slice(&pixel);
        }
    }
    let bulk = bulk_gray_linear_reference(bounds, &values);
    assert_eq!(old.format(), bulk.format());
    assert_eq!(old.default_bytes(), bulk.default_bytes());
    assert_eq!(
        old.try_content_bounds_with_interrupt(&photocraft_raster::Interrupt::NONE).unwrap(),
        bulk.try_content_bounds_with_interrupt(&photocraft_raster::Interrupt::NONE).unwrap()
    );
    assert_eq!(old.try_read_interleaved(bounds).unwrap(), bulk.try_read_interleaved(bounds).unwrap());
    let old_tiles: Vec<_> = old.tiles().map(|(coord, tile)| (*coord, tile.bytes().to_vec())).collect();
    let bulk_tiles: Vec<_> = bulk.tiles().map(|(coord, tile)| (*coord, tile.bytes().to_vec())).collect();
    assert_eq!(old_tiles, bulk_tiles);
}
pub(super) fn reference(doc: &Document) -> photocraft_compose::Buffer {
    use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    // Actual YA source stays two channels. Only the external working reference repeats gray.
    let profile = photocraft_cms::Builtin::SGray.profile();
    let gray_curve = profile.gray_trc.as_ref().unwrap();
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("native") };
    let literal = native::Literal { bounds: backed.operative_bounds(), raw: native::native(doc), sample: doc.depth };
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let FilterValue::Number(ExactNumber::Signed { width: 32, value: radius }) = node.arguments[0].value else { panic!("radius") };
    // Existing full Surface kernel runs anew over independently shifted raw
    // samples. Never translate a previously filtered bitmap: global256 resets
    // are observable for unaligned moves. All reference buffers are external.
    let format = photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Rgb, SampleType::F64, true);
    let bytes = photocraft_raster::checked_region_byte_len(literal.bounds, format).unwrap();
    assert!(literal.bounds.width() <= 1024 && literal.bounds.height() <= 1024 && bytes <= 16 << 20);
    let mut values = Vec::new();
    values.try_reserve_exact(bytes / std::mem::size_of::<f64>()).unwrap();
    for y in literal.bounds.y0..literal.bounds.y1 {
        for x in literal.bounds.x0..literal.bounds.x1 {
            let p = gray_sample(&literal, x, y);
            values.extend_from_slice(&[gray_curve.eval64(p[0]), gray_curve.eval64(p[1]), gray_curve.eval64(p[2]), p[3]]);
        }
    }
    let linear = bulk_gray_linear_reference(literal.bounds, &values);
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
    let normal =
        photocraft_compose::gimp_composite::WorkingSpace::new(profile, photocraft_doc::SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    let opacity = if doc.layers[0].visible { f64::from(doc.layers[0].opacity) } else { 0.0 };
    let mut px = Vec::new();
    for y in 0..doc.size.height as i32 {
        for x in 0..doc.size.width as i32 {
            let p = if extent.contains(x, y) {
                if let Some(filtered) = &filtered {
                    let mut p = [0.; 4];
                    filtered.try_read_pixel_f64(x, y, &mut p).unwrap();
                    [gray_curve.eval_inverse64(p[0]), gray_curve.eval_inverse64(p[1]), gray_curve.eval_inverse64(p[2]), p[3]]
                } else {
                    gray_sample(&literal, x, y)
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
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol4-gray-box-gimp-20261016");
    [
        ("f32-author-v1", "implicit-gray-f32-nl-hdr-r4.xcf", 191079, 4, "9ed61f5c50bd717e455a96a73af14ac27752b6446c66efe74458df7925299f87"),
        ("f32-author-v1", "implicit-gray-f32-nl-hdr-r5.xcf", 191079, 5, "0787259a050e80d6ad37633a3ff63a5d8bff539618460ddc9136fdc0121872d3"),
        ("f64-author-v1", "implicit-gray-f64-nl-hdr-r4.xcf", 108944, 4, "e2b5ab1845d89cbae5d23748c30acc989cb948cce23136941c0bb73774fdab2e"),
        ("f64-author-v1", "implicit-gray-f64-nl-hdr-r5.xcf", 108944, 5, "0f5c90d068b879ec905185e7405260934f0b880b2e4f7034c8b2fddcd6ba380f"),
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
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Grayscale);
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("YA native") };
    assert_eq!(backed.format(), photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, doc.depth, true));
    assert_eq!(backed.default_bytes().len(), 2 * doc.depth.bytes());
    let mask = &doc.xcf_state.as_ref().unwrap().filter_masks[0];
    assert_eq!(mask.surface.format(), photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, doc.depth, false));
    assert_eq!(mask.surface.default_bytes().len(), doc.depth.bytes());
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    let source = doc.source_color_context.as_ref().unwrap();
    assert!(source.original_icc.is_none());
    assert!(matches!(&doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { .. }));
    // Generated CURRENT SGray ICC is not a producer profile equivalence claim.
    assert!(doc.icc_profile.is_some());
    let (precision, width, hash) = match doc.depth {
        SampleType::F32 => (650, 4, "62cc23617c9de7df2a822255dc020fc352dbb9a253132af9cdbecbb75d4b06e1"),
        SampleType::F64 => (750, 8, "a288721d3c735f179f717660120044ad203cac317f963adff3534ffd00da7f6c"),
        _ => panic!("exact observed GrayNL role"),
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
    assert_eq!(little.len(), 257 * 259 * 2 * width);
    assert_eq!(format!("{:x}", Sha256::digest(&little)), hash);
    if width == 4 {
        for (offset, bits) in [(206400, 0xbe000000u32), (206408, 0x3fa00000), (206416, 0x80000000), (208456, 0x3f800008), (208464, 0x8d800000)] {
            assert_eq!(u32::from_le_bytes(little[offset..offset + 4].try_into().unwrap()), bits);
        }
    } else {
        for (offset, bits) in [
            (412800, 0xbfc0000000000000u64),
            (412816, 0x3ff4000000000000),
            (412832, 0x8000000000000000),
            (416912, 0x3ff0000000001000),
            (416928, 0xb690000000000000),
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
// The source still matches Box/GrayNL routing; its radius payload exceeds policy.
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

fn gray_sample(literal: &native::Literal, x: i32, y: i32) -> [f64; 4] {
    let width = literal.sample.bytes();
    let at = ((y - literal.bounds.y0) as usize * literal.bounds.width() as usize + (x - literal.bounds.x0) as usize) * 2 * width;
    let read = |channel| {
        let offset = at + channel * width;
        if literal.sample == SampleType::F32 {
            f64::from(f32::from_ne_bytes(literal.raw[offset..offset + 4].try_into().unwrap()))
        } else {
            f64::from_ne_bytes(literal.raw[offset..offset + 8].try_into().unwrap())
        }
    };
    let gray = read(0);
    [gray, gray, gray, read(1)]
}
