#![cfg(feature = "corpus")]
//! Registered finite Alpha Clip edits against independently saved unit-source references.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
fn import(root: &Path, row: &Value) -> photocraft_doc::Document {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    let hash = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    assert_eq!(hash, row["sha256"]);
    photocraft_io::import("finite-alpha.xcf", &bytes).unwrap().document
}
fn arguments(doc: &photocraft_doc::Document) -> Vec<Value> {
    doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().map(|a| json!({"name":a.name,"value":a.value})).collect()
}
fn render(doc: &photocraft_doc::Document) -> Vec<[f64; 4]> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap().px
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn finite_registered_edits_preserve_source_native_history_and_unit_alpha() {
    let corpus = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut edits = 0;
    for folder in ["point-alpha-clip-value-domain-evidence", "point-alpha-clip-value-domain-nonlinear-controls"] {
        let root = corpus.join(folder);
        let obs: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
        let rows = obs["records"].as_array().unwrap();
        for row in rows.iter().filter(|r| r["id"].as_str().unwrap().ends_with(".unit.ordinary")) {
            let source = import(&root, row);
            let mut unchanged = photocraft_engine::Session::new();
            unchanged.add_document(source.clone(), None).unwrap();
            let unchanged_id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
            unchanged.execute("xcf.filter.set", json!({"filterId": unchanged_id.0.to_string(), "arguments": arguments(&source)})).unwrap();
            assert_eq!(*unchanged.active().unwrap().doc, source);
            assert_eq!(unchanged.active().unwrap().history.past_len(), 0);
            let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
            let base = row["id"].as_str().unwrap().strip_suffix("ordinary").unwrap();
            for config in ["outside", "crossed", "neither", "disabled"] {
                let target = import(&root, rows.iter().find(|r| r["id"] == format!("{base}{config}")).unwrap());
                let mut session = photocraft_engine::Session::new();
                session.add_document(source.clone(), None).unwrap();
                if config == "disabled" {
                    session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
                } else {
                    session.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":arguments(&target)})).unwrap();
                }
                let edited = &session.active().unwrap().doc;
                assert_eq!(edited.layers, source.layers);
                assert_eq!(edited.source_color_context, source.source_color_context);
                assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, source.xcf_state.as_ref().unwrap().opaque);
                assert_eq!(edited.xcf_state.as_ref().unwrap().provenance, source.xcf_state.as_ref().unwrap().provenance);
                assert_eq!(edited.xcf_state.as_ref().unwrap().filter_masks, source.xcf_state.as_ref().unwrap().filter_masks);
                assert_eq!(edited.xcf_state.as_ref().unwrap().items, source.xcf_state.as_ref().unwrap().items);
                assert_eq!(render(edited), render(&target));
                let saved = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
                assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), **edited);
                let mut stream = std::io::Cursor::new(Vec::new());
                let (_, result) = photocraft_io::save_native_stream(edited, &mut stream, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
                assert!(result.warnings.is_empty());
                assert!(stream.get_ref().windows(4).any(|bytes| bytes == b"PK\x06\x06"));
                let stream_reopened =
                    photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
                assert_eq!(stream_reopened, **edited);
                assert_eq!(render(&stream_reopened), render(&target));
                session.execute("edit.undo", json!({})).unwrap();
                assert_eq!(*session.active().unwrap().doc, source);
                session.execute("edit.redo", json!({})).unwrap();
                assert_eq!(render(&session.active().unwrap().doc), render(&target));
                edits += 1;
            }
            for mutation in 0..9 {
                let mut changed = source.clone();
                match mutation {
                    0 => changed.size.width = 9,
                    1 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "0:1".into(),
                    2 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "other".into(),
                    3 => changed.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(7, 5, &[0.5]).unwrap(),
                    4 => changed.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                    5 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
                    6 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                    7 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_blend_space = -1710980527,
                    _ => changed.xcf_state.as_mut().unwrap().current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Half,
                }
                let mut rejected = photocraft_engine::Session::new();
                rejected.add_document(changed.clone(), None).unwrap();
                assert!(
                    rejected.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(),
                    "{} current mutation {mutation}",
                    row["id"]
                );
                assert_eq!(*rejected.active().unwrap().doc, changed);
                assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            }
            let mut cancelled = photocraft_engine::Session::new();
            cancelled.add_document(source.clone(), None).unwrap();
            assert!(
                photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                    &mut cancelled,
                    &json!({"filterId":id.0.to_string(),"arguments":arguments(&source)}),
                    &photocraft_raster::Interrupt::cancel_only(&|| true)
                )
                .is_err()
            );
            assert_eq!(*cancelled.active().unwrap().doc, source);
            // Operation permits finite thresholds; ordinary Normal refuses a computed nonunit output.
            let mut invalid = source.clone();
            let node = &mut invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            node.arguments[1].value =
                photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(1.125f32.to_bits()));
            node.arguments[2].value = photocraft_doc::xcf_filters::FilterValue::Bool(false);
            let mut rejected = photocraft_engine::Session::new();
            rejected.add_document(source.clone(), None).unwrap();
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":arguments(&invalid)})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
        }
    }
    assert_eq!(edits, 64);
}
