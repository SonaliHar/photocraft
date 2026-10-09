//! Destructive pixel operations on layers (Image → Adjustments, flips, fills).

use photocraft_compose::{Buffer, adjust};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba, to_rgba};

/// Flatten a source into the destination's effective colour interpretation before quantizing.
/// The source document and its original imported samples/provenance are unchanged.
pub fn flattened_into(source: &Document, target: &Document) -> crate::Result<Surface> {
    use photocraft_cms::{Intent, Transform};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    let src_profile = crate::color_cmds::composite_profile(source);
    let dst_profile = crate::color_cmds::document_profile(target);
    let transform = Transform::new(&src_profile, &dst_profile, Intent::RelativeColorimetric, true)
        .map_err(|e| crate::EngineError::Other(format!("cannot place image in destination colour space: {e}")))?;
    let fmt = target.pixel_format();
    if transform.inputs() != 3 || transform.outputs() != fmt.mode.color_channels() {
        return Err(crate::EngineError::Other("cannot place image: destination profile does not match its pixel model".into()));
    }
    let float = photocraft_compose::flatten_to_surface(source, PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), None);
    let converted = crate::color_cmds::convert_surface(&float, ColorMode::Rgb, fmt, &transform);
    Ok(converted.convert(fmt))
}
/// Apply an adjustment destructively to a surface, weighted by an optional selection.
/// Applies `adj` to a surface (any colour model and depth, via straight RGBA) through the
/// selection; `mode` is the document's, for the tone transfer (e.g. Exposure in Grayscale).
pub fn adjust_surface(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>, mode: photocraft_color::ColorMode) {
    let transfer = adjust::Transfer::for_document(mode, s.format().sample);
    adjust_surface_with_transfer(s, adj, selection, mode, transfer);
}

pub fn adjust_surface_with_transfer(
    s: &mut Surface,
    adj: &Adjustment,
    selection: Option<&Surface>,
    mode: photocraft_color::ColorMode,
    transfer: adjust::Transfer,
) {
    let r = s.content_bounds();
    if r.is_empty() {
        return;
    }
    let out = adjusted_with_transfer(s, r, adj, selection, mode, transfer);
    s.write_region(r, &out);
}

/// Applies `adj` to a layer mask through the selection. Untouched mask pixels count (they read as
/// the mask's default value), and without a selection the default changes too, since a mask
/// reaches past the canvas: Invert turns a reveal-all mask into a hide-all one (#780).
pub fn adjust_mask(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>) {
    let mode = photocraft_color::ColorMode::Grayscale;
    if let Some(sel) = selection {
        let r = sel.content_bounds();
        if !r.is_empty() {
            let out = adjusted(s, r, adj, Some(sel), mode);
            s.write_region(r, &out);
            s.prune();
        }
        return;
    }
    let fmt = s.format();
    let unit = Rect::from_xywh(0, 0, 1, 1);
    let mut out = Surface::with_default(fmt, &adjusted(&Surface::with_default(fmt, &s.default_pixel()), unit, adj, None, mode));
    let r = s.tile_bounds();
    if !r.is_empty() {
        out.write_region(r, &adjusted(s, r, adj, None, mode));
    }
    out.prune();
    *s = out;
}

/// `s`'s pixels over `r` (untouched ones included) with `adj` applied through the selection, encoded.
fn adjusted(s: &Surface, r: Rect, adj: &Adjustment, selection: Option<&Surface>, mode: photocraft_color::ColorMode) -> Vec<f32> {
    adjusted_with_transfer(s, r, adj, selection, mode, adjust::Transfer::for_document(mode, s.format().sample))
}

fn adjusted_with_transfer(
    s: &Surface,
    r: Rect,
    adj: &Adjustment,
    selection: Option<&Surface>,
    _mode: photocraft_color::ColorMode,
    transfer: adjust::Transfer,
) -> Vec<f32> {
    let fmt = s.format();
    let n = fmt.channels();
    let raw = s.read_region(r);
    let mut buf = Buffer { rect: r, px: raw.chunks_exact(n).map(|p| to_rgba(&fmt, p)).collect() };
    let orig = buf.clone();
    // 32-bit documents get the float behaviour of an adjustment layer there (Levels doesn't clip);
    // integer depths keep the unrounded, clipped curves.
    let depth = (fmt.sample == photocraft_color::SampleType::F32).then_some(fmt.sample);
    adjust::apply_depth(adj, &mut buf, transfer, depth);
    let w = r.width() as usize;
    let mut out = Vec::with_capacity(raw.len());
    for (i, (a, o)) in buf.px.iter().zip(&orig.px).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(r.x0 + (i % w) as i32, r.y0 + (i / w) as i32, 0));
        let mixed: [f32; 4] = std::array::from_fn(|c| o[c] + (a[c] - o[c]) * k);
        let mut enc = [0.0f32; 8];
        let m = photocraft_raster::from_rgba_into(&fmt, mixed, &mut enc);
        out.extend_from_slice(&enc[..m]);
    }
    out
}

/// Fill (respecting selection coverage) with a straight RGBA colour.
pub fn fill_surface(s: &mut Surface, area: Rect, color: [f32; 4], selection: Option<&Surface>, lock_transparency: bool) {
    let fmt = s.format();
    let n = fmt.channels();
    let mut region = s.read_region(area);
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let x = area.x0 + (i % w) as i32;
        let y = area.y0 + (i / w) as i32;
        let k = selection.map_or(1.0, |sel| sel.sample_channel(x, y, 0));
        if k <= 0.0 {
            continue;
        }
        let d = to_rgba(&fmt, px);
        let sa = color[3] * k;
        let oa = sa + d[3] * (1.0 - sa);
        let mut o = [0.0f32; 4];
        if oa > 0.0 {
            for c in 0..3 {
                o[c] = (color[c] * sa + d[c] * d[3] * (1.0 - sa)) / oa;
            }
        }
        o[3] = if lock_transparency { d[3] } else { oa };
        let mut enc = [0.0f32; 8];
        photocraft_raster::from_rgba_into(&fmt, o, &mut enc);
        px.copy_from_slice(&enc[..n]);
    }
    s.write_region(area, &region);
}

/// Clear pixels (make transparent) within the selection.
pub fn clear_surface(s: &mut Surface, area: Rect, selection: Option<&Surface>) {
    let fmt = s.format();
    let n = fmt.channels();
    if !fmt.alpha {
        return;
    }
    let mut region = s.read_region(area);
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(area.x0 + (i % w) as i32, area.y0 + (i / w) as i32, 0));
        px[n - 1] *= 1.0 - k;
        // Fully cleared pixels become the empty pixel, so bounds and tile pruning see them as gone.
        if px[n - 1] <= 0.0 {
            px.fill(0.0);
        }
    }
    s.write_region(area, &region);
    s.prune();
}

/// Remap a surface through a pixel-coordinate mapping (used for flips/rotations).
pub fn remap_surface(s: &Surface, map: impl Fn(i32, i32) -> (i32, i32)) -> Surface {
    let r = s.content_bounds();
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let n = s.channels();
    let raw = s.read_region(r);
    let w = r.width() as usize;
    for (i, px) in raw.chunks_exact(n).enumerate() {
        let (x, y) = (r.x0 + (i % w) as i32, r.y0 + (i / w) as i32);
        let (nx, ny) = map(x, y);
        out.write_pixel(nx, ny, px);
    }
    out
}

/// Merge `upper` onto `lower` producing a raster layer (Layer → Merge Down).
pub fn merge_down(doc: &Document, lower: &Layer, upper: &Layer) -> Layer {
    let doc_bounds = doc.bounds();
    let format = doc.pixel_format();
    let area = [lower, upper].iter().map(|l| l.surface().map(|s| s.content_bounds()).unwrap_or(doc_bounds)).fold(Rect::EMPTY, |a, b| a.union(&b));
    let stack = vec![lower.clone(), upper.clone()];
    let mut tmp = Document::new("merge", doc_bounds.size(), format.mode, format.sample);
    tmp.layers = stack;
    tmp.sample_encoding = doc.sample_encoding;
    tmp.icc_profile = doc.icc_profile.clone();
    tmp.source_color_context = doc.source_color_context.clone();
    let buf = photocraft_compose::render(&tmp, area);
    let mut s = Surface::new(format);
    let data: Vec<f32> = buf.px.iter().flat_map(|p| from_rgba(&format, *p)).collect();
    if !area.is_empty() {
        s.write_region(area, &data);
        s.prune();
    }
    let mut merged = Layer::new(lower.name.clone(), LayerContent::Raster(s));
    merged.id = lower.id;
    merged.blend = lower.blend;
    merged.opacity = 1.0;
    merged
}

#[cfg(test)]
mod imported_flatten_tests {
    use super::*;
    use photocraft_cms::{Builtin, Intent, Transform};
    use photocraft_doc::{ColorMode, ImportCompositing, SampleEncoding, SampleType, Size};
    #[test]
    fn placed_imports_convert_effective_profile_before_destination_quantization() {
        for (depth, encoding, builtin) in [
            (SampleType::U16, SampleEncoding::Linear, Builtin::LinearSrgb),
            (SampleType::F32, SampleEncoding::NonlinearProfile, Builtin::Srgb),
            (SampleType::U8, SampleEncoding::NonlinearProfile, Builtin::DisplayP3),
        ] {
            let mut source = Document::new("source", Size::new(1, 1), ColorMode::Rgb, depth);
            source.sample_encoding = Some(encoding);
            source.icc_profile = Some(builtin.profile().to_bytes());
            let mut layer = Layer::raster("samples", source.pixel_format());
            layer.surface_mut().unwrap().write_pixel(0, 0, &[0.25, 0.5, 0.7, 0.6]);
            source.layers.push(layer);
            let before = source.clone();
            let raw = source.layers[0].surface().unwrap().pixel(0, 0);
            for target_builtin in [Builtin::Srgb, Builtin::LinearSrgb] {
                let mut target = Document::new("target", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
                target.icc_profile = Some(target_builtin.profile().to_bytes());
                target.sample_encoding = Some(if target_builtin == Builtin::LinearSrgb { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile });
                let output = flattened_into(&source, &target).unwrap();
                let transform = Transform::new(builtin.profile(), target_builtin.profile(), Intent::RelativeColorimetric, true).unwrap();
                let mut expected = [0.0; 4];
                transform.convert_f32(&raw, 4, &mut expected, 4, true);
                let actual = output.pixel(0, 0);
                for (a, e) in actual.iter().zip(expected) {
                    assert!((a - e).abs() <= 2.0 / 65535.0, "{depth:?}/{encoding:?}/{target_builtin:?}: {actual:?} vs {expected:?}");
                }
                assert_eq!(source, before);
                assert_eq!(output.format(), target.pixel_format());
            }
        }
    }
    #[test]
    fn merging_imported_layers_keeps_linear_integer_interpretation() {
        let mut doc = Document::new("linear", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
        doc.sample_encoding = Some(SampleEncoding::Linear);
        doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
        let mut lower = Layer::raster("lower", doc.pixel_format());
        lower.surface_mut().unwrap().write_pixel(0, 0, &[0.0, 0.0, 0.0, 1.0]);
        let mut upper = Layer::raster("upper", doc.pixel_format());
        upper.surface_mut().unwrap().write_pixel(0, 0, &[1.0, 1.0, 1.0, 0.5]);
        for layer in [&mut lower, &mut upper] {
            layer.import_compositing =
                Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
        }
        let merged = merge_down(&doc, &lower, &upper);
        assert!((merged.surface().unwrap().pixel(0, 0)[0] - 0.5).abs() < 2.0 / 65535.0);
        assert!(merged.import_compositing.is_none());
    }
}
