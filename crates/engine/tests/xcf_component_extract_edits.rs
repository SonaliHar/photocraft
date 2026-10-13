#![cfg(feature = "corpus")]
//! Four typed selectors, operative opaque alpha, mutable Half and exact source retention.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn import(root: &Path, row: &Value) -> photocraft_doc::Document {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("component.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "local unpublished four-selector Component Extract independent originals"]
fn registered_component_selectors_bools_activation_native_undo_and_atomic_controls() {
    let packet = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-evidence");
    assert_eq!(hash(&fs::read(packet.join("batch-manifest.json")).unwrap()), "6b8e14eb4367f101dd65d072c3daa17c6f2577a4ccd1e9a1030289ebc7b231b8");
    let root = packet.join("enabled");
    let obs: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = obs["records"].as_array().unwrap();
    let disabled_root = packet.join("disabled");
    let disabled: Value = serde_json::from_slice(&fs::read(disabled_root.join("observations.json")).unwrap()).unwrap();
    let mut edits = 0;
    for row in rows.iter().filter(|row| row["component"] == "rgb-r" && row["invert"] == false && row["linear"] == false) {
        let source = import(&root, row);
        let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        for (selector, invert, linear) in
            [("rgb-g", false, false), ("rgb-b", false, false), ("alpha", false, false), ("rgb-r", true, false), ("rgb-r", false, true), ("rgb-r", true, true)]
        {
            let target_row = rows
                .iter()
                .find(|target| {
                    target["model"] == row["model"]
                        && target["precision"] == row["precision"]
                        && target["component"] == selector
                        && target["invert"] == invert
                        && target["linear"] == linear
                })
                .unwrap();
            let target = import(&root, target_row);
            let target_node = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            session.execute("xcf.filter.set",json!({"filterId":id.0.to_string(),"arguments":target_node.arguments.iter().map(|argument|json!({"name":argument.name,"value":argument.value})).collect::<Vec<_>>()})).unwrap();
            let edited = (*session.active().unwrap().doc).clone();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            let current = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(current.opaque, original.opaque);
            assert_eq!(current.provenance, original.provenance);
            assert_eq!(current.filter_masks, original.filter_masks);
            assert_eq!(current.current_filter_output_precision, original.current_filter_output_precision);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, target_node.arguments);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
                photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
            );
            let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), edited);
            assert_eq!(session.active().unwrap().history.past_len(), 1);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, edited);
            edits += 1;
        }
        let target_row = disabled["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["model"] == row["model"] && target["precision"] == row["precision"] && target["component"] == "rgb-r")
            .unwrap();
        let target = import(&disabled_root, target_row);
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
        let edited = (*session.active().unwrap().doc).clone();
        assert_eq!(edited.layers, source.layers);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
        );
        assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap(), edited);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, edited);
        edits += 1;
        for arguments in [
            json!([{"name":"component","value":{"Enum":{"domain":"gegl:component-extract/component","value":3}}}]),
            json!([{"name":"component","value":{"Enum":{"domain":"other","value":0}}}]),
            json!([{"name":"invert","value":{"Number":{"Float32Bits":0}}}]),
            json!([{"name":"linear","value":{"Bool":"false"}}]),
            json!([{"name":"invert","value":{"Bool":true}},{"name":"invert","value":{"Bool":false}}]),
            json!([{"name":"unknown","value":{"Bool":true}}]),
        ] {
            let mut failed = Session::new();
            failed.add_document(source.clone(), None).unwrap();
            assert!(failed.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":arguments})).is_err());
            assert_eq!(*failed.active().unwrap().doc, source);
            assert_eq!(failed.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&failed.active().unwrap().doc, &Default::default()).unwrap(), native);
        }
        let mut cancelled = Session::new();
        cancelled.add_document(source.clone(), None).unwrap();
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut cancelled,
                &json!({"filterId":id.0.to_string(),"arguments":[{"name":"invert","value":{"Bool":true}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
        for mutation in 0..9 {
            let mut changed = source.clone();
            let owner = changed.layers[0].id;
            match mutation {
                0 => changed.size.width = 9,
                1 => {
                    changed
                        .xcf_state
                        .as_mut()
                        .unwrap()
                        .items
                        .iter_mut()
                        .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
                        .unwrap()
                        .raster_bounds = Some(photocraft_geom::Rect::from_xywh(1, 0, 8, 6))
                }
                2 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "0:1".into(),
                3 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "other".into(),
                4 => changed.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(7, 5, &[0.5]).unwrap(),
                5 => changed.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                6 => {
                    changed
                        .xcf_state
                        .as_mut()
                        .unwrap()
                        .items
                        .iter_mut()
                        .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
                        .unwrap()
                        .locks
                        .content = true
                }
                7 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
                _ => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
            }
            let mut rejected = Session::new();
            rejected.add_document(changed.clone(), None).unwrap();
            assert!(rejected.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{} {mutation}", row["id"]);
            assert_eq!(*rejected.active().unwrap().doc, changed);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        }
        if matches!(source.depth, photocraft_doc::SampleType::F32 | photocraft_doc::SampleType::F64) {
            for (color, alpha) in [(f64::NAN, 0.5), (f64::INFINITY, 0.5), (1e30, 0.5), (0.5, 1.00001)] {
                let mut changed = source.clone();
                let pixel = if changed.mode == photocraft_doc::ColorMode::Grayscale { vec![color, alpha] } else { vec![color, 0.5, 0.75, alpha] };
                changed.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &pixel).unwrap();
                let mut failed = Session::new();
                failed.add_document(changed.clone(), None).unwrap();
                assert!(failed.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err());
                // IEEE NaN is retained bitwise; native save may independently
                // reject the hostile current document, never mutate its surface.
                assert_eq!(
                    failed.active().unwrap().doc.layers[0].surface().unwrap().try_read_interleaved(changed.bounds()).unwrap(),
                    changed.layers[0].surface().unwrap().try_read_interleaved(changed.bounds()).unwrap()
                );
                assert_eq!(failed.active().unwrap().history.past_len(), 0);
            }
        }
    }
    assert_eq!(edits, 252);
    println!("252 registered Component reference edits/native/undo,216 hostile params,324 current domain negatives and36 cancellations");
}

#[test]
#[ignore = "local unpublished public Half-to-Float Component Extract controls"]
fn component_current_half_promotion_remove_and_native_history() {
    let packet = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-evidence");
    let source_root = packet.join("enabled");
    let root = packet.join("promotion");
    let obs: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    assert_eq!(obs["records"].as_array().unwrap().len(), 96);
    for row in obs["records"].as_array().unwrap() {
        let original = fs::read(source_root.join(format!("{}.xcf", row["id"].as_str().unwrap()))).unwrap();
        assert_eq!(hash(&original), row["original_source_sha256"]);
        let source = photocraft_io::import("half.xcf", &original).unwrap().document;
        let target = import(&root, row);
        assert_eq!(source.depth, photocraft_doc::SampleType::F32);
        assert_eq!(source.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Half);
        assert_eq!(target.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("image.mode.bits32", json!({})).unwrap();
        let promoted = (*session.active().unwrap().doc).clone();
        assert_eq!(promoted.layers, source.layers);
        assert_eq!(promoted.source_color_context, source.source_color_context);
        assert_eq!(promoted.xcf_state.as_ref().unwrap().opaque, source.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(promoted.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(&promoted, promoted.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
        );
        assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&promoted, &Default::default()).unwrap()).unwrap(), promoted);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, promoted);
        let mut removed = Session::new();
        removed.add_document(source.clone(), None).unwrap();
        removed.execute("xcf.filter.remove", json!({"filterId":id.0.to_string()})).unwrap();
        let edited = (*removed.active().unwrap().doc).clone();
        assert_eq!(edited.layers, source.layers);
        assert_eq!(edited.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        assert!(edited.xcf_state.as_ref().unwrap().filter_stacks.iter().all(|stack| stack.nodes.is_empty()));
        assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap(), edited);
        removed.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*removed.active().unwrap().doc, source);
        removed.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*removed.active().unwrap().doc, edited);
    }
    println!("96 genuine public Half-to-Float current references +96 remove/native/undo histories");
}
