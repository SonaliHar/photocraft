//! Checked command fills and gradients preserve authoritative wide samples.
use photocraft_doc::xcf_state::XcfState;
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, PixelFormat, Rect, SampleEncoding, SampleType, Size};
use photocraft_engine::Session;
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

#[test]
fn forged_retained_tile_keys_fail_commands_before_damage_or_history_publication() {
    for sample in [SampleType::U32, SampleType::F64] {
        for (command, params) in [
            ("edit.fill", json!({"contents":"black"})),
            ("paint.gradient", json!({"from":[0,0],"to":[4,0],"colors":["#000000","#ffffff"],"dither":false})),
            ("paint.stroke", json!({"points":[[0.5,0.5]],"size":1,"hardness":1,"color":"#000000","brush":{"aliased":true}})),
            ("paint.stroke", json!({"points":[[0.5,0.5]],"size":1,"hardness":1,"erase":true,"brush":{"aliased":true}})),
        ] {
            let mut session = Session::new();
            session.add_document(source(sample), None).unwrap();
            let mut forged = Surface::new(session.active().unwrap().doc.pixel_format());
            forged.try_write_pixel_f64(0, 0, &[0.0, 0.0, 0.0, 1.0]).unwrap();
            let tile = forged.tiles().next().unwrap().1.clone();
            let mut target = Surface::new(forged.format());
            target.put_tiles([(photocraft_geom::TileCoord::new(i32::MAX, i32::MIN), tile)]);
            session.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].content = LayerContent::Raster(target);
            let before = session.active().unwrap().doc.clone();
            let history = session.active().unwrap().history.past_len();
            assert!(session.execute(command, params).is_err(), "{command}");
            assert_eq!(session.active().unwrap().doc, before);
            assert_eq!(session.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn current_and_named_constant_opacity_presets_match_checked_explicit_gradient() {
    use photocraft_engine::presets::gradients::{GradientPreset, StopColor};
    for sample in [SampleType::U32, SampleType::F64] {
        let original = source(sample);
        let mut explicit = Session::new();
        explicit.add_document(original.clone(), None).unwrap();
        explicit.execute("paint.gradient", json!({"from":[0,0],"to":[4,0],"colors":[[0,0,0,0.5],[1,1,1,0.5]],"dither":false})).unwrap();
        for named in [false, true] {
            let mut s = Session::new();
            s.add_document(original.clone(), None).unwrap();
            let preset = GradientPreset {
                name: "Own constant".into(),
                stops: vec![(1.0, StopColor::Rgb([1.0; 3])), (0.0, StopColor::Rgb([0.0; 3]))],
                opacity: vec![(0.0, 0.5), (1.0, 0.5)],
            };
            s.presets.gradient = preset.clone();
            s.presets.gradients[0].items.push(preset);
            let mut params = json!({"from":[0,0],"to":[4,0],"dither":false});
            if named {
                params["gradient"] = json!("Own constant");
            }
            s.execute("paint.gradient", params.clone()).unwrap();
            assert_eq!(*s.active().unwrap().doc, *explicit.active().unwrap().doc);
            let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *s.active().unwrap().doc);
            assert!(s.undo());
            assert_eq!(*s.active().unwrap().doc, original);
            if named {
                params["gradient"] = json!("Absent own preset");
            } else {
                s.presets.gradient.opacity[1].1 = 0.75;
            }
            assert!(s.execute("paint.gradient", params).is_err());
            assert_eq!(*s.active().unwrap().doc, original);
        }
    }
}

#[test]
fn solid_selection_exact_tail_default_native_undo() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut original = source(sample);
        let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        mask.try_write_pixel_f64(1, 1, &[0.25 + 2.0_f64.powi(-40)]).unwrap();
        original.selection = Some(mask);
        let old = original.layers[0].surface().unwrap().try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap();
        let mut s = Session::new();
        s.add_document(original.clone(), None).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":[0,0,0,1],"opacity":50})).unwrap();
        let edited = &s.active().unwrap().doc;
        let after = edited.layers[0].surface().unwrap().try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap();
        let weight = 0.5 * (0.25 + 2.0_f64.powi(-40));
        let alpha = weight + old[3] * (1.0 - weight);
        let expected = old[0] * old[3] * (1.0 - weight) / alpha;
        let epsilon = if sample == SampleType::U32 { 1.0 / f64::from(u32::MAX) } else { 1e-14 };
        assert!((after[0] - expected).abs() <= epsilon);
        assert_eq!(pixels(edited, Rect::new(2, 0, 8, 4)), pixels(&original, Rect::new(2, 0, 8, 4)));
        assert_eq!(edited.layers[0].surface().unwrap().default_bytes(), original.layers[0].surface().unwrap().default_bytes());
        let bytes = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), **edited);
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, original);
    }
}
#[test]
fn white_default_selection_and_canonical_extent_are_respected() {
    let mut original = source(SampleType::F64);
    original.selection = Some(Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &1.0_f64.to_ne_bytes()).unwrap());
    let mut s = Session::new();
    s.add_document(original.clone(), None).unwrap();
    s.execute("edit.fill", json!({"contents":"white"})).unwrap();
    let edited = &s.active().unwrap().doc;
    let value = edited.layers[0].surface().unwrap().try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap();
    assert!(value[..3].iter().all(|v| (v - 1.0).abs() < 1e-14));
    assert_eq!(value[3], 1.0);
    assert_eq!(pixels(edited, Rect::new(4, 0, 8, 4)), pixels(&original, Rect::new(4, 0, 8, 4)));
    assert_eq!(edited.xcf_state, original.xcf_state);
}
#[test]
fn gradient_preserve_alpha_and_atomic_unsupported_routes() {
    for sample in [SampleType::U32, SampleType::F64] {
        let original = source(sample);
        let mut s = Session::new();
        s.add_document(original.clone(), None).unwrap();
        s.edit("lock", |doc, _| {
            doc.layers[0].locks.transparency = true;
            Ok(())
        })
        .unwrap();
        let before = (*s.active().unwrap().doc).clone();
        s.execute("paint.gradient", json!({"from":[0,0],"to":[4,0],"colors":["#000000","#ffffff"],"dither":false})).unwrap();
        let edited = &s.active().unwrap().doc;
        let width = sample.bytes();
        let bpp = width * 4;
        for (a, b) in pixels(&before, Rect::new(0, 0, 8, 4)).chunks_exact(bpp).zip(pixels(edited, Rect::new(0, 0, 8, 4)).chunks_exact(bpp)) {
            assert_eq!(&a[3 * width..], &b[3 * width..]);
        }
        assert_ne!(pixels(edited, Rect::new(0, 0, 4, 4)), pixels(&before, Rect::new(0, 0, 4, 4)));
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, before);
        for (cmd, p) in [
            ("paint.bucket", json!({"sampleAllLayers":true,"contents":"pattern"})),
            ("paint.gradient", json!({"from":[0,0],"to":[4,0],"colors":["#000000","#ffffff"]})),
            ("paint.gradient", json!({"from":[0,0],"to":[4,0],"colors":["#000000","#ffffff"],"dither":false,"mode":"multiply"})),
            ("edit.fill", json!({"contents":"pattern"})),
            ("edit.fill", json!({"contents":"history"})),
            ("edit.fill", json!({"contents":"contentAware"})),
            ("edit.fill", json!({"contents":"color","color":[0,0,"bad"]})),
        ] {
            assert!(s.execute(cmd, p).is_err(), "{cmd}");
            assert_eq!(*s.active().unwrap().doc, before);
        }
    }
}

#[test]
fn numeric_channel_fill_uses_luminosity_without_color_profile_and_preserves_layer() {
    let mut original = source(SampleType::F64);
    original.channels.push(photocraft_doc::AlphaChannel::new("Own numeric", Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false))));
    let mut s = Session::new();
    s.add_document(original.clone(), None).unwrap();
    s.execute("edit.fill", json!({"target":{"channel":0},"contents":"color","color":[0,1,0,1]})).unwrap();
    let after = &s.active().unwrap().doc;
    assert_eq!(after.channels[0].surface.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap(), vec![0.587]);
    assert_eq!(after.layers, original.layers);
    assert!(s.undo());
    assert_eq!(*s.active().unwrap().doc, original);
    s.execute("paint.bucket", json!({"target":{"channel":0},"color":[0,1,0,1],"tolerance":0,"antiAlias":false})).unwrap();
    assert_eq!(s.active().unwrap().doc.channels[0].surface.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap(), vec![0.587]);
    assert_eq!(s.active().unwrap().doc.layers, original.layers);
    assert!(s.undo());
    assert_eq!(*s.active().unwrap().doc, original);
}
#[test]
fn gradient_coordinates_and_stop_positions_stay_f64() {
    let mut original = source(SampleType::F64);
    original.layers[0].content = LayerContent::Raster(Surface::new(original.pixel_format()));
    let mut s = Session::new();
    s.add_document(original.clone(), None).unwrap();
    let end = 4.0 + 2.0_f64.powi(-35);
    s.execute("paint.gradient", json!({"from":[0,0],"to":[end,0],"stops":[[0,"#000000"],[1,"#ffffff"]],"dither":false})).unwrap();
    let value = s.active().unwrap().doc.layers[0].surface().unwrap().try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap()[0];
    assert!((value - 0.5 / end).abs() < 1e-14, "{value}");
    assert_ne!(value, 0.125);
    assert_eq!(pixels(&s.active().unwrap().doc, Rect::new(4, 0, 8, 4)), pixels(&original, Rect::new(4, 0, 8, 4)));
}

#[test]
fn invalid_profile_and_pixel_lock_fail_without_history_or_sample_changes() {
    for invalid_profile in [false, true] {
        let mut original = source(SampleType::F64);
        if invalid_profile {
            original.icc_profile = Some(std::sync::Arc::new(vec![0; 128]));
        } else {
            original.layers[0].locks.pixels = true;
        }
        let mut session = Session::new();
        session.add_document(original.clone(), None).unwrap();
        for (cmd, params) in
            [("edit.fill", json!({"contents":"white"})), ("paint.gradient", json!({"from":[0,0],"to":[4,0],"colors":["#000000","#ffffff"],"dither":false}))]
        {
            let history = session.active().unwrap().history.past_len();
            assert!(session.execute(cmd, params).is_err());
            assert_eq!(*session.active().unwrap().doc, original);
            assert_eq!(session.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn exact_bucket_uses_native_boundary_and_restores_bits_on_undo_and_native_reopen() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut original = source(sample);
        let fmt = original.pixel_format();
        let mut bytes = Vec::new();
        for _y in 0..4 {
            for x in 0..8 {
                if sample == SampleType::U32 {
                    for value in [0x80000001_u32 + u32::from(x == 1), 0x80000003, 0x80000004, u32::MAX] {
                        bytes.extend_from_slice(&value.to_ne_bytes());
                    }
                } else {
                    for value in [0.5 + if x == 1 { 2.0_f64.powi(-39) } else { 2.0_f64.powi(-40) }, 0.75, 1.125, 1.0] {
                        bytes.extend_from_slice(&value.to_ne_bytes());
                    }
                }
            }
        }
        original.layers[0].content = LayerContent::Raster(Surface::try_from_interleaved(fmt, Rect::new(0, 0, 8, 4), &bytes).unwrap());
        for contiguous in [true, false] {
            let mut session = Session::new();
            session.add_document(original.clone(), None).unwrap();
            assert_eq!(
                session.execute("paint.bucket", json!({"x":0,"y":0,"color":"#000000","tolerance":0,"antiAlias":false,"contiguous":contiguous})).unwrap()["filled"],
                true
            );
            let after = &session.active().unwrap().doc;
            let v = after.layers[0].surface().unwrap().try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap();
            assert_eq!(v, vec![0.0, 0.0, 0.0, 1.0]);
            assert_eq!(pixels(after, Rect::new(1, 0, 2, 4)), pixels(&original, Rect::new(1, 0, 2, 4)));
            if contiguous {
                assert_eq!(pixels(after, Rect::new(2, 0, 4, 4)), pixels(&original, Rect::new(2, 0, 4, 4)));
            } else {
                assert_eq!(after.layers[0].surface().unwrap().try_read_region_f64(Rect::new(2, 0, 3, 1)).unwrap(), vec![0.0, 0.0, 0.0, 1.0]);
            }
            assert_eq!(pixels(after, Rect::new(4, 0, 8, 4)), pixels(&original, Rect::new(4, 0, 8, 4)));
            assert_eq!(after.xcf_state, original.xcf_state);
            assert_eq!(after.layers[0].surface().unwrap().default_bytes(), original.layers[0].surface().unwrap().default_bytes());
            let saved = photocraft_format::save_to_bytes(after, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), **after);
            assert!(session.undo());
            assert_eq!(*session.active().unwrap().doc, original);
        }
    }
}
