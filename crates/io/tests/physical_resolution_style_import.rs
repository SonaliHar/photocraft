//! Public named-style adapter obeys authoritative physical resolution after native reopen.
use photocraft_doc::{
    ColorMode, CustomUnit, Document, Layer, LayerContent, PhysicalResolution, PhysicalUnit, SampleType, Size, TextLayer, TextStyles,
    text::{CharStyle, TextRun},
    text_styles::{CharacterStyleDef, diff_attrs},
};
use std::sync::Arc;

fn source() -> Document {
    let mut styles = TextStyles::default();
    styles.character.push(CharacterStyleDef {
        id: 1,
        name: "Own30pt".into(),
        attrs: diff_attrs(&CharStyle { size_pt: 30.0, ..Default::default() }, &CharStyle::default()),
    });
    let mut text = TextLayer { text: "calm OWN".into(), ..Default::default() };
    text.runs = vec![
        TextRun { len: 5, style: CharStyle { font_family: "Inter".into(), ..Default::default() } },
        TextRun { len: 3, style: styles.resolve_char(None, Some(1), "Inter") },
    ];
    text.sync_summary();
    let raw = photocraft_text::psd::build_tysh(&text, 144.0, None);
    let raw = photocraft_text::psd_styles::write_style_sheets(&raw, &text, &styles, 144.0).unwrap();
    assert_eq!(
        photocraft_text::psd_styles::read_style_sheets(&raw, 144.0).unwrap().char_sheets.iter().find(|(_, name, _)| name == "Own30pt").unwrap().2.size_pt,
        30.0
    );
    text.psd_raw = Some(Arc::new(raw));
    for run in &mut text.runs {
        run.style.style_sheet = None;
    }
    let mut doc = Document::new("Own named style", Size::new(128, 96), ColorMode::Rgb, SampleType::U8);
    doc.physical_resolution = Some(PhysicalResolution::square(144.0));
    doc.resolution_dpi = 72.0;
    doc.layers.push(Layer::new("Own type", LayerContent::Text(text)));
    doc.text_styles.character.push(CharacterStyleDef { id: 77, name: "Existing".into(), attrs: Default::default() });
    doc.validate_xcf_state().unwrap();
    doc
}
fn size(doc: &Document) -> Option<f64> {
    doc.text_styles.character.iter().find(|style| style.name == "Own30pt").and_then(|style| style.attrs.get("size_pt")).and_then(|value| value.as_f64())
}
#[test]
fn native_reopen_then_public_style_adapter_uses_canonical_square_dpi() {
    let doc = source();
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let mut reopened = photocraft_io::import("own.pcraft", &native).unwrap().document;
    assert_eq!(reopened, doc);
    assert_eq!(size(&reopened), None, "native import does not automatically run the style adapter");
    assert!(photocraft_io::text_styles_map::import(&mut reopened).is_empty());
    assert_eq!(size(&reopened), Some(30.0));
    let LayerContent::Text(text) = &reopened.layers[0].content else { panic!("type") };
    let style = reopened.text_styles.character.iter().find(|style| style.name == "Own30pt").unwrap();
    assert_eq!(text.runs[1].style.style_sheet, Some(style.id));
    assert_eq!(
        text.psd_raw,
        match &doc.layers[0].content {
            LayerContent::Text(t) => t.psd_raw.clone(),
            _ => None,
        }
    );
    let native = photocraft_format::save_to_bytes(&reopened, &Default::default()).unwrap();
    assert_eq!(photocraft_io::import("continued.pcraft", &native).unwrap().document, reopened);
}
#[test]
fn unequal_resolution_uses_y_basis_and_malformed_resolution_refuses_atomically() {
    for case in 0..5 {
        let mut doc = source();
        let resolution = doc.physical_resolution.as_mut().unwrap();
        match case {
            0 => resolution.y_dpi = 72.0,
            1 => resolution.x_dpi = 0.0,
            2 => resolution.y_dpi = f64::INFINITY,
            3 => {
                resolution.unit =
                    PhysicalUnit::Custom(CustomUnit { name: "Own unit".into(), factor: 0.0, digits: 2, symbol: "u".into(), abbreviation: "u".into() })
            }
            _ => {
                resolution.x_dpi = f64::MAX;
                resolution.y_dpi = f64::MAX;
            }
        }
        let before = doc.clone();
        let warnings = photocraft_io::text_styles_map::import(&mut doc);
        if case == 0 {
            // Paired-axis layout is supported: named point sizes use the retained Y-DPI basis.
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(size(&doc), Some(60.0));
            assert_eq!(doc.physical_resolution, before.physical_resolution);
            assert_eq!(doc.resolution_dpi, before.resolution_dpi);
            let LayerContent::Text(text) = &doc.layers[0].content else { panic!("type") };
            let LayerContent::Text(original) = &before.layers[0].content else { panic!("type") };
            assert_eq!(text.psd_raw, original.psd_raw);
            let style = doc.text_styles.character.iter().find(|style| style.name == "Own30pt").unwrap();
            assert_eq!(text.runs[1].style.style_sheet, Some(style.id));
            continue;
        }
        assert!(!warnings.is_empty(), "case {case}: unqualified context silently interpreted");
        assert!(warnings[0].contains("Named PSD styles"));
        assert_eq!(doc, before, "case {case}: refusal changed model/styles/raw");
    }
}

#[test]
fn validated_custom_unit_square_uses_canonical_dpi() {
    let mut doc = source();
    doc.physical_resolution.as_mut().unwrap().unit =
        PhysicalUnit::Custom(CustomUnit { name: "Own unit".into(), factor: 2.0, digits: 2, symbol: "u".into(), abbreviation: "u".into() });
    assert!(photocraft_io::text_styles_map::import(&mut doc).is_empty());
    assert_eq!(size(&doc), Some(30.0));
}

#[test]
fn scalar_legacy_fallback_is_retained_without_canonical_override() {
    let mut doc = source();
    doc.physical_resolution = None;
    doc.resolution_dpi = 144.0;
    assert!(photocraft_io::text_styles_map::import(&mut doc).is_empty());
    assert_eq!(size(&doc), Some(30.0));
}

#[test]
fn public_square_psd_import_keeps_named_styles_at_thirty_points() {
    use photocraft_psd::{Compression, ImageResource, ResolutionInfo, TaggedBlock, Version, resources::ids};
    let doc = source();
    let LayerContent::Text(text) = &doc.layers[0].content else { panic!("type") };
    let raw = text.psd_raw.as_ref().unwrap().as_ref().clone();
    let mut file = photocraft_psd::testgen::small(Version::Psd, Compression::Raw);
    let mut record = file.layers()[0].clone();
    record.blocks = vec![TaggedBlock::new(*b"TySh", raw.clone())];
    file.layers_mut().clear();
    file.layers_mut().push(record);
    file.resources = vec![ImageResource::new(ids::RESOLUTION_INFO, ResolutionInfo::from_dpi(144.0).to_bytes())];
    let imported = photocraft_io::import("own.psd", &file.to_bytes().unwrap()).unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    assert_eq!(imported.document.resolution_dpi, 144.0);
    assert_eq!(size(&imported.document), Some(30.0));
    let LayerContent::Text(text) = &imported.document.layers[0].content else { panic!("type") };
    assert_eq!(text.psd_raw.as_ref().unwrap().as_slice(), raw);
}

#[test]
fn unequal_raster_and_unnamed_text_are_no_op_controls() {
    for raster in [false, true] {
        let mut doc = source();
        doc.physical_resolution.as_mut().unwrap().y_dpi = 72.0;
        if raster {
            doc.layers[0].content = LayerContent::Raster(photocraft_raster::Surface::new(doc.pixel_format()));
        } else {
            let LayerContent::Text(text) = &mut doc.layers[0].content else { panic!("type") };
            text.psd_raw = None;
            let raw = photocraft_text::psd::build_tysh(text, 144.0, None);
            assert!(!photocraft_text::psd_styles::has_named_sheets(&raw));
            text.psd_raw = Some(Arc::new(raw));
        }
        let before = doc.clone();
        assert!(photocraft_io::text_styles_map::import(&mut doc).is_empty());
        assert_eq!(doc, before);
    }
}

#[test]
fn invalid_layer_identities_and_excess_depth_refuse_atomically() {
    for case in 0..3 {
        let mut doc = source();
        match case {
            0 => doc.layers[0].id = photocraft_doc::LayerId(0),
            1 => doc.layers.push(doc.layers[0].clone()),
            _ => {
                let mut layer = doc.layers.remove(0);
                for _ in 0..101 {
                    layer = Layer::group("Own nesting", vec![layer]);
                }
                doc.layers.push(layer);
            }
        }
        let before = doc.clone();
        let warnings = photocraft_io::text_styles_map::import(&mut doc);
        assert!(!warnings.is_empty());
        assert_eq!(doc, before);
    }
}

#[test]
fn layer_count_ceiling_accepts_boundary_and_refuses_overflow_before_mutation() {
    let mut doc = source();
    doc.layers.try_reserve(100000).unwrap();
    for _ in 1..100000 {
        doc.layers.push(Layer::raster("Own ordinary", doc.pixel_format()));
    }
    assert!(photocraft_io::text_styles_map::import(&mut doc).is_empty());
    assert_eq!(size(&doc), Some(30.0));
    let styles = doc.text_styles.clone();
    let original_text = match &doc.layers[0].content {
        LayerContent::Text(text) => text.clone(),
        _ => panic!("type"),
    };
    doc.layers.push(Layer::raster("Own over limit", doc.pixel_format()));
    let warnings = photocraft_io::text_styles_map::import(&mut doc);
    assert!(warnings.iter().any(|w| w.contains("count limit")));
    assert_eq!(doc.text_styles, styles);
    assert_eq!(
        match &doc.layers[0].content {
            LayerContent::Text(text) => text,
            _ => panic!("type"),
        },
        &original_text
    );
    assert_eq!(doc.layers.len(), 100001);
}
