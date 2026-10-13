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
pub(super) fn preflight(doc: &Document, id: LayerId, control: &Interrupt<'_>) -> Result<()> {
    let Some((dx, size)) = plan(doc, id, control)? else {
        return poll(control);
    };
    if doc.xcf_state.is_some() || doc.color_table.is_some() || doc.mode == photocraft_doc::ColorMode::Indexed {
        return Err(bad("source/indexed/accounted placement is not qualified"));
    }
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
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
    let pixels = u64::from(size.width).checked_mul(u64::from(size.height)).ok_or_else(|| bad("canvas area overflow"))?;
    let native_tile = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(40)).ok_or_else(|| bad("tile scratch overflow"))?;
    let peak = payload
        .checked_mul(2)
        .and_then(|n| n.checked_add(model.checked_mul(8)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(12)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(native_tile.checked_add(1024)?)?.checked_mul(5)?))
        .and_then(|n| n.checked_add(pixels.checked_mul(renders)?.checked_mul(64)?))
        .and_then(|n| n.checked_add(128 << 20));
    if peak.is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("aggregate placement model/COW/output/scratch limit"));
    }
    let work = (doc.layers.len() as u64)
        .checked_add(1)
        .and_then(|n| n.checked_pow(2))
        .and_then(|n| n.checked_add(tiles.checked_pow(2)?.checked_mul(2048)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(native_tile)?.checked_mul(16)?))
        .and_then(|n| n.checked_add(pixels.checked_mul(renders)?.checked_mul(2048)?))
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
            poll(control)?;
            crate::type_cmds::try_refresh(context, text)?;
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
    // Same top-level fit/clamp policy; unrelated layers, global paths and marks stay put.
    let (mut width, mut height) = (i64::from(staged.size.width), i64::from(staged.size.height));
    for layer in &staged.layers {
        poll(control)?;
        if let Some(board) = layer.artboard() {
            width = width.max(i64::from(board.rect.x1));
            height = height.max(i64::from(board.rect.y1));
        }
    }
    staged.size = Size::new(width.clamp(1, 300000) as u32, height.clamp(1, 300000) as u32);
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
            SampleType::U32 => [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
            SampleType::F64 => [0.5f64.to_bits() + 1, (-0f64).to_bits(), 0x7ff8000000001234, 0f64.to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
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
    fn ordinary_collision_layout_matches_existing_place_copy_without_global_geometry_changes() {
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
        assert_eq!(doc.work_path, before.work_path);
        assert_eq!(doc.layers[1], before.layers[1]);
        assert_eq!(doc.layer(id).unwrap().artboard().unwrap().rect, Rect::new(250, -4, 258, 4));
    }
    #[test]
    fn wide_negative_origin_hidden_words_and_source_default_survive_raw_placement_and_native() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (mut doc, id, child, raw) = fixture(sample);
            let before = doc.clone();
            run(&mut doc, id, &Interrupt::NONE).unwrap();
            assert_eq!(doc.layer(id).unwrap().artboard().unwrap().rect, Rect::new(103, -4, 111, 4));
            assert_eq!(doc.layer(child).unwrap().surface().unwrap().to_interleaved(Rect::new(106, -1, 107, 0)), raw);
            assert_eq!(doc.layer(child).unwrap().surface().unwrap().default_bytes(), before.layer(child).unwrap().surface().unwrap().default_bytes());
            assert_eq!(before.layer(child).unwrap().surface().unwrap().to_interleaved(Rect::new(-2, -1, -1, 0)), raw);
            let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
            assert_eq!(reopened.layer(child).unwrap().surface(), doc.layer(child).unwrap().surface());
        }
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
