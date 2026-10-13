use photocraft_algo::gimp::point_mono_mixer::MonoMixer;
use photocraft_cms::Builtin;
use photocraft_compose::gimp_mono_mixer::MonoMixerCandidate;
use photocraft_doc::SampleEncoding;

#[test]
fn independently_proved_linear_signed_weights_prepare() {
    MonoMixerCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Linear, MonoMixer { red: -0.5, green: 1.5, blue: 0.25, preserve_luminosity: true })
        .unwrap();
}

#[test]
fn typed_wire_profiles_disabled_and_hostile_inputs() {
    use photocraft_compose::gimp_mono_mixer::typed_arguments;
    use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
    use photocraft_raster::Interrupt;
    let args = vec![
        FilterArgument { name: "red".into(), value: FilterValue::Number(ExactNumber::Float64Bits((-0.5f64).to_bits())) },
        FilterArgument { name: "green".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1.5f32.to_bits())) },
        FilterArgument { name: "blue".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits())) },
        FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) },
    ];
    let operation = typed_arguments(&args).unwrap();
    assert_eq!(operation.red, -0.5);
    for profile in [Builtin::DisplayP3.profile(), Builtin::LabD50.profile()] {
        assert!(MonoMixerCandidate::new(profile, SampleEncoding::Linear, operation).is_err());
    }
    for bad in [
        FilterValue::Bool(false),
        FilterValue::Number(ExactNumber::Float64Bits(f64::NAN.to_bits())),
        FilterValue::Number(ExactNumber::Float64Bits(5.01f64.to_bits())),
    ] {
        let mut altered = args.clone();
        altered[0].value = bad;
        assert!(typed_arguments(&altered).is_err());
    }
    let mut duplicate = args.clone();
    duplicate[0] = args[1].clone();
    assert!(typed_arguments(&duplicate).is_err());
    let mut unknown = args.clone();
    unknown[0].name = "unknown".into();
    assert!(typed_arguments(&unknown).is_err());
    assert!(typed_arguments(&args[..3]).is_err());
    let op = MonoMixerCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Linear, operation).unwrap();
    let flag = std::sync::atomic::AtomicBool::new(true);
    let cancelled = || flag.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = photocraft_raster::Interrupt::cancel_only(&cancelled);
    assert!(op.apply_with(&[0.1, 0.2, 0.3, 1.], true, &ctl, &|_| Ok(())).is_err());
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(
        op.apply_with(&[0.1, 0.2, 0.3, 1.], true, &ctl, &|_| {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        })
        .is_err()
    );
    let input = [-0.0, 1.25, -0.5, 0.123456789012345];
    let result = op.apply_with(&input, false, &Interrupt::NONE, &|_| Ok(())).unwrap();
    assert_eq!(result.map(f64::to_bits), input.map(f64::to_bits));
    assert!(op.apply_with(&input, true, &Interrupt::NONE, &|_| Err(photocraft_raster::RasterInputError::UnsupportedConversion)).is_err());
    for input in [vec![], vec![0.; 3], vec![0., 0., 0., -0.1], vec![f64::NAN, 0., 0., 1.], vec![f64::MAX, 0., 0., 1.]] {
        assert!(op.apply_with(&input, true, &Interrupt::NONE, &|_| Ok(())).is_err());
    }
}

#[test]
#[ignore = "independent local144 Mono Mixer signed precision candidates; no import admission"]
fn explicit_current_profile_all_native_and_live_originals() {
    use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
    use photocraft_raster::Interrupt;
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-signed-precision-evidence");
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
        let weights = row["weights"].as_array().unwrap();
        let operation = MonoMixer {
            red: weights[0].as_f64().unwrap(),
            green: weights[1].as_f64().unwrap(),
            blue: weights[2].as_f64().unwrap(),
            preserve_luminosity: row["preserve"].as_bool().unwrap(),
        };
        let op = MonoMixerCandidate::new(profile, encoding, operation).unwrap();
        let enabled = row["enabled"].as_bool().unwrap();
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
        "144 explicit-profile workingmax={working_max} nativequantizedmax={native_max} projectionmax={projection_max} alphamax={alpha_max}; worst={worst}"
    );
    assert!(working_max <= 1e-6);
    assert!(native_max <= 2. / 255.);
    assert!(projection_max <= 2. / 255.);
    assert!(alpha_max <= 1. / 65535.);
}
