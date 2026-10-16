//! Portable owned-current native serialization after complete masked Point proof.
//! This selects structural storage intent, never a path or original Source grant.
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_format::{FormatError, owned_document::OwnedDocumentLimits};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};
use std::io::{Seek, Write};

fn backing(error: BackingError) -> FormatError {
    FormatError::Io(std::io::Error::other(error))
}
fn claims(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<bool, FormatError> {
    c.check(1).map_err(backing)?;
    if !owner.is_readonly_point_storage() {
        return Ok(false);
    }
    let Some(state) = &owner.document().xcf_state else { return Ok(false) };
    if state.filter_stacks.len() > 1 {
        return Err(backing(BackingError::Length));
    }
    let mut found = false;
    for stack in &state.filter_stacks {
        if stack.nodes.len() > 2 {
            return Err(backing(BackingError::Length));
        }
        for node in &stack.nodes {
            for text in [Some(node.operation.schema_epoch.as_str()), node.composition.as_ref().map(|p| p.compatibility_epoch.as_str())].into_iter().flatten() {
                if text.len() > 4096 {
                    return Err(backing(BackingError::Length));
                }
                c.check(text.len() as u64 + 1).map_err(backing)?;
                if text == photocraft_compose::EXPOSURE_MASKED_EPOCH {
                    found = true;
                } else {
                    // Preserve the old Exposure comparison charge; each new
                    // candidate comparison has its own bounded original work.
                    let extra = (text.len() as u64).checked_add(1).and_then(|n| n.checked_mul(4)).ok_or_else(|| backing(BackingError::Length))?;
                    c.check(extra).map_err(backing)?;
                    found |= matches!(
                        text,
                        photocraft_compose::INVERT_LINEAR_MASKED_EPOCH
                            | photocraft_compose::INVERT_GAMMA_MASKED_EPOCH
                            | photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_EPOCH
                            | photocraft_compose::GEGL_LEVELS_MASKED_EPOCH
                    );
                }
            }
        }
    }
    Ok(found)
}

/// Preserve existing writer behavior for every other class. A new masked claim
/// is final: both exact current interpretation and selected storage must prove.
pub fn save_accounted_current_archive<W: Write + Seek>(
    owner: &AccountedDocument,
    output: W,
    limits: OwnedDocumentLimits,
    quota: &BackingQuota,
    control: &ReadControl<'_>,
) -> Result<W, FormatError> {
    if !owner.quota().shares_ledger(quota) {
        return Err(backing(BackingError::Quota));
    }
    if !claims(owner, control)? {
        return photocraft_format::owned_document::save_accounted_current_document_archive(owner, output, limits, quota, control);
    }
    // Covers the fixed relay/first-error witness and error/header stack. The
    // actual current context and serializer reserve their own independent fees.
    let _boundary = quota.reserve(QuotaKind::Resident, 1024).map_err(|_| backing(BackingError::Quota))?;
    let first = std::sync::Mutex::new(None);
    let work = |units| match (control.work)(units) {
        Ok(()) => Ok(()),
        Err(error) => {
            let mut held = first.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if held.is_none() {
                *held = Some(error);
            }
            Err(error)
        }
    };
    let c = ReadControl { interrupt: control.interrupt, work: &work };
    let map = |error: photocraft_compose::backed_normal::BackedNormalError| {
        if let Some(actual) = *first.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            return backing(actual);
        }
        use photocraft_compose::backed_normal::BackedNormalError as E;
        use photocraft_raster::checked_view::RasterViewError as V;
        match error {
            E::Backing(e) | E::View(V::Backed(e) | V::Control(e) | V::Descriptor(e)) => backing(e),
            E::Quota(_) => backing(BackingError::Quota),
            E::Allocation => backing(BackingError::Allocation),
            E::View(V::Resident(photocraft_raster::RasterInputError::Cancelled))
            | E::Color(photocraft_compose::gimp_composite::CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled)) => FormatError::Cancelled,
            E::View(V::Resident(photocraft_raster::RasterInputError::AllocationFailed))
            | E::Color(photocraft_compose::gimp_composite::CompositionError::Raster(photocraft_raster::RasterInputError::AllocationFailed)) => {
                backing(BackingError::Allocation)
            }
            other => FormatError::Io(std::io::Error::other(other)),
        }
    };
    c.check(1).map_err(backing)?;
    let prepared = photocraft_compose::backed_stock_point_context::PreparedBackedStockPointContext::new(
        owner,
        photocraft_cms::Builtin::Srgb.profile(),
        Default::default(),
        &c,
    )
    .map_err(&map)?;
    prepared.validate_operative(&c).map_err(&map)?;
    drop(prepared);
    c.check(1).map_err(backing)?;
    photocraft_format::owned_document::save_accounted_point_f32_coverage_document_archive(owner, output, limits, quota, &c)
}
