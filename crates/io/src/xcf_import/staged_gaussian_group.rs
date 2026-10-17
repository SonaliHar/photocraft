//! Strict existing isolated small-Auto Gaussian original-source intersection.
use super::*;
pub(super) fn hint<P>(f: &XcfFile<P>) -> bool {
    f.layers.len() == 4 && f.layers.first().is_some_and(|d| d.group && d.effects.first().is_some_and(|e| e.operation == "gegl:gaussian-blur"))
}
fn geometry<P: ExactParsedPlane>(file: &XcfFile<P>) -> bool {
    if file.model != Model::Rgb
        || file.sample != Sample::F32
        || file.encoding != Encoding::NonlinearProfile
        || !file.icc.is_empty()
        || file.width != 16
        || file.height != 12
        || file.layers.len() != 4
        || !file.channels.is_empty()
        || !file.paths.is_empty()
    {
        return false;
    }
    file.layers.iter().enumerate().all(|(index, d)| {
        let effects_qualified = if index == 0 {
            d.effects.len() == 1 && d.effects.first().is_some_and(|e| e.operation == "gegl:gaussian-blur" && e.version.as_deref() == Some("0:0"))
        } else {
            d.effects.is_empty()
        };
        if !d.visible || !effects_qualified || d.floating_attachment.is_some() {
            return false;
        }
        if index == 0 {
            d.group
                && d.mode == 28
                && d.path.is_empty()
                && d.opacity == 1.0
                && d.blend_space == 0
                && d.composite_space == -1
                && d.composite_mode == -1
                && ((d.width == 12 && d.height == 9 && d.offset == (0, -1)) || (d.width == 12 && d.height == 11 && d.offset == (-1, 0)))
                && d.mask_enabled
                && !d.mask_shown
                && d.mask_editing
                && d.mask.as_ref().is_some_and(|m| {
                    m.width == d.width
                        && m.height == d.height
                        && m.offset == (0, 0)
                        && m.model == Model::Gray
                        && !m.alpha
                        && m.plane.as_ref().is_some_and(|p| p.descriptor().sample == Sample::F32 && p.descriptor().channels == 1)
                })
        } else {
            !d.group
                && d.model == Model::Rgb
                && d.alpha
                && d.mask.is_none()
                && d.plane.as_ref().is_some_and(|p| p.descriptor().sample == Sample::F32 && p.descriptor().channels == 4)
                && d.composite_space == -1
                && match index {
                    1 => {
                        d.path == [0, 0]
                            && d.mode == 28
                            && d.blend_space == 0
                            && d.composite_mode == -1
                            && d.width == 11
                            && d.height == 8
                            && d.offset == (0, 0)
                            && d.opacity == 1.0
                    }
                    2 => {
                        d.path == [0, 1]
                            && d.mode == 30
                            && d.blend_space == -1
                            && d.composite_mode == -2
                            && d.width == 10
                            && d.height == 9
                            && ((d.offset == (2, -1) && d.opacity.to_bits() == 0.7125_f32.to_bits())
                                || (d.offset == (-1, 2) && d.opacity.to_bits() == 0.4375_f32.to_bits()))
                    }
                    3 => {
                        d.path.is_empty()
                            && d.mode == 28
                            && d.blend_space == 0
                            && d.composite_mode == -1
                            && d.width == 16
                            && d.height == 12
                            && d.offset == (0, 0)
                            && d.opacity == 1.0
                    }
                    _ => false,
                }
        }
    })
}
fn poll(ctl: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>, n: u64) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    b.consume_work(n, "isolated Gaussian group source").map_err(accounting_error)?;
    ctl.check().map_err(|_| IoError::Cancelled)
}
pub(super) fn qualify<P: ExactParsedPlane>(file: &XcfFile<P>, ctl: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    poll(ctl, b, 256)?;
    if file.version != 22 || file.precision != 650 || !geometry(file) {
        return Ok(false);
    }
    let Some(group) = file.layers.first() else { return Ok(false) };
    let Some(effect) = group.effects.first() else { return Ok(false) };
    if effect.arguments.len() != 5 {
        return Ok(false);
    }
    let st = &effect.state;
    let Some(enabled) = st.enabled else { return Ok(false) };
    let spaces = (st.blend_space, st.composite_space, st.composite_mode);
    if st.mode != Some(62)
        || st.opacity_bits != Some(1f32.to_bits())
        || st.opacity_u8.is_some()
        || st.clipping != Some(0)
        || st.region != Some(if group.offset.1 == -1 { 0 } else { 1 })
        || !(spaces == (Some(-643996976), Some(-643996976), Some(-643996976))
            || !enabled && spaces == (Some(-1474592175), Some(-1474592175), Some(-1474592175)))
    {
        return Ok(false);
    }
    let mut names = 0u8;
    for a in &effect.arguments {
        poll(ctl, b, 32)?;
        if a.configuration.is_some() || a.graph.is_some() {
            return Ok(false);
        }
        let flag = match (a.name.as_str(), &a.value) {
            ("std-dev-x", ArgumentValue::Float32Bits(v)) if *v == 0.8f32.to_bits() => 1,
            ("std-dev-y", ArgumentValue::Float32Bits(v)) if *v == 1.3f32.to_bits() => 2,
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
    for d in &file.layers {
        poll(ctl, b, 32)?;
        if d.text.is_some() || d.vector.is_some() || d.link.is_some() || d.transform.is_some() {
            return Ok(false);
        }
        if !d.group {
            let Some(p) = d.plane.as_ref() else { return Ok(false) };
            if !native(p, d.width, d.height, 4, false, ctl, b)? {
                return Ok(false);
            }
        }
    }
    let Some(mask) = group.mask.as_ref() else { return Ok(false) };
    let Some(p) = mask.plane.as_ref() else { return Ok(false) };
    if !native(p, mask.width, mask.height, 1, false, ctl, b)? {
        return Ok(false);
    }
    let Some(mask) = effect.mask.as_ref() else { return Ok(false) };
    if mask.model != Model::Gray
        || mask.alpha
        || mask.offset != (0, 0)
        || (mask.width, mask.height) != (16, 12)
        || mask.group
        || !mask.effects.is_empty()
        || mask.mask.is_some()
    {
        return Ok(false);
    }
    let Some(p) = mask.plane.as_ref() else { return Ok(false) };
    native(p, 16, 12, 1, true, ctl, b)
}
fn native<P: ExactParsedPlane>(
    p: &P,
    w: u32,
    h: u32,
    channels: usize,
    zero: bool,
    ctl: &Interrupt<'_>,
    b: &photocraft_xcf::Budget<'_>,
) -> Result<bool, IoError> {
    let d = p.descriptor();
    let n = u64::from(w)
        .checked_mul(u64::from(h))
        .and_then(|n| n.checked_mul(channels as u64))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| IoError::Unsupported("Gaussian group native length".into()))?;
    if d.width != w || d.height != h || d.sample != Sample::F32 || d.channels != channels || p.native_byte_len() != Some(n) {
        return Ok(false);
    }
    let mut seen = 0u64;
    let mut valid = true;
    p.visit_native(ctl, &mut |bytes| {
        if bytes.is_empty() || bytes.len() > 64 * channels * 4 || bytes.len() % (channels * 4) != 0 {
            return Err(IoError::Unsupported("Gaussian group native block".into()));
        }
        seen = seen.checked_add(bytes.len() as u64).filter(|v| *v <= n).ok_or_else(|| IoError::Unsupported("Gaussian group extra native".into()))?;
        poll(ctl, b, bytes.len() as u64)?;
        for word in bytes.as_chunks::<4>().0 {
            let raw: [u8; 4] = *word;
            let v = f32::from_ne_bytes(raw);
            valid &= v.is_finite() && (0.0..=1.0).contains(&v) && (!zero || v == 0.0);
        }
        Ok(())
    })?;
    if seen != n {
        return Err(IoError::Unsupported("Gaussian group incomplete native".into()));
    }
    poll(ctl, b, 0)?;
    Ok(valid)
}

#[cfg(all(test, feature = "corpus"))]
#[path = "staged_gaussian_group_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::unwrap_used)]
mod tests;
