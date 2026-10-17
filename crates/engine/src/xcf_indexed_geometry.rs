//! Exact geometry on current indexed ordinals and independent alpha, never source provenance.
use crate::{EngineError, Result};
use photocraft_algo::{
    resample::Resample,
    wide_geometry::{GeometryLimits, OrthogonalTransform},
};
use photocraft_doc::{ColorMode, Document, EditableIndexedRaster, LayerContent, LayerId, PixelFormat};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::sync::Arc;
fn error(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("cannot transform current indexed pixels: {e}"))
}

/// Conservative retained allocations and bounded verification work. These are
/// admission ledgers, not measurements of physical process residency.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IndexedGeometryRequirements {
    pub retained_bytes: u64,
    pub scratch_bytes: u64,
    pub work_units: u64,
    pub layer_count: usize,
}

pub(crate) fn validate_current_with_interrupt(doc: &Document, control: &Interrupt<'_>) -> Result<IndexedGeometryRequirements> {
    validate_current_with_limits(doc, GeometryLimits::default(), control)
}

/// Borrowed current-state validation before a geometry worker clones its model.
/// Original IndexedSource bytes are provenance, never a current-pixel oracle.
pub(crate) fn validate_current_with_limits(doc: &Document, limits: GeometryLimits, control: &Interrupt<'_>) -> Result<IndexedGeometryRequirements> {
    use photocraft_doc::{Layer, SampleType, item_identity::ItemRef};
    fn poll(control: &Interrupt<'_>) -> Result<()> {
        if control.cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
    }
    fn add(value: &mut u64, amount: u64) -> Result<()> {
        *value = value.checked_add(amount).ok_or_else(|| error("verification ledger overflow"))?;
        Ok(())
    }
    poll(control)?;
    let Some(state) = &doc.xcf_state else { return Ok(IndexedGeometryRequirements::default()) };
    if state.editable_indexed.is_empty() {
        return Ok(IndexedGeometryRequirements::default());
    }
    let standard = GeometryLimits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
        return Err(error("indexed verification limits may only be lowered"));
    }
    if doc.mode != ColorMode::Indexed || doc.depth != SampleType::U8 {
        return Err(error("canonical indexed geometry requires Indexed U8"));
    }
    if state.editable_indexed.len() > 10_000 || state.items.len() > 10_000 {
        return Err(error("indexed metadata count limit"));
    }
    let table = doc.color_table.as_ref().ok_or_else(|| error("missing current palette"))?;
    if table.colors.is_empty() || table.colors.len() > 256 || table.transparent.is_some_and(|index| usize::from(index) >= table.colors.len()) {
        return Err(error("invalid current palette"));
    }
    // Reject tiny envelopes before even the allocation-free hierarchy census.
    // This also caps the source lengths before any sample validation or inventory.
    let fixed_count = state.items.len().checked_add(state.editable_indexed.len()).ok_or_else(|| error("inventory count overflow"))?;
    let fixed_work = u64::try_from(fixed_count).map_err(error)?.checked_mul(32).ok_or_else(|| error("inventory work overflow"))?;
    if limits.resident_bytes < 1024 || limits.work_units < fixed_work {
        return Err(error("indexed inventory minimum budget"));
    }
    for plane in &state.editable_indexed {
        poll(control)?;
        if plane.raster.indices().len() > 64 << 20 || plane.raster.alpha().is_some_and(|alpha| alpha.len() > 64 << 20) || plane.raster.palette().len() > 256 {
            return Err(error("indexed source length limit"));
        }
    }
    fn census(siblings: &[Layer], depth: usize, count: &mut usize, work_limit: u64, fixed_work: u64, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        if depth > 100 || siblings.len() > 10_000usize.saturating_sub(*count) {
            return Err(error("indexed hierarchy census limit"));
        }
        for layer in siblings {
            poll(control)?;
            *count = count.checked_add(1).ok_or_else(|| error("hierarchy count overflow"))?;
            let work =
                u64::try_from(*count).map_err(error)?.checked_mul(32).and_then(|n| n.checked_add(fixed_work)).ok_or_else(|| error("census work overflow"))?;
            if work > work_limit {
                return Err(error("indexed hierarchy work budget"));
            }
            if let LayerContent::Group(group) = &layer.content {
                census(&group.children, depth.checked_add(1).ok_or_else(|| error("depth overflow"))?, count, work_limit, fixed_work, control)?;
            }
        }
        Ok(())
    }
    let mut layer_count = 0usize;
    census(&doc.layers, 0, &mut layer_count, limits.work_units, fixed_work, control)?;
    let pending_count = layer_count.checked_add(1).ok_or_else(|| error("inventory count overflow"))?;
    let mut prospective = 1024u64;
    for bytes in [
        layer_count.checked_mul(std::mem::size_of::<&Layer>()),
        pending_count.checked_mul(std::mem::size_of::<(&[Layer], usize)>()),
        state.items.len().checked_mul(std::mem::size_of::<&photocraft_doc::xcf_state::ItemState>()),
        state.editable_indexed.len().checked_mul(std::mem::size_of::<&photocraft_doc::EditableIndexedLayer>()),
    ] {
        add(&mut prospective, u64::try_from(bytes.ok_or_else(|| error("inventory size overflow"))?).map_err(error)?)?;
    }
    if prospective > limits.resident_bytes {
        return Err(error("prospective indexed inventory budget"));
    }
    // A bounded fallible inventory avoids recursive doc.layer lookups and avoids
    // guessing identity from sparse surface content or immutable source snapshots.
    let mut pending = Vec::new();
    pending.try_reserve_exact(pending_count).map_err(error)?;
    pending.push((doc.layers.as_slice(), 0usize));
    let mut layers: Vec<&Layer> = Vec::new();
    layers.try_reserve_exact(layer_count).map_err(error)?;
    let mut initial_capacity = u64::try_from(pending.capacity())
        .map_err(error)?
        .checked_mul(std::mem::size_of::<(&[Layer], usize)>() as u64)
        .and_then(|n| {
            u64::try_from(layers.capacity())
                .ok()
                .and_then(|count| count.checked_mul(std::mem::size_of::<&Layer>() as u64))
                .and_then(|bytes| n.checked_add(bytes))
        })
        .and_then(|n| n.checked_add(1024))
        .ok_or_else(|| error("inventory allocation capacity overflow"))?;
    if initial_capacity > limits.resident_bytes {
        return Err(error("actual indexed inventory capacity budget"));
    }
    while let Some((siblings, depth)) = pending.pop() {
        poll(control)?;
        if depth > 100 {
            return Err(error("indexed hierarchy depth limit"));
        }
        for layer in siblings {
            poll(control)?;
            if layers.len() >= 10_000 || layer.id.0 == 0 {
                return Err(error("indexed layer identity limit"));
            }
            if layers.len() >= layer_count {
                return Err(error("hierarchy census changed"));
            }
            layers.push(layer);
            if let LayerContent::Group(group) = &layer.content {
                if pending.len() >= 10_000 {
                    return Err(error("indexed hierarchy pending limit"));
                }
                if pending.len() >= pending_count {
                    return Err(error("hierarchy census changed"));
                }
                pending.push((group.children.as_slice(), depth.checked_add(1).ok_or_else(|| error("depth overflow"))?));
            }
        }
    }
    layers.sort_unstable_by_key(|layer| layer.id);
    if layers.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a.id == b.id)) {
        return Err(error("duplicate current layer ID"));
    }
    let mut items = Vec::new();
    items.try_reserve_exact(state.items.len()).map_err(error)?;
    add(
        &mut initial_capacity,
        u64::try_from(items.capacity())
            .map_err(error)?
            .checked_mul(std::mem::size_of::<&photocraft_doc::xcf_state::ItemState>() as u64)
            .ok_or_else(|| error("item inventory capacity overflow"))?,
    )?;
    if initial_capacity > limits.resident_bytes {
        return Err(error("actual item inventory budget"));
    }
    items.extend(state.items.iter());
    items.sort_unstable_by_key(|item| item.item);
    if items.iter().any(|item| item.item.raw() == 0) {
        return Err(error("zero typed current item ID"));
    }
    if items.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a.item == b.item)) {
        return Err(error("duplicate typed current item"));
    }
    let mut planes = Vec::new();
    planes.try_reserve_exact(state.editable_indexed.len()).map_err(error)?;
    add(
        &mut initial_capacity,
        u64::try_from(planes.capacity())
            .map_err(error)?
            .checked_mul(std::mem::size_of::<&photocraft_doc::EditableIndexedLayer>() as u64)
            .ok_or_else(|| error("plane inventory capacity overflow"))?,
    )?;
    if initial_capacity > limits.resident_bytes {
        return Err(error("actual plane inventory budget"));
    }
    planes.extend(state.editable_indexed.iter());
    planes.sort_unstable_by_key(|plane| plane.owner);
    if planes.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a.owner == b.owner)) {
        return Err(error("duplicate canonical indexed owner"));
    }
    // Every resident raster in an indexed document needs its own canonical plane.
    for layer in &layers {
        poll(control)?;
        if matches!(layer.content, LayerContent::Raster(_)) && planes.binary_search_by_key(&layer.id, |plane| plane.owner).is_err() {
            return Err(error("resident raster lacks canonical indexed owner"));
        }
        if layer.indexed_source.is_some() && planes.binary_search_by_key(&layer.id, |plane| plane.owner).is_err() {
            return Err(error("original indexed snapshot lacks current owner"));
        }
    }
    let mut required = IndexedGeometryRequirements { scratch_bytes: 1024, layer_count: layers.len(), ..Default::default() };
    for bytes in [
        layers.capacity().checked_mul(std::mem::size_of::<&Layer>()),
        pending.capacity().checked_mul(std::mem::size_of::<(&[Layer], usize)>()),
        items.capacity().checked_mul(std::mem::size_of::<&photocraft_doc::xcf_state::ItemState>()),
        planes.capacity().checked_mul(std::mem::size_of::<&photocraft_doc::EditableIndexedLayer>()),
    ] {
        add(&mut required.scratch_bytes, u64::try_from(bytes.ok_or_else(|| error("inventory capacity overflow"))?).map_err(error)?)?;
    }
    add(
        &mut required.work_units,
        u64::try_from(layers.len().checked_add(items.len()).and_then(|n| n.checked_add(planes.len())).ok_or_else(|| error("inventory work overflow"))?)
            .map_err(error)?
            .checked_mul(32)
            .ok_or_else(|| error("inventory work overflow"))?,
    )?;
    for plane in &planes {
        poll(control)?;
        let layer = layers
            .get(layers.binary_search_by_key(&plane.owner, |layer| layer.id).map_err(|_| error("missing current indexed owner"))?)
            .ok_or_else(|| error("missing current indexed owner"))?;
        let LayerContent::Raster(surface) = &layer.content else { return Err(error("indexed owner is not resident raster")) };
        if surface.format() != PixelFormat::RGBA8 {
            return Err(error("current indexed carrier must be RGBA8"));
        }
        let typed = items
            .get(items.binary_search_by_key(&ItemRef::Layer(plane.owner), |item| item.item).map_err(|_| error("missing typed indexed owner"))?)
            .ok_or_else(|| error("missing typed indexed owner"))?;
        if typed.raster_bounds != Some(plane.raster.bounds()) || plane.raster.palette() != table.colors.as_slice() {
            return Err(error("current indexed bounds/palette mismatch"));
        }
        let bounds = plane.raster.bounds();
        let count = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(|| error("indexed pixel count overflow"))?;
        if bounds.is_empty() || bounds.width() > 262_144 || bounds.height() > 262_144 || count > 64 << 20 {
            return Err(error("indexed extent limit"));
        }
        add(&mut required.work_units, crate::xcf_editing::direct_indexed_sync_work(usize::try_from(count).map_err(error)?, surface.tile_count())?)?;
        if required.work_units > limits.work_units {
            return Err(error("aggregate indexed verification work budget"));
        }
        add(&mut required.retained_bytes, u64::try_from(surface.default_bytes().len()).map_err(error)?)?;
        for (_, tile) in surface.tiles() {
            poll(control)?;
            add(&mut required.retained_bytes, u64::try_from(tile.bytes().len()).map_err(error)?)?;
            add(&mut required.retained_bytes, 256)?;
            if required.retained_bytes > limits.resident_bytes {
                return Err(error("indexed carrier retained budget"));
            }
        }
        add(&mut required.retained_bytes, u64::try_from(plane.raster.indices().capacity()).map_err(error)?)?;
        add(&mut required.retained_bytes, u64::try_from(plane.raster.alpha().map_or(0, |alpha| alpha.capacity())).map_err(error)?)?;
        add(&mut required.retained_bytes, u64::try_from(plane.raster.palette_capacity_bytes()).map_err(error)?)?;
    }
    if required.work_units > limits.work_units || required.retained_bytes.checked_add(required.scratch_bytes).is_none_or(|bytes| bytes > limits.resident_bytes)
    {
        return Err(error("aggregate indexed verification budget"));
    }
    let mut scratch = [0u8; 1024];
    for plane in &planes {
        poll(control)?;
        plane.raster.validate_with(control).map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
        let layer = layers
            .get(layers.binary_search_by_key(&plane.owner, |layer| layer.id).map_err(|_| error("missing indexed owner"))?)
            .ok_or_else(|| error("missing indexed owner"))?;
        let LayerContent::Raster(surface) = &layer.content else { return Err(error("indexed owner changed")) };
        surface.try_validate_tile_origins_with(control).map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
        if surface.default_bytes() != [0u8; 4] {
            return Err(error("indexed carrier default mismatch"));
        }
        let bounds = plane.raster.bounds();
        let side = usize::try_from(photocraft_geom::TILE_SIZE).map_err(error)?;
        let tile_bytes = side.checked_mul(side).and_then(|n| n.checked_mul(4)).ok_or_else(|| error("tile length overflow"))?;
        for (coordinate, tile) in surface.tiles() {
            poll(control)?;
            if tile.bytes().len() != tile_bytes {
                return Err(error("indexed carrier tile length"));
            }
            let rect = coordinate.rect();
            for (offset, pixel) in tile.bytes().as_chunks::<4>().0.iter().enumerate() {
                if offset.is_multiple_of(1024) {
                    poll(control)?;
                }
                let x = i64::from(rect.x0) + i64::try_from(offset % side).map_err(error)?;
                let y = i64::from(rect.y0) + i64::try_from(offset / side).map_err(error)?;
                if (x < i64::from(bounds.x0) || x >= i64::from(bounds.x1) || y < i64::from(bounds.y0) || y >= i64::from(bounds.y1))
                    && pixel.iter().any(|byte| *byte != 0)
                {
                    return Err(error("indexed carrier outside tail mismatch"));
                }
            }
        }
        let width = usize::try_from(bounds.width()).map_err(error)?;
        for y in bounds.y0..bounds.y1 {
            let mut x = bounds.x0;
            while x < bounds.x1 {
                poll(control)?;
                let take = usize::try_from(i64::from(bounds.x1) - i64::from(x)).map_err(error)?.min(256);
                let end = i32::try_from(i64::from(x) + i64::try_from(take).map_err(error)?).map_err(error)?;
                let raw = scratch.get_mut(..take.checked_mul(4).ok_or_else(|| error("chunk length overflow"))?).ok_or_else(|| error("chunk length"))?;
                surface
                    .try_read_interleaved_into_with(Rect::new(x, y, end, y.checked_add(1).ok_or_else(|| error("row overflow"))?), raw, control)
                    .map_err(|failure| if control.cancelled() { EngineError::Cancelled } else { error(failure) })?;
                for (column, pixel) in raw.as_chunks::<4>().0.iter().enumerate() {
                    let px = i32::try_from(i64::from(x) + i64::try_from(column).map_err(error)?).map_err(error)?;
                    let mut expected = plane.raster.expanded_pixel(px, y).map_err(error)?;
                    let ordinal = usize::try_from(i64::from(y) - i64::from(bounds.y0))
                        .map_err(error)?
                        .checked_mul(width)
                        .and_then(|n| usize::try_from(i64::from(px) - i64::from(bounds.x0)).ok().and_then(|column| n.checked_add(column)))
                        .ok_or_else(|| error("ordinal overflow"))?;
                    if plane.raster.indices().get(ordinal).copied() == table.transparent {
                        expected[3] = 0;
                    }
                    if expected.as_slice() != pixel {
                        return Err(error("canonical indexed carrier pixel mismatch"));
                    }
                }
                x = end;
            }
        }
    }
    poll(control)?;
    Ok(required)
}
#[derive(Clone, Copy)]
enum IndexedTransform {
    Resize { sx: f64, sy: f64 },
    Orthogonal { canvas: Rect, operation: OrthogonalTransform },
}
fn geometry_poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn geometry_failure(failure: impl std::fmt::Display, control: &Interrupt<'_>) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(failure) }
}
fn target_bounds(bounds: Rect, operation: IndexedTransform) -> Result<Rect> {
    let target = match operation {
        IndexedTransform::Resize { sx, sy } => {
            if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
                return Err(error("invalid indexed scale"));
            }
            // Same floor/ceil bounds as wide_resample::try_resize. Sampling itself
            // remains in that existing rawNearest implementation.
            let values = [
                (f64::from(bounds.x0) * sx).floor(),
                (f64::from(bounds.y0) * sy).floor(),
                (f64::from(bounds.x1) * sx).ceil(),
                (f64::from(bounds.y1) * sy).ceil(),
            ];
            if values.iter().any(|n| !n.is_finite() || *n < f64::from(i32::MIN) || *n > f64::from(i32::MAX)) {
                return Err(error("indexed scale bounds overflow"));
            }
            let [x0, y0, x1, y1] = values;
            Rect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32)
        }
        IndexedTransform::Orthogonal { canvas, operation } => {
            if canvas.is_empty() {
                return Err(error("empty transform canvas"));
            }
            let (x, y) = match operation {
                OrthogonalTransform::FlipHorizontal => (i64::from(canvas.x0) + i64::from(canvas.x1) - i64::from(bounds.x1), i64::from(bounds.y0)),
                OrthogonalTransform::FlipVertical => (i64::from(bounds.x0), i64::from(canvas.y0) + i64::from(canvas.y1) - i64::from(bounds.y1)),
                OrthogonalTransform::Rotate180 => {
                    (i64::from(canvas.x0) + i64::from(canvas.x1) - i64::from(bounds.x1), i64::from(canvas.y0) + i64::from(canvas.y1) - i64::from(bounds.y1))
                }
                OrthogonalTransform::RotateClockwise => {
                    (i64::from(canvas.x0) + i64::from(canvas.y1) - i64::from(bounds.y1), i64::from(canvas.y0) + i64::from(bounds.x0) - i64::from(canvas.x0))
                }
                OrthogonalTransform::RotateCounterclockwise => {
                    (i64::from(canvas.x0) + i64::from(bounds.y0) - i64::from(canvas.y0), i64::from(canvas.y0) + i64::from(canvas.x1) - i64::from(bounds.x1))
                }
            };
            let swap = matches!(operation, OrthogonalTransform::RotateClockwise | OrthogonalTransform::RotateCounterclockwise);
            let width = if swap { bounds.height() } else { bounds.width() };
            let height = if swap { bounds.width() } else { bounds.height() };
            Rect::new(
                i32::try_from(x).map_err(error)?,
                i32::try_from(y).map_err(error)?,
                i32::try_from(x.checked_add(i64::from(width)).ok_or_else(|| error("target overflow"))?).map_err(error)?,
                i32::try_from(y.checked_add(i64::from(height)).ok_or_else(|| error("target overflow"))?).map_err(error)?,
            )
        }
    };
    let count = u64::from(target.width()).checked_mul(u64::from(target.height())).ok_or_else(|| error("target count overflow"))?;
    if target.is_empty() || target.width() > 262144 || target.height() > 262144 || count > 64 << 20 {
        return Err(error("canonical target extent limit"));
    }
    photocraft_raster::checked_region_byte_len(target, PixelFormat::RGBA8).map_err(error)?;
    Ok(target)
}
fn geometry_tiles(bounds: Rect) -> Result<u64> {
    if bounds.is_empty() {
        return Ok(0);
    }
    let side = i64::from(photocraft_geom::TILE_SIZE);
    let width = (i64::from(bounds.x1) - 1).div_euclid(side) - i64::from(bounds.x0).div_euclid(side) + 1;
    let height = (i64::from(bounds.y1) - 1).div_euclid(side) - i64::from(bounds.y0).div_euclid(side) + 1;
    u64::try_from(width).ok().and_then(|width| width.checked_mul(u64::try_from(height).ok()?)).ok_or_else(|| error("tile count overflow"))
}
fn geometry_add(value: &mut u64, amount: u64) -> Result<()> {
    *value = value.checked_add(amount).ok_or_else(|| error("indexed geometry ledger overflow"))?;
    Ok(())
}
fn check_capacity_overrun(capacity: usize, requested: usize, overrun: &mut u64, peak: u64, limits: GeometryLimits) -> Result<()> {
    geometry_add(overrun, u64::try_from(capacity.saturating_sub(requested)).map_err(error)?)?;
    if peak.checked_add(*overrun).is_none_or(|n| n > limits.resident_bytes) {
        return Err(error("actual indexed allocation capacity limit"));
    }
    Ok(())
}
fn transform(doc: &mut Document, limits: GeometryLimits, control: &Interrupt<'_>, operation: IndexedTransform) -> Result<Vec<(LayerId, Rect)>> {
    geometry_poll(control)?;
    let Some(state) = &doc.xcf_state else { return Ok(Vec::new()) };
    if state.editable_indexed.is_empty() {
        return Ok(Vec::new());
    }
    let standard = GeometryLimits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
        return Err(error("indexed geometry limits may only be lowered"));
    }
    let stage_count = state.editable_indexed.len();
    if stage_count > 10_000 {
        return Err(error("indexed geometry owner count limit"));
    }
    let stage_bytes = stage_count
        .checked_mul(std::mem::size_of::<(LayerId, EditableIndexedRaster, Surface)>() + std::mem::size_of::<(LayerId, Rect)>())
        .ok_or_else(|| error("stage index overflow"))? as u64;
    // Retain every existing model/source allocation once. Output and temporary
    // carriers below are additional private allocations, never a source refund.
    let mut peak = stage_bytes;
    let mut work = 0u64;
    let tile_side = u64::try_from(photocraft_geom::TILE_SIZE).map_err(error)?;
    let tile_pixels = tile_side.checked_mul(tile_side).ok_or_else(|| error("tile overflow"))?;
    for item in &state.editable_indexed {
        geometry_poll(control)?;
        let bounds = item.raster.bounds();
        if bounds.is_empty()
            || bounds.width() > 262144
            || bounds.height() > 262144
            || item.raster.indices().len() > 64 << 20
            || item.raster.alpha().is_some_and(|alpha| alpha.len() > 64 << 20)
            || item.raster.palette().len() > 256
        {
            return Err(error("indexed source extent/length limit"));
        }
        let target = target_bounds(bounds, operation)?;
        let count = u64::from(target.width()).checked_mul(u64::from(target.height())).ok_or_else(|| error("target count overflow"))?;
        let source_count = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(|| error("source count overflow"))?;
        let planes = if item.raster.alpha().is_some() { 2u64 } else { 1 };
        let tiles = geometry_tiles(target)?;
        let palette = (item.raster.palette().len() as u64).checked_mul(3).ok_or_else(|| error("palette charge overflow"))?;
        geometry_add(&mut peak, count.checked_mul(planes).and_then(|n| n.checked_add(palette)).ok_or_else(|| error("plane charge overflow"))?)?;
        geometry_add(&mut peak, planes.checked_mul(128).ok_or_else(|| error("plane owner charge overflow"))?)?;
        // RGBA8 output padded cells, sparse-map/Arc bookkeeping, tile copy and
        // one maximum tile-sized encoding buffer. Pruning never refunds admission.
        geometry_add(
            &mut peak,
            tiles
                .checked_mul(tile_pixels.checked_mul(4).and_then(|n| n.checked_add(1024)).ok_or_else(|| error("carrier charge overflow"))?)
                .ok_or_else(|| error("carrier charge overflow"))?,
        )?;
        geometry_add(&mut peak, tile_pixels.checked_mul(12).ok_or_else(|| error("tile scratch overflow"))?)?;
        geometry_add(
            &mut work,
            count
                .checked_mul(64)
                .and_then(|n| n.checked_add(source_count.checked_mul(8)?))
                .and_then(|n| n.checked_add(tiles.checked_mul(tile_pixels.checked_mul(8)?)?))
                .ok_or_else(|| error("geometry work overflow"))?,
        )?;
        if matches!(operation, IndexedTransform::Resize { .. }) {
            let input_tiles = geometry_tiles(bounds)?;
            // Both ordinal/alpha materializations plus output native reads and
            // the existing lower sampler's tile/tap bookkeeping are precharged.
            geometry_add(
                &mut peak,
                input_tiles
                    .checked_add(tiles)
                    .and_then(|n| n.checked_mul(tile_pixels.checked_add(1024)?))
                    .and_then(|n| n.checked_mul(planes))
                    .and_then(|n| n.checked_add(count.checked_mul(planes)?))
                    .and_then(|n| n.checked_add(tile_pixels.checked_mul(8)?))
                    .ok_or_else(|| error("resize intermediate overflow"))?,
            )?;
        }
        if peak > limits.resident_bytes || work > limits.work_units {
            return Err(error("aggregate indexed geometry budget"));
        }
    }
    // All target allocation/work requirements precede expensive current-carrier
    // verification and model/payload censuses. Each lower lane gets remaining work.
    let verified = validate_current_with_limits(
        doc,
        GeometryLimits {
            resident_bytes: limits.resident_bytes.checked_sub(peak).ok_or_else(|| error("indexed remaining residency"))?,
            work_units: limits.work_units.checked_sub(work).ok_or_else(|| error("indexed remaining work"))?,
        },
        control,
    )?;
    geometry_add(&mut work, verified.work_units)?;
    let lookup_work = (stage_count as u64)
        .checked_mul(verified.layer_count as u64)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add((stage_count as u64).checked_mul(stage_count as u64)?.checked_mul(2)?))
        .ok_or_else(|| error("indexed destination lookup work overflow"))?;
    geometry_add(&mut work, lookup_work)?;
    let prep = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, prep, control).map_err(|e| geometry_failure(e, control))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, prep, control).map_err(|e| geometry_failure(e, control))? as u64;
    let footprint = crate::document_snapshot::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    for retained in [model, payload, footprint.clone_metadata_bytes, verified.scratch_bytes] {
        geometry_add(&mut peak, retained)?;
    }
    if peak > limits.resident_bytes || work > limits.work_units {
        return Err(error("aggregate source/model/indexed geometry budget"));
    }
    geometry_poll(control)?;
    let mut staged = Vec::new();
    staged.try_reserve_exact(stage_count).map_err(error)?;
    let mut extents = Vec::new();
    extents.try_reserve_exact(stage_count).map_err(error)?;
    let mut overrun = 0u64;
    check_capacity_overrun(
        staged.capacity().checked_mul(std::mem::size_of::<(LayerId, EditableIndexedRaster, Surface)>()).ok_or_else(|| error("stage capacity overflow"))?,
        stage_count.checked_mul(std::mem::size_of::<(LayerId, EditableIndexedRaster, Surface)>()).ok_or_else(|| error("stage capacity overflow"))?,
        &mut overrun,
        peak,
        limits,
    )?;
    check_capacity_overrun(
        extents.capacity().checked_mul(std::mem::size_of::<(LayerId, Rect)>()).ok_or_else(|| error("extent capacity overflow"))?,
        stage_count.checked_mul(std::mem::size_of::<(LayerId, Rect)>()).ok_or_else(|| error("extent capacity overflow"))?,
        &mut overrun,
        peak,
        limits,
    )?;
    for item in &state.editable_indexed {
        geometry_poll(control)?;
        let target = target_bounds(item.raster.bounds(), operation)?;
        let indices = transform_plane(item.raster.indices(), item.raster.bounds(), target, operation, limits, control, peak, &mut overrun)?;
        let alpha = if let Some(bytes) = item.raster.alpha() {
            let value = transform_plane(bytes, item.raster.bounds(), target, operation, limits, control, peak, &mut overrun)?;
            Some(Arc::new(value))
        } else {
            None
        };
        let mut palette = Vec::new();
        palette.try_reserve_exact(item.raster.palette().len()).map_err(error)?;
        check_capacity_overrun(
            palette.capacity().checked_mul(3).ok_or_else(|| error("palette capacity"))?,
            item.raster.palette().len().checked_mul(3).ok_or_else(|| error("palette capacity"))?,
            &mut overrun,
            peak,
            limits,
        )?;
        palette.extend_from_slice(item.raster.palette());
        let raster =
            EditableIndexedRaster::try_new_with_interrupt(target, Arc::new(indices), alpha, palette, control).map_err(|e| geometry_failure(e, control))?;
        let output = carrier(&raster, doc.color_table.as_ref().and_then(|table| table.transparent), control, peak, limits, &mut overrun)?;
        extents.push((item.owner, target));
        staged.push((item.owner, raster, output));
    }
    // All destinations were uniquely resolved by the borrowed validator. The
    // final token fence is before installation; no fallible authoring follows it.
    geometry_poll(control)?;
    for (id, _, _) in &staged {
        if doc.layer(*id).is_none() || doc.xcf_state.as_ref().is_none_or(|state| !state.editable_indexed.iter().any(|item| item.owner == *id)) {
            return Err(error("indexed destination changed"));
        }
    }
    geometry_poll(control)?;
    for (id, raster, surface) in staged {
        let layer = doc.layer_mut(id).ok_or_else(|| error("missing indexed owner"))?;
        layer.content = LayerContent::Raster(surface);
        let item = doc
            .xcf_state
            .as_mut()
            .and_then(|state| state.editable_indexed.iter_mut().find(|item| item.owner == id))
            .ok_or_else(|| error("missing indexed state"))?;
        item.raster = raster;
    }
    Ok(extents)
}
// Keep the checked resource envelope and original control paired with the borrowed native plane; regrouping would obscure their independent proof inputs.
#[allow(clippy::too_many_arguments)]
fn transform_plane(
    bytes: &[u8],
    bounds: Rect,
    target: Rect,
    operation: IndexedTransform,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
    peak: u64,
    overrun: &mut u64,
) -> Result<Vec<u8>> {
    geometry_poll(control)?;
    if let IndexedTransform::Resize { sx, sy } = operation {
        let input = Surface::try_from_interleaved_with(PixelFormat::GRAY8, bounds, bytes, control).map_err(|e| geometry_failure(e, control))?;
        let (output, actual) = photocraft_algo::wide_resample::try_resize(
            &input,
            bounds,
            photocraft_algo::wide_resample::ResizeOptions { scale_x: sx, scale_y: sy, filter: Resample::Nearest, clamp_edges: true },
            limits,
            control,
        )
        .map_err(|e| geometry_failure(e, control))?;
        if actual != target {
            return Err(error("resize planned extent mismatch"));
        }
        let count = photocraft_raster::checked_region_byte_len(target, PixelFormat::GRAY8).map_err(error)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(count).map_err(error)?;
        check_capacity_overrun(bytes.capacity(), count, overrun, peak, limits)?;
        bytes.resize(count, 0);
        output.try_read_interleaved_into_with(target, &mut bytes, control).map_err(|e| geometry_failure(e, control))?;
        return Ok(bytes);
    }
    let IndexedTransform::Orthogonal { operation, .. } = operation else { return Err(error("unsupported indexed transform")) };
    let count =
        usize::try_from(u64::from(target.width()).checked_mul(u64::from(target.height())).ok_or_else(|| error("plane count overflow"))?).map_err(error)?;
    let mut output = Vec::new();
    output.try_reserve_exact(count).map_err(error)?;
    check_capacity_overrun(output.capacity(), count, overrun, peak, limits)?;
    let width = i64::from(bounds.width());
    let height = i64::from(bounds.height());
    for y in target.y0..target.y1 {
        geometry_poll(control)?;
        for x in target.x0..target.x1 {
            if output.len().is_multiple_of(4096) {
                geometry_poll(control)?;
            }
            let dx = i64::from(x) - i64::from(target.x0);
            let dy = i64::from(y) - i64::from(target.y0);
            let (sx, sy) = match operation {
                OrthogonalTransform::FlipHorizontal => (width - 1 - dx, dy),
                OrthogonalTransform::FlipVertical => (dx, height - 1 - dy),
                OrthogonalTransform::RotateClockwise => (dy, height - 1 - dx),
                OrthogonalTransform::RotateCounterclockwise => (width - 1 - dy, dx),
                OrthogonalTransform::Rotate180 => (width - 1 - dx, height - 1 - dy),
            };
            if sx < 0 || sy < 0 || sx >= width || sy >= height {
                return Err(error("orthogonal source coordinate"));
            }
            let offset =
                sy.checked_mul(width).and_then(|n| n.checked_add(sx)).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| error("orthogonal source offset"))?;
            output.push(*bytes.get(offset).ok_or_else(|| error("orthogonal source length"))?);
        }
    }
    geometry_poll(control)?;
    Ok(output)
}
fn carrier(
    raster: &EditableIndexedRaster,
    transparent: Option<u8>,
    control: &Interrupt<'_>,
    peak: u64,
    limits: GeometryLimits,
    overrun: &mut u64,
) -> Result<Surface> {
    let mut output = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[0; 4]).map_err(error)?;
    let bounds = raster.bounds();
    let width = usize::try_from(bounds.width()).map_err(error)?;
    for coordinate in bounds.tiles() {
        geometry_poll(control)?;
        let area = coordinate.rect().intersect(&bounds);
        let length = photocraft_raster::checked_region_byte_len(area, PixelFormat::RGBA8).map_err(error)?;
        let mut encoded = Vec::new();
        encoded.try_reserve_exact(length).map_err(error)?;
        check_capacity_overrun(encoded.capacity(), length, overrun, peak, limits)?;
        for y in area.y0..area.y1 {
            geometry_poll(control)?;
            for x in area.x0..area.x1 {
                let mut pixel = raster.expanded_pixel(x, y).map_err(error)?;
                let offset = usize::try_from(i64::from(y) - i64::from(bounds.y0))
                    .map_err(error)?
                    .checked_mul(width)
                    .and_then(|n| usize::try_from(i64::from(x) - i64::from(bounds.x0)).ok().and_then(|x| n.checked_add(x)))
                    .ok_or_else(|| error("carrier offset overflow"))?;
                if raster.indices().get(offset).copied() == transparent {
                    pixel[3] = 0;
                }
                encoded.extend_from_slice(&pixel);
            }
        }
        output.try_write_interleaved_with(area, &encoded, control).map_err(|e| geometry_failure(e, control))?;
    }
    output.try_prune_with(control).map_err(|e| geometry_failure(e, control))?;
    geometry_poll(control)?;
    Ok(output)
}
/// Returns transformed extents; callers update item geometry exactly once.
pub(crate) fn resize_nearest(doc: &mut Document, sx: f64, sy: f64, limits: GeometryLimits, control: &Interrupt<'_>) -> Result<Vec<(LayerId, Rect)>> {
    transform(doc, limits, control, IndexedTransform::Resize { sx, sy })
}
/// Permutes raw planes relative to the declared canvas, including outside-canvas pixels.
pub(crate) fn transform_orthogonal(
    doc: &mut Document,
    canvas: Rect,
    operation: OrthogonalTransform,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<Vec<(LayerId, Rect)>> {
    if canvas.is_empty() {
        return Err(error("empty transform canvas"));
    }
    transform(doc, limits, control, IndexedTransform::Orthogonal { canvas, operation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::xcf_state::XcfState;
    use photocraft_doc::{ColorTable, EditableIndexedLayer, Layer, SampleType, Size};
    #[test]
    fn direct_orthogonal_planes_match_existing_raw_sampler_without_palette_search() {
        let doc = source();
        let raster = &doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        let canvas = Rect::new(-3, -2, 5, 7);
        for operation in [
            OrthogonalTransform::FlipHorizontal,
            OrthogonalTransform::FlipVertical,
            OrthogonalTransform::RotateClockwise,
            OrthogonalTransform::RotateCounterclockwise,
            OrthogonalTransform::Rotate180,
        ] {
            let plan = IndexedTransform::Orthogonal { canvas, operation };
            let target = target_bounds(raster.bounds(), plan).unwrap();
            for bytes in [raster.indices().as_slice(), raster.alpha().unwrap().as_slice()] {
                let mut overrun = 0;
                let direct = transform_plane(bytes, raster.bounds(), target, plan, GeometryLimits::default(), &Interrupt::NONE, 0, &mut overrun).unwrap();
                let input = Surface::try_from_interleaved_with(PixelFormat::GRAY8, raster.bounds(), bytes, &Interrupt::NONE).unwrap();
                let (expected, extent) = photocraft_algo::wide_geometry::try_transform_orthogonal(
                    &input,
                    raster.bounds(),
                    (target.x0, target.y0),
                    operation,
                    GeometryLimits::default(),
                    &Interrupt::NONE,
                )
                .unwrap();
                assert_eq!(extent, target);
                assert_eq!(direct, expected.try_read_interleaved_with(target, &Interrupt::NONE).unwrap());
            }
        }
    }
    #[test]
    fn aggregate_geometry_refuses_before_mutation_and_preserves_original_planes() {
        let mut doc = source();
        let before = doc.clone();
        let old = doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices().clone();
        assert!(resize_nearest(&mut doc, 1000.0, 1000.0, GeometryLimits { resident_bytes: 1024, work_units: 1000 }, &Interrupt::NONE).is_err());
        assert_eq!(doc, before);
        assert!(Arc::ptr_eq(&old, doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices()));
        assert!(matches!(
            transform_orthogonal(&mut doc, Rect::new(0, 0, 4, 4), OrthogonalTransform::Rotate180, GeometryLimits::default(), &Interrupt::cancel_only(&|| true)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(doc, before);
    }
    #[test]
    fn current_carrier_validation_preserves_duplicate_ordinals_and_independent_alpha() {
        let doc = source();
        let before = doc.clone();
        let required = validate_current_with_interrupt(&doc, &Interrupt::NONE).unwrap();
        assert_eq!(required.layer_count, 1);
        assert!(required.work_units > 0);
        assert!(required.scratch_bytes >= 1024);
        assert_eq!(doc, before);
        let indices = &doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster.indices();
        assert_eq!(indices.as_slice(), &[0, 1, 2, 1]);
    }
    #[test]
    fn current_carrier_validation_refuses_mismatch_default_tail_and_budget() {
        let mut doc = source();
        let layer = doc.layers.first_mut().unwrap();
        let LayerContent::Raster(surface) = &mut layer.content else { panic!("fixture") };
        surface.try_write_interleaved_with(Rect::new(-1, 1, 0, 2), &[0, 0, 255, 255], &Interrupt::NONE).unwrap();
        assert!(validate_current_with_interrupt(&doc, &Interrupt::NONE).is_err());
        let mut tail = source();
        let LayerContent::Raster(surface) = &mut tail.layers.first_mut().unwrap().content else { panic!("fixture") };
        surface.try_write_interleaved_with(Rect::new(3, 3, 4, 4), &[1, 0, 0, 0], &Interrupt::NONE).unwrap();
        assert!(validate_current_with_interrupt(&tail, &Interrupt::NONE).is_err());
        let mut default = source();
        default.layers.first_mut().unwrap().content = LayerContent::Raster(Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 0, 0, 0]).unwrap());
        assert!(validate_current_with_interrupt(&default, &Interrupt::NONE).is_err());
        assert!(validate_current_with_limits(&source(), GeometryLimits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE).is_err());
        let standard = GeometryLimits::default();
        assert!(
            validate_current_with_limits(
                &source(),
                GeometryLimits { resident_bytes: standard.resident_bytes + 1, work_units: standard.work_units },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(
            validate_current_with_limits(
                &source(),
                GeometryLimits { resident_bytes: standard.resident_bytes, work_units: standard.work_units + 1 },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(validate_current_with_limits(&source(), GeometryLimits { resident_bytes: 1024, work_units: standard.work_units }, &Interrupt::NONE).is_err());
        let mut duplicate = source();
        let plane = duplicate.xcf_state.as_ref().unwrap().editable_indexed[0].clone();
        duplicate.xcf_state.as_mut().unwrap().editable_indexed.push(plane);
        assert!(validate_current_with_interrupt(&duplicate, &Interrupt::NONE).is_err());
        assert!(matches!(validate_current_with_interrupt(&source(), &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }
    fn source() -> Document {
        let raster = EditableIndexedRaster::try_new(
            Rect::new(-1, 1, 1, 3),
            Arc::new(vec![0, 1, 2, 1]),
            Some(Arc::new(vec![255, 128, 64, 7])),
            vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]],
        )
        .unwrap();
        let mut doc = Document::new("Own indexed", Size::new(4, 4), ColorMode::Indexed, SampleType::U8);
        doc.color_table = Some(ColorTable { colors: raster.palette().to_vec(), transparent: Some(1) });
        let surface = crate::xcf_editing::expanded(&raster, PixelFormat::RGBA8, Some(1)).unwrap();
        let layer = Layer::new("Own ordinals", LayerContent::Raster(surface));
        let id = layer.id;
        doc.layers.push(layer);
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(id)).unwrap().raster_bounds = Some(raster.bounds());
        state.editable_indexed.push(EditableIndexedLayer { owner: id, raster });
        doc.xcf_state = Some(state);
        doc
    }
    #[test]
    fn nearest_command_preserves_duplicate_ordinals_hidden_alpha_native_and_undo() {
        let original = source();
        let mut session = crate::Session::new();
        session.add_document(original.clone(), None).unwrap();
        session.execute("image.imageSize", serde_json::json!({"width":8,"height":8,"resample":"nearest"})).unwrap();
        let doc = &session.active().unwrap().doc;
        let r = &doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert_eq!(r.bounds(), Rect::new(-2, 2, 2, 6));
        assert_eq!(r.indices().as_slice(), &[0, 0, 1, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1]);
        assert_eq!(r.alpha().unwrap().as_slice(), &[255, 255, 128, 128, 255, 255, 128, 128, 64, 64, 7, 7, 64, 64, 7, 7]);
        assert_eq!(r.palette(), original.color_table.as_ref().unwrap().colors);
        let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.xcf_state.as_ref().unwrap().editable_indexed[0].raster, *r);
        session.execute("edit.undo", serde_json::json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.xcf_state, original.xcf_state);
        assert!(session.execute("image.imageSize", serde_json::json!({"width":8,"height":8,"resample":"bilinear"})).is_err());
        assert_eq!(session.active().unwrap().doc.xcf_state, original.xcf_state);
    }
    #[test]
    fn orthogonal_current_planes_roundtrip_outside_canvas_without_palette_collapse() {
        let mut doc = source();
        let original = doc.xcf_state.clone();
        let bounds = Rect::new(0, 0, 4, 4);
        let extents = transform_orthogonal(&mut doc, bounds, OrthogonalTransform::RotateClockwise, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        // The geometry caller installs the returned canonical item extents exactly once.
        for (owner, extent) in extents {
            doc.xcf_state
                .as_mut()
                .unwrap()
                .items
                .iter_mut()
                .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
                .unwrap()
                .raster_bounds = Some(extent);
        }
        let r = &doc.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert_eq!(r.bounds(), Rect::new(1, -1, 3, 1));
        assert_eq!(r.indices().as_slice(), &[2, 0, 1, 1]);
        assert_eq!(r.alpha().unwrap().as_slice(), &[64, 255, 7, 128]);
        let extents = transform_orthogonal(&mut doc, bounds, OrthogonalTransform::RotateCounterclockwise, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        // The geometry caller installs the returned canonical item extents exactly once.
        for (owner, extent) in extents {
            doc.xcf_state
                .as_mut()
                .unwrap()
                .items
                .iter_mut()
                .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
                .unwrap()
                .raster_bounds = Some(extent);
        }
        assert_eq!(doc.xcf_state, original);
    }
    #[test]
    fn cancelled_and_bounded_geometry_leave_all_current_state_unchanged() {
        let mut doc = source();
        let original = doc.clone();
        assert!(resize_nearest(&mut doc, 2.0, 2.0, GeometryLimits::default(), &Interrupt::cancel_only(&|| true)).is_err());
        assert_eq!(doc, original);
        assert!(
            transform_orthogonal(
                &mut doc,
                Rect::new(0, 0, 4, 4),
                OrthogonalTransform::FlipHorizontal,
                GeometryLimits { resident_bytes: 0, work_units: 0 },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(doc, original);
    }
}
