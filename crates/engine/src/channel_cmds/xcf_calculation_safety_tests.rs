//! Safety regression drafts; unrun during implementation-only work.
use super::*;
use photocraft_doc::{SampleType, Size};
use photocraft_raster::Interrupt;
fn fixture() -> Session {
    let mut doc = Document::new("Target", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("Empty", doc.pixel_format()));
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    let source = Document::with_background("Source", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    session.add_document(source, None).unwrap();
    session.set_active(0);
    session
}
#[test]
fn supplied_bad_opacity_blend_and_context_leave_real_redo_untouched() {
    let mut session = fixture();
    let p = json!({"source":{"document":1},"blending":"normal"});
    apply_image_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
    session.execute("edit.undo", json!({})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    for p in [
        json!({"opacity":"100"}),
        json!({"opacity":null}),
        json!({"scale":false}),
        json!({"offset":"bad"}),
        json!({"blending":42}),
        json!({"preserveTransparency":1}),
        json!({"target":{"channel":-1}}),
    ] {
        assert!(apply_image_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(calculations_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        assert_eq!(session.active().unwrap().doc, before);
    }
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.layers[0].surface().unwrap().read_region(session.active().unwrap().doc.bounds()), vec![1.0; 4]);
}
#[test]
fn transparent_target_copy_has_positive_alpha_and_exact_legacy_pixels() {
    let mut session = fixture();
    apply_image_with_interrupt(&mut session, &json!({"source":{"document":1},"blending":"multiply","opacity":100}), &Interrupt::NONE).unwrap();
    let active = session.active().unwrap();
    assert_eq!(active.doc.layers[0].surface().unwrap().read_region(active.doc.bounds()), vec![1.0; 4]);
}
#[test]
fn actual_length_nonfinite_masks_and_cancel_are_graceful() {
    let control = Interrupt::NONE;
    assert!(calculation_planes(&[], 1, "image.applyImage", &control).is_err());
    assert!(calculation_planes(&[vec![0.0, 1.0]], 1, "image.applyImage", &control).is_err());
    assert!(calculation_values(&[f32::NAN], 1, true, "image.applyImage", &control).is_err());
    assert!(calculation_values(&[2.0], 1, true, "image.applyImage", &control).is_err());
    let mut session = fixture();
    let before = session.active().unwrap().doc.clone();
    let cancelled = || true;
    assert!(matches!(apply_image_with_interrupt(&mut session, &json!({}), &Interrupt::cancel_only(&cancelled)), Err(EngineError::Cancelled)));
    assert_eq!(session.active().unwrap().doc, before);
    assert!(matches!(calculations_with_interrupt(&mut session, &json!({}), &Interrupt::cancel_only(&cancelled)), Err(EngineError::Cancelled)));
    assert_eq!(session.active().unwrap().doc, before);
}
#[test]
fn wide_source_never_narrows_and_nonfinite_target_never_publishes() {
    let mut session = fixture();
    let before = session.active().unwrap().doc.clone();
    let wide = Document::with_background("Wide", Size::new(1, 1), ColorMode::Rgb, SampleType::F64, Color::WHITE);
    session.add_document(wide, None).unwrap();
    session.set_active(0);
    assert!(apply_image_with_interrupt(&mut session, &json!({"source":{"document":2}}), &Interrupt::NONE).is_err());
    assert_eq!(session.active().unwrap().doc, before);
    let format = PixelFormat::RGBA32F;
    let bytes = [f32::NAN.to_ne_bytes(), 0.0f32.to_ne_bytes(), 0.0f32.to_ne_bytes(), 1.0f32.to_ne_bytes()].concat();
    let surface = Surface::try_with_default_bytes(format, &bytes).unwrap();
    assert!(calculation_read(&surface, Rect::new(0, 0, 1, 1), &Interrupt::NONE).is_err());
    assert_eq!(surface.default_bytes(), bytes);
}

#[test]
fn null_defaults_remain_valid_and_numeric_tree_has_a_total_visit_bound() {
    let mut apply = fixture();
    assert!(apply_image_with_interrupt(&mut apply, &Value::Null, &Interrupt::NONE).is_ok());
    let mut calculations = fixture();
    assert!(calculations_with_interrupt(&mut calculations, &Value::Null, &Interrupt::NONE).is_ok());
    fn numeric_tree(depth: usize) -> Value {
        if depth == 0 {
            return json!(1);
        }
        let mut object = serde_json::Map::new();
        for index in 0..10 {
            object.insert(index.to_string(), numeric_tree(depth - 1));
        }
        Value::Object(object)
    }
    let tree = numeric_tree(4);
    let mut bytes = 0;
    let mut visits = 0;
    assert!(calculation_shape(&tree, 0, &mut bytes, &mut visits, "image.applyImage").is_err());
    assert!(visits <= 4096);
    assert!(bytes > 0);
}

fn native_fixture(sample: SampleType, top: &[u8], base: &[u8]) -> Session {
    let mut doc = Document::with_background("Native", Size::new(1, 1), ColorMode::Rgb, sample, Color::BLACK);
    let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let mut first = AlphaChannel::new("Top", Surface::try_with_default_bytes(format, top).unwrap());
    first.id = photocraft_doc::item_identity::ChannelId(9007199254740993);
    let mut surface = Surface::try_with_default_bytes(format, base).unwrap();
    surface.try_write_interleaved_with(Rect::new(0, 0, 1, 1), base, &Interrupt::NONE).unwrap();
    let mut second = AlphaChannel::new("Base", surface);
    second.id = photocraft_doc::item_identity::ChannelId(u64::MAX);
    doc.channels.extend([first, second]);
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    s
}
fn native_request(s: &Session, blend: &str) -> Value {
    let active = s.active().unwrap();
    json!({"nativeCoverage":true,"coverageContext":"unit","documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"source1":{"channelId":active.doc.channels[0].id.0.to_string()},"source2":{"channelId":active.doc.channels[1].id.0.to_string()},"blending":blend,"result":"newChannel"})
}
#[test]
fn native_multiply_identity_preserves_u32_f64_words_cow_and_one_undo() {
    for (sample, top, base) in [
        (SampleType::U32, u32::MAX.to_ne_bytes().to_vec(), 1u32.to_ne_bytes().to_vec()),
        (SampleType::F64, 1f64.to_ne_bytes().to_vec(), (0.5f64.to_bits() + 1).to_ne_bytes().to_vec()),
    ] {
        let mut session = native_fixture(sample, &top, &base);
        let before = session.active().unwrap().doc.clone();
        let p = native_request(&session, "multiply");
        calculations_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
        let after = session.active().unwrap().doc.clone();
        assert_eq!(after.channels.len(), 3);
        assert_eq!(&after.channels[..2], &before.channels[..]);
        assert_eq!(after.channels[2].surface.try_read_interleaved(after.bounds()).unwrap(), base);
        assert!(std::sync::Arc::ptr_eq(
            after.channels[2].surface.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap(),
            before.channels[1].surface.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap()
        ));
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        let restored = photocraft_format::load_from_bytes(&bytes).unwrap();
        // The fixture deliberately stores u64::MAX, so native loading follows
        // its existing all-live-ID rekey policy rather than reserving that ID.
        // There are no relationship fields in this exact fixture to normalize:
        // only the document, one raster layer and three channels get new IDs.
        assert!(after.xcf_state.is_none() && after.source_color_context.is_none());
        assert!(after.selection_channel_id.is_none() && after.quick_mask.is_none());
        assert!(after.paths.is_empty() && after.layer_comps.is_empty());
        assert!(after.last_document_state.is_none() && after.timeline.is_none());
        assert_eq!(after.layers.len(), 1);
        assert!(matches!(after.layers[0].content, LayerContent::Raster(_)));
        assert_eq!(restored.layers.len(), 1);
        assert_eq!(restored.channels.len(), 3);
        assert_ne!(restored.id, after.id);
        assert_ne!(restored.layers[0].id, after.layers[0].id);
        let old_ids: std::collections::BTreeSet<_> = after.channels.iter().map(|c| c.id).collect();
        let new_ids: std::collections::BTreeSet<_> = restored.channels.iter().map(|c| c.id).collect();
        assert_eq!(old_ids.len(), 3);
        assert_eq!(new_ids.len(), 3, "each live channel retains its distinct identity");
        assert!(new_ids.iter().all(|id| id.0 != 0 && !old_ids.contains(id)));
        let mut expected = (*after).clone();
        expected.id = restored.id;
        expected.layers[0].id = restored.layers[0].id;
        for (channel, restored_channel) in expected.channels.iter_mut().zip(&restored.channels) {
            channel.id = restored_channel.id;
        }
        assert_eq!(restored, expected, "every non-ID native/source/metadata/sample field stays exact");
        assert_eq!(restored.channels[2].surface.try_read_interleaved(restored.bounds()).unwrap(), base);
        // Rekeyed IDs are now ordinary live IDs: another save/load preserves
        // them and cannot resurrect either historical huge channel identity.
        let again = photocraft_format::save_to_bytes(&restored, &Default::default()).unwrap();
        let again = photocraft_format::load_from_bytes(&again).unwrap();
        assert_eq!(again, restored);
        assert_eq!(again.channels[2].surface.try_read_interleaved(again.bounds()).unwrap(), base);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, after);
    }
}
#[test]
fn native_signed_zero_min_and_bad_configs_preserve_sources() {
    let mut session = native_fixture(SampleType::F64, &0f64.to_ne_bytes(), &(-0f64).to_ne_bytes());
    let p = native_request(&session, "darken");
    calculations_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
    assert_eq!(session.active().unwrap().doc.channels[2].surface.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), (-0f64).to_ne_bytes());
    let mut session = native_fixture(SampleType::U32, &u32::MAX.to_ne_bytes(), &1u32.to_ne_bytes());
    let before = session.active().unwrap().doc.clone();
    for (key, value) in [
        ("opacity", json!(50)),
        ("scale", json!(2)),
        ("offset", json!(1)),
        ("mask", json!(false)),
        ("coverageContext", json!("color")),
        ("result", json!("selection")),
        ("expectedRevision", json!("999")),
    ] {
        let mut p = native_request(&session, "multiply");
        p[key] = value;
        assert!(calculations_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        assert_eq!(session.active().unwrap().doc, before);
    }
    let p = native_request(&session, "multiply");
    let cancel = || true;
    assert!(matches!(calculations_with_interrupt(&mut session, &p, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
    assert_eq!(session.active().unwrap().doc, before);
}
#[test]
fn native_imported_missing_geometry_and_nonunit_default_never_fall_back() {
    let mut session = native_fixture(SampleType::F64, &1f64.to_ne_bytes(), &2f64.to_ne_bytes());
    let before = session.active().unwrap().doc.clone();
    let p = native_request(&session, "multiply");
    assert!(calculations_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
    assert_eq!(session.active().unwrap().doc, before);
    let mut session = native_fixture(SampleType::U32, &u32::MAX.to_ne_bytes(), &1u32.to_ne_bytes());
    {
        let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(doc).unwrap();
        doc.xcf_state = Some(state);
    }
    let before = session.active().unwrap().doc.clone();
    let p = native_request(&session, "multiply");
    assert!(calculations_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
    assert_eq!(session.active().unwrap().doc, before);
}
