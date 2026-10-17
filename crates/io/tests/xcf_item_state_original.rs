//! Independently authored item-state witnesses; opt in while the full corpus is local.
use photocraft_doc::{
    Document,
    item_identity::ItemRef,
    xcf_editing::{ItemFamily, ItemSetRule},
};
use photocraft_raster::Interrupt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full")
}
fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn checked(path: &Path, hash: &Value) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), hash.as_str().unwrap(), "{}", path.display());
    bytes
}
fn names(doc: &Document, ids: &[ItemRef]) -> Vec<String> {
    let state = doc.xcf_state.as_ref().unwrap();
    let mut names: Vec<_> = ids.iter().map(|id| state.items.iter().find(|item| item.item == *id).unwrap().name.clone()).collect();
    names.sort();
    names
}
fn observed_names(value: &Value) -> Vec<String> {
    let mut names: Vec<_> = value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    names.sort();
    names
}
fn assert_state(doc: &Document, reference: &Value, native_root: &Path) {
    let state = doc.xcf_state.as_ref().unwrap();
    state.validate(doc).unwrap();
    for family in ["layers", "channels", "paths"] {
        for observed in reference[family].as_array().unwrap() {
            let tattoo = observed["tattoo"].as_u64().unwrap() as u32;
            let item = state.items.iter().find(|item| item.source_tattoo == Some(tattoo)).unwrap();
            assert_eq!(item.name, observed["name"].as_str().unwrap());
            assert_eq!(item.visible, observed["visible"].as_bool().unwrap(), "{}", item.name);
            if let Some(locks) = observed.get("locks") {
                assert_eq!(item.locks.content, locks["content"].as_bool().unwrap(), "{}", item.name);
                assert_eq!(item.locks.position, locks["position"].as_bool().unwrap(), "{}", item.name);
                assert_eq!(item.locks.visibility, locks["visibility"].as_bool().unwrap(), "{}", item.name);
            }
            if family == "layers" {
                assert!(matches!(item.item, ItemRef::Layer(_)));
                assert_eq!(item.locks.alpha, observed["locks"]["alpha"].as_bool().unwrap());
            } else if family == "channels" {
                let ItemRef::Channel(id) = item.item else { panic!("channel identity") };
                let channel = doc.channels.iter().find(|channel| channel.id == id).unwrap();
                if let Some(native) = observed.get("native") {
                    let expected = checked(&native_root.join(native["file"].as_str().unwrap()), &native["sha256"]);
                    assert_eq!(channel.surface.try_read_interleaved(item.raster_bounds.unwrap()).unwrap(), expected, "{}", item.name);
                }
            } else {
                let ItemRef::Path(id) = item.item else { panic!("path identity") };
                let path = doc.paths.iter().find(|path| path.id == id).unwrap();
                assert_eq!(path.path.subpaths.len(), observed["strokes"].as_array().unwrap().len());
                for (stroke, expected) in path.path.subpaths.iter().zip(observed["strokes"].as_array().unwrap()) {
                    assert_eq!(stroke.closed, expected["closed"].as_bool().unwrap());
                    let actual: Vec<_> =
                        stroke.knots.iter().flat_map(|k| [k.in_ctrl.x, k.in_ctrl.y, k.anchor.x, k.anchor.y, k.out_ctrl.x, k.out_ctrl.y]).collect();
                    let expected: Vec<_> = expected["points"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                    assert_eq!(actual, expected, "{}", item.name);
                }
            }
        }
    }
    let mut expected = observed_names(&reference["selected_layers"]);
    expected.extend(observed_names(&reference["selected_channels"]));
    expected.sort();
    assert_eq!(names(doc, &state.selection.selected), expected);
    assert_eq!(names(doc, &state.selection.selected_paths), observed_names(&reference["selected_paths"]));
    if let Some(active) = state.selection.active {
        assert!(state.selection.selected.contains(&active));
    }
}
fn native_roundtrip(doc: &Document) {
    let saved = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    let before = doc.xcf_state.as_ref().unwrap();
    let after = reopened.xcf_state.as_ref().unwrap();
    assert_eq!(after.items, before.items);
    assert_eq!(after.selection, before.selection);
    assert_eq!(after.item_sets, before.item_sets);
    assert_eq!(after.opaque, before.opaque);
    assert_eq!(reopened.paths, doc.paths);
    assert_eq!(reopened.channels.len(), doc.channels.len());
    for (original, channel) in doc.channels.iter().zip(&reopened.channels) {
        assert_eq!((channel.id, &channel.name), (original.id, &original.name));
        let bounds = before.items.iter().find(|item| item.item == ItemRef::Channel(original.id)).unwrap().raster_bounds.unwrap();
        assert_eq!(channel.surface.try_read_interleaved(bounds).unwrap(), original.surface.try_read_interleaved(bounds).unwrap());
    }
}

#[test]
#[ignore = "requires reviewed local original item-state corpus and fresh GIMP sidecars"]
fn original_active_layers_channels_paths_locks_and_native_planes() {
    let root = root().join("item-state-probe");
    let manifest = json(&root.join("probe-manifest.json"));
    let independent = json(&root.join("fresh-process/proof.json"));
    for case in independent["cases"].as_array().unwrap() {
        let record = manifest["cases"].as_array().unwrap().iter().find(|entry| entry["id"] == case["id"]).unwrap();
        for stage in case["stages"].as_array().unwrap() {
            let (path, hash) = if stage["stage"] == "input" { ("input", "input_sha256") } else { ("reference_edit", "reference_edit_sha256") };
            let bytes = checked(&root.join(record[path].as_str().unwrap()), &record[hash]);
            let imported =
                photocraft_io::xcf_import::import("original", &bytes, &Interrupt::NONE).unwrap_or_else(|e| panic!("{} {}: {e}", case["id"], stage["stage"]));
            assert_state(&imported.document, &stage["state"], &root.join("fresh-process"));
            native_roundtrip(&imported.document);
        }
    }
}

#[test]
#[ignore = "requires reviewed local original item-state corpus and fresh GIMP sidecars"]
fn hidden_channels_import_native_and_match_independent_projection() {
    let root = root().join("item-state-probe");
    let sidecars = root.join("fresh-process");
    let manifest_bytes = checked(&root.join("probe-manifest.json"), &Value::String("befe6224c4073264ce0e40bce9213b9381eb9bba48c9e4d2b736e602ad3d5a81".into()));
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let proof: Value = serde_json::from_slice(&checked(
        &sidecars.join("proof.json"),
        &Value::String("8c044e383f3fa3261096d7d3d0a62753033caf38d02a01cc62e965edb6d2f8dd".into()),
    ))
    .unwrap();
    let case = proof["cases"].as_array().unwrap().iter().find(|case| case["id"] == "active-channels-paths-locks").unwrap();
    let record = manifest["cases"].as_array().unwrap().iter().find(|entry| entry["id"] == case["id"]).unwrap();
    for stage in case["stages"].as_array().unwrap() {
        let (path, hash) = if stage["stage"] == "input" { ("input", "input_sha256") } else { ("reference_edit", "reference_edit_sha256") };
        let bytes = checked(&root.join(record[path].as_str().unwrap()), &record[hash]);
        let doc = photocraft_io::import("channels.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        assert_state(&doc, &stage["state"], &sidecars);
        native_roundtrip(&doc);
        let native = photocraft_io::export(&doc, "channels.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("channels.pcraft", &native.bytes).unwrap().document;
        assert_eq!(*photocraft_io::named_for_file(&doc, "channels.pcraft"), reopened);
        let expected = photocraft_codecs::decode(&checked(&sidecars.join(stage["render"].as_str().unwrap()), &stage["render_sha256"])).unwrap();
        for source in [&doc, &reopened] {
            let actual = photocraft_compose::gimp_render::try_render(source, source.bounds(), &mut |_| Ok(())).unwrap();
            let mut worst = 0.0_f64;
            for (pixel, index) in actual.px.iter().zip(0_u32..) {
                for (channel, value) in pixel.iter().enumerate() {
                    worst = worst.max((value - f64::from(expected.get(index % source.size.width, index / source.size.width, channel))).abs());
                }
            }
            assert!(worst <= 0.501 / 255.0, "{}/255", worst * 255.0);
        }
        assert_eq!(doc, before);
    }
}

#[test]
#[ignore = "requires reviewed stock-canonical item-set corpus and independent layout observations"]
fn original_channel_and_path_set_declarations_native_roundtrip() {
    let root = root().join("item-sets-three-types");
    let proof = json(&root.join("set-types-proof.json"));
    for case in proof["cases"].as_array().unwrap() {
        let bytes = checked(&root.join(case["canonical"].as_str().unwrap()), &case["canonical_sha256"]);
        let imported = photocraft_io::xcf_import::import("original item sets", &bytes, &Interrupt::NONE).unwrap_or_else(|e| panic!("{}: {e}", case["id"]));
        assert_state(&imported.document, &case["state"], &root);
        let state = imported.document.xcf_state.as_ref().unwrap();
        let parsed = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(state.item_sets.len(), parsed.item_sets.len());
        for (set, declaration) in state.item_sets.iter().zip(&parsed.item_sets) {
            assert_eq!(set.name, declaration.name);
            let family = match declaration.item_type {
                0 => ItemFamily::Layer,
                1 => ItemFamily::Channel,
                2 => ItemFamily::Path,
                _ => panic!("item family"),
            };
            assert_eq!(set.family, family);
            let rule = match declaration.method {
                0 => ItemSetRule::Text(set.name.clone()),
                1 => ItemSetRule::Regex(set.name.clone()),
                2 => ItemSetRule::Glob(set.name.clone()),
                u32::MAX => ItemSetRule::Named,
                _ => panic!("item method"),
            };
            assert_eq!(set.rule, rule);
            for member in &set.members {
                assert!(matches!(
                    (family, member),
                    (ItemFamily::Layer, ItemRef::Layer(_)) | (ItemFamily::Channel, ItemRef::Channel(_)) | (ItemFamily::Path, ItemRef::Path(_))
                ));
            }
            // Fresh stock-canonical layout records two declarations: Layer first,
            // Channel second, but each family addresses its own named set by index0.
            let expected = if set.name == "Linked Layers" {
                vec!["Own linked blue".to_owned(), "Own linked orange".to_owned()]
            } else if family == ItemFamily::Channel && declaration.method == u32::MAX {
                vec!["Own linked alpha".to_owned()]
            } else {
                Vec::new()
            };
            assert_eq!(names(&imported.document, &set.members), expected, "{} {}", case["id"], set.name);
        }
        native_roundtrip(&imported.document);
    }
}
