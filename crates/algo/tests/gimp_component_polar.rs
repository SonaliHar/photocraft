use photocraft_algo::gimp::point_component_polar::{extract_model, hsv_hsl};

#[test]
fn independent_primary_and_neutral_model_controls() {
    for (rgb, hsv, hsl) in [
        ([1., 0., 0.], [0., 1., 1.], [0., 1., 0.5]),
        ([0., 1., 0.], [1. / 3., 1., 1.], [1. / 3., 1., 0.5]),
        ([0., 0., 1.], [2. / 3., 1., 1.], [2. / 3., 1., 0.5]),
        ([0.5; 3], [0., 0., 0.5], [0., 0., 0.5]),
        ([0.; 3], [0.; 3], [0.; 3]),
        ([1.; 3], [0., 0., 1.], [0., 0., 1.]),
    ] {
        assert_eq!(hsv_hsl(rgb).unwrap(), (hsv, hsl));
    }
}

#[test]
fn model_domains_boundaries_and_invalid_inputs() {
    for bad in [[f64::NAN, 0., 0.], [0., f64::INFINITY, 0.], [-f64::EPSILON, 0., 0.], [1. + f64::EPSILON, 0., 0.]] {
        assert!(hsv_hsl(bad).is_err());
        assert!(extract_model(3, bad, false, false).is_err());
    }
    for component in [0, 2, 8, 20, 255] {
        assert!(extract_model(component, [0.; 3], false, false).is_err());
    }
    for (component, scalar) in [(3, 0.25), (4, 0.5), (5, 0.75), (6, 0.5), (7, 0.75)] {
        assert_eq!(extract_model(component, [0.25, 0.5, 0.75], false, false).unwrap(), [scalar, scalar, scalar, 1.]);
        assert_eq!(extract_model(component, [0.25, 0.5, 0.75], true, false).unwrap(), [1. - scalar, 1. - scalar, 1. - scalar, 1.]);
    }
    let (hsv, hsl) = hsv_hsl([0.5, 0.5 + 1e-14, 0.5]).unwrap();
    assert_eq!(hsv[0], 1. / 3.);
    assert!(hsv[1] > 0. && hsl[1] > 0.); // no near-neutral hue suppression
    assert_eq!(extract_model(3, [0., 0., 0.], false, true).unwrap(), [0., 0., 0., 1.]);
    assert_eq!(extract_model(3, [1., 1., 1.], false, true).unwrap(), [1.; 4]);
}

#[test]
#[ignore = "local frozen public Double model observations, not RGB pipeline admission"]
fn frozen_double_model_route_gap_is_retained_and_extraction_is_separate() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let packet = full.join("point-component-extract-hsv-hsl-evidence");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let manifest_bytes = std::fs::read(packet.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest_bytes), "32c9948cc94bdba33979deff8f116da9f1ce5f52e58bf4b32649f173c3d3dcf1");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    for path in ["working/observations.json", "states/observations.json"] {
        let artifact = manifest["artifacts"].as_array().unwrap().iter().find(|row| row["path"] == path).unwrap();
        assert_eq!(hash(&std::fs::read(packet.join(path)).unwrap()), artifact["sha256"].as_str().unwrap());
    }
    let root = packet.join("working");
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let read = |descriptor: &serde_json::Value| {
        let encoded = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&encoded), descriptor["sha256"].as_str().unwrap());
        let size = descriptor["decoded_bytes"].as_u64().unwrap();
        assert!(size <= 1536);
        let mut bytes = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(size + 1).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len() as u64, size);
        assert_eq!(hash(&bytes), descriptor["decoded_sha256"].as_str().unwrap());
        bytes
    };
    let number = |bytes: &[u8], index: usize| f64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap());
    let encode = |value: f64| if value <= 0.0031308 { 12.92 * value } else { 1.055 * value.powf(1. / 2.4) - 0.055 };
    let records = json["records"].as_array().unwrap();
    let sources: serde_json::Value = serde_json::from_slice(&std::fs::read(packet.join("states/observations.json")).unwrap()).unwrap();
    let sources = sources["records"].as_array().unwrap();
    assert_eq!(records.len(), 900);
    let mut maxima = [0f64; 2];
    let mut extraction_maximum = 0f64;
    let mut floating_extraction_maximum = 0f64;
    let mut enabled = 0;
    let mut rejected_route_inputs = 0;
    for (row, source) in records.iter().zip(sources) {
        assert_eq!(row["id"], source["id"]);
        assert_eq!(row["source_sha256"], source["sha256"]);
        let linear = read(&row["input_formats"]["RGBA double"]);
        let hsv = read(&row["input_formats"]["HSV double"]);
        let hsl = read(&row["input_formats"]["HSL double"]);
        assert_eq!(linear.len() / 32, hsv.len() / 24);
        assert_eq!(hsv.len(), hsl.len());
        for ((rgb, hsv), hsl) in linear.as_chunks::<32>().0.iter().zip(hsv.as_chunks::<24>().0.iter()).zip(hsl.as_chunks::<24>().0.iter()) {
            let rgb = std::array::from_fn(|index| encode(number(rgb, index)));
            let Ok((actual_hsv, actual_hsl)) = hsv_hsl(rgb) else {
                assert!(rgb.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)), "unexpected model error {rgb:?}");
                rejected_route_inputs += 1;
                continue;
            };
            for (model, (actual, expected)) in [(actual_hsv, hsv), (actual_hsl, hsl)].into_iter().enumerate() {
                for (axis, value) in actual.into_iter().enumerate() {
                    let delta = (value - number(expected, axis)).abs();
                    maxima[model] = maxima[model].max(delta);
                }
            }
        }
        if source["enabled"] == false {
            continue;
        }
        enabled += 1;
        let component = match source["component"].as_str().unwrap() {
            "hue" => 3,
            "hsv-s" => 4,
            "hsv-v" => 5,
            "hsl-s" => 6,
            "hsl-l" => 7,
            other => panic!("unexpected component {other}"),
        };
        let floats = read(&row["input_formats"][if component <= 5 { "HSV float" } else { "HSL float" }]);
        let output = read(&row["output_working_baked"]);
        for (model, expected) in floats.as_chunks::<12>().0.iter().zip(output.as_chunks::<16>().0.iter()) {
            let get = |bytes: &[u8], index| f64::from(f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap()));
            let model = std::array::from_fn(|index| get(model, index));
            let actual = extract_model(component, model, source["invert"].as_bool().unwrap(), source["linear"].as_bool().unwrap()).unwrap();
            assert_eq!(actual[3], 1.);
            for (axis, value) in actual.into_iter().enumerate() {
                let delta = (value - get(expected, axis)).abs();
                extraction_maximum = extraction_maximum.max(delta);
                let precision = source["precision"].as_str().unwrap();
                if precision.starts_with("FLOAT_") || precision.starts_with("DOUBLE_") {
                    floating_extraction_maximum = floating_extraction_maximum.max(delta);
                    assert!(delta <= 1e-6, "{} Float-model extraction axis{axis}: {delta}", source["id"]);
                }
            }
        }
    }
    assert!(maxima[0] > 2e-10 || maxima[1] > 2e-10, "conversion gap must not silently be called exact");
    assert!(extraction_maximum > 2. / 255., "integer baked quantization must not silently be called Float-model equality");
    println!(
        "900 public Double route models maxima {maxima:?}; {enabled} public model/baked extraction max {}/255, Float/Double baked max {floating_extraction_maximum}; no RGB/source/render admission",
        extraction_maximum * 255.
    );
    println!("{rejected_route_inputs} analytic converter inputs outside the unit model domain remain Unsupported");
}
