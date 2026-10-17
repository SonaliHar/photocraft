#![cfg(feature = "corpus")]
//! Real editable-vector imports use current path geometry, retaining immutable caches.
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn rendered(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|pixel| pixel.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8))
        .collect()
}
#[test]
#[ignore = "requires reviewed unpublished editable-vector rectangle source/edit witnesses"]
fn vector_import_native_regeneration_and_current_path_edit() {
    vector_pipeline(
        "vector-rectangle-evidence",
        ["534fc2bf3b95b2384768832e43a60cf52f42286b6e178fc1b8df3989dcfbe9bd", "e67908d884bead724342fa3ed515a3784c3406a62ea8f75c681170c12bb139f7"],
        ["e8eeebe272dd5bd5ce317b770fc26e9a6dc3d4e6590084e42d53a642b468672f", "f336c5a9f4c6b7258db09986bf9c53134099bddf66083fb3d383b2c5a00cc846"],
        [photocraft_geom::Rect::new(12, 14, 46, 33), photocraft_geom::Rect::new(10, 17, 44, 36)],
    );
}
#[test]
#[ignore = "requires reviewed unpublished canonical no-AA rectilinear vector evidence"]
fn orthogonal_vector_import_native_regeneration_and_current_path_edit() {
    vector_pipeline(
        "vector-orthogonal-evidence",
        ["4df42225a017b2c4548e14defabae15f1662c52f003b2e9b59ea693e695042ad", "270f6079a4d7e0ef2f4300f731f2acb0ec5b2ebce221aca8b2bb51588cde4fd0"],
        ["fa2c3c42368cf9366a61738c002baa9be1322180fb2a22c957ae623a95244211", "34247da00162c24d7bab15337f8f4ee95efc835477bf09c8a1237bd5e975bc18"],
        [photocraft_geom::Rect::new(10, 11, 48, 39), photocraft_geom::Rect::new(8, 14, 46, 42)],
    );
}
fn vector_pipeline(subtree: &str, source_hashes: [&str; 2], projection_hashes: [&str; 2], bounds: [photocraft_geom::Rect; 2]) {
    vector_pipeline_edit(subtree, source_hashes, projection_hashes, bounds, 0, [-2.0, 3.0]);
}
#[test]
#[ignore = "requires reviewed unpublished canonical compound vector evidence"]
fn compound_vector_import_native_regeneration_and_current_hole_edit() {
    vector_pipeline_edit(
        "vector-compound-evidence",
        ["de6b936e0873198508953cb293b1256b2627ec4314adc1912f289bb8ab2bb2bd", "ad479b969a799f7dc3dd5bfd40089f8651b55861ad893b7fbfc5ba15e93ff5bf"],
        ["c1b72396f499fe8fe232fa8c97dbf744877e493dc24376fd855ed64d9ee80efd", "599e413b716e20d976a89f90e753f324588efe0d07410cc5cfff98cff3358223"],
        [photocraft_geom::Rect::new(4, 4, 61, 43); 2],
        1,
        [2.0, -1.0],
    );
}
#[test]
#[ignore = "requires reviewed independent same-winding vector source/edit witnesses"]
fn same_winding_vector_import_uses_observed_implicit_parity() {
    vector_pipeline_edit(
        "vector-compound-winding-control",
        ["b68dc17b58a855c5114dbae79568ec7c2da5c42786c7d57a3d2c20a49d4e943f", "e3a1039ce15dbad2839326eb7231dea85c92d3f04202aeeaa8714b51ea4dc312"],
        ["c1b72396f499fe8fe232fa8c97dbf744877e493dc24376fd855ed64d9ee80efd", "599e413b716e20d976a89f90e753f324588efe0d07410cc5cfff98cff3358223"],
        [photocraft_geom::Rect::new(4, 4, 61, 43); 2],
        1,
        [2.0, -1.0],
    );
}
#[test]
#[ignore = "requires reviewed independently reopened no-AA solid stroke originals"]
fn solid_stroke_import_preserves_source_and_regenerates_current_geometry() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("vector-stroke-evidence");
    let manifests = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifests), "9f081f9f5b3e966df9b495a06b4a381049f2b53aff9029f2b99717126b94e6f4");
    for (phase, source_hash, projection_hash, rect) in [
        (
            "source",
            "afe3e369553c1f714525cb25e8b6e8bc84e8e49427beb75f17fb7f236e213cd7",
            "f2e8ad92f58561b7ab6921bdb4ccdcb57be5b344c819954a4a14ece83e617d5b",
            photocraft_geom::Rect::from_xywh(7, 9, 42, 30),
        ),
        (
            "edit",
            "7b1a4a48a8dd93c89d5a876a1aaf5df7b7edaa62983e6cb9b3d8423fcdad9653",
            "d46ef92438c1558ae5d7db32d5eba8749e3f70ed1cee352250db5c7efc955870",
            photocraft_geom::Rect::from_xywh(8, 7, 44, 32),
        ),
    ] {
        let bytes = std::fs::read(root.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(hash(&bytes), source_hash);
        let expected = std::fs::read(root.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(hash(&expected), projection_hash);
        let native = std::fs::read(root.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
        let doc = photocraft_io::import("stroke.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let source = &doc.xcf_state.as_ref().unwrap().source_items[0];
        assert!(matches!(source.source, photocraft_doc::xcf_items::EditableSource::Vector { source_modified: false, .. }));
        assert_eq!(source.cache.as_ref().unwrap().bytes.as_slice(), native);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(rect).unwrap(), native);
        assert_eq!(rendered(&doc), expected);
        let saved = photocraft_io::export(&doc, "stroke.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("stroke.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "stroke.pcraft"));
        assert_eq!(rendered(&reopened), expected);
        assert_eq!(doc, before);
        assert_eq!(std::fs::read(root.join(format!("{phase}.xcf"))).unwrap(), bytes);
    }
}
#[test]
#[ignore = "requires reviewed independently reopened AA solid stroke originals"]
fn antialiased_stroke_import_preserves_source_and_regenerates_current_geometry() {
    aa_stroke_pipeline(
        "vector-aa-stroke-evidence",
        "17587a60a8dabff799cf8103b82348a60de202826efd81393e218d5282de875f",
        [
            ("86f8cbba43879965a77e1218361d9f98ba48cf82e5282924d8f3ce2796844db0", photocraft_geom::Rect::new(8, 10, 47, 37)),
            ("42e0650abb63c875bbc720b79f5f577fe7a08d6bb642abba36215059d616ca06", photocraft_geom::Rect::new(9, 8, 50, 37)),
        ],
    );
}
#[test]
#[ignore = "requires reviewed independently reopened even AA stroke originals"]
fn even_aa_stroke_import_native_and_exact_projection() {
    aa_stroke_pipeline(
        "vector-aa-even-stroke-evidence",
        "ef51ccac24b1240b60c4bdb2555fe895486d5ea9aeacca2fa3d3d1a74e614ee1",
        [
            ("559ddb3e3ef94b680936cbd67176e64688ae90bb7aac19d63ce09579be4bf852", photocraft_geom::Rect::from_xywh(9, 11, 38, 26)),
            ("1b020d836fa6e57133f9738730893a882552c80bd9047fb1ecb2ae86a015dec9", photocraft_geom::Rect::from_xywh(10, 9, 40, 28)),
        ],
    );
}
fn aa_stroke_pipeline(subtree: &str, manifest_hash: &str, cases: [(&str, photocraft_geom::Rect); 2]) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(subtree);
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), manifest_hash);
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let checked = |name: &str| {
        let row = manifest["files"].as_array().unwrap().iter().find(|row| row["path"] == name).unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        assert_eq!(bytes.len(), row["bytes"].as_u64().unwrap() as usize);
        bytes
    };
    for (phase, (source_hash, rect)) in ["source", "edit"].into_iter().zip(cases) {
        let bytes = checked(&format!("{phase}.xcf"));
        assert_eq!(hash(&bytes), source_hash);
        let expected = checked(&format!("{phase}.ref.rgba-u8.bin"));
        let native = checked(&format!("{phase}-reopened.layer.native.bin"));
        let doc = photocraft_io::import("aa-stroke.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let source = &doc.xcf_state.as_ref().unwrap().source_items[0];
        assert!(matches!(source.source, photocraft_doc::xcf_items::EditableSource::Vector { source_modified: false, .. }));
        assert_eq!(source.cache.as_ref().unwrap().bytes.as_slice(), native);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(rect).unwrap(), native);
        assert_eq!(rendered(&doc), expected);
        let saved = photocraft_io::export(&doc, "aa-stroke.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("aa-stroke.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "aa-stroke.pcraft"));
        assert_eq!(rendered(&reopened), expected);
        assert_eq!(doc, before);
        assert_eq!(checked(&format!("{phase}.xcf")), bytes);
    }
}
fn vector_pipeline_edit(
    subtree: &str,
    source_hashes: [&str; 2],
    projection_hashes: [&str; 2],
    bounds: [photocraft_geom::Rect; 2],
    subpath: usize,
    delta: [f64; 2],
) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(subtree);
    let mut sources = Vec::new();
    for (index, phase) in ["source", "edit"].into_iter().enumerate() {
        let (source_hash, projection_hash, rect) = (source_hashes[index], projection_hashes[index], bounds[index]);
        let bytes = std::fs::read(root.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(hash(&bytes), source_hash);
        let doc = photocraft_io::import("vector.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(state.source_items.len(), 1);
        assert!(matches!(state.source_items[0].source, photocraft_doc::xcf_items::EditableSource::Vector { source_modified: false, .. }));
        let cache = state.source_items[0].cache.as_ref().unwrap();
        let native = std::fs::read(root.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
        assert_eq!(cache.bytes.as_slice(), native);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(rect).unwrap(), native);
        let projection =
            if subtree == "vector-compound-winding-control" { format!("{phase}-reopened.projection.native.bin") } else { format!("{phase}.ref.rgba-u8.bin") };
        let expected = std::fs::read(root.join(projection)).unwrap();
        assert_eq!(hash(&expected), projection_hash);
        assert_eq!(rendered(&doc), expected);
        let saved = photocraft_io::export(&doc, "vector.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("vector.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "vector.pcraft"));
        assert_eq!(rendered(&reopened), expected);
        assert_eq!(doc, before);
        assert_eq!(hash(&std::fs::read(root.join(format!("{phase}.xcf"))).unwrap()), source_hash);
        sources.push(doc);
    }
    let original = sources[0].clone();
    let mut moved = original.clone();
    for knot in &mut moved.paths[0].path.subpaths[subpath].knots {
        knot.anchor.x += delta[0];
        knot.anchor.y += delta[1];
        knot.in_ctrl = knot.anchor;
        knot.out_ctrl = knot.anchor;
    }
    assert_eq!(rendered(&moved), rendered(&sources[1]));
    assert_eq!(moved.xcf_state.as_ref().unwrap().source_items[0].cache, original.xcf_state.as_ref().unwrap().source_items[0].cache);
    assert_eq!(moved.layers, original.layers, "geometry editing leaves immutable raster/cache evidence intact");
    let saved = photocraft_io::export(&moved, "moved.pcraft", &Default::default()).unwrap();
    let reopened = photocraft_io::import("moved.pcraft", &saved.bytes).unwrap().document;
    assert_eq!(reopened, *photocraft_io::named_for_file(&moved, "moved.pcraft"));
    assert_eq!(rendered(&reopened), rendered(&sources[1]));
}
