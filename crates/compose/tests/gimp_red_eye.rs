use photocraft_cms::Builtin;
use photocraft_compose::gimp_red_eye::RedEyeCandidate;
use photocraft_doc::SampleEncoding;

#[test]
fn red_eye_typed_current_boundaries_and_disabled_identity() {
    use photocraft_compose::gimp_red_eye::typed_threshold;
    use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
    use photocraft_raster::Interrupt;
    let args = vec![FilterArgument { name: "threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())) }];
    assert_eq!(typed_threshold(&args).unwrap(), 0.25);
    for value in [
        FilterValue::Bool(false),
        FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
        FilterValue::Number(ExactNumber::Float32Bits(0.8f32.to_bits())),
        FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits())),
    ] {
        assert!(typed_threshold(&[FilterArgument { name: "threshold".into(), value }]).is_err());
    }
    assert!(typed_threshold(&[]).is_err());
    for profile in [Builtin::DisplayP3.profile(), Builtin::LabD50.profile()] {
        assert!(RedEyeCandidate::new(profile, SampleEncoding::Linear, 0.25).is_err());
    }
    let candidate = RedEyeCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 0.25).unwrap();
    let input = [-0., 1.25, -0.25, -0.];
    assert_eq!(candidate.apply_with(&input, false, &Interrupt::NONE, &|_| Ok(())).unwrap().map(f64::to_bits), input.map(f64::to_bits));
    assert!(candidate.apply_with(&input, true, &Interrupt::NONE, &|_| Ok(())).is_err());
    assert!(candidate.apply_with(&[0., 0., 0., 1.], true, &Interrupt::cancel_only(&|| true), &|_| Ok(())).is_err());
    assert!(candidate.apply_with(&[0., 0., 0., 1.], true, &Interrupt::NONE, &|_| Err(photocraft_raster::RasterInputError::UnsupportedConversion)).is_err());
    for input in [vec![], vec![0.; 3], vec![0., 0., 0., -0.1], vec![f64::NAN, 0., 0., 1.], vec![f64::MAX, 0., 0., 1.]] {
        assert!(candidate.apply_with(&input, true, &Interrupt::NONE, &|_| Ok(())).is_err());
    }
}

#[test]
#[ignore = "independent local108 Red Eye precision candidates; no import admission"]
fn explicit_red_eye_profile_all_native_and_live_originals() {
    use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
    use photocraft_raster::Interrupt;
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("red-eye-removal-precision-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
    let entry = manifest["artifacts"].as_array().unwrap().iter().find(|r| r["path"] == "observations.json").unwrap();
    let observations = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&observations), entry["sha256"]);
    let records: serde_json::Value = serde_json::from_slice(&observations).unwrap();
    let read = |d: &serde_json::Value| {
        let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert!(bytes.len() <= 1536);
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        assert_eq!(hash(&bytes), d["sha256"]);
        bytes
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
    for row in rows {
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
        let threshold = row["current"]["layers"][0]["filters"][0]["config"]["threshold"].as_f64().unwrap();
        let op = RedEyeCandidate::new(profile, encoding, threshold).unwrap();
        let enabled = row["current"]["layers"][0]["filters"][0]["visible"].as_bool().unwrap();
        let layer = &row["current"]["layers"][0];
        let kind = layer["native_format"].as_str().unwrap().rsplit(' ').next().unwrap();
        let input = decode(&read(&layer["native"]), kind);
        let reference = decode(&read(&row["baked"]["layers"][0]["native"]), kind);
        let working = decode(&read(&layer["working"]["R'G'B'A double"]), "double");
        let projection = decode(&read(&row["current"]["projection"]), "double");
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
                working_max = working_max.max((a - b).abs());
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
            let mut display = [0.; 3];
            photocraft_cms::xcf_color::convert_encoding_f64(
                profile,
                photocraft_compose::gimp_composite::encoding(encoding),
                photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                &stored[..channels],
                &mut display[..channels],
            )
            .unwrap();
            if gray {
                display = [display[0]; 3];
            }
            for (a, b) in [display[0], display[1], display[2], stored[3]].iter().zip(&projection[index * 4..index * 4 + 4]) {
                if (a - b).abs() > projection_max {
                    projection_max = (a - b).abs();
                    worst = serde_json::json!({"id":row["id"],"pixel":index,"actual":a,"expected":b});
                }
            }
        }
    }
    println!(
        "108 explicit-profile workingmax={working_max} nativequantizedmax={native_max} projectionmax={projection_max} alphamax={alpha_max}; worst={worst}"
    );
    assert!(working_max <= 1e-6);
    assert!(native_max <= 2. / 255.);
    assert!(projection_max <= 2. / 255.);
    assert!(alpha_max <= 1. / 65535.);
}
