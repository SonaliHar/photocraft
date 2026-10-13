#![cfg(feature = "corpus")]
use photocraft_algo::gimp::levels::{LevelsChannel, LevelsConfiguration};
use photocraft_cms::{Intent, Profile, Transform};
use std::path::PathBuf;

#[test]
fn repeated_levels_source_edit_trc_clamp_matrix_matches_independent_rgba() {
    compare_levels(false);
}

#[test]
#[ignore = "Perceptual unequal 65-node table ICC low-tone discrepancy remains unqualified; run explicitly for diagnosis"]
fn perceptual_unequal_gamma_levels_admission_diagnostic() {
    compare_levels(true);
}

fn compare_levels(perceptual: bool) {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("curve-config-matrix");
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).expect("stage full XCF corpus")).unwrap();
    for case in manifest["cases"].as_array().unwrap().iter().filter(|c| c["id"].as_str().unwrap().starts_with("levels-")) {
        let id = case["id"].as_str().unwrap();
        let trc = case["dimensions"]["filter_trc"].as_u64().unwrap();
        if (trc == 2) != perceptual {
            continue;
        }
        let ci = id.ends_with("10") || id.ends_with("11");
        let co = id.ends_with("01") || id.ends_with("11");
        for edit in [false, true] {
            let phase = if edit { "edit-source" } else { "source" };
            let requests = &case[if edit { "requested_edit_config" } else { "requested_source_config" }];
            let mut channels = [LevelsChannel::default(); 5];
            for request in requests.as_array().unwrap() {
                let index = request["channel"].as_u64().unwrap() as usize;
                let v = &request["requested"];
                channels[index] = LevelsChannel {
                    low_input: v["low-input"].as_f64().unwrap(),
                    high_input: v["high-input"].as_f64().unwrap(),
                    gamma: v["gamma"].as_f64().unwrap(),
                    low_output: v["low-output"].as_f64().unwrap(),
                    high_output: v["high-output"].as_f64().unwrap(),
                };
            }
            let config = LevelsConfiguration { channels, clamp_input: ci, clamp_output: co };
            let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{id}.{phase}.state.json"))).unwrap()).unwrap();
            assert_eq!(
                state["precision"],
                match trc {
                    0 => "GIMP_PRECISION_FLOAT_LINEAR",
                    1 => "GIMP_PRECISION_FLOAT_NON_LINEAR",
                    _ => "GIMP_PRECISION_FLOAT_PERCEPTUAL",
                }
            );

            let native = std::fs::read(root.join(format!("{id}.{phase}.0.native.bin"))).unwrap();
            assert_eq!(native.len(), 12 * 9 * 16);
            let effective = std::fs::read(root.join(state["effective_profile"]["file"].as_str().unwrap())).unwrap();
            let source_profile = photocraft_cms::xcf_color::effective_profile(
                Some(&effective),
                photocraft_cms::ColorSpace::Rgb,
                match trc {
                    0 => photocraft_cms::xcf_color::Encoding::Linear,
                    1 => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    _ => photocraft_cms::xcf_color::Encoding::Perceptual,
                },
            )
            .unwrap();
            let working = &source_profile;
            let to_working = Transform::new(&source_profile, working, Intent::RelativeColorimetric, false).unwrap();
            let reference =
                photocraft_codecs::decode(&std::fs::read(root.join(case[if edit { "reference_edit_render" } else { "reference" }].as_str().unwrap())).unwrap())
                    .unwrap();
            let reference_profile = Profile::parse(reference.icc.as_ref().unwrap()).unwrap();
            let to_display = Transform::new(working, &reference_profile, Intent::RelativeColorimetric, false).unwrap();
            let reference = reference.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U16);
            let mut max = [0.0_f64; 4];
            let mut sum = [0.0_f64; 4];
            for (raw, expected) in native.as_chunks::<16>().0.iter().zip(reference.data().as_chunks::<8>().0.iter()) {
                let src: Vec<f64> = raw.as_chunks::<4>().0.iter().map(|s| f64::from(f32::from_le_bytes(*s))).collect();
                let mut working_pixel = [0.0; 4];
                to_working.try_eval_f64(&src[..3], &mut working_pixel[..3]).unwrap();
                working_pixel[3] = src[3];
                let mut evaluated = [0.0; 4];
                config.try_eval_pixel(&working_pixel, 3, &mut evaluated).unwrap();
                let mut display = [0.0; 4];
                to_display.try_eval_f64(&evaluated[..3], &mut display[..3]).unwrap();
                display[3] = evaluated[3];
                // Independent PNG export settings explicitly disable save-transparent.
                if display[3] <= 0.0 {
                    display[..3].fill(0.0);
                }
                for c in 0..4 {
                    let want = f64::from(u16::from_ne_bytes(expected[c * 2..c * 2 + 2].try_into().unwrap())) / 65535.0;
                    let d = (display[c].clamp(0.0, 1.0) - want).abs() * 255.0;
                    max[c] = max[c].max(d);
                    sum[c] += d;
                }
            }
            println!("{id} edit={edit} max255={max:?} mean255={:?}", sum.map(|v| v / 108.0));
            assert!(max.iter().all(|v| *v <= 2.0), "{id} edit={edit}: {max:?}");
        }
    }
}
