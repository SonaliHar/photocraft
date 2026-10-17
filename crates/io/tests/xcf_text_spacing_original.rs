#![cfg(feature = "corpus")]
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn root() -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-spacing-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&bytes), "2c987cfa774b38d9004717f68a657c6f7c2b3ff00004b43fcf03c8f61a934687");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, expected) in files {
        assert_eq!(hash(&std::fs::read(root.join(&name)).unwrap()), expected, "{name}");
    }
    root
}
fn projection(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}
#[test]
#[ignore = "requires frozen original exact-font GIMP antialiased text sources and public edits"]
fn spaced_original_states_source_cache_native_and_projection() {
    let root = root();
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    for row in observations["rows"].as_array().unwrap() {
        let name = row["case"].as_str().unwrap();
        let bytes = std::fs::read(root.join(format!("{name}.xcf"))).unwrap();
        let doc = photocraft_io::import("text.xcf", &bytes).unwrap().document;
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(state.source_items.len(), 1);
        let cache = state.source_items[0].cache.as_ref().unwrap();
        assert_eq!(cache.bytes.as_slice(), std::fs::read(root.join(row["state"]["native"]["file"].as_str().unwrap())).unwrap());
        cache.validate().unwrap();
        assert_eq!(
            doc.layers[0].surface().unwrap().try_read_interleaved(photocraft_geom::Rect::new(8, 4, 8 + cache.width as i32, 4 + cache.height as i32)).unwrap(),
            cache.bytes.as_slice()
        );
        let expected = std::fs::read(root.join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(expected, std::fs::read(root.join("round2").join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap());
        let actual = projection(&doc);
        let max = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert_eq!(actual.len(), expected.len());
        assert!(max <= 2, "{name}: {max}/255 exceeds unchanged display bound");
        let saved = photocraft_io::export(&doc, "text.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("text.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "text.pcraft"));
        assert_eq!(projection(&reopened), actual);
        assert_eq!(std::fs::read(root.join(format!("{name}.xcf"))).unwrap(), bytes);
        println!("{name}: max {max}/255, exact immutable cache/native document");
    }
}

#[test]
#[ignore = "requires independently saved/reopened zero-spacing reversal packet"]
fn zero_spacing_saved_reversals_exact_native_cache_and_projection() {
    let prior = root();
    let root = prior.parent().unwrap().join("text-spacing-zero-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "251d5cd635e48e2faff9b7558ee9484ada312efe2ae19cf85c5934544bbd3b92");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, expected) in files {
        assert_eq!(hash(&std::fs::read(root.join(name)).unwrap()), expected);
    }
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = observations["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 48);
    for row in rows {
        let name = row["case"].as_str().unwrap();
        let bytes = std::fs::read(root.join(format!("{name}.xcf"))).unwrap();
        assert_eq!(hash(&bytes), row["saved_sha256"]);
        let document = photocraft_io::import("text.xcf", &bytes).unwrap().document;
        let state = document.xcf_state.as_ref().unwrap();
        let cache = state.source_items[0].cache.as_ref().unwrap();
        let expected_native = std::fs::read(root.join(row["state"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(cache.bytes.as_slice(), expected_native);
        let bounds = state.items.iter().find(|s| s.item == state.source_items[0].owner).unwrap().raster_bounds.unwrap();
        assert_eq!(document.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), expected_native);
        let expected = std::fs::read(root.join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        let actual = projection(&document);
        assert_eq!(actual.len(), expected.len());
        let max = actual.iter().zip(expected).map(|(a, b)| a.abs_diff(b)).max().unwrap();
        assert!(max <= 2, "{name}: {max}/255");
        let native = photocraft_io::export(&document, "text.pcraft", &Default::default()).unwrap();
        assert_eq!(photocraft_io::import("text.pcraft", &native.bytes).unwrap().document, *photocraft_io::named_for_file(&document, "text.pcraft"));
        assert_eq!(std::fs::read(root.join(format!("{name}.xcf"))).unwrap(), bytes);
    }
}
