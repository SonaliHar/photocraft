//! Independently measured literal-color response; no GPL implementation source.
use photocraft_algo::gimp::registry::{WorkingEncoding, color_exchange_member};
use serde_json::Value;

#[test]
#[ignore = "local unpublished Color Exchange public-response evidence; set PHOTOCRAFT_XCF_COLOR_EXCHANGE_CONTROLS"]
fn original_public_working_components_match_typed_color_kernel() {
    let path = std::env::var("PHOTOCRAFT_XCF_COLOR_EXCHANGE_CONTROLS").expect("set PHOTOCRAFT_XCF_COLOR_EXCHANGE_CONTROLS");
    check_controls(&path, 144, 27648);
}

#[test]
#[ignore = "local unpublished independent Color Exchange representable-neighbor controls"]
fn independent_ulp_neighbors_zero_and_arbitrary_thresholds_match_public_response() {
    let path = std::env::var("PHOTOCRAFT_XCF_COLOR_EXCHANGE_CONTROLS").unwrap();
    let file = std::path::Path::new(&path).parent().unwrap().join("color-exchange-boundary-kernel-controls-bits.json");
    check_controls(file.to_str().unwrap(), 12, 6208);
}

fn check_controls(path: &str, records_count: usize, component_count: usize) {
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let records = fixture["records"].as_array().unwrap();
    assert_eq!(records.len(), records_count);
    let mut components = 0;
    for record in records {
        let from: [f32; 4] = std::array::from_fn(|i| f32::from_bits(record["from_bits"][i].as_u64().unwrap() as u32));
        let to: [f32; 4] = std::array::from_fn(|i| f32::from_bits(record["to_bits"][i].as_u64().unwrap() as u32));
        let threshold: [f32; 3] = std::array::from_fn(|i| record["thresholds"][i].as_f64().unwrap() as f32);
        let (member, operation) = color_exchange_member(Some("0:0"), from, to, threshold).unwrap();
        assert_eq!(member.encoding, WorkingEncoding::Nonlinear);
        let prepared = operation.prepare().unwrap();
        for (input, expected) in record["input_f64_bits"].as_array().unwrap().iter().zip(record["expected_f64_bits"].as_array().unwrap()) {
            let input: [f64; 4] = std::array::from_fn(|i| f64::from_bits(input[i].as_u64().unwrap()));
            let expected: [f64; 4] = std::array::from_fn(|i| f64::from_bits(expected[i].as_u64().unwrap()));
            let mut output = [0.; 4];
            prepared.try_eval_pixel(&input, 3, &mut output).unwrap();
            assert_eq!(output, expected, "{}", record["id"]);
            assert_eq!(output[3].to_bits(), input[3].to_bits());
            components += 4;
        }
    }
    assert_eq!(components, component_count);
}

#[test]
fn typed_color_parameters_and_pixel_failures_are_atomic() {
    let from = [0.375, 0.5, 0.625, 0.75];
    let to = [0.125, 0.75, 0.25, 0.25];
    let thresholds = [0.125, 0.1875, 0.25];
    for version in [None, Some(""), Some("1:0"), Some("0:1")] {
        assert!(color_exchange_member(version, from, to, thresholds).is_err());
    }
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.0001, 1.0001] {
        let mut altered = from;
        altered[3] = invalid;
        assert!(color_exchange_member(Some("0:0"), altered, to, thresholds).is_err());
        let mut altered = to;
        altered[0] = invalid;
        assert!(color_exchange_member(Some("0:0"), from, altered, thresholds).is_err());
        let mut altered = thresholds;
        altered[2] = invalid;
        assert!(color_exchange_member(Some("0:0"), from, to, altered).is_err());
    }
    let operation = color_exchange_member(Some("0:0"), from, to, thresholds).unwrap().1;
    for bad in [[f64::NAN, 0.5, 0.625, 1.], [0.375, 0.5, 0.625, f64::INFINITY], [-0.01, 0.5, 0.625, 1.], [0.375, 1.01, 0.625, 1.]] {
        let mut output = [7.; 4];
        assert!(operation.try_eval_pixel(&bad, 3, &mut output).is_err());
        assert_eq!(output, [7.; 4]);
    }
    let mut gray = [7.; 2];
    assert!(operation.try_eval_pixel(&[0.5, 1.], 1, &mut gray).is_err());
    assert_eq!(gray, [7.; 2]);
    let mut output = [7.; 4];
    operation.try_eval_pixel(&[0.375, 0.5, 0.625, -0.], 3, &mut output).unwrap();
    assert_eq!(output[..3], [0.125, 0.75, 0.25]);
    assert_eq!(output[3].to_bits(), (-0f64).to_bits());
}

#[test]
#[ignore = "local unpublished Gray Color Exchange CMS conversion evidence"]
fn active_analytic_gray_rgb_working_and_reverse_conversion_match_original_native() {
    use photocraft_cms::xcf_color::{Encoding, effective_profile_from_definition};
    use photocraft_cms::{Builtin, Intent, Transform};
    let path = std::env::var("PHOTOCRAFT_XCF_COLOR_EXCHANGE_CONTROLS").unwrap();
    let file = std::path::Path::new(&path).parent().unwrap().join("color-exchange-gray-native-controls.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let mut maximum = 0f64;
    for record in fixture["records"].as_array().unwrap() {
        let source_encoding = if record["precision"].as_str().unwrap().ends_with("NON_LINEAR") {
            Encoding::NonlinearProfile
        } else if record["precision"].as_str().unwrap().ends_with("PERCEPTUAL") {
            Encoding::Perceptual
        } else {
            Encoding::Linear
        };
        let target = effective_profile_from_definition(Builtin::SGray.profile(), source_encoding).unwrap();
        let reverse = Transform::new(Builtin::Srgb.profile(), &target, Intent::RelativeColorimetric, false).unwrap();
        let from: [f32; 4] = std::array::from_fn(|i| f32::from_bits(record["from_bits"][i].as_u64().unwrap() as u32));
        let to: [f32; 4] = std::array::from_fn(|i| f32::from_bits(record["to_bits"][i].as_u64().unwrap() as u32));
        let threshold: [f32; 3] = std::array::from_fn(|i| record["thresholds"][i].as_f64().unwrap() as f32);
        let operation = color_exchange_member(Some("0:0"), from, to, threshold).unwrap().1.prepare().unwrap();
        let curve = photocraft_cms::curve::srgb_trc();
        for (input, expected) in record["source_gray_f64_bits"].as_array().unwrap().iter().zip(record["baked_gray_f64_bits"].as_array().unwrap()) {
            let gray = f64::from_bits(input[0].as_u64().unwrap());
            let alpha = f64::from_bits(input[1].as_u64().unwrap());
            let expected = f64::from_bits(expected[0].as_u64().unwrap());
            let nonlinear = if source_encoding == Encoding::Linear { curve.eval_inverse64(gray) } else { gray };
            let nonlinear = f64::from(nonlinear as f32);
            let mut output = [0.; 4];
            operation
                .try_eval_pixel(&[nonlinear, nonlinear, nonlinear, alpha], 3, &mut output)
                .unwrap_or_else(|e| panic!("{} sourcegray {} nonlinear {} alpha {}: {e:?}", record["id"], gray, nonlinear, alpha));
            let mut converted = [0.; 1];
            reverse.try_eval_f64(&output[..3], &mut converted).unwrap();
            let result = if record["enabled"].as_bool().unwrap() { converted[0] } else { gray };
            let delta = (result - expected).abs();
            maximum = maximum.max(delta);
            assert!(delta <= 2. / 255., "{} gray native delta {}", record["id"], delta);
            assert_eq!(alpha.to_bits(), output[3].to_bits());
        }
    }
    println!("Gray native maximum {maximum} ({} /255)", maximum * 255.);
}

#[test]
#[ignore = "local unpublished independent 125-color cube and arbitrary Gray conversion holdouts"]
fn independent_non_neutral_rgb_cube_to_gray_matches_active_analytic_cms() {
    use photocraft_cms::xcf_color::{Encoding, effective_profile_from_definition};
    use photocraft_cms::{Builtin, Intent, Transform};
    let path = std::env::var("PHOTOCRAFT_XCF_COLOR_EXCHANGE_CONTROLS").unwrap();
    let file = std::path::Path::new(&path).parent().unwrap().join("color-exchange-gray-conversion-holdouts/normalized-native-bits.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(fixture["records"].as_array().unwrap().len(), 18);
    let inputs = fixture["input_bits"].as_array().unwrap();
    assert_eq!(inputs.len(), 144);
    for record in fixture["records"].as_array().unwrap() {
        let precision = record["precision"].as_str().unwrap();
        let encoding = if precision.ends_with("NON_LINEAR") {
            Encoding::NonlinearProfile
        } else if precision.ends_with("PERCEPTUAL") {
            Encoding::Perceptual
        } else {
            Encoding::Linear
        };
        let target = effective_profile_from_definition(Builtin::SGray.profile(), encoding).unwrap();
        let conversion = Transform::new(Builtin::Srgb.profile(), &target, Intent::RelativeColorimetric, false).unwrap();
        let mut maximum = 0f64;
        for (input, expected) in inputs.iter().zip(record["gray_alpha_f64_bits"].as_array().unwrap()) {
            let rgb: [f64; 3] = std::array::from_fn(|i| f64::from(f32::from_bits(input[i].as_u64().unwrap() as u32)));
            let mut actual = [0f64];
            conversion.try_eval_f64(&rgb, &mut actual).unwrap();
            let expected_gray = f64::from_bits(expected[0].as_u64().unwrap());
            let delta = (actual[0] - expected_gray).abs();
            maximum = maximum.max(delta);
            assert!(delta <= 2. / 255., "{precision} {rgb:?}: delta {delta}");
        }
        println!("{precision} independent RGB cube maximum {maximum} ({} /255)", maximum * 255.);
    }
}

#[test]
#[ignore = "24 MP release-only Color Exchange kernel benchmark; excludes CMS, tiles, IO and allocation"]
fn prepared_color_exchange_kernel_24mp() {
    let optimized = std::hint::black_box(!cfg!(debug_assertions));
    assert!(optimized, "run this benchmark with --release");
    let (_, operation) = color_exchange_member(Some("0:0"), [0.375, 0.5, 0.625, 0.75], [0.125, 0.75, 0.25, 0.25], [0.125, 0.1875, 0.25]).unwrap();
    let prepared = operation.prepare().unwrap();
    let pixels = [[0.375, 0.5, 0.625, 1.], [0.125, 0.75, 0.25, 0.], [0.5, 0.625, 0.375, 0.5], [0.875, 0.125, 0.5, -0.]];
    let mut output = [0.; 4];
    let started = std::time::Instant::now();
    for index in 0..24_000_000usize {
        let input = std::hint::black_box(&pixels[index % pixels.len()]);
        prepared.try_eval_pixel(input, 3, &mut output).unwrap();
        std::hint::black_box(output);
    }
    let elapsed = started.elapsed();
    assert_eq!(output[3].to_bits(), (-0f64).to_bits());
    println!("24 MP prepared RGB Color Exchange kernel: {:.3} ms; CMS/tiles/IO/allocation excluded", elapsed.as_secs_f64() * 1000.);
}
