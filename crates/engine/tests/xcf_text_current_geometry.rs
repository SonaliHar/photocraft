#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;

#[test]
#[ignore = "requires independent exact-font font-size originals"]
fn picking_and_transform_frame_follow_current_text_without_rewriting_source_cache() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("text-size-evidence");
    let bytes = std::fs::read(root.join("aa-size-source.xcf")).unwrap();
    let original = photocraft_io::import("text.xcf", &bytes).unwrap().document;
    let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("aa-size-edit.xcf")).unwrap()).unwrap().document;
    let owner = original.layers[0].id;
    let old_bounds = photocraft_engine::transform_cmds::transform_bounds(&original, &original.layers[0]);
    let expected_bounds = photocraft_engine::transform_cmds::transform_bounds(&reference, &reference.layers[0]);
    let reference_surface = reference.layers[0].surface().unwrap();
    let point = (expected_bounds.y0..expected_bounds.y1)
        .flat_map(|y| (old_bounds.x1..expected_bounds.x1).map(move |x| (x, y)))
        .find(|(x, y)| {
            let mut pixel = [0.0; 4];
            reference_surface.read_pixel(*x, *y, &mut pixel);
            pixel[3] > 0.0
        })
        .expect("independent new glyph beyond old source extent");
    let mut session = Session::new();
    session.add_document(original.clone(), None).unwrap();
    session.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":24})).unwrap();
    let edited = &session.active().unwrap().doc;
    assert_eq!(edited.layers, original.layers);
    assert_eq!(session.execute("layer.pickAt", json!({"x":point.0,"y":point.1,"select":false})).unwrap()["layer"], owner.0);
    let edited = &session.active().unwrap().doc;
    assert_eq!(photocraft_engine::transform_cmds::transform_bounds(edited, &edited.layers[0]), expected_bounds);
    assert_eq!(
        session.execute("layer.pickAt", json!({"x":expected_bounds.x1-1,"y":expected_bounds.y0,"select":false})).unwrap()["layer"],
        serde_json::Value::Null,
        "layout whitespace is not a glyph hit"
    );
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
    assert_eq!(session.execute("layer.pickAt", json!({"x":point.0,"y":point.1,"select":false})).unwrap()["layer"], serde_json::Value::Null);
    let cancel = photocraft_raster::Interrupt::cancel_only(&|| true);
    assert!(matches!(photocraft_engine::pick_cmds::layers_at_checked(&original, point.0, point.1, &cancel), Err(photocraft_engine::EngineError::Cancelled)));
    let polls = std::sync::atomic::AtomicU32::new(0);
    let late = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 5;
    assert!(matches!(
        photocraft_engine::pick_cmds::layers_at_checked(&original, point.0, point.1, &photocraft_raster::Interrupt::cancel_only(&late)),
        Err(photocraft_engine::EngineError::Cancelled)
    ));
    let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
    doc.xcf_state.as_mut().unwrap().resources[0].name = "Unresolved test font".into();
    let before = doc.clone();
    assert_eq!(photocraft_engine::transform_cmds::transform_bounds(doc, &doc.layers[0]), photocraft_doc::Rect::EMPTY);
    assert!(session.execute("layer.pickAt", json!({"x":old_bounds.x0+2,"y":old_bounds.y0+6})).is_err());
    assert_eq!(*session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    assert_eq!(std::fs::read(root.join("aa-size-source.xcf")).unwrap(), bytes);
}
#[test]
#[ignore = "requires independent exact-font placement originals"]
fn picking_and_frame_follow_current_origin_through_native_reopen_and_undo() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let path = root.join("text-spacing-evidence/none-32-HEFT-control.xcf");
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("text.xcf", &bytes).unwrap().document;
    let owner = original.layers[0].id;
    let before = original.xcf_state.clone().unwrap();
    let mut session = Session::new();
    session.add_document(original.clone(), None).unwrap();
    for (origin, suffix) in [([11, 2], "shift"), ([-5, 7], "negative")] {
        session.execute("xcf.text.position.set", json!({"layerId":owner.0.to_string(),"originPx":origin})).unwrap();
        let edited = &session.active().unwrap().doc;
        let reference_bytes = std::fs::read(root.join(format!("text-placement-evidence/none-32-HEFT-control-{suffix}.projection.bin"))).unwrap();
        let frame = photocraft_engine::transform_cmds::transform_bounds(edited, &edited.layers[0]);
        assert_eq!(frame, photocraft_doc::Rect::new(origin[0], origin[1], origin[0] + 83, origin[1] + 39));
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.xcf_state.as_ref().unwrap().items, before.items);
        assert_eq!(edited.xcf_state.as_ref().unwrap().source_items[0].cache, before.source_items[0].cache);
        let native = photocraft_io::export(edited, "position.pcraft", &Default::default()).unwrap();
        let restored = photocraft_io::import("position.pcraft", &native.bytes).unwrap().document;
        let mut expected_native = (**edited).clone();
        expected_native.name = "position.pcraft".into();
        assert_eq!(restored, expected_native);
        assert_eq!(photocraft_engine::transform_cmds::transform_bounds(&restored, &restored.layers[0]), frame);
        // Sample independent alpha across the canvas and explicitly include
        // a displaced glyph beyond the old frame and transparent layout space.
        let old_frame = photocraft_engine::transform_cmds::transform_bounds(&original, &original.layers[0]);
        let outside_old = reference_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .find_map(|(index, pixel)| {
                let point = (i32::try_from(index % 128).unwrap(), i32::try_from(index / 128).unwrap());
                (pixel[3] > 0 && !old_frame.contains(point.0, point.1)).then_some(point)
            })
            .expect("independent displaced glyph beyond old frame");
        let mut samples = std::collections::BTreeSet::from([outside_old]);
        for y in (0..64).step_by(8) {
            for x in (0..128).step_by(8) {
                samples.insert((x, y));
            }
        }
        for (x, y) in samples {
            let alpha = reference_bytes[usize::try_from(y * 128 + x).unwrap() * 4 + 3];
            let hits = photocraft_engine::pick_cmds::layers_at_checked(&restored, x, y, &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(hits.contains(&owner), alpha > 0, "{suffix} pixel({x},{y})");
        }
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, original);
    }
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
