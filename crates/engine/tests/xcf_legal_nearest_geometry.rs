#![cfg(feature = "corpus")]
use photocraft_doc::{Rect, item_identity::ItemRef};
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
fn bytes(root: &Path, entry: &Value) -> Vec<u8> {
    let raw = std::fs::read(root.join(entry["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), entry["sha256"].as_str().unwrap());
    raw
}
fn bounds(state: &Value) -> Rect {
    let x = state["offset"][0].as_i64().unwrap() as i32;
    let y = state["offset"][1].as_i64().unwrap() as i32;
    Rect::new(x, y, x + state["size"][0].as_i64().unwrap() as i32, y + state["size"][1].as_i64().unwrap() as i32)
}
fn check_render(doc: &photocraft_doc::Document, root: &Path, proof: &Value) -> f64 {
    let native = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    let restored = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(&restored, doc);
    let output = photocraft_compose::gimp_render::try_render(&restored, restored.bounds(), &mut |_| Ok(())).unwrap();
    let reference_encoding =
        if doc.sample_encoding == Some(photocraft_doc::sample_encoding::SampleEncoding::Linear) { "linear_double" } else { "encoded_double" };
    let expected = bytes(root, &proof["projection"][reference_encoding]);
    let components = if doc.mode == photocraft_doc::ColorMode::Grayscale { 2 } else { 4 };
    assert_eq!(expected.len(), output.px.len() * components * 8);
    let mut maximum = 0_f64;
    for (pixel, reference) in output.px.iter().zip(expected.chunks_exact(components * 8)) {
        let values = reference.as_chunks::<8>().0.iter().map(|value| f64::from_le_bytes(*value)).collect::<Vec<_>>();
        for c in 0..4 {
            let value = if components == 2 { values[usize::from(c == 3)] } else { values[c] };
            if c < 3 && pixel[3] == 0. && values[components - 1] == 0. {
                continue;
            }
            maximum = maximum.max((pixel[c] - value).abs() * 255.);
        }
    }
    assert!(maximum <= 2., "independent projection {maximum}/255");
    maximum
}
#[test]
#[ignore = "requires reviewed unpublished legal-nearest corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn nearest_geometry_actual_command_matches_native_originals_and_independent_projection() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS");
    let root = Path::new(&base).join("legal-nearest-geometry-evidence");
    let proof_root = root.join("projection-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let projections: Value = serde_json::from_slice(&std::fs::read(proof_root.join("observations.json")).unwrap()).unwrap();
    let fresh_root = root.join("fresh-process");
    let fresh: Value = serde_json::from_slice(&std::fs::read(fresh_root.join("observations.json")).unwrap()).unwrap();
    let mut worst = 0_f64;
    let mut count = 0;
    for c in packet["records"].as_array().unwrap() {
        let id = c["id"].as_str().unwrap();
        let mut source = None;
        for phase in ["source", "edit"] {
            let raw = bytes(&root, &c[phase]);
            let doc = photocraft_io::import(&format!("{id}.{phase}.xcf"), &raw).unwrap().document;
            let state = &c[if phase == "source" { "fresh_source" } else { "fresh_edit" }];
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(bounds(state)).unwrap(), bytes(&root, &state["native"]));
            let row = fresh["records"].as_array().unwrap().iter().find(|r| r["id"] == id && r["phase"] == phase).unwrap();
            assert_eq!(bytes(&fresh_root, &row["state"]["native"]), bytes(&root, &state["native"]));
            let projection = projections["records"].as_array().unwrap().iter().find(|r| r["id"] == id && r["phase"] == phase).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&raw)), projection["original_sha256"].as_str().unwrap());
            worst = worst.max(check_render(&doc, &proof_root, projection));
            count += 1;
            if phase == "source" {
                source = Some(doc);
            }
        }
        let source = source.unwrap();
        let owner = source.layers[0].id;
        let original = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let m = c["matrix"].as_array().unwrap();
        let params = if m[6].as_f64().unwrap() == 0. && m[7].as_f64().unwrap() == 0. {
            json!({"matrix":[m[0],m[3],m[1],m[4],m[2],m[5]],"interpolation":"nearest"})
        } else {
            let h = photocraft_algo::transform::Homography(m.iter().map(|n| n.as_f64().unwrap()).collect::<Vec<_>>().try_into().unwrap());
            let b = bounds(&c["fresh_source"]);
            let quad = [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)].map(|(x, y)| {
                let (u, v) = h.apply(f64::from(x), f64::from(y));
                [u, v]
            });
            json!({"rect":[b.x0,b.y0,b.x1,b.y1],"quad":quad,"interpolation":"nearest"})
        };
        session.execute("edit.transform", params).unwrap();
        let edited = &session.active().unwrap().doc;
        let expected_bounds = bounds(&c["fresh_edit"]);
        assert_eq!(edited.layer(owner).unwrap().surface().unwrap().try_read_interleaved(expected_bounds).unwrap(), bytes(&root, &c["fresh_edit"]["native"]));
        let mut metadata = source.xcf_state.clone().unwrap();
        metadata.items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(expected_bounds);
        assert_eq!(edited.xcf_state.as_ref().unwrap(), &metadata);
        assert_eq!(edited.source_color_context, source.source_color_context);
        let projection = projections["records"].as_array().unwrap().iter().find(|r| r["id"] == id && r["phase"] == "edit").unwrap();
        worst = worst.max(check_render(edited, &proof_root, projection));
        session.undo();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), original);
        println!("nearest actualnative exact {id}");
    }
    assert_eq!(count, 72);
    println!("nearest geometry72allstage paths/36commands worst {worst}/255");
}

#[test]
#[ignore = "requires reviewed unpublished legal-nearest corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn nearest_geometry_rejects_bad_maps_and_midstage_cancel_atomically() {
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("legal-nearest-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let source = photocraft_io::import("own-geometry.xcf", &bytes(&root, &packet["records"][0]["source"])).unwrap().document;
    for scenario in 0..4 {
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let params = match scenario {
            0 => json!({"matrix":[0,0,0,0,0,0],"interpolation":"nearest"}),
            1 => json!({"matrix":[1,0,0,1,1e12,0],"interpolation":"nearest"}),
            2 => json!({"matrix":[1,0,"bad",1,0,0],"interpolation":"nearest"}),
            _ => json!({"matrix":[1.25,0,0,0.75,0.25,0.375],"interpolation":"nearest"}),
        };
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 10;
        let result = if scenario == 3 {
            photocraft_engine::transform_cmds::transform_with_interrupt(&mut session, &params, &Interrupt::cancel_only(&cancel))
        } else {
            session.execute("edit.transform", params)
        };
        assert!(result.is_err());
        if scenario == 3 {
            assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
        }
        let state = session.active().unwrap();
        assert_eq!(*state.doc, source);
        assert_eq!(state.history.past_len(), 0);
        assert!(!state.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), before);
    }
}
