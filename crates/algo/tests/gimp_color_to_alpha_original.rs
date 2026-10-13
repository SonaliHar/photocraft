//! Original typed-color controls; no authoring implementation access.
use photocraft_algo::gimp::color_to_alpha::ColorToAlpha;

#[test]
#[ignore = "24 MP release-only Color to Alpha kernel benchmark; excludes CMS, tiles, IO and allocation"]
fn color_to_alpha_kernel_24mp() {
    let optimized = std::hint::black_box(!cfg!(debug_assertions));
    assert!(optimized, "run this benchmark with --release");
    let operation = ColorToAlpha { color: [0.375, 0.5, 0.625, 0.75], transparency: 0.125, opacity: 0.5 };
    let pixels = [[0.375, 0.5, 0.625, 1.], [0.125, 0.75, 0.25, 0.], [0.5, 0.625, 0.375, 0.5], [0.875, 0.125, 0.5, 1.]];
    let mut output = [0.; 4];
    let started = std::time::Instant::now();
    for index in 0..24_000_000usize {
        operation.try_eval_pixel(std::hint::black_box(&pixels[index % pixels.len()]), 3, &mut output).unwrap();
        std::hint::black_box(output);
    }
    println!("24 MP RGB Color to Alpha kernel: {:.3} ms; CMS/tiles/IO/allocation excluded", started.elapsed().as_secs_f64() * 1000.);
    assert!(output.iter().all(|value| value.is_finite()));
}

#[test]
#[ignore = "local unpublished Color to Alpha working controls"]
fn original_working_cube_preserves_source_and_produces_observed_alpha() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-xcf-probes/color-to-alpha-working-controls.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(value["records"].as_array().unwrap().len(), 108);
    let mut worst = 0f64;
    for r in value["records"].as_array().unwrap() {
        let operation = ColorToAlpha {
            color: std::array::from_fn(|i| f32::from_bits(r["color_bits"][i].as_u64().unwrap() as u32)),
            transparency: f32::from_bits(r["threshold_bits"][0].as_u64().unwrap() as u32),
            opacity: f32::from_bits(r["threshold_bits"][1].as_u64().unwrap() as u32),
        };
        for (source, expected) in r["input_f64_bits"].as_array().unwrap().iter().zip(r["expected_f64_bits"].as_array().unwrap()) {
            let input: [f64; 4] = std::array::from_fn(|i| f64::from_bits(source[i].as_u64().unwrap()));
            let expected: [f64; 4] = std::array::from_fn(|i| f64::from_bits(expected[i].as_u64().unwrap()));
            let original = input;
            let mut output = [7.; 4];
            operation.try_eval_pixel(&input, 3, &mut output).unwrap();
            assert_eq!(input, original);
            assert!((output[3] - expected[3]).abs() <= 1. / 65535., "{} alpha {:?} != {:?}", r["id"], output, expected);
            for c in 0..3 {
                let delta = (output[c] * output[3] - expected[c] * expected[3]).abs();
                worst = worst.max(delta);
                assert!(delta <= 2. / 255., "{} premult delta {}", r["id"], delta * 255.);
            }
        }
    }
    println!("Color to Alpha working108 premult max {}/255", worst * 255.);
}

#[test]
#[ignore = "local unpublished independently authored Color to Alpha boundary controls"]
fn independently_tied_axes_and_general_holdouts_match_operating_alpha() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-xcf-probes/color-to-alpha-boundary-controls.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(value["records"].as_array().unwrap().len(), 23);
    let mut worst = 0f64;
    let mut unassociated = 0f64;
    for r in value["records"].as_array().unwrap() {
        let operation = ColorToAlpha {
            color: std::array::from_fn(|i| f32::from_bits(r["color_bits"][i].as_u64().unwrap() as u32)),
            transparency: f32::from_bits(r["threshold_bits"][0].as_u64().unwrap() as u32),
            opacity: f32::from_bits(r["threshold_bits"][1].as_u64().unwrap() as u32),
        };
        for (source, expected) in r["input_f64_bits"].as_array().unwrap().iter().zip(r["expected_f64_bits"].as_array().unwrap()) {
            let input: [f64; 4] = std::array::from_fn(|i| f64::from_bits(source[i].as_u64().unwrap()));
            let expected: [f64; 4] = std::array::from_fn(|i| f64::from_bits(expected[i].as_u64().unwrap()));
            let mut output = [7.; 4];
            operation.try_eval_pixel(&input, 3, &mut output).unwrap();
            assert_eq!(output[3], expected[3], "{} operating alpha", r["id"]);
            for c in 0..3 {
                unassociated = unassociated.max((output[c] - expected[c]).abs());
                let delta = (output[c] * output[3] - expected[c] * expected[3]).abs();
                worst = worst.max(delta);
                assert!(delta <= 2. / 255., "{} premult {} /255", r["id"], delta * 255.);
            }
        }
    }
    println!("23 independent boundary controls: premult {}/255; unassociated delta {}", worst * 255., unassociated);
}
