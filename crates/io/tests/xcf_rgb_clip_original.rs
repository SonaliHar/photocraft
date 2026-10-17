#![cfg(feature = "corpus")]
//! Finite HDR RGB Clip originals; source and zero F32 selection mask remain exact.
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
#[test]
#[ignore = "requires reviewed independently authored finite HDR RGB Clip originals"]
fn hdr_clip_sources_limits_boolean_edits_native_and_fresh_projection() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("rgb-clip-hdr-evidence");
    let manifest = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "45f79892e29ebb9bd3ff07a9c6231063bfe4028e86e3a79562a7daab85fa9532");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hash(&fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap()), artifact["sha256"]);
    }
    for stage in ["source", "limits", "low-disabled"] {
        let bytes = fs::read(root.join(format!("clip.{stage}.xcf"))).unwrap();
        let source = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let state: Value = serde_json::from_slice(&fs::read(root.join(format!("clip.{stage}.fresh.state.json"))).unwrap()).unwrap();
        let doc = photocraft_io::import("clip.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let native = fs::read(root.join(state["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&native), state["layers"][0]["native"]["sha256"]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
        assert_eq!(source.layers[0].plane.as_ref().unwrap().bytes, native);
        let retained = doc.xcf_state.as_ref().unwrap();
        let node = &retained.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, "gegl:rgb-clip");
        assert_eq!(node.arguments.len(), 4);
        assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, "xcf-22-stock-rgb-clip-hdr-v1");
        for argument in &node.arguments {
            let expected = &state["layers"][0]["filters"][0]["config"][&argument.name];
            if argument.name.starts_with("clip-") {
                assert_eq!(argument.value, FilterValue::Bool(expected.as_bool().unwrap()));
            } else {
                assert_eq!(argument.value, FilterValue::Number(ExactNumber::Float32Bits((expected.as_f64().unwrap() as f32).to_bits())));
            }
        }
        let mask = source.layers[0].effects[0].mask.as_ref().unwrap();
        let plane = mask.plane.as_ref().unwrap();
        assert_eq!(plane.sample, photocraft_xcf::Sample::F32);
        assert!(plane.bytes.iter().all(|byte| *byte == 0));
        assert_eq!(retained.filter_masks.len(), 1);
        assert_eq!(retained.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), plane.bytes);
        let saved = photocraft_io::export(&doc, "clip.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("clip.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "clip.pcraft"));
        let projection = fs::read(root.join(state["projection"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&projection), state["projection"]["sha256"]);
        use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
        let profile = implicit_source_profile(doc.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, Encoding::Linear).unwrap();
        for candidate in [&doc, &reopened] {
            let image = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(image.px.len(), 48);
            assert_eq!(projection.len(), 48 * 4 * 8);
            for (index, (actual, reference)) in image.px.iter().zip(projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())).enumerate() {
                let mut encoded = [0.0; 3];
                convert_encoding_f64(&profile, Encoding::Linear, Encoding::NonlinearProfile, &actual[..3], &mut encoded).unwrap();
                assert_eq!(actual[3], 1.0);
                for channel in 0..3 {
                    let expected = f64::from_le_bytes(reference[channel * 8..channel * 8 + 8].try_into().unwrap());
                    assert!((encoded[channel] - expected).abs() <= 2.0 / 255.0, "{stage} pixel {index} channel {channel}");
                }
                assert_eq!(f64::from_le_bytes(reference[24..32].try_into().unwrap()), 1.0);
            }
        }
        assert_eq!(doc, before);
        assert_eq!(fs::read(root.join(format!("clip.{stage}.xcf"))).unwrap(), bytes);
    }
}
