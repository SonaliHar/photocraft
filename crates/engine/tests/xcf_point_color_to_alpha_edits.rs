#![cfg(feature = "corpus")]
//! Real six-precision three-transfer typed literal-color color-to-alpha edits retain native source and canvas masks.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
#[test]
#[ignore = "local unpublished independently authored color-to-alpha originals"]
fn original_point_color_to_alpha_activation_native_pixel_history_and_atomic_rejection() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-to-alpha-evidence");
    let manifest = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "91f8d0974d0e122641fb15c7b40b1165b157b599a9bdb17c4776fa3d5b5171dd");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    let obs = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&obs), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap()["sha256"]);
    let obs: Value = serde_json::from_slice(&obs).unwrap();
    let mut count = 0;
    let mut current_float_edits = 0;
    for row in obs["records"].as_array().unwrap().iter().filter(|r| r["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        if row["precision"].as_str().unwrap().starts_with("HALF") {
            let mut current = source.clone();
            let pixel = if current.mode == photocraft_doc::ColorMode::Grayscale { vec![0.123456789, 0.5] } else { vec![0.123456789, 0.5, 0.75, 0.5] };
            current.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &pixel).unwrap();
            let exact_current = current.layers[0].surface().unwrap().try_read_interleaved(current.bounds()).unwrap();
            let mut session = Session::new();
            session.add_document(current.clone(), None).unwrap();
            session.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false})).unwrap();
            let edited = session.active().unwrap().doc.clone();
            assert_eq!(edited.layers[0].surface().unwrap().try_read_interleaved(edited.bounds()).unwrap(), exact_current);
            assert_eq!(edited.source_color_context, source.source_color_context);
            let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *edited);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, current);
            current_float_edits += 1;
        }
        for args in [
            json!({"arguments":[{"name":"color","value":{"Color":{"components":[{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":f32::NAN.to_bits()}],"encoding":"nonlinear_profile","profile":null}}}]}),
            json!({"arguments":[{"name":"color","value":{"Color":{"components":[{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":0}],"encoding":"Linear","profile":null}}}]}),
            json!({"arguments":[{"name":"color","value":{"Color":{"components":[{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":0}],"encoding":"nonlinear_profile","profile":1}}}]}),
            json!({"arguments":[{"name":"transparency-threshold","value":{"Number":{"Float32Bits":1.0001f32.to_bits()}}}]}),
            json!({"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":f32::NAN.to_bits()}}}]}),
            json!({"arguments":[{"name":"black-level","value":{"Number":{"Float64Bits":"0x0000000000000000"}}}]}),
            json!({"arguments":[{"name":"fake","value":{"Bool":true}}]}),
            json!({"unexpected":true}),
        ] {
            let mut params = args;
            params["filterId"] = json!(node.id.0.to_string());
            let mut s = Session::new();
            s.add_document(source.clone(), None).unwrap();
            assert!(s.execute("xcf.filter.set", params).is_err());
            assert_eq!(*s.active().unwrap().doc, source);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), before);
        }
        if family.ends_with("small") {
            for mutation in 0..12 {
                let mut changed = source.clone();
                let owner = changed.layers[0].id;
                let state = changed.xcf_state.as_mut().unwrap();
                match mutation {
                    0 => state.filter_masks[0].surface.try_write_pixel_f64(7, 5, &[0.5]).unwrap(),
                    1 => state.filter_masks[0].bounds.x0 = -13,
                    2 => state.items.iter_mut().find(|item| item.mask.is_some()).unwrap().mask.as_mut().unwrap().offset = [-13, 13],
                    3 => state.filter_stacks[0].nodes[0].mask = None,
                    4 => state.filter_stacks[0].nodes[0].operation.schema_epoch = "unqualified".into(),
                    5 => state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner)).unwrap().locks.content = true,
                    6 => changed.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                    7 => state.filter_masks[0].owner = photocraft_doc::item_identity::FilterId(u64::MAX),
                    8 => state.items.iter_mut().find(|item| item.mask.is_some()).unwrap().mask.as_mut().unwrap().shown = true,
                    9 => state.items.iter_mut().find(|item| item.mask.is_some()).unwrap().visible = false,
                    10 => {
                        changed.depth =
                            if source.depth == photocraft_doc::SampleType::U8 { photocraft_doc::SampleType::U16 } else { photocraft_doc::SampleType::U8 }
                    }
                    _ => state.filter_stacks[0].nodes[0].source_clipping = Some(1),
                }
                let mut rejected = Session::new();
                rejected.add_document(changed.clone(), None).unwrap();
                assert!(rejected.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false})).is_err(), "{family} {mutation}");
                assert_eq!(*rejected.active().unwrap().doc, changed);
                assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            }
        }
        let mut cancelled = Session::new();
        cancelled.add_document(source.clone(), None).unwrap();
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut cancelled,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"transparency-threshold","value":{"Number":{"Float32Bits":0.1375f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::enable_with_interrupt(
                &mut cancelled,
                &json!({"filterId":node.id.0.to_string(),"enabled":false}),
                &photocraft_raster::Interrupt::cancel_only(&|| true),
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
        assert_eq!(photocraft_format::save_to_bytes(&cancelled.active().unwrap().doc, &Default::default()).unwrap(), before);
        for stage in ["edit", "colors", "disabled"] {
            let targetrow = obs["records"].as_array().unwrap().iter().find(|r| r["id"] == format!("{family}.{stage}")).unwrap();
            let targetbytes = fs::read(root.join(targetrow["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&targetbytes), targetrow["sha256"]);
            let target = photocraft_io::import("reference.xcf", &targetbytes).unwrap().document;
            let targetnode = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut s = Session::new();
            s.add_document(source.clone(), None).unwrap();
            if stage != "disabled" {
                let arguments: Vec<_> = targetnode
                    .arguments
                    .iter()
                    .filter(|argument| if stage == "colors" { argument.name == "color" } else { argument.name.ends_with("-threshold") })
                    .map(|argument| json!({"name":argument.name,"value":argument.value}))
                    .collect();
                s.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":arguments})).unwrap();
            } else {
                s.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false})).unwrap();
            }
            let edited = (*s.active().unwrap().doc).clone();
            if stage == "disabled" {
                assert_eq!(edited.layers, source.layers);
            } else {
                assert_eq!(edited.layers[0].surface(), target.layers[0].surface());
            }
            assert_eq!(edited.source_color_context, source.source_color_context);
            let current = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(current.opaque, original.opaque);
            assert_eq!(current.provenance, original.provenance);
            assert_eq!(current.filter_masks, original.filter_masks);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, targetnode.arguments);
            assert_eq!(current.filter_stacks[0].nodes[0].enabled, targetnode.enabled);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
                photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
            );
            let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, source);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, edited);
            count += 1;
        }
        assert_eq!(fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), bytes);
    }
    assert_eq!(count, 108);
    assert_eq!(current_float_edits, 6);
}

#[test]
#[ignore = "local independent Half-to-Float current-buffer promotion witnesses"]
fn explicit_same_depth_float_promotion_and_removal_preserve_source_with_native_undo() {
    use photocraft_doc::FilterOutputPrecision;
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-to-alpha-evidence");
    let promoted_root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-to-alpha-half-promotion-evidence");
    let records: Value = serde_json::from_slice(&fs::read(promoted_root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in records["records"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let original = fs::read(root.join(format!("{id}.xcf"))).unwrap();
        assert_eq!(hash(&original), row["original_source_sha256"]);
        let source = photocraft_io::import("source.xcf", &original).unwrap().document;
        assert_eq!(source.depth, photocraft_doc::SampleType::F32);
        assert_eq!(source.xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Half);
        let reference = fs::read(promoted_root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&reference), row["sha256"]);
        let reference = photocraft_io::import("float-reference.xcf", &reference).unwrap().document;
        assert_eq!(reference.xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Canonical);
        if let Some(family) = id.strip_suffix(".edit") {
            let initial_bytes = fs::read(root.join(format!("{family}.source.xcf"))).unwrap();
            let initial = photocraft_io::import("initial.xcf", &initial_bytes).unwrap().document;
            let id = initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
            let arguments: Vec<_> = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
                .arguments
                .iter()
                .filter(|argument| argument.name.ends_with("-threshold"))
                .map(|argument| json!({"name":argument.name,"value":argument.value}))
                .collect();
            let mut sequence = Session::new();
            sequence.add_document(initial.clone(), None).unwrap();
            sequence.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":arguments})).unwrap();
            let half_edit = (*sequence.active().unwrap().doc).clone();
            assert_eq!(half_edit.xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Half);
            sequence.execute("image.mode.bits32", json!({})).unwrap();
            let promoted = (*sequence.active().unwrap().doc).clone();
            assert_eq!(sequence.active().unwrap().history.past_len(), 2);
            assert_eq!(promoted.layers, initial.layers);
            assert_eq!(promoted.source_color_context, initial.source_color_context);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(&promoted, promoted.bounds(), &mut |_| Ok(())).unwrap().px,
                photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap().px
            );
            let bytes = photocraft_format::save_to_bytes(&promoted, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), promoted);
            sequence.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*sequence.active().unwrap().doc, half_edit);
            sequence.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*sequence.active().unwrap().doc, initial);
        }
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("image.mode.bits32", json!({})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        assert_eq!(current.xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Canonical);
        assert_eq!(current.layers, source.layers);
        assert_eq!(
            current.layers[0].surface().unwrap().try_read_interleaved(current.bounds()).unwrap(),
            reference.layers[0].surface().unwrap().try_read_interleaved(reference.bounds()).unwrap()
        );
        assert_eq!(current.source_color_context, source.source_color_context);
        assert_eq!(current.xcf_state.as_ref().unwrap().opaque, source.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap().px
        );
        let bytes = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), current);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        session.execute("edit.undo", json!({})).unwrap();
        let filter = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        session.execute("xcf.filter.remove", json!({"filterId":filter.0.to_string()})).unwrap();
        let removed = (*session.active().unwrap().doc).clone();
        assert_eq!(removed.xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Canonical);
        assert_eq!(removed.layers, source.layers);
        assert!(removed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        let bytes = photocraft_format::save_to_bytes(&removed, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), removed);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        count += 1;
    }
    assert_eq!(count, 24);
}
