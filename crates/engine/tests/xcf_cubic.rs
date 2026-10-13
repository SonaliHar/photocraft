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
fn actual_wide_cubic_command_preserves_source_and_matches_independent_reference_edit() {
    let folder = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS");
    let root = Path::new(&folder).join("wide-transform-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let proof_root = root.join("projection-evidence");
    let proofs: Value = serde_json::from_slice(&std::fs::read(proof_root.join("observations.json")).unwrap()).unwrap();
    let mut worst = 0_f64;
    let mut worst_projection = 0_f64;
    let mut imported_files = 0;
    for case in packet["records"].as_array().unwrap() {
        let name = case["id"].as_str().unwrap();
        let input = bytes(&root, &case["source"]);
        let source = photocraft_io::import(&format!("{name}.xcf"), &input).unwrap().document;
        let layer = source.layers.first().unwrap();
        let owner = layer.id;
        let before_bounds = bounds(&case["fresh_source"]);
        assert_eq!(layer.surface().unwrap().try_read_interleaved(before_bounds).unwrap(), bytes(&root, &case["fresh_source"]["native"]));
        let rows = proofs["records"].as_array().unwrap();
        let source_proof = rows.iter().find(|row| row["id"] == name && row["phase"] == "source").unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&input)), source_proof["original_sha256"].as_str().unwrap());
        worst_projection = worst_projection.max(check_render(&source, &proof_root, source_proof));
        imported_files += 1;
        let edit_bytes = bytes(&root, &case["edit"]);
        let edit_proof = rows.iter().find(|row| row["id"] == name && row["phase"] == "edit").unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&edit_bytes)), edit_proof["original_sha256"].as_str().unwrap());
        let expected_double = bytes(&root, &case["fresh_edit"]["encoded_double"]);
        let edit_channels = layer.surface().unwrap().channels();
        let alpha_overshoot = expected_double.chunks_exact(edit_channels * 8).any(|pixel| {
            let alpha = f64::from_le_bytes(pixel[(edit_channels - 1) * 8..edit_channels * 8].try_into().unwrap());
            !(0.0..=1.0).contains(&alpha)
        });
        let reference = photocraft_io::import(&format!("{name}.edit.xcf"), &edit_bytes).unwrap().document;
        assert_eq!(
            reference.layers[0].surface().unwrap().try_read_interleaved(bounds(&case["fresh_edit"])).unwrap(),
            bytes(&root, &case["fresh_edit"]["native"])
        );
        worst_projection = worst_projection.max(check_render(&reference, &proof_root, edit_proof));
        imported_files += 1;
        let before_native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let m = case["matrix"].as_array().unwrap();
        let command = session.execute("edit.transform", json!({"matrix":[m[0],m[3],m[1],m[4],m[2],m[5]],"interpolation":"gimpCubic"}));
        command.unwrap();
        let edited = &session.active().unwrap().doc;
        let projection = rows.iter().find(|row| row["id"] == name && row["phase"] == "edit").unwrap();
        worst_projection = worst_projection.max(check_render(edited, &proof_root, projection));
        let expected_bounds = bounds(&case["fresh_edit"]);
        let raster = edited.layer(owner).unwrap().surface().unwrap();
        let expected = bytes(&root, &case["fresh_edit"]["encoded_double"]);
        let channels = raster.channels();
        let mut pixel = [0.; 4];
        let mut cursor = 0;
        let mut maximum = 0_f64;
        for y in expected_bounds.y0..expected_bounds.y1 {
            for x in expected_bounds.x0..expected_bounds.x1 {
                raster.try_read_pixel_f64(x, y, &mut pixel[..channels]).unwrap();
                for value in &pixel[..channels] {
                    let reference = f64::from_le_bytes(expected[cursor..cursor + 8].try_into().unwrap());
                    cursor += 8;
                    maximum = maximum.max((value - reference).abs() * 255.);
                }
            }
        }
        assert_eq!(cursor, expected.len());
        assert!(maximum <= 2., "{name}: {maximum}/255");
        worst = worst.max(maximum);
        let mut preserved = source.xcf_state.clone().unwrap();
        preserved.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(expected_bounds);
        if alpha_overshoot {
            preserved.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().composition.as_mut().unwrap().compatibility_epoch =
                photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH.into();
        }
        assert_eq!(edited.xcf_state.as_ref().unwrap(), &preserved);
        assert_eq!(edited.source_color_context, source.source_color_context);
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        session.undo();
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(photocraft_format::load_from_bytes(&before_native).unwrap(), source);
        println!("{name} command {maximum}/255");
    }
    assert_eq!(imported_files, 32);
    println!(
        "actual wide cubic command maximum {worst}/255; independent projection maximum {worst_projection}/255; all-stage imported files {imported_files}; all16 actual commands accepted"
    );
}

#[test]
#[ignore = "requires reviewed unpublished local wide-transform originals"]
fn cubic_scope_and_cancellation_fail_without_native_or_history_changes() {
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let folder = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS");
    let root = Path::new(&folder).join("wide-transform-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let case = &packet["records"][0];
    let raw = bytes(&root, &case["source"]);
    let source = photocraft_io::import("own-cubic.xcf", &raw).unwrap().document;
    for scenario in 0..5 {
        let mut document = source.clone();
        let owner = document.layers[0].id;
        let mut params = json!({"matrix":[1,0,0,1,0.5,0.375],"interpolation":"gimpCubic"});
        match scenario {
            0 => params["matrix"] = json!([1, 0, 0, 1, 0.2, 0.2]),
            1 => params["matrix"] = json!([1.1, 0, 0, 1, 0.5, 0.375]),
            2 => document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
            3 => {
                document.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().raster_bounds =
                    Some(Rect::new(-2, 1, 31, 8))
            }
            _ => {}
        }
        let original = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(document.clone(), None).unwrap();
        let revision = session.active().unwrap().revision;
        let counter = AtomicUsize::new(0);
        let cancel = || counter.fetch_add(1, Ordering::Relaxed) >= 12;
        let ctl = Interrupt::cancel_only(&cancel);
        let result = if scenario == 4 {
            photocraft_engine::transform_cmds::transform_with_interrupt(&mut session, &params, &ctl)
        } else {
            session.execute("edit.transform", params)
        };
        assert!(result.is_err(), "scope scenario {scenario}");
        if scenario == 4 {
            assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
        }
        let state = session.active().unwrap();
        assert_eq!(*state.doc, document);
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.past_len(), 0);
        assert!(!state.history.can_redo());
        assert_eq!(photocraft_format::load_from_bytes(&original).unwrap(), *state.doc);
    }
}
