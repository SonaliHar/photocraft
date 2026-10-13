use photocraft_algo::gimp::point_component_polar::extract_model;

#[test]
fn independently_observed_lab_normalizers_do_not_erase_white_hue() {
    for (component, model, expected) in [
        (15, [54.28999328613281, 80.8106689453125, 69.8957290649414], 0.5428999066352844),
        (16, [54.28999328613281, 80.8106689453125, 69.8957290649414], 0.816904604434967),
        (17, [54.28999328613281, 80.8106689453125, 69.8957290649414], 0.7741008996963501),
        (18, [54.28999328613281, 106.84464263916016, 40.85757064819336], 0.534223198890686),
        (19, [100.00001525878906, 2.384185791015625e-05, 90.], 0.25),
    ] {
        let actual = extract_model(component, model, false, false).unwrap();
        assert_eq!(actual[3], 1.);
        assert!((actual[0] - expected).abs() <= 1e-6);
    }
}

#[test]
fn raw_model_nonfinite_overflow_clipping_and_inversion() {
    for component in 15..=19 {
        for model in [[f64::NAN, 0., 0.], [0., f64::INFINITY, 0.], [0., 0., f64::MAX]] {
            assert!(extract_model(component, model, false, false).is_err());
        }
    }
    assert_eq!(extract_model(15, [-5.960464477539063e-8, 0., 0.], false, false).unwrap(), [0., 0., 0., 1.]);
    assert_eq!(extract_model(15, [100.00001525878906, 0., 0.], true, false).unwrap(), [0., 0., 0., 1.]);
    assert_eq!(extract_model(19, [100., 2.384185791015625e-5, 90.], true, false).unwrap(), [0.75, 0.75, 0.75, 1.]);
}

#[test]
#[ignore = "local frozen Lab model-before-storage evidence; no RGB conversion or source admission"]
fn frozen_lab_lch_working_normalization_retains_storage_and_repeat_limits() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let packet = full.join("point-component-extract-lab-lch-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let manifest_bytes = std::fs::read(packet.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest_bytes), "f2f13ee0cdded4211309499f86acc73916191d50037339bb1fd1c00de52a07ac");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let load = |path: &str| {
        let bytes = std::fs::read(packet.join(path)).unwrap();
        let artifact = manifest["artifacts"].as_array().unwrap().iter().find(|row| row["path"] == path).unwrap();
        assert_eq!(hash(&bytes), artifact["sha256"].as_str().unwrap());
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };
    let source = load("states/observations.json");
    let working = load("working/observations.json");
    let root = packet.join("working");
    let read = |descriptor: &serde_json::Value| {
        let bytes = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), descriptor["sha256"].as_str().unwrap());
        let size = descriptor["decoded_bytes"].as_u64().unwrap();
        assert!(size <= 1536);
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(bytes.as_slice()).take(size + 1).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded.len() as u64, size);
        assert_eq!(hash(&decoded), descriptor["decoded_sha256"].as_str().unwrap());
        decoded
    };
    let sources = source["records"].as_array().unwrap();
    let models = working["records"].as_array().unwrap();
    assert_eq!(sources.len(), 900);
    assert_eq!(models.len(), 900);
    let mut floating_max = 0f64;
    let mut integer_max = 0f64;
    let mut integer_worst = None;
    let mut excluded = 0;
    let mut enabled = 0;
    for (source, model) in sources.iter().zip(models) {
        assert_eq!(source["id"], model["id"]);
        assert_eq!(source["sha256"], model["source_sha256"]);
        let id = source["id"].as_str().unwrap();
        if matches!(id, "rgb.u8_non_linear.lch-h.source" | "rgb.u8_non_linear.lch-h.invert" | "rgb.u8_non_linear.lch-h.both" | "rgb.u8_non_linear.lch-h.linear")
        {
            excluded += 1;
            continue; // independently unstable repeat; never a qualified positive
        }
        if source["enabled"] == false {
            continue;
        }
        enabled += 1;
        let component = match source["component"].as_str().unwrap() {
            "lab-l" => 15,
            "lab-a" => 16,
            "lab-b" => 17,
            "lch-c" => 18,
            "lch-h" => 19,
            other => panic!("unexpected {other}"),
        };
        let input = read(&model["input_formats"][if component <= 17 { "CIE Lab float" } else { "CIE LCH(ab) float" }]);
        let output = read(&model["output_working_baked"]);
        for (input, output) in input.as_chunks::<12>().0.iter().zip(output.as_chunks::<16>().0.iter()) {
            let sample = |bytes: &[u8], index: usize| f64::from(f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap()));
            let input = std::array::from_fn(|index| sample(input, index));
            let actual = extract_model(component, input, source["invert"].as_bool().unwrap(), source["linear"].as_bool().unwrap()).unwrap();
            for (axis, value) in actual.into_iter().enumerate() {
                let delta = (value - sample(output, axis)).abs();
                let precision = source["precision"].as_str().unwrap();
                if precision.starts_with("FLOAT_") || precision.starts_with("DOUBLE_") {
                    floating_max = floating_max.max(delta);
                    assert!(delta <= 1e-6, "{id} axis{axis}: {delta}");
                } else if delta > integer_max {
                    integer_max = delta;
                    integer_worst = Some((id.to_string(), axis, value, sample(output, axis)));
                }
            }
        }
    }
    assert_eq!(excluded, 4);
    println!(
        "{enabled} enabled direct Lab/LCh model controls: floating max{floating_max}, integer baked max{integer_max};4 unstable U8NL hue repeats excluded; no RGB/source admission"
    );
    println!("unqualified integer model/baked worst {integer_worst:?}");
}
