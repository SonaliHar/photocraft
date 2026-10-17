#![cfg(feature = "corpus")]
//! Synthetic OLD-epoch refusals do not qualify a new owner/source domain.
#[path = "../../io/tests/common/red_eye_geometry_packet.rs"]
mod packet;
use photocraft_doc::{Document, LayerContent, Rect, Size, item_identity::ItemRef};
use photocraft_engine::Session;
use photocraft_raster::Surface;
use serde_json::json;

#[test]
#[ignore = "actual six owner-geometry originals with command and atomic history refusals"]
fn actual_owner_geometry_hostiles_locks_and_cancellation_are_atomic() {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let (root, records) = packet::fresh_records().unwrap();
    let mut count = 0;
    for row in records {
        let source = photocraft_io::import("actual-geometry.xcf", &packet::fresh_artifact(&root, &row["saved"]).unwrap()).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let fid = node.id.0.to_string();
        for value in [
            FilterValue::Bool(true),
            FilterValue::Number(ExactNumber::Float64Bits(0.675f64.to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits(f32::INFINITY.to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits((-0.1f32).to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits(0.8f32.to_bits())),
        ] {
            let mut args = node.arguments.clone();
            args[0].value = value;
            assert_atomic_refusal(source.clone(), "xcf.filter.set", json!({"filterId":fid,"arguments":args})).unwrap();
            count += 1;
        }
        for params in [
            json!({"filterId":fid,"arguments":[]}),
            json!({"filterId":fid,"arguments":node.arguments,"unknown":true}),
            json!({"filterId":"999999","arguments":node.arguments}),
        ] {
            assert_atomic_refusal(source.clone(), "xcf.filter.set", params).unwrap();
            count += 1;
        }
        let mut locked = source.clone();
        let owner = locked.layers[0].id;
        locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().locks.content = true;
        let mut args = node.arguments.clone();
        args[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits()));
        assert_atomic_refusal(locked, "xcf.filter.set", json!({"filterId":fid,"arguments":args})).unwrap();
        count += 1;
        for mask in [false, true] {
            let mut bad = source.clone();
            let state = bad.xcf_state.as_mut().unwrap();
            let reference = if mask { ItemRef::Mask(state.filter_masks[0].id) } else { ItemRef::Layer(owner) };
            state.items.iter_mut().find(|item| item.item == reference).unwrap().source_flags = 1;
            assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&bad).is_err());
            assert_atomic_refusal(bad, "xcf.filter.set", json!({"filterId":fid,"arguments":args})).unwrap();
            count += 1;
        }
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session
            .edit("Cancelled redo witness", |doc, _| {
                doc.layers[0].name.push_str(" redo");
                Ok(())
            })
            .unwrap();
        let redo = session.active().unwrap().doc.clone();
        session.execute("edit.undo", json!({})).unwrap();
        let revision = session.active().unwrap().revision;
        let history = session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut session,
                &json!({"filterId":fid,"arguments":args}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), history);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, redo);
        count += 1;
    }
    assert_eq!(count, 78);
    eprintln!("6 actual enabled/disabled sources: 78 command/current/lock/cancellation atomic refusals");
}

#[test]
#[ignore = "six frozen independent TEMP Red Eye owner geometry controls"]
fn actual_owner_geometry_registered_edits_preserve_source_and_history() {
    let (root, records) = packet::fresh_records().unwrap();
    let mut rejected = Vec::new();
    for geometry in packet::GEOMETRIES {
        let source_row = records.iter().find(|row| row["id"] == format!("{}.source", geometry.id)).unwrap();
        let bytes = packet::fresh_artifact(&root, &source_row["saved"]).unwrap();
        let source = photocraft_io::import("actual-owner-source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(node.operation.schema_epoch, packet::NEW_EPOCH);
        for action in ["edit", "disabled", "reenabled"] {
            let target_action = if action == "reenabled" { "source" } else { action };
            let row = records.iter().find(|row| row["id"] == format!("{}.{target_action}", geometry.id)).unwrap();
            let target_bytes = packet::fresh_artifact(&root, &row["saved"]).unwrap();
            let target = photocraft_io::import("actual-owner-target.xcf", &target_bytes).unwrap().document;
            let desired = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let initial = if action == "reenabled" {
                let disabled = records.iter().find(|row| row["id"] == format!("{}.disabled", geometry.id)).unwrap();
                photocraft_io::import("actual-disabled-owner.xcf", &packet::fresh_artifact(&root, &disabled["saved"]).unwrap()).unwrap().document
            } else {
                source.clone()
            };
            let fid = initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
            let mut session = Session::new();
            session.add_document(initial.clone(), None).unwrap();
            let (command, params) = if action == "edit" {
                ("xcf.filter.set", json!({"filterId":fid,"arguments":desired.arguments}))
            } else {
                ("xcf.filter.enable", json!({"filterId":fid,"enabled":action == "reenabled"}))
            };
            match session.execute(command, params) {
                Err(error) => {
                    eprintln!("ACTUAL ENGINE RED {}.{action}: {error}", geometry.id);
                    rejected.push(format!("{}.{action}", geometry.id));
                }
                Ok(_) => {
                    let current = (*session.active().unwrap().doc).clone();
                    assert_eq!(current.layers, initial.layers, "full offcanvas native owner unchanged");
                    assert_eq!(current.source_color_context, initial.source_color_context);
                    let current_state = current.xcf_state.as_ref().unwrap();
                    let source_state = initial.xcf_state.as_ref().unwrap();
                    assert_eq!(current_state.opaque, source_state.opaque);
                    assert_eq!(current_state.provenance, source_state.provenance);
                    assert_eq!(current_state.filter_masks, source_state.filter_masks);
                    assert_eq!(current_state.filter_stacks[0].nodes[0].arguments, desired.arguments);
                    assert_eq!(current_state.filter_stacks[0].nodes[0].enabled, desired.enabled);
                    assert_eq!(
                        photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap().px,
                        photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px,
                        "registered current output must match separately authored target (independent references verified by IO suite)"
                    );
                    assert_eq!(session.active().unwrap().history.past_len(), 1);
                    let saved = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
                    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), current);
                    let (stream, _) = photocraft_format::PcraftWriter::new()
                        .save_zip_stream(
                            &current,
                            &Default::default(),
                            std::io::Cursor::new(Vec::new()),
                            Default::default(),
                            64 << 20,
                            &photocraft_raster::Interrupt::NONE,
                        )
                        .unwrap();
                    assert_eq!(
                        photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap(),
                        current
                    );
                    session.execute("edit.undo", json!({})).unwrap();
                    assert_eq!(*session.active().unwrap().doc, initial);
                    session.execute("edit.redo", json!({})).unwrap();
                    assert_eq!(*session.active().unwrap().doc, current);
                }
            }
        }
    }
    assert!(rejected.is_empty(), "actual registered edits refused: {rejected:?}");
}

fn synthetic_current(geometry: packet::Geometry, enabled: bool) -> packet::TestResult<Document> {
    let mut doc = photocraft_io::import("old-qualified-red-eye.xcf", &packet::old_source()?)?.document;
    let native = packet::synthetic_native(geometry.owner)?;
    let extent = Rect::from_xywh(geometry.offset.0, geometry.offset.1, geometry.owner.0, geometry.owner.1);
    let layer = doc.layers.first_mut().ok_or("old layer")?;
    let owner = layer.id;
    let format = layer.surface().ok_or("old raster")?.format();
    let mut surface = Surface::new(format);
    surface.try_write_interleaved(extent, &native)?;
    layer.content = LayerContent::Raster(surface);
    doc.size = Size::new(geometry.canvas.0, geometry.canvas.1);
    let canvas = doc.bounds();
    let state = doc.xcf_state.as_mut().ok_or("old XCF state")?;
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).ok_or("old owner item")?.raster_bounds = Some(extent);
    let node = state.filter_stacks.first_mut().and_then(|stack| stack.nodes.first_mut()).ok_or("old node")?;
    node.enabled = enabled;
    let mask = state.filter_masks.first_mut().ok_or("old mask")?;
    mask.bounds = canvas;
    mask.surface = Surface::new(mask.surface.format());
    let item = state.items.iter_mut().find(|item| item.item == ItemRef::Mask(mask.id)).ok_or("old mask item")?;
    item.raster_bounds = Some(canvas);
    item.mask.as_mut().ok_or("old mask metadata")?.bounds = canvas;
    doc.validate_xcf_state()?;
    Ok(doc)
}

fn assert_atomic_refusal(doc: Document, command: &str, params: serde_json::Value) -> packet::TestResult<()> {
    let mut session = Session::new();
    session.add_document(doc.clone(), None)?;
    session.edit("Synthetic redo witness", |doc, _| {
        doc.layers[0].name.push_str(" redo");
        Ok(())
    })?;
    let redo = session.active().ok_or("redo document")?.doc.clone();
    session.execute("edit.undo", json!({}))?;
    let before = session.active().ok_or("active document")?;
    let revision = before.revision;
    let saved_revision = before.saved_revision;
    let history = before.history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
    let redo_labels = before.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
    let result = session.execute(command, params);
    if result.is_ok() {
        return Err("synthetic OLD epoch accepted unqualified geometry".into());
    }
    let after = session.active().ok_or("retained document")?;
    if *after.doc != doc
        || after.revision != revision
        || after.saved_revision != saved_revision
        || after.history.entry_labels().map(str::to_owned).collect::<Vec<_>>() != history
        || after.history.redo_labels().map(str::to_owned).collect::<Vec<_>>() != redo_labels
    {
        return Err("rejected geometry mutated document/history/redo".into());
    }
    session.execute("edit.redo", json!({}))?;
    if session.active().ok_or("continued redo")?.doc != redo {
        return Err("refusal replaced redo snapshot".into());
    }
    Ok(())
}

#[test]
#[ignore = "old hash-bound Red Eye control reshaped only for preincrement current refusal"]
fn old_epoch_geometry_is_refused_enabled_and_disabled_without_native_loss() {
    let old = photocraft_io::import("old-control.xcf", &packet::old_source().unwrap()).unwrap().document;
    assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&old).is_ok());
    for geometry in packet::GEOMETRIES {
        for enabled in [true, false] {
            let doc = synthetic_current(geometry, enabled).unwrap();
            let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(node.operation.schema_epoch, packet::OLD_EPOCH);
            assert_ne!(node.operation.schema_epoch, packet::NEW_EPOCH);
            assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, packet::OLD_POLICY);
            let extent = Rect::from_xywh(geometry.offset.0, geometry.offset.1, geometry.owner.0, geometry.owner.1);
            let native = doc.layers[0].surface().unwrap().try_read_interleaved(extent).unwrap();
            assert_eq!(native, packet::synthetic_native(geometry.owner).unwrap());
            let result = photocraft_compose::gimp_render::PreparedRenderer::new(&doc);
            assert!(result.is_err(), "{} enabled={enabled}", geometry.id);
            let refusal = result.err().unwrap();
            eprintln!("DESIGN REFUSAL {} enabled={enabled}: {refusal}; full owner native bytes={} (not a GIMP oracle)", geometry.id, native.len());
            for (command, params) in [
                (
                    "xcf.filter.set",
                    json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"threshold","value":{"Number":{"Float32Bits":0.625f32.to_bits()}}}]}),
                ),
                ("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":!enabled})),
            ] {
                assert_atomic_refusal(doc.clone(), command, params).unwrap();
            }
            let raw = packet::synthetic_decoded(geometry, enabled).unwrap();
            assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, native);
        }
    }
}

#[test]
#[ignore = "synthetic unqualified geometry must archive exactly without renderer admission"]
fn refused_old_geometry_legacy_and_zip64_reopen_preserve_whole_owner() {
    for geometry in packet::GEOMETRIES {
        let doc = synthetic_current(geometry, true).unwrap();
        let extent = Rect::from_xywh(geometry.offset.0, geometry.offset.1, geometry.owner.0, geometry.owner.1);
        let original = doc.layers[0].surface().unwrap().try_read_interleaved(extent).unwrap();
        let legacy = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        for reopened in [&legacy, &streamed] {
            assert_eq!(reopened, &doc);
            assert_eq!(reopened.layers[0].surface().unwrap().try_read_interleaved(extent).unwrap(), original);
            assert!(photocraft_compose::gimp_render::PreparedRenderer::new(reopened).is_err());
        }
    }
}
