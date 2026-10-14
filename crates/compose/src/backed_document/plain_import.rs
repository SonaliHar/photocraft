//! Current plain imported storage display only. This consumes a genuine paid
//! model owner and existing resolved Normal policy; it grants no source epoch,
//! filter, general importer or write-back authority.
//! Policies below are current canonical metadata, not a raw-XCF correspondence
//! proof. Immutable original bytes/provenance remain owned and uninterpreted.
use super::*;
use photocraft_doc::{SampleType, accounted_document::AccountedDocument, import_compositing::GimpOperator, item_identity::ItemRef};

fn scope() -> BackedNormalError {
    BackedNormalError::Unsupported("closed accounted plain imported Normal stack")
}
fn same_bytes(a: &[u8], b: &[u8], control: &ReadControl<'_>) -> Result<bool, BackedNormalError> {
    control.check(1).map_err(BackedNormalError::Backing)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.chunks(4096).zip(b.chunks(4096)) {
        control.check(a.len() as u64).map_err(BackedNormalError::Backing)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn encoding(space: GimpSpace) -> Result<SampleEncoding, BackedNormalError> {
    match space {
        GimpSpace::LinearRgb => Ok(SampleEncoding::Linear),
        GimpSpace::ProfileRgb => Ok(SampleEncoding::NonlinearProfile),
        GimpSpace::PerceptualRgb => Ok(SampleEncoding::Perceptual),
        GimpSpace::Lab => Err(scope()),
    }
}
fn profile(doc: &Document, definition: &Profile, control: &ReadControl<'_>) -> Result<(), BackedNormalError> {
    let current = doc.icc_profile.as_ref().ok_or_else(scope)?;
    if !(132..=16 << 20).contains(&current.len())
        || definition.channels() != if doc.mode == ColorMode::Rgb { 3 } else { 1 }
        || !same_bytes(current, definition.encoded_bytes().ok_or_else(scope)?, control)?
    {
        return Err(scope());
    }
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    match &doc.working_profile_interpretation {
        WorkingProfileInterpretation::Icc => {
            if !same_bytes(source.original_icc.as_ref().ok_or_else(scope)?, current, control)? {
                return Err(scope());
            }
        }
        WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } | WorkingProfileInterpretation::ImplicitSGray { icc_blake3 } => {
            if source.original_icc.is_some() || icc_blake3.len() != 64 {
                return Err(scope());
            }
            let builtin = match (&doc.working_profile_interpretation, doc.mode) {
                (WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb) => photocraft_cms::Builtin::Srgb,
                (WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale) => photocraft_cms::Builtin::SGray,
                _ => return Err(scope()),
            };
            // Prepay the existing tokened marker/hash validator before hashing;
            // no new CMS/profile parsing or hash implementation is introduced.
            control.check(64).map_err(BackedNormalError::Backing)?;
            for bytes in current.chunks(4096) {
                control.check(bytes.len() as u64).map_err(BackedNormalError::Backing)?;
            }
            let valid = doc.working_profile_interpretation.validate_with_interrupt(Some(current), doc.mode, control.interrupt);
            control.check(0).map_err(BackedNormalError::Backing)?;
            valid.map_err(|_| scope())?;
            // Existing builtin OnceLock initialization is an outer-polled
            // provider boundary. Compare its actual encoded definition, not a
            // name/date/transfer alias, and never materialize uncached bytes.
            control.check(1).map_err(BackedNormalError::Backing)?;
            let known = builtin.profile();
            control.check(1).map_err(BackedNormalError::Backing)?;
            if !same_bytes(current, known.encoded_bytes().ok_or_else(scope)?, control)? {
                return Err(scope());
            }
        }
    }
    Ok(())
}
pub(super) fn qualify(owner: &AccountedDocument, definition: &Profile, control: &ReadControl<'_>) -> Result<GimpSpace, BackedNormalError> {
    control.check(1).map_err(BackedNormalError::Backing)?;
    let doc = owner.document();
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let pixels = u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(scope)?;
    if doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc.size.width == 0
        || doc.size.height == 0
        || doc.size.width > 262144
        || doc.size.height > 262144
        || pixels > 1 << 30
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(
            (source.source_precision, doc.depth),
            (150, SampleType::U8) | (250, SampleType::U16) | (350, SampleType::U32) | (550 | 650, SampleType::F32) | (750, SampleType::F64)
        )
        || doc.layers.is_empty()
        || doc.layers.len() > 4096
        || state.items.len() != doc.layers.len()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || state.opaque.len() > 4096
    {
        return Err(scope());
    }
    for raw in &state.opaque {
        control.check(1).map_err(BackedNormalError::Backing)?;
        if raw.rendering_required {
            return Err(scope());
        }
    }
    // This validates actual paid model/current-state storage and any anonymous
    // selection guard. A bare Document or imported arbitrary selection is not a
    // substitute for the owner. No native source tile is read by this census.
    owner.validate_selection_replacement(control).map_err(BackedNormalError::Backing)?;
    profile(doc, definition, control)?;
    let mut composite = None;
    for layer in &doc.layers {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let LayerContent::BackedRaster(backed) = &layer.content else {
            return Err(scope());
        };
        if !owner.quota().shares_ledger(backed.quota())
            || backed.format().sample != doc.depth
            || backed.format().mode != doc.mode
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
            || layer.indexed_source.is_some()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
            || layer.blend != BlendMode::Normal
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
        {
            return Err(scope());
        }
        let mut found = None;
        for item in &state.items {
            control.check(1).map_err(BackedNormalError::Backing)?;
            if item.item == ItemRef::Layer(layer.id) {
                if found.is_some() {
                    return Err(scope());
                }
                found = Some(item);
            }
        }
        let item = found.ok_or_else(scope)?;
        if item.parent.is_some()
            || item.mask.is_some()
            || item.source_flags != 0
            || item.raster_bounds != Some(backed.operative_bounds())
            || item.visible != layer.visible
            || !same_bytes(item.name.as_bytes(), layer.name.as_bytes(), control)?
        {
            return Err(scope());
        }
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        control.check(policy.compatibility_epoch.len() as u64).map_err(BackedNormalError::Backing)?;
        let resolved = policy.checked_resolved().map_err(|_| scope())?;
        if !matches!(policy.source_mode, 0 | 28)
            || policy.legacy != (policy.source_mode == 0)
            || policy.pass_through
            || resolved.operator != GimpOperator::Union
        {
            return Err(scope());
        }
        let blend = encoding(resolved.blend_space)?;
        let space = encoding(resolved.composite_space)?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if physical.pass_through || physical.blend_encoding != blend || physical.composite_encoding != space {
            return Err(scope());
        }
        if composite.is_some_and(|old| old != resolved.composite_space) {
            return Err(BackedNormalError::Unsupported("plain imported stack has different composite spaces"));
        }
        composite = Some(resolved.composite_space);
    }
    composite.ok_or_else(scope)
}

#[cfg(test)]
#[path = "plain_import_tests.rs"]
mod tests;
