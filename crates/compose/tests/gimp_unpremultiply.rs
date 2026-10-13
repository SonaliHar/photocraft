use photocraft_cms::Builtin;
use photocraft_color::PixelFormat;
use photocraft_compose::gimp_unpremultiply::{UnpremultiplyCandidate, typed_arguments};
use photocraft_doc::SampleEncoding;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError};
fn candidate() -> UnpremultiplyCandidate {
    UnpremultiplyCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Linear, PixelFormat::RGBA32F, Rect::new(0, 0, 8, 6)).unwrap()
}
#[test]
fn native_domain_refuses_profile_precision_model_encoding_geometry() {
    let make = |profile, encoding, format, owner| UnpremultiplyCandidate::new(profile, encoding, format, owner);
    for profile in [Builtin::SGray.profile(), Builtin::DisplayP3.profile(), Builtin::LinearSrgb.profile()] {
        assert!(make(profile, SampleEncoding::Linear, PixelFormat::RGBA32F, Rect::new(0, 0, 8, 6)).is_err());
    }
    for encoding in [SampleEncoding::NonlinearProfile, SampleEncoding::Perceptual] {
        assert!(make(Builtin::Srgb.profile(), encoding, PixelFormat::RGBA32F, Rect::new(0, 0, 8, 6)).is_err());
    }
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA64F, PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, false)] {
        assert!(make(Builtin::Srgb.profile(), SampleEncoding::Linear, format, Rect::new(0, 0, 8, 6)).is_err());
    }
    for owner in [Rect::new(1, 0, 9, 6), Rect::new(0, 0, 8, 7), Rect::new(0, 0, 7, 6)] {
        assert!(make(Builtin::Srgb.profile(), SampleEncoding::Linear, PixelFormat::RGBA32F, owner).is_err());
    }
    assert!(typed_arguments(&[photocraft_doc::xcf_filters::FilterArgument { name: "alpha".into(), value: photocraft_doc::xcf_filters::FilterValue::Bool(true) }]).is_err());
}
#[test]
fn checked_native_division_disabled_identity_and_atomic_failures() {
    let c = candidate();
    let input = [0.25, -0.5, 0.75, 0.5];
    assert_eq!(c.apply_with(&input, true, &Interrupt::NONE, &|_| Ok(())).unwrap(), [0.5, -1., 1.5, 0.5]);
    let hidden = [-0.0, -1., 6., -0.0];
    let zero = c.apply_with(&hidden, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
    assert_eq!(zero.map(f64::to_bits), [0., 0., 0., -0.0].map(f64::to_bits));
    assert_eq!(c.apply_with(&hidden, false, &Interrupt::NONE, &|_| Ok(())).unwrap().map(f64::to_bits), hidden.map(f64::to_bits));
    for enabled in [true, false] {
        for bad in [[f64::NAN, 0., 0., 1.], [0., 0., 0., 1.5], [0.1, 0., 0., 1.], [f64::MAX, 0., 0., 1.]] {
            assert!(c.apply_with(&bad, enabled, &Interrupt::NONE, &|_| Ok(())).is_err());
        }
        assert!(c.apply_with(&input, enabled, &Interrupt::cancel_only(&|| true), &|_| Ok(())).is_err());
        assert!(c.apply_with(&input, enabled, &Interrupt::NONE, &|_| Err(RasterInputError::InvalidSample)).is_err());
    }
    let overflow = [1., 0., 0., f64::from(f32::from_bits(1))];
    assert!(c.apply_with(&overflow, true, &Interrupt::NONE, &|_| Ok(())).is_err());
    assert_eq!(c.apply_with(&overflow, false, &Interrupt::NONE, &|_| Ok(())).unwrap(), overflow);
    let polls = std::sync::atomic::AtomicU32::new(0);
    let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 >= 3;
    assert!(c.apply_with(&input, true, &Interrupt::cancel_only(&cancel), &|_| Ok(())).is_err());
    assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 3);
}
#[test]
#[ignore = "three frozen independent RGB Float Linear Unpremultiply originals"]
fn independent_native_source_edit_disabled_three_exact() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/point-unpremultiply-evidence");
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), "f40f522c4e8b7e7bab1e00b4e2d5fd2729c90062fbc36cb93172f362cd406f3d");
    let j: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let read = |d: &serde_json::Value| {
        let raw = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&raw), d["sha256"]); assert!(raw.len() < 4096);
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(raw.as_slice()).take(769).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded.len(), 768); assert_eq!(hash(&decoded), d["decoded_sha256"]);
        decoded.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect::<Vec<_>>()
    };
    let mut count = 0;
    for row in j["records"].as_array().unwrap().iter().filter(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR") {
        count += 1;
        let original = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap(); assert_eq!(hash(&original), row["sha256"]);
        assert!(row["reopened"]["profiles"]["assigned"].is_null());
        let source = read(&row["reopened"]["layers"][0]["native"]);
        let expected = read(&row["baked"]["layers"][0]["native"]);
        for (src, dst) in source.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()) {
            let actual = candidate().apply_with(src, row["requested"]["enabled"].as_bool().unwrap(), &Interrupt::NONE, &|_| Ok(())).unwrap();
            assert_eq!(actual.map(f64::to_bits), dst.map(f64::to_bits));
        }
        assert_eq!(std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), original);
    }
    assert_eq!(count, 3);
}
