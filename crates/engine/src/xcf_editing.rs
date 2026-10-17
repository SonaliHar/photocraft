//! Canonical indexed edits and imported metadata duplication. No source access is granted here.
use crate::{EngineError, Result};
use photocraft_doc::item_identity::{FilterId, ItemRef, MaskId};
use photocraft_doc::{ColorMode, ColorTable, Document, EditableIndexedLayer, EditableIndexedRaster, Layer, LayerContent, LayerId, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use std::{collections::BTreeMap, sync::Arc};

fn layer_inventory(doc: &Document) -> Result<BTreeMap<LayerId, &Layer>> {
    let mut layers = BTreeMap::new();
    let mut pending = vec![(doc.layers.as_slice(), 0usize)];
    while let Some((siblings, depth)) = pending.pop() {
        if depth > 100 {
            return Err(error("layer depth"));
        }
        for layer in siblings {
            if layer.id.0 == 0 || layers.insert(layer.id, layer).is_some() || layers.len() > 10000 {
                return Err(error("layer identity"));
            }
            if let LayerContent::Group(group) = &layer.content {
                pending.push((group.children.as_slice(), depth + 1));
            }
        }
    }
    Ok(layers)
}
fn error(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("cannot preserve indexed editing state: {e}"))
}
fn raster_format(surface: &Surface) -> Result<usize> {
    let format = surface.format();
    if format.mode != ColorMode::Rgb || format.sample != SampleType::U8 {
        return Err(error("indexed pixels require RGB U8 storage; convert the image mode explicitly"));
    }
    Ok(format.channels())
}
fn allocate(count: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(count).map_err(error)?;
    Ok(bytes)
}
fn plane_count(bounds: Rect) -> Result<usize> {
    let w = i64::from(bounds.x1) - i64::from(bounds.x0);
    let h = i64::from(bounds.y1) - i64::from(bounds.y0);
    let count = w.checked_mul(h).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| error("invalid indexed extent"))?;
    if w <= 0 || h <= 0 || w > 262144 || h > 262144 || count > 64 << 20 {
        return Err(error("indexed extent exceeds limits"));
    }
    Ok(count)
}
#[cfg(test)]
pub(crate) fn expanded(raster: &EditableIndexedRaster, format: PixelFormat, transparent: Option<u8>) -> Result<Surface> {
    expanded_with(raster, format, transparent, &photocraft_raster::Interrupt::NONE)
}
pub(crate) fn expanded_with(
    raster: &EditableIndexedRaster,
    format: PixelFormat,
    transparent: Option<u8>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Surface> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    raster.validate_with(control).map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
    if format.mode != ColorMode::Rgb || format.sample != SampleType::U8 {
        return Err(error("indexed pixels require RGB U8 storage"));
    }
    let channels = format.channels();
    let count = plane_count(raster.bounds())?;
    let mut bytes = allocate(count.checked_mul(channels).ok_or_else(|| error("indexed byte count"))?)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    let bounds = raster.bounds();
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let mut pixel = raster.expanded_pixel(x, y).map_err(error)?;
            let offset =
                usize::try_from((i64::from(y) - i64::from(bounds.y0)) * (i64::from(bounds.x1) - i64::from(bounds.x0)) + i64::from(x) - i64::from(bounds.x0))
                    .map_err(error)?;
            if offset.is_multiple_of(4096) && control.cancelled() {
                return Err(EngineError::Cancelled);
            }
            if raster.indices().get(offset).copied() == transparent {
                pixel[3] = 0;
            }
            bytes.extend_from_slice(pixel.get(..channels).ok_or_else(|| error("indexed format channels"))?);
        }
    }
    let mut surface = Surface::try_with_default_bytes(format, [0; 4].get(..channels).ok_or_else(|| error("indexed format channels"))?).map_err(error)?;
    surface.try_write_interleaved_with(bounds, &bytes, control).map_err(error)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    surface.try_prune_with(control).map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(surface)
}
fn table(doc: &Document) -> Result<&ColorTable> {
    let table = doc.color_table.as_ref().ok_or_else(|| error("missing color table"))?;
    if table.colors.is_empty() || table.colors.len() > 256 || table.transparent.is_some_and(|i| usize::from(i) >= table.colors.len()) {
        return Err(error("invalid color table"));
    }
    Ok(table)
}

/// Synchronize edited expanded pixels with current indices before committing history.
/// Unchanged RGB keeps its existing palette ordinal, including duplicate entries. New colors
/// use the existing Color Table nearest-color rule; alpha is stored independently. Explicit
/// conversion out of Indexed clears current planes, while immutable source provenance survives.
pub fn synchronize_indexed(before: &Document, doc: &mut Document) -> Result<()> {
    synchronize_indexed_with_interrupt(before, doc, &photocraft_raster::Interrupt::NONE)
}

/// Closed direct-lane comparison charge, including allocated tile-tail scanning.
pub(crate) fn direct_indexed_sync_work(pixels: usize, tiles: usize) -> Result<u64> {
    let tile_pixels = u64::try_from(tiles)
        .map_err(error)?
        .checked_mul((photocraft_geom::TILE_SIZE as u64).checked_mul(photocraft_geom::TILE_SIZE as u64).ok_or_else(|| error("indexed work overflow"))?)
        .ok_or_else(|| error("indexed work overflow"))?;
    u64::try_from(pixels).map_err(error)?.checked_mul(8).and_then(|n| n.checked_add(tile_pixels)).ok_or_else(|| error("indexed work overflow"))
}
/// Stage legacy RGB remapping; explicit current planes instead undergo exact verification.
pub fn synchronize_indexed_with_interrupt(before: &Document, doc: &mut Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    let poll = || control.check().map_err(|_| EngineError::Cancelled);
    poll()?;
    let Some(state) = doc.xcf_state.as_ref() else {
        return Ok(());
    };
    if doc.mode != ColorMode::Indexed {
        if let Some(state) = &mut doc.xcf_state {
            state.editable_indexed.clear();
        }
        return Ok(());
    }
    let table = table(doc)?.clone();
    let existing: BTreeMap<_, _> = state.editable_indexed.iter().map(|i| (i.owner, &i.raster)).collect();
    if existing.len() != state.editable_indexed.len() {
        return Err(error("duplicate indexed owner"));
    }
    let before_layers = layer_inventory(before)?;
    let current_layers = layer_inventory(doc)?;
    let mut updates = Vec::new();
    updates.try_reserve_exact(current_layers.len()).map_err(error)?;
    let mut work = 0_u64;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)
        .map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
    let mut resident = footprint.retained_bytes.checked_add(footprint.clone_metadata_bytes).ok_or_else(|| error("indexed residency overflow"))?;
    for indexed in &state.editable_indexed {
        poll()?;
        resident = resident
            .checked_add(indexed.raster.indices().capacity() as u64)
            .and_then(|n| n.checked_add(indexed.raster.alpha().map_or(0, |alpha| alpha.capacity() as u64)))
            .and_then(|n| n.checked_add(indexed.raster.palette_capacity_bytes() as u64))
            .filter(|n| *n <= photocraft_raster::MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| error("indexed resident plane limit"))?;
    }
    for layer in current_layers.values() {
        poll()?;
        let LayerContent::Raster(surface) = &layer.content else {
            if layer.is_group() {
                continue;
            }
            return Err(error("convert out of Indexed before adding procedural layer content"));
        };
        let current = existing.get(&layer.id).copied();
        if let Some(current) = current {
            current.validate_with(control).map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
            if current.palette() != table.colors.as_slice() {
                return Err(error("use Color Table to change the indexed palette"));
            }
            let previous = before.xcf_state.as_ref().and_then(|s| s.editable_indexed.iter().find(|i| i.owner == layer.id));
            if previous.is_some_and(|p| &p.raster == current)
                && before_layers.get(&layer.id).and_then(|l| l.surface()) == Some(surface)
                && before.color_table.as_ref() == Some(&table)
            {
                continue;
            }
            let canonical_changed = previous.is_none_or(|p| &p.raster != current) || before.color_table.as_ref() != Some(&table);
            if canonical_changed {
                // Explicit plane/palette commands already supplied both carriers.
                // Verify that proposal exactly; do not reverse-map duplicate colors
                // or alter its independent alpha/transparent-index representation.
                let channels = raster_format(surface)?;
                let bounds = current.bounds();
                let count = plane_count(bounds)?;
                work = work
                    .checked_add(direct_indexed_sync_work(count, surface.tile_count())?)
                    .filter(|n| *n <= 1 << 30)
                    .ok_or_else(|| error("indexed synchronization work limit"))?;
                let row_bytes = (bounds.width() as u64).checked_mul(channels as u64).ok_or_else(|| error("indexed row overflow"))?;
                if resident.checked_add(row_bytes).is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
                    return Err(error("indexed verification resident limit"));
                }
                surface.try_validate_tile_origins_with(control).map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
                if surface.default_bytes().len() != channels || surface.default_bytes().iter().any(|byte| *byte != 0) {
                    return Err(error("explicit indexed carrier default mismatch"));
                }
                // A whole-owner proposal cannot leave old nondefault pixels outside
                // its canonical extent. Scan allocated tile tails without expanding
                // a huge sparse bounding rectangle or allocating another surface.
                let tile_side = usize::try_from(photocraft_geom::TILE_SIZE).map_err(error)?;
                let tile_bytes = tile_side.checked_mul(tile_side).and_then(|n| n.checked_mul(channels)).ok_or_else(|| error("indexed tile overflow"))?;
                for (coordinate, tile) in surface.tiles() {
                    poll()?;
                    if tile.bytes().len() != tile_bytes {
                        return Err(error("explicit indexed carrier tile length"));
                    }
                    let rect = coordinate.rect();
                    for (offset, pixel) in tile.bytes().chunks_exact(channels).enumerate() {
                        if offset.is_multiple_of(4096) {
                            poll()?;
                        }
                        let x = i64::from(rect.x0) + i64::try_from(offset % tile_side).map_err(error)?;
                        let y = i64::from(rect.y0) + i64::try_from(offset / tile_side).map_err(error)?;
                        if (x < i64::from(bounds.x0) || x >= i64::from(bounds.x1) || y < i64::from(bounds.y0) || y >= i64::from(bounds.y1))
                            && pixel.iter().any(|byte| *byte != 0)
                        {
                            return Err(error("explicit indexed carrier outside extent mismatch"));
                        }
                    }
                }
                let width = usize::try_from(bounds.width()).map_err(error)?;
                for y in bounds.y0..bounds.y1 {
                    poll()?;
                    let y1 = y.checked_add(1).ok_or_else(|| error("indexed row overflow"))?;
                    let raw = surface
                        .try_read_interleaved_with(Rect::new(bounds.x0, y, bounds.x1, y1), control)
                        .map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
                    if raw.len() != usize::try_from(row_bytes).map_err(error)? {
                        return Err(error("explicit indexed carrier row length"));
                    }
                    for (column, pixel) in raw.chunks_exact(channels).enumerate() {
                        if column.is_multiple_of(4096) {
                            poll()?;
                        }
                        let x = i32::try_from(i64::from(bounds.x0) + i64::try_from(column).map_err(error)?).map_err(error)?;
                        let mut expected = current.expanded_pixel(x, y).map_err(error)?;
                        let ordinal = usize::try_from(i64::from(y) - i64::from(bounds.y0))
                            .map_err(error)?
                            .checked_mul(width)
                            .and_then(|n| n.checked_add(column))
                            .ok_or_else(|| error("indexed offset overflow"))?;
                        if current.indices().get(ordinal).copied() == table.transparent {
                            expected[3] = 0;
                        }
                        if expected.get(..channels) != Some(pixel) {
                            return Err(error("explicit indexed carrier pixel mismatch"));
                        }
                    }
                }
                poll()?;
                continue;
            }
        }
        let channels = raster_format(surface)?;
        let bounds = current.map_or_else(
            || {
                let content = surface.content_bounds();
                if content.is_empty() { doc.bounds() } else { content }
            },
            |r| r.bounds().union(&surface.content_bounds()),
        );
        let count = plane_count(bounds)?;
        work = work
            .checked_add((count as u64).checked_mul(table.colors.len() as u64 + 8).ok_or_else(|| error("indexed work overflow"))?)
            .filter(|n| *n <= 1 << 30)
            .ok_or_else(|| error("indexed synchronization work limit"))?;
        // Include raw reads, both new planes and tile-expanded output before
        // allocating any of them. Output tile edges can exceed declared bounds.
        let width = u64::from(bounds.width());
        let height = u64::from(bounds.height());
        let tile_width = width.checked_add(2 * photocraft_geom::TILE_SIZE as u64).ok_or_else(|| error("indexed staging overflow"))?;
        let tile_height = height.checked_add(2 * photocraft_geom::TILE_SIZE as u64).ok_or_else(|| error("indexed staging overflow"))?;
        let staging = (count as u64)
            .checked_mul(channels as u64 + 2)
            .and_then(|n| n.checked_add(tile_width.checked_mul(tile_height)?.checked_mul(channels as u64)?))
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| error("indexed staging overflow"))?;
        resident = resident
            .checked_add(staging)
            .filter(|n| *n <= photocraft_raster::MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| error("indexed staging resident limit"))?;
        let raw =
            surface.try_read_interleaved_with(bounds, control).map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
        let mut indices = allocate(count)?;
        let mut alpha = allocate(count)?;
        let mut has_alpha = current.is_some_and(|r| r.alpha().is_some());
        let width = i64::from(bounds.x1) - i64::from(bounds.x0);
        for (offset, pixel) in raw.chunks_exact(channels).enumerate() {
            if offset.is_multiple_of(4096) {
                poll()?;
            }
            let rgb: [u8; 3] = pixel.get(..3).and_then(|v| v.try_into().ok()).ok_or_else(|| error("indexed RGB sample"))?;
            let x = i32::try_from(i64::from(bounds.x0) + i64::try_from(offset).map_err(error)? % width).map_err(error)?;
            let y = i32::try_from(i64::from(bounds.y0) + i64::try_from(offset).map_err(error)? / width).map_err(error)?;
            let old = current.and_then(|r| r.expanded_pixel(x, y).ok());
            let old_index = current.and_then(|r| {
                let b = r.bounds();
                if x < b.x0 || x >= b.x1 || y < b.y0 || y >= b.y1 {
                    return None;
                }
                let n = usize::try_from((i64::from(y) - i64::from(b.y0)) * (i64::from(b.x1) - i64::from(b.x0)) + i64::from(x) - i64::from(b.x0)).ok()?;
                r.indices().get(n).copied()
            });
            let index = if old.is_some_and(|p| p.get(..3) == Some(rgb.as_slice())) {
                old_index.ok_or_else(|| error("indexed identity"))?
            } else {
                u8::try_from(table.nearest(rgb.map(|v| f32::from(v) / 255.0))).map_err(error)?
            };
            let mut a = pixel.get(3).copied().unwrap_or(255);
            if table.transparent == Some(index) && a == 0 && old_index == Some(index) {
                a = old.map_or(a, |p| p[3]);
            }
            has_alpha |= a != 255;
            indices.push(index);
            alpha.push(a);
        }
        let raster =
            EditableIndexedRaster::try_new_with_interrupt(bounds, Arc::new(indices), has_alpha.then(|| Arc::new(alpha)), table.colors.clone(), control)
                .map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
        let output = expanded_with(&raster, surface.format(), table.transparent, control)
            .map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { failure })?;
        updates.push((layer.id, raster, output));
    }
    poll()?;
    for (owner, raster, surface) in updates {
        *doc.layer_mut(owner).and_then(Layer::surface_mut).ok_or_else(|| error("missing indexed layer"))? = surface;
        let state = doc.xcf_state.as_mut().ok_or_else(|| error("missing indexed state"))?;
        if let Some(current) = state.editable_indexed.iter_mut().find(|i| i.owner == owner) {
            current.raster = raster;
        } else {
            state.editable_indexed.push(EditableIndexedLayer { owner, raster });
        }
    }
    Ok(())
}

/// Recolor canonical indexed layers by their exact ordinals; preserve independent alpha.
/// Returns handled layer IDs so the legacy palette path does not recolor them a second time.
pub fn replace_palette(doc: &mut Document, table: &ColorTable) -> Result<Vec<LayerId>> {
    replace_palette_with_interrupt(doc, table, &photocraft_raster::Interrupt::NONE)
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PaletteReplacementRequirements {
    pub retained_bytes: u64,
    pub staged_bytes: u64,
    pub scratch_bytes: u64,
    pub work_units: u64,
}
struct PaletteReplacementUpdate {
    owner: LayerId,
    indexed: usize,
    raster: Option<EditableIndexedRaster>,
    surface: Option<Surface>,
}
fn palette_poll(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn palette_failure(control: &photocraft_raster::Interrupt<'_>, failure: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(failure) }
}
fn palette_add(value: &mut u64, amount: u64) -> Result<()> {
    *value = value.checked_add(amount).ok_or_else(|| error("palette ledger overflow"))?;
    Ok(())
}
fn palette_layer_count(layers: &[Layer], depth: usize, count: &mut usize, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("palette hierarchy depth"));
    }
    for layer in layers {
        palette_poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= 10_000).ok_or_else(|| error("palette layer count"))?;
        if let LayerContent::Group(group) = &layer.content {
            palette_layer_count(&group.children, depth + 1, count, control)?;
        }
    }
    Ok(())
}
fn palette_layer<'a>(layers: &'a [Layer], owner: LayerId, control: &photocraft_raster::Interrupt<'_>) -> Result<Option<&'a Layer>> {
    for layer in layers {
        palette_poll(control)?;
        if layer.id == owner {
            return Ok(Some(layer));
        }
        if let LayerContent::Group(group) = &layer.content
            && let Some(found) = palette_layer(&group.children, owner, control)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}
fn palette_inventory<'a>(layers: &'a [Layer], output: &mut Vec<&'a Layer>, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    for layer in layers {
        palette_poll(control)?;
        if layer.id.0 == 0 {
            return Err(error("zero palette layer identity"));
        }
        let position = match output.binary_search_by_key(&layer.id, |layer| layer.id) {
            Ok(_) => return Err(error("duplicate palette layer identity")),
            Err(position) => position,
        };
        // Capacity was admitted and reserved for the complete counted tree.
        if output.len() >= output.capacity() {
            return Err(error("palette inventory capacity"));
        }
        output.insert(position, layer);
        if let LayerContent::Group(group) = &layer.content {
            palette_inventory(&group.children, output, control)?;
        }
    }
    Ok(())
}
fn palette_tile_count(bounds: Rect) -> Result<u64> {
    use photocraft_geom::{TILE_SIZE, TileCoord};
    if bounds.is_empty() {
        return Err(error("palette empty canonical extent"));
    }
    let first = TileCoord::containing(bounds.x0, bounds.y0);
    let last = TileCoord::containing(
        bounds.x1.checked_sub(1).ok_or_else(|| error("palette extent"))?,
        bounds.y1.checked_sub(1).ok_or_else(|| error("palette extent"))?,
    );
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(|| error("palette tile coordinate"))?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(|| error("palette tile coordinate"))?;
    }
    u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .ok()
        .and_then(|w| u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| error("palette tile count"))
}
pub(crate) fn palette_replacement_requirements(
    doc: &Document,
    table: &ColorTable,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<PaletteReplacementRequirements> {
    palette_poll(control)?;
    let Some(state) = &doc.xcf_state else { return Ok(PaletteReplacementRequirements::default()) };
    if state.editable_indexed.is_empty() {
        return Ok(PaletteReplacementRequirements::default());
    }
    if table.colors.is_empty() || table.colors.len() > 256 || table.transparent.is_some_and(|i| usize::from(i) >= table.colors.len()) {
        return Err(error("invalid color table"));
    }
    if state.editable_indexed.len() > 10_000 {
        return Err(error("palette indexed owner count"));
    }
    let mut layers = 0usize;
    palette_layer_count(&doc.layers, 0, &mut layers, control)?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, control).map_err(|failure| palette_failure(control, failure))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, control).map_err(|failure| palette_failure(control, failure))? as u64;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let mut required = PaletteReplacementRequirements {
        retained_bytes: payload
            .checked_add(model)
            .and_then(|n| n.checked_add(footprint.clone_metadata_bytes))
            .ok_or_else(|| error("palette retained ledger"))?,
        ..Default::default()
    };
    let entries = state.editable_indexed.len() as u64;
    palette_add(
        &mut required.staged_bytes,
        entries
            .checked_mul((std::mem::size_of::<PaletteReplacementUpdate>() + std::mem::size_of::<LayerId>() + std::mem::size_of::<(LayerId, usize)>()) as u64)
            .and_then(|n| n.checked_add((layers as u64).checked_mul(std::mem::size_of::<&Layer>() as u64)?))
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| error("palette index ledger"))?,
    )?;
    palette_add(
        &mut required.work_units,
        entries
            .checked_mul(layers as u64)
            .and_then(|n| n.checked_add(entries.checked_mul(entries)?))
            .and_then(|n| n.checked_add((layers as u64).checked_mul(layers as u64)?))
            .and_then(|n| n.checked_mul(8))
            .ok_or_else(|| error("palette lookup work"))?,
    )?;
    for (position, item) in state.editable_indexed.iter().enumerate() {
        palette_poll(control)?;
        if item.owner.0 == 0 || state.editable_indexed.get(..position).is_some_and(|prior| prior.iter().any(|entry| entry.owner == item.owner)) {
            return Err(error("duplicate palette owner"));
        }
        let Some(layer) = palette_layer(&doc.layers, item.owner, control)? else { continue };
        let LayerContent::Raster(surface) = &layer.content else { return Err(error("indexed palette owner is not resident raster")) };
        let channels = raster_format(surface)?;
        item.raster.validate_with(control).map_err(|failure| palette_failure(control, failure))?;
        let mut removed = false;
        for chunk in item.raster.indices().chunks(4096) {
            palette_poll(control)?;
            removed |= chunk.iter().any(|index| usize::from(*index) >= table.colors.len());
        }
        let tiles = palette_tile_count(item.raster.bounds())?;
        let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(channels as u64))
            .ok_or_else(|| error("palette tile bytes"))?;
        palette_add(
            &mut required.staged_bytes,
            tiles
                .checked_mul(tile_bytes.checked_add(1024).ok_or_else(|| error("palette map ledger"))?)
                .and_then(|n| n.checked_add(if removed { item.raster.indices().len() as u64 } else { 0 }))
                .and_then(|n| n.checked_add(1024))
                .ok_or_else(|| error("palette output ledger"))?,
        )?;
        required.scratch_bytes =
            required.scratch_bytes.max(tile_bytes.checked_mul(8).and_then(|n| n.checked_add(4096)).ok_or_else(|| error("palette scratch ledger"))?);
        let pixels = item.raster.indices().len() as u64;
        let sample_work = pixels
            .checked_mul(if removed {
                (table.colors.len() as u64).checked_mul(8).and_then(|n| n.checked_add(64)).ok_or_else(|| error("palette remap work"))?
            } else {
                64
            })
            .ok_or_else(|| error("palette sample work"))?;
        palette_add(
            &mut required.work_units,
            sample_work
                .checked_add(
                    tiles
                        .checked_mul(tiles.checked_add(1).ok_or_else(|| error("palette scheduler work"))?)
                        .and_then(|n| n.checked_mul(256))
                        .ok_or_else(|| error("palette scheduler work"))?,
                )
                .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes)?.checked_mul(8)?))
                .ok_or_else(|| error("palette work overflow"))?,
        )?;
        if required.work_units > 8_000_000_000
            || required
                .retained_bytes
                .checked_add(required.staged_bytes)
                .and_then(|n| n.checked_add(required.scratch_bytes))
                .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
        {
            return Err(error("aggregate palette preparation limit"));
        }
    }
    if required.work_units > 8_000_000_000
        || required
            .retained_bytes
            .checked_add(required.staged_bytes)
            .and_then(|n| n.checked_add(required.scratch_bytes))
            .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(error("aggregate palette preparation limit"));
    }
    palette_poll(control)?;
    Ok(required)
}
fn palette_reserve<T>(count: usize) -> Result<Vec<T>> {
    let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or_else(|| error("palette capacity overflow"))?;
    if bytes as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(error("palette capacity limit"));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| error("palette allocation failed"))?;
    if result.capacity().checked_mul(std::mem::size_of::<T>()).is_none_or(|n| n > bytes) {
        return Err(error("palette actual capacity limit"));
    }
    Ok(result)
}
fn palette_carrier(
    raster: &EditableIndexedRaster,
    format: PixelFormat,
    transparent: Option<u8>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Surface> {
    let channels = format.channels();
    let bounds = raster.bounds();
    let side = photocraft_geom::TILE_SIZE as usize;
    let maximum = side.checked_mul(side).and_then(|n| n.checked_mul(channels)).ok_or_else(|| error("palette tile scratch"))?;
    let mut bytes = palette_reserve::<u8>(maximum)?;
    while bytes.len() < maximum {
        palette_poll(control)?;
        bytes.resize(bytes.len().saturating_add(64 * 1024).min(maximum), 0);
    }
    let mut output = Surface::try_with_default_bytes(format, [0u8; 4].get(..channels).ok_or_else(|| error("palette channels"))?)
        .map_err(|failure| palette_failure(control, failure))?;
    for coordinate in bounds.tiles() {
        palette_poll(control)?;
        let part = coordinate.rect().intersect(&bounds);
        let width = part.width() as usize;
        let length = width.checked_mul(part.height() as usize).and_then(|n| n.checked_mul(channels)).ok_or_else(|| error("palette tile length"))?;
        let raw = bytes.get_mut(..length).ok_or_else(|| error("palette tile scratch range"))?;
        for (row, data) in raw.chunks_exact_mut(width.checked_mul(channels).ok_or_else(|| error("palette row length"))?).enumerate() {
            palette_poll(control)?;
            let y = i32::try_from(i64::from(part.y0) + row as i64).map_err(error)?;
            for (column, pixel) in data.chunks_exact_mut(channels).enumerate() {
                let x = i32::try_from(i64::from(part.x0) + column as i64).map_err(error)?;
                let mut expanded = raster.expanded_pixel(x, y).map_err(error)?;
                let offset = usize::try_from(i64::from(y) - i64::from(bounds.y0))
                    .ok()
                    .and_then(|n| n.checked_mul(bounds.width() as usize))
                    .and_then(|n| usize::try_from(i64::from(x) - i64::from(bounds.x0)).ok().and_then(|x| n.checked_add(x)))
                    .ok_or_else(|| error("palette ordinal offset"))?;
                if raster.indices().get(offset).copied() == transparent {
                    expanded[3] = 0;
                }
                pixel.copy_from_slice(expanded.get(..channels).ok_or_else(|| error("palette channels"))?);
            }
        }
        output.try_write_interleaved_with(part, raw, control).map_err(|failure| palette_failure(control, failure))?;
    }
    output.try_prune_with(control).map_err(|failure| palette_failure(control, failure))?;
    palette_poll(control)?;
    Ok(output)
}
/// Stage every carrier/plane before a final cancellation fence. Once publishing
/// starts the bounded, allocation-free commit defers cancellation; no standalone
/// caller can observe an error with a partly replaced palette.
pub fn replace_palette_with_interrupt(doc: &mut Document, table: &ColorTable, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<LayerId>> {
    palette_poll(control)?;
    let required = palette_replacement_requirements(doc, table, control)?;
    let Some(state) = &doc.xcf_state else { return Ok(Vec::new()) };
    if state.editable_indexed.is_empty() {
        return Ok(Vec::new());
    }
    let mut layer_count = 0;
    palette_layer_count(&doc.layers, 0, &mut layer_count, control)?;
    let mut inventory = palette_reserve(layer_count)?;
    palette_inventory(&doc.layers, &mut inventory, control)?;
    let mut updates: Vec<PaletteReplacementUpdate> = palette_reserve(state.editable_indexed.len())?;
    let mut ids = palette_reserve::<LayerId>(state.editable_indexed.len())?;
    let mut order = palette_reserve::<(LayerId, usize)>(state.editable_indexed.len())?;
    for (indexed, item) in state.editable_indexed.iter().enumerate() {
        palette_poll(control)?;
        let Some(layer) = palette_layer(&doc.layers, item.owner, control)? else { continue };
        let LayerContent::Raster(surface) = &layer.content else { return Err(error("indexed palette owner is not resident raster")) };
        let mut palette = palette_reserve(table.colors.len())?;
        palette.extend_from_slice(&table.colors);
        let mut removed = false;
        for chunk in item.raster.indices().chunks(4096) {
            palette_poll(control)?;
            removed |= chunk.iter().any(|index| usize::from(*index) >= table.colors.len());
        }
        let indices = if removed {
            let mut values = palette_reserve(item.raster.indices().len())?;
            for chunk in item.raster.indices().chunks(4096) {
                palette_poll(control)?;
                for &index in chunk {
                    values.push(if usize::from(index) < table.colors.len() {
                        index
                    } else {
                        let color = item.raster.palette().get(usize::from(index)).ok_or_else(|| error("palette ordinal"))?;
                        u8::try_from(table.nearest(color.map(|value| f32::from(value) / 255.0))).map_err(error)?
                    });
                }
            }
            Arc::new(values)
        } else {
            item.raster.indices().clone()
        };
        let raster = EditableIndexedRaster::try_new_with_interrupt(item.raster.bounds(), indices, item.raster.alpha().cloned(), palette, control)
            .map_err(|failure| palette_failure(control, failure))?;
        let carrier = palette_carrier(&raster, surface.format(), table.transparent, control)?;
        let position = order.binary_search_by_key(&item.owner, |entry| entry.0).unwrap_or_else(|position| position);
        order.insert(position, (item.owner, updates.len()));
        ids.push(item.owner);
        updates.push(PaletteReplacementUpdate { owner: item.owner, indexed, raster: Some(raster), surface: Some(carrier) });
    }
    let _ = required;
    fn publish(layers: &mut [Layer], order: &[(LayerId, usize)], updates: &mut [PaletteReplacementUpdate]) {
        for layer in layers {
            if let Ok(position) = order.binary_search_by_key(&layer.id, |entry| entry.0)
                && let Some(update) = order.get(position).and_then(|entry| updates.get_mut(entry.1))
                && let Some(surface) = update.surface.take()
            {
                layer.content = LayerContent::Raster(surface);
            }
            if let LayerContent::Group(group) = &mut layer.content {
                publish(&mut group.children, order, updates);
            }
        }
    }
    // Rehearse all publication locations with immutable borrows first.
    for update in &updates {
        palette_poll(control)?;
        if !matches!(palette_layer(&doc.layers, update.owner, control)?.map(|layer| &layer.content), Some(LayerContent::Raster(_)))
            || !state.editable_indexed.get(update.indexed).is_some_and(|item| item.owner == update.owner)
        {
            return Err(error("palette publication target changed"));
        }
    }
    palette_poll(control)?;
    publish(&mut doc.layers, &order, &mut updates);
    if let Some(state) = &mut doc.xcf_state {
        for update in &mut updates {
            if let Some(raster) = update.raster.take()
                && let Some(item) = state.editable_indexed.get_mut(update.indexed)
            {
                item.raster = raster;
            }
        }
    }
    Ok(ids)
}

#[cfg(test)]
fn palette_replacement_fixture() -> (Document, LayerId) {
    let raster = EditableIndexedRaster::try_new(
        Rect::new(-1, 0, 2, 1),
        Arc::new(vec![0, 1, 2]),
        Some(Arc::new(vec![255, 128, 0])),
        vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]],
    )
    .unwrap();
    let surface = expanded(&raster, PixelFormat::RGBA8, Some(2)).unwrap();
    let mut doc = Document::new("Own palette replacement", photocraft_doc::Size::new(2, 2), ColorMode::Indexed, SampleType::U8);
    doc.color_table = Some(ColorTable { colors: raster.palette().to_vec(), transparent: Some(2) });
    let mut layer = Layer::new("Own ordinals", LayerContent::Raster(surface.clone()));
    layer.indexed_source = Some(
        photocraft_doc::IndexedSource::new(raster.indices().clone(), raster.alpha().cloned(), raster.palette().to_vec(), raster.bounds(), &surface).unwrap(),
    );
    let owner = layer.id;
    doc.layers.push(layer);
    let mut state = photocraft_doc::XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(raster.bounds());
    state.editable_indexed.push(EditableIndexedLayer { owner, raster });
    doc.xcf_state = Some(state);
    (doc, owner)
}
#[cfg(test)]
#[test]
fn controlled_palette_retains_duplicate_ordinals_and_removes_only_missing_entries() {
    let (source, owner) = palette_replacement_fixture();
    let previous = &source.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
    for (colors, expected) in [(vec![[0, 0, 0], [255, 255, 255], [0, 0, 255]], vec![0, 1, 2]), (vec![[255, 0, 0], [255, 0, 0]], vec![0, 1, 0])] {
        let mut doc = source.clone();
        let table = ColorTable { colors, transparent: None };
        assert_eq!(replace_palette_with_interrupt(&mut doc, &table, &photocraft_raster::Interrupt::NONE).unwrap(), vec![owner]);
        let current = &doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert_eq!(current.indices().as_slice(), expected);
        assert!(Arc::ptr_eq(current.alpha().unwrap(), previous.alpha().unwrap()));
        if table.colors.len() == 3 {
            assert!(Arc::ptr_eq(current.indices(), previous.indices()));
        }
        assert_eq!(doc.layers[0].indexed_source, source.layers[0].indexed_source);
        assert_eq!(doc.color_table, source.color_table, "helper does not replace the caller's table");
        assert_eq!(
            doc.layers[0].surface().unwrap().try_read_interleaved_with(current.bounds(), &photocraft_raster::Interrupt::NONE).unwrap(),
            expanded(current, PixelFormat::RGBA8, None).unwrap().try_read_interleaved_with(current.bounds(), &photocraft_raster::Interrupt::NONE).unwrap()
        );
    }
}
#[cfg(test)]
#[test]
fn controlled_palette_cancellation_and_bad_late_owner_do_not_publish_prefix() {
    let (mut doc, owner) = palette_replacement_fixture();
    let before = doc.clone();
    let table = ColorTable { colors: vec![[0, 0, 0], [255, 255, 255]], transparent: None };
    let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
    assert!(matches!(replace_palette_with_interrupt(&mut doc, &table, &cancelled), Err(EngineError::Cancelled)));
    assert_eq!(doc, before);
    let duplicate = doc.xcf_state.as_ref().unwrap().editable_indexed[0].clone();
    doc.xcf_state.as_mut().unwrap().editable_indexed.push(duplicate);
    let invalid = doc.clone();
    assert!(replace_palette_with_interrupt(&mut doc, &table, &photocraft_raster::Interrupt::NONE).is_err());
    assert_eq!(doc, invalid);
    assert_eq!(doc.layers[0].id, owner);
}

/// Explicit whole-pixel move hook. Call before moving expanded pixels, including group children.
/// Current indexed extents and linked canonical mask bounds/offsets shift transactionally.
pub fn translate_indexed(doc: &mut Document, root: LayerId, dx: i32, dy: i32) -> Result<()> {
    if doc.xcf_state.is_none() {
        return Ok(());
    }
    let layers = layer_inventory(doc)?;
    let root_layer = layers.get(&root).copied().ok_or(EngineError::NoLayer(root))?;
    let mut pending = vec![root_layer];
    let mut ids = Vec::new();
    while let Some(layer) = pending.pop() {
        ids.push(layer.id);
        if ids.len() > 10000 {
            return Err(error("layer count"));
        }
        if let LayerContent::Group(group) = &layer.content {
            pending.extend(&group.children);
        }
    }
    let linked_masks: Vec<_> =
        ids.iter().filter(|id| layers.get(id).is_some_and(|layer| layer.mask.as_ref().is_some_and(|mask| mask.linked))).copied().collect();
    let Some(state) = doc.xcf_state.as_mut() else {
        return Ok(());
    };
    let mut mask_updates = Vec::new();
    let mut extent_updates = Vec::new();
    for (n, item) in state.items.iter().enumerate() {
        if let Some(bounds) = item.raster_bounds
            && matches!(item.item, ItemRef::Layer(id) if ids.contains(&id))
        {
            let add = |a: i32, b: i32| a.checked_add(b).ok_or_else(|| error("drawable coordinate overflow"));
            let shifted = Rect::new(add(bounds.x0, dx)?, add(bounds.y0, dy)?, add(bounds.x1, dx)?, add(bounds.y1, dy)?);
            extent_updates.push((n, shifted));
        }
    }
    for (n, item) in state.items.iter().enumerate() {
        if !state.filter_masks.iter().any(|mask| item.item == ItemRef::Mask(mask.id))
            && let Some(mask) = &item.mask
            && matches!(mask.owner, ItemRef::Layer(id) if linked_masks.contains(&id))
        {
            let mut moved = mask.clone();
            let add = |a: i32, b: i32| a.checked_add(b).ok_or_else(|| error("mask coordinate overflow"));
            moved.bounds = Rect::new(add(mask.bounds.x0, dx)?, add(mask.bounds.y0, dy)?, add(mask.bounds.x1, dx)?, add(mask.bounds.y1, dy)?);
            moved.offset = [add(mask.offset[0], dx)?, add(mask.offset[1], dy)?];
            mask_updates.push((n, moved));
        }
    }
    let mut updates = Vec::new();
    for (n, item) in state.editable_indexed.iter().enumerate().filter(|(_, i)| ids.contains(&i.owner)) {
        let mut raster = item.raster.clone();
        raster.translate_bounds(dx, dy).map_err(error)?;
        updates.push((n, raster));
    }
    for (n, bounds) in extent_updates {
        state.items.get_mut(n).ok_or_else(|| error("drawable owner"))?.raster_bounds = Some(bounds);
    }
    for (n, mask) in mask_updates {
        state.items.get_mut(n).ok_or_else(|| error("mask owner"))?.mask = Some(mask);
    }
    for (n, raster) in updates {
        state.editable_indexed.get_mut(n).ok_or_else(|| error("indexed owner"))?.raster = raster;
    }
    Ok(())
}

/// Clone a newly inserted layer/group using explicit bounded source/target tree pairing.
pub fn clone_duplicate(doc: &mut Document, source: LayerId, target: LayerId) -> Result<()> {
    let Some(mut state) = doc.xcf_state.take() else {
        return Ok(());
    };
    let outcome = (|| {
        let layers = layer_inventory(doc)?;
        let mut pairs = BTreeMap::new();
        let mut pending = vec![(
            layers.get(&source).copied().ok_or(EngineError::NoLayer(source))?,
            layers.get(&target).copied().ok_or(EngineError::NoLayer(target))?,
            0usize,
        )];
        while let Some((a, b, depth)) = pending.pop() {
            if depth > 100 || pairs.len() >= 10000 || pairs.insert(a.id, b.id).is_some() {
                return Err(error("duplicate layer tree"));
            }
            match (&a.content, &b.content) {
                (LayerContent::Group(a), LayerContent::Group(b)) if a.children.len() == b.children.len() => {
                    pending.extend(a.children.iter().zip(&b.children).map(|(a, b)| (a, b, depth + 1)))
                }
                (LayerContent::Group(_), _) | (_, LayerContent::Group(_)) => return Err(error("duplicate hierarchy")),
                _ => {}
            }
        }
        let mut masks = BTreeMap::new();
        let mut filters = BTreeMap::new();
        let mut mask_id = state.items.iter().filter_map(|i| if let ItemRef::Mask(id) = i.item { Some(id.0) } else { None }).max().unwrap_or(0);
        let mut filter_id = state.filter_stacks.iter().flat_map(|s| &s.nodes).map(|n| n.id.0).max().unwrap_or(0);
        for item in &state.items {
            if let ItemRef::Mask(id) = item.item
                && !state.filter_masks.iter().any(|mask| mask.id == id)
                && item.mask.as_ref().is_some_and(|m| matches!(m.owner,ItemRef::Layer(id) if pairs.contains_key(&id)))
            {
                mask_id = mask_id.checked_add(1).ok_or_else(|| error("mask IDs exhausted"))?;
                masks.insert(id, MaskId(mask_id));
            }
        }
        for stack in state.filter_stacks.iter().filter(|s| matches!(s.owner,ItemRef::Layer(id) if pairs.contains_key(&id))) {
            for node in &stack.nodes {
                filter_id = filter_id.checked_add(1).ok_or_else(|| error("filter IDs exhausted"))?;
                filters.insert(node.id, FilterId(filter_id));
            }
        }
        for mask in state.filter_masks.iter().filter(|mask| filters.contains_key(&mask.owner)) {
            mask_id = mask_id.checked_add(1).ok_or_else(|| error("mask IDs exhausted"))?;
            masks.insert(mask.id, MaskId(mask_id));
        }
        state.clone_layers_metadata(&pairs, doc, &masks, &filters).map_err(error)
    })();
    doc.xcf_state = Some(state);
    outcome
}

/// Controlled metadata pairing for an already-created private native clone.
/// This does not make higher legacy duplication commands asynchronous.
pub fn clone_duplicate_with_interrupt(doc: &mut Document, source: LayerId, target: LayerId, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    palette_poll(control)?;
    let Some(state) = doc.xcf_state.as_ref() else {
        return Ok(());
    };
    let mut layer_count = 0;
    palette_layer_count(&doc.layers, 0, &mut layer_count, control)?;
    if state.items.len() > 10_000 || state.filter_stacks.len() > 10_000 || state.filter_masks.len() > 10_000 {
        return Err(error("duplicate metadata limit"));
    }
    let mut nodes = 0usize;
    for stack in &state.filter_stacks {
        palette_poll(control)?;
        nodes = nodes.checked_add(stack.nodes.len()).filter(|n| *n <= 100_000).ok_or_else(|| error("duplicate filter limit"))?;
    }
    // All map/stack inventories have a closed envelope before their first
    // allocation; doc's helper separately admits cloned model/shared payload.
    let map_bytes = (layer_count as u64)
        .checked_mul(256)
        .and_then(|n| n.checked_add((state.items.len() as u64 + state.filter_masks.len() as u64 + nodes as u64).checked_mul(256)?))
        .ok_or_else(|| error("duplicate planning limit"))?;
    let work = (state.items.len() as u64)
        .checked_mul(state.filter_masks.len() as u64)
        .and_then(|n| n.checked_add((layer_count as u64).checked_mul(layer_count as u64)?))
        .and_then(|n| n.checked_add(nodes as u64))
        .ok_or_else(|| error("duplicate planning limit"))?;
    if map_bytes > 64 << 20 || work > 8_000_000_000 {
        return Err(error("duplicate planning limit"));
    }
    let mut inventory = Vec::new();
    inventory.try_reserve_exact(layer_count).map_err(|_| error("duplicate inventory allocation"))?;
    if inventory.capacity().checked_mul(std::mem::size_of::<&Layer>()).is_none_or(|n| n > 64 << 20) {
        return Err(error("duplicate inventory allocation"));
    }
    palette_inventory(&doc.layers, &mut inventory, control)?;
    let find = |id| inventory.binary_search_by_key(&id, |layer| layer.id).ok().and_then(|index| inventory.get(index).copied()).ok_or(EngineError::NoLayer(id));
    let mut pending = Vec::new();
    pending.try_reserve_exact(layer_count).map_err(|_| error("duplicate pairing allocation"))?;
    if pending.capacity().checked_mul(std::mem::size_of::<(&Layer, &Layer, usize)>()).is_none_or(|n| n > 64 << 20) {
        return Err(error("duplicate pairing allocation"));
    }
    pending.push((find(source)?, find(target)?, 0usize));
    let mut pairs = BTreeMap::new();
    while let Some((a, b, depth)) = pending.pop() {
        palette_poll(control)?;
        if depth > 100 || pairs.len() >= 10_000 || pairs.insert(a.id, b.id).is_some() {
            return Err(error("duplicate layer tree"));
        }
        match (&a.content, &b.content) {
            (LayerContent::Group(a), LayerContent::Group(b)) if a.children.len() == b.children.len() => {
                for (a, b) in a.children.iter().zip(&b.children) {
                    palette_poll(control)?;
                    if pending.len() >= pending.capacity() {
                        return Err(error("duplicate pairing capacity"));
                    }
                    pending.push((a, b, depth + 1));
                }
            }
            (LayerContent::Group(_), _) | (_, LayerContent::Group(_)) => return Err(error("duplicate hierarchy")),
            _ => {}
        }
    }
    let mut masks = BTreeMap::new();
    let mut filters = BTreeMap::new();
    let mut mask_id = 0u64;
    let mut filter_id = 0u64;
    for item in &state.items {
        palette_poll(control)?;
        if let ItemRef::Mask(id) = item.item {
            mask_id = mask_id.max(id.0);
        }
    }
    for stack in &state.filter_stacks {
        palette_poll(control)?;
        for node in &stack.nodes {
            palette_poll(control)?;
            filter_id = filter_id.max(node.id.0);
        }
    }
    for item in &state.items {
        palette_poll(control)?;
        if let ItemRef::Mask(id) = item.item
            && item.mask.as_ref().is_some_and(|mask| matches!(mask.owner,ItemRef::Layer(id) if pairs.contains_key(&id)))
        {
            let mut filter_mask = false;
            for mask in &state.filter_masks {
                palette_poll(control)?;
                filter_mask |= mask.id == id;
            }
            if !filter_mask {
                mask_id = mask_id.checked_add(1).ok_or_else(|| error("mask IDs exhausted"))?;
                masks.insert(id, MaskId(mask_id));
            }
        }
    }
    for stack in &state.filter_stacks {
        palette_poll(control)?;
        if matches!(stack.owner,ItemRef::Layer(id) if pairs.contains_key(&id)) {
            for node in &stack.nodes {
                palette_poll(control)?;
                filter_id = filter_id.checked_add(1).ok_or_else(|| error("filter IDs exhausted"))?;
                filters.insert(node.id, FilterId(filter_id));
            }
        }
    }
    for mask in &state.filter_masks {
        palette_poll(control)?;
        if filters.contains_key(&mask.owner) {
            mask_id = mask_id.checked_add(1).ok_or_else(|| error("mask IDs exhausted"))?;
            masks.insert(mask.id, MaskId(mask_id));
        }
    }
    palette_poll(control)?;
    // End all native borrows before taking metadata. The doc method is atomic
    // on every error, and the Option is restored on both outcomes.
    drop(inventory);
    drop(pending);
    let Some(mut state) = doc.xcf_state.take() else {
        return Ok(());
    };
    let outcome = state.clone_layers_metadata_with_interrupt(&pairs, doc, &masks, &filters, control).map_err(|failure| palette_failure(control, failure));
    doc.xcf_state = Some(state);
    outcome
}

/// Keep current imported lock flags in step with real canonical lock changes.
/// Unchanged effective components preserve distinct imported state; visibility and raw
/// source flags are independent. Updating an ancestor leaves descendant inheritance live.
pub fn synchronize_layer_locks(before: &Document, doc: &mut Document) -> Result<()> {
    if doc.xcf_state.is_none() {
        return Ok(());
    }
    let previous = layer_inventory(before)?;
    let current = layer_inventory(doc)?;
    let mut updates = Vec::new();
    for (&id, layer) in &current {
        let Some(previous) = previous.get(&id) else {
            continue;
        };
        let old = previous.locks;
        let new = layer.locks;
        let content = ((old.pixels || old.all) != (new.pixels || new.all)).then_some(new.pixels || new.all);
        let position = ((old.position || old.all) != (new.position || new.all)).then_some(new.position || new.all);
        let alpha = ((old.transparency || old.all) != (new.transparency || new.all)).then_some(new.transparency || new.all);
        if content.is_some() || position.is_some() || alpha.is_some() {
            updates.push((id, content, position, alpha));
        }
    }
    let state = doc.xcf_state.as_mut().ok_or_else(|| EngineError::Other("imported state disappeared during lock synchronization".into()))?;
    for (id, content, position, alpha) in updates {
        if let Some(item) = state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)) {
            if let Some(value) = content {
                item.locks.content = value;
            }
            if let Some(value) = position {
                item.locks.position = value;
            }
            if let Some(value) = alpha {
                item.locks.alpha = value;
            }
        }
    }
    Ok(())
}

/// Synchronize only native lock components that actually changed. The plan
/// contains scalar ItemLocks and item indices; source/cache/payload metadata is
/// neither copied nor interpreted. Final bounded publication cannot fail.
pub fn synchronize_layer_locks_with_interrupt(before: &Document, doc: &mut Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    palette_poll(control)?;
    let Some(state) = doc.xcf_state.as_ref() else {
        return Ok(());
    };
    if state.items.len() > 10_000 {
        return Err(error("lock synchronization item limit"));
    }
    let mut previous_count = 0;
    let mut current_count = 0;
    palette_layer_count(&before.layers, 0, &mut previous_count, control)?;
    palette_layer_count(&doc.layers, 0, &mut current_count, control)?;
    // Count both borrowed layer indexes, the typed-identity set and every
    // prospective scalar update before allocating any collection. No raster or
    // retained source payload allocation is needed by this operation.
    let bytes = (previous_count as u64)
        .checked_add(current_count as u64)
        .and_then(|n| n.checked_mul(std::mem::size_of::<&Layer>() as u64))
        .and_then(|n| n.checked_add((state.items.len() as u64).checked_mul(96)?))
        .and_then(|n| n.checked_add((current_count as u64).checked_mul(std::mem::size_of::<(usize, photocraft_doc::xcf_state::ItemLocks)>() as u64)?))
        .ok_or_else(|| error("lock synchronization allocation limit"))?;
    let work = (previous_count as u64)
        .checked_mul(previous_count as u64)
        .and_then(|n| n.checked_add((current_count as u64).checked_mul(current_count as u64)?))
        .and_then(|n| n.checked_add((current_count as u64).checked_mul(state.items.len() as u64)?))
        .and_then(|n| n.checked_add((state.items.len() as u64).checked_mul(4)?))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| error("lock synchronization work limit"))?;
    if bytes > 8 << 20 || work > 8_000_000_000 {
        return Err(error("lock synchronization limit"));
    }
    let mut previous: Vec<&Layer> = Vec::new();
    let mut current: Vec<&Layer> = Vec::new();
    let mut updates: Vec<(usize, photocraft_doc::xcf_state::ItemLocks)> = Vec::new();
    let mut identities: std::collections::HashSet<ItemRef> = std::collections::HashSet::new();
    previous.try_reserve_exact(previous_count).map_err(|_| error("lock synchronization allocation"))?;
    current.try_reserve_exact(current_count).map_err(|_| error("lock synchronization allocation"))?;
    updates.try_reserve_exact(current_count).map_err(|_| error("lock synchronization allocation"))?;
    identities.try_reserve(state.items.len()).map_err(|_| error("lock synchronization allocation"))?;
    let actual = (previous.capacity() as u64)
        .checked_add(current.capacity() as u64)
        .and_then(|n| n.checked_mul(std::mem::size_of::<&Layer>() as u64))
        .and_then(|n| n.checked_add((updates.capacity() as u64).checked_mul(std::mem::size_of::<(usize, photocraft_doc::xcf_state::ItemLocks)>() as u64)?))
        .and_then(|n| n.checked_add((identities.capacity() as u64).checked_mul(64)?))
        .ok_or_else(|| error("lock synchronization allocation limit"))?;
    if actual > 8 << 20 {
        return Err(error("lock synchronization allocation limit"));
    }
    palette_inventory(&before.layers, &mut previous, control)?;
    palette_inventory(&doc.layers, &mut current, control)?;
    for item in &state.items {
        palette_poll(control)?;
        if item.item.raw() == 0 || identities.len() >= identities.capacity() || !identities.insert(item.item) {
            return Err(error("lock synchronization typed identity"));
        }
    }
    for layer in &current {
        palette_poll(control)?;
        let Ok(index) = previous.binary_search_by_key(&layer.id, |layer| layer.id) else {
            continue;
        };
        let old = previous.get(index).ok_or_else(|| error("lock synchronization index"))?.locks;
        let new = layer.locks;
        let content = ((old.pixels || old.all) != (new.pixels || new.all)).then_some(new.pixels || new.all);
        let position = ((old.position || old.all) != (new.position || new.all)).then_some(new.position || new.all);
        let alpha = ((old.transparency || old.all) != (new.transparency || new.all)).then_some(new.transparency || new.all);
        if content.is_none() && position.is_none() && alpha.is_none() {
            continue;
        }
        for (index, item) in state.items.iter().enumerate() {
            palette_poll(control)?;
            if item.item != ItemRef::Layer(layer.id) {
                continue;
            }
            let mut locks = item.locks;
            if let Some(value) = content {
                locks.content = value;
            }
            if let Some(value) = position {
                locks.position = value;
            }
            if let Some(value) = alpha {
                locks.alpha = value;
            }
            let position = updates.binary_search_by_key(&index, |(index, _)| *index).unwrap_or_else(|position| position);
            if updates.len() >= updates.capacity() {
                return Err(error("lock synchronization update capacity"));
            }
            updates.insert(position, (index, locks));
            break;
        }
    }
    drop(previous);
    drop(current);
    drop(identities);
    // No callback, allocation, fallible lookup or cancellation after the last
    // fence. Item indices cannot change between planning and this commit.
    palette_poll(control)?;
    let state = doc.xcf_state.as_mut().ok_or_else(|| error("imported state disappeared during lock synchronization"))?;
    let mut updates = updates.into_iter().peekable();
    for (index, item) in state.items.iter_mut().enumerate() {
        if updates.peek().is_some_and(|(planned, _)| *planned == index)
            && let Some((_, locks)) = updates.next()
        {
            item.locks = locks;
        }
    }
    Ok(())
}

#[cfg(test)]
mod controlled_lock_sync_tests {
    use super::*;
    fn fixture() -> Document {
        let mut doc = Document::new("Own lock sync", photocraft_doc::Size::new(2, 2), ColorMode::Rgb, SampleType::F32);
        doc.layers = vec![Layer::group("Parent", vec![Layer::raster("Child", doc.pixel_format())])];
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        for item in &mut state.items {
            item.locks.content = true;
            item.locks.visibility = true;
            item.source_flags = 0x0102_0304;
            item.source_tattoo = Some(42);
        }
        doc.xcf_state = Some(state);
        doc
    }
    #[test]
    fn controlled_lock_sync_matches_legacy_without_reinterpreting_inheritance_or_source_flags() {
        let before = fixture();
        let mut doc = before.clone();
        doc.layers[0].locks.all = true;
        let mut expected = doc.clone();
        synchronize_layer_locks(&before, &mut expected).unwrap();
        synchronize_layer_locks_with_interrupt(&before, &mut doc, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(doc, expected);
        let state = doc.xcf_state.as_ref().unwrap();
        assert!(state.items.iter().all(|item| item.source_flags == 0x0102_0304 && item.source_tattoo == Some(42) && item.locks.visibility));
    }
    #[test]
    fn late_lock_plan_cancel_and_duplicate_typed_identity_do_not_mutate() {
        let before = fixture();
        let mut doc = before.clone();
        doc.layers[0].locks.position = true;
        let expected = doc.clone();
        let calls = std::sync::atomic::AtomicUsize::new(0usize);
        let observe = || {
            calls.store(calls.load(std::sync::atomic::Ordering::Relaxed) + 1, std::sync::atomic::Ordering::Relaxed);
            false
        };
        let mut rehearsal = expected.clone();
        synchronize_layer_locks_with_interrupt(&before, &mut rehearsal, &photocraft_raster::Interrupt::cancel_only(&observe)).unwrap();
        let last_fence = calls.load(std::sync::atomic::Ordering::Relaxed);
        assert!(last_fence > 1);
        calls.store(0, std::sync::atomic::Ordering::Relaxed);
        let cancel = || {
            let next = calls.load(std::sync::atomic::Ordering::Relaxed) + 1;
            calls.store(next, std::sync::atomic::Ordering::Relaxed);
            next >= last_fence
        };
        let control = photocraft_raster::Interrupt::cancel_only(&cancel);
        assert!(matches!(synchronize_layer_locks_with_interrupt(&before, &mut doc, &control), Err(EngineError::Cancelled)));
        assert_eq!(doc, expected);
        let mut doc = expected.clone();
        let state = doc.xcf_state.as_mut().unwrap();
        state.items.push(state.items[0].clone());
        let expected = doc.clone();
        assert!(synchronize_layer_locks_with_interrupt(&before, &mut doc, &photocraft_raster::Interrupt::NONE).is_err());
        assert_eq!(doc, expected);
    }
    #[test]
    fn equal_numeric_channel_identity_is_not_a_layer_lock_target() {
        let before = fixture();
        let mut doc = before.clone();
        let id = doc.layers[0].id;
        let mut metadata = doc.xcf_state.as_ref().unwrap().items[0].clone();
        metadata.item = ItemRef::Channel(photocraft_doc::item_identity::ChannelId(id.0));
        metadata.locks = Default::default();
        doc.xcf_state.as_mut().unwrap().items.push(metadata.clone());
        doc.layers[0].locks.all = true;
        synchronize_layer_locks_with_interrupt(&before, &mut doc, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(doc.xcf_state.as_ref().unwrap().items.last().unwrap(), &metadata);
    }
}

#[path = "xcf_relationship_targets.rs"]
mod relationship_targets;

fn relationship_bad(cmd: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn relationship_params(cmd: &str, params: &serde_json::Value, allowed: &[&str]) -> Result<()> {
    if params.is_null() && allowed.is_empty() {
        return Ok(());
    }
    let object = params.as_object().ok_or_else(|| relationship_bad(cmd, "expected an object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(relationship_bad(cmd, "unknown relationship parameter"));
    }
    Ok(())
}
fn imported_state(s: &crate::Session) -> std::result::Result<(), String> {
    s.active().and_then(|state| state.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no imported editing state".into())
}
fn floating_enabled(s: &crate::Session) -> std::result::Result<(), String> {
    let document = &s.active().ok_or("no document open")?.doc;
    let state = document.xcf_state.as_ref().ok_or("no imported editing state")?;
    let attachment = state.floating.as_ref().ok_or("no saved floating attachment")?;
    document.layer(attachment.layer).ok_or("missing floating layer")?;
    let effective = crate::xcf_item_locks::effective_layer_locks(document, attachment.layer).map_err(|e| e.to_string())?;
    let canonical = effective.native;
    let imported = effective.imported;
    if canonical.all || canonical.pixels || imported.content { Err("the floating layer's content is locked".into()) } else { Ok(()) }
}
fn detach_floating(s: &mut crate::Session, params: &serde_json::Value) -> Result<serde_json::Value> {
    const CMD: &str = "xcf.floating.detach";
    relationship_params(CMD, params, &[])?;
    floating_enabled(s).map_err(EngineError::Other)?;
    let attachment = s.edit("Detach Floating Layer", |document, _| {
        document.xcf_state.as_mut().and_then(|state| state.floating.take()).ok_or_else(|| relationship_bad(CMD, "missing floating attachment"))
    })?;
    Ok(serde_json::json!({"layer":attachment.layer.0,"detached":true,"previousTarget":attachment.target}))
}
fn relationship_id(cmd: &str, value: &serde_json::Value) -> Result<u64> {
    let raw = if let Some(text) = value.as_str() {
        if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(relationship_bad(cmd, "expected canonical nonzero decimal identity"));
        }
        text.parse::<u64>().map_err(|_| relationship_bad(cmd, "canonical identity overflow"))?
    } else {
        value.as_u64().ok_or_else(|| relationship_bad(cmd, "expected nonzero integer or canonical decimal identity string"))?
    };
    if raw == 0 {
        return Err(relationship_bad(cmd, "zero canonical identity"));
    }
    Ok(raw)
}
fn set_id(cmd: &str, params: &serde_json::Value) -> Result<photocraft_doc::item_identity::ItemSetId> {
    let value = params.get("setId").ok_or_else(|| relationship_bad(cmd, "missing setId"))?;
    relationship_id(cmd, value).map(photocraft_doc::item_identity::ItemSetId)
}
fn member_reference(cmd: &str, value: &serde_json::Value) -> Result<ItemRef> {
    let object = value.as_object().ok_or_else(|| relationship_bad(cmd, "invalid typed item reference"))?;
    let (family, raw) = if object.len() == 1 {
        let (family, value) = object.iter().next().ok_or_else(|| relationship_bad(cmd, "missing typed item reference"))?;
        let family = match family.as_str() {
            "Layer" => "layer",
            "Channel" => "channel",
            "Path" => "path",
            _ => return Err(relationship_bad(cmd, "invalid member family")),
        };
        (family, relationship_id(cmd, value)?)
    } else if object.len() == 2 && object.contains_key("kind") && object.contains_key("id") {
        let family = object.get("kind").and_then(serde_json::Value::as_str).ok_or_else(|| relationship_bad(cmd, "invalid member kind"))?;
        let value =
            object.get("id").filter(|value| value.is_string()).ok_or_else(|| relationship_bad(cmd, "inspection member id must be a canonical string"))?;
        (family, relationship_id(cmd, value)?)
    } else {
        return Err(relationship_bad(cmd, "invalid typed item reference keys"));
    };
    match family {
        "layer" => Ok(ItemRef::Layer(LayerId(raw))),
        "channel" => Ok(ItemRef::Channel(photocraft_doc::item_identity::ChannelId(raw))),
        "path" => Ok(ItemRef::Path(photocraft_doc::item_identity::PathId(raw))),
        _ => Err(relationship_bad(cmd, "invalid member family")),
    }
}
fn manage_set_context(s: &crate::Session, cmd: &str) -> Result<()> {
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.floating.is_some() {
        return Err(relationship_bad(cmd, "finish or cancel floating state before editing saved sets"));
    }
    // This closed resident metadata envelope preserves persisted attachment
    // identities. checked_manage_set validates the complete typed graph before
    // no-op/staging and again in the private edit; pixel previews remain closed.
    if s.command_is_background() || s.command_has_pending_job() || s.has_jobs() || s.coalesce_request.is_some() || s.color_restrict.is_some() {
        return Err(relationship_bad(cmd, "saved-set management requires synchronous isolated dispatch"));
    }
    if active.revision == u64::MAX {
        return Err(relationship_bad(cmd, "revision headroom exhausted"));
    }
    active.doc.validate_resident().map_err(|_| relationship_bad(cmd, "resident metadata editing required"))?;
    Ok(())
}
fn captured_set_target(s: &crate::Session, cmd: &str, params: &serde_json::Value) -> Result<()> {
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => Ok(()),
        (Some(document), Some(revision)) => {
            let parse = |value: &serde_json::Value, zero: bool| -> Result<u64> {
                let text = value.as_str().ok_or_else(|| relationship_bad(cmd, "captured selectors must be canonical decimal strings"))?;
                if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) || text.len() > 1 && text.starts_with('0') {
                    return Err(relationship_bad(cmd, "captured selectors must be canonical decimal strings"));
                }
                let value = text.parse::<u64>().map_err(|_| relationship_bad(cmd, "captured selector overflow"))?;
                if value == 0 && !zero {
                    return Err(relationship_bad(cmd, "zero document identity"));
                }
                Ok(value)
            };
            let active = s.active().ok_or(EngineError::NoDocument)?;
            if active.doc.id.0 != parse(document, false)? || active.revision != parse(revision, true)? {
                return Err(relationship_bad(cmd, "stale captured target"));
            }
            Ok(())
        }
        _ => Err(relationship_bad(cmd, "documentId and expectedRevision must be supplied together")),
    }
}
/// Admit borrowed saved-set parameters before an implicit pixel-preview drop.
pub(crate) fn dispatch_set_preflight(s: &crate::Session, cmd: &str, params: &serde_json::Value) -> Result<()> {
    let allowed: &[&str] = match cmd {
        "xcf.itemSet.rename" => &["setId", "name", "documentId", "expectedRevision"],
        "xcf.itemSet.delete" => &["setId", "documentId", "expectedRevision"],
        "xcf.itemSet.setMembers" => &["setId", "members", "documentId", "expectedRevision"],
        _ => return Ok(()),
    };
    relationship_params(cmd, params, allowed)?;
    set_id(cmd, params)?;
    if matches!(cmd, "xcf.itemSet.rename" | "xcf.itemSet.delete" | "xcf.itemSet.setMembers") {
        manage_set_context(s, cmd)?;
        captured_set_target(s, cmd, params)?;
    }
    match cmd {
        "xcf.itemSet.rename" => {
            params
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| name.len() <= 1 << 20 && !name.contains('\0') && !name.trim().is_empty())
                .ok_or_else(|| relationship_bad(cmd, "expected bounded nonempty name"))?;
        }
        "xcf.itemSet.setMembers" => {
            let members = params
                .get("members")
                .and_then(serde_json::Value::as_array)
                .filter(|members| members.len() <= 10000)
                .ok_or_else(|| relationship_bad(cmd, "expected at most 10000 typed members"))?;
            for member in members {
                member_reference(cmd, member)?;
            }
        }
        _ => {}
    }
    if s.active().ok_or(EngineError::NoDocument)?.floating.is_some() {
        return Err(relationship_bad(cmd, "finish or cancel the transient floating preview before editing saved sets"));
    }
    Ok(())
}
fn checked_manage_set(
    document: &Document,
    id: photocraft_doc::item_identity::ItemSetId,
    cmd: &str,
    name_bytes: usize,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<usize> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let state = document.xcf_state.as_ref().ok_or_else(|| relationship_bad(cmd, "missing imported state"))?;
    let index = relationship_targets::set_index(state, id, cmd)?;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(document, control)?;
    // Raster/COW and saved-set structures only; not total Document heap/RSS.
    let mut structures = (state.item_sets.capacity() as u64)
        .checked_mul(std::mem::size_of::<photocraft_doc::xcf_editing::ItemSet>() as u64)
        .ok_or_else(|| relationship_bad(cmd, "saved-set COW limit"))?;
    for set in &state.item_sets {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if set.name.len() > 1 << 20 {
            return Err(relationship_bad(cmd, "saved name limit"));
        }
        structures = structures
            .checked_add(set.name.capacity() as u64)
            .and_then(|n| n.checked_add((set.members.capacity() as u64).checked_mul(std::mem::size_of::<ItemRef>() as u64)?))
            .ok_or_else(|| relationship_bad(cmd, "saved-set COW limit"))?;
    }
    let peak = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes)
        .and_then(|n| n.checked_add(structures.checked_mul(2)?))
        .and_then(|n| n.checked_add(name_bytes as u64))
        .ok_or_else(|| relationship_bad(cmd, "saved-set COW limit"))?;
    if peak > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(relationship_bad(cmd, "saved-set COW limit"));
    }
    document
        .validate_xcf_state_with(control)
        .map_err(|error| if error.code == "cancelled" { EngineError::Cancelled } else { relationship_bad(cmd, error.to_string()) })?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(index)
}
/// Optional captured selector pair strengthens the compatible legacy setId API.
/// Rule/name metadata is not evaluated; source records, members and selection stay exact.
pub fn rename_set_with_interrupt(s: &mut crate::Session, params: &serde_json::Value, control: &photocraft_raster::Interrupt<'_>) -> Result<serde_json::Value> {
    const CMD: &str = "xcf.itemSet.rename";
    dispatch_set_preflight(s, CMD, params)?;
    let id = set_id(CMD, params)?;
    let requested = params.get("name").and_then(serde_json::Value::as_str).ok_or_else(|| relationship_bad(CMD, "expected name"))?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let index = checked_manage_set(&active.doc, id, CMD, requested.len(), control)?;
    let previous = active.doc.xcf_state.as_ref().and_then(|state| state.item_sets.get(index)).ok_or_else(|| relationship_bad(CMD, "unknown setId"))?;
    let changed = previous.name != requested;
    let response = serde_json::json!({"setId":id.0,"setIdString":id.0.to_string(),"updated":changed});
    if !changed {
        control.check().map_err(|_| EngineError::Cancelled)?;
        return Ok(response);
    }
    let mut name = String::new();
    name.try_reserve_exact(requested.len()).map_err(|_| relationship_bad(CMD, "name allocation limit"))?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    name.push_str(requested);
    checked_manage_set(&active.doc, id, CMD, name.capacity(), control)?;
    let revision = active.revision;
    let baseline = active.doc.clone();
    let fence = baseline.downgrade();
    dispatch_set_preflight(s, CMD, params)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !fence.is_current(&active.doc) {
        return Err(relationship_bad(CMD, "stale saved-set snapshot"));
    }
    s.edit_with_interrupt("Rename Saved Item Set", control, move |document, _| {
        let current = checked_manage_set(document, id, CMD, name.capacity(), control)?;
        if current != index
            || document.xcf_state.as_ref().and_then(|state| state.item_sets.get(current))
                != baseline.xcf_state.as_ref().and_then(|state| state.item_sets.get(index))
        {
            return Err(relationship_bad(CMD, "stale saved-set target"));
        }
        document.xcf_state.as_mut().and_then(|state| state.item_sets.get_mut(current)).ok_or_else(|| relationship_bad(CMD, "unknown setId"))?.name = name;
        document
            .validate_xcf_state_with(control)
            .map_err(|error| if error.code == "cancelled" { EngineError::Cancelled } else { relationship_bad(CMD, error.to_string()) })?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        Ok(())
    })?;
    Ok(response)
}
fn rename_set(s: &mut crate::Session, params: &serde_json::Value) -> Result<serde_json::Value> {
    rename_set_with_interrupt(s, params, &photocraft_raster::Interrupt::NONE)
}
pub fn delete_set_with_interrupt(s: &mut crate::Session, params: &serde_json::Value, control: &photocraft_raster::Interrupt<'_>) -> Result<serde_json::Value> {
    const CMD: &str = "xcf.itemSet.delete";
    dispatch_set_preflight(s, CMD, params)?;
    let id = set_id(CMD, params)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let index = checked_manage_set(&active.doc, id, CMD, 0, control)?;
    let revision = active.revision;
    let baseline = active.doc.clone();
    let fence = baseline.downgrade();
    let response = serde_json::json!({"setId":id.0,"setIdString":id.0.to_string(),"deleted":true});
    dispatch_set_preflight(s, CMD, params)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !fence.is_current(&active.doc) {
        return Err(relationship_bad(CMD, "stale saved-set snapshot"));
    }
    s.edit_with_interrupt("Delete Saved Item Set", control, move |document, _| {
        let current = checked_manage_set(document, id, CMD, 0, control)?;
        if current != index
            || document.xcf_state.as_ref().and_then(|state| state.item_sets.get(current))
                != baseline.xcf_state.as_ref().and_then(|state| state.item_sets.get(index))
        {
            return Err(relationship_bad(CMD, "stale saved-set target"));
        }
        // ItemSetId has no other typed stored selectors in this model. Deleting
        // this record does not delete members or change current semantic selection.
        let state = document.xcf_state.as_mut().ok_or_else(|| relationship_bad(CMD, "missing imported state"))?;
        if current >= state.item_sets.len() {
            return Err(relationship_bad(CMD, "unknown setId"));
        }
        state.item_sets.remove(current);
        document
            .validate_xcf_state_with(control)
            .map_err(|error| if error.code == "cancelled" { EngineError::Cancelled } else { relationship_bad(CMD, error.to_string()) })?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        Ok(())
    })?;
    Ok(response)
}
fn delete_set(s: &mut crate::Session, params: &serde_json::Value) -> Result<serde_json::Value> {
    delete_set_with_interrupt(s, params, &photocraft_raster::Interrupt::NONE)
}
pub fn set_members_with_interrupt(s: &mut crate::Session, params: &serde_json::Value, control: &photocraft_raster::Interrupt<'_>) -> Result<serde_json::Value> {
    const CMD: &str = "xcf.itemSet.setMembers";
    dispatch_set_preflight(s, CMD, params)?;
    let id = set_id(CMD, params)?;
    let values = params
        .get("members")
        .and_then(serde_json::Value::as_array)
        .filter(|values| values.len() <= 10000)
        .ok_or_else(|| relationship_bad(CMD, "expected at most 10000 typed members"))?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let anticipated = values.len().checked_mul(std::mem::size_of::<ItemRef>() + 64).ok_or_else(|| relationship_bad(CMD, "membership allocation limit"))?;
    let index = checked_manage_set(&active.doc, id, CMD, anticipated, control)?;
    let mut members = Vec::new();
    members.try_reserve_exact(values.len()).map_err(|_| relationship_bad(CMD, "membership allocation limit"))?;
    let mut identities = std::collections::HashSet::new();
    identities.try_reserve(values.len()).map_err(|_| relationship_bad(CMD, "membership allocation limit"))?;
    for value in values {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let member = member_reference(CMD, value)?;
        if member.raw() == 0 || !identities.insert(member) {
            return Err(relationship_bad(CMD, "zero or duplicate member"));
        }
        members.push(member);
    }
    let state = active.doc.xcf_state.as_ref().ok_or_else(|| relationship_bad(CMD, "missing imported state"))?;
    relationship_targets::members(state, id, &members, CMD)?;
    let actual = members
        .capacity()
        .checked_mul(std::mem::size_of::<ItemRef>())
        .and_then(|n| n.checked_add(identities.capacity().checked_mul(64)?))
        .ok_or_else(|| relationship_bad(CMD, "membership allocation limit"))?;
    checked_manage_set(&active.doc, id, CMD, actual, control)?;
    let changed = state.item_sets.get(index).is_none_or(|set| set.members != members);
    let response = serde_json::json!({"setId":id.0,"setIdString":id.0.to_string(),"members":members.len(),"updated":changed});
    if !changed {
        control.check().map_err(|_| EngineError::Cancelled)?;
        return Ok(response);
    }
    drop(identities);
    let revision = active.revision;
    let baseline = active.doc.clone();
    let fence = baseline.downgrade();
    dispatch_set_preflight(s, CMD, params)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !fence.is_current(&active.doc) {
        return Err(relationship_bad(CMD, "stale saved-set snapshot"));
    }
    s.edit_with_interrupt("Change Saved Item Set Members", control, move |document, _| {
        let current = checked_manage_set(document, id, CMD, actual, control)?;
        let state = document.xcf_state.as_ref().ok_or_else(|| relationship_bad(CMD, "missing imported state"))?;
        relationship_targets::members(state, id, &members, CMD)?;
        if current != index || state.item_sets.get(current) != baseline.xcf_state.as_ref().and_then(|state| state.item_sets.get(index)) {
            return Err(relationship_bad(CMD, "stale saved-set target"));
        }
        document.xcf_state.as_mut().and_then(|state| state.item_sets.get_mut(current)).ok_or_else(|| relationship_bad(CMD, "unknown setId"))?.members = members;
        document
            .validate_xcf_state_with(control)
            .map_err(|error| if error.code == "cancelled" { EngineError::Cancelled } else { relationship_bad(CMD, error.to_string()) })?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        Ok(())
    })?;
    Ok(response)
}
fn set_members(s: &mut crate::Session, params: &serde_json::Value) -> Result<serde_json::Value> {
    set_members_with_interrupt(s, params, &photocraft_raster::Interrupt::NONE)
}

/// Agent-reachable imported relationship edits. No saved search expression is evaluated.
/// A saved floating attachment is never converted into the transient `select.float` preview.
pub fn relationship_specs() -> Vec<crate::commands::CommandSpec> {
    use crate::commands::CommandSpec;
    vec![
        CommandSpec {
            id: "xcf.floating.detach",
            label: "Detach Floating Layer",
            menu: &[],
            shortcut: None,
            params: r#"{} -> {layer, detached, previousTarget} (remove only the saved attachment; preserve registered layer pixels and identities)"#,
            enabled: floating_enabled,
            run: detach_floating,
            journal: true,
        },
        CommandSpec {
            id: "xcf.itemSet.rename",
            label: "Rename Saved Item Set",
            menu: &[],
            shortcut: None,
            params: r#"{setId:canonical nonzero decimal string|legacy integer,name:bounded string,documentId?:canonical string,expectedRevision?:canonical string} -> {setId:integer,setIdString:string} (preserve stable identity, family, rule and membership)"#,
            enabled: imported_state,
            run: rename_set,
            journal: true,
        },
        CommandSpec {
            id: "xcf.itemSet.delete",
            label: "Delete Saved Item Set",
            menu: &[],
            shortcut: None,
            params: r#"{setId:canonical nonzero decimal string|legacy integer,documentId?:canonical string,expectedRevision?:canonical string} -> {setId:integer,setIdString:string,deleted:true} (remove the saved set without changing its items or selection)"#,
            enabled: imported_state,
            run: delete_set,
            journal: true,
        },
        CommandSpec {
            id: "xcf.itemSet.setMembers",
            label: "Change Saved Item Set Members",
            menu: &[],
            shortcut: None,
            params: r#"{setId:canonical nonzero decimal string|legacy integer,documentId?:canonical string,expectedRevision?:canonical string,members:[{kind:"layer"|"channel"|"path",id:canonical string}|{Layer:id}|{Channel:id}|{Path:id}]} -> {setId:integer,setIdString:string,members:count} (legacy tagged IDs accept canonical strings or integers; Named sets only; explicit ordered membership)"#,
            enabled: imported_state,
            run: set_members,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use photocraft_doc::{Color, IndexedSource, Size, XcfState};
    use serde_json::json;
    fn setup(group: bool) -> (Session, LayerId, LayerId) {
        let palette = vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]];
        let bounds = Rect::new(-1, 1, 2, 2);
        let indices = Arc::new(vec![0, 1, 2]);
        let alpha = Some(Arc::new(vec![255, 128, 0]));
        let raster = EditableIndexedRaster::try_new(bounds, indices.clone(), alpha.clone(), palette.clone()).unwrap();
        let surface = expanded(&raster, PixelFormat::RGBA8, None).unwrap();
        let mut d = Document::with_background("own indexed", Size::new(3, 3), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let child = d.layers[0].id;
        d.layers[0].locks = Default::default();
        d.layers[0].content = LayerContent::Raster(surface.clone());
        d.layers[0].indexed_source = Some(IndexedSource::new(indices, alpha, palette.clone(), bounds, &surface).unwrap());
        d.mode = ColorMode::Indexed;
        d.color_table = Some(ColorTable { colors: palette, transparent: None });
        let root = if group {
            let group = Layer::group("own group", vec![d.layers.remove(0)]);
            let id = group.id;
            d.layers.push(group);
            id
        } else {
            child
        };
        let mut state = XcfState::default();
        state.reconcile_layers(&d).unwrap();
        state.editable_indexed.push(EditableIndexedLayer { owner: child, raster });
        d.xcf_state = Some(state);
        let mut s = Session::new();
        s.add_document(d, None).unwrap();
        s.select_layer(root).unwrap();
        (s, root, child)
    }
    fn current(s: &Session, owner: LayerId) -> &EditableIndexedRaster {
        &s.active().unwrap().doc.xcf_state.as_ref().unwrap().editable_indexed.iter().find(|i| i.owner == owner).unwrap().raster
    }
    #[test]
    fn palette_edits_use_duplicate_ordinals_in_nested_layers_and_undo_native() {
        let (mut s, _, child) = setup(true);
        let original = s.active().unwrap().doc.clone();
        s.execute("image.mode.colorTable", json!({"entries":{"1":"#00ff00"}})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layer(child).unwrap().surface().unwrap().rgba(-1, 1), [1.0, 0.0, 0.0, 1.0]);
        let p = doc.layer(child).unwrap().surface().unwrap().rgba(0, 1);
        assert_eq!(p.get(..3), Some([0.0, 1.0, 0.0].as_slice()));
        assert_eq!(current(&s, child).indices().as_slice(), &[0, 1, 2]);
        assert_eq!(current(&s, child).alpha().unwrap().as_slice(), &[255, 128, 0]);
        assert_eq!(doc.layer(child).unwrap().indexed_source, original.layer(child).unwrap().indexed_source);
        let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        let back = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert!(back == **doc);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc == original);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(current(&s, child).palette()[1], [0, 255, 0]);
    }

    #[test]
    fn indexed_synchronization_observed_cancellation_preserves_private_document_and_original_planes() {
        use photocraft_raster::Interrupt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (session, _, layer) = setup(false);
        let before = session.active().unwrap().doc.clone();
        let mut changed = (*before).clone();
        changed.layer_mut(layer).unwrap().surface_mut().unwrap().try_write_interleaved(Rect::new(-1, 1, 0, 2), &[0, 0, 255, 255]).unwrap();
        let retained = changed.clone();
        let source = retained.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices().clone();
        let measured = AtomicUsize::new(0);
        let measure = || {
            measured.fetch_add(1, Ordering::Relaxed);
            false
        };
        synchronize_indexed_with_interrupt(&before, &mut changed, &Interrupt::cancel_only(&measure)).unwrap();
        assert!(measured.load(Ordering::Relaxed) > 8);
        for threshold in 0..measured.load(Ordering::Relaxed) {
            let mut private = retained.clone();
            let calls = AtomicUsize::new(0);
            let stop = || calls.fetch_add(1, Ordering::Relaxed) >= threshold;
            let result = synchronize_indexed_with_interrupt(&before, &mut private, &Interrupt::cancel_only(&stop));
            if matches!(result, Err(EngineError::Cancelled)) {
                assert_eq!(private, retained);
                assert!(Arc::ptr_eq(private.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices(), &source));
            } else {
                assert!(result.is_ok());
            }
        }
        assert_eq!(before.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices().as_slice(), &[0, 1, 2]);
    }
    #[test]
    fn explicit_duplicate_ordinal_proposal_keeps_exact_indices_alpha_and_carrier() {
        let (session, _, owner) = setup(false);
        let before = session.active().unwrap().doc.clone();
        let mut proposed = (*before).clone();
        let indexed = proposed.xcf_state.as_mut().unwrap().editable_indexed.iter_mut().find(|item| item.owner == owner).unwrap();
        indexed.raster.try_set_pixel(0, 1, 0, 128).unwrap();
        let requested = indexed.raster.clone();
        let before_sync = proposed.clone();
        synchronize_indexed_with_interrupt(&before, &mut proposed, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(proposed, before_sync);
        let after = &proposed.xcf_state.as_ref().unwrap().editable_indexed.iter().find(|item| item.owner == owner).unwrap().raster;
        assert!(Arc::ptr_eq(after.indices(), requested.indices()));
        assert!(Arc::ptr_eq(after.alpha().unwrap(), requested.alpha().unwrap()));
        assert_eq!(after.indices().as_slice(), &[0, 0, 2]);
        assert_eq!(after.alpha().unwrap().as_slice(), &[255, 128, 0]);
        assert_eq!(proposed.layer(owner).unwrap().indexed_source, before.layer(owner).unwrap().indexed_source);
    }
    #[test]
    fn explicit_changed_plane_with_wrong_carrier_or_outside_pixels_is_refused_atomically() {
        let (session, _, owner) = setup(false);
        let before = session.active().unwrap().doc.clone();
        for outside in [false, true] {
            let mut proposed = (*before).clone();
            let indexed = proposed.xcf_state.as_mut().unwrap().editable_indexed.iter_mut().find(|item| item.owner == owner).unwrap();
            indexed.raster.try_set_pixel(0, 1, 2, 128).unwrap();
            let raster = indexed.raster.clone();
            if outside {
                *proposed.layer_mut(owner).unwrap().surface_mut().unwrap() = expanded(&raster, PixelFormat::RGBA8, None).unwrap();
                proposed.layer_mut(owner).unwrap().surface_mut().unwrap().try_write_interleaved(Rect::new(10, 10, 11, 11), &[1, 2, 3, 4]).unwrap();
            }
            let retained = proposed.clone();
            assert!(synchronize_indexed_with_interrupt(&before, &mut proposed, &photocraft_raster::Interrupt::NONE).is_err());
            assert_eq!(proposed, retained);
        }
    }
    #[test]
    fn explicit_transparent_table_change_preserves_independent_alpha_without_reprojection() {
        let (session, _, owner) = setup(false);
        let before = session.active().unwrap().doc.clone();
        let mut proposed = (*before).clone();
        proposed.color_table.as_mut().unwrap().transparent = Some(1);
        let raster = proposed.xcf_state.as_ref().unwrap().editable_indexed.iter().find(|item| item.owner == owner).unwrap().raster.clone();
        *proposed.layer_mut(owner).unwrap().surface_mut().unwrap() = expanded(&raster, PixelFormat::RGBA8, Some(1)).unwrap();
        let retained = proposed.clone();
        synchronize_indexed_with_interrupt(&before, &mut proposed, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(proposed, retained);
        assert_eq!(proposed.xcf_state.as_ref().unwrap().editable_indexed[0].raster.alpha().unwrap().as_slice(), &[255, 128, 0]);
        let native = photocraft_format::save_to_bytes(&proposed, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), proposed);
    }
    #[test]
    fn direct_verification_cancellation_leaves_proposed_and_original_carriers_exact() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (session, _, owner) = setup(false);
        let before = session.active().unwrap().doc.clone();
        let mut proposed = (*before).clone();
        let indexed = proposed.xcf_state.as_mut().unwrap().editable_indexed.iter_mut().find(|item| item.owner == owner).unwrap();
        indexed.raster.try_set_pixel(0, 1, 0, 128).unwrap();
        let retained = proposed.clone();
        let calls = AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, Ordering::Relaxed) >= 12;
        assert!(matches!(
            synchronize_indexed_with_interrupt(&before, &mut proposed, &photocraft_raster::Interrupt::cancel_only(&stop)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(proposed, retained);
        assert_eq!(before.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices().as_slice(), &[0, 1, 2]);
    }
    #[test]
    fn transparent_table_entry_preserves_independent_alpha_and_can_be_unset() {
        let (mut s, _, child) = setup(false);
        s.execute("image.mode.colorTable", json!({"transparent":1})).unwrap();
        assert_eq!(current(&s, child).alpha().unwrap().as_slice(), &[255, 128, 0]);
        assert_eq!(s.active().unwrap().doc.layer(child).unwrap().surface().unwrap().rgba(0, 1)[3], 0.0);
        s.execute("image.mode.colorTable", json!({"transparent":null})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(child).unwrap().surface().unwrap().rgba(0, 1)[3], 128.0 / 255.0);
    }
    #[test]
    fn generic_raster_edits_quantize_new_colors_keep_duplicate_identity_alpha_and_undo() {
        let (mut s, _, child) = setup(false);
        let original = s.active().unwrap().doc.clone();
        s.edit("own pixel edit", |doc, _| {
            let surface = doc.layer_mut(child).unwrap().surface_mut().unwrap();
            surface.try_write_pixel_f64(-1, 1, &[0.05, 0.05, 1.0, 1.0]).map_err(error)?;
            surface.try_write_pixel_f64(0, 1, &[1.0, 0.0, 0.0, 0.25]).map_err(error)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(current(&s, child).indices().as_slice(), &[2, 1, 2]);
        assert_eq!(current(&s, child).alpha().unwrap().as_slice(), &[255, 64, 0]);
        assert_eq!(s.active().unwrap().doc.layer(child).unwrap().surface().unwrap().rgba(-1, 1), [0.0, 0.0, 1.0, 1.0]);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc == original);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(current(&s, child).indices().as_slice(), &[2, 1, 2]);
    }
    #[test]
    fn layer_move_preserves_duplicate_indices_declared_extent_and_source_snapshot() {
        let (mut s, _, child) = setup(false);
        let source = s.active().unwrap().doc.layer(child).unwrap().indexed_source.clone();
        s.execute("layer.translate", json!({"layer":child.0,"dx":1,"dy":-2})).unwrap();
        assert_eq!(current(&s, child).bounds(), Rect::new(0, -1, 3, 0));
        assert_eq!(current(&s, child).indices().as_slice(), &[0, 1, 2]);
        assert_eq!(s.active().unwrap().doc.layer(child).unwrap().indexed_source, source);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(current(&s, child).bounds(), Rect::new(-1, 1, 2, 2));
    }
    #[test]
    fn recursive_group_duplication_preserves_current_state_native_and_undo() {
        let (mut s, root, child) = setup(true);
        let result = s.execute("layer.duplicate", json!({"layer":root.0,"inPlace":true})).unwrap();
        let target = LayerId(result["layer"].as_u64().unwrap());
        let doc = &s.active().unwrap().doc;
        let target_child = doc.layer(target).unwrap().children().unwrap()[0].id;
        assert_eq!(current(&s, target_child), current(&s, child));
        assert_eq!(doc.layer(target_child).unwrap().indexed_source, doc.layer(child).unwrap().indexed_source);
        assert_eq!(doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(target_child)).unwrap().parent, Some(ItemRef::Layer(target)));
        let back = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()).unwrap();
        assert!(back == **doc);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().editable_indexed.len(), 1);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(current(&s, target_child), current(&s, child));
    }
    #[test]
    fn explicit_palette_shrink_remaps_only_removed_ordinals_and_undoes() {
        let (mut s, _, child) = setup(false);
        s.execute("image.mode.colorTable", json!({"colors":["#ff0000","#00ff00"]})).unwrap();
        assert_eq!(current(&s, child).indices().as_slice(), &[0, 1, 0]);
        assert_eq!(current(&s, child).alpha().unwrap().as_slice(), &[255, 128, 0]);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(current(&s, child).indices().as_slice(), &[0, 1, 2]);
    }
    #[test]
    fn invalid_palette_and_depth_edits_fail_atomically() {
        let (mut s, _, child) = setup(false);
        let original = s.active().unwrap().doc.clone();
        assert!(s.execute("image.mode.colorTable", json!({"colors":[]})).is_err());
        assert!(s.active().unwrap().doc == original);
        assert!(
            s.edit("invalid depth", |doc, _| {
                doc.layer_mut(child).unwrap().content = LayerContent::Raster(Surface::new(PixelFormat::RGBA16));
                Ok(())
            })
            .is_err()
        );
        assert!(s.active().unwrap().doc == original);
    }
    #[test]
    fn explicit_rgb_conversion_clears_current_planes_keeps_original_provenance() {
        let (mut s, _, child) = setup(false);
        let source = s.active().unwrap().doc.layer(child).unwrap().indexed_source.clone();
        s.edit("explicit RGB", |doc, _| {
            doc.mode = ColorMode::Rgb;
            Ok(())
        })
        .unwrap();
        assert!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().editable_indexed.is_empty());
        assert_eq!(s.active().unwrap().doc.layer(child).unwrap().indexed_source, source);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(current(&s, child).indices().as_slice(), &[0, 1, 2]);
    }
    #[test]
    fn linked_mask_group_move_without_indexed_planes_preserves_native_and_undo() {
        use photocraft_doc::LayerMask;
        use photocraft_doc::xcf_state::{ItemState, MaskState};
        let (mut session, root, child) = setup(true);
        session
            .edit("own mask", |doc, _| {
                doc.mode = ColorMode::Rgb;
                doc.layer_mut(child).unwrap().mask = Some(LayerMask::reveal_all());
                doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(child)).unwrap().raster_bounds =
                    Some(Rect::new(-1, 1, 2, 2));
                doc.xcf_state.as_mut().unwrap().items.push(ItemState {
                    item: ItemRef::Mask(MaskId(1)),
                    name: "own mask".into(),
                    custom_name: true,
                    visible: true,
                    locks: Default::default(),
                    parent: None,
                    source_tattoo: None,
                    source_flags: 0,
                    raster_bounds: None,
                    composition: None,
                    mask: Some(MaskState {
                        owner: ItemRef::Layer(child),
                        bounds: Rect::new(-2, 1, 4, 5),
                        offset: [-2, 1],
                        outside_f64_bits: 0.0f64.to_bits(),
                        enabled: true,
                        shown: false,
                        editing: false,
                        linked: true,
                    }),
                });
                Ok(())
            })
            .unwrap();
        let original = session.active().unwrap().doc.clone();
        assert!(original.xcf_state.as_ref().unwrap().editable_indexed.is_empty());
        session.execute("layer.translate", json!({"layer":root.0,"dx":3,"dy":-2})).unwrap();
        let doc = &session.active().unwrap().doc;
        let mask = doc.xcf_state.as_ref().unwrap().items.iter().find_map(|i| i.mask.as_ref()).unwrap();
        assert_eq!(mask.bounds, Rect::new(1, -1, 7, 3));
        assert_eq!(mask.offset, [1, -1]);
        assert_eq!(
            doc.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(child)).unwrap().raster_bounds,
            Some(Rect::new(2, -1, 5, 0))
        );
        let back = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()).unwrap();
        assert!(back == **doc);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc == original);
        session
            .edit("unlink mask", |doc, _| {
                doc.layer_mut(child).unwrap().mask.as_mut().unwrap().linked = false;
                Ok(())
            })
            .unwrap();
        session.execute("layer.translate", json!({"layer":root.0,"dx":3,"dy":-2})).unwrap();
        let mask = session.active().unwrap().doc.xcf_state.as_ref().unwrap().items.iter().find_map(|i| i.mask.as_ref()).unwrap();
        assert_eq!(mask.bounds, Rect::new(-2, 1, 4, 5));
        assert!(!mask.linked);
    }
    #[test]
    fn mask_translation_overflow_is_atomic() {
        use photocraft_doc::LayerMask;
        use photocraft_doc::xcf_state::{ItemState, MaskState};
        let (session, root, child) = setup(false);
        let mut doc = (*session.active().unwrap().doc).clone();
        doc.layer_mut(child).unwrap().mask = Some(LayerMask::reveal_all());
        doc.xcf_state.as_mut().unwrap().items.push(ItemState {
            item: ItemRef::Mask(MaskId(1)),
            name: "own mask".into(),
            custom_name: true,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: None,
            composition: None,
            mask: Some(MaskState {
                owner: ItemRef::Layer(child),
                bounds: Rect::new(i32::MAX - 1, 0, i32::MAX, 1),
                offset: [0, 0],
                outside_f64_bits: 0.0f64.to_bits(),
                enabled: true,
                shown: false,
                editing: false,
                linked: true,
            }),
        });
        let before = doc.clone();
        assert!(translate_indexed(&mut doc, root, 1, 0).is_err());
        assert!(doc == before);
    }
}

#[cfg(test)]
mod relationship_id_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn inspected_members_round_trip_to_legacy_typed_targets_without_loss() {
        for target in [
            ItemRef::Layer(LayerId(u64::MAX)),
            ItemRef::Channel(photocraft_doc::item_identity::ChannelId(8)),
            ItemRef::Path(photocraft_doc::item_identity::PathId(9)),
        ] {
            let inspected = crate::xcf_inspect_cmds::reference(target);
            assert_eq!(member_reference("test", &inspected).unwrap(), target);
        }
        assert_eq!(member_reference("test", &json!({"Layer":"18446744073709551615"})).unwrap(), ItemRef::Layer(LayerId(u64::MAX)));
        assert_eq!(member_reference("test", &json!({"Layer":7})).unwrap(), ItemRef::Layer(LayerId(7)));
        assert_eq!(set_id("test", &json!({"setId":"7"})).unwrap(), photocraft_doc::item_identity::ItemSetId(7));
        assert_eq!(set_id("test", &json!({"setId":7})).unwrap(), photocraft_doc::item_identity::ItemSetId(7));
    }
    #[test]
    fn noncanonical_ids_and_unknown_shapes_never_select_an_item() {
        for value in [
            json!(null),
            json!(false),
            json!(-1),
            json!(0),
            json!(1.0),
            json!(""),
            json!("0"),
            json!("01"),
            json!("+1"),
            json!(" 1"),
            json!("1 "),
            json!("18446744073709551616"),
        ] {
            assert!(relationship_id("test", &value).is_err());
        }
        for value in [
            json!({"kind":"layer","id":7}),
            json!({"kind":"Layer","id":"7"}),
            json!({"kind":"mask","id":"7"}),
            json!({"kind":"layer","id":"7","name":"guess"}),
            json!({"Layer":"7","Path":"8"}),
            json!({"id":"7"}),
        ] {
            assert!(member_reference("test", &value).is_err());
        }
    }
}
