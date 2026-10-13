//! Wide raw geometry is authoritative; native persistence and undo must keep its bits.
use photocraft_doc::item_identity::{ChannelId, ItemRef, MaskId};
use photocraft_doc::xcf_state::{MaskState, XcfState};
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, LayerMask, PixelFormat, Rect, SampleType, Size};
use photocraft_engine::Session;
use photocraft_raster::Surface;
use serde_json::json;

fn source(sample: SampleType) -> Document {
    let mut document = Document::new("Own wide", Size::new(4, 4), ColorMode::Rgb, sample);
    let extent = Rect::new(-1, 1, 2, 3);
    let mut surface = Surface::new(document.pixel_format());
    let pixel = match sample {
        SampleType::U32 => [1_u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>(),
        SampleType::F64 => [-0.0_f64, 0.5 + 2.0_f64.powi(-40), 1.125, 1.0e-100].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
        _ => panic!("wide fixture only"),
    };
    surface.try_write_interleaved(extent, &pixel.repeat(6)).unwrap();
    let mut layer = Layer::new("Own pixels", LayerContent::Raster(surface));
    let mut mask = LayerMask::reveal_all();
    mask.surface = std::sync::Arc::new(
        Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(0.5 + 2.0_f64.powi(-40)).to_ne_bytes()).unwrap(),
    );
    layer.mask = Some(mask);
    let id = layer.id;
    document.layers.push(layer);
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    document.channels.push(photocraft_doc::AlphaChannel::new("Own alpha", Surface::try_with_default_bytes(gray, &0.25_f64.to_ne_bytes()).unwrap()));
    document.selection = Some(Surface::try_with_default_bytes(gray, &1.0_f64.to_ne_bytes()).unwrap());
    document.selection_channel_id = Some(ChannelId::fresh());
    let mut state = XcfState::default();
    state.reconcile_layers(&document).unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(if item.item == ItemRef::Layer(id) { extent } else { document.bounds() });
    }
    let mut metadata = state.items.iter().find(|item| item.item == ItemRef::Layer(id)).unwrap().clone();
    metadata.item = ItemRef::Mask(MaskId(71));
    metadata.mask = Some(MaskState {
        owner: ItemRef::Layer(id),
        bounds: extent,
        offset: [-1, 1],
        outside_f64_bits: (0.5 + 2.0_f64.powi(-40)).to_bits(),
        enabled: true,
        shown: false,
        editing: false,
        linked: true,
    });
    state.items.push(metadata);
    document.xcf_state = Some(state);
    document
}

#[test]
fn nearest_wide_resize_uses_declared_offset_extent_and_preserves_bits_defaults_native_undo() {
    for sample in [SampleType::U32, SampleType::F64] {
        let original = source(sample);
        let owner = original.layers[0].id;
        let mut session = Session::new();
        session.add_document(original.clone(), None).unwrap();
        session.execute("image.imageSize", json!({"width":8,"height":8,"resample":"nearest"})).unwrap();
        let edited = &session.active().unwrap().doc;
        let state = edited.xcf_state.as_ref().unwrap();
        assert_eq!(state.items.iter().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds, Some(Rect::new(-2, 2, 4, 6)));
        let metadata = state.items.iter().find(|item| item.item == ItemRef::Mask(MaskId(71))).unwrap().mask.as_ref().unwrap();
        assert_eq!(metadata.bounds, Rect::new(-2, 2, 4, 6));
        assert_eq!(metadata.offset, [-2, 2]);
        let old = original.layers[0].surface().unwrap();
        let resized = edited.layers[0].surface().unwrap();
        let mut expected = vec![0; old.format().bytes_per_pixel()];
        let mut actual = expected.clone();
        old.try_read_pixel_bytes(-1, 1, &mut expected).unwrap();
        for position in [(-2, 2), (-1, 2), (3, 5)] {
            resized.try_read_pixel_bytes(position.0, position.1, &mut actual).unwrap();
            assert_eq!(actual, expected);
        }
        assert_eq!(resized.default_bytes(), old.default_bytes());
        assert_eq!(edited.layers[0].mask.as_ref().unwrap().surface.default_bytes(), original.layers[0].mask.as_ref().unwrap().surface.default_bytes());
        assert_eq!(edited.channels[0].surface.default_bytes(), original.channels[0].surface.default_bytes());
        assert_eq!(edited.selection.as_ref().unwrap().default_bytes(), original.selection.as_ref().unwrap().default_bytes());
        let bytes = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), **edited);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, original);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.size, Size::new(8, 8));
    }
}

#[test]
fn wide_preserve_details_and_aggregate_resize_budget_reject_atomically() {
    let mut session = Session::new();
    let original = source(SampleType::F64);
    session.add_document(original.clone(), None).unwrap();
    for params in [json!({"width":8,"height":8,"resample":"preserveDetails","resolution":144}), json!({"width":300000,"height":300000,"resample":"nearest"})] {
        assert!(session.execute("image.imageSize", params).is_err());
        assert_eq!(*session.active().unwrap().doc, original);
    }
}

#[test]
fn bilinear_wide_resize_retains_f64_low_bits_hdr_and_tiny_alpha_without_display_conversion() {
    let original = source(SampleType::F64);
    let mut session = Session::new();
    session.add_document(original.clone(), None).unwrap();
    session.execute("image.imageSize", json!({"width":8,"height":8,"resample":"bilinear"})).unwrap();
    let edited = &session.active().unwrap().doc;
    let mut pixel = [0.0; 4];
    edited.layers[0].surface().unwrap().try_read_pixel_f64(0, 3, &mut pixel).unwrap();
    assert!(pixel[1] > 0.5, "f32 would collapse source low bits");
    assert!((pixel[2] - 1.125).abs() < 1.0e-14);
    assert!(pixel[3] > 0.0 && pixel[3] < 1.0e-90, "tiny source alpha remains representable");
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
}

#[test]
fn indexed_canvas_rotation_preserves_raw_identity_alpha_source_native_undo() {
    use photocraft_doc::{ColorTable, EditableIndexedLayer, EditableIndexedRaster, IndexedSource};
    use std::sync::Arc;
    let extent = Rect::new(-1, 1, 1, 3);
    let indices = Arc::new(vec![0, 1, 2, 1]);
    let alpha = Some(Arc::new(vec![255, 128, 64, 7]));
    let palette = vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]];
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.try_write_interleaved(extent, &[255, 0, 0, 255, 255, 0, 0, 128, 0, 0, 255, 64, 255, 0, 0, 7]).unwrap();
    let provenance = IndexedSource::new(indices.clone(), alpha.clone(), palette.clone(), extent, &surface).unwrap();
    let mut layer = Layer::new("Own duplicate ordinals", LayerContent::Raster(surface));
    layer.indexed_source = Some(provenance.clone());
    let id = layer.id;
    let mut doc = Document::new("Own indexed canvas", Size::new(4, 4), ColorMode::Indexed, SampleType::U8);
    doc.layers.push(layer);
    doc.color_table = Some(ColorTable { colors: palette.clone(), transparent: None });
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().raster_bounds = Some(extent);
    state.editable_indexed.push(EditableIndexedLayer { owner: id, raster: EditableIndexedRaster::try_new(extent, indices, alpha, palette).unwrap() });
    doc.xcf_state = Some(state);
    let original = doc.clone();
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session.execute("image.imageSize", json!({"width":8,"height":8,"resample":"nearest"})).unwrap();
    assert_eq!(session.active().unwrap().doc.layer(id).unwrap().indexed_source.as_ref(), Some(&provenance));
    assert_eq!(
        session.active().unwrap().doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices().as_slice(),
        &[0, 0, 1, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1]
    );
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
    session.execute("image.imageRotation.90cw", json!({})).unwrap();
    let edited = &session.active().unwrap().doc;
    let r = &edited.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
    assert_eq!(r.bounds(), Rect::new(1, -1, 3, 1));
    assert_eq!(r.indices().as_slice(), &[2, 0, 1, 1]);
    assert_eq!(r.alpha().unwrap().as_slice(), &[64, 255, 7, 128]);
    assert_eq!(edited.layer(id).unwrap().indexed_source.as_ref(), Some(&provenance));
    let bytes = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), **edited);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
}
