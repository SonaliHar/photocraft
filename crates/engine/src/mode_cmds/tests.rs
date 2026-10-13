use super::*;
use photocraft_geom::Rect;

fn session(w: u32, h: u32, depth: u64, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth, "mode": mode})).unwrap();
    paint(&mut s, |x, y| [x as f32 / w as f32, y as f32 / h as f32, 0.5, 1.0]);
    s
}

fn paint(s: &mut Session, f: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn colours(d: &Document) -> std::collections::HashSet<[u8; 3]> {
    let surf = d.layers[0].surface().unwrap();
    let mut set = std::collections::HashSet::new();
    for y in 0..d.size.height as i32 {
        for x in 0..d.size.width as i32 {
            let c = surf.rgba(x, y);
            if c[3] > 0.0 {
                set.insert([c[0], c[1], c[2]].map(|v| (v * 255.0).round() as u8));
            }
        }
    }
    set
}

#[test]
fn rotate_arbitrary_grows_canvas_and_keeps_background() {
    for depth in [8, 16, 32] {
        let mut s = session(40, 20, depth, "rgb");
        s.execute("layer.new.layer", json!({})).unwrap();
        paint(&mut s, |x, y| if (15..25).contains(&x) && (5..15).contains(&y) { [1.0, 0.0, 0.0, 1.0] } else { [0.0; 4] });
        s.execute("tools.setColors", json!({"background": "#00ff00"})).unwrap();
        let r = s.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "cw"})).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(20), Some(40)));
        let d = doc(&s);
        assert_eq!(d.layers[0].name, "Background");
        // The red square stays centred.
        assert!(d.layers[1].surface().unwrap().rgba(10, 20)[0] > 0.9, "{depth}");
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("image.rotation.arbitrary", json!({"angle": 30, "direction": "ccw"})).unwrap();
        let d = doc(&s);
        assert_eq!(d.size, rotated_size(Size::new(40, 20), 30.0));
        assert!(d.size.width > 40 && d.size.height > 20);
        // Exposed corners take the background colour on the Background layer.
        let corner = d.layers[0].surface().unwrap().rgba(0, 0);
        assert!(corner[1] > 0.9 && corner[0] < 0.1, "{corner:?}");
    }
    let mut s = session(10, 10, 8, "rgb");
    assert!(s.execute("image.rotation.arbitrary", json!({"angle": 10, "direction": "up"})).is_err());
    let n = s.active().unwrap().history.past_len();
    s.execute("image.rotation.arbitrary", json!({"angle": 360})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), n, "a full turn changes nothing");
}

#[test]
fn rotate_arbitrary_other_modes() {
    for mode in ["gray", "cmyk", "lab"] {
        let mut s = session(16, 12, 16, mode);
        s.execute("image.rotation.arbitrary", json!({"angle": 45})).unwrap();
        assert_eq!(doc(&s).size, rotated_size(Size::new(16, 12), 45.0), "{mode}");
    }
}

#[test]
fn indexed_color_palettes_and_dithers() {
    for palette in ["selective", "perceptual", "adaptive", "web", "uniform", "systemMac", "systemWindows"] {
        for dither in ["none", "diffusion", "pattern", "noise"] {
            let mut s = session(24, 16, 16, "rgb");
            let r = s.execute("image.mode.indexedColor", json!({"palette": palette, "colors": 16, "dither": dither, "forced": "none"})).unwrap();
            let d = doc(&s);
            assert_eq!((d.mode, d.depth), (ColorMode::Indexed, SampleType::U8), "{palette} {dither}");
            let table = d.color_table.as_ref().unwrap();
            assert_eq!(table.colors.len() as u64, r["colors"].as_u64().unwrap());
            let used = colours(d);
            assert!(used.iter().all(|c| table.colors.contains(c)), "{palette} {dither}: pixels off the table");
            if !matches!(palette, "web" | "systemMac" | "systemWindows") {
                assert!(table.colors.len() <= 16, "{palette}: {}", table.colors.len());
            }
        }
    }
}

#[test]
fn indexed_exact_transparency_and_sources() {
    // Two colours + transparency: exact palette with a transparent entry.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    paint(&mut s, |x, _| {
        if x < 3 {
            [1.0, 0.0, 0.0, 1.0]
        } else if x < 6 {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [0.0; 4]
        }
    });
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none"})).unwrap();
    let d = doc(&s);
    let t = d.color_table.as_ref().unwrap();
    assert_eq!(t.colors.len(), 3);
    assert_eq!(t.transparent, Some(2));
    assert_eq!(d.layers[0].name, "Index");
    assert_eq!(d.layers[0].surface().unwrap().rgba(7, 0)[3], 0.0);
    // Exact fails on a gradient.
    let mut s = session(64, 64, 8, "rgb");
    assert!(s.execute("image.mode.indexedColor", json!({"palette": "exact"})).is_err());
    // Gray, CMYK and Lab sources flatten through the composite.
    for mode in ["gray", "cmyk", "lab"] {
        let mut s = session(12, 12, 16, mode);
        s.execute("image.mode.indexedColor", json!({"colors": 8})).unwrap();
        assert_eq!(doc(&s).mode, ColorMode::Indexed, "{mode}");
    }
    // Back to RGB drops the table.
    s.execute("image.mode.indexedColor", json!({"colors": 8})).unwrap();
    s.execute("image.mode.rgb", json!({})).unwrap();
    assert!(doc(&s).color_table.is_none());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(doc(&s).color_table.is_some());
}

#[test]
fn color_table_presets_and_entries() {
    let mut s = session(16, 16, 8, "rgb");
    assert!(!s.is_enabled("image.mode.colorTable"));
    s.execute("image.mode.indexedColor", json!({"palette": "uniform", "colors": 8, "dither": "none", "forced": "none"})).unwrap();
    let before = doc(&s).color_table.clone().unwrap();
    // No changes: a report, no history step.
    let n = s.active().unwrap().history.past_len();
    let r = s.execute("image.mode.colorTable", json!({})).unwrap();
    assert_eq!(r["colors"].as_array().unwrap().len(), before.colors.len());
    assert_eq!(s.active().unwrap().history.past_len(), n);
    // Replace entry 0: pixels of index 0 change colour.
    let px0 = doc(&s).layers[0].surface().unwrap().rgba(0, 0);
    let i0 = before.nearest([px0[0], px0[1], px0[2]]);
    s.execute("image.mode.colorTable", json!({"entries": {i0.to_string(): "#ff00ff"}})).unwrap();
    assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(0, 0), [1.0, 0.0, 1.0, 1.0]);
    s.execute("image.mode.colorTable", json!({"table": "blackBody"})).unwrap();
    assert_eq!(doc(&s).color_table.as_ref().unwrap().colors.len(), 256);
    s.execute("image.mode.colorTable", json!({"transparent": 0})).unwrap();
    assert!(s.execute("image.mode.colorTable", json!({"table": "nope"})).is_err());
    assert!(s.execute("image.mode.colorTable", json!({"entries": {"999": "#000000"}})).is_err());
}

#[test]
fn bitmap_methods_need_grayscale() {
    let mut s = session(32, 32, 16, "rgb");
    assert!(!s.is_enabled("image.mode.bitmap"));
    s.execute("image.mode.grayscale", json!({})).unwrap();
    for method in ["threshold", "pattern", "diffusion", "halftone"] {
        s.execute("image.mode.bitmap", json!({"method": method, "frequency": 12, "shape": "diamond"})).unwrap();
        let d = doc(&s);
        assert_eq!((d.mode, d.depth), (ColorMode::Bitmap, SampleType::U8), "{method}");
        let surf = d.layers[0].surface().unwrap();
        for y in 0..32 {
            for x in 0..32 {
                let v = surf.pixel(x, y)[0];
                assert!(v == 0.0 || v == 1.0, "{method}: {v}");
            }
        }
        s.execute("edit.undo", json!({})).unwrap();
    }
    assert!(s.execute("image.mode.bitmap", json!({"method": "nope"})).is_err());
}

#[test]
fn duotone_inks_and_display() {
    let mut s = session(16, 8, 8, "gray");
    paint(&mut s, |x, _| [x as f32 / 15.0; 4].map(|v| v.min(1.0)));
    assert!(s.execute("image.mode.duotone", json!({"type": "quintone"})).is_err());
    let r = s
        .execute(
            "image.mode.duotone",
            json!({"type": "duotone", "inks": [{"name": "Black"}, {"name": "Orange", "color": "#ff8000", "curve": [[0, 0], [100, 80]]}]}),
        )
        .unwrap();
    assert_eq!(r["inks"], json!(["Black", "Orange"]));
    let d = doc(&s);
    assert_eq!(d.mode, ColorMode::Duotone);
    let inks = &d.duotone.as_ref().unwrap().inks;
    assert_eq!(inks[1].curve.last().unwrap().output, 0.8);
    // Display: warm midtones (red > blue) where the gray image is neutral.
    let shown = display_document(d).unwrap();
    let px = photocraft_compose::render(&shown, Rect::from_xywh(8, 4, 1, 1)).unwrap().px[0];
    assert!(px[0] > px[2] + 0.05, "{px:?}");
    assert!(display_document(&Document::new("x", Size::new(1, 1), ColorMode::Grayscale, SampleType::U8)).is_none());
    // Tritone defaults and leaving the mode.
    s.execute("image.mode.duotone", json!({"type": "tritone"})).unwrap();
    assert_eq!(doc(&s).duotone.as_ref().unwrap().inks.len(), 3);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    assert!(doc(&s).duotone.is_none());
    assert_eq!(doc(&s).mode, ColorMode::Grayscale);
    assert!(!session(4, 4, 8, "rgb").is_enabled("image.mode.duotone"));
}

#[test]
fn modes_round_trip_through_pcraft() {
    let mut s = session(12, 12, 8, "rgb");
    s.execute("image.mode.indexedColor", json!({"colors": 6})).unwrap();
    let bytes = photocraft_format::save_to_bytes(doc(&s), &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back.color_table, doc(&s).color_table);
    assert_eq!(back.mode, ColorMode::Indexed);
}
#[test]
fn indexed_channel_quantization_reads_f64_once_and_undo_restores_bits() {
    let mut s = session(2, 2, 8, "rgb");
    let value: f64 = (128.5 - 1e-8) / 255.;
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    s.edit("wide channel setup", |d, _| {
        d.channels.push(photocraft_doc::AlphaChannel::new(
            "Own exact channel",
            Surface::try_from_interleaved(format, Rect::new(0, 0, 1, 1), &value.to_ne_bytes()).unwrap(),
        ));
        Ok(())
    })
    .unwrap();
    let before = doc(&s).clone();
    s.execute("image.mode.indexedColor", json!({"colors":4,"forced":"none","dither":"none"})).unwrap();
    assert_eq!(doc(&s).channels[0].surface.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), vec![128]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s), &before);
}
#[test]
fn unsupported_wide_mode_and_auxiliary_model_changes_are_atomic() {
    let mut s = session(2, 2, 8, "rgb");
    s.edit("invalid channel model", |d, _| {
        d.channels.push(photocraft_doc::AlphaChannel::new("RGB auxiliary", Surface::new(PixelFormat::RGBA64F)));
        Ok(())
    })
    .unwrap();
    let before = doc(&s).clone();
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("image.mode.indexedColor", json!({"colors":4})).is_err());
    assert_eq!(doc(&s), &before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    let mut s = session(2, 2, 8, "rgb");
    s.edit("wide source", |d, _| {
        d.depth = SampleType::F64;
        d.layers[0].content = LayerContent::Raster(Surface::new(PixelFormat::RGBA64F));
        Ok(())
    })
    .unwrap();
    let before = doc(&s).clone();
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("image.mode.indexedColor", json!({"colors":4})).is_err());
    assert_eq!(doc(&s), &before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

fn bitmap_physical_fixture(legacy: f32, physical: Option<photocraft_doc::PhysicalResolution>) -> Session {
    let mut d = Document::with_background(
        "Physical halftone",
        Size::new(64, 64),
        ColorMode::Grayscale,
        SampleType::U8,
        photocraft_color::Color::rgba(0.5, 0.5, 0.5, 1.0),
    );
    d.resolution_dpi = legacy;
    d.physical_resolution = physical;
    let bytes = photocraft_format::save_to_bytes(&d, &Default::default()).unwrap();
    let mut s = Session::new();
    s.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None).unwrap();
    s
}
fn bitmap_physical_samples(s: &Session) -> Vec<f32> {
    doc(s).layers[0].surface().unwrap().read_region(doc(s).bounds())
}
fn bitmap_physical_run(s: &mut Session) {
    s.execute("image.mode.bitmap", json!({"method":"halftone","frequency":36,"angle":0,"shape":"round"})).unwrap();
}
#[test]
fn bitmap_physical_canonical_square_native_and_history() {
    use photocraft_doc::{CustomUnit, PhysicalResolution, PhysicalUnit};
    for dpi in [144.0, 144.25] {
        let mut legacy = bitmap_physical_fixture(dpi as f32, None);
        bitmap_physical_run(&mut legacy);
        let expected = bitmap_physical_samples(&legacy);
        let resolution = PhysicalResolution {
            x_dpi: dpi,
            y_dpi: dpi,
            unit: PhysicalUnit::Custom(CustomUnit { name: "Quarter inch".into(), factor: 4.0, digits: 3, symbol: "qi".into(), abbreviation: "qi".into() }),
        };
        let mut canonical = bitmap_physical_fixture(300.0, Some(resolution.clone()));
        bitmap_physical_run(&mut canonical);
        let actual = bitmap_physical_samples(&canonical);
        assert_eq!(actual.iter().zip(&expected).filter(|(a, b)| a != b).count(), 0, "canonical square DPI controls lines/inch, not stale scalar");
        assert_eq!(doc(&canonical).physical_resolution, Some(resolution.clone()));
        assert_eq!(doc(&canonical).resolution_dpi.to_bits(), 300.0f32.to_bits());
        let bytes = photocraft_format::save_to_bytes(doc(&canonical), &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.physical_resolution, Some(resolution.clone()));
        assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), actual);
        assert!(canonical.undo());
        assert_eq!(doc(&canonical).mode, ColorMode::Grayscale);
        assert_eq!(doc(&canonical).physical_resolution, Some(resolution.clone()));
        assert!(canonical.redo());
        assert_eq!(bitmap_physical_samples(&canonical), actual);
        assert_eq!(doc(&canonical).physical_resolution, Some(resolution));
    }
    let mut default = Session::new();
    default.execute("file.new", json!({"width":64,"height":64,"mode":"gray","background":"#808080"})).unwrap();
    assert_eq!(doc(&default).effective_resolution(), PhysicalResolution::square(72.0));
    bitmap_physical_run(&mut default);
    assert_eq!(doc(&default).mode, ColorMode::Bitmap);
}
#[test]
fn bitmap_physical_invalid_and_unequal_are_atomic() {
    use photocraft_doc::{CustomUnit, PhysicalResolution, PhysicalUnit};
    let mut cases = vec![PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: PhysicalUnit::Inch }];
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        cases.push(PhysicalResolution::square(invalid));
    }
    cases.push(PhysicalResolution {
        x_dpi: 144.0,
        y_dpi: 144.0,
        unit: PhysicalUnit::Custom(CustomUnit { name: "bad".into(), factor: 0.0, digits: 1, symbol: "x".into(), abbreviation: "x".into() }),
    });
    for resolution in cases {
        let mut s = bitmap_physical_fixture(300.0, None);
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().physical_resolution = Some(resolution);
        let before = format!("{:?}", doc(&s));
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute("image.mode.bitmap", json!({"method":"halftone","frequency":36})).is_err());
        assert_eq!(format!("{:?}", doc(&s)), before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}
#[test]
fn bitmap_physical_non_halftone_accepts_valid_unequal_axes() {
    for method in ["threshold", "pattern", "diffusion"] {
        let mut s =
            bitmap_physical_fixture(300.0, Some(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch }));
        s.execute("image.mode.bitmap", json!({"method":method})).unwrap();
        assert_eq!(doc(&s).mode, ColorMode::Bitmap);
        assert_eq!(doc(&s).effective_resolution().y_dpi, 288.0);
    }
}

#[test]
fn bitmap_physical_oversized_custom_units_are_refused_atomically() {
    use photocraft_doc::{CustomUnit, PhysicalResolution, PhysicalUnit};
    for field in 0..3 {
        let mut s = bitmap_physical_fixture(300.0, None);
        let mut unit = CustomUnit { name: "Quarter inch".into(), factor: 4.0, digits: 3, symbol: "qi".into(), abbreviation: "qi".into() };
        let oversized = "x".repeat((1 << 20) + 1);
        match field {
            0 => unit.name = oversized,
            1 => unit.symbol = oversized,
            _ => unit.abbreviation = oversized,
        }
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().physical_resolution =
            Some(PhysicalResolution { x_dpi: 144.25, y_dpi: 144.25, unit: PhysicalUnit::Custom(unit) });
        let before = bitmap_physical_samples(&s);
        let state = s.active().unwrap();
        let history = state.history.past_len();
        let revision = state.revision;
        let id = state.doc.id;
        let error = s.execute("image.mode.bitmap", json!({"method":"halftone","frequency":36})).unwrap_err().to_string();
        assert!(error.contains("invalid physical resolution"), "{error}");
        assert_eq!(bitmap_physical_samples(&s), before);
        let state = s.active().unwrap();
        assert_eq!(state.doc.id, id);
        assert_eq!(state.doc.mode, ColorMode::Grayscale);
        assert_eq!(state.history.past_len(), history);
        assert_eq!(state.revision, revision);
        let PhysicalUnit::Custom(unit) = &state.doc.physical_resolution.as_ref().unwrap().unit else { panic!("custom unit was lost") };
        let retained = match field {
            0 => &unit.name,
            1 => &unit.symbol,
            _ => &unit.abbreviation,
        };
        assert_eq!(retained.len(), (1 << 20) + 1);
        assert!(retained.bytes().all(|byte| byte == b'x'));
    }
}
