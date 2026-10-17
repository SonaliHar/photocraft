#![cfg(feature = "corpus")]
//! Three independently decoded RGB Float Linear originals; no other precision admission.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn packet() -> (PathBuf, Value) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-unpremultiply-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), "f40f522c4e8b7e7bab1e00b4e2d5fd2729c90062fbc36cb93172f362cd406f3d");
    (root, serde_json::from_slice(&bytes).unwrap())
}
fn bound(root: &Path, descriptor: &Value) -> Vec<u8> {
    let name = descriptor["file"].as_str().unwrap();
    assert!(Path::new(name).components().all(|c| matches!(c, std::path::Component::Normal(_))));
    let bytes = fs::read(root.join(name)).unwrap();
    assert_eq!(hash(&bytes), descriptor["sha256"], "{name}");
    if descriptor["encoding"] == "zlib" {
        let length = descriptor["decoded_bytes"].as_u64().unwrap();
        assert!(length <= 4096);
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(bytes.as_slice()).take(length + 1).read_to_end(&mut raw).unwrap();
        assert_eq!(raw.len() as u64, length);
        assert_eq!(hash(&raw), descriptor["decoded_sha256"]);
        raw
    } else {
        bytes
    }
}
#[test]
#[ignore = "independent108 originals: three Float Linear RGB imports and105 explicit refusals"]
fn actual108_import_admits_only_three_rgb_float_linear_originals() {
    let (root, report) = packet();
    let rows = report["records"].as_array().unwrap();
    assert_eq!(rows.len(), 108);
    let mut count = (0, 0);
    for row in rows {
        let source = bound(&root, row);
        let result = photocraft_io::import("unpremultiply.xcf", &source);
        if row["model"] == "RGB" && row["precision"] == "FLOAT_LINEAR" {
            let doc = result.unwrap_or_else(|e| panic!("{}: {e}", row["id"]));
            let doc = doc.document;
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), bound(&root, &row["reopened"]["layers"][0]["native"]));
            let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(node.operation.schema_epoch, "xcf-22-stock-unpremultiply-rgb-f32-linear-v1");
            assert!(node.arguments.is_empty());
            assert_eq!(node.enabled, row["requested"]["enabled"].as_bool().unwrap());
            count.0 += 1;
        } else {
            assert!(matches!(result, Err(photocraft_io::IoError::Unsupported(_))), "{}", row["id"]);
            count.1 += 1;
        }
        assert_eq!(fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), source);
    }
    assert_eq!(count, (3, 105));
}

#[test]
#[ignore = "independent three qualified originals, unchanged native pixels/opaque/ICC and live projections"]
fn actual_three_native_reopen_and_gimp_projection() {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    let (root, report) = packet();
    let mut count = 0;
    let mut maximum = 0f64;
    for row in report["records"].as_array().unwrap().iter().filter(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR") {
        let bytes = bound(&root, row);
        let doc = photocraft_io::import("unpremultiply.xcf", &bytes).unwrap().document;
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened, doc);
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(streamed, doc);
        let projection = bound(&root, &row["reopened"]["projection"]);
        let profile = implicit_source_profile(doc.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, Encoding::Linear).unwrap();
        let reference = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(reference.px.len(), 48);
        assert_eq!(projection.len(), 48 * 32);
        for candidate in [&doc, &reopened, &streamed] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px, reference.px);
            for (pixel, raw) in rendered.px.iter().zip(projection.as_chunks::<32>().0) {
                let mut color = [0.; 3];
                convert_encoding_f64(&profile, Encoding::Linear, Encoding::NonlinearProfile, &pixel[..3], &mut color).unwrap();
                let expected: [f64; 4] = std::array::from_fn(|i| f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap()));
                assert!((pixel[3] - expected[3]).abs() <= 1. / 65535.);
                for i in 0..3 {
                    let delta = (color[i] * pixel[3] - expected[i] * expected[3]).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} delta{}/255", row["id"], delta * 255.);
                }
            }
        }
        assert_eq!(fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), bytes);
        count += 1;
    }
    assert_eq!(count, 3);
    println!("Unpremultiply native/projection3 max{}/255", maximum * 255.);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
#[ignore = "three small resident controls through existing accounted native archive, exact state and refunds"]
fn actual_three_accounted_native_archive_keeps_state_and_refunds() {
    use photocraft_format::owned_document::OwnedDocumentLimits;
    use photocraft_io::{accounted_native::*, xcf_source::ServiceAuthorizedScratchDirectory};
    use photocraft_raster::{
        Interrupt,
        backing::{ReadControl, quota::*},
    };
    use photocraft_xcf::{Budget, ResourceLimits, ResourcePolicy};
    let (root, report) = packet();
    let mut count = 0;
    for row in report["records"].as_array().unwrap().iter().filter(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR") {
        let source = bound(&root, row);
        let doc = photocraft_io::import("unpremultiply.xcf", &source).unwrap().document;
        let path = std::env::temp_dir().join(format!("photocraft-unpremultiply-native-{}-{count}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let scratch = path.join("scratch");
        fs::create_dir(&scratch).unwrap();
        let native = path.join("current.pcraft");
        let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, scratch: 128 << 20, index: 16 << 20 });
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let write = ServiceAuthorizedNativeWritePath::from_platform_authorized(native.clone(), &q, &ctl).unwrap();
        save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &ctl, |_, _| Ok(())).unwrap();
        let saved = fs::read(&native).unwrap();
        let usage = q.usage().unwrap();
        let gates = std::sync::atomic::AtomicUsize::new(0);
        assert!(
            save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &ctl, |_, _| {
                if gates.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Ok(())
                } else {
                    Err(photocraft_raster::backing::BackingError::Cancelled.into())
                }
            })
            .is_err()
        );
        assert_eq!(fs::read(&native).unwrap(), saved);
        assert_eq!(q.usage().unwrap(), usage);
        let read = ServiceAuthorizedNativeReadPath::from_platform_authorized(native.clone(), &q, &ctl).unwrap();
        let owned = {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            open_accounted_native(&read, &service, OwnedDocumentLimits::default(), q.clone(), &ctl).unwrap()
        };
        let mut expected_native = doc.clone();
        expected_native.name = native.file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(*owned.document(), expected_native);
        let expected = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let observed = photocraft_compose::gimp_render::try_render(owned.document(), doc.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(observed.px, expected.px);
        drop(read);
        drop(write);
        fs::remove_file(native).unwrap();
        assert!(q.usage().unwrap().resident > 0);
        // The owned archive supports this small resident control. The separate
        // application AccountedDocument lifecycle requires accounted raster
        // storage and remains explicitly unqualified for these resident layers.
        assert!(matches!(owned.into_accounted_document(&ctl),Err(photocraft_format::FormatError::Io(error))
            if error.get_ref().and_then(|source|source.downcast_ref::<photocraft_raster::backing::BackingError>())==Some(&photocraft_raster::backing::BackingError::Unsupported)));
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        assert_eq!(fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), source);
        fs::remove_dir_all(path).unwrap();
        count += 1;
    }
    assert_eq!(count, 3);
}
