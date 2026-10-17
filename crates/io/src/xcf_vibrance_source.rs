//! Exact authored Vibrance source intersection. This independently released
//! unit Replace envelope retains present raw words as opaque provenance, never enums.
use crate::{IoError, xcf_import::plane::ExactParsedPlane};
use photocraft_raster::Interrupt;
use photocraft_xcf::{ArgumentValue, Effect, Encoding, Model, Sample, XcfFile};
pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<[u32; 2]> {
    if effect.operation != "gegl:vibrance" || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 2 {
        return None;
    }
    let mut result = [None; 2];
    for argument in &effect.arguments {
        if argument.configuration.is_some() || argument.graph.is_some() {
            return None;
        }
        let (index, lower, upper) = match argument.name.as_str() {
            "vibrance" => (0, -100.0, 100.0),
            "saturation" => (1, 0.0, 10.0),
            _ => return None,
        };
        let ArgumentValue::Float32Bits(bits) = &argument.value else { return None };
        let value = f32::from_bits(*bits);
        if !value.is_finite() || !(lower..=upper).contains(&value) {
            return None;
        }
        let slot = result.get_mut(index)?;
        if slot.is_some() {
            return None;
        }
        *slot = Some(*bits);
    }
    Some([result.first().copied().flatten()?, result.get(1).copied().flatten()?])
}
pub(crate) fn envelope<P>(effect: &Effect<P>) -> bool {
    let s = &effect.state;
    if effect.properties.len() > 16 {
        return false;
    }
    parameters(effect).is_some()
        && s.enabled == Some(true)
        && s.mode == Some(62)
        && s.opacity_bits == Some(1f32.to_bits())
        && s.opacity_u8.is_none()
        && s.clipping == Some(0)
        && s.region == Some(0)
        && s.blend_space.is_some()
        && s.composite_space.is_some()
        && s.composite_mode.is_some()
        && effect.properties.iter().all(|p| matches!(p.tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 45 | 46))
}
fn work(i: &Interrupt<'_>, b: Option<&photocraft_xcf::Budget<'_>>, n: usize) -> Result<(), IoError> {
    i.check().map_err(|_| IoError::Cancelled)?;
    if let Some(b) = b {
        b.consume_work(n as u64, "Vibrance source proof")
            .map_err(|e| if e.kind == photocraft_xcf::ErrorKind::Cancelled { IoError::Cancelled } else { IoError::Xcf(e) })?;
    }
    i.check().map_err(|_| IoError::Cancelled)
}
pub(crate) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    work(i, b, 128)?;
    if f.version != 22
        || f.model != Model::Rgb
        || f.encoding != Encoding::NonlinearProfile
        || !matches!((f.sample, f.precision), (Sample::F32, 650) | (Sample::F64, 750))
        || !f.icc.is_empty()
        || f.width == 0
        || f.height == 0
        || f.width > 262144
        || f.height > 262144
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > (1u64 << 30))
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
        || d.model != Model::Rgb
        || !d.alpha
        || !d.visible
        || d.opacity.to_bits() != 1f32.to_bits()
        || (d.width, d.height, d.offset) != (f.width, f.height, (0, 0))
        || d.mode != 28
        || (d.blend_space, d.composite_space, d.composite_mode) != (0, -1, -1)
        || d.path.len() > 1
        || d.mask.is_some()
        || d.text.is_some()
        || d.vector.is_some()
        || d.link.is_some()
        || d.transform.is_some()
        || d.floating_attachment.is_some()
        || d.effects.len() != 1
        || d.group_flags != 0
        || d.text_flags != 0
    {
        return Ok(false);
    }
    work(i, b, d.unsupported.len())?;
    if d.unsupported.iter().any(|s| s != "GEGL layer effects") {
        return Ok(false);
    }
    let Some(e) = d.effects.first() else { return Ok(false) };
    if !envelope(e) {
        return Ok(false);
    }
    let Some(p) = d.plane.as_ref() else { return Ok(false) };
    if !native(p, f.sample, f.width, f.height, 4, false, i, b)? {
        return Ok(false);
    }
    let Some(m) = e.mask.as_ref() else { return Ok(false) };
    if m.group
        || m.alpha
        || m.model != Model::Gray
        || !m.visible
        || m.opacity.to_bits() != 0.5f32.to_bits()
        || (m.width, m.height, m.offset) != (f.width, f.height, (0, 0))
        || !m.mask_enabled
        || m.mask_shown
        || m.mask_editing
        || m.legacy_linked
        || m.mode != 0
        || (m.blend_space, m.composite_space, m.composite_mode) != (2, 2, 1)
        || !m.path.is_empty()
        || !m.effects.is_empty()
        || m.mask.is_some()
        || !m.unsupported.is_empty()
        || m.text.is_some()
        || m.text_flags != 0
        || m.group_flags != 0
        || m.vector.is_some()
        || m.link.is_some()
        || m.transform.is_some()
        || m.floating_attachment.is_some()
    {
        return Ok(false);
    }
    let Some(p) = m.plane.as_ref() else { return Ok(false) };
    native(p, f.sample, f.width, f.height, 1, true, i, b)
}
// Keep the fixed parameter contract: source identity, limits, control and original budget remain explicit.
#[allow(clippy::too_many_arguments)]
fn native<P: ExactParsedPlane>(
    p: &P,
    sample: Sample,
    width: u32,
    height: u32,
    channels: usize,
    zero: bool,
    i: &Interrupt<'_>,
    b: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    let word = match sample {
        Sample::F32 => 4,
        Sample::F64 => 8,
        _ => return Ok(false),
    };
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(channels as u64))
        .and_then(|n| n.checked_mul(word as u64))
        .ok_or_else(|| IoError::Unsupported("Vibrance source length".into()))?;
    let d = p.descriptor();
    if d.sample != sample || (d.width, d.height, d.channels) != (width, height, channels) || p.native_byte_len() != Some(expected) {
        return Ok(false);
    }
    let mut valid = true;
    let mut seen = 0u64;
    p.visit_native(i, &mut |chunk| {
        work(i, b, chunk.len())?;
        seen = seen.checked_add(chunk.len() as u64).ok_or_else(|| IoError::Unsupported("Vibrance source count".into()))?;
        if chunk.is_empty() || chunk.len() > 64 * channels * word || chunk.len() % (channels * word) != 0 || seen > expected {
            valid = false;
            return Ok(());
        }
        for raw in chunk.chunks_exact(word) {
            let v = if word == 4 {
                f64::from(f32::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("Vibrance F32 word".into()))?))
            } else {
                f64::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("Vibrance F64 word".into()))?)
            };
            valid &= v.is_finite() && if zero { v == 0.0 } else { (0.0..=1.0).contains(&v) }
        }
        Ok(())
    })?;
    work(i, b, 1)?;
    Ok(valid && seen == expected)
}

#[cfg(test)]
#[path = "xcf_vibrance_source_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "xcf_vibrance_source_geometry_tests.rs"]
mod geometry_tests;
