use photocraft_cms::Builtin;
use photocraft_compose::gimp_channel_mixer::ChannelMixerCandidate;
use photocraft_doc::SampleEncoding;

#[test]
fn current_double_authority_types_profiles_and_atomic_refusals() {
    use photocraft_compose::gimp_channel_mixer::typed_arguments;
    use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
    use photocraft_raster::{Interrupt, RasterInputError};
    let names = ["rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain"];
    let mut args: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| FilterArgument {
            name: (*name).into(),
            value: FilterValue::Number(ExactNumber::Float64Bits(if i % 4 == 0 { 1.0_f64.to_bits() } else { 0.0_f64.to_bits() })),
        })
        .collect();
    args.push(FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(false) });
    args[0].value = FilterValue::Number(ExactNumber::Float64Bits(1.23456789_f64.to_bits()));
    let operation = typed_arguments(&args).unwrap();
    assert_eq!(operation.gains[0][0].to_bits(), 1.23456789_f64.to_bits());
    args[0].value = FilterValue::Number(ExactNumber::Float32Bits((1.23456789_f64 as f32).to_bits()));
    assert_ne!(typed_arguments(&args).unwrap().gains[0][0].to_bits(), operation.gains[0][0].to_bits());
    for value in [
        FilterValue::Bool(false),
        FilterValue::Number(ExactNumber::Float64Bits(f64::NAN.to_bits())),
        FilterValue::Number(ExactNumber::Float64Bits(2.01_f64.to_bits())),
    ] {
        args[0].value = value;
        assert!(typed_arguments(&args).is_err());
    }
    assert!(typed_arguments(&args[..9]).is_err());
    args[0] = args[1].clone();
    assert!(typed_arguments(&args).is_err());
    let candidate = ChannelMixerCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Linear, operation).unwrap();
    assert!(ChannelMixerCandidate::new(Builtin::DisplayP3.profile(), SampleEncoding::Linear, operation).is_err());
    let input = [-0.0, 1.25, -0.25, -0.0];
    assert_eq!(candidate.apply_with(&input, false, &Interrupt::NONE, &|_| Ok(())).unwrap().map(f64::to_bits), input.map(f64::to_bits));
    assert!(candidate.apply_with(&input, true, &Interrupt::cancel_only(&|| true), &|_| Ok(())).is_err());
    assert!(candidate.apply_with(&input, true, &Interrupt::NONE, &|_| Err(RasterInputError::LimitExceeded)).is_err());
    for bad in [vec![], vec![0.0; 3], vec![0.0, 0.0, 0.0, 1.1], vec![f64::MAX, 0.0, 0.0, 1.0]] {
        assert!(candidate.apply_with(&bad, true, &Interrupt::NONE, &|_| Ok(())).is_err());
    }
}

fn control_operation(config: &serde_json::Value) -> photocraft_algo::gimp::point_channel_mixer::ChannelMixer {
    let gain = |name: &str| config[name].as_f64().unwrap();
    photocraft_algo::gimp::point_channel_mixer::ChannelMixer {
        gains: [
            [gain("rr-gain"), gain("rg-gain"), gain("rb-gain")],
            [gain("gr-gain"), gain("gg-gain"), gain("gb-gain")],
            [gain("br-gain"), gain("bg-gain"), gain("bb-gain")],
        ],
        preserve_luminosity: config["preserve-luminosity"].as_bool().unwrap(),
    }
}

fn control_blob(root: &std::path::Path, descriptor: &serde_json::Value) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert_eq!(bytes.len() as u64, descriptor["bytes"].as_u64().unwrap());
    assert_eq!(Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(), descriptor["sha256"]);
    bytes
}

#[test]
#[ignore = "independent unpublished current U32 and Half to Float controls"]
fn current_float_promotion_uses_current_storage_not_original_precision() {
    use photocraft_color::{SampleType, try_round_f32_to_f16_bits, widen_f16_to_f32_bits};
    use photocraft_raster::Interrupt;
    let base = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut maximum = 0.0_f64;
    let mut half_difference = 0.0_f64;
    let mut count = 0;
    for (folder, expected_count) in [("channel-mixer-current-storage-evidence", 2), ("channel-mixer-half-promotion-evidence", 6)] {
        let root = base.join(folder);
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let rows = observations["records"].as_array().unwrap();
        assert_eq!(rows.len(), expected_count);
        for row in rows {
            let original = row["source_precision"].as_str().unwrap();
            assert!(original.starts_with("U32_") || original.starts_with("HALF_"));
            let precision = row["precision"].as_str().unwrap();
            assert!(precision.starts_with("FLOAT_"));
            let gray = row["model"] == "GRAY";
            let channels = if gray { 1 } else { 3 };
            let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
            let encoding = if precision.ends_with("NON_LINEAR") {
                SampleEncoding::NonlinearProfile
            } else if precision.ends_with("PERCEPTUAL") {
                SampleEncoding::Perceptual
            } else {
                SampleEncoding::Linear
            };
            let candidate = ChannelMixerCandidate::new(profile, encoding, control_operation(&row["current"]["layers"][0]["filters"][0]["config"])).unwrap();
            let input: Vec<f64> = control_blob(&root, &row["current"]["layers"][0]["native"])
                .as_chunks::<4>().0.iter()
                .map(|v| f64::from(f32::from_le_bytes(*v)))
                .collect();
            let expected: Vec<f64> =
                control_blob(&root, &row["current"]["projection"]).as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect();
            for (i, pixel) in input.chunks_exact(channels + 1).enumerate() {
                let result = candidate.apply_with_current_storage(pixel, true, SampleType::F32, &Interrupt::NONE, &|_| Ok(())).unwrap();
                let stored = result.map(|v| f64::from(v as f32));
                let half = stored.map(|v| f64::from(f32::from_bits(widen_f16_to_f32_bits(try_round_f32_to_f16_bits(v as f32).unwrap()))));
                if original.starts_with("HALF_") {
                    for (a, b) in stored.iter().zip(half) {
                        half_difference = half_difference.max((a - b).abs());
                    }
                }
                if original.starts_with("U32_") && i == 9 {
                    let stale = candidate.apply_with_current_storage(pixel, true, SampleType::U32, &Interrupt::NONE, &|_| Ok(())).unwrap();
                    assert!((stale[0] - result[0]).abs() > 0.02);
                }
                let mut display = [0.0; 3];
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
                for (a, b) in [display[0], display[1], display[2], stored[3]].iter().zip(&expected[i * 4..i * 4 + 4]) {
                    maximum = maximum.max((a - b).abs());
                }
            }
            count += 1;
        }
    }
    println!("{count} current Float controls projectionmax={maximum}, staleHalfDifference={half_difference}");
    assert_eq!(count, 8);
    assert!(maximum <= 2.0 / 255.0);
    assert!(half_difference > 1e-5);
}

#[test]
#[ignore = "public pre-save Double authority/HDR controls; no source admission"]
fn public_current_double_gains_before_serialized_narrowing() {
    use photocraft_raster::Interrupt;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("channel-mixer-domain-response-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut maximum = 0.0_f64;
    let mut count = 0;
    for row in observations["records"].as_array().unwrap() {
        let gray = row["model"] == "GRAY";
        let channels = if gray { 1 } else { 3 };
        let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        let encoding = if row["precision"].as_str().unwrap().ends_with("NON_LINEAR") { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
        let layer = &row["before"]["layers"][0];
        let candidate = ChannelMixerCandidate::new(profile, encoding, control_operation(&layer["filters"][0]["config"])).unwrap();
        let kind = layer["native_format"].as_str().unwrap().rsplit(' ').next().unwrap();
        let decode = |d: &serde_json::Value| -> Vec<f64> {
            let bytes = control_blob(&root, d);
            if kind == "float" {
                bytes.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect()
            } else {
                bytes.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect()
            }
        };
        let input = decode(&layer["native"]);
        let baked = decode(&row["baked"]["layers"][0]["native"]);
        for (pixel, expected) in input.chunks_exact(channels + 1).zip(baked.chunks_exact(channels + 1)) {
            let result = candidate.apply_with(pixel, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
            let stored = result.map(|v| if kind == "float" { f64::from(v as f32) } else { v });
            let native = if gray { vec![stored[0], stored[3]] } else { stored.to_vec() };
            for (a, b) in native.iter().zip(expected) {
                maximum = maximum.max((a - b).abs());
            }
        }
        count += 1;
    }
    println!("{count} public pre-save Double-gain native maximum={maximum}");
    assert_eq!(count, 60);
    assert!(maximum <= 2.0 / 255.0);
}

#[test]
#[ignore = "independent local144 Channel Mixer precision candidates; no import admission"]
fn explicit_channel_mixer_profile_all_native_and_live_originals() {
    use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
    use photocraft_raster::Interrupt;
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("channel-mixer-precision-evidence");
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
    assert_eq!(rows.len(), 144);
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
        let config = &row["current"]["layers"][0]["filters"][0]["config"];
        let gain = |name: &str| config[name].as_f64().unwrap();
        let operation = photocraft_algo::gimp::point_channel_mixer::ChannelMixer {
            gains: [
                [gain("rr-gain"), gain("rg-gain"), gain("rb-gain")],
                [gain("gr-gain"), gain("gg-gain"), gain("gb-gain")],
                [gain("br-gain"), gain("bg-gain"), gain("bb-gain")],
            ],
            preserve_luminosity: config["preserve-luminosity"].as_bool().unwrap(),
        };
        let op = ChannelMixerCandidate::new(profile, encoding, operation).unwrap();
        let enabled = row["current"]["layers"][0]["filters"][0]["visible"].as_bool().unwrap();
        let layer = &row["current"]["layers"][0];
        let kind = layer["native_format"].as_str().unwrap().rsplit(' ').next().unwrap();
        let input = decode(&read(&layer["native"]), kind);
        let reference = decode(&read(&row["baked"]["layers"][0]["native"]), kind);
        let working = decode(&read(&layer["working"]["RGBA double"]), "double");
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
            let current_sample = match kind {
                "u8" => SampleType::U8,
                "u16" => SampleType::U16,
                "u32" => SampleType::U32,
                "float" | "half" => SampleType::F32,
                "double" => SampleType::F64,
                _ => panic!("own current sample"),
            };
            let live = op.apply_with_current_storage(pixel, enabled, current_sample, &Interrupt::NONE, &|_| Ok(())).unwrap().map(quantize);
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
        "144 explicit-profile workingmax={working_max} nativequantizedmax={native_max} projectionmax={projection_max} alphamax={alpha_max}; worst={worst}"
    );
    assert!(working_max <= 1e-6);
    assert!(native_max <= 2. / 255.);
    assert!(projection_max <= 2. / 255.);
    assert!(alpha_max <= 1. / 65535.);
}
