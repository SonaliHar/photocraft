//! Placement of the existing same-size, local-zero ordinary layer-mask domain.
//! A raw mask offset is not a world placement and is never silently discarded.
//! This does not admit unequal extents, infer outside coverage, or qualify filters.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Placement {
    pub(super) world_bounds: Rect,
    pub(super) world_offset: [i32; 2],
}

/// Allocation-free descriptor/length checks precede owner or mask pixel copies.
/// Decoder-owned native bytes remain borrowed; Half has already widened to F32.
pub(super) fn validate<P: MappingPlane>(owner: &Drawable<P>, source_sample: Sample, ctl: &Interrupt<'_>) -> Result<Option<Placement>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let Some(mask) = &owner.mask else { return Ok(None) };
    if mask.offset != (0, 0) {
        return Err(unsupported(mask, "nonzero local layer-mask offset is unqualified"));
    }
    if mask.width == 0 || mask.height == 0 || mask.width != owner.width || mask.height != owner.height {
        return Err(unsupported(mask, "unequal layer-mask geometry is unqualified"));
    }
    if mask.model != Model::Gray || mask.alpha || mask.group {
        return Err(unsupported(mask, "layer mask requires native non-alpha Gray pixels"));
    }
    let plane = mask.plane.as_ref().ok_or_else(|| unsupported(mask, "missing layer-mask plane"))?;
    let actual = plane.descriptor();
    let sample = if source_sample == Sample::F16 { Sample::F32 } else { source_sample };
    if actual.width != mask.width || actual.height != mask.height || actual.channels != 1 || actual.sample != sample {
        return Err(unsupported(mask, "layer-mask native descriptor mismatch"));
    }
    let expected = u64::from(mask.width).checked_mul(u64::from(mask.height))
        .and_then(|n| n.checked_mul(sample.source_bytes() as u64))
        .ok_or_else(|| unsupported(mask, "layer-mask native length overflow"))?;
    if plane.native_byte_len() != Some(expected) {
        return Err(unsupported(mask, "layer-mask native length/format mismatch"));
    }
    let world_bounds = bounds(owner)?;
    if world_bounds.is_empty() || world_bounds.width() != mask.width || world_bounds.height() != mask.height {
        return Err(unsupported(mask, "layer-mask world placement mismatch"));
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(Some(Placement { world_bounds, world_offset: [world_bounds.x0, world_bounds.y0] }))
}

#[cfg(test)]
#[path = "layer_mask_geometry_tests.rs"]
mod tests;
