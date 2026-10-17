//! Private, checked ordinary artboard placement. Raw surfaces never pass through f32.
use crate::{EngineError, Result};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Path, Size};
use photocraft_geom::{Affine, Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use std::sync::Arc;
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("artboard copy placement: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error) }
}
fn moved(rect: Rect, dx: i32) -> Result<Rect> {
    if rect.is_empty() {
        return Ok(Rect::EMPTY);
    }
    Ok(Rect::new(
        rect.x0.checked_add(dx).ok_or_else(|| bad("left edge overflow"))?,
        rect.y0,
        rect.x1.checked_add(dx).ok_or_else(|| bad("right edge overflow"))?,
        rect.y1,
    ))
}
fn plan(doc: &Document, id: LayerId, control: &Interrupt<'_>) -> Result<Option<(i32, Size)>> {
    poll(control)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad("unsupported canvas dimensions"));
    }
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |_| Ok(true))?;
    let mut found = None;
    for layer in &doc.layers {
        poll(control)?;
        if layer.id == id {
            if found.is_some() {
                return Err(bad("ambiguous artboard identity"));
            }
            found = Some(layer);
        }
    }
    let Some(layer) = found else {
        if doc.layer(id).is_some_and(|layer| layer.artboard().is_none()) {
            return Ok(None);
        }
        return Err(bad("artboard placement requires a top-level target"));
    };
    let Some(rect) = layer.artboard().map(|board| board.rect) else {
        return Ok(None);
    };
    if rect.x1 <= rect.x0 || rect.y1 <= rect.y0 {
        return Err(bad("positive artboard extent required"));
    }
    let (y0, y1, width) = (i64::from(rect.y0), i64::from(rect.y1), i64::from(rect.x1) - i64::from(rect.x0));
    let mut x = i64::from(rect.x1) + i64::from(super::GAP);
    // Same top-level collision predicate and bounded number of rounds as legacy.
    for _ in 0..=doc.layers.len() {
        poll(control)?;
        let end = x.checked_add(width).ok_or_else(|| bad("collision edge overflow"))?;
        let mut blocked = None;
        for other in &doc.layers {
            poll(control)?;
            if other.id == id {
                continue;
            }
            let Some(board) = other.artboard() else {
                continue;
            };
            let bounds = board.rect;
            if i64::from(bounds.x0) < end && i64::from(bounds.x1) > x && i64::from(bounds.y0) < y1 && i64::from(bounds.y1) > y0 {
                blocked = Some(blocked.map_or(bounds.x1, |edge: i32| edge.max(bounds.x1)));
            }
        }
        match blocked {
            Some(edge) => x = i64::from(edge) + i64::from(super::GAP),
            None => break,
        }
    }
    let edge = x.checked_add(width).filter(|edge| *edge <= i64::from(i32::MAX)).ok_or_else(|| bad("no room right of artboard; duplicate inPlace"))?;
    let dx = i32::try_from(x - i64::from(rect.x0)).map_err(|_| bad("placement offset overflow"))?;
    let size = Size::new(i64::from(doc.size.width).max(edge).clamp(1, 300000) as u32, doc.size.height);
    poll(control)?;
    Ok(Some((dx, size)))
}
fn surface_admission(surface: &Surface, dx: i32, control: &Interrupt<'_>, tiles: &mut u64) -> Result<()> {
    poll(control)?;
    let bounds = surface.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?;
    let destination = moved(bounds, dx)?;
    if !destination.is_empty() {
        let first = photocraft_geom::TileCoord::containing(destination.x0, destination.y0);
        let last = photocraft_geom::TileCoord::containing(
            destination.x1.checked_sub(1).ok_or_else(|| bad("tile edge"))?,
            destination.y1.checked_sub(1).ok_or_else(|| bad("tile edge"))?,
        );
        for coordinate in [first, last] {
            coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(|| bad("translated tile x overflow"))?;
            coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(|| bad("translated tile y overflow"))?;
        }
    }
    *tiles = tiles.checked_add(surface.tile_count() as u64).ok_or_else(|| bad("tile count overflow"))?;
    poll(control)
}
fn subtree(layer: &Layer, doc: &Document, dx: i32, depth: usize, control: &Interrupt<'_>, tiles: &mut u64, renders: &mut u64) -> Result<()> {
    poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("group depth limit"));
    }
    if layer.indexed_source.is_some() || layer.video.is_some() {
        return Err(bad("indexed placement is not qualified"));
    }
    if let Some(mask) = &layer.mask
        && mask.linked
    {
        surface_admission(&mask.surface, dx, control, tiles)?;
    }
    match &layer.content {
        LayerContent::Raster(surface) => surface_admission(surface, dx, control, tiles)?,
        LayerContent::Group(group) => {
            if let Some(board) = &group.artboard {
                moved(board.rect, dx)?;
            }
            for child in &group.children {
                subtree(child, doc, dx, depth + 1, control, tiles, renders)?;
            }
        }
        LayerContent::Text(text) => {
            if matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64)
                || text.cache.as_ref().is_some_and(|s| matches!(s.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64))
            {
                return Err(bad("wide text cache generation is not qualified"));
            }
            *renders = renders.checked_add(1).ok_or_else(|| bad("render count overflow"))?;
        }
        LayerContent::Shape(shape) => {
            if matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64)
                || shape.cache.as_ref().is_some_and(|s| matches!(s.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64))
            {
                return Err(bad("wide shape cache generation is not qualified"));
            }
            *renders = renders.checked_add(1).ok_or_else(|| bad("render count overflow"))?;
        }
        LayerContent::Smart(smart) => {
            if let Some(cache) = &smart.cache {
                surface_admission(cache, dx, control, tiles)?;
            }
            if let Some(mask) = &smart.filter_mask {
                surface_admission(&mask.surface, dx, control, tiles)?;
            }
        }
        LayerContent::BackedRaster(_) => return Err(bad("backed placement is not qualified")),
        _ => {}
    }
    poll(control)
}
// The same canvas-plus-top-level-board span used by the upstream fit_canvas.
// `moving` previews placement without allocating or mutating a Document.
fn fit_geometry(doc: &Document, moving: Option<(LayerId, i32)>, size: Size, control: &Interrupt<'_>) -> Result<((i32, i32), Size)> {
    let max = i64::from(crate::document_preset_cmds::MAX_DIMENSION);
    let (mut x0, mut y0, mut x1, mut y1) = (0i64, 0i64, i64::from(size.width), i64::from(size.height));
    for layer in &doc.layers {
        poll(control)?;
        if let Some(board) = layer.artboard() {
            let rect = if let Some((_, dx)) = moving.filter(|(id, _)| *id == layer.id) { moved(board.rect, dx)? } else { board.rect };
            x0 = x0.min(i64::from(rect.x0));
            y0 = y0.min(i64::from(rect.y0));
            x1 = x1.max(i64::from(rect.x1));
            y1 = y1.max(i64::from(rect.y1));
        }
    }
    let shift = |lo: i64, hi: i64| if hi - lo <= max { i32::try_from(-lo).unwrap_or(0) } else { 0 };
    let offset = (shift(x0, x1), shift(y0, y1));
    let size = Size::new((x1 + i64::from(offset.0)).clamp(1, max) as u32, (y1 + i64::from(offset.1)).clamp(1, max) as u32);
    Ok((offset, size))
}
fn fit_canvas_controlled(doc: &mut Document, control: &Interrupt<'_>) -> Result<()> {
    let (offset, mut size) = fit_geometry(doc, None, doc.size, control)?;
    if offset != (0, 0) {
        // Old fit_canvas keeps the complete translation unshifted if it cannot
        // be admitted. Cancellation still aborts this private candidate.
        match crate::image_cmds::canvas_translation_preflight(doc, offset, control) {
            Ok(()) => crate::image_cmds::canvas_translate_doc_with_interrupt(doc, offset, control)?,
            Err(_) => {
                poll(control)?;
                let max = i64::from(crate::document_preset_cmds::MAX_DIMENSION);
                let (mut width, mut height) = (i64::from(doc.size.width), i64::from(doc.size.height));
                for layer in &doc.layers {
                    poll(control)?;
                    if let Some(board) = layer.artboard() {
                        width = width.max(i64::from(board.rect.x1));
                        height = height.max(i64::from(board.rect.y1));
                    }
                }
                size = Size::new(width.clamp(1, max) as u32, height.clamp(1, max) as u32);
            }
        }
    }
    if size != doc.size {
        doc.size = size;
        crate::canvas_geom::refresh_with_interrupt(doc, crate::canvas_geom::Refresh::Shapes, control)?;
    }
    poll(control)
}
// Text rasterization is not canvas-clipped. Plan its actual existing ink
// rectangle only for the fallback which will render; cached whole-pixel moves
// keep their original native bytes and never acquire layout/font authority.
fn generated_text_envelope(layer: &Layer, doc: &Document, dx: i32, depth: usize, control: &Interrupt<'_>) -> Result<(u64, u64)> {
    poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("group depth limit"));
    }
    match &layer.content {
        LayerContent::Group(group) => {
            let mut total = (0u64, 0u64);
            for child in &group.children {
                let next = generated_text_envelope(child, doc, dx, depth + 1, control)?;
                total.0 = total.0.checked_add(next.0).ok_or_else(|| bad("text output overflow"))?;
                total.1 = total.1.checked_add(next.1).ok_or_else(|| bad("text work overflow"))?;
            }
            Ok(total)
        }
        LayerContent::Text(text) => {
            // translate_tysh touches only bytes34..50; with no TySh the cached
            // surface is also shifted directly, exactly as upstream.
            if text.cache.is_some() && text.psd_raw.as_ref().is_none_or(|raw| raw.len() >= 50) {
                return Ok((0, 0));
            }
            let resolution = crate::type_cmds::checked_layout_resolution(doc)?;
            poll(control)?;
            let layout =
                crate::type_cmds::text_engine()?.lock().unwrap_or_else(|error| error.into_inner()).layout_with_resolution(text, &resolution).map_err(bad)?;
            poll(control)?;
            let warp = photocraft_text::render::layout_warp(&layout, text.warp.as_ref());
            let transform = Affine::translate(f64::from(dx), 0.).mul(&text.transform);
            let rect = photocraft_text::render::ink_rect_warped(&layout, &transform, warp.as_ref());
            poll(control)?;
            if rect.is_empty() {
                return Ok((0, 0));
            }
            let pixels = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(|| bad("text rectangle overflow"))?;
            // The existing rasterizer returns an empty cache beyond MAX_PIXELS.
            if pixels > 256 * 1024 * 1024 {
                return Ok((0, 0));
            }
            let first = photocraft_geom::TileCoord::containing(rect.x0, rect.y0);
            let last = photocraft_geom::TileCoord::containing(rect.x1 - 1, rect.y1 - 1);
            let tiles = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
                .ok()
                .and_then(|w| u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok().and_then(|h| w.checked_mul(h)))
                .ok_or_else(|| bad("text tile count overflow"))?;
            let format = photocraft_color::PixelFormat { alpha: true, ..doc.pixel_format() };
            let tile = (TILE_SIZE as u64)
                .checked_mul(TILE_SIZE as u64)
                .and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64))
                .ok_or_else(|| bad("text native tile overflow"))?;
            let bytes = tiles
                .checked_mul(tile.checked_mul(6).and_then(|n| n.checked_add(256 * 12)).ok_or_else(|| bad("text tile metadata overflow"))?)
                .and_then(|n| n.checked_add(pixels.checked_mul(64)?))
                .and_then(|n| n.checked_add(4096))
                .ok_or_else(|| bad("text generated cache envelope overflow"))?;
            let work = pixels
                .checked_mul(2048)
                .and_then(|n| n.checked_add(tiles.checked_mul(tile)?.checked_mul(32)?))
                .and_then(|n| n.checked_add(tiles.checked_pow(2)?.checked_mul(65536)?))
                .ok_or_else(|| bad("text generated work overflow"))?;
            Ok((bytes, work))
        }
        _ => Ok((0, 0)),
    }
}
pub(super) fn preflight(doc: &Document, id: LayerId, control: &Interrupt<'_>) -> Result<()> {
    let Some((dx, size)) = plan(doc, id, control)? else {
        return poll(control);
    };
    if doc.xcf_state.is_some() || doc.color_table.is_some() || doc.mode == photocraft_doc::ColorMode::Indexed {
        return Err(bad("source/indexed/accounted placement is not qualified"));
    }
    let (_, fitted_size) = fit_geometry(doc, Some((id, dx)), size, control)?;
    let mut shapes = 0u64;
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
        if matches!(layer.content, LayerContent::Shape(_)) {
            // Forecast all possible refresh outputs, but leave actual unrelated
            // format admission to refresh_with_interrupt after real fit/fallback.
            shapes = shapes.saturating_add(1);
        }
        if matches!(layer.content, LayerContent::BackedRaster(_)) {
            return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
        }
        Ok(true)
    })?;
    let layer = doc.layers.iter().find(|layer| layer.id == id).ok_or(EngineError::NoLayer(id))?;
    let mut tiles = 0u64;
    let mut renders = 0u64;
    subtree(layer, doc, dx, 0, control, &mut tiles, &mut renders)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    if payload
        .checked_mul(2)
        .and_then(|n| n.checked_add(model.checked_mul(12)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(40)?))
        .is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("aggregate placement model/layout limit"));
    }
    let (text_output, text_work) = generated_text_envelope(layer, doc, dx, 0, control)?;
    let pixels = u64::from(fitted_size.width).checked_mul(u64::from(fitted_size.height)).ok_or_else(|| bad("canvas area overflow"))?;
    let tile_pixels = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).ok_or_else(|| bad("tile area overflow"))?;
    // The raster census is per occurrence, includes every physical tile/default,
    // and never deduplicates Arc sharing. Even GrayU8 occupies tile_pixels bytes.
    let all_tiles =
        raster.retained_bytes.checked_add(tile_pixels - 1).and_then(|n| n.checked_div(tile_pixels)).ok_or_else(|| bad("native tile count overflow"))?;
    let shape_tiles = u64::from(fitted_size.width)
        .div_ceil(TILE_SIZE as u64)
        .checked_mul(u64::from(fitted_size.height).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| bad("shape tile count overflow"))?;
    let shape_native_tile = tile_pixels.checked_mul(doc.pixel_format().bytes_per_pixel() as u64).ok_or_else(|| bad("shape native tile overflow"))?;
    let all_renders = renders.checked_add(shapes).ok_or_else(|| bad("shape render count overflow"))?;
    let generated_tiles = all_renders.checked_mul(shape_tiles).ok_or_else(|| bad("generated tile count overflow"))?;
    let generated_tile_bytes = shape_native_tile.checked_mul(6).and_then(|n| n.checked_add(256 * 12)).ok_or_else(|| bad("generated tile staging overflow"))?;
    let shape_output = generated_tiles
        .checked_mul(generated_tile_bytes)
        .and_then(|n| n.checked_add(all_renders.checked_mul(4096)?))
        .ok_or_else(|| bad("shape output staging overflow"))?;
    let native_tile = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(40)).ok_or_else(|| bad("tile scratch overflow"))?;
    let peak = payload
        .checked_mul(2)
        .and_then(|n| n.checked_add(model.checked_mul(12)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(40)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(64)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(native_tile.checked_add(1024)?)?.checked_mul(5)?))
        // Original + placement (<=4x tiles) + global shift (<=16x), raw
        // input/write scratch and refresh snapshot/map overlap are retained.
        .and_then(|n| n.checked_add(shape_output))
        .and_then(|n| n.checked_add(text_output))
        .and_then(|n| n.checked_add(pixels.checked_mul(all_renders)?.checked_mul(64)?))
        .and_then(|n| n.checked_add(128 << 20));
    if peak.is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("aggregate placement model/COW/output/scratch limit"));
    }
    let work = (doc.layers.len() as u64)
        .checked_add(1)
        .and_then(|n| n.checked_pow(2))
        .and_then(|n| n.checked_add(tiles.checked_pow(2)?.checked_mul(2048)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(native_tile)?.checked_mul(16)?))
        .and_then(|n| n.checked_add(text_work))
        .and_then(|n| n.checked_add(all_tiles.checked_add(generated_tiles)?.checked_pow(2)?.checked_mul(65536)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(128)?))
        .and_then(|n| n.checked_add(shape_tiles.checked_mul(all_renders)?.checked_mul(shape_native_tile)?.checked_mul(16)?))
        .and_then(|n| n.checked_add(pixels.checked_mul(all_renders)?.checked_mul(2048)?))
        .and_then(|n| n.checked_add(payload.checked_add(model)?.checked_mul(16)?));
    if work.is_none_or(|n| n > 8_000_000_000) {
        return Err(bad("placement aggregate work limit"));
    }
    poll(control)
}
fn translate_surface(source: &Surface, dx: i32, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let bounds = source.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?;
    moved(bounds, dx)?;
    let mut output = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|e| error(control, e))?;
    for (coordinate, _) in source.tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&bounds);
        if part.is_empty() {
            continue;
        }
        let destination = moved(part, dx)?;
        let raw = source.try_read_interleaved_with(part, control).map_err(|e| error(control, e))?;
        output.try_write_interleaved_with(destination, &raw, control).map_err(|e| error(control, e))?;
    }
    poll(control)?;
    Ok(output)
}
fn translate_path(path: &mut Path, affine: &Affine, control: &Interrupt<'_>) -> Result<()> {
    for subpath in &mut path.subpaths {
        poll(control)?;
        for knot in &mut subpath.knots {
            poll(control)?;
            let next = knot.transform(affine);
            for point in [next.anchor, next.in_ctrl, next.out_ctrl] {
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Err(bad("vector geometry overflow"));
                }
            }
            *knot = next;
        }
    }
    poll(control)
}
fn move_layer(layer: &mut Layer, context: &Document, dx: i32, depth: usize, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("group depth limit"));
    }
    let affine = Affine::translate(f64::from(dx), 0.);
    if let Some(reference) = &mut layer.effects.reference {
        let next = reference.0 + f64::from(dx);
        if !next.is_finite() || !reference.1.is_finite() {
            return Err(bad("effect reference overflow"));
        }
        reference.0 = next;
    }
    if let Some(mask) = &mut layer.mask
        && mask.linked
    {
        mask.surface = Arc::new(translate_surface(&mask.surface, dx, control)?);
    }
    if let Some(mask) = &mut layer.vector_mask
        && mask.linked
    {
        translate_path(&mut mask.path, &affine, control)?;
    }
    match &mut layer.content {
        LayerContent::Raster(surface) => *surface = translate_surface(surface, dx, control)?,
        LayerContent::Group(group) => {
            if let Some(board) = &mut group.artboard {
                board.rect = moved(board.rect, dx)?;
            }
            for child in &mut group.children {
                move_layer(child, context, dx, depth + 1, control)?;
            }
        }
        LayerContent::Text(text) => {
            text.transform = affine.mul(&text.transform);
            if !text.transform.m.iter().all(|v| v.is_finite()) {
                return Err(bad("text transform overflow"));
            }
            // Whole-pixel translation preserves imported rendering and TySh.
            let raw = text.psd_raw.as_deref().map(|raw| photocraft_text::psd::translate_tysh(raw, f64::from(dx), 0.));
            match (&mut text.cache, raw) {
                (Some(cache), Some(Some(raw))) => {
                    *cache = translate_surface(cache, dx, control)?;
                    text.psd_raw = Some(Arc::new(raw));
                }
                (Some(cache), None) => *cache = translate_surface(cache, dx, control)?,
                _ => {
                    poll(control)?;
                    crate::type_cmds::try_refresh(context, text)?;
                }
            }
            poll(control)?;
        }
        LayerContent::Shape(shape) => {
            let mut path = std::mem::take(&mut shape.path);
            translate_path(&mut path, &affine, control)?;
            poll(control)?;
            crate::vector_cmds::transform_shape(shape, &affine);
            shape.path = path;
            crate::vector_cmds::refresh_shape(context, shape);
            poll(control)?;
        }
        LayerContent::Smart(smart) => {
            crate::smart_cmds::transform_placement(smart, &affine);
            if !smart.transform.m.iter().all(|v| v.is_finite()) || smart.perspective.as_ref().is_some_and(|p| !p.iter().all(|v| v.is_finite())) {
                return Err(bad("smart placement overflow"));
            }
            if let Some(cache) = &mut smart.cache {
                *cache = translate_surface(cache, dx, control)?;
            }
            if let Some(mask) = &mut smart.filter_mask {
                mask.surface = Arc::new(translate_surface(&mask.surface, dx, control)?);
            }
        }
        _ => {}
    }
    poll(control)
}
pub(super) fn run(doc: &mut Document, id: LayerId, control: &Interrupt<'_>) -> Result<()> {
    preflight(doc, id, control)?;
    let Some((dx, size)) = plan(doc, id, control)? else {
        return poll(control);
    };
    poll(control)?;
    let mut staged = doc.clone();
    staged.size = size;
    let context = staged.clone();
    let copy = staged.layers.iter_mut().find(|layer| layer.id == id).ok_or(EngineError::NoLayer(id))?;
    move_layer(copy, &context, dx, 0, control)?;
    drop(context);
    fit_canvas_controlled(&mut staged, control)?;
    poll(control)?;
    *doc = staged;
    Ok(())
}

// Genuine layout/native/atomic regression drafts; no execution in this wave.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Artboard, ColorMode, SampleType};
    fn fixture(sample: SampleType) -> (Document, LayerId, LayerId, Vec<u8>) {
        let mut doc = Document::new("Own placement", Size::new(32, 20), ColorMode::Rgb, sample);
        let raw = match sample {
            SampleType::U32 => [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>(),
            SampleType::F64 => [0.5f64.to_bits() + 1, (-0f64).to_bits(), 0x7ff8000000001234, 0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect(),
            _ => vec![1, 2, 3, 255],
        };
        let mut raster = Layer::raster("Own raw", doc.pixel_format());
        raster.surface_mut().unwrap().try_write_interleaved_with(Rect::new(-2, -1, -1, 0), &raw, &Interrupt::NONE).unwrap();
        let child = raster.id;
        let mut board = Layer::group("Own board", vec![raster]);
        let LayerContent::Group(group) = &mut board.content else {
            panic!("group fixture");
        };
        group.artboard = Some(Artboard::new(Rect::new(-5, -4, 3, 4)));
        let id = board.id;
        doc.layers.push(board);
        (doc, id, child, raw)
    }
    #[test]
    fn ordinary_collision_layout_matches_existing_place_copy_with_global_negative_overhang() {
        let (mut doc, id, _, _) = fixture(SampleType::U8);
        let mut blocker = Layer::group("Blocker", Vec::new());
        let LayerContent::Group(group) = &mut blocker.content else {
            panic!("group fixture");
        };
        group.artboard = Some(Artboard::new(Rect::new(100, -3, 150, 8)));
        doc.layers.push(blocker);
        doc.work_path = Some(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(1., 2.), (3., 4.)])]));
        let before = doc.clone();
        let mut old = doc.clone();
        super::super::place_copy(&mut old, id).unwrap();
        run(&mut doc, id, &Interrupt::NONE).unwrap();
        assert_eq!(doc, old);
        assert_eq!(doc.size, Size::new(258, 24));
        assert_eq!(doc.work_path, before.work_path.as_ref().map(|path| path.transform(&Affine::translate(0., 4.))));
        let mut expected_blocker = before.layers[1].clone();
        let LayerContent::Group(group) = &mut expected_blocker.content else { panic!("group fixture") };
        group.artboard.as_mut().unwrap().rect = Rect::new(100, 1, 150, 12);
        assert_eq!(doc.layers[1], expected_blocker);
        assert_eq!(doc.layer(id).unwrap().artboard().unwrap().rect, Rect::new(250, 0, 258, 8));
    }
    #[test]
    fn wide_negative_origin_hidden_words_and_source_default_survive_raw_placement_and_native() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (mut doc, id, child, raw) = fixture(sample);
            let before = doc.clone();
            run(&mut doc, id, &Interrupt::NONE).unwrap();
            assert_eq!(doc.layer(id).unwrap().artboard().unwrap().rect, Rect::new(103, 0, 111, 8));
            assert_eq!(doc.layer(child).unwrap().surface().unwrap().to_interleaved(Rect::new(106, 3, 107, 4)), raw);
            assert_eq!(doc.layer(child).unwrap().surface().unwrap().default_bytes(), before.layer(child).unwrap().surface().unwrap().default_bytes());
            assert_eq!(before.layer(child).unwrap().surface().unwrap().to_interleaved(Rect::new(-2, -1, -1, 0)), raw);
            let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
            assert_eq!(reopened.layer(child).unwrap().surface(), doc.layer(child).unwrap().surface());
        }
    }
    #[test]
    fn generated_shape_physical_tiles_are_admitted_before_any_placement_clone() {
        let (mut doc, id, _, _) = fixture(SampleType::U8);
        for _ in 0..4096 {
            let shape = photocraft_doc::ShapeLayer {
                path: Path::new(vec![photocraft_doc::Subpath::polygon(&[(0., 0.), (1., 0.), (1., 1.), (0., 1.)])]),
                fill: Some(photocraft_doc::Fill::Solid(photocraft_color::Color::WHITE)),
                ..Default::default()
            };
            doc.layers.push(Layer::new("Uncached shape", LayerContent::Shape(shape)));
        }
        let before = doc.clone();
        let error = preflight(&doc, id, &Interrupt::NONE).unwrap_err();
        assert!(error.to_string().contains("aggregate placement"), "{error}");
        assert_eq!(doc, before);
    }
    #[test]
    fn controlled_placement_preserves_cached_imported_text_and_tysh_under_global_fit() {
        let (mut doc, id, child, raw) = fixture(SampleType::U8);
        let cache = doc.layer(child).unwrap().surface().unwrap().clone();
        let mut text = photocraft_doc::TextLayer { text: "Hello!".into(), font_family: "No Such Font".into(), size_pt: 26., ..Default::default() };
        text.transform = Affine::translate(-2., 0.);
        text.psd_raw = Some(Arc::new(photocraft_text::psd::build_tysh(&text, 72., None)));
        text.cache = Some(cache);
        let old_raw = text.psd_raw.clone().unwrap();
        doc.layer_mut(child).unwrap().content = LayerContent::Text(text);
        let before = doc.clone();
        let mut local = before.layer(child).unwrap().clone();
        move_layer(&mut local, &before, 108, 0, &Interrupt::NONE).unwrap();
        let LayerContent::Text(local_text) = &local.content else { panic!("text fixture") };
        let local_raw = local_text.psd_raw.as_ref().unwrap();
        assert_eq!((&local_raw[..34], &local_raw[50..]), (&old_raw[..34], &old_raw[50..]), "placement only updates raw TySh translation");
        assert_eq!(local_text.cache.as_ref().unwrap().to_interleaved(Rect::new(106, -1, 107, 0)), raw);
        run(&mut doc, id, &Interrupt::NONE).unwrap();
        let LayerContent::Text(text) = &doc.layer(child).unwrap().content else { panic!("text fixture") };
        assert_eq!(text.cache.as_ref().unwrap().to_interleaved(Rect::new(106, 3, 107, 4)), raw);
        assert_eq!(text.transform.m[4..], [106., 4.]);
        let next_raw = text.psd_raw.as_ref().unwrap();
        // Whole-canvas fit follows the existing retag_text contract: it rebuilds
        // TySh geometry at the same resolution while retaining the native cache.
        let resolution = crate::type_cmds::checked_layout_resolution(&doc).unwrap();
        let layout =
            crate::type_cmds::text_engine().unwrap().lock().unwrap_or_else(|error| error.into_inner()).layout_with_resolution(text, &resolution).unwrap();
        assert_eq!(next_raw.as_ref(), &photocraft_text::psd::build_tysh_with_resolution(text, &resolution, layout.bounds()).unwrap());
        let parsed = photocraft_text::psd::parse_tysh(next_raw).unwrap();
        assert_eq!(parsed.transform.m[4..], [106., 4.]);
        assert_eq!(before.layer(child).unwrap().surface().unwrap().to_interleaved(Rect::new(-2, -1, -1, 0)), raw);
    }
    #[test]
    fn late_cancel_overflow_and_source_context_refusals_leave_caller_document_exact() {
        let (mut doc, id, _, _) = fixture(SampleType::U32);
        let before = doc.clone();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 30;
        assert!(matches!(run(&mut doc, id, &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
        assert_eq!(doc, before);
        doc.xcf_state = Some(photocraft_doc::XcfState::default());
        let source = doc.clone();
        assert!(run(&mut doc, id, &Interrupt::NONE).is_err());
        assert_eq!(doc, source);
        doc = before;
        let LayerContent::Group(group) = &mut doc.layers[0].content else {
            panic!("group fixture");
        };
        group.artboard.as_mut().unwrap().rect = Rect::new(i32::MAX - 20, 0, i32::MAX, 20);
        let overflow = doc.clone();
        assert!(run(&mut doc, id, &Interrupt::NONE).is_err());
        assert_eq!(doc, overflow);
    }
}
