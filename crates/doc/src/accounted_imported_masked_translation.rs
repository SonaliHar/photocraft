//! Closed assigned CURRENT foreground + linked resident-mask translation.
//! Canonical bounds synchronization is explicit CURRENT fidelity; preserving the
//! entire stored physical map is deliberately stronger than content pruning.
use super::*;
use crate::{Surface, item_identity::ItemRef};
use photocraft_geom::{TILE_SIZE, TileCoord};

// Source and generated surfaces drop before their corresponding quota receipts.
struct PaidMask {
    root_surface: Arc<Surface>,
    surface: Arc<Surface>,
    offset: (i64, i64),
    _index: QuotaLease,
    _payload: QuotaLease,
}
// Derived source references/fees drop before original physical-source receipts.
// Repeat translation replaces this wrapper, never retains a prior model/wrapper.
struct TranslationRetention {
    mask: Arc<PaidMask>,
    root: Arc<dyn Any + Send + Sync>,
    _allocation: QuotaLease,
}
struct Plan {
    translated: crate::BackedLayer,
    mask_item: usize,
    root_surface: Arc<Surface>,
    offset: (i64, i64),
    root_retention: Arc<dyn Any + Send + Sync>,
}
fn shifted(r: Rect, dx: i64, dy: i64) -> Result<Rect, BackingError> {
    let x = |v: i32| i64::from(v).checked_add(dx).and_then(|v| i32::try_from(v).ok()).ok_or(BackingError::Bounds);
    let y = |v: i32| i64::from(v).checked_add(dy).and_then(|v| i32::try_from(v).ok()).ok_or(BackingError::Bounds);
    Ok(Rect::new(x(r.x0)?, y(r.y0)?, x(r.x1)?, y(r.y1)?))
}
fn tile_rect(c: TileCoord) -> Result<Rect, BackingError> {
    let x = c.tx.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
    let y = c.ty.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
    let x1 = x.checked_add(TILE_SIZE).ok_or(BackingError::Bounds)?;
    let y1 = y.checked_add(TILE_SIZE).ok_or(BackingError::Bounds)?;
    if [x, y, x1, y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) {
        return Err(BackingError::Bounds);
    }
    Ok(Rect::new(x, y, x1, y1))
}
fn plan(owner: &AccountedDocument, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<Plan, BackingError> {
    owner.validate_assigned_analytic_geometry_storage(c)?;
    owner.validate_imported_metadata_edit(AccountedEdit::LayerMetadata { layer, name: None, visible: None, opacity: None }, c)?;
    let foreground = owner.document().layers.get(1).ok_or(BackingError::Unsupported)?;
    if foreground.id != layer {
        return Err(BackingError::Unsupported);
    }
    let mask = foreground.mask.as_ref().filter(|m| m.linked).ok_or(BackingError::Unsupported)?;
    let LayerContent::BackedRaster(backed) = &foreground.content else { return Err(BackingError::Unsupported) };
    let translated = backed.try_translated(dx, dy, c).map_err(|e| match e {
        BackedLayerError::Backing(e) => e,
        _ => BackingError::Integrity,
    })?;
    if [translated.operative_bounds().x0, translated.operative_bounds().y0, translated.operative_bounds().x1, translated.operative_bounds().y1]
        .iter()
        .any(|v| !(-(1 << 29)..=(1 << 29)).contains(v))
    {
        return Err(BackingError::Bounds);
    }
    let mut mask_item = None;
    for (index, item) in owner.document().xcf_state.as_ref().ok_or(BackingError::Unsupported)?.items.iter().enumerate() {
        c.check(1)?;
        if item.mask.as_ref().is_some_and(|m| m.owner == ItemRef::Layer(layer)) {
            if mask_item.replace(index).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let (root_surface, offset, root_retention) = if let Some(previous) = owner.owner.retention.downcast_ref::<TranslationRetention>() {
        if !Arc::ptr_eq(&previous.mask.surface, &mask.surface) {
            return Err(BackingError::Integrity);
        }
        (
            previous.mask.root_surface.clone(),
            (
                previous.mask.offset.0.checked_add(i64::from(dx)).ok_or(BackingError::Bounds)?,
                previous.mask.offset.1.checked_add(i64::from(dy)).ok_or(BackingError::Bounds)?,
            ),
            previous.root.clone(),
        )
    } else {
        (mask.surface.clone(), (i64::from(dx), i64::from(dy)), owner.owner.retention.clone())
    };
    // Preflight every stored physical source rectangle and shifted target grid,
    // including off-operative/padding tiles. No data-derived bounds or pruning.
    for (coordinate, _) in root_surface.tiles() {
        c.check(1)?;
        let moved = shifted(tile_rect(*coordinate)?, offset.0, offset.1)?;
        for target in moved.tiles() {
            c.check(1)?;
            tile_rect(target)?;
        }
    }
    Ok(Plan { translated, mask_item: mask_item.ok_or(BackingError::Integrity)?, root_surface, offset, root_retention })
}
fn map_header_bytes(entries: usize, default: usize) -> Result<u64, BackingError> {
    (entries as u64)
        .checked_mul(256)
        .and_then(|n| n.checked_add((default as u64).checked_mul(2)?))
        .and_then(|n| n.checked_add((std::mem::size_of::<Surface>() + std::mem::size_of::<PaidMask>() + 256) as u64))
        .ok_or(BackingError::Length)
}
fn local_offset(tile: Rect, x: i32, y: i32, bpp: usize) -> Result<usize, BackingError> {
    let row = usize::try_from(i64::from(y) - i64::from(tile.y0)).map_err(|_| BackingError::Bounds)?;
    let col = usize::try_from(i64::from(x) - i64::from(tile.x0)).map_err(|_| BackingError::Bounds)?;
    row.checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_add(col)).and_then(|n| n.checked_mul(bpp)).ok_or(BackingError::Bounds)
}
fn translate_mask(plan: &Plan, q: &BackingQuota, c: &ReadControl<'_>) -> Result<Arc<PaidMask>, BackingError> {
    let root = &plan.root_surface;
    let upper = root.tile_count().checked_mul(4).ok_or(BackingError::Length)?;
    let candidate_bytes =
        (upper as u64).checked_mul((2 * std::mem::size_of::<TileCoord>()) as u64).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
    let mut candidates_fee = q.reserve(QuotaKind::Index, candidate_bytes).map_err(|_| BackingError::Quota)?;
    c.check(128)?;
    let mut candidates = Vec::new();
    candidates.try_reserve_exact(upper).map_err(|_| BackingError::Allocation)?;
    if candidates.capacity() > upper.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    let actual =
        (candidates.capacity() as u64).checked_mul(std::mem::size_of::<TileCoord>() as u64).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
    candidates_fee.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
    for (coord, _) in root.tiles() {
        c.check(1)?;
        for target in shifted(tile_rect(*coord)?, plan.offset.0, plan.offset.1)?.tiles() {
            c.check(1)?;
            if candidates.len() >= upper {
                return Err(BackingError::Integrity);
            }
            candidates.push(target);
        }
    }
    let passes = u32::BITS - u32::try_from(candidates.len().max(1)).map_err(|_| BackingError::Length)?.leading_zeros();
    c.check((candidates.len() as u64).checked_mul(u64::from(passes) + 2).and_then(|n| n.checked_mul(16)).ok_or(BackingError::Length)?)?;
    candidates.sort_unstable_by_key(|coord| (coord.ty, coord.tx));
    candidates.dedup();
    if candidates.len() > 4096 {
        return Err(BackingError::Length);
    }
    let bpp = root.format().bytes_per_pixel();
    let tile_bytes = (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_mul(bpp)).ok_or(BackingError::Length)?;
    let index_bytes = map_header_bytes(candidates.len(), root.default_bytes().len())?;
    let index = q.reserve(QuotaKind::Index, index_bytes).map_err(|_| BackingError::Quota)?;
    let payload_bytes = (candidates.len() as u64)
        .checked_mul((tile_bytes as u64).checked_mul(2).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?)
        .ok_or(BackingError::Length)?;
    let mut payload = q.reserve(QuotaKind::Resident, payload_bytes).map_err(|_| BackingError::Quota)?;
    c.check(root.default_bytes().len() as u64 + 1)?;
    let mut surface = Surface::try_with_default_bytes(root.format(), root.default_bytes()).map_err(|_| BackingError::Allocation)?;
    for coord in &candidates {
        let target = tile_rect(*coord)?;
        c.check((tile_bytes as u64).checked_mul(2).ok_or(BackingError::Length)?)?;
        // L0 checks actual temporary Vec capacity <=2*requested before fill.
        let mut tile = surface.try_default_tile().map_err(|_| BackingError::Allocation)?;
        let source = shifted(target, plan.offset.0.checked_neg().ok_or(BackingError::Bounds)?, plan.offset.1.checked_neg().ok_or(BackingError::Bounds)?)?;
        for source_coord in source.tiles() {
            c.check(64)?;
            let Some(original) = root.tile(source_coord) else { continue };
            let original_rect = tile_rect(source_coord)?;
            let part = source.intersect(&original_rect);
            for y in part.y0..part.y1 {
                let moved = shifted(Rect::new(part.x0, y, part.x1, y.checked_add(1).ok_or(BackingError::Bounds)?), plan.offset.0, plan.offset.1)?;
                let from = local_offset(original_rect, part.x0, y, bpp)?;
                let to = local_offset(target, moved.x0, moved.y0, bpp)?;
                let count = (part.width() as usize).checked_mul(bpp).ok_or(BackingError::Length)?;
                c.check(count as u64)?;
                tile.bytes_mut()
                    .get_mut(to..to.checked_add(count).ok_or(BackingError::Length)?)
                    .ok_or(BackingError::Integrity)?
                    .copy_from_slice(original.bytes().get(from..from.checked_add(count).ok_or(BackingError::Length)?).ok_or(BackingError::Integrity)?);
            }
        }
        c.check(256)?;
        surface.put_tiles(std::iter::once((*coord, Arc::new(tile))));
    }
    let retained = (candidates.len() as u64).checked_mul((tile_bytes as u64).checked_add(128).ok_or(BackingError::Length)?).ok_or(BackingError::Length)?;
    payload.reconcile_capacity(retained).map_err(|_| BackingError::Quota)?;
    c.check(1)?;
    Ok(Arc::new(PaidMask { root_surface: root.clone(), surface: Arc::new(surface), offset: plan.offset, _index: index, _payload: payload }))
}
impl AccountedDocument {
    /// Storage-only typed preflight; effective locks, links/selection command
    /// semantics and complete profile/native/render checks belong to the engine.
    pub fn validate_assigned_layer_translation(&self, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<(), BackingError> {
        plan(self, layer, dx, dy, c)?;
        c.check(1)
    }
    pub fn try_with_assigned_layer_translation(&self, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let plan = plan(self, layer, dx, dy, c)?;
        if dx == 0 && dy == 0 {
            c.check(1)?;
            return Ok(self.clone());
        }
        let logical = selection_census(self.document(), false, None, self.quota(), c)?;
        let owner_bytes = (std::mem::size_of::<ModelOwner>() + 128) as u64;
        let index_envelope =
            logical.index.checked_sub(owner_bytes).and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(4096)).ok_or(BackingError::Length)?;
        let payload_envelope = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
        let allocation = self.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
        let mut index = self.quota().reserve(QuotaKind::Index, index_envelope).map_err(|_| BackingError::Quota)?;
        let mut payload = self.quota().reserve(QuotaKind::Resident, payload_envelope).map_err(|_| BackingError::Quota)?;
        let retention_allocation =
            self.quota().reserve(QuotaKind::Index, (std::mem::size_of::<TranslationRetention>() + 128) as u64).map_err(|_| BackingError::Quota)?;
        let mask = translate_mask(&plan, self.quota(), c)?;
        c.check(logical.total()?)?;
        let document = imported_selection::copy_masked_normal(self.document(), self.quota(), c)?;
        let mut staged = ModelStaging { document, selection_view: None };
        let bounds = plan.translated.operative_bounds();
        let target = staged.document.layers.get_mut(1).ok_or(BackingError::Integrity)?;
        target.content = LayerContent::BackedRaster(plan.translated);
        imported_translation::shift_reference(target, dx, dy, c)?;
        target.mask.as_mut().ok_or(BackingError::Integrity)?.surface = mask.surface.clone();
        let state = staged.document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
        for item in &mut state.items {
            c.check(1)?;
            if item.item == ItemRef::Layer(layer) {
                item.raster_bounds = Some(bounds);
            }
        }
        let record = state.items.get_mut(plan.mask_item).ok_or(BackingError::Integrity)?;
        record.raster_bounds = Some(bounds);
        let canonical = record.mask.as_mut().ok_or(BackingError::Integrity)?;
        canonical.bounds = bounds;
        canonical.offset = [bounds.x0, bounds.y0];
        imported_selection::validate_assigned_geometry(&staged.document, self.quota(), c)?;
        let retained = selection_census(&staged.document, true, None, self.quota(), c)?;
        let retained_index = retained.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        if retained_index > index_envelope || retained.payload > payload_envelope {
            return Err(BackingError::Allocation);
        }
        index.reconcile_capacity(retained_index).map_err(|_| BackingError::Quota)?;
        payload.reconcile_capacity(retained.payload).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        let retention = Arc::new(TranslationRetention { mask, root: plan.root_retention, _allocation: retention_allocation });
        let ModelStaging { document, selection_view } = staged;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::Legacy,
                document,
                _selection: None,
                _selection_view: selection_view,
                retention,
                quota: self.quota().clone(),
                _model: Some(ModelCharges { _index: index, _payload: payload }),
                _allocation: Arc::new(allocation),
            }),
        })
    }
}

#[cfg(test)]
#[path = "accounted_imported_masked_translation_tests.rs"]
mod tests;
