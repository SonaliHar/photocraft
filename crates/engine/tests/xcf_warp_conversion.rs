use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size};
use photocraft_engine::Session;
use serde_json::json;

#[test]
fn wide_bilinear_transform_keeps_low_codes_native_state_and_undo() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut document = Document::new("Own wide", Size::new(4, 4), ColorMode::Grayscale, sample);
        let mut layer = Layer::raster("Own pixels", document.pixel_format());
        let bounds = photocraft_doc::Rect::new(-2, -1, 0, 0);
        let raw: Vec<u8> = if sample == SampleType::U32 {
            [1u32, u32::MAX, 3, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect()
        } else {
            [0.5 + 2f64.powi(-40), 1.0, 0.5 + 3.0 * 2f64.powi(-40), 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()
        };
        layer.surface_mut().unwrap().try_write_interleaved(bounds, &raw).unwrap();
        document.layers.push(layer);
        let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(document.clone(), None).unwrap();
        session.execute("edit.transform", json!({"matrix":[1,0,0,1,0.5,0],"interpolation":"bilinear"})).unwrap();
        let active = session.active().unwrap();
        let surface = active.doc.layers[0].surface().unwrap();
        let mut pixel = [0.; 2];
        surface.try_read_pixel_f64(-1, -1, &mut pixel).unwrap();
        let expected = if sample == SampleType::U32 { 2.0 / f64::from(u32::MAX) } else { 0.5 + 2.0 * 2f64.powi(-40) };
        assert_eq!(pixel, [expected, 1.0]);
        let saved = photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), *active.doc);
        session.undo();
        assert_eq!(*session.active().unwrap().doc, document);
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), document);
    }
}

#[test]
fn canonical_raster_transform_tracks_declared_transparent_extent_and_exact_owner_id() {
    use photocraft_doc::{item_identity::ItemRef, xcf_state::XcfState};
    for sample in [SampleType::U32, SampleType::F64] {
        for offset in [1.0, 0.5] {
            let mut document = Document::new("Own canonical", Size::new(8, 8), ColorMode::Grayscale, sample);
            let mut layer = Layer::raster("Own pixels", document.pixel_format());
            let bounds = photocraft_doc::Rect::new(-2, -1, 2, 1);
            let raw: Vec<u8> = if sample == SampleType::U32 {
                [1u32, u32::MAX, 3, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect()
            } else {
                [0.5 + 2f64.powi(-40), 1.0, 0.5 + 3.0 * 2f64.powi(-40), 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()
            };
            layer.surface_mut().unwrap().try_write_interleaved(photocraft_doc::Rect::new(-2, -1, 0, 0), &raw).unwrap();
            let id = layer.id;
            document.layers.push(layer);
            let mut state = XcfState::default();
            state.reconcile_layers(&document).unwrap();
            let item = state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap();
            item.raster_bounds = Some(bounds);
            item.source_tattoo = Some(47);
            document.xcf_state = Some(state);
            let source_native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let mut session = Session::new();
            session.add_document(document.clone(), None).unwrap();
            session.execute("edit.transform", json!({"matrix":[1,0,0,1,offset,0],"interpolation":"bilinear"})).unwrap();
            let edited = &session.active().unwrap().doc;
            let state = edited.xcf_state.as_ref().unwrap();
            let item = state.items.iter().find(|item| item.item == ItemRef::Layer(id)).unwrap();
            assert_eq!(item.raster_bounds, Some(photocraft_doc::Rect::new(if offset == 1.0 { -1 } else { -2 }, -1, 3, 1)));
            assert_eq!(item.source_tattoo, Some(47));
            assert_eq!(edited.layers[0].id, id);
            assert_eq!(state.provenance, document.xcf_state.as_ref().unwrap().provenance);
            let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
            session.undo();
            assert_eq!(*session.active().unwrap().doc, document);
            assert_eq!(photocraft_format::load_from_bytes(&source_native).unwrap(), document);
        }
    }
}

#[test]
fn canonical_linked_mask_geometry_rejects_without_publishing_extent_changes() {
    let mut document = Document::new("Own mask", Size::new(4, 4), ColorMode::Grayscale, SampleType::F64);
    let mut layer = Layer::raster("Own pixels", document.pixel_format());
    layer
        .surface_mut()
        .unwrap()
        .try_write_interleaved(photocraft_doc::Rect::new(0, 0, 1, 1), &[0.25f64, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>())
        .unwrap();
    layer.mask = Some(photocraft_doc::LayerMask::reveal_all());
    document.layers.push(layer);
    let mut state = photocraft_doc::xcf_state::XcfState::default();
    state.reconcile_layers(&document).unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(photocraft_doc::Rect::new(0, 0, 1, 1));
    }
    document.xcf_state = Some(state);
    let mut session = Session::new();
    session.add_document(document.clone(), None).unwrap();
    let past = session.active().unwrap().history.past_len();
    assert!(session.execute("edit.transform", json!({"matrix":[1,0,0,1,0.5,0],"interpolation":"bilinear"})).is_err());
    assert_eq!(*session.active().unwrap().doc, document);
    assert_eq!(session.active().unwrap().history.past_len(), past);
}

#[test]
fn wide_legacy_lens_and_crop_commands_reject_atomically() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut document = Document::new("Own wide", Size::new(4, 4), ColorMode::Rgb, sample);
        let mut layer = Layer::raster("Own pixels", document.pixel_format());
        let bytes: Vec<u8> = if sample == SampleType::U32 {
            [1_u32, 0x80000001, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect()
        } else {
            [-0.0_f64, 0.5 + 2.0_f64.powi(-40), 1.125, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()
        };
        layer.surface_mut().unwrap().try_write_interleaved(photocraft_doc::Rect::new(0, 0, 1, 1), &bytes).unwrap();
        document.layers.push(layer);
        let mut session = Session::new();
        session.add_document(document.clone(), None).unwrap();
        for command in ["filter.adaptiveWideAngle", "filter.lensCorrection", "filter.vanishingPoint", "file.automate.cropAndStraightenPhotos"] {
            let past = session.active().unwrap().history.past_len();
            assert!(session.execute(command, json!({})).is_err(), "{command}");
            assert_eq!(*session.active().unwrap().doc, document, "{command}");
            assert_eq!(session.active().unwrap().history.past_len(), past);
        }
    }
}

#[test]
fn narrow_layer_with_wide_mask_rejects_transform_without_history_or_native_loss() {
    let mut document = Document::new("Own mask", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Own pixels", document.pixel_format());
    layer.surface_mut().unwrap().write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.0]);
    let mut mask = photocraft_doc::LayerMask::reveal_all();
    let format = photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    mask.surface = std::sync::Arc::new(photocraft_raster::Surface::try_with_default_bytes(format, &(-0.0_f64).to_ne_bytes()).unwrap());
    std::sync::Arc::make_mut(&mut mask.surface).try_write_interleaved(photocraft_doc::Rect::new(0, 0, 1, 1), &(0.5 + 2.0_f64.powi(-40)).to_ne_bytes()).unwrap();
    layer.mask = Some(mask);
    document.layers.push(layer);
    let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let mut session = Session::new();
    session.add_document(document.clone(), None).unwrap();
    let past = session.active().unwrap().history.past_len();
    assert!(session.execute("edit.transform", json!({"matrix":[1,0,0,1,0.5,0.5]})).is_err());
    assert_eq!(*session.active().unwrap().doc, document);
    assert_eq!(session.active().unwrap().history.past_len(), past);
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), document);
}

#[test]
fn wide_transform_cancellation_before_commit_keeps_native_pixels_and_history() {
    use photocraft_algo::transform::{Homography, Interp};
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicBool, Ordering};
    for sample in [SampleType::U32, SampleType::F64] {
        let mut document = Document::new("Own cancellable", Size::new(4, 4), ColorMode::Grayscale, sample);
        let mut layer = Layer::raster("Pixels", document.pixel_format());
        let bounds = photocraft_doc::Rect::new(-2, -1, 0, 0);
        let raw: Vec<u8> = if sample == SampleType::U32 {
            [1u32, u32::MAX, 3, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect()
        } else {
            [0.5 + 2f64.powi(-40), 1.0, 0.5 + 3.0 * 2f64.powi(-40), 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()
        };
        layer.surface_mut().unwrap().try_write_interleaved(bounds, &raw).unwrap();
        document.layers.push(layer);
        let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let mut session = Session::new();
        session.add_document(document.clone(), None).unwrap();
        let revision = session.active().unwrap().revision;
        let cancelled = AtomicBool::new(false);
        let cancel = || cancelled.load(Ordering::Relaxed);
        let control = Interrupt::cancel_only(&cancel);
        let result = session.edit_with_interrupt("Cancelled transform", &control, |doc, _| {
            let surface = doc.layers[0].surface_mut().unwrap();
            *surface = photocraft_algo::transform::try_warp_surface_with_limits(
                surface,
                bounds,
                &Homography([1., 0., 0.5, 0., 1., 0., 0., 0., 1.]),
                Interp::Bilinear,
                Default::default(),
                &control,
            )
            .map_err(|error| photocraft_engine::EngineError::Other(error.to_string()))?;
            cancelled.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
        let state = session.active().unwrap();
        assert_eq!(*state.doc, document);
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.past_len(), 0);
        assert!(!state.history.can_redo());
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *state.doc);
        let result =
            photocraft_engine::transform_cmds::transform_with_interrupt(&mut session, &json!({"matrix":[1,0,0,1,0.5,0],"interpolation":"bilinear"}), &control);
        assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
        assert_eq!(*session.active().unwrap().doc, document);
    }
}
