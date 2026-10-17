#![cfg(feature = "corpus")]
use sha2::{Digest, Sha256};
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
#[test]
#[ignore = "requires independent hash-bound multiline controls"]
fn original_multiline_native_cache_whole_doc_and_projection() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-multiline-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(sha(&manifest), "b399e6e64e4dbb2390ff52a037a0b6eb60071721b32bbfd3ef922552317dfb50");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (path, hash) in hashes {
        assert_eq!(sha(&std::fs::read(root.join(path)).unwrap()), hash);
    }
    let evidence: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    for row in evidence["rows"].as_array().unwrap() {
        let name = row["case"].as_str().unwrap();
        let bytes = std::fs::read(root.join(format!("{name}.xcf"))).unwrap();
        let doc = photocraft_io::import("placed.xcf", &bytes).unwrap().document;
        let state = doc.xcf_state.as_ref().unwrap();
        let source = &state.source_items[0];
        let cache = source.cache.as_ref().unwrap();
        let photocraft_doc::xcf_items::EditableSource::Text { current_origin, .. } = &source.source else { panic!() };
        assert_eq!(*current_origin, None);
        let item = state.items.iter().find(|item| item.item == source.owner).unwrap();
        let bounds = item.raster_bounds.unwrap();
        assert_eq!([bounds.x0, bounds.y0], [row["state"]["offset"][0].as_i64().unwrap() as i32, row["state"]["offset"][1].as_i64().unwrap() as i32]);
        let expected = std::fs::read(root.join(row["state"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(cache.bytes.as_slice(), expected);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), expected);
        let render = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let actual: Vec<u8> = render.px.iter().flat_map(|px| px.iter().map(|x| (x * 255.).round().clamp(0., 255.) as u8)).collect();
        let expected = std::fs::read(root.join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(actual.len(), expected.len());
        let delta = actual.iter().zip(expected).map(|(a, b)| a.abs_diff(b)).max().unwrap();
        println!("{name}: {delta}/255");
        assert!(delta <= 2);
        let saved = photocraft_io::export(&doc, "placed.pcraft", &Default::default()).unwrap();
        assert_eq!(photocraft_io::import("placed.pcraft", &saved.bytes).unwrap().document, *photocraft_io::named_for_file(&doc, "placed.pcraft"));
        assert_eq!(sha(&std::fs::read(root.join(format!("{name}.xcf"))).unwrap()), row["source_sha256"].as_str().unwrap());
    }
}
