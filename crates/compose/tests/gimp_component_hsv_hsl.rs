use photocraft_cms::Builtin;
use photocraft_compose::gimp_component_hsv_hsl::HsvHslCandidate;
use photocraft_doc::SampleEncoding;

#[test]
fn native_primary_gray_model_conversion_and_disabled_exact_identity() {
    for (pixel, component, expected) in [
        ([1., 0., 0., 0.3], 3, 0.),
        ([0., 1., 0., 0.3], 3, 1. / 3.),
        ([0., 0., 1., 0.3], 3, 2. / 3.),
        ([0.5, 0.5, 0.5, 0.3], 4, 0.),
        ([0.5, 0.5, 0.5, 0.3], 5, 0.5),
        ([1., 0., 0., 0.3], 6, 1.),
        ([1., 0., 0., 0.3], 7, 0.5),
    ] {
        let candidate = HsvHslCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, component, false, false).unwrap();
        let out = candidate.apply(&pixel, true).unwrap();
        assert!((out[0] - expected).abs() < 4.0e-8);
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        assert_eq!(out[3], 1.);
        assert_eq!(candidate.apply(&pixel, false).unwrap(), pixel);
    }
    let candidate = HsvHslCandidate::new(Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, 3, false, false).unwrap();
    assert_eq!(
        candidate.apply(&[0.12345678901234567, 0.4567890123456789], false).unwrap(),
        [0.12345678901234567, 0.12345678901234567, 0.12345678901234567, 0.4567890123456789]
    );
    assert_eq!(candidate.apply(&[0.75, 0.5], true).unwrap(), [0., 0., 0., 1.]);
}

#[test]
fn hostile_profile_argument_and_pixel_inputs_refuse_without_guessing_conversion() {
    for profile in [Builtin::DisplayP3.profile(), Builtin::LabD50.profile()] {
        assert!(HsvHslCandidate::new(profile, SampleEncoding::NonlinearProfile, 3, false, false).is_err());
    }
    for component in [0, 2, 8, 19, 20, 255] {
        assert!(HsvHslCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, component, false, false).is_err());
    }
}

#[test]
#[ignore = "independent local900 HSV/HSL native and display states; conversion candidate only"]
fn explicit_profile_native_and_projection_all_originals() {
    use photocraft_color::{SampleType, try_read_sample_f64, try_write_sample_f64};
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let packet = full.join("point-component-extract-hsv-hsl-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let manifest = std::fs::read(packet.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "32c9948cc94bdba33979deff8f116da9f1ce5f52e58bf4b32649f173c3d3dcf1");
    let census: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let bytes = std::fs::read(packet.join("states/observations.json")).unwrap();
    let a = census["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "states/observations.json").unwrap();
    assert_eq!(hash(&bytes), a["sha256"]);
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let states = packet.join("states");
    let read = |d: &serde_json::Value| {
        let bytes = std::fs::read(states.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), d["sha256"]);
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        let size = d["decoded_bytes"].as_u64().unwrap();
        assert!(size <= 1536);
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(bytes.as_slice()).take(size + 1).read_to_end(&mut raw).unwrap();
        assert_eq!(raw.len() as u64, size);
        assert_eq!(hash(&raw), d["decoded_sha256"]);
        raw
    };
    let decode = |bytes: &[u8], kind: &str| -> Vec<f64> {
        match kind {
            "u8" => bytes.iter().map(|b| f64::from(*b) / 255.).collect(),
            "u16" => bytes.as_chunks::<2>().0.iter().map(|b| f64::from(u16::from_le_bytes(*b)) / 65535.).collect(),
            "u32" => bytes.as_chunks::<4>().0.iter().map(|b| f64::from(u32::from_le_bytes(*b)) / 4294967295.).collect(),
            "half" => bytes
                .as_chunks::<2>().0.iter()
                .map(|b| f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(*b)))))
                .collect(),
            "float" => bytes.as_chunks::<4>().0.iter().map(|b| f64::from(f32::from_le_bytes(*b))).collect(),
            "double" => bytes.as_chunks::<8>().0.iter().map(|b| f64::from_le_bytes(*b)).collect(),
            _ => panic!("unknown frozen sample kind"),
        }
    };
    let mut maximum_native = 0.0f64;
    let mut maximum_projection = 0.0f64;
    let mut maximum_alpha = 0.0f64;
    let mut enabled = 0;
    let mut failures = Vec::new();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 900);
    for row in rows {
        let source = std::fs::read(states.join(row["source"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&source), row["sha256"]);
        let gray = row["model"] == "GRAY";
        let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        let channels = if gray { 2 } else { 4 };
        let precision = row["precision"].as_str().unwrap();
        let encoding = if precision.ends_with("NON_LINEAR") {
            SampleEncoding::NonlinearProfile
        } else if precision.ends_with("PERCEPTUAL") {
            SampleEncoding::Perceptual
        } else {
            SampleEncoding::Linear
        };
        let component = match row["component"].as_str().unwrap() {
            "hue" => 3,
            "hsv-s" => 4,
            "hsv-v" => 5,
            "hsl-s" => 6,
            "hsl-l" => 7,
            _ => panic!("unknown frozen selector"),
        };
        let candidate = HsvHslCandidate::new(profile, encoding, component, row["invert"].as_bool().unwrap(), row["linear"].as_bool().unwrap()).unwrap();
        let input_layer = &row["before"]["layers"][0];
        let kind = input_layer["native_format"].as_str().unwrap().rsplit(' ').next().unwrap();
        let native = read(&input_layer["native"]);
        assert_eq!(read(&row["current"]["layers"][0]["native"]), native, "{} source changed", row["id"]);
        let input = decode(&native, kind);
        let expected = decode(&read(&row["baked"]["layers"][0]["native"]), kind);
        let projection = decode(&read(&row["current"]["projection"]), "double");
        assert_eq!(input.len(), 48 * channels);
        assert_eq!(input.len(), expected.len());
        assert_eq!(projection.len(), 48 * 4);
        let active = row["enabled"].as_bool().unwrap();
        enabled += usize::from(active);
        let quantize = |value: f64| {
            if kind == "half" {
                return f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(photocraft_color::try_round_f32_to_f16_bits(value as f32).unwrap())));
            }
            let sample = match kind {
                "u8" => SampleType::U8,
                "u16" => SampleType::U16,
                "u32" => SampleType::U32,
                "float" => SampleType::F32,
                "double" => SampleType::F64,
                _ => panic!("sample kind"),
            };
            let mut buffer = [0; 8];
            try_write_sample_f64(&mut buffer, sample, 0, value).unwrap();
            try_read_sample_f64(&buffer, sample, 0).unwrap()
        };
        for (pixel, ((input, expected), projection)) in
            input.chunks_exact(channels).zip(expected.chunks_exact(channels)).zip(projection.as_chunks::<4>().0.iter()).enumerate()
        {
            let raw = match candidate.apply(input, active) {
                Ok(raw) => raw,
                Err(error) => {
                    failures.push(format!("{} pixel{pixel} conversion {error:?}, native{input:?}", row["id"]));
                    continue;
                }
            };
            if !active {
                let unchanged = if gray { [input[0], input[0], input[0], input[1]] } else { input.try_into().unwrap() };
                assert_eq!(raw, unchanged);
            }
            let output = if active { raw.map(quantize) } else { raw };
            let expected_rgba = if gray { [expected[0], expected[0], expected[0], expected[1]] } else { expected.try_into().unwrap() };
            for (a, b) in output.into_iter().zip(expected_rgba) {
                let delta = (a - b).abs();
                maximum_native = maximum_native.max(delta);
                if delta > 2. / 255. {
                    failures.push(format!("{} pixel{pixel} native {output:?} vs {expected_rgba:?} delta{delta}", row["id"]));
                }
            }
            let mut display = [0.; 3];
            photocraft_cms::xcf_color::convert_encoding_f64(
                profile,
                match encoding {
                    SampleEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
                    SampleEncoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    SampleEncoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
                },
                photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                &output[..channels - 1],
                &mut display[..channels - 1],
            )
            .unwrap();
            if gray {
                display = [display[0]; 3];
            }
            let alpha = (output[3] - projection[3]).abs();
            maximum_alpha = maximum_alpha.max(alpha);
            if alpha > 1. / 65535. {
                failures.push(format!("{} pixel{pixel} alpha{alpha}", row["id"]));
            }
            for (a, b) in display.into_iter().zip(projection) {
                let delta = (a * output[3] - b * projection[3]).abs();
                maximum_projection = maximum_projection.max(delta);
                if delta > 2. / 255. {
                    failures.push(format!("{} pixel{pixel} premultiplied projection{delta}", row["id"]));
                }
            }
        }
    }
    assert_eq!(enabled, 720);
    println!(
        "900 exact native/source states,720active; private current-profile HSV/HSL native {}/255,projection {}/255,alpha{maximum_alpha}; no source admission",
        maximum_native * 255.,
        maximum_projection * 255.
    );
    for failure in &failures {
        println!("UNQUALIFIED {failure}");
    }
    assert!(failures.is_empty(), "{} native-conversion/reference failures; unchanged bounds", failures.len());
}
