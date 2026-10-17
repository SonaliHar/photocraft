//! Type layers read from a file, made ready for the canvas and the panels.

use photocraft_doc::{Document, Layer, LayerContent, TextLayer};

/// Largest type raster the import draws on its own (pixels). The model comes from an untrusted
/// file: an absurd type size, scale or box must not make opening it allocate gigabytes (the
/// renderer's own guard is far larger). A layer over it keeps empty pixels, as before, until
/// it is edited.
pub(crate) const MAX_RENDER_PIXELS: u64 = 64 * 1024 * 1024;

/// Renders `t` from its model unless its ink would exceed [`MAX_RENDER_PIXELS`].
#[cfg(test)]
fn render_bounded(eng: &mut photocraft_text::TextEngine, t: &mut TextLayer, dpi: f32, format: photocraft_color::PixelFormat) {
    let layout = eng.layout(t, dpi);
    let warp = photocraft_text::render::layout_deform(&layout, t);
    let ink = photocraft_text::render::ink_rect_warped(&layout, &t.transform, warp.as_ref());
    if u64::from(ink.width()).saturating_mul(u64::from(ink.height())) > MAX_RENDER_PIXELS {
        t.cache = Some(photocraft_raster::Surface::new(photocraft_color::PixelFormat { alpha: true, ..format }));
        return;
    }
    let r = photocraft_text::render::rasterize_warped(&layout, &t.transform, format, t.antialias, warp.as_ref());
    t.cache = Some(r.surface);
}

fn render_bounded_with_resolution(
    eng: &mut photocraft_text::TextEngine,
    t: &mut TextLayer,
    resolution: &photocraft_doc::PhysicalResolution,
    format: photocraft_color::PixelFormat,
) -> Result<(), crate::IoError> {
    let layout =
        eng.layout_with_resolution(t, resolution).map_err(|error| crate::IoError::Unsupported(format!("PSD missing text cache resolution: {error}")))?;
    let warp = photocraft_text::render::layout_warp(&layout, t.warp.as_ref());
    let ink = photocraft_text::render::ink_rect_warped(&layout, &t.transform, warp.as_ref());
    if u64::from(ink.width()).saturating_mul(u64::from(ink.height())) > MAX_RENDER_PIXELS {
        t.cache = Some(photocraft_raster::Surface::new(photocraft_color::PixelFormat { alpha: true, ..format }));
        return Ok(());
    }
    let rendered = photocraft_text::render::rasterize_warped(&layout, &t.transform, format, t.antialias, warp.as_ref());
    t.cache = Some(rendered.surface);
    Ok(())
}

/// Validate only the physical layout that this adapter will actually generate.
/// The immutable census also proves the recursive mutation walk stays within model bounds.
fn checked_layout_resolution(doc: &Document) -> Result<Option<photocraft_doc::PhysicalResolution>, crate::IoError> {
    let failure = |message: &str| crate::IoError::Unsupported(format!("PSD missing text cache resolution: {message}"));
    let mut pending = Vec::new();
    pending.try_reserve(1).map_err(|_| failure("layer traversal allocation failed"))?;
    pending.push((doc.layers.as_slice(), 0usize));
    let mut visited = 0usize;
    let mut needs_render = false;
    while let Some((layers, depth)) = pending.pop() {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(failure("layer depth limit exceeded"));
        }
        visited = visited.checked_add(layers.len()).ok_or_else(|| failure("layer count overflow"))?;
        if visited > 100_000 {
            return Err(failure("layer count limit exceeded"));
        }
        for layer in layers {
            match &layer.content {
                LayerContent::Text(text) => needs_render |= text.cache.is_none(),
                LayerContent::Group(group) => {
                    pending.try_reserve(1).map_err(|_| failure("layer traversal allocation failed"))?;
                    pending.push((group.children.as_slice(), depth + 1));
                }
                _ => {}
            }
        }
    }
    if !needs_render {
        return Ok(None);
    }
    let legacy;
    let resolution = if let Some(resolution) = &doc.physical_resolution {
        resolution
    } else {
        legacy = photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
        &legacy
    };
    resolution.validate().map_err(|error| failure(&error.to_string()))?;
    Ok(Some(resolution.clone()))
}

/// Prepares the type layers of a document that was just imported:
/// - a run whose font came as a PostScript name (PSD files) gets the installed family that name
///   resolves to, as the layout does, so Missing Fonts and the Character panel see the real
///   family (`CCWildWords-Regular` is the family "CCWildWords", not the guessed "CC Wild Words");
/// - a type layer without pixels (ag-psd, GIMP and other writers leave the image data out and
///   let Photoshop re-render on open) is rendered from its model, instead of staying empty
///   until it is edited.
pub(crate) fn prepare(doc: &mut Document) -> Result<(), crate::IoError> {
    let resolution = checked_layout_resolution(doc)?;
    let format = doc.pixel_format();
    let mut eng = photocraft_text::try_shared()?.lock().unwrap_or_else(|e| e.into_inner());
    // Clone only text models, after bounded ID/structure/COW admission in L1.
    let mut staged =
        photocraft_doc::staged_text::try_clone_text_layers(&doc.layers, photocraft_raster::MAX_CHECKED_RASTER_BYTES, &photocraft_raster::Interrupt::NONE)
            .map_err(|error| crate::IoError::Unsupported(format!("PSD text staging admission: {error}")))?;
    for (_, text) in &mut staged {
        let mut changed = false;
        for run in &mut text.runs {
            if let Some(ps) = &run.style.postscript_name
                && !eng.fonts.faces(&run.style.font_family).iter().any(|face| face.postscript_name.as_deref() == Some(ps.as_str()))
            {
                let font = eng.fonts.resolve_postscript(ps);
                let installed = eng.fonts.has_family(&run.style.font_family);
                let exact_recovery =
                    font.exact && (!installed || font.family.strip_prefix(&run.style.font_family).is_some_and(|suffix| suffix.starts_with('-')));
                if exact_recovery || (!installed && eng.fonts.has_family(&font.family)) {
                    run.style.font_family = font.family;
                    run.style.weight = font.weight;
                    run.style.italic = font.italic;
                    changed = true;
                }
            }
        }
        if changed {
            text.sync_summary();
        }
        if text.cache.is_none()
            && let Some(resolution) = &resolution
        {
            render_bounded_with_resolution(&mut eng, text, resolution, format)?;
        }
    }
    staged.sort_unstable_by_key(|(id, _)| *id);
    fn publication_matches(layers: &[Layer], staged: &[(photocraft_doc::LayerId, TextLayer)]) -> bool {
        layers.iter().all(|layer| match &layer.content {
            LayerContent::Text(_) => staged.binary_search_by_key(&layer.id, |(id, _)| *id).is_ok(),
            LayerContent::Group(group) => publication_matches(&group.children, staged),
            _ => true,
        })
    }
    if !publication_matches(&doc.layers, &staged) {
        return Err(crate::IoError::Unsupported("PSD text staging identity changed".into()));
    }
    // The immutable census established unique IDs and bounded tree depth; the
    // document has not changed while preparing models. Publication changes only Text.
    fn publish(layers: &mut [Layer], staged: &mut [(photocraft_doc::LayerId, TextLayer)]) {
        for layer in layers {
            match &mut layer.content {
                LayerContent::Text(text) => {
                    if let Ok(index) = staged.binary_search_by_key(&layer.id, |(id, _)| *id)
                        && let Some((_, prepared)) = staged.get_mut(index)
                    {
                        std::mem::swap(text, prepared);
                    }
                }
                LayerContent::Group(group) => publish(&mut group.children, staged),
                _ => {}
            }
        }
    }
    publish(&mut doc.layers, &mut staged);
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod adapter_tests {
    use super::*;

    #[test]
    fn fallible_prepare_still_resolves_postscript_names_and_renders_missing_cache() {
        let mut doc = Document::new("font adapter", photocraft_doc::Size::new(128, 96), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut text = TextLayer { text: "AV import preparation".into(), ..Default::default() };
        let mut style = text.char_runs()[0].style.clone();
        style.font_family = "Unresolved source spelling".into();
        style.postscript_name = Some("Inter-Regular".into());
        text.runs = vec![photocraft_doc::text::TextRun { len: text.text.len(), style }];
        doc.layers.push(Layer::new("Editable source type", LayerContent::Text(text)));
        prepare(&mut doc).unwrap();
        let LayerContent::Text(text) = &doc.layers[0].content else { panic!("type model was lost") };
        assert_eq!(text.runs[0].style.font_family, "Inter");
        assert!(text.cache.as_ref().is_some_and(|surface| !surface.content_bounds().is_empty()));
    }
    #[test]
    fn late_invalid_missing_cache_refuses_all_font_run_and_cache_mutation() {
        let mut doc = Document::new("atomic type adapter", photocraft_doc::Size::new(32, 24), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch }).unwrap();
        // First layer would resolve its PostScript spelling; second layer requires rendering.
        // The preflight must find the latter before any mutation of the earlier layer.
        let mut first = TextLayer { text: "Keep source spelling".into(), ..Default::default() };
        let mut style = first.char_runs()[0].style.clone();
        style.font_family = "Unresolved source spelling".into();
        style.postscript_name = Some("Inter-Regular".into());
        first.runs = vec![photocraft_doc::text::TextRun { len: first.text.len(), style }];
        first.cache = Some(photocraft_raster::Surface::new(doc.pixel_format()));
        doc.layers.push(Layer::new("Cached first", LayerContent::Text(first)));
        let mut later = TextLayer { text: "AV late".into(), ..Default::default() };
        let mut invalid_style = later.char_runs()[0].style.clone();
        invalid_style.size_pt = f32::INFINITY;
        later.runs = vec![photocraft_doc::text::TextRun { len: later.text.len(), style: invalid_style }];
        doc.layers.push(Layer::new("Missing later", LayerContent::Text(later)));
        let before = doc.clone();
        let error = prepare(&mut doc).unwrap_err();
        assert!(matches!(error, crate::IoError::Unsupported(ref message) if message.contains("unrepresentable geometry")));
        assert_eq!(doc, before);
    }

    #[test]
    fn unequal_missing_cache_uses_paired_layout_and_retains_painted_cache() {
        let mut doc = Document::new("unequal type adapter", photocraft_doc::Size::new(128, 96), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let resolution = photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch };
        doc.physical_resolution = Some(resolution.clone());
        doc.resolution_dpi = 72.0;
        let cached = TextLayer { text: "Source pixels".into(), cache: Some(photocraft_raster::Surface::new(doc.pixel_format())), ..Default::default() };
        let before_cache = cached.cache.clone();
        doc.layers.push(Layer::new("Painted", LayerContent::Text(cached)));
        let missing = TextLayer { text: "AV physical axes".into(), ..Default::default() };
        let mut expected = missing.clone();
        let mut engine = photocraft_text::TextEngine::new();
        render_bounded_with_resolution(&mut engine, &mut expected, &resolution, doc.pixel_format()).unwrap();
        doc.layers.push(Layer::new("Missing", LayerContent::Text(missing)));
        prepare(&mut doc).unwrap();
        let LayerContent::Text(painted) = &doc.layers[0].content else { panic!("painted type lost") };
        let LayerContent::Text(actual) = &doc.layers[1].content else { panic!("missing type lost") };
        assert_eq!(painted.cache, before_cache);
        assert_eq!(actual.cache, expected.cache);
        assert_eq!(doc.physical_resolution, Some(resolution));
        assert_eq!(doc.resolution_dpi.to_bits(), 72.0f32.to_bits());
    }

    #[test]
    fn missing_cache_uses_canonical_square_dpi_before_legacy_scalar() {
        let mut doc =
            Document::new("canonical type adapter", photocraft_doc::Size::new(32, 24), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.physical_resolution = Some(photocraft_doc::PhysicalResolution::square(144.0));
        doc.resolution_dpi = 72.0;
        let mut expected = TextLayer { text: "AV canonical".into(), ..Default::default() };
        let mut engine = photocraft_text::TextEngine::new();
        render_bounded(&mut engine, &mut expected, 144.0, doc.pixel_format());
        doc.layers.push(Layer::new("Missing cache", LayerContent::Text(TextLayer { text: "AV canonical".into(), ..Default::default() })));
        prepare(&mut doc).unwrap();
        let LayerContent::Text(actual) = &doc.layers[0].content else { panic!("type lost") };
        assert_eq!(actual.cache, expected.cache);
        assert_eq!(doc.resolution_dpi.to_bits(), 72.0f32.to_bits());
        assert_eq!(doc.physical_resolution, Some(photocraft_doc::PhysicalResolution::square(144.0)));
    }
    fn preflight_document() -> Document {
        let mut doc =
            Document::new("hostile adapter preflight", photocraft_doc::Size::new(32, 24), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut first = TextLayer { text: "Keep source spelling".into(), ..Default::default() };
        let mut style = first.char_runs()[0].style.clone();
        style.font_family = "Unresolved source spelling".into();
        style.postscript_name = Some("Inter-Regular".into());
        first.runs = vec![photocraft_doc::text::TextRun { len: first.text.len(), style }];
        first.cache = Some(photocraft_raster::Surface::new(doc.pixel_format()));
        doc.layers.push(Layer::new("Cached first", LayerContent::Text(first)));
        doc.layers.push(Layer::new("Missing later", LayerContent::Text(TextLayer { text: "AV late".into(), ..Default::default() })));
        doc
    }
    fn refusal_is_atomic(mut doc: Document, reason: &str) {
        let before = doc.clone();
        let result = prepare(&mut doc);
        assert!(matches!(&result, Err(crate::IoError::Unsupported(message)) if message.contains(reason)), "{result:?}");
        assert_eq!(doc.resolution_dpi.to_bits(), before.resolution_dpi.to_bits());
        // Structural equality separately from float bits makes NaN fixtures meaningful.
        let mut before = before;
        doc.resolution_dpi = 1.0;
        before.resolution_dpi = 1.0;
        if let (Some(actual), Some(expected)) = (&mut doc.physical_resolution, &mut before.physical_resolution) {
            assert_eq!(actual.x_dpi.to_bits(), expected.x_dpi.to_bits());
            assert_eq!(actual.y_dpi.to_bits(), expected.y_dpi.to_bits());
            actual.x_dpi = 1.0;
            expected.x_dpi = 1.0;
            actual.y_dpi = 1.0;
            expected.y_dpi = 1.0;
            if let (photocraft_doc::PhysicalUnit::Custom(actual), photocraft_doc::PhysicalUnit::Custom(expected)) = (&mut actual.unit, &mut expected.unit) {
                assert_eq!(actual.factor.to_bits(), expected.factor.to_bits());
                actual.factor = 1.0;
                expected.factor = 1.0;
            }
        }
        assert_eq!(doc, before);
    }
    #[test]
    fn hostile_depth_refuses_before_earlier_font_run_mutation() {
        let mut doc = preflight_document();
        let mut nested = doc.layers.pop().unwrap();
        for _ in 0..=photocraft_doc::MAX_GROUP_DEPTH {
            nested = Layer::new("Nested", LayerContent::Group(photocraft_doc::Group { children: vec![nested], expanded: false, artboard: None }));
        }
        doc.layers.push(nested);
        refusal_is_atomic(doc, "layer depth");
    }
    #[test]
    fn hostile_count_refuses_before_earlier_font_run_mutation() {
        let mut doc = preflight_document();
        for _ in doc.layers.len()..100_001 {
            doc.layers.push(Layer::raster("Empty", doc.pixel_format()));
        }
        refusal_is_atomic(doc, "layer count");
    }
    #[test]
    fn invalid_canonical_axes_and_unrepresentable_scalar_refuse_atomically() {
        for dpi in [0.0, f64::NAN, f64::INFINITY, f64::MIN_POSITIVE, f64::MAX] {
            let mut doc = preflight_document();
            doc.physical_resolution = Some(photocraft_doc::PhysicalResolution::square(dpi));
            refusal_is_atomic(doc, "text cache resolution");
        }
    }
    #[test]
    fn invalid_legacy_dpi_refuses_atomically() {
        for dpi in [0.0, f32::NAN] {
            let mut doc = preflight_document();
            doc.resolution_dpi = dpi;
            refusal_is_atomic(doc, "text cache resolution");
        }
    }
    #[test]
    fn malformed_custom_unit_refuses_but_valid_custom_square_renders() {
        let unit = photocraft_doc::CustomUnit { name: "Custom physical unit".into(), factor: 2.54, digits: 3, symbol: "cu".into(), abbreviation: "cu".into() };
        for factor in [0.0, f64::NAN, f64::INFINITY] {
            let mut doc = preflight_document();
            doc.physical_resolution = Some(photocraft_doc::PhysicalResolution {
                x_dpi: 144.0,
                y_dpi: 144.0,
                unit: photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit { factor, ..unit.clone() }),
            });
            refusal_is_atomic(doc, "custom unit");
        }
        let mut doc = preflight_document();
        doc.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 144.0, unit: photocraft_doc::PhysicalUnit::Custom(unit) });
        doc.resolution_dpi = f32::NAN; // Valid canonical axes are the authority.
        prepare(&mut doc).unwrap();
        let LayerContent::Text(first) = &doc.layers[0].content else { panic!("cached type lost") };
        assert_eq!(first.runs[0].style.font_family, "Inter");
        let LayerContent::Text(second) = &doc.layers[1].content else { panic!("generated type lost") };
        assert!(second.cache.as_ref().is_some_and(|cache| !cache.content_bounds().is_empty()));
        assert!(doc.resolution_dpi.is_nan());
    }
}
