//! Typed PSD point sizes use the canonical physical resolution.
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, PhysicalResolution, Size, TextLayer};

fn source(depth: SampleType) -> Document {
    let mut doc = Document::new("Canonical PSD type", Size::new(32, 24), ColorMode::Rgb, depth);
    doc.resolution_dpi = 72.0;
    doc.physical_resolution = Some(PhysicalResolution::square(144.0));
    let mut text = TextLayer { text: "Physical type".into(), ..Default::default() };
    let mut style = text.char_runs()[0].style.clone();
    style.size_pt = 24.0;
    text.runs = vec![photocraft_doc::text::TextRun { len: text.text.len(), style }];
    text.sync_summary();
    // Cached pixels are retained independently of the generated editable TySh.
    text.cache = Some(photocraft_raster::Surface::new(doc.pixel_format()));
    doc.layers.push(Layer::new("Editable type", LayerContent::Text(text)));
    doc
}

#[test]
fn generated_psd_type_keeps_point_size_at_authoritative_square_dpi() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let doc = source(depth);
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        for extension in ["psd", "psb"] {
            let exported = photocraft_io::export(&doc, extension, &Default::default()).unwrap();
            let reopened = photocraft_io::import(extension, &exported.bytes).unwrap().document;
            let LayerContent::Text(text) = &reopened.layers[0].content else { panic!("editable text was lost") };
            assert_eq!(text.char_runs()[0].style.size_pt, 24.0, "{depth:?}/{extension}");
            assert_eq!(reopened.effective_resolution(), PhysicalResolution::square(144.0));
        }
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
    }
}

#[test]
fn generated_psd_type_supports_paired_axes_and_refuses_invalid_canonical_axes() {
    for axes in [[144.0, 72.0], [144.0, 0.0], [f64::NAN, 144.0]] {
        let mut doc = source(SampleType::U8);
        let resolution = doc.physical_resolution.as_mut().unwrap();
        (resolution.x_dpi, resolution.y_dpi) = (axes[0], axes[1]);
        for extension in ["psd", "psb"] {
            let result = photocraft_io::export(&doc, extension, &Default::default());
            if axes == [144.0, 72.0] {
                let back = photocraft_io::import(extension, &result.unwrap().bytes).unwrap().document;
                let LayerContent::Text(text) = &back.layers[0].content else { panic!("type lost") };
                assert_eq!(text.char_runs()[0].style.size_pt, 24.0);
                assert_eq!([back.effective_resolution().x_dpi, back.effective_resolution().y_dpi], axes);
            } else {
                assert!(result.is_err(), "invalid canonical axes {axes:?}/{extension} generated editable text");
            }
        }
    }
}

#[test]
fn untouched_psd_text_can_preserve_raw_data_with_unequal_axes() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = source(depth);
        let format = doc.pixel_format();
        let bounds = photocraft_geom::Rect::new(3, 4, 5, 6);
        let LayerContent::Text(text) = &mut doc.layers[0].content else { panic!("missing type") };
        // Actual cached ink prevents this passthrough control from exercising missing-cache regeneration.
        let cache = text.cache.as_mut().unwrap();
        for (x, y, pixel) in
            [(3, 4, [0.125, 0.25, 0.75, 1.0]), (4, 4, [0.75, 0.5, 0.25, 0.5]), (3, 5, [0.25, 0.75, 0.5, 0.25]), (4, 5, [0.5, 0.125, 0.75, 0.0])]
        {
            cache.try_write_pixel_f64(x, y, &pixel).unwrap();
        }
        assert!(!cache.content_bounds().is_empty());
        let expected = cache.try_read_region_f64(bounds).unwrap();
        assert!(expected.iter().any(|sample| *sample != 0.0));
        assert_eq!(cache.format(), format);
        let raw = std::sync::Arc::new(
            photocraft_text::psd::build_tysh_with_resolution(
                text,
                &PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch },
                None,
            )
            .unwrap(),
        );
        text.psd_raw = Some(raw.clone());
        doc.physical_resolution.as_mut().unwrap().y_dpi = 288.0;
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        for extension in ["psd", "psb"] {
            let out = photocraft_io::export(&doc, extension, &Default::default()).unwrap();
            let reopened = photocraft_io::import(extension, &out.bytes).unwrap().document;
            let LayerContent::Text(text) = &reopened.layers[0].content else { panic!("missing type") };
            let preserved = text.psd_raw.as_ref().unwrap();
            assert!(preserved.starts_with(raw.as_slice()), "source TySh payload changed {depth:?}/{extension}");
            assert!(preserved.len() - raw.len() <= 3, "unexpected PSD block padding");
            assert!(preserved[raw.len()..].iter().all(|byte| *byte == 0), "nonzero PSD padding");
            assert_eq!(text.char_runs()[0].style.size_pt, 24.0);
            assert_eq!(reopened.depth, depth);
            assert_eq!((reopened.effective_resolution().x_dpi, reopened.effective_resolution().y_dpi), (144.0, 288.0));
            let cached = text.cache.as_ref().unwrap();
            assert!(!cached.content_bounds().is_empty());
            assert_eq!(cached.format(), format);
            // Compare stored samples at their actual depth, not an sRGB/display conversion.
            assert_eq!(cached.try_read_region_f64(bounds).unwrap(), expected, "cached samples changed {depth:?}/{extension}");
        }
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
    }
}

#[test]
fn named_psd_styles_use_canonical_dpi_when_rewriting_existing_or_generated_type() {
    use photocraft_doc::text::CharStyle;
    use photocraft_doc::text_styles::{CharacterStyleDef, diff_attrs};
    for existing_raw in [false, true] {
        let mut doc = source(SampleType::U16);
        doc.text_styles.character.push(CharacterStyleDef {
            id: 3,
            name: "Physical style".into(),
            attrs: diff_attrs(&CharStyle { size_pt: 30.0, ..Default::default() }, &CharStyle::default()),
        });
        let style = doc.text_styles.resolve_char(None, Some(3), "Inter");
        let LayerContent::Text(text) = &mut doc.layers[0].content else { panic!("missing type") };
        let normal = text.runs[0].style.clone();
        text.runs = vec![photocraft_doc::text::TextRun { len: 8, style: normal }, photocraft_doc::text::TextRun { len: text.text.len() - 8, style }];
        text.sync_summary();
        if existing_raw {
            text.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::build_tysh(text, 144.0, None)));
        }
        for extension in ["psd", "psb"] {
            let out = photocraft_io::export(&doc, extension, &Default::default()).unwrap();
            let back = photocraft_io::import(extension, &out.bytes).unwrap().document;
            assert_eq!(back.text_styles.character[0].attrs.get("size_pt").and_then(serde_json::Value::as_f64), Some(30.0));
            let LayerContent::Text(text) = &back.layers[0].content else { panic!("missing type") };
            assert_eq!(text.char_runs()[0].style.size_pt, 24.0);
            assert_eq!(text.char_runs()[1].style.size_pt, 30.0, "existing raw={existing_raw}/{extension}");
        }
        doc.physical_resolution.as_mut().unwrap().y_dpi = 288.0;
        let output = photocraft_io::export(&doc, "psd", &Default::default()).unwrap();
        let back = photocraft_io::import("psd", &output.bytes).unwrap().document;
        let LayerContent::Text(text) = &back.layers[0].content else { panic!("type lost") };
        assert_eq!(text.char_runs()[0].style.size_pt, 24.0);
        assert_eq!(text.char_runs()[1].style.size_pt, 30.0);
        assert_eq!(back.effective_resolution().y_dpi, 288.0);
    }
}

#[test]
fn legacy_type_and_fractional_custom_unit_keep_point_sizes() {
    use photocraft_doc::{CustomUnit, PhysicalUnit};
    for canonical in [false, true] {
        let mut doc = source(SampleType::F32);
        if canonical {
            doc.resolution_dpi = f32::NAN;
            doc.physical_resolution = Some(PhysicalResolution {
                x_dpi: 144.25,
                y_dpi: 144.25,
                unit: PhysicalUnit::Custom(CustomUnit { name: "Own unit".into(), factor: 2.5, digits: 2, symbol: "u".into(), abbreviation: "ou".into() }),
            });
        } else {
            doc.physical_resolution = None;
        }
        let out = photocraft_io::export(&doc, "psd", &Default::default()).unwrap();
        let back = photocraft_io::import("psd", &out.bytes).unwrap().document;
        let LayerContent::Text(text) = &back.layers[0].content else { panic!("missing type") };
        assert!((text.char_runs()[0].style.size_pt - 24.0).abs() < 0.00001);
        assert_eq!(back.effective_resolution().x_dpi, if canonical { 144.25 } else { 72.0 });
    }
}

#[test]
fn adding_generated_type_does_not_rewrite_unrelated_shape_resolution() {
    use photocraft_doc::vector::{LiveShape, Path, ShapeStroke, Subpath};
    use photocraft_doc::{Fill, ShapeLayer};
    let mut with_text = source(SampleType::U8);
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(1.0, 1.0), (12.0, 1.0), (12.0, 12.0), (1.0, 12.0)])]),
        fill: Some(Fill::Solid(photocraft_color::Color::BLACK)),
        stroke: Some(ShapeStroke::default()),
        live: Some(LiveShape::Rect { rect: [1.0, 1.0, 11.0, 11.0], radii: [0.0; 4] }),
        cache: Some(photocraft_raster::Surface::new(with_text.pixel_format())),
        ..Default::default()
    };
    with_text.layers.push(Layer::new("Unrelated shape", LayerContent::Shape(shape)));
    let mut without_text = with_text.clone();
    without_text.layers.remove(0);
    for extension in ["psd", "psb"] {
        let read = |doc: &Document| {
            let out = photocraft_io::export(doc, extension, &Default::default()).unwrap();
            photocraft_psd::PsdFile::from_bytes(&out.bytes).unwrap()
        };
        let before = read(&without_text);
        let after = read(&with_text);
        let find = |file: &photocraft_psd::PsdFile| {
            let layer = file.layers().iter().find(|layer| layer.name() == "Unrelated shape").unwrap();
            [*b"vmsk", *b"vstk", *b"vogk"].map(|key| layer.block(&key).unwrap().data.clone())
        };
        assert_eq!(find(&after), find(&before), "{extension}");
    }
}

#[test]
fn generated_type_refuses_dpi_outside_positive_psd_fixed_point_range() {
    for dpi in [1e-40, 1e-10, 65_536.0, 1e20] {
        let mut doc = source(SampleType::U8);
        doc.physical_resolution = Some(PhysicalResolution::square(dpi));
        let result = photocraft_io::export(&doc, "psd", &Default::default());
        assert!(result.is_err(), "DPI {dpi} would lose its positive value in PSD metadata");
    }
}

#[test]
fn generated_type_matches_the_dpi_actually_encoded_in_psd() {
    for dpi in [1.1 / 65_536.0, 144.123456789, 65_535.999] {
        let mut doc = source(SampleType::U8);
        doc.physical_resolution = Some(PhysicalResolution::square(dpi));
        let out = photocraft_io::export(&doc, "psd", &Default::default()).unwrap();
        let back = photocraft_io::import("psd", &out.bytes).unwrap().document;
        let LayerContent::Text(text) = &back.layers[0].content else { panic!("missing type") };
        assert!((text.char_runs()[0].style.size_pt - 24.0).abs() < 0.00001, "DPI {dpi}: {}pt", text.char_runs()[0].style.size_pt);
        assert_eq!(doc.physical_resolution.as_ref().unwrap().x_dpi, dpi);
    }
}

#[test]
fn type_export_rejects_excessive_depth_and_invalid_custom_metadata_before_staging() {
    use photocraft_doc::{CustomUnit, Group, PhysicalUnit};
    let mut deep = source(SampleType::U8);
    let mut children = std::mem::take(&mut deep.layers);
    for _ in 0..=photocraft_doc::MAX_GROUP_DEPTH {
        children = vec![Layer::new("Nested", LayerContent::Group(Group { children, expanded: false, artboard: None }))];
    }
    deep.layers = children;
    assert!(photocraft_io::export(&deep, "psd", &Default::default()).is_err());
    let mut doc = source(SampleType::U8);
    doc.physical_resolution.as_mut().unwrap().unit =
        PhysicalUnit::Custom(CustomUnit { name: "Oversized own unit".repeat(65_536), factor: 1.0, digits: 2, symbol: "u".into(), abbreviation: "u".into() });
    assert!(photocraft_io::export(&doc, "psd", &Default::default()).is_err());
}
