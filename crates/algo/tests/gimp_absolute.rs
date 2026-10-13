use photocraft_algo::gimp::point_absolute::Absolute;

#[test]
fn linear_signed_color_absolute_preserves_hidden_alpha_bits() {
    let mut output = [9.0; 4];
    Absolute.try_eval_linear_float(&[-1.5, -0.0, 3.0, -0.0], &mut output).unwrap();
    assert_eq!(output.map(f32::to_bits), [1.5_f32.to_bits(), 0.0_f32.to_bits(), 3.0_f32.to_bits(), (-0.0_f32).to_bits()]);
}

#[test]
fn invalid_linear_samples_and_lengths_are_atomic() {
    for input in [[f32::NAN, 0.0, 0.0, 1.0], [0.0, f32::INFINITY, 0.0, 1.0], [0.0, 0.0, 0.0, -0.01], [0.0, 0.0, 0.0, 1.01]] {
        let mut output = [7.0; 4];
        assert!(Absolute.try_eval_linear_float(&input, &mut output).is_err());
        assert_eq!(output, [7.0; 4]);
    }
    let mut output = [7.0; 4];
    assert!(Absolute.try_eval_linear_float(&[0.0; 3], &mut output).is_err());
    assert_eq!(output, [7.0; 4]);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "independent unpublished public Absolute preferred working controls"]
fn independent_signed_zero_and_hidden_alpha_float_controls_are_bit_exact() {
    use sha2::{Digest, Sha256};
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("../photocraft-corpus/xcf/full"));
    let root = full.join("point-absolute-working-evidence");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    assert_eq!(report["formats"]["input"]["format"], "RGBA float");
    assert_eq!(report["formats"]["output"]["format"], "RGBA float");
    let load = |key: &str| {
        let descriptor = &report[key];
        let raw = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(raw.len() as u64, descriptor["bytes"].as_u64().unwrap());
        let hash = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hash, descriptor["sha256"].as_str().unwrap());
        raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect::<Vec<_>>()
    };
    let source = load("input");
    let expected = load("output");
    assert_eq!(source.len(), expected.len());
    assert_eq!(source.len(), 60);
    for (input, reference) in source.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()) {
        let mut output = [0.0; 4];
        Absolute.try_eval_linear_float(input, &mut output).unwrap();
        assert_eq!(output.map(f32::to_bits).as_slice(), reference.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
    }
}

#[test]
#[ignore = "release-only 24MP standalone working arithmetic benchmark"]
fn prepared_24mp_linear_working_arithmetic() {
    let start = std::time::Instant::now();
    let mut checksum = 0.0;
    for i in 0..24_000_000 {
        let input = [-((i % 257) as f32 / 256.0), -0.0, 1.5, 0.625];
        let mut output = [0.0; 4];
        Absolute.try_eval_linear_float(std::hint::black_box(&input), &mut output).unwrap();
        checksum += f64::from(output[0]);
        std::hint::black_box(output);
    }
    println!("24MP Absolute {:?} checksum{checksum}; pure working arithmetic, CMS/storage excluded", start.elapsed());
}
