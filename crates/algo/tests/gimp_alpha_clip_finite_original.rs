#![cfg(feature = "corpus")]
use photocraft_algo::gimp::alpha_clip_finite::AlphaClipFinite;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
fn blob(root: &Path, row: &serde_json::Value) -> Vec<u8> {
    let encoded = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&encoded)), row["sha256"].as_str().unwrap());
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(encoded.as_slice()).read_to_end(&mut bytes).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), row["decoded_sha256"].as_str().unwrap());
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn crossed_finite_thresholds_match_independent_native_alpha() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("point-alpha-clip-value-domain-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let d: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let r = d["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-double_linear.alpha-nonunit.crossed").unwrap();
    let input = blob(&root, &r["reopened"]["layers"][0]["native"]);
    let output = blob(&root, &r["baked"]["layers"][0]["native"]);
    let op = AlphaClipFinite { clip_low: true, low_limit: 0.875, clip_high: true, high_limit: 0.125 };
    for (a, b) in input.as_chunks::<32>().0.iter().zip(output.as_chunks::<32>().0.iter()) {
        let source: Vec<_> = a.as_chunks::<8>().0.iter().map(|x| f64::from_le_bytes(*x)).collect();
        let expected: Vec<_> = b.as_chunks::<8>().0.iter().map(|x| f64::from_le_bytes(*x)).collect();
        let mut actual = [0.; 4];
        op.try_eval_pixel(&source, &mut actual).unwrap();
        assert_eq!(actual.map(f64::to_bits).as_slice(), expected.iter().map(|x| x.to_bits()).collect::<Vec<_>>().as_slice());
    }
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn all_240_source_working_and_independent_native_conversion_controls() {
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let control = full.join("point-alpha-clip-finite-working-controls");
    let authority = fs::read(control.join("observations.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&authority)), "28353fe06bfdabf8c9b3b5ed394594e77e3e5e858a9ca73d12a5da709503acee");
    let d: serde_json::Value = serde_json::from_slice(&authority).unwrap();
    let mut active = 0;
    let mut identity = 0;
    for r in d["records"].as_array().unwrap() {
        let origin = full.join(r["original_folder"].as_str().unwrap());
        let original: serde_json::Value = serde_json::from_slice(&fs::read(origin.join("observations.json")).unwrap()).unwrap();
        let old = original["records"].as_array().unwrap().iter().find(|v| v["id"] == r["original_id"]).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(fs::read(origin.join(old["file"].as_str().unwrap())).unwrap())), old["sha256"].as_str().unwrap());
        let p = &r["requested"]["properties"];
        let op = AlphaClipFinite {
            clip_low: p["clip-low"].as_bool().unwrap(),
            low_limit: p["low-limit"].as_f64().unwrap() as f32,
            clip_high: p["clip-high"].as_bool().unwrap(),
            high_limit: p["high-limit"].as_f64().unwrap() as f32,
        };
        let enabled = r["requested"]["enabled"].as_bool().unwrap();
        let source = blob(&origin, &old["reopened"]["layers"][0]["native"]);
        let reference = blob(&origin, &old["baked"]["layers"][0]["native"]);
        if r["active"].as_bool().unwrap() {
            active += 1;
            let input = blob(&control, &r["working_source"]);
            let expected = blob(&control, &r["predicted_working"]);
            let before = input.clone();
            for (a, b) in input.as_chunks::<16>().0.iter().zip(expected.as_chunks::<16>().0.iter()) {
                let values: Vec<_> = a.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
                let predicted: Vec<_> = b.as_chunks::<4>().0.iter().map(|v| f32::from_le_bytes(*v).to_bits()).collect();
                let mut output = [0.; 4];
                op.try_eval_pixel_with(&values, &mut output, enabled, &photocraft_raster::Interrupt::NONE, &|_| Ok(())).unwrap();
                assert_eq!(output.map(|v| (v as f32).to_bits()).as_slice(), predicted.as_slice());
            }
            assert_eq!(input, before);
            assert!(r["predicted_native_exact"].as_bool().unwrap());
            assert_eq!(blob(&control, &r["predicted_native"]["layers"][0]["native"]), reference);
        } else {
            identity += 1;
            assert_eq!(source, reference);
            let component = if r["precision"].as_str().unwrap().starts_with("FLOAT") { 4 } else { 8 };
            let channels = if r["model"] == "RGB" { 4 } else { 2 };
            for pixel in source.chunks_exact(component * channels) {
                let values: Vec<_> = pixel
                    .chunks_exact(component)
                    .map(|v| if component == 4 { f64::from(f32::from_le_bytes(v.try_into().unwrap())) } else { f64::from_le_bytes(v.try_into().unwrap()) })
                    .collect();
                let input = if channels == 4 { [values[0], values[1], values[2], values[3]] } else { [values[0], values[0], values[0], values[1]] };
                let mut output = [0.; 4];
                op.try_eval_pixel_with(&input, &mut output, enabled, &photocraft_raster::Interrupt::NONE, &|_| Ok(())).unwrap();
                assert_eq!(output.map(f64::to_bits), input.map(f64::to_bits));
            }
        }
    }
    assert_eq!((active, identity), (144, 96));
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn all_twelve_default_equal_and_signed_zero_native_alpha_controls() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("point-alpha-clip-default-boundary-evidence");
    let authority = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&authority)), "9ff0215a89bf091bde1f255bcc26e82e2c91ae7ee27b3c47f4a7489e8b8bf6fe");
    let d: serde_json::Value = serde_json::from_slice(&authority).unwrap();
    let mut count = 0;
    for r in d["records"].as_array().unwrap() {
        let input = blob(&root, &r["reopened"]["layers"][0]["native"]);
        let expected = blob(&root, &r["baked"]["layers"][0]["native"]);
        let channels = if r["model"] == "RGB" { 4 } else { 2 };
        let p = &r["requested"]["properties"];
        let op = AlphaClipFinite {
            clip_low: true,
            low_limit: p["low-limit"].as_f64().unwrap() as f32,
            clip_high: true,
            high_limit: p["high-limit"].as_f64().unwrap() as f32,
        };
        for (a, b) in input.chunks_exact(channels * 8).zip(expected.chunks_exact(channels * 8)) {
            let source = f64::from_le_bytes(a[(channels - 1) * 8..channels * 8].try_into().unwrap());
            let reference = f64::from_le_bytes(b[(channels - 1) * 8..channels * 8].try_into().unwrap());
            let mut output = [0.; 4];
            op.try_eval_pixel(&[0.25, 0.5, 0.75, source], &mut output).unwrap();
            assert_eq!(output[3].to_bits(), reference.to_bits());
        }
        count += 1;
    }
    assert_eq!(count, 12);
}
