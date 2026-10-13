use photocraft_algo::gimp::point_color_rotate::{ColorRotate, GrayMode, HueArc};
use photocraft_cms::Builtin;
use photocraft_compose::gimp_color_rotate::ColorRotateCandidate;
use photocraft_doc::SampleEncoding;
use photocraft_raster::Interrupt;

fn operation(v: &serde_json::Value) -> ColorRotate {
    let f = |name| v[name].as_f64().unwrap() as f32;
    ColorRotate {
        source: HueArc { from_degrees: f("src-from"), to_degrees: f("src-to"), clockwise: v["src-clockwise"].as_bool().unwrap() },
        destination: HueArc { from_degrees: f("dest-from"), to_degrees: f("dest-to"), clockwise: v["dest-clockwise"].as_bool().unwrap() },
        gray_threshold: f("threshold"),
        gray_mode: if v["gray-mode"] == "change-to" { GrayMode::ChangeTo } else { GrayMode::TreatAs },
        gray_hue_degrees: f("hue"),
        gray_saturation: f("saturation"),
    }
}

#[test]
fn explicit_profile_hostile_input_work_and_cancellation() {
    use photocraft_raster::RasterInputError;
    let op = operation(
        &serde_json::json!({"src-from":0,"src-to":90,"src-clockwise":false,"dest-from":180,"dest-to":270,"dest-clockwise":false,"threshold":0,"gray-mode":"change-to","hue":0,"saturation":0}),
    );
    assert!(ColorRotateCandidate::new(Builtin::DisplayP3.profile(), SampleEncoding::Linear, op).is_err());
    assert!(ColorRotateCandidate::new(Builtin::LabD50.profile(), SampleEncoding::Linear, op).is_err());
    let candidate = ColorRotateCandidate::new(Builtin::Srgb.profile(), SampleEncoding::Perceptual, op).unwrap();
    for input in [vec![], vec![0.; 3], vec![f64::NAN, 0., 0., 1.], vec![0., 0., 0., -0.1], vec![0., 0., 0., 1.1], vec![f64::MAX, 0., 0., 1.]] {
        assert!(candidate.apply_with(&input, true, &Interrupt::NONE, &|_| Ok(())).is_err());
    }
    let input = [0.125, 0.3, 0.5, -0.0];
    assert_eq!(candidate.apply_with(&input, false, &Interrupt::NONE, &|_| Ok(())).unwrap().map(f64::to_bits), input.map(f64::to_bits));
    assert!(candidate.apply_with(&input, true, &Interrupt::NONE, &|_| Err(RasterInputError::LimitExceeded)).is_err());
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(candidate.apply_with(&input, false, &cancelled, &|_| panic!("cancel must precede work")).is_err());
    let gray = ColorRotateCandidate::new(Builtin::SGray.profile(), SampleEncoding::Perceptual, op).unwrap();
    assert_eq!(gray.working_input(&[0.5, 0.25]).unwrap(), [0.5, 0.5, 0.5, 0.25]);
}

#[test]
#[ignore = "diagnostic explicit ICC routes; not a qualification assertion"]
fn frozen_preferred_input_conversion_decomposition() {
    use photocraft_cms::{
        xcf_color::{effective_profile_from_definition, Encoding},
        Intent, Profile, Transform,
    };
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let response = root.join("color-rotate-response-evidence");
    let preferred = root.join("color-rotate-preferred-working-probe-evidence");
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|n| format!("{n:02x}")).collect::<String>();
    assert_eq!(hash(&std::fs::read(response.join("batch-manifest.json")).unwrap()), "d64019de7f02c318b56dad0e16bb6fb7ab8e85784e8e794710d114c2c5d15cbb");
    assert_eq!(hash(&std::fs::read(preferred.join("batch-manifest.json")).unwrap()), "cf2cee246b65b37d4bd2b28d008602f5c9a7d43ee3d5bd41e79bd9c614eba9ad");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(response.join("observations.json")).unwrap()).unwrap();
    let working: serde_json::Value = serde_json::from_slice(&std::fs::read(preferred.join("observations.json")).unwrap()).unwrap();
    let read = |root: &std::path::Path, d: &serde_json::Value| {
        let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), d["sha256"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        bytes
    };
    let row = observations["records"].as_array().unwrap().iter().find(|r| r["id"] == "ccw.enabled").unwrap();
    let actual = working["records"].as_array().unwrap().iter().find(|r| r["id"] == "ccw.enabled").unwrap();
    assert_eq!(actual["source_sha256"], row["sha256"]);
    let profile = Profile::parse(&read(&response, &row["current"]["profiles"]["effective"])).unwrap();
    let decode = |b: &[u8]| b.as_chunks::<4>().0.iter().map(|n| f64::from(f32::from_le_bytes(*n))).collect::<Vec<_>>();
    let input = decode(&read(&response, &row["current"]["layers"][0]["native"]));
    let expected = decode(&read(&preferred, &actual["input"]));
    let working_builtin = effective_profile_from_definition(Builtin::Srgb.profile(), Encoding::Perceptual).unwrap();
    let working_source = effective_profile_from_definition(&profile, Encoding::Perceptual).unwrap();
    for (name, source, target) in [
        ("builtin-matrix", Builtin::Srgb.profile().clone(), working_builtin.clone()),
        ("serialized-to-builtin", profile.clone(), working_builtin),
        ("serialized-same-primaries", profile, working_source),
    ] {
        let transform = Transform::new(&source, &target, Intent::RelativeColorimetric, false).unwrap();
        let mut maximum = 0f64;
        let mut differing = 0;
        let mut point175 = [0.; 3];
        for (index, (pixel, expected)) in input.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()).enumerate() {
            let mut converted = [0.; 3];
            transform.try_eval_f64(&pixel[..3], &mut converted).unwrap();
            let converted = converted.map(|n| f64::from(n as f32));
            if index == 175 {
                point175 = converted;
            }
            for (actual, expected) in converted.into_iter().zip(expected) {
                maximum = maximum.max((actual - expected).abs());
                differing += usize::from(actual.to_bits() != expected.to_bits());
            }
        }
        eprintln!("route={name} differing={differing}/1536 maximum={maximum} pixel175={point175:?}");
    }
}

#[test]
#[ignore = "independent Perceptual RGB/Gray controls; no import admission"]
fn native_perceptual_rgb_gray_source_edit_disabled() {
    use photocraft_cms::{
        xcf_color::{effective_profile_from_definition, Encoding},
        Intent, Transform,
    };
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-evidence");
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|n| format!("{n:02x}")).collect::<String>();
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "8b338d140af0add90f7390e7fc2db4f0adef9dac24f45151ff50140dd66bb59c");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    let entry = manifest["files"].as_array().unwrap().iter().find(|n| n["file"] == "observations.json").unwrap();
    assert_eq!(hash(&bytes), entry["sha256"].as_str().unwrap());
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let read = |d: &serde_json::Value| {
        let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), d["sha256"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        bytes
    };
    let decode = |b: &[u8]| b.as_chunks::<4>().0.iter().map(|n| f64::from(f32::from_le_bytes(*n))).collect::<Vec<_>>();
    let mut states = 0;
    let mut native_max = 0f64;
    let mut display_max = 0f64;
    let mut failures = Vec::new();
    for row in observations["records"].as_array().unwrap() {
        let source = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&source), row["sha256"].as_str().unwrap());
        assert!(row["current"]["profiles"]["assigned"].is_null());
        let gray = row["model"] == "GRAY";
        let channels = if gray { 1 } else { 3 };
        let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        let op = operation(&row["current"]["layers"][0]["filters"][0]["config"]);
        let candidate = ColorRotateCandidate::new(profile, SampleEncoding::Perceptual, op).unwrap();
        let source_profile = effective_profile_from_definition(profile, Encoding::Perceptual).unwrap();
        let display = Transform::new(&source_profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
        let input = decode(&read(&row["current"]["layers"][0]["native"]));
        let baked = decode(&read(&row["baked"]["layers"][0]["native"]));
        let projection = read(&row["current"]["projection"]);
        let projection = projection.as_chunks::<8>().0.iter().map(|n| f64::from_le_bytes(*n)).collect::<Vec<_>>();
        let enabled = row["enabled"].as_bool().unwrap();
        for (index, ((pixel, expected), projection)) in
            input.chunks_exact(channels + 1).zip(baked.chunks_exact(channels + 1)).zip(projection.as_chunks::<4>().0.iter()).enumerate()
        {
            let output = candidate.apply_with(pixel, enabled, &Interrupt::NONE, &|_| Ok(())).unwrap();
            let output = output.map(|n| f64::from(n as f32));
            let native = if gray { vec![output[0], output[3]] } else { output.to_vec() };
            if !enabled {
                assert_eq!(native.iter().map(|n| n.to_bits()).collect::<Vec<_>>(), pixel.iter().map(|n| n.to_bits()).collect::<Vec<_>>());
            }
            for (channel, (actual, expected)) in native.iter().zip(expected).enumerate() {
                let delta = (actual - expected).abs();
                native_max = native_max.max(delta);
                if delta > 2. / 255. {
                    failures.push(format!("{} pixel{index} native{channel} actual{actual} expected{expected}", row["id"]));
                }
            }
            let mut rgb = [0.; 3];
            display.try_eval_f64(&native[..channels], &mut rgb).unwrap();
            let rendered = [rgb[0], rgb[1], rgb[2], output[3]];
            for (channel, (actual, expected)) in rendered.into_iter().zip(projection).enumerate() {
                let delta = (actual - expected).abs();
                display_max = display_max.max(delta);
                if delta > 2. / 255. {
                    failures.push(format!("{} pixel{index} display{channel} actual{actual} expected{expected}", row["id"]));
                }
            }
        }
        states += 1;
    }
    eprintln!("Perceptual originals{states} native{native_max} display{display_max} failures{} first{:?}", failures.len(), failures.first());
    assert_eq!(states, 6);
    assert!(failures.is_empty());
}

#[test]
#[ignore = "independent native Perceptual→preferred working buffers"]
fn perceptual_native_preferred_working_four_controls() {
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let source = root.join("color-rotate-perceptual-evidence");
    let preferred = root.join("color-rotate-perceptual-working-evidence");
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|n| format!("{n:02x}")).collect::<String>();
    assert_eq!(hash(&std::fs::read(preferred.join("batch-manifest.json")).unwrap()), "2a1701077fa5a9ee8507ed091c470b9c3bc8b30aae5672370bad495f28e66346");
    assert_eq!(hash(&std::fs::read(source.join("batch-manifest.json")).unwrap()), "8b338d140af0add90f7390e7fc2db4f0adef9dac24f45151ff50140dd66bb59c");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(source.join("observations.json")).unwrap()).unwrap();
    let working: serde_json::Value = serde_json::from_slice(&std::fs::read(preferred.join("observations.json")).unwrap()).unwrap();
    let read = |root: &std::path::Path, d: &serde_json::Value| {
        let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), d["sha256"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        bytes.as_chunks::<4>().0.iter().map(|n| f32::from_le_bytes(*n)).collect::<Vec<_>>()
    };
    let mut input_max = 0f32;
    let mut output_max = 0f32;
    let mut count = 0;
    for row in working["records"].as_array().unwrap() {
        let original = observations["records"].as_array().unwrap().iter().find(|n| n["id"] == row["id"]).unwrap();
        assert_eq!(row["source_sha256"], original["sha256"]);
        assert_eq!(row["preferred_pad_formats"]["input"], "R~G~B~A float");
        assert_eq!(row["preferred_pad_formats"]["output"], "R~G~B~A float");
        let gray = original["model"] == "GRAY";
        let channels = if gray { 2 } else { 4 };
        let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        let op = operation(&original["current"]["layers"][0]["filters"][0]["config"]);
        let candidate = ColorRotateCandidate::new(profile, SampleEncoding::Perceptual, op).unwrap();
        let native = read(&source, &original["current"]["layers"][0]["native"]);
        let input = read(&preferred, &row["input"]);
        let output = read(&preferred, &row["output"]);
        for ((pixel, expected), expected_output) in native.chunks_exact(channels).zip(input.as_chunks::<4>().0.iter()).zip(output.as_chunks::<4>().0.iter()) {
            let native = pixel.iter().copied().map(f64::from).collect::<Vec<_>>();
            let working = candidate.working_input(&native).unwrap().map(|n| n as f32);
            for (actual, expected) in working.iter().zip(expected) {
                input_max = input_max.max((actual - expected).abs());
                assert!((actual - expected).abs() <= 1e-6);
            }
            let mut actual = [0.; 4];
            op.try_eval_perceptual_float(&working, &mut actual).unwrap();
            for (actual, expected) in actual.into_iter().zip(expected_output) {
                output_max = output_max.max((actual - expected).abs());
                assert!((actual - expected).abs() <= 1e-6);
            }
            count += 1;
        }
    }
    eprintln!("Perceptual preferred4 pixels{count} input{input_max} output{output_max}");
    assert_eq!(count, 2048);
}

#[test]
#[ignore = "frozen independent Color Rotate native references; no source admission"]
fn unproved_nonlinear_original_transfer_is_rejected() {
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-response-evidence");
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|n| format!("{n:02x}")).collect::<String>();
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "d64019de7f02c318b56dad0e16bb6fb7ab8e85784e8e794710d114c2c5d15cbb");
    let census: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let entries = census["files"].as_array().unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    let entry = entries.iter().find(|e| e["file"] == "observations.json").unwrap();
    assert_eq!(hash(&bytes), entry["sha256"].as_str().unwrap());
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let read = |d: &serde_json::Value, double: bool| {
        let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), d["sha256"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        if double {
            bytes.as_chunks::<8>().0.iter().map(|n| f64::from_le_bytes(*n)).collect::<Vec<_>>()
        } else {
            bytes.as_chunks::<4>().0.iter().map(|n| f64::from(f32::from_le_bytes(*n))).collect()
        }
    };
    let mut states = 0;
    for row in observations["records"].as_array().unwrap() {
        let op = operation(&row["current"]["layers"][0]["filters"][0]["config"]);
        if op.validate().is_err() {
            continue;
        }
        let original = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&original), row["sha256"].as_str().unwrap());
        let input = read(&row["current"]["layers"][0]["native"], false);
        assert_eq!(input.len(), 2048);
        assert!(ColorRotateCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, op).is_err());
        states += 1;
    }
    assert_eq!(states, 8);
}

#[test]
#[ignore = "independent boundary-native source arcs and gray modes; admission remains witnessed-only"]
fn native_perceptual_boundary_holdouts_after_sector_wrap() {
    use photocraft_cms::{
        xcf_color::{effective_profile_from_definition, Encoding},
        Intent, Transform,
    };
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-boundary-evidence");
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|n| format!("{n:02x}")).collect::<String>();
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "72e93210100f51ce988929d0ba9010f817b7ca5c1e69cb4f66caaae0eeacc9ac");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    let entry = manifest["files"].as_array().unwrap().iter().find(|n| n["file"] == "observations.json").unwrap();
    assert_eq!(hash(&bytes), entry["sha256"].as_str().unwrap());
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let read = |d: &serde_json::Value| {
        let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), d["sha256"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
        bytes
    };
    let decode = |b: &[u8]| b.as_chunks::<4>().0.iter().map(|n| f64::from(f32::from_le_bytes(*n))).collect::<Vec<_>>();
    let mut states = 0;
    let mut native_max = 0f64;
    let mut display_max = 0f64;
    let mut failures = Vec::new();
    for row in observations["records"].as_array().unwrap() {
        let source = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&source), row["sha256"].as_str().unwrap());
        assert!(row["current"]["profiles"]["assigned"].is_null());
        let gray = row["model"] == "GRAY";
        let channels = if gray { 1 } else { 3 };
        let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        let op = operation(&row["current"]["layers"][0]["filters"][0]["config"]);
        let candidate = ColorRotateCandidate::new(profile, SampleEncoding::Perceptual, op).unwrap();
        let source_profile = effective_profile_from_definition(profile, Encoding::Perceptual).unwrap();
        let display = Transform::new(&source_profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
        let input = decode(&read(&row["current"]["layers"][0]["native"]));
        let baked = decode(&read(&row["baked"]["layers"][0]["native"]));
        let projection = read(&row["current"]["projection"]);
        let projection = projection.as_chunks::<8>().0.iter().map(|n| f64::from_le_bytes(*n)).collect::<Vec<_>>();
        let enabled = row["enabled"].as_bool().unwrap();
        for (index, ((pixel, expected), projection)) in
            input.chunks_exact(channels + 1).zip(baked.chunks_exact(channels + 1)).zip(projection.as_chunks::<4>().0.iter()).enumerate()
        {
            let output = candidate.apply_with(pixel, enabled, &Interrupt::NONE, &|_| Ok(())).unwrap();
            let output = output.map(|n| f64::from(n as f32));
            let native = if gray { vec![output[0], output[3]] } else { output.to_vec() };
            if !enabled {
                assert_eq!(native.iter().map(|n| n.to_bits()).collect::<Vec<_>>(), pixel.iter().map(|n| n.to_bits()).collect::<Vec<_>>());
            }
            for (channel, (actual, expected)) in native.iter().zip(expected).enumerate() {
                let delta = (actual - expected).abs();
                native_max = native_max.max(delta);
                if delta > 2. / 255. {
                    failures.push(format!("{} pixel{index} native{channel} actual{actual} expected{expected}", row["id"]));
                }
            }
            let mut rgb = [0.; 3];
            display.try_eval_f64(&native[..channels], &mut rgb).unwrap();
            let rendered = [rgb[0], rgb[1], rgb[2], output[3]];
            for (channel, (actual, expected)) in rendered.into_iter().zip(projection).enumerate() {
                let delta = (actual - expected).abs();
                display_max = display_max.max(delta);
                if delta > 2. / 255. {
                    failures.push(format!("{} pixel{index} display{channel} actual{actual} expected{expected}", row["id"]));
                }
            }
        }
        states += 1;
    }
    eprintln!("Perceptual originals{states} native{native_max} display{display_max} failures{} first{:?}", failures.len(), failures.first());
    assert_eq!(states, 8);
    // Original source admission remains closed. This candidate comparison
    // replays the unchanged independent native/projection boundary controls.
    assert!(failures.is_empty(), "{:?}", failures.first());
    assert!(native_max <= 2. / 255.);
}
