//! Separately sealed fresh mixed source activity; old unit qualifier stays exact.
use super::*;
pub(in super::super) fn has_nonunit<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !(1..=4096).contains(&f.layers.len()) {
        return Err(IoError::Unsupported("mixed opacity source root count".into()));
    }
    let mut count = 0usize;
    let mut found = false;
    for d in &f.layers {
        work(i, b, 8)?;
        count = count.checked_add(d.effects.len()).ok_or_else(|| IoError::Unsupported("mixed opacity source count".into()))?;
        if d.effects.len() > 1024 || count > 4096 {
            return Err(IoError::Unsupported("mixed opacity source count".into()));
        }
        for e in &d.effects {
            work(i, b, 8)?;
            found |= e.state.opacity_bits.is_some_and(|v| v != 1f32.to_bits());
        }
    }
    Ok(found)
}
pub(in super::super) fn root_domain<P>(d: &Drawable<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<Option<NeighborhoodDomain>, IoError> {
    work(i, b, 16)?;
    if d.effects.is_empty() {
        return Ok(None);
    }
    if d.effects.len() > 1024 {
        return Err(IoError::Unsupported("filtered flat per-stack node count".into()));
    }
    let mut first = None;
    for e in &d.effects {
        work(i, b, e.operation.len() as u64 + 256 + crate::xcf_filters::mixed_stock_source::parsing_work(&e.operation))?;
        let family = crate::xcf_filters::filtered_flat_opacity_source::family(e).ok_or_else(|| IoError::Unsupported("filtered flat leaf envelope".into()))?;
        if family == crate::xcf_filters::PointReplaceFamily::MixedExposure && !crate::xcf_filters::mixed_exposure_source::source_geometry(d.width, d.height) {
            return Err(IoError::Unsupported("mixed Exposure source geometry".into()));
        }
        if first.is_none() {
            first = Some(NeighborhoodDomain::PointReplace(family));
        }
    }
    Ok(first)
}
pub(in super::super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
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
    let mut nodes = 0usize;
    for d in &f.layers {
        work(i, b, 1)?;
        if d.effects.len() > 1024 {
            return Err(IoError::Unsupported("filtered flat per-stack node count".into()));
        }
        nodes = nodes.checked_add(d.effects.len()).ok_or_else(|| IoError::Unsupported("filtered flat node count".into()))?;
        if nodes > 4096 {
            return Err(IoError::Unsupported("filtered flat node count".into()));
        }
    }
    if nodes == 0 || f.layers.len() == 1 && nodes < 2 {
        return Err(IoError::Unsupported("filtered flat closed topology".into()));
    }
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
                work(i, b, 8)?;
                let m = e.mask.as_ref().ok_or_else(|| IoError::Unsupported("filtered flat physical mask".into()))?;
                if !mask(m, f, i, b)? {
                    return Err(IoError::Unsupported("filtered flat mask".into()));
                }
            }
        }
    }
    if filtered == 0 {
        return Err(IoError::Unsupported("filtered flat requires filter".into()));
    }
    if !has_nonunit(f, i, b)? {
        return Err(IoError::Unsupported("mixed opacity source requires nonunit filter".into()));
    }
    Ok(())
}
