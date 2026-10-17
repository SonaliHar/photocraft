#![cfg(feature = "corpus")]
//! Independently authored false-luminosity RGB mixer activation and weight edits.
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn verify(root: &Path, input: &str, state: &Value, png: Option<&str>, projection: Option<&str>) {
    let path = root.join(input);
    let bytes = fs::read(&path).unwrap();
    let doc = photocraft_io::import("mixer.xcf", &bytes).unwrap().document;
    let original = doc.clone();
    let layer = &state["layers"][0];
    let native = fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&native), layer["native"]["sha256"]);
    assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.name, "gegl:mono-mixer");
    assert_eq!(node.enabled, layer["filters"][0]["visible"].as_bool().unwrap());
    assert_eq!(node.arguments.len(), 4);
    for argument in &node.arguments {
        let expected = &layer["filters"][0]["config"][&argument.name];
        if argument.name == "preserve-luminosity" {
            assert_eq!(expected, false);
            assert_eq!(argument.value, FilterValue::Bool(false));
        } else {
            assert_eq!(argument.value, FilterValue::Number(ExactNumber::Float32Bits((expected.as_f64().unwrap() as f32).to_bits())));
        }
    }
    let saved = photocraft_io::export(&doc, "mixer.pcraft", &Default::default()).unwrap();
    let reopened = photocraft_io::import("mixer.pcraft", &saved.bytes).unwrap().document;
    assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "mixer.pcraft"));
    let png = png.map(|name| photocraft_codecs::decode(&fs::read(root.join(name)).unwrap()).unwrap());
    let projection: Option<Vec<f64>> = projection.map(|name| {
        fs::read(root.join(name))
            .unwrap()
            .as_chunks::<8>()
            .0
            .iter()
            .map(|chunk| chunk.as_slice())
            .map(|bytes| f64::from_le_bytes(bytes.try_into().unwrap()))
            .collect()
    });
    if let Some(projection) = &projection {
        assert_eq!(projection.len(), 32 * 32 * 4);
    }
    for candidate in [&doc, &reopened] {
        let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
        for (index, pixel) in rendered.px.iter().enumerate() {
            for (channel, value) in pixel.iter().enumerate() {
                if let Some(reference) = &png {
                    assert!((value - f64::from(reference.get(index as u32 % 32, index as u32 / 32, channel))).abs() <= 2.0 / 255.0);
                }
                if let Some(reference) = &projection {
                    assert!((value - reference[index * 4 + channel]).abs() <= 2.0 / 255.0, "{input} pixel {index} channel {channel}");
                }
            }
        }
    }
    assert_eq!(doc, original);
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
#[ignore = "requires reviewed unpublished Mono Mixer activation originals"]
fn mixer_activation_native_and_independent_projection() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations/gegl-mono-mixer");
    let bytes = fs::read(root.join("probe-activation.json")).unwrap();
    assert_eq!(hash(&bytes), "d68fd9e7289fc15c0f22263a34619af70c7c4e8b202479f0ab393a49fb5aac53");
    let proof: Value = serde_json::from_slice(&bytes).unwrap();
    let case = &proof["cases"][0];
    for (input, state, png) in [("input", "state_reopened", "reference"), ("reference_edit", "state_edited", "reference_edit_render")] {
        assert_eq!(hash(&fs::read(root.join(case[input].as_str().unwrap())).unwrap()), case[format!("{input}_sha256")]);
        assert_eq!(hash(&fs::read(root.join(case[png].as_str().unwrap())).unwrap()), case[format!("{png}_sha256")]);
        verify(&root, case[input].as_str().unwrap(), &case[state], Some(case[png].as_str().unwrap()), None);
    }
    let ordinary: Value = serde_json::from_slice(&fs::read(root.join("probe.json")).unwrap()).unwrap();
    let true_edit = root.join(ordinary["cases"][0]["reference_edit"].as_str().unwrap());
    assert!(photocraft_io::import("true.xcf", &fs::read(true_edit).unwrap()).is_err());
}

#[test]
#[ignore = "requires reviewed unpublished Mono Mixer independent weight edits"]
fn mixer_weight_originals_native_and_fresh_projection() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-edits-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "37935d87fac7c0e8aed14f326c4c39eeedd2e450fb6a7074d90c75d34dcb84f2");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hash(&fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap()), artifact["sha256"]);
    }
    for stage in ["source", "weights"] {
        let state: Value = serde_json::from_slice(&fs::read(root.join(format!("mono.{stage}.fresh.state.json"))).unwrap()).unwrap();
        verify(&root, &format!("mono.{stage}.xcf"), &state, None, Some(&format!("mono.{stage}.fresh.projection.bin")));
    }
}
