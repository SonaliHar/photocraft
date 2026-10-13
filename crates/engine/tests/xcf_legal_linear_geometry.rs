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
fn native_storage_bytes(root: &Path, state: &Value) -> Vec<u8> {
    let raw = bytes(root, &state["native"]);
    if state["native_format"].as_str().unwrap().contains("half") {
        assert_eq!(raw.len() % 2, 0);
        raw.as_chunks::<2>().0.iter().flat_map(|b| photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(*b)).to_ne_bytes()).collect()
    } else {
        raw
    }
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
#[ignore = "requires reviewed unpublished legal-linear corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn linear_geometry_actual_command_matches_native_originals_and_independent_projection() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS");
    let root = Path::new(&base).join("legal-linear-geometry-evidence");
    check_packet(&root, 36);
}
fn check_packet(root: &Path, cases: usize) {
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
            let raw = bytes(root, &c[phase]);
            let doc = photocraft_io::import(&format!("{id}.{phase}.xcf"), &raw).unwrap().document;
            let state = &c[if phase == "source" { "fresh_source" } else { "fresh_edit" }];
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(bounds(state)).unwrap(), native_storage_bytes(root, state));
            let row = fresh["records"].as_array().unwrap().iter().find(|r| r["id"] == id && r["phase"] == phase).unwrap();
            assert_eq!(bytes(&fresh_root, &row["state"]["native"]), bytes(root, &state["native"]));
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
            json!({"matrix":[m[0],m[3],m[1],m[4],m[2],m[5]],"interpolation":"gimpLinear"})
        } else {
            let h = photocraft_algo::transform::Homography(m.iter().map(|n| n.as_f64().unwrap()).collect::<Vec<_>>().try_into().unwrap());
            let b = bounds(&c["fresh_source"]);
            let quad = [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)].map(|(x, y)| {
                let (u, v) = h.apply(f64::from(x), f64::from(y));
                [u, v]
            });
            json!({"rect":[b.x0,b.y0,b.x1,b.y1],"quad":quad,"interpolation":"gimpLinear"})
        };
        session.execute("edit.transform", params).unwrap();
        let edited = &session.active().unwrap().doc;
        let expected_bounds = bounds(&c["fresh_edit"]);
        let actual = edited.layer(owner).unwrap().surface().unwrap();
        let reference =
            photocraft_raster::Surface::try_from_interleaved(actual.format(), expected_bounds, &native_storage_bytes(root, &c["fresh_edit"])).unwrap();
        let mut native_maximum = 0_f64;
        for y in expected_bounds.y0..expected_bounds.y1 {
            for x in expected_bounds.x0..expected_bounds.x1 {
                let mut a = [0.; 4];
                let mut b = [0.; 4];
                actual.try_read_pixel_f64(x, y, &mut a[..actual.channels()]).unwrap();
                reference.try_read_pixel_f64(x, y, &mut b[..actual.channels()]).unwrap();
                for (a, b) in a.iter().zip(b.iter()) {
                    native_maximum = native_maximum.max((a - b).abs() * 255.);
                }
            }
        }
        assert!(native_maximum <= 2., "generated native {id}: {native_maximum}/255");
        println!("linear generated-native maximum {id} {native_maximum}/255");
        let mut metadata = source.xcf_state.clone().unwrap();
        metadata.items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(expected_bounds);
        assert_eq!(edited.xcf_state.as_ref().unwrap(), &metadata);
        assert_eq!(edited.source_color_context, source.source_color_context);
        let projection = projections["records"].as_array().unwrap().iter().find(|r| r["id"] == id && r["phase"] == "edit").unwrap();
        worst = worst.max(check_render(edited, &proof_root, projection));
        session.undo();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), original);
        println!("linear retained-native exact; generated bounded {id}");
    }
    assert_eq!(count, cases * 2);
    println!("linear geometry {count} allstage paths/{cases} commands worst {worst}/255");
}

#[test]
#[ignore = "requires reviewed unpublished legal-linear corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn linear_geometry_rejects_bad_maps_and_midstage_cancel_atomically() {
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("legal-linear-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let source = photocraft_io::import("own-geometry.xcf", &bytes(&root, &packet["records"][0]["source"])).unwrap().document;
    for scenario in 0..4 {
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let params = match scenario {
            0 => json!({"matrix":[0,0,0,0,0,0],"interpolation":"gimpLinear"}),
            1 => json!({"matrix":[1,0,0,1,1e12,0],"interpolation":"gimpLinear"}),
            2 => json!({"matrix":[1,0,"bad",1,0,0],"interpolation":"gimpLinear"}),
            _ => json!({"matrix":[1.25,0,0,0.75,0.25,0.375],"interpolation":"gimpLinear"}),
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

#[test]
#[ignore = "requires reviewed unpublished legal-linear alpha corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn associated_floor_native_holdouts_pass_actual_command_and_native_undo() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("legal-linear-geometry-evidence");
    check_packet(&root.join("hidden-basis"), 44);
    check_packet(&root.join("boundary-basis"), 16);
    check_packet(&root.join("mixed-floor-basis"), 12);
}

#[test]
#[ignore = "requires reviewed unpublished legal-linear corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn forged_half_origin_rejects_nonrepresentable_f32_current_data_atomically() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("legal-linear-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let c = packet["records"].as_array().unwrap().iter().find(|c| c["id"] == "float_non_linear-rgb-scale").unwrap();
    let mut source = photocraft_io::import("own-linear.xcf", &bytes(&root, &c["source"])).unwrap().document;
    source.source_color_context.as_mut().unwrap().source_precision = 550;
    let original = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let mut session = Session::new();
    session.add_document(source.clone(), None).unwrap();
    let error = session.execute("edit.transform", json!({"matrix":[1.25,0,0,0.75,0.25,0.375],"interpolation":"gimpLinear"})).unwrap_err();
    assert!(error.to_string().contains("half-float"), "{error}");
    let state = session.active().unwrap();
    assert_eq!(*state.doc, source);
    assert_eq!(state.history.past_len(), 0);
    assert!(!state.history.can_redo());
    assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), original);
}

#[test]
#[ignore = "requires reviewed unpublished legal-linear corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn unqualified_generated_float_color_overshoot_is_not_clamped_and_is_atomic() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("legal-linear-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let c = packet["records"].as_array().unwrap().iter().find(|c| c["id"] == "float_linear-rgb-scale").unwrap();
    let mut source = photocraft_io::import("own-linear.xcf", &bytes(&root, &c["source"])).unwrap().document;
    for y in 1..8 {
        for x in -2..6 {
            source.layers[0].surface_mut().unwrap().try_write_pixel_f64(x, y, &[1., 1., 1., if x % 2 == 0 { 0. } else { 1. }]).unwrap();
        }
    }
    let original = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let mut session = Session::new();
    session.add_document(source.clone(), None).unwrap();
    let error = session.execute("edit.transform", json!({"matrix":[1.25,0,0,0.75,0.25,0.375],"interpolation":"gimpLinear"})).unwrap_err();
    assert!(error.to_string().contains("generated color"), "{error}");
    let state = session.active().unwrap();
    assert_eq!(*state.doc, source);
    assert_eq!(state.history.past_len(), 0);
    assert!(!state.history.can_redo());
    assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), original);
}

#[test]
#[ignore = "requires reviewed unpublished legal-linear corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn changed_transfer_profile_depth_and_hdr_sources_reject_atomically() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("legal-linear-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let c = packet["records"].as_array().unwrap().iter().find(|c| c["id"] == "float_linear-rgb-scale").unwrap();
    let original = photocraft_io::import("own-linear.xcf", &bytes(&root, &c["source"])).unwrap().document;
    for scenario in 0..5 {
        let mut source = original.clone();
        match scenario {
            0 => source.sample_encoding = Some(photocraft_doc::SampleEncoding::Perceptual),
            1 => source.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
            2 => source.depth = photocraft_doc::SampleType::F64,
            3 => source.layers[0].surface_mut().unwrap().try_write_pixel_f64(-1, 2, &[1.5, 0.4, 0.5, 1.]).unwrap(),
            _ => {}
        }
        let native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let matrix = if scenario == 4 { json!([1, 0, 0, 1, 0.8, 0.2]) } else { json!([1.25, 0, 0, 0.75, 0.25, 0.375]) };
        assert!(session.execute("edit.transform", json!({"matrix":matrix,"interpolation":"gimpLinear"})).is_err());
        let state = session.active().unwrap();
        assert_eq!(*state.doc, source);
        assert_eq!(state.history.past_len(), 0);
        assert!(!state.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native);
    }
}

#[test]
#[ignore = "requires reviewed unpublished wide-linear-alpha corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn wide_floor_actual_source_edit_native_command_and_undo() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    check_packet(&Path::new(&base).join("wide-linear-alpha-geometry-evidence"), 24);
}

#[test]
#[ignore = "requires reviewed unpublished Half-linear-alpha corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn half_floor_actual_native_source_edit_command_target_quantization_and_undo() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    check_packet(&Path::new(&base).join("half-linear-alpha-geometry-evidence"), 12);
}

#[test]
#[ignore = "requires reviewed unpublished Perceptual-linear-alpha corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn perceptual_floor_actual_native_source_edit_command_quantization_and_undo() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    check_packet(&Path::new(&base).join("perceptual-linear-alpha-geometry-evidence"), 36);
}

#[test]
#[ignore = "requires reviewed unpublished Half/Perceptual fractional corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn fractional_half_perceptual_actual_native_source_edit_command_and_undo() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    check_packet(&Path::new(&base).join("fractional-half-perceptual-linear-evidence"), 48);
}

#[test]
#[ignore = "requires reviewed unpublished Half-linear-alpha corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn half_stale_current_samples_mismatched_encoding_and_unobserved_phase_reject_atomically() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("half-linear-alpha-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let original = photocraft_io::import("own-half.xcf", &bytes(&root, &packet["records"][0]["source"])).unwrap().document;
    for scenario in 0..3 {
        let mut source = original.clone();
        if scenario == 0 {
            source.layers[0].surface_mut().unwrap().try_write_pixel_f64(-1, 2, &[1. / 3., 0.5, 0.5, 0.5]).unwrap();
        }
        if scenario == 1 {
            source.source_color_context.as_mut().unwrap().source_precision = 575;
            source.sample_encoding = None;
        }
        let native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let matrix = if scenario == 2 { json!([1, 0, 0, 1, 0.125, 0.875]) } else { json!([1.25, 0, 0, 0.75, 0.25, 0.375]) };
        assert!(session.execute("edit.transform", json!({"matrix":matrix,"interpolation":"gimpLinear"})).is_err());
        let state = session.active().unwrap();
        assert_eq!(*state.doc, source);
        assert_eq!(state.history.past_len(), 0);
        assert!(!state.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native);
    }
}

#[test]
#[ignore = "requires reviewed unpublished Perceptual-linear-alpha corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn perceptual_mismatched_precision_profile_phase_hdr_and_stale_half_reject_atomically() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("perceptual-linear-alpha-geometry-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    for scenario in 0..5 {
        let id = if scenario == 4 { "half_perceptual-rgb-scale" } else { "double_perceptual-rgb-scale" };
        let c = packet["records"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap();
        let mut source = photocraft_io::import("own-perceptual.xcf", &bytes(&root, &c["source"])).unwrap().document;
        match scenario {
            0 => source.source_color_context.as_mut().unwrap().source_precision = 999,
            1 => source.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
            3 => source.layers[0].surface_mut().unwrap().try_write_pixel_f64(-1, 2, &[1.5, 0.5, 0.5, 0.5]).unwrap(),
            4 => source.layers[0].surface_mut().unwrap().try_write_pixel_f64(-1, 2, &[1. / 3., 0.5, 0.5, 0.5]).unwrap(),
            _ => {}
        }
        let native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let matrix = if scenario == 2 { json!([1, 0, 0, 1, 0.125, 0.875]) } else { json!([1.25, 0, 0, 0.75, 0.25, 0.375]) };
        assert!(session.execute("edit.transform", json!({"matrix":matrix,"interpolation":"gimpLinear"})).is_err());
        let state = session.active().unwrap();
        assert_eq!(*state.doc, source);
        assert_eq!(state.history.past_len(), 0);
        assert!(!state.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native);
    }
}

#[test]
#[ignore = "requires reviewed unpublished Half/Perceptual fractional corpus; set PHOTOCRAFT_XCF_FULL_CORPUS"]
fn fractional_current_guards_and_midstage_cancel_preserve_native_history() {
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let root = Path::new(&base).join("fractional-half-perceptual-linear-evidence");
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    for id in ["half_non_linear-rgb-phase25", "double_perceptual-rgb-phase25"] {
        let c = packet["records"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap();
        let original = photocraft_io::import("own-fractional.xcf", &bytes(&root, &c["source"])).unwrap().document;
        for scenario in 0..if id.starts_with("half") { 4 } else { 3 } {
            let mut source = original.clone();
            if scenario == 2 {
                source.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
            }
            if scenario == 3 {
                source.layers[0].surface_mut().unwrap().try_write_pixel_f64(-1, 2, &[1. / 3., 0.5, 0.5, 0.5]).unwrap();
            }
            let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            let matrix = if scenario == 0 { json!([1, 0, 0, 1, 0.125, 0.875]) } else { json!([1, 0, 0, 1, 0.25, 0.75]) };
            let params = json!({"matrix":matrix,"interpolation":"gimpLinear"});
            let polls = AtomicUsize::new(0);
            let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 10;
            let result = if scenario == 1 {
                photocraft_engine::transform_cmds::transform_with_interrupt(&mut session, &params, &Interrupt::cancel_only(&cancelled))
            } else {
                session.execute("edit.transform", params)
            };
            assert!(result.is_err());
            if scenario == 1 {
                assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
                assert!(polls.load(Ordering::Relaxed) >= 11);
            }
            let state = session.active().unwrap();
            assert_eq!(*state.doc, source);
            assert_eq!(state.history.past_len(), 0);
            assert!(!state.history.can_redo());
            assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), before);
        }
    }
}
