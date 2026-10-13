#![cfg(feature = "corpus")]
//! Independent public Linear Float-working responses, without importer admission.
use photocraft_algo::gimp::point_mono_mixer::MonoMixer;
use photocraft_raster::Interrupt;
use serde_json::Value;
use std::path::PathBuf;

fn doubles(bytes: &[u8]) -> Vec<f64> {
    assert_eq!(bytes.len() % 8, 0);
    bytes.as_chunks::<8>().0.iter().map(|b| f64::from_le_bytes(*b)).collect()
}

#[test]
#[ignore = "local original Mono Mixer public signed-weight response controls"]
fn independently_observed_linear_signed_luminosity_and_zero_sum_math() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut pixels = 0;
    for family in ["mono-mixer-domain-response-v2", "mono-mixer-normalization-response"] {
        let dir = root.join(family);
        let records: Value = serde_json::from_slice(&std::fs::read(dir.join("observations.json")).unwrap()).unwrap();
        for row in records["records"].as_array().unwrap() {
            if row["model"] != "RGB" || row["precision"] == "DOUBLE_NON_LINEAR" {
                continue;
            }
            let weights = row["weights"].as_array().unwrap();
            let operation = MonoMixer {
                red: weights[0].as_f64().unwrap(),
                green: weights[1].as_f64().unwrap(),
                blue: weights[2].as_f64().unwrap(),
                preserve_luminosity: row["preserve"].as_bool().unwrap(),
            };
            let input = doubles(&std::fs::read(dir.join(row["before"]["layers"][0]["working"]["RGBA double"]["file"].as_str().unwrap())).unwrap());
            let reference = doubles(&std::fs::read(dir.join(row["baked"]["layers"][0]["working"]["RGBA double"]["file"].as_str().unwrap())).unwrap());
            for (input, expected) in input.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
                let mut output = [0.125_f64, 0.25, 0.5, 0.75];
                let result = operation.try_eval_pixel_with(input, &mut output, true, &Interrupt::NONE, &|_| Ok(()));
                if expected.iter().any(|v| !v.is_finite()) {
                    assert!(result.is_err());
                    assert_eq!(output, [0.125, 0.25, 0.5, 0.75]);
                } else {
                    result.unwrap_or_else(|e| panic!("{}: {e:?}", row["id"]));
                    for (actual, expected) in output.iter().zip(expected) {
                        assert_eq!(actual.to_bits(), expected.to_bits(), "{}", row["id"]);
                    }
                }
                pixels += 1;
            }
        }
    }
    assert_eq!(pixels, 896);
}

#[test]
#[ignore = "local independent Double native alpha boundary controls"]
fn independent_double_native_active_alpha_uses_float_working_boundary() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-signed-precision-evidence");
    let records: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut pixels = 0;
    for row in records["records"].as_array().unwrap() {
        if row["precision"] != "DOUBLE_LINEAR" || row["enabled"] != true {
            continue;
        }
        let weights = row["weights"].as_array().unwrap();
        let operation = MonoMixer {
            red: weights[0].as_f64().unwrap(),
            green: weights[1].as_f64().unwrap(),
            blue: weights[2].as_f64().unwrap(),
            preserve_luminosity: row["preserve"].as_bool().unwrap(),
        };
        let input = doubles(&std::fs::read(root.join(row["current"]["layers"][0]["working"]["RGBA double"]["file"].as_str().unwrap())).unwrap());
        let reference = doubles(&std::fs::read(root.join(row["baked"]["layers"][0]["working"]["RGBA double"]["file"].as_str().unwrap())).unwrap());
        for (input, expected) in input.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
            let mut output = [0.; 4];
            operation.try_eval_pixel_with(input, &mut output, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
            assert_eq!(output[3].to_bits(), expected[3].to_bits(), "{}", row["id"]);
            pixels += 1;
        }
    }
    assert_eq!(pixels, 288);
}
