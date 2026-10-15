//! Closed isolated Normal group storage and title custody only. This validates
//! owned structure, never original-source, profile interpretation or rendering.
use super::*;
use crate::import_compositing::{GimpOperator, GimpSpace};
use crate::{ColorMode, PixelFormat, SampleEncoding, SampleType, item_identity::ItemRef};
const EPOCH: &str = "own-accounted-isolated-normal-group-v1";
fn fail() -> BackingError {
    BackingError::Unsupported
}
fn bounds(r: Rect) -> Result<(), BackingError> {
    if r.is_empty() || r.width() > 320 || r.height() > 320 || [r.x0, r.y0, r.x1, r.y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) {
        return Err(BackingError::Bounds);
    }
    Ok(())
}
pub(super) fn validate(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let s = doc.xcf_state.as_ref().ok_or_else(fail)?;
    let source = doc.source_color_context.as_ref().ok_or_else(fail)?;
    let current = doc.icc_profile.as_deref().ok_or_else(fail)?;
    let original = source.original_icc.as_deref().ok_or_else(fail)?;
    if doc.layers.len() != 2
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!((doc.depth, source.source_precision), (SampleType::U16, 250) | (SampleType::F32, 650) | (SampleType::F64, 750))
        || !(132..=65536).contains(&current.len())
        || original.len() != current.len()
        || s.provenance.format != "xcf"
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !s.source_items.is_empty()
        || !s.filter_stacks.is_empty()
        || !s.filter_masks.is_empty()
        || !s.editable_indexed.is_empty()
        || s.floating.is_some()
        || s.opaque.len() > 4096
        || s.resources.len() > 4096
        || s.item_sets.len() > 4096
        || s.selection.selected.len() > 5
        || !s.selection.selected_paths.is_empty()
        || s.selection.mask_target.is_some()
    {
        return Err(fail());
    }
    bounds(doc.bounds())?;
    for (a, b) in current.chunks(4096).zip(original.chunks(4096)) {
        c.check((a.len() as u64).checked_mul(2).ok_or(BackingError::Length)?)?;
        if a != b {
            return Err(BackingError::Integrity);
        }
    }
    let base = doc.layers.first().ok_or_else(fail)?;
    let top = doc.layers.get(1).ok_or_else(fail)?;
    let LayerContent::Group(g) = &top.content else { return Err(fail()) };
    if g.children.len() != 2
        || g.artboard.is_some()
        || base.mask.is_some()
        || s.items.len() != 4 + usize::from(top.mask.is_some())
        || s.provenance.format_version != if top.mask.is_some() { 13 } else { 12 }
    {
        return Err(fail());
    }
    let lower = g.children.first().ok_or_else(fail)?;
    let upper = g.children.get(1).ok_or_else(fail)?;
    if lower.mask.is_some() || upper.mask.is_some() {
        return Err(fail());
    }
    let layers = [base, top, lower, upper];
    let mut extents = [doc.bounds(); 4];
    for (index, layer) in layers.iter().enumerate() {
        c.check(1)?;
        if layer.id.0 == 0 {
            return Err(BackingError::Integrity);
        }
        for previous in layers.get(..index).ok_or(BackingError::Length)? {
            c.check(1)?;
            if previous.id == layer.id {
                return Err(BackingError::Integrity);
            }
        }
        let mut found = None;
        for item in &s.items {
            c.check(1)?;
            if item.item == ItemRef::Layer(layer.id) {
                if found.is_some() {
                    return Err(BackingError::Integrity);
                }
                found = Some(item);
            }
        }
        let item = found.ok_or(BackingError::Integrity)?;
        let extent = item.raster_bounds.ok_or_else(fail)?;
        bounds(extent)?;
        *extents.get_mut(index).ok_or(BackingError::Length)? = extent;
        if item.parent != if index < 2 { None } else { Some(ItemRef::Layer(top.id)) }
            || item.mask.is_some()
            || item.visible != layer.visible
            || layer.blend != crate::BlendMode::Normal
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
            || layer.indexed_source.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
            || layer.import_compositing.is_some()
        {
            return Err(fail());
        }
        let policy = item.composition.as_ref().ok_or_else(fail)?;
        c.check(policy.compatibility_epoch.len() as u64)?;
        if policy.compatibility_epoch != EPOCH
            || policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, 1, 1)
        {
            return Err(fail());
        }
        let resolved = policy.checked_resolved().map_err(|_| BackingError::Integrity)?;
        if resolved.blend_space != GimpSpace::LinearRgb || resolved.composite_space != GimpSpace::LinearRgb || resolved.operator != GimpOperator::Union {
            return Err(fail());
        }
        if index != 1 {
            let LayerContent::BackedRaster(b) = &layer.content else { return Err(fail()) };
            if b.format() != PixelFormat::new(doc.mode, doc.depth, true) || !q.shares_ledger(b.quota()) || b.operative_bounds() != extent {
                return Err(BackingError::Integrity);
            }
        }
    }
    if extents[0] != doc.bounds() || extents[2].union(&extents[3]) != extents[1] {
        return Err(fail());
    }
    if let Some(mask) = &top.mask {
        let mut record = None;
        for item in &s.items {
            c.check(1)?;
            if item.mask.as_ref().is_some_and(|m| m.owner == ItemRef::Layer(top.id)) {
                if record.is_some() {
                    return Err(BackingError::Integrity);
                }
                record = Some(item);
            }
        }
        let item = record.ok_or(BackingError::Integrity)?;
        let m = item.mask.as_ref().ok_or(BackingError::Integrity)?;
        if !matches!(item.item,ItemRef::Mask(id) if id.0!=0)
            || item.parent.is_some()
            || item.composition.is_some()
            || item.raster_bounds != Some(extents[1])
            || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
            || mask.surface.tile_count() > 64
            || mask.density.to_bits() != 1f32.to_bits()
            || mask.feather.to_bits() != 0f32.to_bits()
            || m.bounds != extents[1]
            || m.offset != [extents[1].x0, extents[1].y0]
            || m.outside_f64_bits != 0f64.to_bits()
            || m.enabled != mask.enabled
            || m.linked != mask.linked
            || m.shown
            || m.editing
        {
            return Err(fail());
        }
        validate_shared_mask(mask, &mut 0, c)?;
    }
    let reference = |id: ItemRef| -> Result<(), BackingError> {
        for item in &s.items {
            c.check(1)?;
            if item.item == id {
                return Ok(());
            }
        }
        Err(BackingError::Integrity)
    };
    for id in &s.selection.selected {
        reference(*id)?;
    }
    if let Some(id) = s.selection.active {
        reference(id)?;
    }
    for raw in &s.opaque {
        c.check(1)?;
        if raw.rendering_required {
            return Err(fail());
        }
        if let Some(id) = raw.owner {
            reference(id)?;
        }
    }
    for (index, resource) in s.resources.iter().enumerate() {
        c.check(1)?;
        if resource.id.0 == 0 {
            return Err(BackingError::Integrity);
        }
        for previous in s.resources.get(..index).ok_or(BackingError::Length)? {
            c.check(1)?;
            if previous.id == resource.id {
                return Err(BackingError::Integrity);
            }
        }
    }
    for set in &s.item_sets {
        c.check(1)?;
        if set.family != crate::xcf_editing::ItemFamily::Layer || set.members.len() > 4 {
            return Err(fail());
        }
        for id in &set.members {
            if !matches!(id, ItemRef::Layer(_)) {
                return Err(fail());
            }
            reference(*id)?;
        }
    }
    c.check(1)?;
    Ok(())
}
impl AccountedDocument {
    /// Actual paid storage/title custody only; profile, renderer and original
    /// source acceptance remain independent. No projection replaces the leaves.
    pub fn validate_assigned_isolated_normal_group_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.owner._storage_class != StorageClass::Legacy || self.owner._selection.is_some() || self.owner._selection_view.is_some() {
            return Err(fail());
        }
        validate(self.document(), self.quota(), c)?;
        selection_census(self.document(), true, None, self.quota(), c)?;
        Ok(())
    }
}
#[cfg(test)]
#[path = "accounted_imported_isolated_group_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "accounted_imported_isolated_group_edit_tests.rs"]
mod edit_tests;
