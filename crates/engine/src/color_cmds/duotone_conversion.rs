//! Ordinary Duotone ink conversion. The caller admits the complete intermediate
//! footprint before cloning its layer tree or constructing its CMS transform.
use crate::{EngineError, Result};
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Duotone, Effect, Fill, FxPaint, Layer, LayerContent};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, Surface};

fn error(message: &str) -> EngineError {
    EngineError::Other(format!("Duotone conversion: {message}"))
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
pub(super) fn enabled(doc: &Document, destination: ColorMode) -> bool {
    doc.mode == ColorMode::Duotone && destination != ColorMode::Grayscale && doc.duotone.as_ref().is_some_and(|ink| !ink.inks.is_empty())
}
/// Add actual intermediate requirements atomically to the original phase totals.
pub(super) fn admit_extra(retained: &mut u64, work: &mut u64, extra: (u64, u64), bytes_limit: u64, work_limit: u64) -> Result<()> {
    let bytes = retained.checked_add(extra.0).filter(|n| *n <= bytes_limit).ok_or_else(|| error("ink intermediate headroom limit"))?;
    let units = work.checked_add(extra.1).filter(|n| *n <= work_limit).ok_or_else(|| error("ink intermediate work limit"))?;
    *retained = bytes;
    *work = units;
    Ok(())
}
#[derive(Clone, Copy)]
pub(super) struct Ink<'a> {
    value: &'a Duotone,
    per_render: u64,
    inventory: u64,
}
impl<'a> Ink<'a> {
    pub(super) fn selected(doc: &'a Document, destination: ColorMode, c: &Interrupt<'_>) -> Result<Option<Self>> {
        poll(c)?;
        if !enabled(doc, destination) {
            return Ok(None);
        }
        if doc.xcf_state.is_some() || !matches!(doc.depth, SampleType::U8 | SampleType::U16 | SampleType::F32) {
            return Err(error("only ordinary narrow Duotone conversion is supported"));
        }
        let value = doc.duotone.as_ref().ok_or_else(|| error("missing inks"))?;
        if value.inks.len() > 4 {
            return Err(error("at most four inks are supported"));
        }
        let mut per_render = 64u64;
        let mut inventory = 1u64;
        for ink in &value.inks {
            poll(c)?;
            if ink.curve.len() > 4096 || ink.color.iter().any(|v| !v.is_finite()) {
                return Err(error("invalid ink definition"));
            }
            let points = ink.curve.len() as u64;
            per_render = per_render.checked_add(32).and_then(|n| n.checked_add(points.max(1).checked_mul(16)?)).ok_or_else(|| error("ink work overflow"))?;
            inventory = inventory.checked_add(points).and_then(|n| n.checked_add(1)).ok_or_else(|| error("ink inventory overflow"))?;
            for point in &ink.curve {
                poll(c)?;
                if !point.input.is_finite() || !point.output.is_finite() {
                    return Err(error("nonfinite ink curve"));
                }
            }
        }
        poll(c)?;
        Ok(Some(Self { value, per_render, inventory }))
    }
    /// Three simultaneous copies cover intermediate output, fallible allocation
    /// capacity and the following CMS conversion's overlap. No earlier fee credit.
    pub(super) fn surface_requirements(&self, s: &Surface, c: &Interrupt<'_>) -> Result<(u64, u64)> {
        poll(c)?;
        if s.format().mode != ColorMode::Grayscale {
            return Ok((0, 0));
        }
        if !matches!(s.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
            return Err(error("wide ink surfaces are unsupported"));
        }
        s.try_validate_tile_origins_with(c).map_err(|failure| match failure {
            photocraft_raster::RasterInputError::Cancelled => EngineError::Cancelled,
            _ => error("invalid ink tile origins"),
        })?;
        let tiles = s.tile_count() as u64;
        let pixels = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).ok_or_else(|| error("tile pixel overflow"))?;
        let format = PixelFormat { mode: ColorMode::Rgb, ..s.format() };
        let bpp = format.bytes_per_pixel() as u64;
        let bytes = tiles
            .checked_mul(pixels.checked_mul(bpp).and_then(|n| n.checked_add(512)).ok_or_else(|| error("intermediate tile overflow"))?)
            .and_then(|n| n.checked_add(bpp))
            .ok_or_else(|| error("intermediate footprint overflow"))?;
        let visits = tiles.checked_mul(pixels).and_then(|n| n.checked_add(1)).ok_or_else(|| error("ink sample count overflow"))?;
        let work = visits
            .checked_mul(self.per_render)
            .and_then(|n| n.checked_add(visits.checked_mul(bpp)?.checked_mul(2)?))
            .ok_or_else(|| error("ink sample work overflow"))?;
        Ok((bytes.checked_mul(3).ok_or_else(|| error("intermediate peak overflow"))?, work))
    }
    pub(super) fn metadata_requirements(&self, doc: &Document, model_bytes: u64, c: &Interrupt<'_>) -> Result<(u64, u64)> {
        let mut count = 0u64;
        visit_colors(&doc.layers, 0, &mut 0, c, &mut |_| {
            count = count.checked_add(1).ok_or_else(|| error("color count overflow"))?;
            Ok(())
        })?;
        // Eight inventory passes conservatively cover admission and repeated
        // private/profile preparation; the actual bounded inventory is borrowed.
        let work = model_bytes
            .checked_mul(4)
            .and_then(|n| n.checked_add(count.checked_mul(self.per_render)?))
            .and_then(|n| n.checked_add(self.inventory.checked_mul(8)?))
            .ok_or_else(|| error("metadata work overflow"))?;
        Ok((4096, work))
    }
    fn render(&self, gray: f32) -> Result<[f32; 3]> {
        if !gray.is_finite() {
            return Err(error("nonfinite gray ink sample"));
        }
        let value = self.value.render(gray);
        if value.iter().any(|v| !v.is_finite()) {
            return Err(error("nonfinite ink output"));
        }
        Ok(value)
    }
    fn pixel(&self, source: &[u8], sf: PixelFormat, output: &mut [u8], to: PixelFormat) -> Result<()> {
        if source.len() != sf.bytes_per_pixel() || output.len() != to.bytes_per_pixel() {
            return Err(error("ink pixel length"));
        }
        // These checked narrow pixel slices preserve upstream f32 ink arithmetic
        // and U8/U16 quantization before the separate existing CMS pass.
        let rgb = self.render(photocraft_color::read_sample(source, sf.sample, 0))?;
        for (channel, value) in rgb.into_iter().enumerate() {
            photocraft_color::write_sample(output, to.sample, channel, value);
        }
        if sf.alpha {
            let alpha = photocraft_color::sample_byte_range(sf.sample, 1, source.len()).map_err(|_| error("ink source alpha"))?;
            let destination = photocraft_color::sample_byte_range(to.sample, 3, output.len()).map_err(|_| error("ink destination alpha"))?;
            output.get_mut(destination).ok_or_else(|| error("ink output alpha"))?.copy_from_slice(source.get(alpha).ok_or_else(|| error("ink input alpha"))?);
        }
        Ok(())
    }
    pub(super) fn surface(&self, source: &Surface, c: &Interrupt<'_>) -> Result<Surface> {
        poll(c)?;
        let sf = source.format();
        if sf.mode != ColorMode::Grayscale {
            return Ok(source.clone());
        }
        if !matches!(sf.sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
            return Err(error("wide ink surfaces are unsupported"));
        }
        source.try_validate_tile_origins_with(c).map_err(|failure| match failure {
            photocraft_raster::RasterInputError::Cancelled => EngineError::Cancelled,
            _ => error("invalid ink tile origins"),
        })?;
        let to = PixelFormat { mode: ColorMode::Rgb, ..sf };
        let mut default = [0u8; 16];
        let default = default.get_mut(..to.bytes_per_pixel()).ok_or_else(|| error("ink default length"))?;
        self.pixel(source.default_bytes(), sf, default, to)?;
        let mut output = Surface::try_with_default_bytes(to, default).map_err(|_| error("ink default allocation"))?;
        let pixels = (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).ok_or_else(|| error("tile size overflow"))?;
        for (coordinate, tile) in source.tiles() {
            poll(c)?;
            if tile.bytes().len() != pixels.checked_mul(sf.bytes_per_pixel()).ok_or_else(|| error("ink source tile size"))? {
                return Err(error("ink source tile length"));
            }
            let destination = output.try_tile_mut(*coordinate).map_err(|_| error("ink tile allocation"))?.bytes_mut();
            if destination.len() != pixels.checked_mul(to.bytes_per_pixel()).ok_or_else(|| error("ink output tile size"))? {
                return Err(error("ink output tile length"));
            }
            for (index, (src, dst)) in tile.bytes().chunks_exact(sf.bytes_per_pixel()).zip(destination.chunks_exact_mut(to.bytes_per_pixel())).enumerate() {
                if index.is_multiple_of(64) {
                    poll(c)?;
                }
                self.pixel(src, sf, dst, to)?;
            }
        }
        poll(c)?;
        Ok(output)
    }
    pub(super) fn colors(&self, layers: &mut [Layer], c: &Interrupt<'_>) -> Result<()> {
        visit_colors_mut(layers, 0, &mut 0, c, &mut |color| {
            if matches!(color.mode, ColorMode::Grayscale | ColorMode::Duotone) {
                let [r, g, b] = self.render(color.c.first().copied().ok_or_else(|| error("missing gray color"))?)?;
                *color = Color::rgba(r, g, b, color.alpha);
            }
            Ok(())
        })
    }
}

// Identical field inventory for the paid borrowed count and staged mutation.
macro_rules! color_visitors {
    ($visit:ident, $fill:ident, $paint:ident $(, $mutable:tt)?) => {
        fn $fill(value: & $($mutable)? Fill, c: &Interrupt<'_>, f: &mut dyn FnMut(& $($mutable)? Color) -> Result<()>) -> Result<()> {
            match value {
                Fill::Solid(color) => { poll(c)?; f(color)?; },
                Fill::Gradient { stops, .. } => for (_, color) in stops { poll(c)?; f(color)?; },
                Fill::Pattern { .. } => {},
            } Ok(())
        }
        fn $paint(value: & $($mutable)? FxPaint, c: &Interrupt<'_>, f: &mut dyn FnMut(& $($mutable)? Color) -> Result<()>) -> Result<()> {
            match value {
                FxPaint::Color(color) => { poll(c)?; f(color)?; },
                FxPaint::Gradient(gradient) => for (_, color) in & $($mutable)? gradient.stops { poll(c)?; f(color)?; },
                FxPaint::Pattern { .. } => {},
            } Ok(())
        }
        fn $visit(layers: & $($mutable)? [Layer], depth: usize, count: &mut usize, c: &Interrupt<'_>, f: &mut dyn FnMut(& $($mutable)? Color) -> Result<()>) -> Result<()> {
            if depth > photocraft_doc::MAX_GROUP_DEPTH { return Err(error("ink layer depth")); }
            for layer in layers {
                poll(c)?; *count = count.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| error("ink layer count"))?;
                for effect in & $($mutable)? layer.effects.items {
                    poll(c)?;
                    match effect {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => f(& $($mutable)? s.color)?,
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => $paint(& $($mutable)? g.paint,c,f)?,
                        Effect::Stroke(s) => $paint(& $($mutable)? s.paint,c,f)?,
                        Effect::ColorOverlay {color,..} => f(color)?,
                        Effect::GradientOverlay {gradient,..} => for (_,color) in & $($mutable)? gradient.stops {poll(c)?;f(color)?;},
                        Effect::Satin(s) => f(& $($mutable)? s.color)?,
                        Effect::BevelEmboss(b) => { f(& $($mutable)? b.highlight_color)?; poll(c)?; f(& $($mutable)? b.shadow_color)?; },
                        Effect::PatternOverlay {..} => {},
                    }
                }
                match & $($mutable)? layer.content {
                    LayerContent::Fill(fill) => $fill(fill,c,f)?,
                    LayerContent::Text(text) => {f(& $($mutable)? text.color)?; for run in & $($mutable)? text.runs {poll(c)?;f(& $($mutable)? run.style.color)?;}},
                    LayerContent::Shape(shape) => {if let Some(fill) = & $($mutable)? shape.fill {$fill(fill,c,f)?;} if let Some(stroke) = & $($mutable)? shape.stroke {$fill(& $($mutable)? stroke.paint,c,f)?;}},
                    LayerContent::Group(group) => $visit(& $($mutable)? group.children,depth+1,count,c,f)?,
                    _=>{},
                }
            } Ok(())
        }
    }
}
color_visitors!(visit_colors, visit_fill, visit_paint);
color_visitors!(visit_colors_mut, visit_fill_mut, visit_paint_mut, mut);

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use photocraft_doc::{DuotoneInk, TextLayer};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture(sample: SampleType) -> Document {
        let mut doc = Document::new("ordinary inks", photocraft_geom::Size::new(2, 1), ColorMode::Duotone, sample);
        doc.duotone = Some(Duotone { inks: vec![DuotoneInk::new("orange", [1.0, 0.5, 0.0])], psd_raw: None });
        doc
    }
    fn gray(value: f32, alpha: f32, mode: ColorMode) -> Color {
        let mut color = Color::rgba(value, value, value, alpha);
        color.mode = mode;
        color.c = [value, 0.0, 0.0, 0.0];
        color
    }
    #[test]
    fn ordinary_ink_defaults_and_physical_pixels_keep_same_depth_quantization_and_alpha_bits() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let doc = fixture(sample);
            let ink = Ink::selected(&doc, ColorMode::Rgb, &Interrupt::NONE).unwrap().unwrap();
            let sf = PixelFormat::new(ColorMode::Grayscale, sample, true);
            let mut bytes = vec![0u8; sf.bytes_per_pixel()];
            photocraft_color::write_sample(&mut bytes, sample, 0, 0.25);
            photocraft_color::write_sample(&mut bytes, sample, 1, 0.5);
            let mut source = Surface::try_with_default_bytes(sf, &bytes).unwrap();
            source.write_pixel(1, 0, &[0.75, 0.25]);
            let before = source.clone();
            let output = ink.surface(&source, &Interrupt::NONE).unwrap();
            assert_eq!(output.format(), PixelFormat::new(ColorMode::Rgb, sample, true));
            let verify = |input: &[u8], actual: &[u8]| {
                let mut expected = vec![0u8; output.format().bytes_per_pixel()];
                let rgb = doc.duotone.as_ref().unwrap().render(photocraft_color::read_sample(input, sample, 0));
                for (index, value) in rgb.into_iter().enumerate() {
                    photocraft_color::write_sample(&mut expected, sample, index, value);
                }
                let alpha = photocraft_color::sample_byte_range(sample, 1, input.len()).unwrap();
                let outalpha = photocraft_color::sample_byte_range(sample, 3, expected.len()).unwrap();
                expected[outalpha].copy_from_slice(&input[alpha]);
                assert_eq!(actual, expected);
            };
            verify(source.default_bytes(), output.default_bytes());
            for (coord, tile) in source.tiles() {
                let actual = output.tile(*coord).unwrap();
                for (src, dst) in tile.bytes().chunks_exact(sf.bytes_per_pixel()).zip(actual.bytes().chunks_exact(output.format().bytes_per_pixel())) {
                    verify(src, dst);
                }
            }
            assert_eq!(source, before);
        }
    }
    #[test]
    fn borrowed_metadata_inventory_matches_nested_fill_text_and_effect_ink_mutation() {
        let mut doc = fixture(SampleType::F32);
        let mut fill = Layer::new("ink fill", LayerContent::Fill(Fill::Solid(gray(0.5, 0.75, ColorMode::Duotone))));
        fill.effects.items.push(Effect::ColorOverlay {
            common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
            color: gray(0.25, 0.5, ColorMode::Grayscale),
        });
        let text = TextLayer { color: gray(0.75, 0.25, ColorMode::Grayscale), ..Default::default() };
        doc.layers = vec![Layer::group("group", vec![fill, Layer::new("text", LayerContent::Text(text))])];
        let ink = Ink::selected(&doc, ColorMode::Rgb, &Interrupt::NONE).unwrap().unwrap();
        let mut expected = Vec::new();
        visit_colors(&doc.layers, 0, &mut 0, &Interrupt::NONE, &mut |color| {
            let [r, g, b] = doc.duotone.as_ref().unwrap().render(color.c[0]);
            expected.push(Color::rgba(r, g, b, color.alpha));
            Ok(())
        })
        .unwrap();
        assert_eq!(expected.len(), 3);
        let mut layers = doc.layers.clone();
        ink.colors(&mut layers, &Interrupt::NONE).unwrap();
        let mut actual = Vec::new();
        visit_colors(&layers, 0, &mut 0, &Interrupt::NONE, &mut |color| {
            actual.push(*color);
            Ok(())
        })
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(ink.metadata_requirements(&doc, 128, &Interrupt::NONE).unwrap(), (4096, 512 + 3 * ink.per_render + 8 * ink.inventory));
        assert!(Ink::selected(&doc, ColorMode::Grayscale, &Interrupt::NONE).unwrap().is_none());
    }
    #[test]
    fn actual_intermediate_one_less_byte_or_work_refuses_without_changing_original_totals() {
        let doc = fixture(SampleType::U16);
        let ink = Ink::selected(&doc, ColorMode::Cmyk, &Interrupt::NONE).unwrap().unwrap();
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true));
        source.write_pixel(0, 0, &[0.5, 1.0]);
        let extra = ink.surface_requirements(&source, &Interrupt::NONE).unwrap();
        let limit = (11 + extra.0, 13 + extra.1);
        let mut bytes = 11;
        let mut work = 13;
        assert!(admit_extra(&mut bytes, &mut work, extra, limit.0 - 1, limit.1).is_err());
        assert_eq!((bytes, work), (11, 13));
        assert!(admit_extra(&mut bytes, &mut work, extra, limit.0, limit.1 - 1).is_err());
        assert_eq!((bytes, work), (11, 13));
        admit_extra(&mut bytes, &mut work, extra, limit.0, limit.1).unwrap();
        assert_eq!((bytes, work), limit);
        assert!(ink.metadata_requirements(&doc, u64::MAX, &Interrupt::NONE).is_err());
    }
    #[test]
    fn original_interrupt_and_late_invalid_ink_sample_preserve_actual_source_and_document() {
        let mut doc = fixture(SampleType::F32);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true));
        surface.write_pixel(0, 0, &[0.5, 1.0]);
        let original = surface.clone();
        let ink = Ink::selected(&doc, ColorMode::Rgb, &Interrupt::NONE).unwrap().unwrap();
        let calls = AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, Ordering::Relaxed) > 8;
        assert!(matches!(ink.surface(&surface, &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
        assert!(calls.load(Ordering::Relaxed) > 8);
        assert_eq!(surface, original);
        surface.write_pixel(256, 0, &[f32::NAN, 1.0]);
        doc.layers.push(Layer::new("late invalid ink", LayerContent::Raster(surface)));
        let before = doc.clone();
        assert!(
            super::super::convert_document_with_interrupt(
                &mut doc,
                photocraft_cms::Builtin::Srgb.profile(),
                photocraft_cms::Intent::RelativeColorimetric,
                true,
                &Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(doc, before);
    }
}
