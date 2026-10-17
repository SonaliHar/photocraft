//! Exact existing two-plane legacy source intersection; never a general retry.
use super::*;
pub(super) fn mode(value: i32) -> bool {
    matches!(value, 3 | 4 | 6..=21)
}
pub(super) fn geometry<P: ExactParsedPlane>(f: &XcfFile<P>) -> bool {
    geometry_model(f, Model::Rgb)
}
pub(super) fn geometry_model<P: ExactParsedPlane>(f: &XcfFile<P>, model: Model) -> bool {
    if !matches!(model, Model::Rgb | Model::Gray) {
        return false;
    }
    if f.version != 12
        || f.model != model
        || f.sample != Sample::F32
        || f.precision != 650
        || f.encoding != Encoding::NonlinearProfile
        || !f.icc.is_empty()
        || f.width != 16
        || f.height != 12
        || f.layers.len() != 2
        || !f.channels.is_empty()
        || !f.paths.is_empty()
    {
        return false;
    }
    if !f.layers.iter().all(|d| {
        !d.group
            && d.path.len() <= 1
            && d.effects.is_empty()
            && d.visible
            && d.floating_attachment.is_none()
            && d.mask.is_none()
            && d.model == model
            && d.alpha
            && d.text.is_none()
            && d.vector.is_none()
            && d.link.is_none()
            && d.transform.is_none()
            && d.plane.as_ref().is_some_and(|p| p.descriptor().sample == Sample::F32 && p.descriptor().channels == if model == Model::Gray { 2 } else { 4 })
    }) {
        return false;
    }
    let (Some(top), Some(base)) = (f.layers.first(), f.layers.get(1)) else { return false };
    top.width == 10
        && top.height == 9
        && ((top.offset == (2, -1) && top.opacity.to_bits() == 0.7125f32.to_bits()) || (top.offset == (-1, 2) && top.opacity.to_bits() == 0.4375f32.to_bits()))
        && base.mode == 28
        && base.width == 16
        && base.height == 12
        && base.offset == (0, 0)
        && base.opacity == 1.0
        && base.blend_space == 0
        && base.composite_space == -1
        && base.composite_mode == -1
}
pub(super) fn domain<P: ExactParsedPlane>(f: &XcfFile<P>) -> bool {
    geometry(f)
        && f.layers.first().is_some_and(|top| {
            photocraft_doc::import_compositing::flat_legacy_source_rule(f.version, top.mode, top.blend_space, top.composite_space, top.composite_mode).is_some()
        })
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !domain(f) {
        return Ok(false);
    }
    native(f, ctl, budget)
}
// Caller has selected an exact bounded two-plane metadata domain before this
// common unit visitor. Source activity never skips operative native words.
pub(super) fn native<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    native_layout(f, 4, ctl, budget)
}
pub(super) fn native_layout<P: ExactParsedPlane>(
    f: &XcfFile<P>,
    channels: usize,
    ctl: &Interrupt<'_>,
    budget: &photocraft_xcf::Budget<'_>,
) -> Result<bool, IoError> {
    if !matches!(channels, 2 | 4) {
        return Err(IoError::Unsupported("strict source native channel count".into()));
    }
    let pixel_bytes = channels * 4;
    let mut valid = true;
    for d in &f.layers {
        pay(ctl, budget, 1)?;
        let Some(p) = d.plane.as_ref() else { return Ok(false) };
        let desc = p.descriptor();
        let expected = u64::from(d.width)
            .checked_mul(u64::from(d.height))
            .and_then(|n| n.checked_mul(pixel_bytes as u64))
            .ok_or_else(|| IoError::Unsupported("legacy separable native length".into()))?;
        if desc.width != d.width || desc.height != d.height || desc.channels != channels || desc.sample != Sample::F32 || p.native_byte_len() != Some(expected)
        {
            return Ok(false);
        }
        let mut seen = 0u64;
        p.visit_native(ctl, &mut |block| {
            if block.is_empty() || block.len() > 64 * pixel_bytes || block.len() % pixel_bytes != 0 {
                return Err(IoError::Unsupported("legacy separable native block".into()));
            }
            seen =
                seen.checked_add(block.len() as u64).filter(|n| *n <= expected).ok_or_else(|| IoError::Unsupported("legacy separable native excess".into()))?;
            pay(ctl, budget, block.len() as u64)?;
            for raw in block.as_chunks::<4>().0 {
                let word = *raw;
                let value = f32::from_ne_bytes(word);
                valid &= value.is_finite() && (0.0..=1.0).contains(&value);
            }
            Ok(())
        })?;
        if seen != expected {
            return Err(IoError::Unsupported("legacy separable native incomplete".into()));
        }
    }
    pay(ctl, budget, 1)?;
    Ok(valid)
}
