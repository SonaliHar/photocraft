#![cfg(feature = "corpus")]
//! Independently saved offset Component controls exercise registered edits.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn import(root: &std::path::Path, row: &Value) -> photocraft_doc::Document {
    let bytes = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("geometry.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "local unpublished independent132 Component geometry originals"]
fn registered_geometry_bool_activation_native_and_history_controls() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-geometry-evidence");
    assert_eq!(hash(&std::fs::read(root.join("batch-manifest.json")).unwrap()), "fe13672593245e18d8224faec99c18a3bfcd10cd5860427f60494e5d29396e88");
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = observations["records"].as_array().unwrap();
    let mut count = 0;
    for row in rows.iter().filter(|row| row["enabled"] == true && row["invert"] == false && row["linear"] == false) {
        let source = import(&root, row);
        let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let native = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        for arguments in [
            json!([]),
            json!([{"name":"unknown","value":{"Bool":true}}]),
            json!([{"name":"invert","value":{"Bool":"true"}}]),
            json!([{"name":"component","value":{"Enum":{"domain":"gegl:component-extract/component","value":3}}}]),
            json!([{"name":"invert","value":{"Bool":true}},{"name":"invert","value":{"Bool":false}}]),
        ] {
            let mut failed = Session::new();
            failed.add_document(source.clone(), None).unwrap();
            assert!(failed.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":arguments})).is_err());
            assert_eq!(*failed.active().unwrap().doc, source);
            assert_eq!(failed.active().unwrap().history.past_len(), 0);
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
        for mutation in 0..11 {
            let mut changed = source.clone();
            let owner = changed.layers[0].id;
            match mutation {
                0 => changed.size.width += 1,
                1 => {
                    changed
                        .xcf_state
                        .as_mut()
                        .unwrap()
                        .items
                        .iter_mut()
                        .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
                        .unwrap()
                        .raster_bounds = Some(photocraft_geom::Rect::from_xywh(0, 0, 19, 13))
                }
                2 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "0:1".into(),
                3 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "other".into(),
                4 => changed.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.5]).unwrap(),
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
                8 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                9 => changed.xcf_state.as_mut().unwrap().current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Half,
                _ => changed.depth = photocraft_doc::SampleType::F64,
            }
            let mut failed = Session::new();
            failed.add_document(changed.clone(), None).unwrap();
            assert!(failed.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{} mutation {mutation}", row["id"]);
            assert_eq!(*failed.active().unwrap().doc, changed);
            assert_eq!(failed.active().unwrap().history.past_len(), 0);
        }

        for disabled in [false, true] {
            let target_row = rows
                .iter()
                .find(|target| {
                    target["model"] == row["model"]
                        && target["canvas"] == row["canvas"]
                        && target["drawable"] == row["drawable"]
                        && target["offset"] == row["offset"]
                        && target["component"] == row["component"]
                        && target["enabled"] == !disabled
                        && target["invert"] == !disabled
                        && target["linear"] == !disabled
                })
                .unwrap();
            let target = import(&root, target_row);
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            if disabled {
                session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(), "enabled":false})).unwrap();
            } else {
                session.execute("xcf.filter.set", json!({"filterId":id.0.to_string(), "arguments":[{"name":"invert","value":{"Bool":true}}]})).unwrap();
                session.execute("xcf.filter.set", json!({"filterId":id.0.to_string(), "arguments":target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().map(|argument|json!({"name":argument.name,"value":argument.value})).collect::<Vec<_>>()})).unwrap();
            }
            let edited = (*session.active().unwrap().doc).clone();
            assert_eq!(edited.layers, source.layers);
            let current = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(current.opaque, original.opaque);
            assert_eq!(current.provenance, original.provenance);
            assert_eq!(current.filter_masks, original.filter_masks);
            assert_eq!(current.items, original.items);
            assert_eq!(current.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
                photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
            );
            assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap(), edited);
            for _ in 0..if disabled { 1 } else { 2 } {
                session.execute("edit.undo", json!({})).unwrap();
            }
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
            for _ in 0..if disabled { 1 } else { 2 } {
                session.execute("edit.redo", json!({})).unwrap();
            }
            assert_eq!(*session.active().unwrap().doc, edited);
            count += 1;
        }
    }
    assert_eq!(count, 88);
}
