#![cfg(feature = "corpus")]
use photocraft_algo::gimp::alpha_clip::AlphaClip;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
fn blob(root: &Path, row: &serde_json::Value) -> Vec<u8> {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), row["sha256"].as_str().unwrap());
    let mut decoded = Vec::new();
    flate2::read::ZlibDecoder::new(bytes.as_slice()).read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded.len() as u64, row["decoded_bytes"].as_u64().unwrap());
    assert_eq!(format!("{:x}", Sha256::digest(&decoded)), row["decoded_sha256"].as_str().unwrap());
    decoded
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn default_and_equal_limits_preserve_observed_signed_zero_native_alpha() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("point-alpha-clip-default-boundary-evidence");
    let authority = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&authority)), "9ff0215a89bf091bde1f255bcc26e82e2c91ae7ee27b3c47f4a7489e8b8bf6fe");
    let d: serde_json::Value = serde_json::from_slice(&authority).unwrap();
    for r in d["records"].as_array().unwrap().iter().filter(|r| r["id"].as_str().unwrap().starts_with("rgb-double_linear.")) {
        let source = blob(&root, &r["reopened"]["layers"][0]["native"]);
        let reference = blob(&root, &r["baked"]["layers"][0]["native"]);
        let props = &r["requested"]["properties"];
        let op = AlphaClip {
            clip_low: true,
            low_limit: props["low-limit"].as_f64().unwrap() as f32,
            clip_high: true,
            high_limit: props["high-limit"].as_f64().unwrap() as f32,
        };
        for (a, b) in source.as_chunks::<32>().0.iter().zip(reference.as_chunks::<32>().0.iter()) {
            let input: Vec<_> = a.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect();
            let expected: Vec<_> = b.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect();
            let mut actual = [0.; 4];
            op.try_eval_pixel(&input, 3, &mut actual).unwrap();
            assert_eq!(actual.map(f64::to_bits).as_slice(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>().as_slice());
        }
    }
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn literal_double_linear_active_clamp_matches_independent_native_rgba() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("point-alpha-clip-working-format-evidence");
    let authority = fs::read(root.join("clamp-observations.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&authority)), "2e733a37eaa447ab06e77d9a3225de68c631a630c185b6c80149594a58975add");
    let d: serde_json::Value = serde_json::from_slice(&authority).unwrap();
    let r = d["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-double_linear.clamp-active").unwrap();
    let original = fs::read(root.join(r["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(original)), r["sha256"].as_str().unwrap());
    let source = blob(&root, &r["reopened"]["layers"][0]["native"]);
    let reference = blob(&root, &r["baked"]["layers"][0]["native"]);
    let source_before = source.clone();
    let op = AlphaClip { clip_low: true, low_limit: 0.25, clip_high: true, high_limit: 0.75 };
    for (a, b) in source.as_chunks::<32>().0.iter().zip(reference.as_chunks::<32>().0.iter()) {
        let input: Vec<_> = a.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect();
        let expected: Vec<_> = b.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect();
        let mut actual = [0.; 4];
        op.try_eval_pixel(&input, 3, &mut actual).unwrap();
        assert_eq!(actual.map(f64::to_bits), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>().as_slice());
    }
    assert_eq!(source, source_before);
}

fn native_alpha(root: &Path, r: &serde_json::Value, key: &str) -> Vec<(f64, u64)> {
    let raw = blob(root, &r[key]["layers"][0]["native"]);
    let precision = r["precision"].as_str().unwrap().split('_').next().unwrap();
    let channels = if r["model"] == "RGB" { 4 } else { 2 };
    let bytes = match precision {
        "U8" => 1,
        "U16" | "HALF" => 2,
        "U32" | "FLOAT" => 4,
        "DOUBLE" => 8,
        _ => panic!("unexpected precision"),
    };
    raw.chunks_exact(bytes * channels)
        .map(|p| {
            let a = &p[bytes * (channels - 1)..];
            match precision {
                "U8" => (f64::from(a[0]) / 255., u64::from(a[0])),
                "U16" => {
                    let v = u16::from_le_bytes(a.try_into().unwrap());
                    (f64::from(v) / 65535., u64::from(v))
                }
                "U32" => {
                    let v = u32::from_le_bytes(a.try_into().unwrap());
                    (f64::from(v) / 4294967295., u64::from(v))
                }
                "HALF" => {
                    let v = u16::from_le_bytes(a.try_into().unwrap());
                    (f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(v))), u64::from(v))
                }
                "FLOAT" => {
                    let v = u32::from_le_bytes(a.try_into().unwrap());
                    (f64::from(f32::from_bits(v)), u64::from(v))
                }
                "DOUBLE" => {
                    let v = u64::from_le_bytes(a.try_into().unwrap());
                    (f64::from_bits(v), v)
                }
                _ => panic!("unexpected precision"),
            }
        })
        .collect()
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_alpha_all_six_native_depths_and_transfers_have_separate_output_quantization() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("point-alpha-clip-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "527bc502322bcbe33d0ecb7a659363be85d0ce99a46e1fc552a77d0c77cef38d");
    let d: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut count = 0;
    let mut worst_u8 = 0f64;
    for r in d["records"].as_array().unwrap() {
        if r["role"].as_str().unwrap().starts_with("diagnostic") {
            continue;
        }
        let original = fs::read(root.join(r["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&original)), r["sha256"].as_str().unwrap());
        let original_before = original.clone();
        let props = &r["requested"]["properties"];
        let op = AlphaClip {
            clip_low: props["clip-low"].as_bool().unwrap(),
            low_limit: props["low-limit"].as_f64().unwrap() as f32,
            clip_high: props["clip-high"].as_bool().unwrap(),
            high_limit: props["high-limit"].as_f64().unwrap() as f32,
        };
        let precision = r["precision"].as_str().unwrap().split('_').next().unwrap();
        let source = native_alpha(&root, r, "reopened");
        let result = native_alpha(&root, r, "baked");
        assert_eq!(source.len(), 48);
        assert_eq!(result.len(), 48);
        for ((input, _), (reference, code)) in source.into_iter().zip(result) {
            let mut out = [0.; 4];
            op.try_eval_pixel_with(
                &[0.125, 0.375, 0.625, input],
                3,
                &mut out,
                r["requested"]["enabled"].as_bool().unwrap(),
                &photocraft_raster::Interrupt::NONE,
                &|_| Ok(()),
            )
            .unwrap();
            let a = out[3];
            match precision {
                "U8" => {
                    assert_eq!((a * 255.).round() as u64, code);
                    worst_u8 = worst_u8.max((a - reference).abs());
                }
                "U16" => assert_eq!((a * 65535.).round() as u64, code),
                "U32" => assert!(((a * 4294967295.).round() - (code as f64)).abs() <= 1., "{} integer alpha oneLSB", r["id"]),
                "HALF" => assert_eq!(u64::from(photocraft_color::try_round_f32_to_f16_bits(a as f32).unwrap()), code),
                "FLOAT" => assert_eq!(u64::from((a as f32).to_bits()), code),
                "DOUBLE" => assert_eq!(a.to_bits(), code),
                _ => panic!("unexpected precision"),
            }
        }
        assert_eq!(original, original_before);
        count += 1;
    }
    assert_eq!(count, 132);
    assert!(worst_u8 <= 0.5 / 255. + f64::from(f32::EPSILON));
    println!("Alpha Clip132 nativealpha members; U8 derived half-code maximum {worst_u8}");
}
