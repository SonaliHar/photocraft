//! Explicit observational Box geometry source. Opaque Replace fields are retained, never resolved.
use super::*;
pub(super) fn hint<P: ExactParsedPlane>(f: &XcfFile<P>) -> bool {
    let Some(d) = f.layers.first() else { return false };
    let old_identity = f.width == 16
        && f.height == 12
        && d.width == 16
        && d.height == 12
        && d.offset == (0, 0)
        && d.effects.first().is_some_and(|e| {
            (e.state.blend_space, e.state.composite_space, e.state.composite_mode) == (Some(-1474592175), Some(-1474592175), Some(-1474592175))
        });
    f.layers.len() == 1
        && f.model == Model::Rgb
        && matches!((f.sample, f.precision), (Sample::F32, 650) | (Sample::F64, 750))
        && f.icc.is_empty()
        && !old_identity
}

fn rectangle(width: u32, height: u32, offset: (i32, i32)) -> Option<Rect> {
    if width == 0 || height == 0 || width > 262144 || height > 262144 || u64::from(width).checked_mul(u64::from(height))? > 1 << 30 {
        return None;
    }
    let right = offset.0.checked_add(i32::try_from(width).ok()?)?;
    let bottom = offset.1.checked_add(i32::try_from(height).ok()?)?;
    if [offset.0, offset.1, right, bottom].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) {
        return None;
    }
    Some(Rect::new(offset.0, offset.1, right, bottom))
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    if f.version != 22
        || f.model != Model::Rgb
        || !matches!((f.sample, f.precision), (Sample::F32, 650) | (Sample::F64, 750))
        || f.encoding != Encoding::NonlinearProfile
        || !f.icc.is_empty()
        || f.layers.len() != 1
        || !f.channels.is_empty()
        || !f.paths.is_empty()
        || !f.palette.is_empty()
        || !f.item_sets.is_empty()
    {
        return Ok(false);
    }
    if rectangle(f.width, f.height, (0, 0)).is_none() {
        return Ok(false);
    }
    let Some(d) = f.layers.first() else { return Ok(false) };
    if rectangle(d.width, d.height, d.offset).is_none() {
        return Ok(false);
    }
    if d.group
        || d.path.len() > 1
        || !d.visible
        || !d.alpha
        || d.model != Model::Rgb
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
    if e.operation != "gegl:box-blur"
        || e.version.as_deref() != Some("0:0")
        || e.arguments.len() != 1
        || s.enabled != Some(true)
        || s.mode != Some(62)
        || s.opacity_u8.is_some()
        || s.opacity_bits != Some(1f32.to_bits())
        || s.clipping != Some(0)
        || s.region != Some(0)
        || s.blend_space.is_none()
        || s.composite_space.is_none()
        || s.composite_mode.is_none()
    {
        return Ok(false);
    }
    poll(ctl, budget, e.properties.len() as u64)?;
    if e.properties.iter().any(|p| !matches!(p.tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 45 | 46)) {
        return Ok(false);
    }
    let Some(a) = e.arguments.first() else { return Ok(false) };
    if a.name != "radius" || !matches!(a.value, ArgumentValue::Integer(4 | 5)) || a.configuration.is_some() || a.graph.is_some() {
        return Ok(false);
    }
    let Some(p) = d.plane.as_ref() else { return Ok(false) };
    if !native(p, d.width, d.height, f.sample, 4, false, ctl, budget)? {
        return Ok(false);
    }
    let Some(m) = e.mask.as_ref() else { return Ok(false) };
    if m.group
        || m.path.len() > 1
        || m.model != Model::Gray
        || m.alpha
        || m.width != f.width
        || m.height != f.height
        || m.offset != (0, 0)
        || m.opacity.to_bits() != 0.5f32.to_bits()
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
    native(p, f.width, f.height, f.sample, 1, true, ctl, budget)
}
fn native<P: ExactParsedPlane>(
    p: &P,
    width: u32,
    height: u32,
    sample: Sample,
    channels: usize,
    empty: bool,
    ctl: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    let d = p.descriptor();
    let word = match sample {
        Sample::F32 => 4,
        Sample::F64 => 8,
        _ => return Ok(false),
    };
    let Some(size) = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(channels as u64)).and_then(|n| n.checked_mul(word as u64))
    else {
        return Ok(false);
    };
    if d.sample != sample || d.width != width || d.height != height || d.channels != channels || p.native_byte_len() != Some(size) {
        return Ok(false);
    }
    let mut seen = 0u64;
    let mut valid = true;
    p.visit_native(ctl, &mut |b| {
        if b.is_empty() || b.len() > 64 * channels * word || b.len() % (channels * word) != 0 {
            return Err(IoError::Unsupported("observed Box geometry native block".into()));
        }
        seen = seen.checked_add(b.len() as u64).filter(|n| *n <= size).ok_or_else(|| IoError::Unsupported("observed Box geometry native excess".into()))?;
        poll(ctl, budget, b.len() as u64)?;
        for (index, word) in b.chunks_exact(word).enumerate() {
            let v = if sample == Sample::F64 {
                f64::from_ne_bytes(word.try_into().map_err(|_| IoError::Unsupported("observed Box geometry native word".into()))?)
            } else {
                f64::from(f32::from_ne_bytes(word.try_into().map_err(|_| IoError::Unsupported("observed Box geometry native word".into()))?))
            };
            valid &= v.is_finite() && if empty { v == 0.0 } else { index % channels != channels - 1 || (0.0..=1.0).contains(&v) };
        }
        Ok(())
    })?;
    if seen != size {
        return Err(IoError::Unsupported("observed Box geometry incomplete native".into()));
    }
    poll(ctl, budget, 1)?;
    Ok(valid)
}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "box_geometry_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "box_geometry_f64_tests.rs"]
mod f64_tests;
