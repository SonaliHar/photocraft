use photocraft_algo::gimp::point_unpremultiply::Unpremultiply;

#[test]
fn registered_linear_member_preserves_float_equation_and_atomic_refusal() {
    let (member, operation) = photocraft_algo::gimp::registry::unpremultiply_linear_member(Some("0:0")).unwrap();
    assert_eq!(member.encoding, photocraft_algo::gimp::registry::WorkingEncoding::Linear);
    let mut output = [9.; 4];
    operation.try_eval_pixel(&[0.25, -0.5, 0.75, 0.5], 3, &mut output).unwrap();
    assert_eq!(output, [0.5, -1., 1.5, 0.5]);
    for version in [None, Some("1:0"), Some("0:00")] { assert!(photocraft_algo::gimp::registry::unpremultiply_linear_member(version).is_err()); }
    output = [9.; 4];
    assert!(operation.try_eval_pixel(&[1., 0., 0., f64::from(f32::from_bits(1))], 3, &mut output).is_err());
    assert_eq!(output, [9.; 4]);
}

#[test]
fn independently_observed_hidden_and_signed_color_controls() {
    for (input, expected) in [
        ([0.125, 0.25, 0.375, 0.5], [0.25, 0.5, 0.75, 0.5]),
        ([-0.25, 0.5, -1.0, 0.5], [-0.5, 1.0, -2.0, 0.5]),
        ([0.125, 0.25, 0.375, -0.0], [0.0, 0.0, 0.0, -0.0]),
    ] {
        let mut output = [9.0; 4];
        Unpremultiply.try_eval_linear_float(&input, &mut output).unwrap();
        assert_eq!(output.map(f32::to_bits), expected.map(f32::to_bits));
    }
}

#[test]
fn invalid_input_and_overflow_are_atomic() {
    for input in [[f32::NAN, 0.0, 0.0, 1.0], [0.0, f32::INFINITY, 0.0, 1.0], [0.0, 0.0, 0.0, -0.5], [0.0, 0.0, 0.0, 1.5], [1.0, -1.0, 0.0, f32::from_bits(1)]] {
        let mut output = [7.0; 4];
        assert!(Unpremultiply.try_eval_linear_float(&input, &mut output).is_err());
        assert_eq!(output, [7.0; 4]);
    }
    for length in [0, 1, 3, 5] {
        let input = vec![0.5; length];
        let mut output = [7.0; 4];
        assert!(Unpremultiply.try_eval_linear_float(&input, &mut output).is_err());
        assert_eq!(output, [7.0; 4]);
        let mut output = vec![7.0; length];
        assert!(Unpremultiply.try_eval_linear_float(&[0.5; 4], &mut output).is_err());
        assert_eq!(output, vec![7.0; length]);
    }
}

#[test]
fn smallest_subnormal_and_large_finite_color_are_not_clipped() {
    for (input, expected) in [
        ([f32::from_bits(1), 0.0, -f32::from_bits(1), f32::from_bits(1)], [1.0, 0.0, -1.0, f32::from_bits(1)]),
        ([1.0, -1.0, 0.0, f32::MIN_POSITIVE], [2.0_f32.powi(126), -2.0_f32.powi(126), 0.0, f32::MIN_POSITIVE]),
    ] {
        let mut output = [9.0; 4];
        Unpremultiply.try_eval_linear_float(&input, &mut output).unwrap();
        assert_eq!(output.map(f32::to_bits), expected.map(f32::to_bits));
    }
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "private public-API preferred Float32 controls; explicit evidence path required"]
fn independent_float_rounding_and_boundary_packet_is_bit_exact() {
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_UNPREMULTIPLY_WORKING_EVIDENCE").unwrap());
    // The original diagnostic deliberately retains Infinity tokens. Use a strict
    // descriptor sidecar while binding the unmodified original report bytes.
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("serde-descriptors.json")).unwrap()).unwrap();
    let original = std::fs::read(root.join("observations.json")).unwrap();
    let original_hash = Sha256::digest(&original).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(original_hash, report["original_report_sha256"].as_str().unwrap());
    assert_eq!(report["operation"], "gegl:unpremultiply");
    assert_eq!(report["formats"]["input"]["format"], "RGBA float");
    assert_eq!(report["formats"]["output"]["format"], "RGBA float");
    let load = |key: &str| {
        let descriptor = &report[key];
        let raw = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(raw.len() as u64, descriptor["bytes"].as_u64().unwrap());
        let hash = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hash, descriptor["sha256"].as_str().unwrap());
        let (words, remainder) = raw.as_chunks::<4>();
        assert!(remainder.is_empty());
        words.iter().map(|b| f32::from_le_bytes(*b)).collect::<Vec<_>>()
    };
    let inputs = load("input");
    let expected = load("output");
    assert_eq!(inputs.len(), 4104 * 4);
    assert_eq!(inputs.len(), expected.len());
    let mut accepted = 0;
    let mut rejected = 0;
    for (input, reference) in inputs.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()) {
        let mut output = [7.0; 4];
        if reference.iter().all(|v| v.is_finite()) && (0.0..=1.0).contains(&input[3]) {
            Unpremultiply.try_eval_linear_float(input, &mut output).unwrap();
            assert_eq!(output.map(f32::to_bits).as_slice(), reference.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            accepted += 1;
        } else {
            assert!(Unpremultiply.try_eval_linear_float(input, &mut output).is_err());
            assert_eq!(output, [7.0; 4]);
            rejected += 1;
        }
    }
    assert_eq!((accepted, rejected), (4101, 3));
}
