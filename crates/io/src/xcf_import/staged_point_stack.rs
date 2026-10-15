//! Exact original witnessed1..2 Point-stack source, separate from singleton
//! PointReplace families. This chooses a strict validator, never a retry.
use super::*;

pub(super) fn hint<P: ExactParsedPlane>(file: &XcfFile<P>) -> bool {
    file.layers.first().is_some_and(|d| !d.effects.is_empty() && (d.blend_space, d.composite_space, d.composite_mode) == (0, 2, 1))
}
fn poll(ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>, units: u64) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Some(budget) = budget {
        budget.consume_work(units, "witnessed Point stack source").map_err(accounting_error)?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)
}
/// Metadata and native byte predicate from qualified_point_stack_edits, over
/// real bounded plane visits. Eager/default predicate is intentionally unchanged.
/// Optional effect masks retain their old semantics; the five existing observed
/// originals all contain one mask per node, which is not evidence for absence.
pub(super) fn qualify<P: ExactParsedPlane>(file: &XcfFile<P>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    if !(file.version == 22
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && (file.width, file.height) == (8, 6)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1)
    {
        return Ok(false);
    }
    let Some(d) = file.layers.first() else {
        return Ok(false);
    };
    if !(!d.group
        && d.path.len() <= 1
        && d.visible
        && d.alpha
        && d.model == Model::Rgb
        && (d.width, d.height) == (8, 6)
        && d.offset == (0, 0)
        && d.mode == 28
        && (d.blend_space, d.composite_space, d.composite_mode) == (0, 2, 1)
        && d.opacity.to_bits() == 1.0_f32.to_bits()
        && d.mask.is_none()
        && d.text.is_none()
        && d.vector.is_none()
        && d.link.is_none()
        && d.transform.is_none()
        && d.floating_attachment.is_none()
        && matches!(d.effects.len(), 1..=2)
        && d.effects.iter().all(|e| matches!(e.operation.as_str(), "gegl:exposure" | "gegl:invert-linear")))
    {
        return Ok(false);
    }
    let Some(plane) = d.plane.as_ref() else {
        return Ok(false);
    };
    let descriptor = plane.descriptor();
    if descriptor.sample != Sample::U8 || descriptor.channels != 4 || plane.native_byte_len() != Some(8 * 6 * 4) {
        return Ok(false);
    }
    let mut seen = 0usize;
    plane.visit_native(ctl, &mut |bytes| {
        if bytes.is_empty() || bytes.len() > 256 || bytes.len() % 4 != 0 {
            return Err(IoError::Unsupported("Point stack native block".into()));
        }
        seen = seen.checked_add(bytes.len()).filter(|n| *n <= 192).ok_or_else(|| IoError::Unsupported("Point stack extra bytes".into()))?;
        poll(ctl, budget, bytes.len() as u64)
    })?;
    if seen != 192 {
        return Err(IoError::Unsupported("Point stack incomplete native bytes".into()));
    }
    poll(ctl, budget, 0)?;
    Ok(true)
}

#[cfg(test)]
#[path = "staged_point_stack_tests.rs"]
mod tests;
