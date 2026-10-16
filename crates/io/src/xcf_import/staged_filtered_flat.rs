//! Sealed source policy for mixed ordinary roots. Never a singleton facade.
use super::*;
use crate::xcf_filters::NeighborhoodDomain;
pub(super) fn work(i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>, n: u64) -> Result<(), IoError> {
    i.check().map_err(|_| IoError::Cancelled)?;
    b.consume_work(n, "filtered flat source").map_err(accounting_error)?;
    i.check().map_err(|_| IoError::Cancelled)
}
pub(super) fn hint<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !(1..=4096).contains(&f.layers.len()) || f.layers.len()==1 && f.layers.first().is_none_or(|d|d.effects.len()<2) {
        return Ok(false);
    }
    // Preserve the exact existing staged_source::point_stack::hint identity
    // before new mixed-name claims. The old selector/strict qualifier remains
    // final; this is metadata identity, never an acceptance/native fallback.
    work(i,b,8)?;
    if f.layers.len()==1 && f.layers.first().is_some_and(|d| !d.effects.is_empty()
        && (d.blend_space,d.composite_space,d.composite_mode)==(0,2,1)) {
        return Ok(false);
    }
    let mut count=0usize;
    let mut known=false;
    for d in &f.layers {
        work(i,b,8)?;
        count=count.checked_add(d.effects.len()).ok_or_else(||IoError::Unsupported("filtered flat node count".into()))?;
        if count>4096 {return Err(IoError::Unsupported("filtered flat node count".into()));}
        for e in &d.effects {
            work(i,b,e.operation.len() as u64+1)?;
            known |= matches!(e.operation.as_str(),"gimp:semi-flatten"|"gegl:hue-chroma"|"gegl:vibrance"|"gegl:exposure"|"gegl:invert-linear"|"gegl:invert-gamma"|"gegl:brightness-contrast"|"gegl:levels");
        }
    }
    Ok(known)
}
pub(super) fn root_domain<P>(d: &Drawable<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<Option<NeighborhoodDomain>, IoError> {
    work(i, b, 16)?;
    if d.effects.is_empty() {
        return Ok(None);
    }
    if d.effects.len()>1024 {return Err(IoError::Unsupported("filtered flat per-stack node count".into()));}
    let mut first=None;
    for e in &d.effects {
        work(i,b,e.operation.len() as u64+128)?;
        let family=crate::xcf_filters::filtered_flat_source_family(e).ok_or_else(||IoError::Unsupported("filtered flat leaf envelope".into()))?;
        if family==crate::xcf_filters::PointReplaceFamily::MixedExposure
            && !crate::xcf_filters::mixed_exposure_source::source_geometry(d.width,d.height) {
            return Err(IoError::Unsupported("mixed Exposure source geometry".into()));
        }
        if first.is_none(){first=Some(NeighborhoodDomain::PointReplace(family));}
    }
    Ok(first)
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
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
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > 1 << 30)
        || !(1..=4096).contains(&f.layers.len())
        || !f.channels.is_empty()
        || !f.paths.is_empty()
        || !f.palette.is_empty()
        || !f.item_sets.is_empty()
    {
        return Err(IoError::Unsupported("filtered flat source header".into()));
    }
    let mut nodes=0usize;
    for d in &f.layers {
        work(i,b,1)?;
        if d.effects.len()>1024 {return Err(IoError::Unsupported("filtered flat per-stack node count".into()));}
        nodes=nodes.checked_add(d.effects.len()).ok_or_else(||IoError::Unsupported("filtered flat node count".into()))?;
        if nodes>4096 {return Err(IoError::Unsupported("filtered flat node count".into()));}
    }
    if nodes==0 || f.layers.len()==1 && nodes<2 {return Err(IoError::Unsupported("filtered flat closed topology".into()));}
    let mut filtered = 0usize;
    for d in &f.layers {
        work(i, b, 64)?;
        if d.group
            || d.model != Model::Rgb
            || !d.alpha
            || !d.opacity.is_finite()
            || !(0.0..=1.0).contains(&d.opacity)
            || (d.width, d.height, d.offset) != (f.width, f.height, (0, 0))
            || d.mode != 28
            || (d.blend_space, d.composite_space, d.composite_mode) != (0, -1, -1)
            || d.path.len() > 1
            || d.mask.is_some()
            || d.text.is_some()
            || d.vector.is_some()
            || d.link.is_some()
            || d.legacy_linked
            || d.selection
            || d.transform.is_some()
            || d.floating_attachment.is_some()
            || d.group_flags != 0
            || d.text_flags != 0
            || !d.item_sets.is_empty()
        {
            return Err(IoError::Unsupported("filtered flat source owner".into()));
        }
        for text in &d.unsupported {
            work(i, b, text.len() as u64 + 1)?;
        }
        if d.unsupported.iter().any(|s| s != "GEGL layer effects") {
            return Err(IoError::Unsupported("filtered flat unsupported owner".into()));
        }
        let domain = root_domain(d, i, b)?;
        let p = d.plane.as_ref().ok_or_else(|| IoError::Unsupported("filtered flat source pixels".into()))?;
        if !native(p, f.sample, f.width, f.height, 4, false, i, Some(b))? {
            return Err(IoError::Unsupported("filtered flat unit native".into()));
        }
        if domain.is_some() {
            filtered += 1;
            for e in &d.effects {
                work(i,b,8)?;
                let m=e.mask.as_ref().ok_or_else(||IoError::Unsupported("filtered flat physical mask".into()))?;
                if !mask(m,f,i,b)? {return Err(IoError::Unsupported("filtered flat mask".into()));}
            }
        }
    }
    if filtered == 0 {
        return Err(IoError::Unsupported("filtered flat requires filter".into()));
    }
    Ok(())
}
fn mask<P: ExactParsedPlane>(m: &Drawable<P>, f: &XcfFile<P>, i: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    let b = Some(budget);
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
fn scan_work(i: &Interrupt<'_>, b: Option<&photocraft_xcf::Budget<'_>>, n: usize) -> Result<(), IoError> {
    i.check().map_err(|_| IoError::Cancelled)?;
    if let Some(b) = b {
        work(i, b, n as u64)?;
    }
    i.check().map_err(|_| IoError::Cancelled)
}
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
        .ok_or_else(|| IoError::Unsupported("filtered flat source length".into()))?;
    let d = p.descriptor();
    if d.sample != sample || (d.width, d.height, d.channels) != (width, height, channels) || p.native_byte_len() != Some(expected) {
        return Ok(false);
    }
    let mut valid = true;
    let mut seen = 0u64;
    p.visit_native(i, &mut |chunk| {
        scan_work(i, b, chunk.len().checked_add(chunk.len() / word).ok_or_else(|| IoError::Unsupported("filtered flat scan work".into()))?)?;
        seen = seen.checked_add(chunk.len() as u64).ok_or_else(|| IoError::Unsupported("filtered flat source count".into()))?;
        if chunk.is_empty() || chunk.len() > 64 * channels * word || chunk.len() % (channels * word) != 0 || seen > expected {
            valid = false;
            return Ok(());
        }
        for raw in chunk.chunks_exact(word) {
            let v = if word == 4 {
                f64::from(f32::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("Semi-Flatten F32 word".into()))?))
            } else {
                f64::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("Semi-Flatten F64 word".into()))?)
            };
            valid &= v.is_finite() && if zero { v == 0.0 } else { (0.0..=1.0).contains(&v) }
        }
        Ok(())
    })?;
    scan_work(i, b, 1)?;
    Ok(valid && seen == expected)
}

#[cfg(test)]
#[path = "staged_filtered_flat_tests.rs"]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "staged_filtered_flat_activity_tests.rs"]
mod activity_tests;

#[cfg(test)]
use crate::xcf_filters::PointReplaceFamily;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="staged_filtered_flat_ordered_tests.rs"]
mod ordered_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "staged_filtered_flat_exposure_tests.rs"]
mod exposure_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "staged_filtered_flat_invert_tests.rs"]
mod invert_tests;

#[path = "staged_filtered_flat_opacity.rs"]
pub(super) mod opacity;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "staged_filtered_flat_opacity_tests.rs"]
mod opacity_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "staged_filtered_flat_affine_tests.rs"]
mod affine_tests;
