use photocraft_algo::gimp::point_channel_mixer::ChannelMixer;

#[test]
fn independent_linear_basis_and_signed_normalization() {
    let input = [0.125, 0.375, 0.75, 0.0];
    let mut output = [9.0; 4];
    let mixer = ChannelMixer { gains: [[1.5, 0.25, 0.125], [0.125, 1.25, 0.5], [0.25, 0.125, 1.75]], preserve_luminosity: false };
    mixer.try_eval_linear_float(&input, &mut output).unwrap();
    assert_eq!(output, [0.375, 0.859375, 1.390625, 0.0]);
    ChannelMixer { preserve_luminosity: true, ..mixer }.try_eval_linear_float(&input, &mut output).unwrap();
    assert_eq!(output, [(0.375_f64 / 1.875) as f32, (0.859375_f64 / 1.875) as f32, (1.390625_f64 / 2.125) as f32, 0.0]);
}

#[test]
fn invalid_inputs_and_output_overflow_leave_output_unchanged() {
    let mixer = ChannelMixer { gains: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], preserve_luminosity: false };
    for input in [[f32::NAN, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -0.01], [0.0, 0.0, 0.0, f32::INFINITY]] {
        let mut output = [7.0; 4];
        assert!(mixer.try_eval_linear_float(&input, &mut output).is_err());
        assert_eq!(output, [7.0; 4]);
    }
    for value in [f64::NAN, f64::INFINITY, 2.01, -2.01] {
        let mut invalid = mixer;
        invalid.gains[0][0] = value;
        let mut output = [7.0; 4];
        assert!(invalid.try_eval_linear_float(&[0.1, 0.2, 0.3, 0.4], &mut output).is_err());
        assert_eq!(output, [7.0; 4]);
    }
    let mut output = [7.0; 4];
    assert!(mixer.try_eval_linear_float(&[0.0; 3], &mut output).is_err());
    assert_eq!(output, [7.0; 4]);
    let overflow = ChannelMixer { gains: [[2.0; 3]; 3], ..mixer };
    assert!(overflow.try_eval_linear_float(&[f32::MAX, f32::MAX, f32::MAX, 0.5], &mut output).is_err());
    assert_eq!(output, [7.0; 4]);
}

#[test]
#[ignore = "release-only 24MP standalone working arithmetic benchmark"]
fn prepared_24mp_linear_working_arithmetic() {
    let mixer = ChannelMixer { gains: [[1.5, 0.25, 0.125], [0.125, 1.25, 0.5], [0.25, 0.125, 1.75]], preserve_luminosity: true };
    let start = std::time::Instant::now();
    let mut sum = 0.0;
    for i in 0..24_000_000 {
        let input = [(i % 257) as f32 / 256.0, 0.375, 0.75, 0.625];
        let mut output = [0.0; 4];
        mixer.try_eval_linear_float(std::hint::black_box(&input), &mut output).unwrap();
        sum += f64::from(output[0]);
        std::hint::black_box(output);
    }
    println!("24MP Channel Mixer {:?} checksum{sum}; pure O(1) working arithmetic, CMS/storage excluded", start.elapsed());
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires local unpublished Channel Mixer response candidate packet"]
fn independent_float_linear_original_matrix_outputs_match_exactly() {
    use sha2::{Digest, Sha256};
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("../photocraft-corpus/xcf/full"));
    let folder = root.join("channel-mixer-domain-response-evidence");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(folder.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in report["records"].as_array().unwrap() {
        if row["model"] != "RGB" || row["precision"] != "FLOAT_LINEAR" {
            continue;
        }
        let weights: Vec<f64> = row["weights"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let mixer = ChannelMixer {
            gains: [[weights[0], weights[1], weights[2]], [weights[3], weights[4], weights[5]], [weights[6], weights[7], weights[8]]],
            preserve_luminosity: row["preserve"].as_bool().unwrap(),
        };
        let read = |phase: &str| {
            let descriptor = &row[phase]["layers"][0]["working"]["RGBA double"];
            let bytes = std::fs::read(folder.join(descriptor["file"].as_str().unwrap())).unwrap();
            let hash = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
            assert_eq!(hash, descriptor["sha256"].as_str().unwrap());
            bytes.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v) as f32).collect::<Vec<_>>()
        };
        let source = read("before");
        let expected = read("baked");
        for (input, expected) in source.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()) {
            let mut output = [0.0; 4];
            mixer.try_eval_linear_float(input, &mut output).unwrap();
            assert_eq!(output, *expected, "{}", row["id"]);
        }
        count += 1;
    }
    assert_eq!(count, 10);
}
