//! Closed structural file-title custody for assigned masked Normal geometry.
//! No source, profile interpretation or renderer proof is granted.
use super::*;
use crate::{
    ColorMode, PixelFormat, SampleEncoding, SampleType,
    import_compositing::{
        GimpOperator, GimpSpace, XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
    },
    item_identity::ItemRef,
};

fn bounds(value: Rect) -> Result<(), BackingError> {
    if value.is_empty()
        || value.width() > 262144
        || value.height() > 262144
        || u64::from(value.width()).checked_mul(u64::from(value.height())).is_none_or(|n| n > 1 << 30)
        || [value.x0, value.y0, value.x1, value.y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v))
    {
        return Err(BackingError::Bounds);
    }
    Ok(())
}
pub(super) fn validate(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    validate_with(doc, q, c, false)
}
pub(super) fn validate_assigned(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    validate_with(doc, q, c, true)
}
fn validate_with(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>, transfer_override: bool) -> Result<(), BackingError> {
    c.check(1)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    let source = doc.source_color_context.as_ref().ok_or(BackingError::Unsupported)?;
    let current = doc.icc_profile.as_deref().ok_or(BackingError::Unsupported)?;
    let original = source.original_icc.as_deref().ok_or(BackingError::Unsupported)?;
    let (stored, epoch) = match (doc.sample_encoding, source.source_precision, doc.depth) {
        (Some(SampleEncoding::NonlinearProfile), 250, SampleType::U16) | (Some(SampleEncoding::NonlinearProfile), 350, SampleType::U32) => {
            (SampleEncoding::NonlinearProfile, XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH)
        }
        (Some(SampleEncoding::NonlinearProfile), 650, SampleType::F32) | (Some(SampleEncoding::NonlinearProfile), 750, SampleType::F64) => {
            (SampleEncoding::NonlinearProfile, XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH)
        }
        (Some(SampleEncoding::Linear), 600, SampleType::F32) | (Some(SampleEncoding::Linear), 700, SampleType::F64) if transfer_override => {
            (SampleEncoding::Linear, XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH)
        }
        (Some(SampleEncoding::Perceptual), 675, SampleType::F32) | (Some(SampleEncoding::Perceptual), 775, SampleType::F64) if transfer_override => {
            (SampleEncoding::Perceptual, XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH)
        }
        _ => return Err(BackingError::Unsupported),
    };
    if doc.layers.len() != 2
        || state.items.len() != 3
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
        || !(132..=65536).contains(&current.len())
        || !(132..=65536).contains(&original.len())
        || (stored == SampleEncoding::NonlinearProfile && current.len() != original.len())
        || state.provenance.format != "xcf"
        || state.provenance.format_version != 12
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.opaque.len() > 4096
        || state.selection.selected.len() > 3
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
    {
        return Err(BackingError::Unsupported);
    }
    // Nonlinear retains the exact old equality rule. Transfer overrides retain
    // both independently bounded byte owners; a separate CMS consumer must
    // prove the original -> effective-current relation before execution.
    if stored == SampleEncoding::NonlinearProfile {
        for (a, b) in current.chunks(4096).zip(original.chunks(4096)) {
            c.check(a.len() as u64)?;
            if a != b {
                return Err(BackingError::Integrity);
            }
        }
    }
    bounds(doc.bounds())?;
    if doc.layers.first().zip(doc.layers.get(1)).is_none_or(|(a, b)| a.id.0 == 0 || b.id.0 == 0 || a.id == b.id) {
        return Err(BackingError::Integrity);
    }
    for (index, layer) in doc.layers.iter().enumerate() {
        c.check(1)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(BackingError::Unsupported) };
        if !backed.quota().shares_ledger(q) || backed.format() != PixelFormat::new(doc.mode, doc.depth, true) {
            return Err(BackingError::Integrity);
        }
        let extent = backed.operative_bounds();
        bounds(extent)?;
        if !layer.visible
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.blend != photocraft_color::BlendMode::Normal
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
        {
            return Err(BackingError::Unsupported);
        }
        if layer
            .import_compositing
            .as_ref()
            .is_none_or(|p| p.pass_through || p.blend_encoding != SampleEncoding::Linear || p.composite_encoding != SampleEncoding::Linear)
        {
            return Err(BackingError::Unsupported);
        }
        let mut metadata = None;
        for item in &state.items {
            c.check(1)?;
            if item.item == ItemRef::Layer(layer.id) {
                if metadata.is_some() {
                    return Err(BackingError::Integrity);
                }
                metadata = Some(item);
            }
        }
        let item = metadata.ok_or(BackingError::Integrity)?;
        let policy = item.composition.as_ref().ok_or(BackingError::Unsupported)?;
        c.check(policy.compatibility_epoch.len() as u64)?;
        if item.parent.is_some()
            || item.mask.is_some()
            || item.source_flags != 0
            || !item.visible
            || item.raster_bounds != Some(extent)
            || policy.compatibility_epoch != epoch
            || policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || policy.stored_blend_space != 0
            || policy.stored_composite_space != if index == 0 { -1 } else { 1 }
            || if index == 0 {
                policy.stored_operator != -1
            } else if transfer_override {
                !(1..=4).contains(&policy.stored_operator)
            } else {
                policy.stored_operator != 1
            }
        {
            return Err(BackingError::Unsupported);
        }
        let resolved = policy.checked_resolved().map_err(|_| BackingError::Integrity)?;
        if resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || ((index == 0 || !transfer_override) && resolved.operator != GimpOperator::Union)
        {
            return Err(BackingError::Unsupported);
        }
        if index == 0 {
            if extent != doc.bounds() || layer.opacity.to_bits() != 1f32.to_bits() || layer.mask.is_some() {
                return Err(BackingError::Unsupported);
            }
        } else {
            let mask = layer.mask.as_ref().ok_or(BackingError::Unsupported)?;
            let mut record = None;
            for candidate in &state.items {
                c.check(1)?;
                if candidate.mask.as_ref().is_some_and(|v| v.owner == ItemRef::Layer(layer.id)) {
                    if record.is_some() {
                        return Err(BackingError::Integrity);
                    }
                    record = Some(candidate);
                }
            }
            let record = record.ok_or(BackingError::Integrity)?;
            let ItemRef::Mask(id) = record.item else { return Err(BackingError::Integrity) };
            let m = record.mask.as_ref().ok_or(BackingError::Integrity)?;
            if id.0 == 0
                || record.parent.is_some()
                || record.composition.is_some()
                || record.raster_bounds != Some(extent)
                || !mask.enabled
                || mask.density.to_bits() != 1f32.to_bits()
                || mask.feather.to_bits() != 0f32.to_bits()
                || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
                || m.bounds != extent
                || m.offset != [extent.x0, extent.y0]
                || m.outside_f64_bits != 0f64.to_bits()
                || !m.enabled
                || m.shown
                || m.editing
                || m.linked != mask.linked
            {
                return Err(BackingError::Unsupported);
            }
            super::super::validate_shared_mask(mask, &mut 0, c)?;
        }
    }
    let reference = |id: ItemRef| -> Result<(), BackingError> {
        for item in &state.items {
            c.check(1)?;
            if item.item == id {
                return Ok(());
            }
        }
        Err(BackingError::Integrity)
    };
    for id in &state.selection.selected {
        reference(*id)?;
    }
    if let Some(id) = state.selection.active {
        reference(id)?;
    }
    for raw in &state.opaque {
        c.check(1)?;
        if raw.rendering_required {
            return Err(BackingError::Unsupported);
        }
        if let Some(id) = raw.owner {
            reference(id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "accounted_imported_masked_normal_tests.rs"]
mod tests;

impl AccountedDocument {
    /// Actual paid model/title-copy storage validation only. Neither analytic
    /// profile interpretation, original XCF eligibility nor rendering is proven.
    pub fn validate_masked_analytic_normal_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.owner._storage_class != StorageClass::Legacy || self.owner._selection.is_some() || self.owner._selection_view.is_some() {
            return Err(BackingError::Unsupported);
        }
        validate(self.document(), self.quota(), c)?;
        super::super::selection_census(self.document(), true, None, self.quota(), c)?;
        Ok(())
    }

    /// Structural custody for the three closed stored interpretations. Original
    /// and current ICC are retained independently for Linear/Perceptual; this
    /// does not establish their colorimetric relationship or rendering validity.
    pub fn validate_assigned_analytic_geometry_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.owner._storage_class != StorageClass::Legacy || self.owner._selection.is_some() || self.owner._selection_view.is_some() {
            return Err(BackingError::Unsupported);
        }
        validate_assigned(self.document(), self.quota(), c)?;
        super::super::selection_census(self.document(), true, None, self.quota(), c)?;
        Ok(())
    }
}
