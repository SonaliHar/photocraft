#![cfg(feature = "corpus")]
#[path = "../../io/tests/common/video_options.rs"]
mod packet;
use photocraft_engine::Session;
use serde_json::json;

#[test]
#[ignore = "immutable temp independently authored Video options command controls"]
fn actual_new_epoch_edit_and_enable() {
    let observations = packet::observations().unwrap();
    let rows = observations["records"].as_array().unwrap();
    let load = |id: &str| {
        let row = rows.iter().find(|row| row["id"] == id).unwrap();
        photocraft_io::import(row["file"].as_str().unwrap(), &packet::blob(&packet::root(), row).unwrap()).unwrap().document
    };
    let source = load("staggered.add0.rot0.source");
    let edited = load("staggered.add0.rot0.edit");
    let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.schema_epoch, packet::EPOCH);
    let id = node.id.0.to_string();
    let arguments = edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.clone();
    let mut session = Session::new();
    session.add_document(source.clone(), None).unwrap();
    let edit = session.execute("xcf.filter.set", json!({"filterId":id,"arguments":arguments}));
    eprintln!("Actual NEW saved source cyclic edit: {edit:?}");
    if edit.is_err() {
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    let mut disable_session = Session::new();
    disable_session.add_document(source.clone(), None).unwrap();
    let enable = disable_session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false}));
    eprintln!("Actual NEW saved source disable: {enable:?}");
    if enable.is_err() {
        assert_eq!(*disable_session.active().unwrap().doc, source);
        assert_eq!(disable_session.active().unwrap().history.past_len(), 0);
    }
    assert!(edit.is_ok() && enable.is_ok(), "saved NEW epoch edit and disable must be admitted");
}

fn projection(document: &photocraft_doc::Document, expected: &[u8]) {
    let actual = photocraft_compose::gimp_render::try_render(document, document.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(expected.len(), actual.px.len() * 32);
    for (pixel, raw) in actual.px.iter().zip(expected.as_chunks::<32>().0.iter()) {
        let reference: Vec<_> = raw.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect();
        assert!((pixel[3] - reference[3]).abs() <= 1e-6);
        for c in 0..3 {
            assert!((pixel[c] * pixel[3] - reference[c] * reference[3]).abs() <= 1e-6);
        }
    }
}
fn immutable(current: &photocraft_doc::Document, source: &photocraft_doc::Document) {
    assert_eq!(current.layers, source.layers);
    assert_eq!(current.icc_profile, source.icc_profile);
    assert_eq!(current.source_color_context, source.source_color_context);
    let (a, b) = (current.xcf_state.as_ref().unwrap(), source.xcf_state.as_ref().unwrap());
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.resources, b.resources);
}
#[test]
#[ignore = "immutable temp Video36 cyclic edits/disable and atomic history/native controls"]
fn actual_36_cyclic_edits_disable_native_history_cancel_and_invalid_parameters() {
    let observations = packet::observations().unwrap();
    let rows = observations["records"].as_array().unwrap();
    let root = packet::root();
    let mut count = 0;
    for row in rows.iter().filter(|row| row["role"] == "source") {
        let name = row["id"].as_str().unwrap().strip_suffix(".source").unwrap();
        let find = |role: &str| rows.iter().find(|row| row["id"] == format!("{name}.{role}")).unwrap();
        let source_bytes = packet::blob(&root, row).unwrap();
        let source = photocraft_io::import(row["file"].as_str().unwrap(), &source_bytes).unwrap().document;
        let edited = photocraft_io::import(find("edit")["file"].as_str().unwrap(), &packet::blob(&root, find("edit")).unwrap()).unwrap().document;
        let original = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(original));
        let id = original.id.0.to_string();
        let arguments = edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.clone();
        assert_ne!(arguments, original.arguments);
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        let no_op = session.execute("xcf.filter.set", json!({"filterId":id,"arguments":original.arguments})).unwrap();
        assert_eq!(no_op["updated"], false);
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        session.execute("xcf.filter.set", json!({"filterId":id,"arguments":arguments})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        immutable(&current, &source);
        projection(&current, &packet::blob(&root, &find("edit")["current"]["projection"]).unwrap());
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened, current);
        projection(&reopened, &packet::blob(&root, &find("edit")["current"]["projection"]).unwrap());
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        session.execute("edit.undo", json!({})).unwrap();
        session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap();
        let disabled = (*session.active().unwrap().doc).clone();
        immutable(&disabled, &source);
        let no_op = session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap();
        assert_eq!(no_op["updated"], false);
        assert_eq!(*session.active().unwrap().doc, disabled);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        projection(&disabled, &packet::blob(&root, &find("disabled")["current"]["projection"]).unwrap());
        let native = photocraft_format::save_to_bytes(&disabled, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), disabled);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, disabled);
        session.execute("edit.undo", json!({})).unwrap();
        for late in [false, true] {
            let polls = std::sync::atomic::AtomicUsize::new(0);
            let cancel = || {
                let old = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                !late || old >= 2
            };
            assert!(matches!(
                photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                    &mut session,
                    &json!({"filterId":id,"arguments":arguments}),
                    &photocraft_raster::Interrupt::cancel_only(&cancel)
                ),
                Err(photocraft_engine::EngineError::Cancelled)
            ));
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
        }
        for mutation in 0..5 {
            let mut bad = arguments.clone();
            let pattern = bad.iter_mut().find(|arg| arg.name == "pattern").unwrap();
            pattern.value = match mutation {
                0 => photocraft_doc::xcf_filters::FilterValue::Enum { domain: "gegl:video-degradation/pattern".into(), value: 9 },
                1 => photocraft_doc::xcf_filters::FilterValue::Enum { domain: "wrong".into(), value: 0 },
                2 => photocraft_doc::xcf_filters::FilterValue::Bool(false),
                3 => photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::NAN.to_bits())),
                _ => photocraft_doc::xcf_filters::FilterValue::Config(vec![]),
            };
            assert!(session.execute("xcf.filter.set", json!({"filterId":id,"arguments":bad})).is_err());
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
        }
        assert_eq!(packet::blob(&root, row).unwrap(), source_bytes);
        count += 1;
    }
    assert_eq!(count, 36);
}

#[test]
#[ignore = "Video options commands must reject swapped source/current envelopes atomically"]
fn swapped_current_envelopes_and_content_locks_are_atomic() {
    let observations = packet::observations().unwrap();
    let rows = observations["records"].as_array().unwrap();
    let source = photocraft_io::import(rows[0]["file"].as_str().unwrap(), &packet::blob(&packet::root(), &rows[0]).unwrap()).unwrap().document;
    let edited = photocraft_io::import(rows[1]["file"].as_str().unwrap(), &packet::blob(&packet::root(), &rows[1]).unwrap()).unwrap().document;
    let original = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let id = original.id.0.to_string();
    let arguments = edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.clone();
    for mutation in 0..9 {
        let mut bad = source.clone();
        match mutation {
            0 => {
                bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch =
                    photocraft_compose::gimp_video_degradation::VIDEO_DEGRADATION_EPOCH.into()
            }
            1 => {
                bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch =
                    "xcf-22-stock-point-replace-fields-v1".into()
            }
            2 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = -1877441967,
            3 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_composite_space = 0,
            4 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
            5 => bad.xcf_state.as_mut().unwrap().filter_masks.clear(),
            6 => bad.size = photocraft_geom::Size::new(17, 16),
            7 => bad.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear),
            8 => {
                let owner = bad.layers[0].id;
                bad.xcf_state
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
                    .unwrap()
                    .locks
                    .content = true;
            }
            _ => panic!("unexpected mutation ordinal"),
        }
        let mut session = Session::new();
        // Native structural validation may refuse an inconsistent mask reference
        // before command dispatch; every admitted hostile context is staged below.
        if session.add_document(bad.clone(), None).is_err() {
            continue;
        }
        assert!(session.execute("xcf.filter.set", json!({"filterId":id,"arguments":arguments})).is_err(), "mutation{mutation}");
        assert_eq!(*session.active().unwrap().doc, bad);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
}
