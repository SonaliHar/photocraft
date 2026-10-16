//! Sealed ordinary Point nonempty mask program. Old bytezero sources stay old.
use crate::{IoError, xcf_import::plane::ExactParsedPlane};
use photocraft_raster::{Interrupt, backing::BackingError};
use photocraft_xcf::{Effect, Encoding, Model, Sample, XcfFile};
pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<[u32; 2]> {
    crate::xcf_filters::mixed_exposure_source::parameters(effect)
}
/// Called only after the old selector chose ordinary strict Exposure, never
/// before old PointStack/legacy identities. Raw nonempty precedes Float casts.
pub(crate) fn hint<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    work(i, Some(b), 32)?;
    if f.layers.len() != 1
        || f.width == 0
        || f.height == 0
        || f.width > 262144
        || f.height > 262144
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > 1 << 30)
    {
        return Ok(false);
    }
    let Some(d) = f.layers.first() else { return Ok(false) };
    if d.effects.len() != 1 {
        return Ok(false);
    }
    let Some(e) = d.effects.first() else { return Ok(false) };
    if e.operation != "gegl:exposure" {
        return Ok(false);
    }
    let Some(p) = e.mask.as_ref().and_then(|m| m.plane.as_ref()) else { return Ok(false) };
    let (_, nonempty) = visit(p, f.sample, f.width, f.height, 1, true, i, Some(b))?;
    Ok(nonempty)
}
/// Preserve typed work/cancellation at the controlled structural scan boundary.
pub(crate) fn coverage_error(e: BackingError) -> IoError {
    if e == BackingError::Cancelled {
        IoError::Cancelled
    } else if matches!(e, BackingError::Work | BackingError::Quota | BackingError::Allocation) {
        IoError::Xcf(photocraft_xcf::XcfError {
            kind: photocraft_xcf::ErrorKind::Limit,
            offset: 0,
            item: String::new(),
            message: format!("masked source coverage: {e}"),
        })
    } else {
        IoError::Unsupported(format!("masked source coverage: {e}"))
    }
}
/// Strict original singleton activity, independent of mixed source activity.
pub(crate) fn envelope<P>(effect: &Effect<P>) -> bool {
    let s = &effect.state;
    effect.properties.len() <= 16
        && parameters(effect).is_some()
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
        b.consume_work(n as u64, "masked Exposure source proof")
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
        || d.legacy_linked
        || d.selection
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
    if !visit(p, f.sample, f.width, f.height, 4, false, i, b)?.0 {
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
    let (valid, nonempty) = visit(p, f.sample, f.width, f.height, 1, true, i, b)?;
    Ok(valid && nonempty)
}
pub(crate) fn visit<P: ExactParsedPlane>(
    p: &P,
    sample: Sample,
    width: u32,
    height: u32,
    channels: usize,
    coverage: bool,
    i: &Interrupt<'_>,
    b: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<(bool, bool), IoError> {
    let word = match sample {
        Sample::F32 => 4,
        Sample::F64 => 8,
        _ => return Ok((false, false)),
    };
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(channels as u64))
        .and_then(|n| n.checked_mul(word as u64))
        .ok_or_else(|| IoError::Unsupported("masked source length".into()))?;
    let d = p.descriptor();
    if d.sample != sample || (d.width, d.height, d.channels) != (width, height, channels) || p.native_byte_len() != Some(expected) {
        return Ok((false, false));
    }
    let (mut valid, mut nonempty, mut seen) = (true, false, 0u64);
    p.visit_native(i, &mut |chunk| {
        let count = chunk.len() / word;
        work(
            i,
            b,
            chunk
                .len()
                .checked_add(count.checked_mul(16).ok_or_else(|| IoError::Unsupported("masked source work".into()))?)
                .ok_or_else(|| IoError::Unsupported("masked source work".into()))?,
        )?;
        seen = seen.checked_add(chunk.len() as u64).ok_or_else(|| IoError::Unsupported("masked source count".into()))?;
        if chunk.is_empty() || chunk.len() > 64 * channels * word || chunk.len() % (channels * word) != 0 || seen > expected {
            valid = false;
            return Ok(());
        }
        for raw in chunk.chunks_exact(word) {
            let value = if word == 4 {
                f64::from(f32::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("masked F32 word".into()))?))
            } else {
                f64::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("masked F64 word".into()))?)
            };
            if coverage {
                nonempty |= value != 0.;
                let effective = value as f32;
                valid &= value.is_finite() && value >= 0. && effective.is_finite() && (0.0..=1.0).contains(&effective);
            } else {
                valid &= value.is_finite() && (0.0..=1.0).contains(&value);
            }
        }
        Ok(())
    })?;
    work(i, b, 1)?;
    Ok((valid && seen == expected, nonempty))
}
