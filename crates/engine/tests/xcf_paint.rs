//! Actual command strokes keep authoritative wide samples and finite imported extent.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, PixelFormat, Rect, SampleEncoding, SampleType, Size};
use photocraft_doc::{item_identity::ItemRef, xcf_state::XcfState};
use photocraft_engine::{Session, brush_cmds::LiveStroke};
use photocraft_raster::Surface;
use serde_json::json;

fn source(sample: SampleType) -> Document {
    let mut doc = Document::new("Own wide brush", Size::new(8, 4), ColorMode::Rgb, sample);
    doc.sample_encoding = Some(SampleEncoding::Linear);
    let values = match sample {
        SampleType::U32 => [0x8000_0001_u32, 0x8000_0002, 0x8000_0003, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>(),
        SampleType::F64 => [0.5 + 2.0_f64.powi(-40), -0.0, 1.125, 0.75 + 2.0_f64.powi(-40)].into_iter().flat_map(f64::to_ne_bytes).collect(),
        _ => panic!("wide fixture"),
    };
    let surface = Surface::try_with_default_bytes(doc.pixel_format(), &values).unwrap();
    doc.layers.push(Layer::new("Own color", LayerContent::Raster(surface)));
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(Rect::new(0, 0, 4, 4));
    }
    doc.xcf_state = Some(state);
    doc
}
fn pixels(doc: &Document, area: Rect) -> Vec<u8> {
    doc.layers[0].surface().unwrap().try_read_interleaved(area).unwrap()
}
fn stroke(erase: bool) -> serde_json::Value {
    json!({"points":[[0.5,1.5],[6.5,1.5]],"size":1,"hardness":1,"opacity":0.5,"flow":1,"seed":42,"erase":erase,"color":"#000000","brush":{"aliased":true}})
}
#[test]
fn alpha_eraser_preserves_actual_adjacent_u32_and_f64_color_bits_undo_native_and_extent() {
    for sample in [SampleType::U32, SampleType::F64] {
        let original = source(sample);
        let before = pixels(&original, Rect::new(0, 0, 8, 4));
        let surface = original.layers[0].surface().unwrap();
        let mut legacy = surface.clone();
        legacy.write_region(Rect::new(0, 0, 1, 1), &surface.read_region(Rect::new(0, 0, 1, 1)));
        assert_ne!(pixels_from(&legacy, Rect::new(0, 0, 1, 1)), pixels_from(surface, Rect::new(0, 0, 1, 1)));
        let mut session = Session::new();
        session.add_document(original.clone(), None).unwrap();
        session.execute("paint.stroke", stroke(true)).unwrap();
        let edited = &session.active().unwrap().doc;
        let after = pixels(edited, Rect::new(0, 0, 8, 4));
        let bpp = surface.format().bytes_per_pixel();
        let color = bpp - sample.bytes();
        for (old, new) in before.chunks_exact(bpp).zip(after.chunks_exact(bpp)) {
            assert_eq!(&old[..color], &new[..color]);
        }
        assert_ne!(before, after);
        assert_eq!(pixels(edited, Rect::new(4, 0, 8, 4)), pixels(&original, Rect::new(4, 0, 8, 4)));
        assert_eq!(edited.layers[0].surface().unwrap().default_bytes(), surface.default_bytes());
        let owner = original.layers[0].id;
        assert_eq!(
            edited.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds,
            Some(Rect::new(0, 0, 4, 4))
        );
        let saved = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), **edited);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, original);
    }
}
fn pixels_from(surface: &Surface, area: Rect) -> Vec<u8> {
    surface.try_read_interleaved(area).unwrap()
}
#[test]
fn normal_stroke_selection_uses_f64_and_untouched_tail_remains_exact() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut original = source(sample);
        let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut selection = Surface::new(gray);
        selection.try_write_pixel_f64(1, 1, &[0.25 + 2.0_f64.powi(-40)]).unwrap();
        original.selection = Some(selection);
        let before = original.layers[0].surface().unwrap().try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap();
        let mut session = Session::new();
        session.add_document(original.clone(), None).unwrap();
        session.execute("paint.stroke", stroke(false)).unwrap();
        let edited = &session.active().unwrap().doc;
        assert_eq!(pixels(edited, Rect::new(2, 0, 8, 4)), pixels(&original, Rect::new(2, 0, 8, 4)));
        let after = edited.layers[0].surface().unwrap().try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap();
        let w = 0.5 * (0.25 + 2.0_f64.powi(-40));
        let oa = w + before[3] * (1.0 - w);
        let expected = before[0] * before[3] * (1.0 - w) / oa;
        let tolerance = if sample == SampleType::U32 { 1.0 / f64::from(u32::MAX) } else { 1e-14 };
        assert!((after[0] - expected).abs() <= tolerance, "{sample:?}: {} vs {expected}", after[0]);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, original);
    }
}
#[test]
fn checked_live_preview_recomposes_exactly_and_matches_commit() {
    for sample in [SampleType::U32, SampleType::F64] {
        for erase in [false, true] {
            let original = source(sample);
            let mut session = Session::new();
            session.add_document(original.clone(), None).unwrap();
            let mut params = stroke(erase);
            params["smoothing"] = json!(0.7);
            params["points"] = json!([[0.5, 1.5]]);
            let mut live = LiveStroke::begin(&session, &params).unwrap();
            let more = photocraft_paint::StrokePoint::new(3.5, 1.5, 1.0);
            live.push(&[more]).unwrap();
            // Discarding a preview is cancellation: it has never mutated Session/history.
            assert_eq!(*session.active().unwrap().doc, original);
            params["points"] = json!([[0.5, 1.5], [3.5, 1.5]]);
            session.execute("paint.stroke", params).unwrap();
            assert_eq!(pixels(&live.doc, Rect::new(-2, -2, 10, 6)), pixels(&session.active().unwrap().doc, Rect::new(-2, -2, 10, 6)));
            assert_eq!(pixels(&live.doc, Rect::new(4, 0, 8, 4)), pixels(&original, Rect::new(4, 0, 8, 4)));
        }
    }
}
#[test]
fn unsupported_wide_modes_reject_without_history_or_document_mutation() {
    for sample in [SampleType::U32, SampleType::F64] {
        for extra in [json!({"mode":"multiply"}), json!({"brush":{"colorDynamics":{"enabled":true}}})] {
            let original = source(sample);
            let mut session = Session::new();
            session.add_document(original.clone(), None).unwrap();
            let mut params = stroke(false);
            for (key, value) in extra.as_object().unwrap() {
                params[key] = value.clone();
            }
            assert!(session.execute("paint.stroke", params.clone()).is_err());
            assert!(LiveStroke::begin(&session, &params).is_err());
            assert_eq!(*session.active().unwrap().doc, original);
        }
        let mut original = source(sample);
        original.layers[0].locks.transparency = true;
        let mut session = Session::new();
        session.add_document(original.clone(), None).unwrap();
        assert!(session.execute("paint.stroke", stroke(true)).is_err());
        assert_eq!(*session.active().unwrap().doc, original);
    }
}

#[test]
fn no_alpha_eraser_and_unqualified_adjacent_tools_reject_atomically() {
    for sample in [SampleType::U32, SampleType::F64] {
        let original = source(sample);
        for command in ["paint.mixerBrush", "paint.colorReplacement"] {
            let mut session = Session::new();
            session.add_document(original.clone(), None).unwrap();
            assert!(session.execute(command, stroke(false)).is_err());
            assert_eq!(*session.active().unwrap().doc, original);
        }
        let mut no_alpha = original.clone();
        let format = PixelFormat::new(ColorMode::Rgb, sample, false);
        no_alpha.layers[0].content = LayerContent::Raster(Surface::new(format));
        let mut session = Session::new();
        session.add_document(no_alpha.clone(), None).unwrap();
        assert!(session.execute("paint.stroke", stroke(true)).is_err());
        assert_eq!(*session.active().unwrap().doc, no_alpha);
    }
}

#[test]
fn absurd_wide_coverage_is_rejected_before_generation_and_live_pixels_stay_atomic() {
    let original = source(SampleType::F64);
    let mut session = Session::new();
    session.add_document(original.clone(), None).unwrap();
    let mut params = stroke(false);
    params["size"] = json!(100000.0);
    assert!(session.execute("paint.stroke", params.clone()).is_err());
    assert!(LiveStroke::begin(&session, &params).is_err());
    assert_eq!(*session.active().unwrap().doc, original);
    let mut live = LiveStroke::begin(&session, &stroke(false)).unwrap();
    let before = live.doc.clone();
    assert!(live.push(&[photocraft_paint::StrokePoint::new(f64::NAN, 0.0, 1.0)]).is_err());
    assert_eq!(*live.doc, *before);
    assert_eq!(*session.active().unwrap().doc, original);
}

#[test]
fn wide_foreground_color_is_converted_to_effective_linear_encoding() {
    let original = source(SampleType::F64);
    let mut session = Session::new();
    session.add_document(original, None).unwrap();
    let mut params = stroke(false);
    params["points"] = json!([[1.5, 1.5]]);
    params["opacity"] = json!(1.0);
    params["color"] = json!("#808080");
    session.execute("paint.stroke", params).unwrap();
    let native = session.active().unwrap().doc.layers[0].surface().unwrap().try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap();
    let encoded = 128.0_f64 / 255.0;
    let expected = ((encoded + 0.055) / 1.055).powf(2.4);
    for channel in &native[..3] {
        assert!((*channel - expected).abs() < 1e-6, "{channel} vs {expected}");
    }
    assert_eq!(native[3], 1.0);
}

#[test]
fn numeric_wide_channel_brush_uses_green_luminosity() {
    let mut original = source(SampleType::F64);
    original.channels.push(photocraft_doc::AlphaChannel::new("Own numeric", Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false))));
    let mut session = Session::new();
    session.add_document(original.clone(), None).unwrap();
    let mut params = stroke(false);
    params["points"] = json!([[1.5, 1.5]]);
    params["opacity"] = json!(1.0);
    let green = 0.5 + 2.0_f64.powi(-35);
    params["color"] = json!([0.0, green, 0.0, 1.0]);
    params["target"] = json!({"channel":0});
    session.execute("paint.stroke", params).unwrap();
    assert_eq!(session.active().unwrap().doc.channels[0].surface.try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap(), vec![0.587 * green]);
    assert_eq!(session.active().unwrap().doc.layers, original.layers);
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, original);
}
