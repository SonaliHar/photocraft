#![cfg(not(target_arch = "wasm32"))]

use photocraft_cms::Builtin;
use photocraft_compose::gimp_red_eye::RedEyeCandidate;
use photocraft_doc::SampleEncoding;
use photocraft_raster::Interrupt;
use serde_json::Value;
#[path = "../../xcf/tests/common/red_eye_geometry_evidence.rs"]
mod evidence;
use std::path::Path;

fn read(root: &Path, descriptor: &Value) -> Vec<u8> {
    evidence::artifact(root, descriptor).unwrap()
}

fn doubles(bytes: &[u8]) -> Vec<f64> {
    let (chunks, tail) = bytes.as_chunks::<8>();
    assert!(tail.is_empty());
    chunks.iter().map(|v| f64::from_le_bytes(*v)).collect()
}

#[test]
#[ignore = "fresh independent GIMP owner geometry packet; explicit local witness"]
fn fresh_six_red_eye_whole_owner_and_clipped_display() {
    let witness = evidence::selected(env!("CARGO_MANIFEST_DIR")).unwrap();
    let packet = witness.packet.clone();
    for geometry in ["negative-owner", "tile-crossing-owner"] {
        for control in ["source", "edit", "disabled"] {
            let stem = format!("{geometry}.{control}");
            let state = |suffix: &str| -> Value { witness.state(&stem, suffix).unwrap() };
            let current = state("reopened");
            let baked = state("baked");
            assert_eq!(current["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
            assert!(current["profiles"]["assigned"].is_null());
            witness.assert_implicit_profile(&current).unwrap();
            assert_eq!(current["owner"]["native_format"], "R'G'B'A float");
            assert_eq!(current["owner"]["size"], baked["owner"]["size"]);
            assert_eq!(current["owner"]["offset"], baked["owner"]["offset"]);
            let input_bytes = read(&packet, &current["owner"]["native"]);
            let reference_bytes = read(&packet, &baked["owner"]["native"]);
            let (input_chunks, tail) = input_bytes.as_chunks::<4>();
            assert!(tail.is_empty());
            let input: Vec<f64> = input_chunks.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
            let (ref_chunks, tail) = reference_bytes.as_chunks::<4>();
            assert!(tail.is_empty());
            let reference: Vec<f64> = ref_chunks.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
            let threshold = current["filters"][0]["properties"][0]["value"]["value"].as_f64().unwrap();
            let bits = if control == "edit" { 0x3f2c_cccd } else { 0x3ecc_cccd };
            assert_eq!(threshold, f64::from(f32::from_bits(bits)));
            let enabled = current["filters"][0]["visible"].as_bool().unwrap();
            assert_eq!(enabled, control != "disabled");
            let op = RedEyeCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, threshold).unwrap();
            let working = doubles(&read(&packet, &current["owner"]["working"]["R'G'B'A double"]));
            let mut output = Vec::with_capacity(input.len());
            let mut native_max = 0f64;
            let mut working_max = 0f64;
            let mut alpha_max = 0f64;
            let mut hidden = 0usize;
            let (pixels, tail) = input.as_chunks::<4>();
            assert!(tail.is_empty());
            for (i, pixel) in pixels.iter().enumerate() {
                for (a, b) in op.working_input(pixel).unwrap().iter().zip(&working[i * 4..i * 4 + 4]) {
                    working_max = working_max.max((a - b).abs());
                }
                let result = op.apply_with(pixel, enabled, &Interrupt::NONE, &|_| Ok(())).unwrap();
                if !enabled {
                    assert_eq!(result.map(f64::to_bits), pixel.map(f64::to_bits));
                }
                let stored = result.map(|v| f64::from(v as f32));
                if stored[3] == 0. {
                    hidden += 1;
                }
                for (a, b) in stored.iter().zip(&reference[i * 4..i * 4 + 4]) {
                    native_max = native_max.max((a - b).abs());
                }
                alpha_max = alpha_max.max((stored[3] - reference[i * 4 + 3]).abs());
                output.push(stored);
            }
            let width = current["owner"]["size"][0].as_i64().unwrap();
            let height = current["owner"]["size"][1].as_i64().unwrap();
            let ox = current["owner"]["offset"][0].as_i64().unwrap();
            let oy = current["owner"]["offset"][1].as_i64().unwrap();
            let cw = current["canvas"][0].as_i64().unwrap();
            let ch = current["canvas"][1].as_i64().unwrap();
            assert_eq!(output.len() as i64, width * height);
            let projection = doubles(&read(&packet, &current["projection"]["formats"]["R'G'B'A double"]));
            assert_eq!(projection.len() as i64, cw * ch * 4);
            let mut display_max = 0f64;
            let mut display_alpha = 0f64;
            let mut offcanvas = 0usize;
            for y in 0..height {
                for x in 0..width {
                    if !(0..cw).contains(&(x + ox)) || !(0..ch).contains(&(y + oy)) {
                        offcanvas += 1;
                    }
                }
            }
            for y in 0..ch {
                for x in 0..cw {
                    let lx = x - ox;
                    let ly = y - oy;
                    let actual = if (0..width).contains(&lx) && (0..height).contains(&ly) { output[(ly * width + lx) as usize] } else { [0.; 4] };
                    let expected = &projection[((y * cw + x) * 4) as usize..((y * cw + x) * 4 + 4) as usize];
                    for c in 0..3 {
                        display_max = display_max.max((actual[c] * actual[3] - expected[c] * expected[3]).abs());
                    }
                    display_alpha = display_alpha.max((actual[3] - expected[3]).abs());
                }
            }
            println!(
                "{stem}: owner={width}x{height}@{ox},{oy}; offcanvas={offcanvas}; alpha0={hidden}; working={working_max}; native={native_max}; alpha={alpha_max}; premult_display={display_max}; display_alpha={display_alpha}"
            );
            assert!(offcanvas > 0 && hidden > 0);
            assert!(working_max <= 1e-6);
            assert!(native_max <= 2. / 255.);
            assert!(alpha_max <= 1. / 65535.);
            assert!(display_max <= 2. / 255.);
            assert!(display_alpha <= 1. / 65535.);
        }
    }
}
