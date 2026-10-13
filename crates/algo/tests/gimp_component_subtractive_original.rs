use photocraft_algo::gimp::registry::component_extract_member;

#[test]
fn independently_observed_subtractive_selectors_produce_opaque_normalized_output() {
    // Independently authored public CMYK/YCbCr controls, not inferred from the
    // already qualified RGB channel selectors. Remaining choices stay closed.
    for (component, expected) in [(8, 0.), (9, 1.), (10, 1.), (11, 0.), (12, 0.299), (13, 0.3312641083521445), (14, 1.)] {
        let (_, operation) = component_extract_member(Some("0:0"), component, false, false).unwrap();
        let mut output = [7.; 4];
        operation.try_eval_pixel(&[1., 0., 0., 0.125], 3, &mut output).unwrap();
        assert_eq!(output[3], 1.);
        for value in &output[..3] {
            assert!((*value - expected).abs() <= 1e-6, "component {component}: {output:?}");
        }
    }
}

#[test]
fn black_neutral_endpoints_inversion_and_invalid_samples_are_atomic() {
    for component in 8..=14 {
        let (_, operation) = component_extract_member(Some("0:0"), component, false, false).unwrap();
        let mut output = [7.; 4];
        operation.try_eval_pixel(&[0., 0., 0., 0.], 3, &mut output).unwrap();
        let expected = if component <= 11 {
            1.
        } else if component == 12 {
            0.
        } else {
            0.5
        };
        assert_eq!(output, [expected, expected, expected, 1.]);
        for bad in [[f64::NAN, 0., 0., 0.], [0., f64::INFINITY, 0., 0.], [0., 0., 0., 1.00001], [-0.00001, 0., 0., 1.]] {
            let mut unchanged = [7.; 4];
            assert!(operation.try_eval_pixel(&bad, 3, &mut unchanged).is_err());
            assert_eq!(unchanged, [7.; 4]);
        }
        let (_, invert) = component_extract_member(Some("0:0"), component, true, false).unwrap();
        invert.try_eval_pixel(&[0., 0., 0., 0.], 3, &mut output).unwrap();
        assert_eq!(output, [1. - expected, 1. - expected, 1. - expected, 1.]);
    }
    for component in (3..8).chain(15..20).chain([21, 255]) {
        assert!(component_extract_member(Some("0:0"), component, false, false).is_err());
    }
}

fn check_working(family: &str, expected_count: usize) {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let packet = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(format!("point-component-extract-{family}-evidence"));
    let root = packet.join("working");
    let sources: serde_json::Value = serde_json::from_slice(&std::fs::read(packet.join("states/observations.json")).unwrap()).unwrap();
    let working: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let bytes = |descriptor: &serde_json::Value| {
        let encoded = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        let hex = |data: &[u8]| Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(hex(&encoded), descriptor["sha256"].as_str().unwrap());
        assert_eq!(encoded.len() as u64, descriptor["bytes"].as_u64().unwrap());
        let limit = descriptor["decoded_bytes"].as_u64().unwrap();
        assert!(matches!(limit, 576 | 768));
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(limit + 1).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded.len() as u64, limit);
        assert_eq!(hex(&decoded), descriptor["decoded_sha256"].as_str().unwrap());
        decoded
    };
    let sources = sources["records"].as_array().unwrap();
    let controls = working["records"].as_array().unwrap();
    assert_eq!(sources.len(), expected_count);
    assert_eq!(controls.len(), expected_count);
    let mut maximum = 0f64;
    for (source, control) in sources.iter().zip(controls) {
        assert_eq!(source["id"], control["id"]);
        assert_eq!(source["sha256"], control["source_sha256"]);
        if source["enabled"] == false {
            continue;
        }
        let component = match source["component"].as_str().unwrap() {
            "cmyk-c" => 8,
            "cmyk-m" => 9,
            "cmyk-y" => 10,
            "cmyk-k" => 11,
            "ycbcr-y" => 12,
            "ycbcr-cb" => 13,
            "ycbcr-cr" => 14,
            other => panic!("unqualified {other}"),
        };
        let (_, operation) =
            component_extract_member(Some("0:0"), component, source["invert"].as_bool().unwrap(), source["linear"].as_bool().unwrap()).unwrap();
        let inputs = bytes(&control["input_formats"]["R'G'B'A float"]);
        // Before-storage public model buffers are distinct from a baked integer
        // native buffer converted back to nonlinear. Baked-native quantization
        // and live projection have separate original-source tests.
        let model = bytes(&control["input_formats"][if family == "cmyk" { "CMYK float" } else { "Y\'CbCr float" }]);
        let model_channels = if family == "cmyk" { 4 } else { 3 };
        for (input, model) in inputs.as_chunks::<16>().0.iter().zip(model.chunks_exact(model_channels * 4)) {
            let input: [f64; 4] = std::array::from_fn(|index| f64::from(f32::from_le_bytes(input[index * 4..index * 4 + 4].try_into().unwrap())));
            let index = usize::from(component - if family == "cmyk" { 8 } else { 12 });
            let model = f32::from_le_bytes(model[index * 4..index * 4 + 4].try_into().unwrap());
            let mut scalar = (model + if matches!(component, 13 | 14) { 0.5 } else { 0. }).clamp(0., 1.);
            if source["invert"] == true {
                scalar = 1. - scalar;
            }
            let expected = if source["linear"] == true {
                let value = f64::from(scalar);
                (if value <= 0.0031308 { 12.92 * value } else { 1.055 * value.powf(1. / 2.4) - 0.055 }) as f32
            } else {
                scalar
            };
            let expected = [f64::from(expected), f64::from(expected), f64::from(expected), 1.];
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
    println!("{expected_count} {family} independent public model-before-storage controls max {}/255", maximum * 255.);
}

#[test]
#[ignore = "local unpublished1260 independent CMYK/YCbCr working/native states"]
fn original_subtractive_selectors_and_bools_match_independent_public_model_controls() {
    check_working("cmyk", 720);
    check_working("ycbcr", 540);
}

#[test]
#[ignore = "release-only24MP CMYK/YCbCr kernel; CMS/storage/allocation excluded"]
fn subtractive_kernel_24mp_release() {
    if cfg!(debug_assertions) {
        return;
    }
    for component in [8, 12] {
        for linear in [false, true] {
            let (_, operation) = component_extract_member(Some("0:0"), component, true, linear).unwrap();
            let input = [0.251125, 0.5078125, 0.75001, 0.125];
            let mut output = [0.; 4];
            let start = std::time::Instant::now();
            for _ in 0..24_000_000 {
                operation.try_eval_pixel(std::hint::black_box(&input), 3, std::hint::black_box(&mut output)).unwrap();
            }
            assert_eq!(output[3], 1.);
            println!("24MP Component selector={component} linear={linear}: {:.3}ms (kernel only)", start.elapsed().as_secs_f64() * 1000.);
        }
    }
}
