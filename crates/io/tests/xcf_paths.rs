use photocraft_io::IoError;
#[path = "../src/xcf_paths.rs"]
mod mapper;
use photocraft_raster::Interrupt;
use photocraft_xcf::{PathPoint, PathStroke, VectorPath};
fn source(legacy: bool) -> VectorPath {
    let kinds = if legacy { [1, 2, 2] } else { [1, 0, 1] };
    VectorPath {
        name: "Own Ω".into(),
        tattoo: Some(1201),
        lock_visibility: true,
        selected: true,
        legacy_format: legacy.then_some(3),
        raw_record: if legacy { vec![] } else { vec![1, 2, 3] },
        item_sets: vec![2],
        strokes: vec![PathStroke {
            closed: true,
            points: kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| {
                    let mut components =
                        vec![(i as f32 + 0.125).to_bits(), (-0.0f32).to_bits(), 0.5f32.to_bits(), (-0.25f32).to_bits(), 0.125f32.to_bits(), 0.875f32.to_bits()];
                    if legacy {
                        components.truncate(2);
                    }
                    PathPoint { kind, legacy_integer_coordinates: None, components }
                })
                .collect(),
        }],
        ..Default::default()
    }
}
#[test]
fn exact_handles_identity_selection_members_and_raw_source() {
    for legacy in [false, true] {
        let input = source(legacy);
        let mapped = mapper::map_paths(vec![input], 24, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let p = &mapped.paths[0];
        let k = &p.path.subpaths[0].knots[0];
        assert_eq!((k.in_ctrl.x, k.anchor.x, k.out_ctrl.x), if legacy { (2.125, 0.125, 1.125) } else { (0.125, 1.125, 2.125) });
        assert_eq!(k.anchor.y.to_bits(), (-0.0f64).to_bits());
        assert!(p.path.subpaths[0].closed);
        assert_eq!(mapped.tattoos[0], (1201, p.id));
        assert_eq!(mapped.items[0].source_tattoo, Some(1201));
        assert!(mapped.items[0].locks.visibility);
        assert_eq!(mapped.selected[0], mapped.items[0].item);
        assert_eq!(mapped.memberships[0], (2, mapped.items[0].item));
        assert_eq!(mapped.opaque[0].namespace, "photocraft/xcf/derived-path-geometry-v1");
        assert_eq!(&mapped.opaque[0].bytes[..8], b"PCXPATH1");
        if !legacy {
            assert_eq!(&mapped.opaque[0].bytes[52..56], &0.5f32.to_bits().to_be_bytes());
        }
        if !legacy {
            assert_eq!(mapped.opaque[0].owner, Some(mapped.items[0].item));
            assert_eq!(mapped.opaque[1].bytes.as_slice(), &[1, 2, 3]);
        }
        let again = mapper::map_paths(vec![source(legacy)], 24, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        assert_ne!(p.id, again.paths[0].id);
    }
}
#[test]
fn legacy_anchor_first_open_endpoints_and_closed_wrap_are_exact() {
    for closed in [false, true] {
        let mut path = source(true);
        path.strokes[0].closed = closed;
        path.strokes[0].points = (0..if closed { 6 } else { 5 })
            .map(|index| PathPoint {
                kind: if index % 3 == 0 { 1 } else { 2 },
                components: vec![(index as f32 + 0.25).to_bits(), (-0.0f32).to_bits()],
                legacy_integer_coordinates: None,
            })
            .collect();
        let mapped = mapper::map_paths(vec![path], 18, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let knots = &mapped.paths[0].path.subpaths[0].knots;
        assert_eq!(knots.len(), 2);
        assert_eq!((knots[0].in_ctrl.x, knots[0].anchor.x, knots[0].out_ctrl.x), (if closed { 5.25 } else { 0.25 }, 0.25, 1.25));
        assert_eq!((knots[1].in_ctrl.x, knots[1].anchor.x, knots[1].out_ctrl.x), (2.25, 3.25, 4.25));
        assert_eq!(knots[0].in_ctrl.y.to_bits(), (-0.0f64).to_bits());
    }
    let mut unqualified = source(true);
    unqualified.strokes[0].points.iter_mut().zip([2, 1, 2]).for_each(|(point, kind)| point.kind = kind);
    assert!(mapper::map_paths(vec![unqualified], 18, &Interrupt::NONE, &mut |_| Ok(())).is_err());
}
#[test]
fn malformed_geometry_duplicate_tattoos_budget_and_cancel_are_atomic_errors() {
    for variant in 0..7 {
        let mut path = source(false);
        match variant {
            0 => {
                path.strokes[0].points.pop();
            }
            1 => path.strokes[0].points[1].kind = 1,
            2 => path.strokes[0].points[0].components[0] = f32::NAN.to_bits(),
            3 => path.legacy_format = Some(1),
            4 => path.strokes[0].points[0].components[0] = f32::MAX.to_bits(),
            5 => {
                path = source(true);
                path.strokes[0].points[0].components.push(0.5f32.to_bits());
            }
            _ => path.unsupported.push("required field".into()),
        };
        assert!(mapper::map_paths(vec![path], 24, &Interrupt::NONE, &mut |_| Ok(())).is_err());
    }
    assert!(mapper::map_paths(vec![source(false), source(false)], 24, &Interrupt::NONE, &mut |_| Ok(())).is_err());
    assert!(
        mapper::map_paths(vec![source(false)], 24, &Interrupt::NONE, &mut |_| Err(IoError::Unsupported("budget denial".into())))
            .unwrap_err_string()
            .contains("budget")
    );
    let stop = || true;
    assert!(matches!(mapper::map_paths(vec![source(false)], 24, &Interrupt::cancel_only(&stop), &mut |_| Ok(())), Err(IoError::Cancelled)));
}
trait ErrorString {
    fn unwrap_err_string(self) -> String;
}
impl<T> ErrorString for Result<T, IoError> {
    fn unwrap_err_string(self) -> String {
        match self {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        }
    }
}
#[test]
fn native_save_reopen_retains_ids_evidence_and_edited_handles() {
    for case in 0..3 {
        let mut path = source(case == 1);
        if case == 2 {
            path.raw_record.clear();
        }
        let mapped = mapper::map_paths(vec![path], if case == 0 { 24 } else { 0 }, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let source_evidence = mapped.opaque.clone();
        for (original, shared) in mapped.opaque.iter().zip(&source_evidence) {
            assert!(std::sync::Arc::ptr_eq(&original.bytes, &shared.bytes));
        }
        let mut doc = photocraft_doc::Document::new("own", photocraft_geom::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.paths = mapped.paths;
        doc.xcf_state = Some(photocraft_doc::xcf_state::XcfState { items: mapped.items, opaque: mapped.opaque, ..Default::default() });
        let id = doc.paths[0].id;
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let fresh = photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
        assert_ne!(fresh.paths[0].id, id);
        assert_eq!(fresh.xcf_state.as_ref().unwrap().opaque[0].owner, Some(photocraft_doc::item_identity::ItemRef::Path(fresh.paths[0].id)));
        assert_eq!(fresh.xcf_state.as_ref().unwrap().items[0].source_tattoo, Some(1201));
        let mut reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.paths[0].id, id);
        assert_eq!(reopened.xcf_state.as_ref().unwrap().opaque[0].owner, Some(photocraft_doc::item_identity::ItemRef::Path(id)));
        reopened.paths[0].path.subpaths[0].knots[0].anchor.x += 2.0;
        let bytes = photocraft_format::save_to_bytes(&reopened, &Default::default()).unwrap();
        let edited = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(edited.paths[0].id, id);
        assert_eq!(edited.paths[0].path.subpaths[0].knots[0].anchor.x, if case == 1 { 2.125 } else { 3.125 });
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, source_evidence);
    }
}
#[test]
#[ignore = "requires local independently authored full-XCF corpus"]
fn original_vector_basic_saved_path_matches_independent_xy_oracle() {
    for (suffix, expected) in [
        ("xcf", [12., 14., 12., 14., 12., 14., 74., 19., 74., 19., 88., 38., 65., 61., 29., 62., 29., 62.]),
        ("edit.xcf", [15., 16., 15., 16., 15., 16., 77., 21., 77., 21., 91., 40., 68., 63., 32., 64., 32., 64.]),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../photocraft-corpus/xcf/full/vector-basic.{suffix}"));
        let file = photocraft_xcf::decode(&std::fs::read(path).unwrap(), &Default::default()).unwrap();
        let mapped = mapper::map_paths(file.paths, file.version, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let knots = &mapped.paths[0].path.subpaths[0].knots;
        let xy: Vec<f64> = knots.iter().flat_map(|k| [k.in_ctrl.x, k.in_ctrl.y, k.anchor.x, k.anchor.y, k.out_ctrl.x, k.out_ctrl.y]).collect();
        assert_eq!(xy, expected);
        assert_eq!(mapped.tattoos[0].0, 1201);
    }
}

#[test]
#[ignore = "requires original saved paths and independent fresh GIMP state observations"]
fn original_path_states_geometry_and_native_source_evidence_are_exact() {
    for name in ["vector-basic", "vector-transform", "vector-modified"] {
        for (suffix, state_suffix) in [("xcf", "reopened_state.json"), ("edit.xcf", "edited_state.json")] {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
            let input = std::fs::read(root.join(format!("{name}.{suffix}"))).unwrap();
            let file = photocraft_xcf::decode(&input, &Default::default()).unwrap();
            let oracle: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{name}.{state_suffix}"))).unwrap()).unwrap();
            let mapped = mapper::map_paths(file.paths, file.version, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
            let path = &mapped.paths[0];
            let state = &mapped.items[0];
            let reference = &oracle["paths"][0];
            assert_eq!(path.name, reference["name"].as_str().unwrap());
            assert_eq!(state.visible, reference["visible"].as_bool().unwrap());
            assert_eq!(state.source_tattoo, Some(reference["tattoo"].as_u64().unwrap() as u32));
            assert_eq!(state.locks.content, reference["locks"]["content"].as_bool().unwrap());
            assert_eq!(state.locks.position, reference["locks"]["position"].as_bool().unwrap());
            assert_eq!(state.locks.visibility, reference["locks"]["visibility"].as_bool().unwrap());
            for (stroke, observed) in path.path.subpaths.iter().zip(reference["strokes"].as_array().unwrap()) {
                assert_eq!(stroke.closed, observed["closed"].as_bool().unwrap());
                let actual: Vec<f64> =
                    stroke.knots.iter().flat_map(|k| [k.in_ctrl.x, k.in_ctrl.y, k.anchor.x, k.anchor.y, k.out_ctrl.x, k.out_ctrl.y]).collect();
                let expected: Vec<f64> = observed["points"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect();
                assert_eq!(actual, expected);
            }
            let expected_opaque = mapped.opaque.clone();
            let expected_paths = mapped.paths.clone();
            let expected_items = mapped.items.clone();
            let mut doc = photocraft_doc::Document::new(
                "Own original path",
                photocraft_geom::Size::new(96, 80),
                photocraft_doc::ColorMode::Rgb,
                photocraft_doc::SampleType::U8,
            );
            doc.paths = mapped.paths;
            doc.xcf_state = Some(photocraft_doc::xcf_state::XcfState {
                items: mapped.items,
                opaque: mapped.opaque,
                selection: photocraft_doc::xcf_editing::EditingSelection { selected_paths: mapped.selected, ..Default::default() },
                ..Default::default()
            });
            let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(reopened.paths, expected_paths);
            assert_eq!(reopened.xcf_state.as_ref().unwrap().items, expected_items);
            assert_eq!(reopened.xcf_state.as_ref().unwrap().opaque, expected_opaque);
        }
    }
}
