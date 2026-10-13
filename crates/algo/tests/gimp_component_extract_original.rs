use photocraft_algo::gimp::registry::component_extract_member;

#[test]
fn rgb_selector_produces_independently_observed_opaque_output() {
    let (_, operation) = component_extract_member(Some("0:0"), 0, false, false).unwrap();
    let mut result = [7.; 4];
    operation.try_eval_pixel(&[0.25, 0.5, 0.75, 0.125], 3, &mut result).unwrap();
    assert_eq!(result, [0.25, 0.25, 0.25, 1.]);
}

#[test]
fn unqualified_choices_versions_and_bad_samples_fail_atomically() {
    for component in (3..8).chain(15..20) {
        assert!(component_extract_member(Some("0:0"), component, false, false).is_err());
    }
    for version in [None, Some("0:1"), Some("1:0"), Some("")] {
        assert!(component_extract_member(version, 0, false, false).is_err());
    }
    let (_, operation) = component_extract_member(Some("0:0"), 20, true, true).unwrap();
    for input in [[f64::NAN, 0.5, 0.75, 0.125], [0.25, f64::INFINITY, 0.75, 0.125], [0.25, 0.5, 0.75, 1.1]] {
        let mut output = [7.; 4];
        assert!(operation.try_eval_pixel(&input, 3, &mut output).is_err());
        assert_eq!(output, [7.; 4]);
    }
    let mut short = [7.; 3];
    assert!(operation.try_eval_pixel(&[0.25, 0.5, 0.75], 3, &mut short).is_err());
    assert_eq!(short, [7.; 3]);
    for component in [0, 1, 2] {
        let (_, gray) = component_extract_member(Some("0:0"), component, false, false).unwrap();
        let mut output = [7.; 2];
        gray.try_eval_pixel(&[0.5, 0.125], 1, &mut output).unwrap();
        assert_eq!(output, [0.5, 1.]);
    }
}

#[test]
#[ignore = "local unpublished 576 independently authored Component Extract working controls"]
fn original_four_selectors_and_bools_match_independent_working_controls() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-xcf-probes/component-extract-working-private");
    let sources: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.with_file_name("component-extract-precision-private").join("observations.json")).unwrap()).unwrap();
    let working: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let bytes = |descriptor: &serde_json::Value| {
        let encoded = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        let hex = |data: &[u8]| Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(hex(&encoded), descriptor["sha256"].as_str().unwrap());
        assert_eq!(encoded.len() as u64, descriptor["bytes"].as_u64().unwrap());
        assert_eq!(descriptor["decoded_bytes"].as_u64().unwrap(), 768);
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(769).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded.len(), 768);
        assert_eq!(hex(&decoded), descriptor["decoded_sha256"].as_str().unwrap());
        decoded
    };
    let sources = sources["records"].as_array().unwrap();
    let controls = working["records"].as_array().unwrap();
    assert_eq!(sources.len(), 576);
    assert_eq!(controls.len(), 576);
    let mut maximum = 0f64;
    for (source, control) in sources.iter().zip(controls) {
        assert_eq!(source["id"], control["id"]);
        let component = match source["component"].as_str().unwrap() {
            "rgb-r" => 0,
            "rgb-g" => 1,
            "rgb-b" => 2,
            "alpha" => 20,
            other => panic!("unqualified {other}"),
        };
        let (_, operation) =
            component_extract_member(Some("0:0"), component, source["invert"].as_bool().unwrap(), source["linear"].as_bool().unwrap()).unwrap();
        let inputs = bytes(&control["input_working"]);
        let expected = bytes(&control["output_working_baked"]);
        for (input, expected) in inputs.as_chunks::<16>().0.iter().zip(expected.as_chunks::<16>().0.iter()) {
            let input: [f64; 4] = std::array::from_fn(|index| f64::from(f32::from_le_bytes(input[index * 4..index * 4 + 4].try_into().unwrap())));
            let expected: [f64; 4] = std::array::from_fn(|index| f64::from(f32::from_le_bytes(expected[index * 4..index * 4 + 4].try_into().unwrap())));
            let mut output = [7.; 4];
            operation.try_eval_pixel(&input, 3, &mut output).unwrap();
            assert_eq!(output[3], 1.);
            assert_eq!(expected[3], 1.);
            for axis in 0..3 {
                let delta = (output[axis] - expected[axis]).abs();
                maximum = maximum.max(delta);
                assert!(delta <= 2. / 255., "{} {} /255", source["id"], delta * 255.);
            }
        }
    }
    println!("576 Component Extract working/native storage controls max {}/255", maximum * 255.);
}

#[test]
#[ignore = "release-only 24MP Component Extract kernel; CMS/storage/allocation excluded"]
fn component_extract_kernel_24mp_release() {
    if cfg!(debug_assertions) {
        return;
    }
    for linear in [false, true] {
        let (_, operation) = component_extract_member(Some("0:0"), 0, true, linear).unwrap();
        let input = [0.25, 0.5, 0.75, 0.125];
        let mut output = [0.; 4];
        let start = std::time::Instant::now();
        for _ in 0..24_000_000 {
            operation.try_eval_pixel(std::hint::black_box(&input), 3, std::hint::black_box(&mut output)).unwrap();
        }
        assert_eq!(output[3], 1.);
        println!("24MP Component linear={linear}: {:.3}ms (kernel-only)", start.elapsed().as_secs_f64() * 1000.);
    }
}
