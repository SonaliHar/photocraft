//! Checked physical axes for ordinary type layout; no source-cache or font authority.
use photocraft_doc::{PhysicalResolution, TextLayer, text::TextShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalLayoutError {
    InvalidResolution,
    Geometry,
    InputLimit,
}
impl std::fmt::Display for PhysicalLayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidResolution => "physical text layout: invalid resolution",
            Self::Geometry => "physical text layout: unrepresentable geometry",
            Self::InputLimit => "physical text layout: input limit",
        })
    }
}
impl std::error::Error for PhysicalLayoutError {}

pub(crate) struct Axes {
    pub dpi: f32,
    pub scale: [f64; 2],
}
impl Axes {
    pub fn checked(layer: &TextLayer, resolution: &PhysicalResolution) -> Result<Self, PhysicalLayoutError> {
        resolution.validate().map_err(|_| PhysicalLayoutError::InvalidResolution)?;
        let dpi = resolution.y_dpi as f32;
        let x = resolution.x_dpi as f32;
        if !dpi.is_finite() || dpi <= 0.0 || !x.is_finite() || x <= 0.0 || dpi / 72.0 <= 0.0 {
            return Err(PhysicalLayoutError::InvalidResolution);
        }
        let scale = [resolution.x_dpi / resolution.y_dpi, 1.0];
        if !scale[0].is_finite() || scale[0] <= 0.0 {
            return Err(PhysicalLayoutError::Geometry);
        }
        if layer.text.len() > 1 << 20 || layer.runs.len() > 100_000 || layer.paragraphs.len() > 100_000 {
            return Err(PhysicalLayoutError::InputLimit);
        }
        let k = f64::from(dpi) / 72.0;
        for style in layer.runs.iter().map(|run| &run.style) {
            for v in [style.size_pt, style.baseline_shift_pt, style.tracking, style.kern, style.horizontal_scale, style.vertical_scale] {
                bounded(f64::from(v) * k)?;
            }
            if let Some(leading) = style.leading_pt {
                bounded(f64::from(leading) * k)?;
            }
        }
        for factor in scale {
            bounded(f64::from(layer.size_pt) * k * factor)?;
        }
        for style in layer.runs.iter().map(|run| &run.style) {
            let size = f64::from(style.size_pt) * k;
            for factor in scale {
                bounded(size * factor)?;
                bounded(f64::from(style.baseline_shift_pt) * k * factor)?;
            }
            bounded(size * f64::from(style.horizontal_scale.max(1.0)) * scale[0])?;
            bounded(size * f64::from(style.vertical_scale.max(1.0)) * scale[1])?;
        }
        for paragraph in &layer.paragraphs {
            for value in [
                paragraph.style.first_line_indent_pt,
                paragraph.style.start_indent_pt,
                paragraph.style.end_indent_pt,
                paragraph.style.space_before_pt,
                paragraph.style.space_after_pt,
                paragraph.style.auto_leading,
            ] {
                for factor in scale {
                    bounded(f64::from(value) * k * factor)?;
                }
            }
        }
        if let TextShape::Box { x, y, width, height } = layer.shape {
            for (value, factor) in [(x, scale[0]), (y, scale[1]), (width, scale[0]), (height, scale[1])] {
                bounded(f64::from(value) / factor)?;
            }
        }
        Ok(Self { dpi, scale })
    }
}
pub(crate) fn bounded(value: f64) -> Result<f32, PhysicalLayoutError> {
    if !value.is_finite() || value.abs() > f64::from(1 << 29) {
        return Err(PhysicalLayoutError::Geometry);
    }
    Ok(value as f32)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::TextEngine;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{
        PhysicalUnit,
        text::{Orientation, TextShape},
    };
    use photocraft_geom::{Affine, Rect};

    fn resolution(x: f64, y: f64) -> PhysicalResolution {
        PhysicalResolution { x_dpi: x, y_dpi: y, unit: PhysicalUnit::Inch }
    }
    fn text() -> TextLayer {
        TextLayer { text: "Craft".into(), font_family: "Inter".into(), size_pt: 12.0, ..Default::default() }
    }
    #[test]
    fn square_axes_keep_scalar_geometry_and_raster_exact_at_three_depths() {
        let mut engine = TextEngine::new();
        let t = text();
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Rgb, depth, true);
            let (old, a) = engine.render(&t, 144.0, format);
            let (new, b) = engine.render_with_resolution(&t, &resolution(144.0, 144.0), format).unwrap();
            assert_eq!(old.glyphs, new.glyphs);
            assert_eq!(old.lines, new.lines);
            assert_eq!(old.clusters, new.clusters);
            assert_eq!(old.decorations, new.decorations);
            assert_eq!(old.styles, new.styles);
            assert_eq!(old.bounds(), new.bounds());
            assert_eq!(new.physical_scale, None);
            assert_eq!(a.rect, b.rect);
            assert_eq!(a.surface, b.surface);
        }
    }
    #[test]
    fn unequal_axes_scale_outline_geometry_before_raster_and_keep_caret_roundtrips() {
        let mut engine = TextEngine::new();
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let t = TextLayer { orientation, ..text() };
            let base = engine.layout(&t, 288.0);
            let axes = engine.layout_with_resolution(&t, &resolution(144.0, 288.0)).unwrap();
            let scaled = Affine { m: [0.5, 0.0, 0.0, 1.0, 0.0, 0.0] };
            assert_eq!(crate::render::outlines(&base, &scaled, None), crate::render::outlines(&axes, &Affine::IDENTITY, None));
            assert_eq!(axes.px_per_pt_axes(), [2.0, 4.0]);
            for cluster in &axes.clusters {
                let caret = axes.caret_segment(cluster.range.start);
                let midpoint = ((caret[0].0 + caret[1].0) * 0.5, (caret[0].1 + caret[1].1) * 0.5);
                assert_eq!(axes.hit_test(midpoint.0, midpoint.1), cluster.range.start);
            }
        }
    }
    #[test]
    fn paragraph_pixel_box_is_inverted_before_line_breaking_not_afterwards() {
        let mut engine = TextEngine::new();
        let physical = TextLayer { text: "Craft Craft Craft Craft".into(), shape: TextShape::Box { x: 0.0, y: 0.0, width: 90.0, height: 240.0 }, ..text() };
        let scalar = TextLayer { shape: TextShape::Box { x: 0.0, y: 0.0, width: 180.0, height: 240.0 }, ..physical.clone() };
        let a = engine.layout_with_resolution(&physical, &resolution(144.0, 288.0)).unwrap();
        let b = engine.layout(&scalar, 288.0);
        assert_eq!(a.lines.iter().map(|l| l.range.clone()).collect::<Vec<_>>(), b.lines.iter().map(|l| l.range.clone()).collect::<Vec<_>>());
        assert!(a.lines.len() > 1);
        assert_eq!(physical.shape, TextShape::Box { x: 0.0, y: 0.0, width: 90.0, height: 240.0 });
    }
    #[test]
    fn resolution_refusal_preserves_a_painted_cache_and_source_model() {
        let mut engine = TextEngine::new();
        let cache = photocraft_raster::Surface::try_from_interleaved(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[7, 31, 89, 255]).unwrap();
        let mut t = TextLayer { cache: Some(cache), ..text() };
        let before = t.clone();
        for (x, y) in [(f64::NAN, 144.0), (144.0, f64::INFINITY), (0.0, 144.0), (f64::MAX, 144.0), (144.0, 0.0)] {
            assert!(engine.render_layer_with_resolution(&mut t, &resolution(x, y), PixelFormat::RGBA8).is_err());
            assert_eq!(t, before);
        }
    }
    #[test]
    fn psd_axes_roundtrip_point_styles_box_and_affine_without_painting() {
        let t = TextLayer {
            shape: TextShape::Box { x: 4.0, y: 6.0, width: 90.0, height: 240.0 },
            transform: Affine { m: [1.0, 0.25, -0.125, 1.0, 29.0, 37.0] },
            ..text()
        };
        let physical = resolution(144.0, 288.0);
        let bytes = crate::psd::build_tysh_with_resolution(&t, &physical, None).unwrap();
        let raw = crate::psd::parse_tysh(&bytes).unwrap();
        assert_eq!(raw.transform.m, [0.5, 0.125, -0.125, 1.0, 29.0, 37.0]);
        let restored = crate::psd::text_layer_from_tysh_with_resolution(&bytes, &physical).unwrap().unwrap();
        assert_eq!(restored.size_pt, t.size_pt);
        assert_eq!(restored.shape, t.shape);
        assert_eq!(restored.transform, t.transform);
        assert!(restored.cache.is_none());
        let square = resolution(144.0, 144.0);
        assert_eq!(crate::psd::build_tysh_with_resolution(&t, &square, None).unwrap(), crate::psd::build_tysh(&t, 144.0, None));
    }
}
