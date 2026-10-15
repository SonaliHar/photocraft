//! Original opaque 96x80 Gaussian source intersection. This selector does not
//! execute an operation or change the old eager/default importer. The mapper
//! repeats its existing RasterOriginal schema and complete prepared validation.
use super::*;

pub(super) fn qualify<P: ExactParsedPlane>(file: &XcfFile<P>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    // Existing point_context + legacy_point_candidate, narrowed to the actual
    // singleton Gaussian RasterOriginal geometry and structural owner lane.
    if !(file.version == 22
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && (file.width, file.height) == (96, 80)
        && file.layers.len() == 1
        && file.channels.is_empty()
        && file.paths.is_empty())
    {
        return Ok(false);
    }
    let Some(owner) = file.layers.first() else { return Ok(false) };
    if owner.group
        || owner.path.len() > 1
        || !owner.visible
        || !owner.alpha
        || owner.model != Model::Rgb
        || (owner.width, owner.height) != (96, 80)
        || owner.offset != (0, 0)
        || owner.mode != 28
        || (owner.blend_space, owner.composite_space, owner.composite_mode) != (0, -1, -1)
        || owner.opacity.to_bits() != 1f32.to_bits()
        || owner.mask.is_some()
        || owner.text.is_some()
        || owner.vector.is_some()
        || owner.link.is_some()
        || owner.transform.is_some()
        || owner.floating_attachment.is_some()
        || owner.effects.len() != 1
    {
        return Ok(false);
    }
    poll(ctl, budget, (owner.unsupported.len() as u64).checked_mul(32).ok_or_else(|| IoError::Unsupported("original Gaussian metadata work".into()))?)?;
    if owner.unsupported.iter().any(|s| s != "GEGL layer effects") {
        return Ok(false);
    }
    let Some(effect) = owner.effects.first() else { return Ok(false) };
    let state = &effect.state;
    if effect.operation != "gegl:gaussian-blur"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 5
        || state.enabled != Some(true)
        || state.opacity_u8.is_some()
        || state.opacity_bits != Some(1f32.to_bits())
        || state.mode != Some(62)
        || state.clipping != Some(0)
        || state.region != Some(0)
        || (state.blend_space, state.composite_space, state.composite_mode) != (Some(-856260015), Some(-856260015), Some(-856260015))
    {
        return Ok(false);
    }
    poll(ctl, budget, effect.properties.len() as u64)?;
    if effect.properties.iter().any(|p| !matches!(p.tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 45 | 46)) {
        return Ok(false);
    }
    let mut names = 0u8;
    for arg in &effect.arguments {
        poll(ctl, budget, 32)?;
        if arg.configuration.is_some() || arg.graph.is_some() {
            return Ok(false);
        }
        let flag = match (arg.name.as_str(), &arg.value) {
            ("std-dev-x", ArgumentValue::Float32Bits(bits)) if matches!(f32::from_bits(*bits), 2.25 | 3.75) => 1,
            ("std-dev-y", ArgumentValue::Float32Bits(bits)) if *bits == 1.5f32.to_bits() => 2,
            ("filter", ArgumentValue::Enum(0)) => 4,
            ("abyss-policy", ArgumentValue::Enum(1)) => 8,
            ("clip-extent", ArgumentValue::Boolean(true)) => 16,
            _ => return Ok(false),
        };
        if names & flag != 0 {
            return Ok(false);
        }
        names |= flag;
    }
    if names != 31 {
        return Ok(false);
    }
    let Some(plane) = owner.plane.as_ref() else { return Ok(false) };
    if !native(plane, 4, 96 * 80 * 4, true, ctl, budget)? {
        return Ok(false);
    }
    let Some(mask) = effect.mask.as_ref() else { return Ok(false) };
    if mask.group
        || mask.path.len() > 1
        || mask.model != Model::Gray
        || mask.alpha
        || (mask.width, mask.height) != (96, 80)
        || mask.offset != (0, 0)
        || !mask.visible
        || !mask.mask_enabled
        || mask.mask_shown
        || mask.mask_editing
        || mask.legacy_linked
        || mask.mode != 0
        || (mask.blend_space, mask.composite_space, mask.composite_mode) != (2, 2, 1)
        || !mask.effects.is_empty()
        || mask.mask.is_some()
        || !mask.unsupported.is_empty()
        || mask.text.is_some()
        || mask.vector.is_some()
        || mask.link.is_some()
        || mask.transform.is_some()
        || mask.floating_attachment.is_some()
    {
        return Ok(false);
    }
    let Some(plane) = mask.plane.as_ref() else { return Ok(false) };
    native(plane, 1, 96 * 80, false, ctl, budget)
}

// No eager Plane materialization. Consume every bounded native block even after
// a refused alpha/mask value, so visitor continuation remains charged/pollable.
fn native<P: ExactParsedPlane>(
    plane: &P,
    channels: usize,
    expected: usize,
    opaque: bool,
    ctl: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    if channels == 0 || channels > 4 {
        return Ok(false);
    }
    let descriptor = plane.descriptor();
    if descriptor.sample != Sample::U8
        || descriptor.channels != channels
        || descriptor.width != 96
        || descriptor.height != 80
        || plane.native_byte_len() != Some(expected as u64)
    {
        return Ok(false);
    }
    let mut seen = 0usize;
    let mut valid = true;
    plane.visit_native(ctl, &mut |block| {
        if block.is_empty() || block.len() > 64 * channels || block.len() % channels != 0 {
            return Err(IoError::Unsupported("original Gaussian native block".into()));
        }
        seen = seen.checked_add(block.len()).filter(|n| *n <= expected).ok_or_else(|| IoError::Unsupported("original Gaussian extra native bytes".into()))?;
        poll(ctl, budget, (block.len() + block.len() / channels) as u64)?;
        valid &= if opaque { block.chunks_exact(channels).all(|pixel| pixel.last() == Some(&255)) } else { block.iter().all(|v| *v == 0) };
        Ok(())
    })?;
    if seen != expected {
        return Err(IoError::Unsupported("original Gaussian incomplete native bytes".into()));
    }
    poll(ctl, budget, 0)?;
    Ok(valid)
}

#[cfg(test)]
#[path = "original_gaussian_tests.rs"]
mod tests;
