//! Exact original two-leaf legacy source preparation, independent of CURRENT.
//! Original pixel-unit proof belongs to the separately bounded I/O qualifier.
use super::*;
use photocraft_doc::import_compositing::{XCF12_FLAT_LEGACY_SOURCE_EPOCH, flat_legacy_source_rule};
pub(super) fn metadata(doc: &Document, interrupt: &Interrupt<'_>, check: &mut impl FnMut(u64) -> Result<(), E>) -> Result<[Recipe; 2], E> {
    let source = true;
    check(1)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let original = doc.source_color_context.as_ref().ok_or_else(scope)?;
    if doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::F32
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. })
        || original.source_precision != 650
        || original.original_icc.is_some()
        || doc.bounds() != Rect::new(0, 0, 16, 12)
        || doc.layers.len() != 2
        || state.items.len() != 2
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !matches!(state.provenance.format_version, 12 | 23)
    {
        return Err(scope());
    }
    check(1)?;
    let profile = Builtin::Srgb.profile();
    check(1)?;
    if !equal(doc.icc_profile.as_deref().map(Vec::as_slice).ok_or_else(scope)?, profile.encoded_bytes().ok_or_else(scope)?, check)? {
        return Err(scope());
    }
    let current = doc.icc_profile.as_deref().map(Vec::as_slice).ok_or_else(scope)?;
    check(current.len() as u64 + 64)?;
    doc.working_profile_interpretation
        .validate_with_interrupt(Some(current), doc.mode, interrupt)
        .map_err(|_| if interrupt.cancelled() { E::Backing(BackingError::Cancelled) } else { scope() })?;
    check(0)?;
    for opaque in &state.opaque {
        check(1)?;
        if opaque.rendering_required {
            return Err(scope());
        }
    }
    let mut out = [Recipe { mode: 28, blend: GimpSpace::LinearRgb, composite: GimpSpace::LinearRgb, operator: GimpOperator::Union }; 2];
    for (index, layer) in doc.layers.iter().enumerate() {
        check(512)?;
        let item = state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
        let extent = item.raster_bounds.ok_or_else(scope)?;
        let format = match &layer.content {
            LayerContent::BackedRaster(backed) if !source && backed.operative_bounds() == extent => backed.format(),
            // Source preparation precedes the move-only native promotion. Its
            // original native plane has already passed the source qualifier.
            LayerContent::Raster(surface) if source => surface.format(),
            _ => return Err(scope()),
        };
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        if policy.compatibility_epoch.len() > 128 {
            return Err(scope());
        }
        check(policy.compatibility_epoch.len() as u64)?;
        let resolved = policy.checked_resolved().map_err(|_| scope())?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if item.parent.is_some()
            || item.visible != layer.visible
            || (source && !layer.visible)
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.blend_if != photocraft_doc::BlendIf::default()
            || policy.legacy != (index == 1)
            || policy.pass_through
            || physical.pass_through
            || physical.blend_encoding != transfer(resolved.blend_space)?
            || physical.composite_encoding != transfer(resolved.composite_space)?
            || format != doc.pixel_format()
            || !format.alpha
        {
            return Err(scope());
        }
        if index == 0 {
            let epoch = if state.provenance.format_version == 12 { "xcf-12-normal" } else { "xcf-23-normal" };
            if policy.source_mode != 28
                || policy.compatibility_epoch != epoch
                || extent != doc.bounds()
                || (source && layer.opacity.to_bits() != 1f32.to_bits())
                || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1)
                || resolved.blend_space != GimpSpace::LinearRgb
                || resolved.composite_space != GimpSpace::LinearRgb
                || resolved.operator != GimpOperator::Union
            {
                return Err(scope());
            }
        } else {
            if !lower::is_supported_legacy_mode(policy.source_mode) {
                return Err(scope());
            }
            let bounds = extent;
            if !(bounds == Rect::new(2, -1, 12, 8) || bounds == Rect::new(-1, 2, 9, 11))
                || layer.opacity.to_bits() != if bounds.x0 == 2 { 0.7125f32.to_bits() } else { 0.4375f32.to_bits() }
            {
                return Err(scope());
            }
            if policy.compatibility_epoch != XCF12_FLAT_LEGACY_SOURCE_EPOCH {
                return Err(scope());
            }
            let rule = flat_legacy_source_rule(
                state.provenance.format_version,
                policy.source_mode,
                policy.stored_blend_space,
                policy.stored_composite_space,
                policy.stored_operator,
            )
            .ok_or_else(scope)?;
            if resolved != &rule.resolved() {
                return Err(scope());
            }
        }
        *out.get_mut(index).ok_or_else(scope)? =
            Recipe { mode: policy.source_mode, blend: resolved.blend_space, composite: resolved.composite_space, operator: resolved.operator };
    }
    if doc.layers.first().map(|l| l.id) == doc.layers.get(1).map(|l| l.id) {
        return Err(scope());
    }
    Ok(out)
}
