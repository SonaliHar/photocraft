use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, PhysicalResolution, PixelFormat, Rect, SampleType, Size};
use photocraft_engine::{Session, color_cmds};
use photocraft_raster::Surface;
use serde_json::json;

#[test]
fn wide_canvas_turns_preserve_exact_default_and_pixel_bits_through_undo() {
    for (depth, samples) in [
        (
            SampleType::F64,
            [(-0.0_f64).to_bits(), (1.0 + f64::EPSILON).to_bits(), (-3.25_f64).to_bits(), 1.0_f64.to_bits()]
                .into_iter()
                .flat_map(u64::to_ne_bytes)
                .collect::<Vec<_>>(),
        ),
        (SampleType::U32, [1_u32, u32::MAX - 1, 2, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()),
    ] {
        let mut doc = Document::new("exact turn", Size::new(1, 1), ColorMode::Rgb, depth);
        let fmt = doc.pixel_format();
        let mut layer = Layer::raster("source", fmt);
        *layer.surface_mut().unwrap() = Surface::try_from_interleaved(fmt, doc.bounds(), &samples).unwrap();
        let original_default = layer.surface().unwrap().default_bytes().to_vec();
        doc.layers.push(layer);
        for command in [
            "image.imageRotation.flipCanvasHorizontal",
            "image.imageRotation.flipCanvasVertical",
            "image.imageRotation.180",
            "image.imageRotation.90cw",
            "image.imageRotation.90ccw",
        ] {
            let mut session = Session::new();
            session.open_document(doc.clone(), None).unwrap();
            session.execute(command, json!({})).unwrap();
            let actual = session.active().unwrap().doc.layers[0].surface().unwrap();
            assert_eq!(actual.try_read_interleaved(doc.bounds()).unwrap(), samples, "{depth:?}/{command}");
            assert_eq!(actual.default_bytes(), original_default);
            assert!(session.undo());
            assert_eq!(*session.active().unwrap().doc, doc);
        }
    }
}

#[test]
fn wide_canvas_rotation_maps_off_canvas_samples_and_preserves_nonfinite_defaults() {
    let mut doc = Document::new("rotation", Size::new(2, 3), ColorMode::Rgb, SampleType::F64);
    let format = doc.pixel_format();
    let default: Vec<_> = [f64::from_bits(0x7ff8_0000_0000_0037), -0.0, 1.0 + f64::EPSILON, 0.0].into_iter().flat_map(f64::to_ne_bytes).collect();
    let mut surface = Surface::try_with_default_bytes(format, &default).unwrap();
    let pixels: Vec<_> = (0..6).flat_map(|index| [f64::from(index) + f64::EPSILON, -3.25, -0.0, 1.0].into_iter().flat_map(f64::to_ne_bytes)).collect();
    let source_bounds = Rect::new(-1, 1, 1, 4);
    surface.try_write_interleaved(source_bounds, &pixels).unwrap();
    doc.layers.push(Layer::new("source", LayerContent::Raster(surface)));
    let mut session = Session::new();
    session.open_document(doc.clone(), None).unwrap();
    session.execute("image.imageRotation.90cw", json!({})).unwrap();
    let changed = &session.active().unwrap().doc;
    assert_eq!(changed.size, Size::new(3, 2));
    let surface = changed.layers[0].surface().unwrap();
    assert_eq!(surface.default_bytes(), default);
    for y in source_bounds.y0..source_bounds.y1 {
        for x in source_bounds.x0..source_bounds.x1 {
            let mut expected = vec![0; format.bytes_per_pixel()];
            let mut actual = expected.clone();
            doc.layers[0].surface().unwrap().try_read_pixel_bytes(x, y, &mut expected).unwrap();
            surface.try_read_pixel_bytes(2 - y, x, &mut actual).unwrap();
            assert_eq!(actual, expected, "{x}/{y}");
        }
    }
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, doc);
}

#[test]
fn edit_cannot_replace_a_working_imported_filter_with_an_unqualified_operation() {
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::{FilterId, ItemRef},
        xcf_filters::{FilterNode, FilterStack, OperationIdentity},
        xcf_state::{ItemLocks, ItemState, XcfState},
    };
    let mut doc = Document::new("filtered", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    let layer = Layer::new("pixels", LayerContent::Raster(Surface::try_with_default_bytes(doc.pixel_format(), &[20, 70, 120, 255]).unwrap()));
    let owner = ItemRef::Layer(layer.id);
    let mut state = XcfState::default();
    state.provenance.format_version = 22;
    state.items.push(ItemState {
        item: owner,
        name: layer.name.clone(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        mask: None,
        composition: None,
    });
    state.filter_stacks.push(FilterStack {
        owner,
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:invert-gamma".into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-point-replace".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: true,
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: "xcf-22-stock-point-replace".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -261651887,
                stored_composite_space: -261651887,
                stored_operator: -261651887,
                resolved: None,
                pass_through: false,
            }),
        }],
    });
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap();
    let mut session = Session::new();
    session.open_document(doc.clone(), None).unwrap();
    let revision = session.active().unwrap().revision;
    let result = session.edit("Invalid filter", |candidate, _| {
        candidate.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.name = "own:unqualified".into();
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(*session.active().unwrap().doc, doc);
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(!session.undo());
}

#[test]
fn wide_invert_preserves_alpha_and_undo_and_rejects_unimplemented_adjustments() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let rgba = [1.0 + f64::EPSILON, -2.25, 0.25, 0.5 + f64::EPSILON];
    let raw: Vec<_> = rgba.into_iter().flat_map(f64::to_ne_bytes).collect();
    let surface = Surface::try_from_interleaved(format, Rect::new(0, 0, 1, 1), &raw).unwrap();
    let mut doc = Document::new("wide", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(Layer::new("pixels", LayerContent::Raster(surface)));
    let mut session = Session::new();
    session.open_document(doc.clone(), None).unwrap();
    session.execute("image.adjustments.invert", json!({})).unwrap();
    let actual = session.active().unwrap().doc.layers[0].surface().unwrap().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
    let expected: Vec<_> = [1.0 - rgba[0], 1.0 - rgba[1], 1.0 - rgba[2], rgba[3]].into_iter().flat_map(f64::to_ne_bytes).collect();
    assert_eq!(actual, expected);
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, doc);
    assert!(session.execute("image.adjustments.brightnessContrast", json!({"brightness":10})).is_err());
    assert_eq!(*session.active().unwrap().doc, doc);
}

#[test]
fn wide_mask_inversion_keeps_precise_default_and_alpha() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let raw: Vec<_> = [0.5 + f64::EPSILON, -0.0].into_iter().flat_map(f64::to_ne_bytes).collect();
    let mask = Surface::try_with_default_bytes(format, &raw).unwrap();
    let mut doc = Document::new("mask", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    let mut layer = Layer::raster("pixels", doc.pixel_format());
    layer.mask = Some(photocraft_doc::LayerMask { surface: std::sync::Arc::new(mask), enabled: true, linked: true, density: 1.0, feather: 0.0 });
    doc.layers.push(layer);
    let mut session = Session::new();
    session.open_document(doc, None).unwrap();
    let original = session.active().unwrap().doc.clone();
    session.execute("image.adjustments.invert", json!({"target":"mask"})).unwrap();
    let mask = &session.active().unwrap().doc.layers[0].mask.as_ref().unwrap().surface;
    let expected: Vec<_> = [0.5 - f64::EPSILON, -0.0].into_iter().flat_map(f64::to_ne_bytes).collect();
    assert_eq!(mask.default_bytes(), expected);
    assert_eq!(mask.try_read_interleaved(Rect::new(500, 500, 501, 501)).unwrap(), expected);
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, original);
}

#[test]
fn wide_identity_profile_conversion_keeps_defaults_and_low_bits() {
    for (sample, raw) in [
        (SampleType::U32, [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()),
        (SampleType::F64, [-0.0f64, 1.0 + f64::EPSILON, 3.25, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()),
    ] {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut surface = Surface::try_with_default_bytes(format, &raw).unwrap();
        surface.try_write_interleaved(Rect::new(2, 3, 3, 4), &raw).unwrap();
        let transform = Transform::new(Builtin::LinearSrgb.profile(), Builtin::LinearSrgb.profile(), Intent::RelativeColorimetric, false).unwrap();
        let converted = color_cmds::convert_surface(&surface, ColorMode::Rgb, format, &transform).unwrap();
        assert_eq!(converted.default_bytes(), raw);
        assert_eq!(converted.try_read_interleaved(Rect::new(2, 3, 3, 4)).unwrap(), raw);
        assert_eq!(surface.default_bytes(), raw);
    }
}

#[test]
fn late_nonfinite_profile_failure_is_atomic_outside_session() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let raw = [f64::NAN, 0.5, 0.75, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let bad = Surface::try_from_interleaved(format, Rect::new(0, 0, 1, 1), &raw).unwrap();
    let mut doc = Document::new("wide", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
    doc.layers.push(Layer::new("first", LayerContent::Raster(Surface::with_default(format, &[0.2, 0.3, 0.4, 1.0]))));
    doc.layers.push(Layer::new("bad", LayerContent::Raster(bad)));
    let before = doc.clone();
    assert!(color_cmds::convert_document(&mut doc, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).is_err());
    assert_eq!(doc.layers[0].surface().unwrap().default_bytes(), before.layers[0].surface().unwrap().default_bytes());
    assert_eq!(doc.layers[1].surface().unwrap().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), raw);
    assert_eq!(doc.icc_profile, before.icc_profile);
}

#[test]
fn physical_resolution_edits_preserve_axes_or_explicitly_set_both_with_undo() {
    let mut doc = Document::new("axes", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut resolution = PhysicalResolution::square(137.125);
    resolution.y_dpi = 92.75;
    doc.set_physical_resolution(resolution.clone()).unwrap();
    let mut session = Session::new();
    session.open_document(doc, None).unwrap();
    session.execute("image.imageSize", json!({"resample":"none"})).unwrap();
    assert_eq!(session.active().unwrap().doc.effective_resolution(), resolution);
    session.execute("image.imageSize", json!({"resample":"none","resolution":144.123456789})).unwrap();
    let changed = session.active().unwrap().doc.effective_resolution();
    assert_eq!((changed.x_dpi, changed.y_dpi), (144.123456789, 144.123456789));
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.effective_resolution(), resolution);
    let before = session.active().unwrap().doc.clone();
    assert!(session.execute("image.imageSize", json!({"resolution":"invalid"})).is_err());
    assert_eq!(session.active().unwrap().doc, before);
}

#[test]
fn canvas_turn_preserves_independent_layer_mask_extent_and_default() {
    use photocraft_doc::{
        item_identity::{ItemRef, MaskId},
        xcf_state::{ItemState, MaskState, XcfState},
    };
    let mut doc = Document::new("independent mask", Size::new(10, 10), ColorMode::Rgb, SampleType::F64);
    let mut layer = Layer::raster("pixels", doc.pixel_format());
    let mask_format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mask_bounds = Rect::new(1, 2, 2, 4);
    let mut mask = Surface::try_with_default_bytes(mask_format, &0.25f64.to_ne_bytes()).unwrap();
    let raw: Vec<_> = [0.125 + f64::EPSILON, 0.75].into_iter().flat_map(f64::to_ne_bytes).collect();
    mask.try_write_interleaved(mask_bounds, &raw).unwrap();
    layer.mask = Some(photocraft_doc::LayerMask { surface: std::sync::Arc::new(mask), enabled: true, linked: false, density: 1.0, feather: 0.0 });
    let item = |owner, name: &str, bounds, mask| ItemState {
        item: owner,
        name: name.into(),
        custom_name: false,
        visible: true,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(bounds),
        composition: None,
        mask,
    };
    let mut state = XcfState::default();
    state.items.push(item(ItemRef::Layer(layer.id), "pixels", Rect::new(4, 5, 6, 6), None));
    state.items.push(item(
        ItemRef::Mask(MaskId(777)),
        "mask",
        mask_bounds,
        Some(MaskState {
            owner: ItemRef::Layer(layer.id),
            bounds: mask_bounds,
            offset: [1, 2],
            outside_f64_bits: 0.25f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        }),
    ));
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    let before = doc.clone();
    let mut session = Session::new();
    session.open_document(doc, None).unwrap();
    session.execute("image.imageRotation.90cw", json!({})).unwrap();
    let turned = &session.active().unwrap().doc;
    let expected: Vec<_> = [0.75, 0.125 + f64::EPSILON].into_iter().flat_map(f64::to_ne_bytes).collect();
    let mask = &turned.layers[0].mask.as_ref().unwrap().surface;
    assert_eq!(mask.try_read_interleaved(Rect::new(6, 1, 8, 2)).unwrap(), expected);
    assert_eq!(mask.default_bytes(), 0.25f64.to_ne_bytes());
    let metadata = turned.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Mask(MaskId(777))).unwrap();
    assert_eq!(metadata.mask.as_ref().unwrap().bounds, Rect::new(6, 1, 8, 2));
    assert_eq!(metadata.mask.as_ref().unwrap().offset, [6, 1]);
    let native = photocraft_format::save_to_bytes(turned, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.xcf_state, turned.xcf_state);
    assert_eq!(reopened.layers, turned.layers);
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, before);
}

#[test]
fn checked_document_profile_rejects_malformed_and_mismatched_assignments() {
    let mut doc = Document::new("profile", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    assert!(color_cmds::try_document_profile(&doc).is_ok());
    doc.icc_profile = Some(std::sync::Arc::new(b"malformed assigned profile".to_vec()));
    assert!(color_cmds::try_document_profile(&doc).is_err());
    doc.icc_profile = Some(Builtin::SGray.profile().to_bytes());
    assert!(color_cmds::try_document_profile(&doc).is_err());
}
