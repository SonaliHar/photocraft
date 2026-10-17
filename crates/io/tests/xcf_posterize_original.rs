#![cfg(feature = "corpus")]
//! Typed Signed32 Posterize originals, including disabled activation.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
#[test]
#[ignore = "requires reviewed unpublished independently authored Posterize originals"]
fn posterize_source_activation_native_and_independent_projection() {
    scoped_point_original(
        "gimp-posterize",
        "gimp:posterize",
        ["4eac864e360186bb24174f091b7b84d424e804177fe75c0330d6e888b801968a", "7e6d944a2f97d404d0db01a45c8c4bbf8cbb220c829a2c06a35149b41c7c8c37"],
    );
}
#[test]
#[ignore = "requires reviewed unpublished independently authored Value Invert originals"]
fn value_invert_source_activation_native_and_independent_projection() {
    scoped_point_original(
        "gegl-value-invert",
        "gegl:value-invert",
        ["213cf8594acc1d90cd71718f87e70a080b1c31694798ec513bc429d65d03feb9", "f449e1cfb3028b13f48cfe0550e01cc1bc42422450eec44fc4916f773b71bb49"],
    );
}
fn scoped_point_original(directory: &str, operation: &str, proof_hashes: [&str; 2]) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations").join(directory);
    for (proof_name, proof_hash) in ["probe.json", "probe-activation.json"].into_iter().zip(proof_hashes) {
        let proof = fs::read(root.join(proof_name)).unwrap();
        assert_eq!(hash(&proof), proof_hash);
        let proof: Value = serde_json::from_slice(&proof).unwrap();
        let record = &proof["cases"][0];
        for (input, input_hash, reference, reference_hash, state_name) in [
            ("input", "input_sha256", "reference", "reference_sha256", "state_reopened"),
            ("reference_edit", "reference_edit_sha256", "reference_edit_render", "reference_edit_render_sha256", "state_edited"),
        ] {
            let source_path = root.join(record[input].as_str().unwrap());
            let bytes = fs::read(&source_path).unwrap();
            assert_eq!(hash(&bytes), record[input_hash]);
            let parsed = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let imported = photocraft_io::import("posterize.xcf", &bytes).unwrap();
            assert!(imported.warnings.iter().all(|warning| !warning.contains("parasite") || warning.contains("retained as source metadata")));
            let doc = imported.document;
            let state = doc.xcf_state.as_ref().unwrap();
            assert!(parsed.raw_properties.iter().any(|property| property.tag == 21));
            for property in &parsed.raw_properties {
                let identity = format!("{}@{}:{}", property.tag, property.offset, property.declared_bytes);
                let retained = state
                    .opaque
                    .iter()
                    .find(|record| record.owner.is_none() && record.namespace == "gimp/xcf/source-property" && record.identity == identity)
                    .unwrap();
                assert_eq!(retained.bytes.as_slice(), property.payload);
                assert_eq!(retained.blake3, blake3::hash(&property.payload).to_hex().to_string());
                assert!(!retained.rendering_required);
            }
            let before = doc.clone();
            let expected = &record[state_name]["layers"][0];
            let native = fs::read(root.join(expected["native"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&native), expected["native"]["sha256"]);
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
            let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(node.operation.name, operation);
            assert_eq!(node.enabled, expected["filters"][0]["visible"].as_bool().unwrap());
            if operation == "gimp:posterize" {
                assert_eq!(
                    node.arguments[0].value,
                    photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed {
                        width: 32,
                        value: expected["filters"][0]["config"]["levels"].as_i64().unwrap()
                    })
                );
            } else {
                assert!(node.arguments.is_empty());
            }
            let png = fs::read(root.join(record[reference].as_str().unwrap())).unwrap();
            assert_eq!(hash(&png), record[reference_hash]);
            let reference = photocraft_codecs::decode(&png).unwrap();
            let saved = photocraft_io::export(&doc, "posterize.pcraft", &Default::default()).unwrap();
            let reopened = photocraft_io::import("posterize.pcraft", &saved.bytes).unwrap().document;
            assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "posterize.pcraft"));
            for candidate in [&doc, &reopened] {
                let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
                for (index, pixel) in rendered.px.iter().enumerate() {
                    for (channel, value) in pixel.iter().enumerate() {
                        assert!((value - f64::from(reference.get(index as u32 % 32, index as u32 / 32, channel))).abs() <= 2.0 / 255.0);
                    }
                }
            }
            assert_eq!(doc, before);
            assert_eq!(fs::read(source_path).unwrap(), bytes);
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished independently authored Posterize originals"]
fn native_posterize_rejects_unqualified_owner_and_typed_parameter_edits() {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations/gimp-posterize");
    let proof: Value = serde_json::from_slice(&fs::read(root.join("probe.json")).unwrap()).unwrap();
    let bytes = fs::read(root.join(proof["cases"][0]["input"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), "22dd717b5ca0863521f20791705746d104ee6a35fa90806759bb47d96c16b9c3");
    let original = photocraft_io::import("posterize.xcf", &bytes).unwrap().document;
    let owner = original.xcf_state.as_ref().unwrap().filter_stacks[0].owner;
    let owner_index = original.xcf_state.as_ref().unwrap().items.iter().position(|item| item.item == owner).unwrap();
    photocraft_compose::gimp_render::try_render(&original, original.bounds(), &mut |_| Ok(())).unwrap();
    for change in 0..10 {
        let mut invalid = original.clone();
        match change {
            0 => invalid.layers[0].opacity = 0.5,
            1 => invalid.size.width = 33,
            2 => invalid.source_color_context.as_mut().unwrap().original_icc = invalid.icc_profile.clone(),
            3 => invalid.xcf_state.as_mut().unwrap().items[owner_index].raster_bounds = Some(photocraft_doc::Rect::new(1, 0, 33, 32)),
            4 => invalid.xcf_state.as_mut().unwrap().items[owner_index].composition.as_mut().unwrap().stored_blend_space = 2,
            5 => invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "9:0".into(),
            6 => {
                invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                    FilterValue::Number(ExactNumber::Float32Bits(3.0_f32.to_bits()))
            }
            7 => {
                invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                    FilterValue::Number(ExactNumber::Signed { width: 64, value: 3 })
            }
            8 => {
                invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                    FilterValue::Number(ExactNumber::Signed { width: 32, value: 1 })
            }
            _ => {
                invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                    FilterValue::Number(ExactNumber::Signed { width: 32, value: 257 })
            }
        }
        let before = invalid.clone();
        assert!(photocraft_compose::gimp_render::try_render(&invalid, invalid.bounds(), &mut |_| Ok(())).is_err(), "variant {change}");
        assert_eq!(invalid, before);
    }
}
