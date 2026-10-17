//! Strict assigned-profile finite-RGB/unit-alpha F32 Gaussian source; no implicit/HDR fallback.
use super::*;
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    if f.version != 22
        || f.model != Model::Rgb
        || f.sample != Sample::F32
        || f.precision != 650
        || f.encoding != Encoding::NonlinearProfile
        || f.icc.is_empty()
        || f.icc.len() > 16 << 20
        || f.width != 16
        || f.height != 12
        || f.layers.len() != 1
        || !f.channels.is_empty()
        || !f.paths.is_empty()
        || !f.palette.is_empty()
        || !f.item_sets.is_empty()
    {
        return Ok(false);
    }
    let Some(d) = f.layers.first() else { return Ok(false) };
    if d.group
        || d.path.len() > 1
        || !d.visible
        || !d.alpha
        || d.model != Model::Rgb
        || d.width != 16
        || d.height != 12
        || d.offset != (0, 0)
        || d.mode != 28
        || (d.blend_space, d.composite_space, d.composite_mode) != (0, -1, -1)
        || d.opacity.to_bits() != 1f32.to_bits()
        || d.mask.is_some()
        || d.text.is_some()
        || d.vector.is_some()
        || d.link.is_some()
        || d.transform.is_some()
        || d.floating_attachment.is_some()
        || d.effects.len() != 1
    {
        return Ok(false);
    }
    poll(ctl, budget, d.unsupported.len() as u64)?;
    if d.unsupported.iter().any(|s| s != "GEGL layer effects") {
        return Ok(false);
    }
    let Some(e) = d.effects.first() else { return Ok(false) };
    let s = &e.state;
    if e.operation != "gegl:gaussian-blur"
        || e.version.as_deref() != Some("0:0")
        || e.arguments.len() != 5
        || s.enabled != Some(true)
        || s.mode != Some(62)
        || s.opacity_u8.is_some()
        || s.opacity_bits != Some(1f32.to_bits())
        || s.clipping != Some(0)
        || s.region != Some(0)
        || (s.blend_space, s.composite_space, s.composite_mode) != (Some(-1474592175), Some(-1474592175), Some(-1474592175))
    {
        return Ok(false);
    }
    poll(ctl, budget, e.properties.len() as u64)?;
    if e.properties.iter().any(|p| !matches!(p.tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 45 | 46)) {
        return Ok(false);
    }
    let mut seen = 0u8;
    let mut x = None;
    let mut y = None;
    let mut filter = None;
    for a in &e.arguments {
        if a.configuration.is_some() || a.graph.is_some() {
            return Ok(false);
        }
        let bit = match (a.name.as_str(), &a.value) {
            ("std-dev-x", ArgumentValue::Float32Bits(v)) if *v == 2.25f32.to_bits() || *v == 3.75f32.to_bits() => {
                x = Some(*v);
                1
            }
            ("std-dev-y", ArgumentValue::Float32Bits(v)) if *v == 1.3f32.to_bits() || *v == 0.8f32.to_bits() => {
                y = Some(*v);
                2
            }
            ("filter", ArgumentValue::Enum(v @ (0 | 1))) => {
                filter = Some(*v);
                4
            }
            ("abyss-policy", ArgumentValue::Enum(1)) => 8,
            ("clip-extent", ArgumentValue::Boolean(true)) => 16,
            _ => return Ok(false),
        };
        if seen & bit != 0 {
            return Ok(false);
        }
        seen |= bit;
    }
    if seen != 31
        || !((x == Some(2.25f32.to_bits()) && y == Some(1.3f32.to_bits()) && filter == Some(0))
            || (x == Some(3.75f32.to_bits()) && y == Some(0.8f32.to_bits()) && filter == Some(1)))
    {
        return Ok(false);
    }
    let Some(p) = d.plane.as_ref() else { return Ok(false) };
    if !native(p, 4, false, ctl, budget)? {
        return Ok(false);
    }
    let Some(m) = e.mask.as_ref() else { return Ok(false) };
    if m.group
        || m.path.len() > 1
        || m.model != Model::Gray
        || m.alpha
        || m.width != 16
        || m.height != 12
        || m.offset != (0, 0)
        || !m.visible
        || !m.mask_enabled
        || m.mask_shown
        || m.mask_editing
        || m.legacy_linked
        || m.mode != 0
        || (m.blend_space, m.composite_space, m.composite_mode) != (2, 2, 1)
        || !m.effects.is_empty()
        || m.mask.is_some()
        || !m.unsupported.is_empty()
        || m.text.is_some()
        || m.vector.is_some()
        || m.link.is_some()
        || m.transform.is_some()
        || m.floating_attachment.is_some()
    {
        return Ok(false);
    }
    let Some(p) = m.plane.as_ref() else { return Ok(false) };
    native(p, 1, true, ctl, budget)
}
fn native<P: ExactParsedPlane>(p: &P, channels: usize, empty: bool, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    let d = p.descriptor();
    let size = 16 * 12 * channels * 4;
    if d.sample != Sample::F32 || d.width != 16 || d.height != 12 || d.channels != channels || p.native_byte_len() != Some(size as u64) {
        return Ok(false);
    }
    let mut seen = 0usize;
    let mut valid = true;
    p.visit_native(ctl, &mut |b| {
        if b.is_empty() || b.len() > 64 * channels * 4 || b.len() % (channels * 4) != 0 {
            return Err(IoError::Unsupported("assigned Gaussian native block".into()));
        }
        seen = seen.checked_add(b.len()).filter(|n| *n <= size).ok_or_else(|| IoError::Unsupported("assigned Gaussian native excess".into()))?;
        poll(ctl, budget, b.len() as u64)?;
        for (index, word) in b.as_chunks::<4>().0.iter().enumerate() {
            let v = f32::from_ne_bytes(*word);
            valid &= v.is_finite() && if empty { v == 0.0 } else { index % channels != channels - 1 || (0.0..=1.0).contains(&v) };
        }
        Ok(())
    })?;
    if seen != size {
        return Err(IoError::Unsupported("assigned Gaussian incomplete native".into()));
    }
    poll(ctl, budget, 1)?;
    Ok(valid)
}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "assigned_gaussian_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::unwrap_used)]
mod tests;
