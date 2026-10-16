//! Destructive pixel operations on layers (Image → Adjustments, flips, fills).

use photocraft_compose::{Buffer, adjust};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Surface, to_rgba, from_rgba};

/// Composite `layers` inside their visible composite bounds (effects included) and write the result directly into a
/// tiled surface. Large merges used to hold both a full-canvas RGBA buffer and a second converted
/// float buffer at once; banded rendering keeps only one compositor band plus the destination.
pub fn composite_layers(solo: &Document, format: photocraft_color::PixelFormat, background: Option<[f32; 3]>) -> crate::Result<Surface> {
    let canvas = solo.bounds();
    let area = solo
        .layers
        .iter()
        .filter(|layer| layer.visible)
        // Composite bounds include layer effects (a drop shadow reaches past the pixels); `None`
        // means the layer may draw anywhere.
        .map(|layer| photocraft_compose::composite_bounds(layer, canvas).unwrap_or(canvas))
        .filter(|bounds| !bounds.is_empty())
        .reduce(|a, b| a.union(&b))
        .unwrap_or(Rect::EMPTY)
        .intersect(&canvas);
    if area.is_empty() {
        return Ok(Surface::new(format));
    }
    // Also keeps a plain CMYK stack in its ink values (#3778).
    Ok(photocraft_compose::composite_to_surface(solo, area, format, background)?)
}

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
    let float = photocraft_compose::flatten_to_surface(source, PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), None)?;
    let converted = crate::color_cmds::convert_surface(&float, ColorMode::Rgb, fmt, &transform)?;
    converted.try_convert(fmt).map_err(|e| crate::EngineError::Other(e.to_string()))
}
/// Apply an adjustment destructively to a surface, weighted by an optional selection.
/// Applies `adj` to a surface (any colour model and depth, via straight RGBA) through the
/// selection; `mode` is the document's, for the tone transfer (e.g. Exposure in Grayscale).
pub fn adjust_surface(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>, mode: photocraft_color::ColorMode) -> crate::Result<()> {
    let transfer = adjust::Transfer::for_document(mode, s.format().sample);
    adjust_surface_with_transfer(s, adj, selection, mode, transfer)
}

pub fn adjust_surface_with_transfer(
    s: &mut Surface,
    adj: &Adjustment,
    selection: Option<&Surface>,
    mode: photocraft_color::ColorMode,
    transfer: adjust::Transfer,
) -> crate::Result<()> {
    if matches!(s.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64) {
        if !matches!(adj, Adjustment::Invert) {
            return Err(crate::EngineError::Other(
                "this adjustment does not yet support authoritative U32/F64 samples; explicitly convert sample depth first".into(),
            ));
        }
        let area = s.content_bounds();
        let converted = photocraft_algo::wide::try_invert(s, area, selection, &photocraft_raster::Interrupt::NONE)
            .map_err(|e| crate::EngineError::Other(format!("wide inversion: {e}")))?;
        *s = converted;
        return Ok(());
    }
    let r = s.content_bounds();
    if r.is_empty() {
        return Ok(());
    }
    let out = adjusted_with_transfer(s, r, adj, selection, mode, transfer);
    s.write_region(r, &out);
    Ok(())
}

#[cfg(test)]
#[path = "flatten_cmyk_tests.rs"]
mod flatten_cmyk_tests;
#[cfg(test)]
#[path = "pixels_tests.rs"]
mod tests;

/// Applies `adj` to a layer mask through the selection. Untouched mask pixels count (they read as
/// the mask's default value), and without a selection the default changes too, since a mask
/// reaches past the canvas: Invert turns a reveal-all mask into a hide-all one (#780).
pub fn adjust_mask(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>) -> crate::Result<()> {
    if matches!(s.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64) {
        if !matches!(adj, Adjustment::Invert) {
            return Err(crate::EngineError::Other("this mask adjustment does not yet support authoritative U32/F64 samples".into()));
        }
        let control = photocraft_raster::Interrupt::NONE;
        let area = selection.map_or_else(|| s.tile_bounds(), Surface::content_bounds);
        let edited = photocraft_algo::wide::try_invert(s, area, selection, &control).map_err(|e| crate::EngineError::Other(e.to_string()))?;
        if selection.is_some() {
            *s = edited;
        } else {
            let unit = Rect::new(0, 0, 1, 1);
            let defaults = Surface::try_with_default_bytes(s.format(), s.default_bytes()).map_err(|e| crate::EngineError::Other(e.to_string()))?;
            let defaults = photocraft_algo::wide::try_invert(&defaults, unit, None, &control).map_err(|e| crate::EngineError::Other(e.to_string()))?;
            let raw = defaults.try_read_interleaved(unit).map_err(|e| crate::EngineError::Other(e.to_string()))?;
            let mut output = Surface::try_with_default_bytes(s.format(), &raw).map_err(|e| crate::EngineError::Other(e.to_string()))?;
            output.put_tiles(edited.tiles().map(|(coordinate, tile)| (*coordinate, tile.clone())));
            *s = output;
        }
        return Ok(());
    }
    let mode = photocraft_color::ColorMode::Grayscale;
    if let Some(sel) = selection {
        let r = sel.content_bounds();
        if !r.is_empty() {
            let out = adjusted(s, r, adj, Some(sel), mode);
            s.write_region(r, &out);
            s.prune();
        }
        return Ok(());
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
    Ok(())
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
    mode: photocraft_color::ColorMode,
    transfer: adjust::Transfer,
) -> Vec<f32> {
    let fmt = s.format();
    if fmt.mode == mode && matches!(mode, photocraft_color::ColorMode::Lab | photocraft_color::ColorMode::Cmyk) && adjust::stored_channel_adjustment(adj) {
        return adjusted_channels(s, r, adj, selection);
    }
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
/// Works tile by tile in parallel on the encoded pixels; a fully selected tile of an opaque fill
/// that may set alpha becomes one shared solid tile.
pub(crate) fn try_fill_surface(s: &mut Surface, area: Rect, color: [f32; 4], selection: Option<&Surface>, lock_transparency: bool) -> crate::Result<()> {
    if matches!(s.format().sample,photocraft_doc::SampleType::U32|photocraft_doc::SampleType::F64){return Err(crate::EngineError::Other("ordinary fill does not support authoritative U32/F64 arithmetic".into()));}
    let fmt = s.format();
    // Coverage 1 of an opaque colour replaces the colour (and the alpha, unless it is kept).
    let opaque = color[3] == 1.0 && (!lock_transparency || !fmt.alpha) && selection.is_none_or(|m| m.format().sample != photocraft_color::SampleType::F32);
    let solid = opaque.then(|| {
        let mut enc = [0.0f32; 8];
        let n = photocraft_raster::from_rgba_into(&fmt, [color[0], color[1], color[2], 1.0], &mut enc);
        s.solid_tile(enc.get(..n).unwrap_or(&[]))
    });
    let whole = |full: bool| solid.as_ref().filter(|_| full).map(|t| TileEdit::Put(t.clone()));
    edit_tiles(s, area, selection, "filling pixels", whole, |k, px| {
        if k <= 0.0 {
            return false;
        }
        fill_pixel(&fmt, px, color, k, lock_transparency);
        true
    })
}

/// One pixel of [`try_fill_surface`]: `color` over `px` (encoded) at coverage `k`.
fn fill_pixel(fmt: &photocraft_color::PixelFormat, px: &mut [f32], color: [f32; 4], k: f32, lock_transparency: bool) {
    let d = to_rgba(fmt, px);
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
    let m = photocraft_raster::from_rgba_into(fmt, o, &mut enc);
    let n = px.len().min(m);
    if let (Some(dst), Some(src)) = (px.get_mut(..n), enc.get(..n)) {
        dst.copy_from_slice(src);
    }
}

/// Clear pixels (make transparent) within the selection.
/// Works tile by tile in parallel on the encoded pixels; a fully selected tile is dropped (it
/// would only hold empty pixels), and tiles the selection leaves alone stay shared with history.
pub(crate) fn try_clear_surface_ordinary(s: &mut Surface, area: Rect, selection: Option<&Surface>) -> crate::Result<()> {
    let fmt = s.format();
    let n = fmt.channels();
    if !fmt.alpha {
        return Ok(());
    }
    // Cleared pixels become all zeros; when that is also the default pixel, a cleared tile is no tile.
    let empty_default = s.default_bytes().iter().all(|&b| b == 0);
    let whole = |full: bool| (full && empty_default).then_some(TileEdit::Drop);
    edit_tiles(s, area, selection, "erasing pixels", whole, |k, px| {
        let Some(a) = px.get_mut(n - 1) else { return false };
        *a *= 1.0 - k;
        // Fully cleared pixels become the empty pixel, so bounds and tile pruning see them as gone.
        if *a <= 0.0 {
            px.fill(0.0);
        }
        true
    })?;
    s.prune();
    Ok(())
}

/// A whole-tile shortcut of [`edit_tiles`].
enum TileEdit {
    /// Replace the tile with this (shared) one.
    Put(std::sync::Arc<photocraft_raster::Tile>),
    /// Remove the tile (it reads as the default pixel again).
    Drop,
}

/// Edit `s` over `area` tile by tile in parallel. `whole` may handle a tile lying wholly inside
/// `area` and fully selected (`true`) in one go; otherwise `pixel` edits each pixel of the area
/// with its selection coverage (decoded channel values in and out; `false` = left untouched).
/// A tile is copied only when one of its pixels actually changes (tiles are shared with undo
/// states), and pixels are decoded and re-encoded exactly as a whole-region float round trip
/// would, so the result is the same as one pass over `area`.
fn edit_tiles(
    s: &mut Surface,
    area: Rect,
    selection: Option<&Surface>,
    purpose: &str,
    whole: impl Fn(bool) -> Option<TileEdit> + Sync,
    pixel: impl Fn(f32, &mut [f32]) -> bool + Sync,
) -> crate::Result<()> {
    use photocraft_raster::Tile;
    use rayon::prelude::*;
    use std::sync::Arc;
    let fmt = s.format();
    let (n, bpp) = (fmt.channels(), fmt.bytes_per_pixel());
    let oom = |e| crate::EngineError::Other(format!("not enough memory while {purpose} ({e}); the document was not changed"));
    let mut coords = crate::allocation::capacity(area.tiles().count(), purpose)?;
    coords.extend(area.tiles());
    // Untouched pixels read as the default where the surface has no tile.
    let blank = if coords.iter().any(|&tc| s.tile(tc).is_none()) { Some(s.try_default_tile().map_err(oom)?) } else { None };
    let surf = &*s;
    let edits = coords
        .into_par_iter()
        .map(|tc| -> crate::Result<Option<(photocraft_geom::TileCoord, Option<Arc<Tile>>)>> {
            let tr = tc.rect().intersect(&area);
            let mask = selection.map(|m| crate::fill_cmds::Mask::of(m, tc));
            let full = tr == tc.rect() && mask.as_ref().is_none_or(|m| m.full(tr));
            let current = surf.tile(tc);
            match whole(full) {
                Some(TileEdit::Put(t)) => return Ok(Some((tc, Some(t)))),
                Some(TileEdit::Drop) => return Ok(current.map(|_| (tc, None))),
                None => {}
            }
            let Some(src) = current.map(|t| &**t).or(blank.as_ref()) else { return Ok(None) };
            let mut out: Option<Tile> = None;
            let (mut px, mut enc) = ([0.0f32; 8], [0u8; 32]);
            let (Some(px), Some(enc)) = (px.get_mut(..n), enc.get_mut(..bpp)) else { return Ok(None) };
            for y in tr.y0..tr.y1 {
                let row = (y - tc.ty * TILE_SIZE) as usize * TILE_SIZE as usize;
                for x in tr.x0..tr.x1 {
                    let i = (row + (x - tc.tx * TILE_SIZE) as usize) * bpp;
                    let Some(old) = src.bytes().get(i..i + bpp) else { continue };
                    for (c, v) in px.iter_mut().enumerate() {
                        *v = photocraft_color::read_sample(old, fmt.sample, c);
                    }
                    if !pixel(mask.as_ref().map_or(1.0, |m| m.at(x, y)), px) {
                        continue;
                    }
                    photocraft_raster::encode_pixel(&fmt, px, enc);
                    if enc == old {
                        continue;
                    }
                    if out.is_none() {
                        out = Some(src.try_clone().map_err(oom)?);
                    }
                    if let Some(dst) = out.as_mut().and_then(|t| t.bytes_mut().get_mut(i..i + bpp)) {
                        dst.copy_from_slice(enc);
                    }
                }
            }
            Ok(out.map(|t| (tc, Some(Arc::new(t)))))
        })
        .collect::<crate::Result<Vec<_>>>()?;
    for (tc, tile) in edits.into_iter().flatten() {
        match tile {
            Some(t) => s.put_tiles([(tc, t)]),
            None => drop(s.take_tiles(tc.rect())),
        }
    }
    Ok(())
}

fn clear_error(error: impl std::fmt::Display) -> crate::EngineError {
    crate::EngineError::Other(format!("wide clear: {error}"))
}

fn native_clear_weight(selection: Option<&Surface>, x: i32, y: i32) -> crate::Result<f64> {
    let Some(selection) = selection else { return Ok(1.0) };
    let mut samples = [0.0_f64; 5];
    let samples = samples.get_mut(..selection.channels()).ok_or_else(|| clear_error("invalid selection channels"))?;
    selection.try_read_pixel_f64(x, y, samples).map_err(clear_error)?;
    let weight = samples.first().copied().ok_or_else(|| clear_error("invalid selection channels"))?;
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) { return Err(clear_error("invalid selection coverage")); }
    Ok(weight)
}

/// Validate wide selection semantics and conservative simultaneous raster storage
/// before cloning private staging or entering a caller's document edit.
pub fn preflight_clear_surface(s: &Surface, area: Rect, selection: Option<&Surface>, control: &photocraft_raster::Interrupt<'_>) -> crate::Result<()> {
    control.check().map_err(|_| crate::EngineError::Cancelled)?;
    if !matches!(s.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64) || !s.format().alpha {
        return Ok(());
    }
    photocraft_raster::checked_region_byte_len(area, s.format()).map_err(clear_error)?;
    s.try_validate_tile_origins_with(control).map_err(clear_error)?;
    if let Some(selection) = selection {
        selection.try_validate_tile_origins_with(control).map_err(clear_error)?;
    }
    let tile_pixels = (photocraft_geom::TILE_SIZE as u64).checked_mul(photocraft_geom::TILE_SIZE as u64).ok_or_else(|| clear_error("allocation overflow"))?;
    let tile_bytes = tile_pixels.checked_mul(s.format().bytes_per_pixel() as u64).ok_or_else(|| clear_error("allocation overflow"))?;
    let replacement = (area.tiles().count() as u64)
        .checked_mul(tile_bytes.checked_add(512).ok_or_else(|| clear_error("allocation overflow"))?)
        .ok_or_else(|| clear_error("allocation overflow"))?;
    // Old source plus every replacement, source/staged map clones and the largest
    // native read/write scratch tile. BTreeMap/Arc nodes are bounded/prepaid;
    // their std allocation is not claimed to have a stable fallible allocator API.
    let peak = s
        .resident_accounted_bytes()
        .map_err(clear_error)?
        .checked_add(selection.map_or(Ok(0), Surface::resident_accounted_bytes).map_err(clear_error)?)
        .and_then(|bytes| bytes.checked_add(replacement))
        .and_then(|bytes| bytes.checked_add((s.tile_count() as u64).checked_mul(512)?))
        .and_then(|bytes| bytes.checked_add(tile_bytes.checked_mul(3)?))
        .and_then(|bytes| bytes.checked_add(s.default_bytes().len() as u64))
        .ok_or_else(|| clear_error("allocation overflow"))?;
    if peak > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(clear_error("allocation limit exceeded"));
    }
    if selection.is_some() {
        for y in area.y0..area.y1 {
            control.check().map_err(|_| crate::EngineError::Cancelled)?;
            for x in area.x0..area.x1 {
                if (i64::from(x) - i64::from(area.x0)) % 256 == 0 {
                    control.check().map_err(|_| crate::EngineError::Cancelled)?;
                }
                let weight = native_clear_weight(selection, x, y)?;
                if weight > 0.0 && weight < 1.0 {
                    let mut raw = [0u8; 40];
                    let pixel = raw.get_mut(..s.format().bytes_per_pixel()).ok_or_else(|| clear_error("invalid pixel length"))?;
                    s.try_read_pixel_bytes(x, y, pixel).map_err(clear_error)?;
                    let alpha = photocraft_color::try_read_sample_f64(pixel, s.format().sample, s.format().mode.color_channels()).map_err(clear_error)?;
                    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) { return Err(clear_error("invalid fractional clear alpha")); }
                }
            }
        }
    }
    control.check().map_err(|_| crate::EngineError::Cancelled)
}

/// Checked native wide clear; every unselected encoded bit stays untouched.
/// Full clear retains legacy semantics: the whole pixel becomes canonical zero.
pub fn try_clear_surface(s: &mut Surface, area: Rect, selection: Option<&Surface>, control: &photocraft_raster::Interrupt<'_>) -> crate::Result<()> {
    control.check().map_err(|_| crate::EngineError::Cancelled)?;
    if !matches!(s.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64) {
        try_clear_surface_ordinary(s, area, selection)?;
        control.check().map_err(|_|crate::EngineError::Cancelled)?;
        return Ok(());
    }
    preflight_clear_surface(s, area, selection, control)?;
    if !s.format().alpha || area.is_empty() {
        return Ok(());
    }
    let mut staged = s.clone();
    let bpp = s.format().bytes_per_pixel();
    for tile in area.tiles() {
        let rect = tile.rect().intersect(&area);
        let mut bytes = s.try_read_interleaved_with(rect, control).map_err(clear_error)?;
        let width = usize::try_from(rect.width()).map_err(clear_error)?;
        if width == 0 {
            return Err(clear_error("invalid clear width"));
        }
        let mut changed = false;
        for (index, pixel) in bytes.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) {
                control.check().map_err(|_| crate::EngineError::Cancelled)?;
            }
            let x = rect.x0.checked_add(i32::try_from(index % width).map_err(clear_error)?).ok_or_else(|| clear_error("coordinate overflow"))?;
            let y = rect.y0.checked_add(i32::try_from(index / width).map_err(clear_error)?).ok_or_else(|| clear_error("coordinate overflow"))?;
            let weight = native_clear_weight(selection, x, y)?;
            if weight == 1.0 {
                pixel.fill(0);
                changed = true;
            } else if weight != 0.0 {
                changed |= photocraft_paint::wide_erase::try_clear_native_pixel(pixel, s.format(), weight).map_err(clear_error)?;
            }
        }
        if changed {
            staged.try_write_interleaved_with(rect, &bytes, control).map_err(clear_error)?;
        }
    }
    control.check().map_err(|_| crate::EngineError::Cancelled)?;
    // No global prune: it can remove untouched explicit storage outside the edit.
    *s = staged;
    Ok(())
}

/// Merge `upper` onto `lower` producing a raster layer (Layer → Merge Down).
pub fn merge_down(doc: &Document, lower: &Layer, upper: &Layer) -> crate::Result<Layer> {
    let doc_bounds = doc.bounds();
    let format = doc.pixel_format();
    let area = [lower, upper].iter().map(|l| l.surface().map(|s| s.content_bounds()).unwrap_or(doc_bounds)).fold(Rect::EMPTY, |a, b| a.union(&b));
    let stack = vec![lower.clone(), upper.clone()];
    let mut tmp = Document::new("merge", doc.size, format.mode, format.sample);
    // The pair is read (and a CMYK one separated again) through the document's own profile.
    tmp.icc_profile = doc.icc_profile.clone();
    tmp.layers = stack;
    tmp.sample_encoding = doc.sample_encoding;
    tmp.icc_profile = doc.icc_profile.clone();
    tmp.source_color_context = doc.source_color_context.clone();
    let buf = photocraft_compose::render(&tmp, area)?;
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
    Ok(merged)
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
        let merged = merge_down(&doc, &lower, &upper).unwrap();
        assert!((merged.surface().unwrap().pixel(0, 0)[0] - 0.5).abs() < 2.0 / 65535.0);
        assert!(merged.import_compositing.is_none());
    }
}

fn adjusted_channels(s: &Surface, r: Rect, adj: &Adjustment, selection: Option<&Surface>) -> Vec<f32> {
    let fmt = s.format();
    let (n, colors) = (fmt.channels(), fmt.mode.color_channels());
    let mut raw = s.read_region(r);
    let w = (r.width() as usize).max(1);
    let mut coverage = vec![1.0f32; w];
    for (j, row) in raw.chunks_mut(w * n).enumerate() {
        if let Some(sel) = selection {
            sel.sample_row_strided(r.y0.saturating_add(j as i32), r.x0, 1, 0, &mut coverage);
        }
        for (px, &k) in row.chunks_exact_mut(n).zip(&coverage) {
            if k <= 0.0 {
                continue;
            }
            let p = to_rgba(&fmt, px);
            let gray = photocraft_color::convert::rgb_to_gray([p[0], p[1], p[2]]);
            let Some(ch) = px.get_mut(..colors) else { continue };
            let mut out = [0.0f32; 4];
            let Some(out) = out.get_mut(..colors) else { continue };
            out.copy_from_slice(ch);
            adjust::stored_channels(adj, fmt.mode, Some(fmt.sample), out, gray);
            for (v, o) in ch.iter_mut().zip(out.iter()) {
                *v += (o - *v) * k;
            }
        }
    }
    raw
}

pub(crate) fn mix_through_selection(
    fmt: &photocraft_color::PixelFormat,
    r: Rect,
    orig: &[[f32; 4]],
    edited: &[[f32; 4]],
    selection: Option<&Surface>,
) -> Vec<f32> {
    let w = (r.width() as usize).max(1);
    let mut out = Vec::with_capacity(orig.len() * fmt.channels());
    // The selection's coverage (channel 0) one row at a time, each of its tiles looked up once
    // per row rather than once per pixel.
    let mut coverage = vec![1.0f32; w];
    for (j, (edited_row, orig_row)) in edited.chunks(w).zip(orig.chunks(w)).enumerate() {
        if let Some(sel) = selection {
            sel.sample_row_strided(r.y0.saturating_add(j as i32), r.x0, 1, 0, &mut coverage);
        }
        for ((a, o), k) in edited_row.iter().zip(orig_row).zip(&coverage) {
            let mixed: [f32; 4] = std::array::from_fn(|c| o[c] + (a[c] - o[c]) * k);
            let mut enc = [0.0f32; 8];
            let m = photocraft_raster::from_rgba_into(fmt, mixed, &mut enc);
            out.extend_from_slice(&enc[..m]);
        }
    }
    out
}

#[cfg(test)]
#[path = "pixels_fractional_clear_tests.rs"]
mod fractional_clear_tests;
