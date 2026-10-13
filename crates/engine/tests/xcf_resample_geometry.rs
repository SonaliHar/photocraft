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
    let expected = bytes(root, &proof["projection"]["encoded_double"]);
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
#[ignore = "requires reviewed unpublished local wide-transform originals"]
fn resample_geometry_actual_command_matches_native_originals_and_independent_projection() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS");
    let root = Path::new(&base).join("wide-resample-geometry-evidence");
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
        let n = if id.contains("-rgb-") { 4 } else { 2 };
        let generated = bytes(&root, &c["fresh_edit"]["encoded_double"]);
        let outside_coverage = generated.chunks_exact(n * 8).any(|px| {
            let a = f64::from_le_bytes(px[(n - 1) * 8..n * 8].try_into().unwrap());
            !(0.0..=1.0).contains(&a)
        });
        for phase in ["source", "edit"] {
            let raw = bytes(&root, &c[phase]);
            let result = photocraft_io::import(&format!("{id}.{phase}.xcf"), &raw);
            let doc = result.unwrap().document;
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
            json!({"matrix":[m[0],m[3],m[1],m[4],m[2],m[5]],"interpolation":if id.starts_with("linear-"){"gimpLinear"}else{"gimpCubic"}})
        } else {
            let h = photocraft_algo::transform::Homography(m.iter().map(|n| n.as_f64().unwrap()).collect::<Vec<_>>().try_into().unwrap());
            let b = bounds(&c["fresh_source"]);
            let quad = [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)].map(|(x, y)| {
                let (u, v) = h.apply(f64::from(x), f64::from(y));
                [u, v]
            });
            json!({"rect":[b.x0,b.y0,b.x1,b.y1],"quad":quad,"interpolation":if id.starts_with("linear-"){"gimpLinear"}else{"gimpCubic"}})
        };
        let result = session.execute("edit.transform", params);
        result.unwrap();
        let edited = &session.active().unwrap().doc;
        let expected_bounds = bounds(&c["fresh_edit"]);
        let raster = edited.layer(owner).unwrap().surface().unwrap();
        let expected = bytes(&root, &c["fresh_edit"]["encoded_double"]);
        let mut maximum = 0_f64;
        let mut at = 0;
        for y in expected_bounds.y0..expected_bounds.y1 {
            for x in expected_bounds.x0..expected_bounds.x1 {
                let mut pixel = [0.; 4];
                raster.try_read_pixel_f64(x, y, &mut pixel[..raster.channels()]).unwrap();
                for v in &pixel[..raster.channels()] {
                    let e = f64::from_le_bytes(expected[at..at + 8].try_into().unwrap());
                    at += 8;
                    maximum = maximum.max((v - e).abs() * 255.);
                }
            }
        }
        assert_eq!(at, expected.len());
        assert!(maximum <= 2., "{id} resample {maximum}/255");
        println!("{id} resample {maximum}/255");
        let mut metadata = source.xcf_state.clone().unwrap();
        metadata.items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(expected_bounds);
        if outside_coverage {
            metadata.items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().composition.as_mut().unwrap().compatibility_epoch =
                photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH.into();
        }
        assert_eq!(edited.xcf_state.as_ref().unwrap(), &metadata);
        assert_eq!(edited.source_color_context, source.source_color_context);
        let projection = projections["records"].as_array().unwrap().iter().find(|r| r["id"] == id && r["phase"] == "edit").unwrap();
        worst = worst.max(check_render(edited, &proof_root, projection));
        session.undo();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), original);
        println!("resample generated native bounded {id}");
    }
    assert_eq!(count, 48);
    println!("resample geometry48allstage paths/24commands worst {worst}/255");
}

#[test]
#[ignore = "requires reviewed unpublished local wide-transform originals"]
fn unobserved_resample_geometry_hdr_and_cancellation_leave_native_history_exact() {
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("wide-resample-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let case = packet["records"].as_array().unwrap().iter().find(|c| c["id"] == "linear-double_non_linear-gray-scale").unwrap();
    let imported = photocraft_io::import("own-scope.xcf", &bytes(&root, &case["source"])).unwrap().document;
    for scenario in 0..4 {
        let mut doc = imported.clone();
        let owner = doc.layers[0].id;
        let mut params = json!({"matrix":[1.25,0,0,0.75,0.25,0.375],"interpolation":"gimpLinear"});
        match scenario {
            0 => params["matrix"] = json!([1.2, 0, 0, 0.75, 0.25, 0.375]),
            1 => doc.layer_mut(owner).unwrap().surface_mut().unwrap().try_write_pixel_f64(-2, 1, &[1.25, 0.5]).unwrap(),
            2 => doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
            _ => {}
        }
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(doc.clone(), None).unwrap();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 12;
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
        assert_eq!(*state.doc, doc);
        assert_eq!(state.history.past_len(), 0);
        assert!(!state.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), before);
    }
}
