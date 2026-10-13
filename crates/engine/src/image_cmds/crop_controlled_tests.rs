//! Controlled Crop regressions; authored for the coordinated validation batch.
use super::*;
use photocraft_raster::Interrupt;

fn fixture() -> Session {
    let mut document = Document::new("crop", Size::new(8, 6), ColorMode::Rgb, SampleType::U32);
    let mut surface = Surface::new(photocraft_color::PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U32, alpha: true });
    let words: Vec<u8> = [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect();
    surface.try_write_interleaved_with(Rect::new(2, 1, 3, 2), &words, &Interrupt::NONE).unwrap();
    document.layers.push(Layer::new("raw", LayerContent::Raster(surface)));
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    session
}
#[test]
fn malformed_rectangle_does_not_fall_back_or_consume_redo() {
    let mut session = fixture();
    session
        .edit("seed", |doc, _| {
            doc.name = "seed".into();
            Ok(())
        })
        .unwrap();
    session.execute("edit.undo", Value::Null).unwrap();
    let state = session.active().unwrap();
    let before = state.doc.clone();
    let revision = state.revision;
    let redo: Vec<_> = state.history.redo_labels().map(str::to_owned).collect();
    for params in [
        json!({"x":1}),
        json!({"x":0,"y":0,"width":0,"height":1}),
        json!({"x":false,"y":0,"width":1,"height":1}),
        json!({"deleteCroppedPixels":0}),
        json!({"x":i32::MAX,"y":0,"width":1,"height":1}),
    ] {
        assert!(session.execute("image.crop", params).is_err());
        let state = session.active().unwrap();
        assert!(state.doc.shares_state(&before));
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
    }
}
#[test]
fn wide_crop_retains_low_bits_and_one_undo() {
    let mut session = fixture();
    let before = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
    session.execute("image.crop", json!({"x":1,"y":0,"width":4,"height":3,"deleteCroppedPixels":false})).unwrap();
    let expected: Vec<_> = [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect();
    assert_eq!(session.active().unwrap().doc.layers[0].surface().unwrap().to_interleaved(Rect::new(1, 1, 2, 2)), expected);
    session.execute("edit.undo", Value::Null).unwrap();
    assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), before);
}
#[test]
fn floating_raw_words_and_default_survive_sparse_crop() {
    let format = photocraft_color::PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F64, alpha: true };
    let mut source = Surface::new(format);
    let raw: Vec<_> = [1f64.to_bits() + 1, (-0.0f64).to_bits(), 0x7ff8_0000_0000_1234, 1f64.to_bits()].into_iter().flat_map(u64::to_le_bytes).collect();
    source.try_write_interleaved_with(Rect::new(-2, 1, -1, 2), &raw, &Interrupt::NONE).unwrap();
    let before = source.clone();
    let output = crop_surface_with_interrupt(&source, Rect::new(-2, 0, 2, 3), &Interrupt::NONE).unwrap();
    assert_eq!(output.to_interleaved(Rect::new(-2, 1, -1, 2)), raw);
    assert_eq!(output.default_bytes(), before.default_bytes());
    assert_eq!(source.to_interleaved(Rect::new(-2, 1, -1, 2)), raw);
    assert!(matches!(crop_surface_with_interrupt(&source, Rect::new(-2, 0, 2, 3), &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
}
#[test]
fn far_off_deleting_crop_uses_post_crop_bounds_and_cancel_keeps_original() {
    let mut session = fixture();
    let original = session.active().unwrap().doc.clone();
    let r = Rect::new(i32::MIN + 1, 0, i32::MIN + 2, 1);
    crop_prepare(&original, r, true, &Interrupt::NONE).unwrap();
    assert!(crop_prepare(&original, r, false, &Interrupt::NONE).is_err());
    assert!(matches!(crop_prepare(&original, Rect::new(0, 0, 4, 4), true, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    assert!(session.active().unwrap().doc.shares_state(&original));
    session.execute("image.crop", json!({"x":i32::MIN+1,"y":0,"width":1,"height":1})).unwrap();
    assert_eq!(session.active().unwrap().doc.size, Size::new(1, 1));
}

fn indexed_fixture() -> Session {
    use photocraft_doc::{ColorTable, EditableIndexedLayer, EditableIndexedRaster};
    let raster = EditableIndexedRaster::try_new(
        Rect::new(-1, 1, 1, 3),
        std::sync::Arc::new(vec![0, 1, 2, 1]),
        Some(std::sync::Arc::new(vec![255, 128, 7, 0])),
        vec![[8, 9, 10], [8, 9, 10], [2, 3, 4]],
    )
    .unwrap();
    let mut doc = Document::new("indexed crop", Size::new(4, 4), ColorMode::Indexed, SampleType::U8);
    doc.color_table = Some(ColorTable { colors: raster.palette().to_vec(), transparent: Some(1) });
    let carrier = crate::xcf_editing::expanded_with(&raster, photocraft_color::PixelFormat::RGBA8, Some(1), &Interrupt::NONE).unwrap();
    let layer = Layer::new("ordinals", LayerContent::Raster(carrier));
    let id = layer.id;
    doc.layers.push(layer);
    let mut state = photocraft_doc::xcf_state::XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(id)).unwrap().raster_bounds = Some(raster.bounds());
    state.editable_indexed.push(EditableIndexedLayer { owner: id, raster });
    doc.xcf_state = Some(state);
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
#[test]
fn indexed_crop_preserves_duplicate_ordinals_alpha_table_native_and_undo() {
    let mut session = indexed_fixture();
    let original = session.active().unwrap().doc.clone();
    session.execute("image.crop", json!({"x":-1,"y":1,"width":1,"height":2})).unwrap();
    let current = &session.active().unwrap().doc;
    let raster = &current.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
    assert_eq!(raster.bounds(), Rect::new(0, 0, 1, 2));
    assert_eq!(raster.indices().as_slice(), &[0, 2]);
    assert_eq!(raster.alpha().unwrap().as_slice(), &[255, 7]);
    assert_eq!(current.color_table, original.color_table);
    assert_eq!(raster.palette(), original.color_table.as_ref().unwrap().colors);
    let saved = photocraft_format::save_to_bytes(current, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.xcf_state, current.xcf_state);
    session.execute("edit.undo", Value::Null).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state, original.xcf_state);
    session.execute("edit.redo", Value::Null).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state, reopened.xcf_state);
}
#[test]
fn indexed_retention_shares_planes_and_empty_or_cancel_keeps_original() {
    let mut session = indexed_fixture();
    let original = session.active().unwrap().doc.clone();
    let before = &original.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
    session.execute("image.crop", json!({"x":1,"y":1,"width":2,"height":2,"deleteCroppedPixels":false})).unwrap();
    let after = &session.active().unwrap().doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
    assert_eq!(after.bounds(), Rect::new(-2, 0, 0, 2));
    assert!(std::sync::Arc::ptr_eq(before.indices(), after.indices()));
    assert!(std::sync::Arc::ptr_eq(before.alpha().unwrap(), after.alpha().unwrap()));
    session.execute("edit.undo", Value::Null).unwrap();
    let restored = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    assert!(session.execute("image.crop", json!({"x":3,"y":3,"width":1,"height":1})).is_err());
    assert!(session.active().unwrap().doc.shares_state(&restored));
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(matches!(crop_prepare(&restored, Rect::new(-1, 1, 1, 3), true, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    assert!(session.active().unwrap().doc.shares_state(&restored));
}
