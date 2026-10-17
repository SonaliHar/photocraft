#![cfg(feature = "corpus")]
//! Actual Normal operators and masks, including exact sub-f32 saved double samples.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn read(root: &Path, blob: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), blob["sha256"]);
    if let Some(length) = blob["bytes"].as_u64() {
        assert_eq!(bytes.len(), length as usize);
    }
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished own assigned Linear Normal mask originals"]
fn assigned_linear_normal_native_profile_and_faithful_png() {
    verify_packet("normal-mask-assigned-linear-evidence", include_str!("../../compose/tests/gimp_normal_mask_assigned_linear_hashes.json"), false, true, false);
}
fn native_bytes(bytes: Vec<u8>, half: bool) -> Vec<u8> {
    if !half {
        return bytes;
    }
    assert_eq!(bytes.len() % 2, 0);
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| chunk.as_slice())
        .flat_map(|raw| photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(raw.try_into().unwrap())).to_ne_bytes())
        .collect()
}
fn verify_packet(directory: &str, hashes: &str, linear: bool, assigned: bool, integer: bool) {
    let perceptual = false;
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(directory);
    let hashes: Value = serde_json::from_str(hashes).unwrap();
    let raw = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&raw), hashes["observations.json"]);
    let observations: Value = serde_json::from_slice(&raw).unwrap();
    let raw = fs::read(root.join("faithful-observations.json")).unwrap();
    assert_eq!(hash(&raw), hashes["faithful-observations.json"]);
    let faithful: Value = serde_json::from_slice(&raw).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap() {
        let half = row["sample_type"] == "half";
        let linear = linear
            || (integer || half) && row["precision"].as_str().unwrap().ends_with("_LINEAR") && !row["precision"].as_str().unwrap().ends_with("_NON_LINEAR");
        let id = row["id"].as_str().unwrap();
        let path = root.join(row["source_file"].as_str().unwrap());
        let bytes = fs::read(&path).unwrap();
        assert_eq!(hash(&bytes), row["source_sha256"]);
        let doc = photocraft_io::import("normal-mask.xcf", &bytes).unwrap_or_else(|error| panic!("{id}: {error}")).document;
        if assigned {
            let icc = read(&root, &row["assigned_icc"]);
            assert_eq!(icc, read(&root, &row["authored_icc"]));
            let definition = photocraft_cms::Profile::parse(&icc).unwrap();
            let effective = photocraft_cms::xcf_color::effective_profile_from_definition(&definition, photocraft_cms::xcf_color::Encoding::Linear).unwrap();
            assert_eq!(doc.icc_profile.as_ref().unwrap().as_slice(), effective.to_bytes().as_slice());
            assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::Linear));
            assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, if row["id"].as_str().unwrap().contains("f64") { 700 } else { 600 });
            assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice(), icc);
            assert_eq!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc);
        }
        let before = doc.clone();
        for observed in row["layers"].as_array().unwrap() {
            let layer = doc.layers.iter().find(|layer| layer.name == observed["name"].as_str().unwrap()).unwrap();
            let owner = photocraft_doc::item_identity::ItemRef::Layer(layer.id);
            let state = doc.xcf_state.as_ref().unwrap();
            let item = state.items.iter().find(|item| item.item == owner).unwrap();
            let bounds = item.raster_bounds.unwrap();
            assert_eq!([bounds.x0, bounds.y0], [observed["offset"][0].as_i64().unwrap() as i32, observed["offset"][1].as_i64().unwrap() as i32]);
            let native = native_bytes(read(&root, &observed["native"]), half);
            assert_eq!(layer.surface().unwrap().try_read_interleaved(bounds).unwrap(), native, "{id} source samples");
            if let Some(mask) = observed.get("mask").filter(|mask| !mask.is_null()) {
                let retained = state.items.iter().find_map(|item| item.mask.as_ref().filter(|mask| mask.owner == owner)).unwrap();
                assert_eq!(retained.bounds, bounds);
                assert_eq!(retained.offset, [bounds.x0, bounds.y0]);
                assert_eq!(retained.outside_f64_bits, 0.0_f64.to_bits());
                assert!(retained.enabled);
                assert!(!retained.shown && !retained.editing);
                assert_eq!(
                    layer.mask.as_ref().unwrap().surface.try_read_interleaved(bounds).unwrap(),
                    native_bytes(read(&root, &mask["native"]), half),
                    "{id} mask samples"
                );
                let policy = item.composition.as_ref().unwrap();
                assert_eq!(policy.source_mode, 28);
                assert_eq!(policy.stored_operator, observed["spaces"]["composite_mode"].as_i64().unwrap() as i32);
            }
        }
        let saved = photocraft_io::export(&doc, "normal-mask.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("normal-mask.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "normal-mask.pcraft"));
        let reference = read(&root, &row["projection"]["nonlinear_double"]);
        let faithful_row = faithful["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
        let png = photocraft_codecs::decode(&read(&root, &faithful_row["reference"])).unwrap();
        let channels = if row["model"] == "gray" { 2 } else { 4 };
        let color_tolerance =
            if assigned || integer || half || perceptual && matches!(doc.depth, photocraft_doc::SampleType::U8 | photocraft_doc::SampleType::U16) {
                2.0 / 255.0
            } else {
                1.0 / 65535.0
            };
        let alpha_tolerance = if half {
            2.0_f64.powi(-12) + 1.0 / 65535.0
        } else if integer || perceptual && matches!(doc.depth, photocraft_doc::SampleType::U8 | photocraft_doc::SampleType::U16) {
            2.0 / 255.0
        } else {
            1.0 / 65535.0
        };
        if assigned {
            assert_eq!(png.icc.as_ref().unwrap().as_slice(), doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice());
        }
        for candidate in [&doc, &reopened] {
            let actual = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            let mut nonlinear = actual.clone();
            if assigned {
                let profile = photocraft_cms::Profile::parse(candidate.icc_profile.as_ref().unwrap()).unwrap();
                let target = if channels == 2 { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
                let transform = photocraft_cms::Transform::new(&profile, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
                for pixel in &mut nonlinear.px {
                    let mut output = [0.0; 3];
                    transform.try_eval_f64(&pixel[..channels - 1], &mut output[..channels - 1]).unwrap();
                    pixel[..channels - 1].copy_from_slice(&output[..channels - 1]);
                }
            }
            if linear || perceptual {
                use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
                let space = if channels == 2 { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
                let encoding = Encoding::Linear;
                let source_profile = implicit_source_profile(candidate.icc_profile.as_ref().unwrap(), space, encoding).unwrap();
                for pixel in &mut nonlinear.px {
                    let mut output = [0.0; 3];
                    convert_encoding_f64(&source_profile, encoding, Encoding::NonlinearProfile, &pixel[..channels - 1], &mut output[..channels - 1]).unwrap();
                    pixel[..channels - 1].copy_from_slice(&output[..channels - 1]);
                }
                let png_profile = photocraft_cms::Profile::parse(png.icc.as_ref().unwrap()).unwrap();
                let is_linear = |curve: &photocraft_cms::curve::Curve| (curve.eval64(0.375) - 0.375).abs() < 1e-9;
                if linear && channels == 2 {
                    assert!(is_linear(png_profile.gray_trc.as_ref().unwrap()));
                } else if linear {
                    assert!(png_profile.trc.as_ref().unwrap().iter().all(is_linear));
                }
            }
            let current = photocraft_cms::Profile::parse(candidate.icc_profile.as_ref().unwrap()).unwrap();
            let embedded = photocraft_cms::Profile::parse(png.icc.as_ref().unwrap()).unwrap();
            let to_png = photocraft_cms::Transform::new(&current, &embedded, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
            let mut png_encoded = actual.clone();
            for pixel in &mut png_encoded.px {
                let mut out = [0.0; 3];
                to_png.try_eval_f64(&pixel[..channels - 1], &mut out[..channels - 1]).unwrap();
                pixel[..channels - 1].copy_from_slice(&out[..channels - 1]);
            }
            for (index, ((pixel, encoded), native)) in actual.px.iter().zip(&nonlinear.px).zip(reference.chunks_exact(channels * 8)).enumerate() {
                let png_pixel = png_encoded.px[index];
                let alpha = f64::from_le_bytes(native[(channels - 1) * 8..channels * 8].try_into().unwrap());
                let png_alpha = f64::from(png.get(index as u32 % 16, index as u32 / 16, channels - 1));
                assert!((pixel[3] - alpha).abs() <= alpha_tolerance, "{id} native alpha");
                assert!((pixel[3] - png_alpha).abs() <= alpha_tolerance, "{id} PNG alpha");
                for channel in 0..channels - 1 {
                    let native = f64::from_le_bytes(native[channel * 8..channel * 8 + 8].try_into().unwrap());
                    let png = f64::from(png.get(index as u32 % 16, index as u32 / 16, channel));
                    if alpha > 0.0 {
                        assert!((encoded[channel] - native).abs() <= 1.0 / 65535.0, "{id} native visible color");
                    }
                    if png_alpha > 0.0 {
                        assert!((png_pixel[channel] - png).abs() <= color_tolerance, "{id} PNG visible color");
                    }
                    assert!((encoded[channel] * encoded[3] - native * alpha).abs() <= 1.0 / 65535.0, "{id} native premultiplied color");
                    assert!((png_pixel[channel] * png_pixel[3] - png * png_alpha).abs() <= color_tolerance, "{id} PNG premultiplied color");
                }
            }
        }
        assert_eq!(doc, before);
        assert_eq!(fs::read(path).unwrap(), bytes);
        count += 1;
    }
    assert_eq!(
        count,
        if perceptual {
            48
        } else if integer {
            96
        } else {
            32
        }
    );
}
