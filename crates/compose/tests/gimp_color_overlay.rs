use photocraft_cms::Builtin;
use photocraft_compose::gimp_color_overlay::{ColorOverlayCandidate, literal_arguments};
use photocraft_doc::SampleEncoding;

#[test]
fn literal_schema_rejects_missing_duplicate_unknown_and_nonliteral_forms() {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
    let color = FilterArgument {
        name: "value".into(),
        value: FilterValue::Color {
            components: [0.25f32, 0.5, 0.75, 0.25].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
            encoding: SampleEncoding::NonlinearProfile,
            profile: None,
        },
    };
    let srgb = FilterArgument { name: "srgb".into(), value: FilterValue::Bool(false) };
    assert_eq!(literal_arguments(&[color.clone(), srgb.clone()]).unwrap(), ([0.25, 0.5, 0.75, 0.25], false));
    for arguments in
        [vec![], vec![color.clone()], vec![color.clone(), color.clone()], vec![srgb.clone(), srgb.clone()], vec![color.clone(), srgb.clone(), srgb.clone()]]
    {
        assert!(literal_arguments(&arguments).is_err());
    }
    for value in [
        FilterValue::Bool(false),
        FilterValue::Color { components: vec![ExactNumber::Float64Bits(0.25f64.to_bits()); 4], encoding: SampleEncoding::NonlinearProfile, profile: None },
        FilterValue::Color { components: vec![ExactNumber::Float32Bits(0.25f32.to_bits()); 4], encoding: SampleEncoding::Linear, profile: None },
    ] {
        assert!(literal_arguments(&[FilterArgument { name: "value".into(), value }, srgb.clone()]).is_err());
    }
    assert!(literal_arguments(&[color, FilterArgument { name: "unknown".into(), value: FilterValue::Bool(false) }]).is_err());
}

#[test]
fn candidate_rejects_unproved_profiles_and_hostile_operands() {
    assert!(ColorOverlayCandidate::new(Builtin::DisplayP3.profile(), SampleEncoding::Linear, [0.5; 4], false).is_err());
    assert!(ColorOverlayCandidate::new(Builtin::LabD50.profile(), SampleEncoding::Linear, [0.5; 4], false).is_err());
    for bad in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(ColorOverlayCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Linear, [bad; 4], false).is_err());
    }
    let op = ColorOverlayCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Linear, [0.5; 4], false).unwrap();
    for input in [vec![], vec![0.; 3], vec![0., 0., 0., -0.1], vec![0., 0., 0., 1.1], vec![f64::NAN, 0., 0., 1.], vec![f64::MAX, 0., 0., 1.]] {
        assert!(op.apply(&input, true).is_err());
    }
    let pixel = [0.12345678901234567, 0.2, 0.3, 0.4567890123456789];
    assert_eq!(op.apply(&pixel, false).unwrap(), pixel);
    let gray = ColorOverlayCandidate::new(Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, [0.5; 4], false).unwrap();
    assert_eq!(gray.working_input(&[1., 0.5]).unwrap(), [1., 1., 1., 0.5]);
}

#[test]
#[ignore = "independent private360 native controls; no import admission"]
fn explicit_profile_native_conversion_all_originals() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/point-color-overlay-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|n| format!("{n:02x}")).collect::<String>();
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "828c1da757d0706cd6eb502699c74c0c2856745d3603c6ab6d1dd56d70df9831");
    let census: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let entry = census["artifacts"].as_array().unwrap().iter().find(|entry| entry["path"] == "observations.json").unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), entry["sha256"].as_str().unwrap());
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let read = |d: &serde_json::Value| {
        let encoded = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&encoded), d["sha256"].as_str().unwrap());
        let size = d["decoded_bytes"].as_u64().unwrap();
        assert!(size <= 1536);
        let mut bytes = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(size + 1).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len() as u64, size);
        assert_eq!(hash(&bytes), d["decoded_sha256"].as_str().unwrap());
        bytes
    };
    let decode = |bytes: &[u8], kind: &str| -> Vec<f64> {
        match kind {
            "u8" => bytes.iter().map(|n| f64::from(*n) / 255.).collect(),
            "u16" => bytes.as_chunks::<2>().0.iter().map(|n| f64::from(u16::from_le_bytes(*n)) / 65535.).collect(),
            "u32" => bytes.as_chunks::<4>().0.iter().map(|n| f64::from(u32::from_le_bytes(*n)) / 4294967295.).collect(),
            "half" => bytes
                .as_chunks::<2>().0.iter()
                .map(|n| f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(*n)))))
                .collect(),
            "float" => bytes.as_chunks::<4>().0.iter().map(|n| f64::from(f32::from_le_bytes(*n))).collect(),
            "double" => bytes.as_chunks::<8>().0.iter().map(|n| f64::from_le_bytes(*n)).collect(),
            _ => panic!("unknown own descriptor"),
        }
    };
    let mut working_max = 0f64;
    let mut native_max = 0f64;
    let mut quantized_max = 0f64;
    let mut display_max = 0f64;
    let mut display_worst = serde_json::Value::Null;
    let mut working_worst = serde_json::Value::Null;
    let mut native_worst = serde_json::Value::Null;
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 360);
    for row in rows {
        let source = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&source), row["sha256"].as_str().unwrap());
        let gray = row["model"] == "GRAY";
        let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        let precision = row["precision"].as_str().unwrap();
        let encoding = if precision.ends_with("NON_LINEAR") {
            SampleEncoding::NonlinearProfile
        } else if precision.ends_with("PERCEPTUAL") {
            SampleEncoding::Perceptual
        } else {
            SampleEncoding::Linear
        };
        let color = std::array::from_fn(|i| row["requested_color"][i].as_f64().unwrap() as f32);
        let op = ColorOverlayCandidate::new(profile, encoding, color, row["srgb"].as_bool().unwrap()).unwrap();
        let layer = &row["before"]["layers"][0];
        let format = layer["native_format"].as_str().unwrap();
        let kind = format.rsplit(' ').next().unwrap();
        let input = decode(&read(&layer["native"]), kind);
        let baked = decode(&read(&row["baked"]["layers"][0]["native"]), kind);
        let working = decode(&read(&row["working"]["input"]), "float");
        let projection = decode(&read(&row["reopened"]["projection"]), "double");
        let source = photocraft_cms::xcf_color::effective_profile_from_definition(profile, photocraft_compose::gimp_composite::encoding(encoding)).unwrap();
        let display = photocraft_cms::Transform::new(&source, Builtin::Srgb.profile(), photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
        let quantize = |value: f64| {
            use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
            if kind == "half" {
                return f64::from(f32::from_bits(widen_f16_to_f32_bits(try_round_f32_to_f16_bits(value as f32).unwrap())));
            }
            let sample = match kind {
                "u8" => SampleType::U8,
                "u16" => SampleType::U16,
                "u32" => SampleType::U32,
                "float" => SampleType::F32,
                "double" => SampleType::F64,
                _ => panic!("unknown ownkind"),
            };
            let mut bytes = [0; 8];
            try_write_sample_f64(&mut bytes, sample, 0, value).unwrap();
            try_read_sample_f64(&bytes, sample, 0).unwrap()
        };
        let count = if gray { 2 } else { 4 };
        for (index, (input, expected)) in input.chunks_exact(count).zip(baked.chunks_exact(count)).enumerate() {
            let actual_working = op.working_input(input).unwrap();
            for (channel, (a, b)) in actual_working.iter().zip(&working[index * 4..index * 4 + 4]).enumerate() {
                if (a - b).abs() > working_max {
                    working_max = (a - b).abs();
                    working_worst = serde_json::json!({"id":row["id"],"pixel":index,"channel":channel,"input":input,"actual":a,"expected":b});
                }
            }
            let output = op.apply(input, row["enabled"].as_bool().unwrap()).unwrap();
            let quantized = output.map(quantize);
            let mut rgb = [0.; 3];
            // This public projection is straight RGBA, including hidden RGB at
            // zero coverage. It is not an opaque PNG/checkerboard display.
            display.try_eval_f64(&quantized[..if gray { 1 } else { 3 }], &mut rgb).unwrap();
            for (a, b) in [rgb[0], rgb[1], rgb[2], quantized[3]].iter().zip(&projection[index * 4..index * 4 + 4]) {
                if (a - b).abs() > display_max {
                    display_max = (a - b).abs();
                    display_worst = serde_json::json!({"id":row["id"],"pixel":index,"quantized":quantized,"actual":a,"expected":b,"projection":&projection[index*4..index*4+4]});
                }
            }
            let quantized = if gray { [quantized[0], quantized[3], 0., 0.] } else { quantized };
            for (a, b) in quantized.iter().zip(expected) {
                quantized_max = quantized_max.max((a - b).abs());
            }
            let output = if gray { [output[0], output[3], 0., 0.] } else { output };
            for (channel, (a, b)) in output.iter().zip(expected).enumerate() {
                if (a - b).abs() > native_max {
                    native_max = (a - b).abs();
                    native_worst = serde_json::json!({"id":row["id"],"pixel":index,"channel":channel,"input":input,"actual":a,"expected":b});
                }
            }
        }
    }
    println!("360 explicit-profile candidates working max {working_max}, unquantized native max {native_max}");
    println!("quantized native max {quantized_max}; matched-current-profile display max {display_max}");
    println!("display worst {display_worst}");
    println!("working worst {working_worst}; native worst {native_worst}");
    assert!(working_max <= 1e-6, "actual requested working conversion differs {working_max}");
    assert!(native_max <= 2. / 255., "native boundary differs {native_max}");
    assert!(quantized_max <= 2. / 255., "native storage differs {quantized_max}");
    assert!(display_max <= 2. / 255., "actualprojection differs {display_max}");
}
