//! Public registered ordinary Shape transparency regressions. No Source/XCF grant.
//! Whole-area legacy CompiledShape alpha is the independent exact oracle;
//! this integration target does not stand in for an unrerun private unit target.
use photocraft_doc::{Color, ColorMode, Document, Fill, GradientStyle, Layer, LayerContent, LayerId, Path, PixelFormat, SampleType, ShapeLayer, Size, Subpath};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn fixture(width: u32, gradient: bool) -> TestResult<(Session, LayerId, LayerId, Vec<f32>)> {
    let mut doc = Document::with_background(
        "Own ordinary fractional Shape alpha",
        Size::new(width, 80),
        ColorMode::Rgb,
        SampleType::F32,
        Color::rgba(0.25, 0.5, 0.75, 1.0),
    );
    let opaque = doc.layers.first().ok_or("missing actual opaque raster")?.id;
    let mut shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(2.5, 3.0), (90.5, 5.5), (76.0, 66.0), (3.0, 54.0)])]),
        fill: Some(Fill::Solid(Color::rgba(0.2, 0.3, 0.7, 0.375))),
        ..Default::default()
    };
    if gradient {
        shape.fill = Some(Fill::Gradient {
            stops: vec![(0.0, Color::rgba(0.2, 0.3, 0.7, 0.1875)), (1.0, Color::rgba(0.7, 0.5, 0.2, 0.8125))],
            angle: 23.5,
            scale: 0.81,
            style: GradientStyle::Linear,
            reverse: false,
            opacity_stops: vec![(0.0, 0.375), (1.0, 0.875)],
            midpoints: vec![0.375],
            offset: (0.125, -0.0625),
            dither: true,
            align: false,
        });
    }
    assert!(shape.cache.is_none(), "no offered cached-alpha shortcut");
    let area = doc.bounds();
    // width65 deliberately crops the original fractional polygon. width5000
    // needs a whole horizontal row larger than the old4096-cell sample target.
    let oracle: Vec<_> =
        photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, area).render_rgba(area).into_iter().map(|pixel| pixel[3]).collect();
    assert!(oracle.iter().any(|a| *a > 0.0 && *a < 1.0));
    let mut layer = Layer::new("Actual cacheless fractional Shape", LayerContent::Shape(shape));
    let shape_id = layer.id;
    // Intrinsic transparency must not turn into composite visibility/opacity.
    layer.visible = false;
    layer.opacity = 0.25;
    doc.layers.push(layer);
    let mut session = Session::new();
    session.add_document(doc, None)?;
    Ok((session, shape_id, opaque, oracle))
}

fn assert_f32_words(surface: &Surface, area: Rect, expected: &[f32]) -> TestResult {
    assert_eq!(surface.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    let raw = surface.try_read_interleaved(area)?;
    let (words, tail) = raw.as_chunks::<4>();
    assert!(tail.is_empty());
    assert_eq!(words.len(), expected.len());
    for (index, (word, expected)) in words.iter().zip(expected).enumerate() {
        assert_eq!(*word, expected.to_ne_bytes(), "native alpha word index={index} xy=({}, {})", index % area.width() as usize, index / area.width() as usize);
    }
    Ok(())
}

fn calculation(shape: LayerId, opaque: LayerId, name: &str) -> Value {
    json!({
        "source1":{"layer":opaque.0,"channel":"transparency"},
        "source2":{"layer":shape.0,"channel":"transparency"},
        "blending":"multiply", "opacity":100, "result":"newChannel", "name":name
    })
}

#[test]
fn registered_calculations_keep_exact_fractional_shape_alpha_f32_and_one_native_undo() -> TestResult {
    for width in [65, 100, 257, 5000] {
        for gradient in [false, true] {
            let (mut session, shape, opaque, oracle) = fixture(width, gradient)?;
            let before = session.active().ok_or("active fixture")?.doc.clone();
            let past = session.active().ok_or("active fixture")?.history.past_len();
            let revision = session.active().ok_or("active fixture")?.revision;
            let result = session.execute("image.calculations", calculation(shape, opaque, "Exact Shape alpha"))?;
            assert_eq!(result["channel"].as_u64(), Some(before.channels.len() as u64));
            let after = session.active().ok_or("active result")?.doc.clone();
            let channel = after.channels.last().ok_or("new calculated channel")?;
            assert_eq!(channel.name, "Exact Shape alpha");
            assert_eq!(after.channels.len(), before.channels.len() + 1);
            // Calculations takes source2 as base. alpha + (alpha*1-alpha)*1
            // preserves exactly the old Shape alpha, without cancellation from1.
            assert_f32_words(&channel.surface, after.bounds(), &oracle)?;
            let mut expected = (*before).clone();
            expected.channels.push(channel.clone());
            assert_eq!(*after, expected, "only actual new channel may differ");
            assert_eq!(session.active().ok_or("active result")?.history.past_len(), past + 1);
            assert_eq!(session.active().ok_or("active result")?.revision, revision + 1);
            session.execute("edit.undo", json!({}))?;
            assert_eq!(session.active().ok_or("after undo")?.doc, before);
            session.execute("edit.redo", json!({}))?;
            assert_eq!(session.active().ok_or("after redo")?.doc, after);
            let bytes = photocraft_format::save_to_bytes(&after, &Default::default())?;
            let restored = photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: true, ..Default::default() })?;
            assert_eq!(restored, *after);
            assert_f32_words(&restored.channels.last().ok_or("restored channel")?.surface, restored.bounds(), &oracle)?;
            let mut resumed = Session::new();
            resumed.add_document(restored, None)?;
            let resumed_before = resumed.active().ok_or("native active")?.doc.clone();
            let resumed_past = resumed.active().ok_or("native active")?.history.past_len();
            resumed.execute("image.calculations", calculation(shape, opaque, "Continued exact Shape alpha"))?;
            let continued = resumed.active().ok_or("continued native result")?.doc.clone();
            assert_f32_words(&continued.channels.last().ok_or("continued channel")?.surface, continued.bounds(), &oracle)?;
            assert_eq!(resumed.active().ok_or("continued active")?.history.past_len(), resumed_past + 1);
            let mut expected = (*resumed_before).clone();
            expected.channels.push(continued.channels.last().ok_or("continued channel")?.clone());
            assert_eq!(*continued, expected);
            resumed.execute("edit.undo", json!({}))?;
            assert_eq!(resumed.active().ok_or("continued undo")?.doc, resumed_before);
            resumed.execute("edit.redo", json!({}))?;
            assert_eq!(resumed.active().ok_or("continued redo")?.doc, continued);
        }
    }
    Ok(())
}

#[test]
fn registered_load_selection_keeps_exact_legacy_gray8_quantization_and_native_custody() -> TestResult {
    for width in [65, 100, 257, 5000] {
        for gradient in [false, true] {
            let (mut session, shape, _, oracle) = fixture(width, gradient)?;
            let before = session.active().ok_or("active fixture")?.doc.clone();
            let past = session.active().ok_or("active fixture")?.history.past_len();
            let revision = session.active().ok_or("active fixture")?.revision;
            session.execute("select.loadSelection", json!({"layer":shape.0,"channel":"transparency"}))?;
            let after = session.active().ok_or("loaded selection")?.doc.clone();
            let selection = after.selection.as_ref().ok_or("nonempty loaded selection")?;
            assert_eq!(selection.format(), PixelFormat::GRAY8);
            let expected: Vec<u8> = oracle.iter().map(|alpha| (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
            assert_eq!(selection.try_read_interleaved(after.bounds())?, expected, "legacy Gray8 encoding is deliberate, not F32 equality");
            let mut model = (*before).clone();
            model.selection = Some(selection.clone());
            assert_eq!(*after, model, "only actual selection may differ");
            assert_eq!(session.active().ok_or("loaded selection")?.history.past_len(), past + 1);
            assert_eq!(session.active().ok_or("loaded selection")?.revision, revision + 1);
            session.execute("edit.undo", json!({}))?;
            assert_eq!(session.active().ok_or("selection undo")?.doc, before);
            session.execute("edit.redo", json!({}))?;
            assert_eq!(session.active().ok_or("selection redo")?.doc, after);
            let restored = photocraft_format::load_from_bytes_with(
                &photocraft_format::save_to_bytes(&after, &Default::default())?,
                &photocraft_format::LoadOptions { preserve_ids: true, ..Default::default() },
            )?;
            assert_eq!(restored, *after);
            assert_eq!(restored.selection.as_ref().ok_or("native selection")?.try_read_interleaved(restored.bounds())?, expected);
        }
    }
    Ok(())
}
