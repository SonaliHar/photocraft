use photocraft_cms::Builtin;
use photocraft_compose::gimp_absolute::{AbsoluteCandidate, typed_arguments};
use photocraft_doc::SampleEncoding;
use photocraft_raster::{Interrupt, RasterInputError};

#[test]
fn explicit_profile_signed_linear_and_disabled_identity_are_checked() {
    let op = AbsoluteCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Linear).unwrap();
    let pixel = [-1.5, -0.0, 0.5, -0.0];
    assert_eq!(op.apply_with(&pixel, false, &Interrupt::NONE, &|_| Ok(())).unwrap().map(f64::to_bits), pixel.map(f64::to_bits));
    let result = op.apply_with(&pixel, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
    for (actual, expected) in result.iter().zip([1.5, 0.0, 0.5, -0.0]) {
        assert!((actual - expected).abs() <= 1e-6);
    }
    assert_eq!(result[3].to_bits(), (-0.0_f64).to_bits());
    assert!(typed_arguments(&[]).is_ok());
    assert!(AbsoluteCandidate::new(Builtin::DisplayP3.profile(), SampleEncoding::Linear).is_err());
    assert!(op.apply_with(&pixel, true, &Interrupt::cancel_only(&|| true), &|_| Ok(())).is_err());
    assert!(op.apply_with(&pixel, true, &Interrupt::NONE, &|_| Err(RasterInputError::LimitExceeded)).is_err());
    for invalid in [vec![], vec![0.0; 3], vec![0.0, 0.0, 0.0, 1.1], vec![f64::MAX, 0.0, 0.0, 1.0], vec![f64::NAN, 0.0, 0.0, 1.0]] {
        assert!(op.apply_with(&invalid, true, &Interrupt::NONE, &|_| Ok(())).is_err());
    }
}

#[test]
#[ignore = "independent local108 Absolute precision candidates; no import admission"]
fn explicit_absolute_profile_all_native_and_live_originals() {
    compare_originals(false);
}

#[test]
#[ignore = "independent36 Linear Absolute originals; nonlinear/perceptual conversion remains separately diagnostic"]
fn explicit_absolute_linear_profile_native_and_live_originals() {
    compare_originals(true);
}

fn compare_originals(linear_only: bool) {
    use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
    use photocraft_raster::Interrupt;
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-absolute-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
    let entry = manifest["artifacts"].as_array().unwrap().iter().find(|r| r["path"] == "observations.json").unwrap();
    let observations = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&observations), entry["sha256"]);
    let records: serde_json::Value = serde_json::from_slice(&observations).unwrap();
    let read = |d: &serde_json::Value| {
        let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert!(bytes.len() <= 4096);
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        assert_eq!(hash(&bytes), d["sha256"]);
        if d["encoding"] == "zlib" {
            use std::io::Read;
            let length = d["decoded_bytes"].as_u64().unwrap();
            assert!(length <= 2048);
            let mut raw = Vec::new();
            flate2::read::ZlibDecoder::new(bytes.as_slice()).take(length + 1).read_to_end(&mut raw).unwrap();
            assert_eq!(raw.len() as u64, length);
            assert_eq!(hash(&raw), d["decoded_sha256"]);
            raw
        } else {
            bytes
        }
    };
    let decode = |bytes: &[u8], kind: &str| -> Vec<f64> {
        match kind {
            "u8" => bytes.iter().map(|v| f64::from(*v) / 255.).collect(),
            "u16" => bytes.as_chunks::<2>().0.iter().map(|v| f64::from(u16::from_le_bytes(*v)) / 65535.).collect(),
            "u32" => bytes.as_chunks::<4>().0.iter().map(|v| f64::from(u32::from_le_bytes(*v)) / 4294967295.).collect(),
            "half" => bytes.as_chunks::<2>().0.iter().map(|v| f64::from(f32::from_bits(widen_f16_to_f32_bits(u16::from_le_bytes(*v))))).collect(),
            "float" => bytes.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect(),
            "double" => bytes.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect(),
            _ => panic!("own native type"),
        }
    };
    let mut working_max = 0f64;
    let mut native_max = 0f64;
    let mut projection_max = 0f64;
    let mut alpha_max = 0f64;
    let mut worst = serde_json::Value::Null;
    let rows = records["records"].as_array().unwrap();
    assert_eq!(rows.len(), 108);
    let mut count = 0;
    for row in rows {
        if linear_only && !row["precision"].as_str().unwrap().ends_with("_LINEAR") {
            continue;
        }
        if linear_only && row["precision"].as_str().unwrap().ends_with("_NON_LINEAR") {
            continue;
        }
        count += 1;
        let source = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&source), row["sha256"]);
        let gray = row["model"] == "GRAY";
        let channels = if gray { 1 } else { 3 };
        let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        let precision = row["precision"].as_str().unwrap();
        let encoding = if precision.ends_with("NON_LINEAR") {
            SampleEncoding::NonlinearProfile
        } else if precision.ends_with("PERCEPTUAL") {
            SampleEncoding::Perceptual
        } else {
            SampleEncoding::Linear
        };
        assert!(row["reopened"]["layers"][0]["filters"][0]["config"].as_object().unwrap().is_empty());
        let op = AbsoluteCandidate::new(profile, encoding).unwrap();
        let enabled = row["reopened"]["layers"][0]["filters"][0]["visible"].as_bool().unwrap();
        let layer = &row["reopened"]["layers"][0];
        let kind = layer["native_format"].as_str().unwrap().rsplit(' ').next().unwrap();
        let input = decode(&read(&layer["native"]), kind);
        let reference = decode(&read(&row["baked"]["layers"][0]["native"]), kind);
        // The independently published operation pads are RGBA Float; the
        // separately captured Double conversion is not this operative boundary.
        let working = decode(&read(&layer["working"]["RGBA float"]), "float");
        let projection = decode(&read(&row["reopened"]["projection"]), "double");
        let quantize = |v: f64| {
            if kind == "half" {
                return f64::from(f32::from_bits(widen_f16_to_f32_bits(try_round_f32_to_f16_bits(v as f32).unwrap())));
            }
            let sample = match kind {
                "u8" => SampleType::U8,
                "u16" => SampleType::U16,
                "u32" => SampleType::U32,
                "float" => SampleType::F32,
                "double" => SampleType::F64,
                _ => panic!("own kind"),
            };
            let mut b = [0; 8];
            try_write_sample_f64(&mut b, sample, 0, v).unwrap();
            try_read_sample_f64(&b, sample, 0).unwrap()
        };
        for (index, (pixel, expected)) in input.chunks_exact(channels + 1).zip(reference.chunks_exact(channels + 1)).enumerate() {
            let actual_working = op.working_input(pixel).unwrap();
            for (a, b) in actual_working.iter().zip(&working[index * 4..index * 4 + 4]) {
                working_max = working_max.max((f64::from(*a) - b).abs());
            }
            let result = op.apply_with(pixel, enabled, &Interrupt::NONE, &|_| Ok(())).unwrap();
            if !enabled {
                let original = if gray { [pixel[0], pixel[0], pixel[0], pixel[1]] } else { pixel.try_into().unwrap() };
                assert_eq!(result.map(f64::to_bits), original.map(f64::to_bits));
            }
            let stored = result.map(quantize);
            let native = if gray { vec![stored[0], stored[3]] } else { stored.to_vec() };
            for (a, b) in native.iter().zip(expected) {
                native_max = native_max.max((a - b).abs());
            }
            alpha_max = alpha_max.max((stored[3] - expected[channels]).abs());
            let live = result.map(quantize);
            let mut display = [0.; 3];
            photocraft_cms::xcf_color::convert_encoding_f64(
                profile,
                photocraft_compose::gimp_composite::encoding(encoding),
                photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                &live[..channels],
                &mut display[..channels],
            )
            .unwrap();
            if gray {
                display = [display[0]; 3];
            }
            for (a, b) in [display[0], display[1], display[2], live[3]].iter().zip(&projection[index * 4..index * 4 + 4]) {
                if (a - b).abs() > projection_max {
                    projection_max = (a - b).abs();
                    worst = serde_json::json!({"id":row["id"],"pixel":index,"actual":a,"expected":b});
                }
            }
        }
    }
    println!(
        "{count} explicit-profile workingmax={working_max} nativequantizedmax={native_max} projectionmax={projection_max} alphamax={alpha_max}; worst={worst}"
    );
    assert_eq!(count, if linear_only { 36 } else { 108 });
    assert!(working_max <= 1e-6);
    assert!(native_max <= 2. / 255.);
    assert!(projection_max <= 2. / 255.);
    assert!(alpha_max <= 1. / 65535.);
}
