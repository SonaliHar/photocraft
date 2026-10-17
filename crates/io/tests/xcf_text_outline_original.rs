#![cfg(feature = "corpus")]
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
#[ignore = "requires private independently authored exact-font AA controls"]
fn original_aa_canonical_configuration_qualifies_without_source_cache_rendering() {
    use photocraft_doc::{
        LayerId,
        item_identity::{ItemRef, ResourceId},
    };
    use photocraft_raster::Interrupt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let font = photocraft_text::xcf::ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let mut count = 0;
    for row in observations["rows"].as_array().unwrap().iter().filter(|row| row["antialias"] == true) {
        let original = std::fs::read(root.join(format!("{}.xcf", row["case"].as_str().unwrap()))).unwrap();
        assert_eq!(sha(&original), row["source_sha256"].as_str().unwrap());
        let file = photocraft_xcf::decode(&original, &Default::default()).unwrap();
        assert_eq!(file.version, 19);
        let mut layer = file.layers.into_iter().find(|layer| layer.text.is_some()).unwrap();
        let plane = layer.plane.take();
        let mapped = photocraft_io::xcf_sources::map_source(
            &layer,
            plane,
            photocraft_io::xcf_sources::SourceContext {
                owner: ItemRef::Layer(LayerId(99)),
                vector_path: None,
                format: photocraft_doc::PixelFormat::RGBA8,
                source_version: file.version,
            },
            &Interrupt::NONE,
            &mut || ResourceId(100),
            &mut |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        let request = photocraft_text::xcf::qualify_outline_source(&mapped.item, &mapped.resources[0], [8, 4], &Interrupt::NONE).unwrap();
        assert_eq!(request.content, row["text"].as_str().unwrap());
        photocraft_text::xcf::outline::qualify_outline_geometry(&font, request, &Interrupt::NONE).unwrap();
        assert_eq!(mapped.item.cache.as_ref().unwrap().bytes.as_slice(), std::fs::read(root.join(row["state"]["native"]["file"].as_str().unwrap())).unwrap());
        count += 1;
    }
    assert_eq!(count, 24);
}

#[test]
#[ignore = "requires private independently authored exact-font AA controls"]
fn original_quadratic_multicontour_and_kerning_text_pipeline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(sha(&manifest), "346c44a055c79a327db247f8df2019cef2e00d6a6462bcbb5f55fda0c5960c22");
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in &hashes {
        assert_eq!(sha(&std::fs::read(root.join(name)).unwrap()), *hash);
    }
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut tested = 0;
    for row in observations["rows"].as_array().unwrap().iter().filter(|row| row["antialias"] == true) {
        let name = row["case"].as_str().unwrap();
        let original = std::fs::read(root.join(format!("{name}.xcf"))).unwrap();
        assert_eq!(sha(&original), row["source_sha256"].as_str().unwrap());
        let doc = photocraft_io::import(name, &original).unwrap_or_else(|error| panic!("{name}: original import: {error}")).document;
        let state = doc.xcf_state.as_ref().unwrap();
        let source = &state.source_items[0];
        let cache = source.cache.as_ref().unwrap();
        let expected_native = std::fs::read(root.join(row["state"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(cache.bytes.as_slice(), expected_native);
        let item = state.items.iter().find(|item| item.item == source.owner).unwrap();
        let bounds = item.raster_bounds.unwrap();
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), cache.bytes.as_slice());
        let render = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let actual: Vec<u8> = render.px.iter().flat_map(|pixel| pixel.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8)).collect();
        let reference = std::fs::read(root.join(row["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(actual.len(), reference.len());
        let maximum = actual.iter().zip(&reference).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        println!("{name}: {maximum}/255");
        assert!(maximum <= 2, "{name}: unchanged original projection criterion");
        let saved = photocraft_io::export(&doc, "outline.pcraft", &Default::default()).unwrap();
        assert_eq!(photocraft_io::import("outline.pcraft", &saved.bytes).unwrap().document, doc);
        assert_eq!(sha(&std::fs::read(root.join(format!("{name}.xcf"))).unwrap()), sha(&original));
        tested += 1;
    }
    assert_eq!(tested, 24);
}
