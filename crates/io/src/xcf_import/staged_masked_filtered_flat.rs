//! Original-source preallocation plan for the separate masked mixed purpose.
//! Metadata descriptors grant no pixel/source-semantic authority.
use super::*;
use crate::xcf_import::staged_filtered_flat::work;
use photocraft_xcf::{PlaneDescriptor, PlaneRole, Reservation};
pub(super) const SOURCE_RELEASED: bool = true; // Root packet c9d51bd7 / resource4329e968 released.
pub(super) struct DescriptorPlan<'b, 'a> {
    descriptors: Vec<PlaneDescriptor>,
    next: usize,
    mixed_upgrade: bool,
    _fee: Reservation<'b, 'a>,
}
impl DescriptorPlan<'_, '_> {
    pub(super) fn mixed_upgrade(&self) -> bool {
        self.mixed_upgrade
    }
    pub(super) fn verify(&mut self, d: &PlaneDescriptor, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
        staged_filtered_flat::work(&Interrupt::NONE, b, 32)?;
        if self.descriptors.get(self.next) != Some(d) {
            return Err(IoError::Unsupported("masked mixed source descriptor changed".into()));
        }
        self.next = self.next.checked_add(1).ok_or_else(|| IoError::Unsupported("masked mixed descriptor index".into()))?;
        Ok(())
    }
    pub(super) fn complete(&self) -> Result<(), IoError> {
        if self.next != self.descriptors.len() { Err(IoError::Unsupported("masked mixed descriptors missing".into())) } else { Ok(()) }
    }
}
pub(super) fn preplan<'b, 'a>(
    source: &mut dyn photocraft_xcf::Source,
    o: &DecodeOptions<'_>,
    b: &'b photocraft_xcf::Budget<'a>,
    a: photocraft_xcf::PlainSourceAccounting,
    i: &Interrupt<'_>,
) -> Result<Option<DescriptorPlan<'b, 'a>>, IoError> {
    let metadata = photocraft_xcf::read_plain_source_metadata_with_budget(source, o, b, a).map_err(accounting_error)?;
    let f = metadata.file();
    // Only eligible NEW topology is planned. A cap miss is permanent old-only;
    // no old Source cap, identity or validator is changed by this optimization.
    if f.version != 22
        || f.model != Model::Rgb
        || f.encoding != photocraft_xcf::Encoding::NonlinearProfile
        || !matches!((f.sample, f.precision), (Sample::F32, 650) | (Sample::F64, 750))
        || !f.icc.is_empty()
        || !(1..=4096).contains(&f.layers.len())
        || !f.channels.is_empty()
    {
        return Ok(None);
    }
    // Fixed scalar metadata parse paid before any cheap helper. This ordinary
    // permission does not select a Source family or a mixed class.
    if f.layers.len() == 1 {
        staged_filtered_flat::work(i, b, 512)?;
    }
    let ordinary = f.layers.len() == 1
        && f.layers.first().is_some_and(|d| {
            d.effects.len() == 1
                && d.mode == 28
                && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
                && d.effects.first().is_some_and(|e| crate::xcf_filters::masked_mixed_source::parameters(e).is_some())
        });
    if !ordinary
        && f.layers.len() == 1
        && f.layers.first().is_some_and(|d| d.effects.len() < 2 || (d.blend_space, d.composite_space, d.composite_mode) == (0, 2, 1))
    {
        return Ok(None);
    }
    if f.width == 0 || f.height == 0 || f.width > 262144 || f.height > 262144 || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > 1 << 30)
    {
        return Ok(None);
    }
    let tiles = u64::from(f.width)
        .checked_add(255)
        .and_then(|v| v.checked_div(256))
        .and_then(|x| u64::from(f.height).checked_add(255).and_then(|v| v.checked_div(256)).and_then(|y| x.checked_mul(y)))
        .ok_or_else(|| IoError::Unsupported("masked mixed planned grid".into()))?;
    let one = tiles
        .checked_mul(65536)
        .and_then(|v| v.checked_mul(f.sample.source_bytes() as u64))
        .and_then(|v| v.checked_add(f.sample.source_bytes() as u64))
        .ok_or_else(|| IoError::Unsupported("masked mixed planned bytes".into()))?;
    if one > 16 << 20 {
        return Ok(None);
    }
    let mut nodes = 0usize;
    let mut known = false;
    for d in &f.layers {
        staged_filtered_flat::work(i, b, 64)?;
        if d.group || d.mask.is_some() || d.effects.len() > 1024 || d.plane.is_none() || (d.width, d.height, d.offset) != (f.width, f.height, (0, 0)) {
            return Ok(None);
        }
        nodes = nodes.checked_add(d.effects.len()).ok_or_else(|| IoError::Unsupported("masked mixed planned nodes".into()))?;
        if nodes > 4096 || tiles.checked_mul(d.effects.len() as u64).is_none_or(|n| n > 4096) {
            return Ok(None);
        }
        for e in &d.effects {
            staged_filtered_flat::work(i, b, e.operation.len() as u64 + 32)?;
            known |= matches!(e.operation.as_str(), "gegl:exposure" | "gegl:invert-linear" | "gegl:invert-gamma" | "gegl:brightness-contrast" | "gegl:levels");
            let Some(m) = e.mask.as_ref() else { return Ok(None) };
            let Some(p) = m.plane.as_ref() else { return Ok(None) };
            if m.mask.is_some()
                || !m.effects.is_empty()
                || p.role != PlaneRole::Mask
                || p.sample != f.sample
                || p.channels != 1
                || (p.width, p.height, p.offset) != (f.width, f.height, (0, 0))
            {
                return Ok(None);
            }
        }
    }
    if !known || nodes == 0 || f.layers.len() == 1 && nodes < 2 && !ordinary || one.checked_mul(nodes as u64).is_none_or(|n| n > 64 << 20) {
        return Ok(None);
    }
    let count = nodes.checked_add(f.layers.len()).ok_or_else(|| IoError::Unsupported("masked mixed planned planes".into()))?;
    let cap = count.checked_mul(2).ok_or_else(|| IoError::Unsupported("masked mixed plan capacity".into()))?;
    let bytes = cap
        .checked_mul(std::mem::size_of::<PlaneDescriptor>())
        .and_then(|v| v.checked_add(std::mem::size_of::<DescriptorPlan<'_, '_>>()))
        .and_then(|v|v.checked_add(std::mem::size_of::<Sink<'b,'a,'_,'_,SourceSelectedPolicy>>()))
        .ok_or_else(|| IoError::Unsupported("masked mixed plan capacity".into()))?;
    let fee = b.reserve(photocraft_xcf::BudgetKind::Resident, bytes as u64, "masked mixed descriptor plan").map_err(accounting_error)?;
    let mut descriptors = Vec::new();
    descriptors.try_reserve_exact(count).map_err(|_| IoError::Unsupported("masked mixed plan allocation".into()))?;
    if descriptors.capacity() > cap {
        return Err(IoError::Unsupported("masked mixed plan capacity".into()));
    }
    for d in &f.layers {
        for e in &d.effects {
            staged_filtered_flat::work(i, b, 32)?;
            descriptors.push(e.mask.as_ref().and_then(|m| m.plane.as_ref()).ok_or_else(|| IoError::Unsupported("masked mixed planned mask".into()))?.clone());
        }
        staged_filtered_flat::work(i, b, 32)?;
        descriptors.push(d.plane.as_ref().ok_or_else(|| IoError::Unsupported("masked mixed planned root".into()))?.clone());
    }
    Ok(Some(DescriptorPlan { descriptors, next: 0, mixed_upgrade: !ordinary, _fee: fee }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MaskClass {
    ByteZero,
    Nonempty,
}
pub(super) fn mask_class<P: ExactParsedPlane>(
    m: &Drawable<P>,
    sample: Sample,
    width: u32,
    height: u32,
    i: &Interrupt<'_>,
    b: &photocraft_xcf::Budget<'_>,
) -> Result<MaskClass, IoError> {
    if m.group
        || m.alpha
        || m.model != Model::Gray
        || !m.visible
        || m.opacity.to_bits() != 0.5f32.to_bits()
        || (m.width, m.height, m.offset) != (width, height, (0, 0))
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
        return Err(IoError::Unsupported("masked mixed source mask metadata".into()));
    }
    let p = m.plane.as_ref().ok_or_else(|| IoError::Unsupported("masked mixed source mask plane".into()))?;
    let (valid, nonempty) = crate::xcf_point_replace_source::masked_exposure_source::visit(p, sample, width, height, 1, true, i, Some(b))?;
    if !valid {
        return Err(IoError::Unsupported("masked mixed raw coverage domain".into()));
    }
    if nonempty {
        return Ok(MaskClass::Nonempty);
    }
    let mut bytezero = true;
    p.visit_native(i, &mut |chunk| {
        staged_filtered_flat::work(i, b, chunk.len() as u64 + 1)?;
        bytezero &= chunk.iter().all(|v| *v == 0);
        Ok(())
    })?;
    if bytezero { Ok(MaskClass::ByteZero) } else { Err(IoError::Unsupported("masked mixed signedzero third state".into())) }
}
pub(super) fn hint<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !SOURCE_RELEASED || !f.layers.first().and_then(|d| d.plane.as_ref()).is_some_and(|p| p.allows_masked_mixed_source_upgrade()) {
        return Ok(false);
    }
    // Plan already bounds every descriptor/count before source/native allocation.
    let mut found = false;
    let mut third = false;
    for d in &f.layers {
        for e in &d.effects {
            staged_filtered_flat::work(i, b, 32)?;
            let m = e.mask.as_ref().ok_or_else(|| IoError::Unsupported("masked mixed mask".into()))?;
            let p = m.plane.as_ref().ok_or_else(|| IoError::Unsupported("masked mixed plane".into()))?;
            let (valid, nonempty) = crate::xcf_point_replace_source::masked_exposure_source::visit(p, f.sample, f.width, f.height, 1, true, i, Some(b))?;
            if !valid {
                return Err(IoError::Unsupported("masked mixed raw coverage".into()));
            }
            if nonempty {
                found = true;
            } else {
                p.visit_native(i, &mut |chunk| {
                    staged_filtered_flat::work(i, b, chunk.len() as u64 + 1)?;
                    third |= chunk.iter().any(|v| *v != 0);
                    Ok(())
                })?;
            }
        }
    }
    if found && third {
        return Err(IoError::Unsupported("masked mixed third mask state".into()));
    }
    Ok(found)
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
    let mut nonempty = 0usize;
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
        let domain = if d.effects.is_empty() { None } else { Some(()) };
        let p = d.plane.as_ref().ok_or_else(|| IoError::Unsupported("filtered flat source pixels".into()))?;
        if !staged_filtered_flat::native(p, f.sample, f.width, f.height, 4, false, i, Some(b))? {
            return Err(IoError::Unsupported("filtered flat unit native".into()));
        }
        if domain.is_some() {
            filtered += 1;
            for e in &d.effects {
                work(i, b, 8)?;
                let m = e.mask.as_ref().ok_or_else(|| IoError::Unsupported("filtered flat physical mask".into()))?;
                match mask_class(m, f.sample, f.width, f.height, i, b)? {
                    MaskClass::ByteZero => {
                        staged_filtered_flat::work(i, b, 512 + crate::xcf_filters::mixed_stock_source::parsing_work(&e.operation))?;
                        crate::xcf_filters::filtered_flat_source_family(e).ok_or_else(|| IoError::Unsupported("masked mixed zero sibling".into()))?;
                    }
                    MaskClass::Nonempty => {
                        staged_filtered_flat::work(i, b, 512)?;
                        if !crate::xcf_filters::filtered_flat_source_envelope(e) || crate::xcf_filters::masked_mixed_source::parameters(e).is_none() {
                            return Err(IoError::Unsupported("masked mixed nonempty leaf".into()));
                        }
                        nonempty += 1;
                    }
                }
            }
        }
    }
    if filtered == 0 || nonempty == 0 {
        return Err(IoError::Unsupported("filtered flat requires filter".into()));
    }
    Ok(())
}

pub(in crate::xcf_import) struct Domains<'b, 'a> {
    pub(in crate::xcf_import) values: Vec<crate::xcf_filters::NeighborhoodDomain>,
    _fee: Reservation<'b, 'a>,
}
pub(in crate::xcf_import) fn domains<'b, 'a, P: ExactParsedPlane>(
    d: &Drawable<P>,
    sample: Sample,
    w: u32,
    h: u32,
    i: &Interrupt<'_>,
    b: &'b photocraft_xcf::Budget<'a>,
) -> Result<Domains<'b, 'a>, IoError> {
    let count = d.effects.len();
    if count > 1024 {
        return Err(IoError::Unsupported("masked mixed perroot nodes".into()));
    }
    let capacity = count.checked_mul(2).ok_or_else(|| IoError::Unsupported("masked mixed domain capacity".into()))?;
    let bytes = capacity
        .checked_mul(std::mem::size_of::<crate::xcf_filters::NeighborhoodDomain>())
        .and_then(|v| v.checked_add(std::mem::size_of::<Domains<'_, '_>>()))
        .ok_or_else(|| IoError::Unsupported("masked mixed domain bytes".into()))?;
    let fee = b.reserve(photocraft_xcf::BudgetKind::Resident, bytes as u64, "masked mixed mapping domain plan").map_err(accounting_error)?;
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| IoError::Unsupported("masked mixed domain allocation".into()))?;
    if values.capacity() > capacity {
        return Err(IoError::Unsupported("masked mixed domain capacity".into()));
    }
    for e in &d.effects {
        staged_filtered_flat::work(i, b, 512 + crate::xcf_filters::mixed_stock_source::parsing_work(&e.operation))?;
        let class = mask_class(e.mask.as_ref().ok_or_else(|| IoError::Unsupported("masked mixed source mask".into()))?, sample, w, h, i, b)?;
        let family = if class == MaskClass::Nonempty {
            if !crate::xcf_filters::filtered_flat_source_envelope(e) {
                return Err(IoError::Unsupported("masked mixed unit envelope".into()));
            }
            crate::xcf_filters::masked_mixed_source::parameters(e).ok_or_else(|| IoError::Unsupported("masked mixed leaf".into()))?.family()
        } else {
            crate::xcf_filters::filtered_flat_source_family(e).ok_or_else(|| IoError::Unsupported("masked mixed zero leaf".into()))?
        };
        values.push(crate::xcf_filters::NeighborhoodDomain::PointReplace(family));
    }
    Ok(Domains { values, _fee: fee })
}

/// A unique real source-ledger reservation under the live preparation envelope.
/// No independent BackingQuota reservation or refundable borrowed receipt.
pub(in crate::xcf_import) struct CoverageLease<'b, 'a>(pub(in crate::xcf_import) Reservation<'b, 'a>);
impl photocraft_doc::xcf_filter_masks::MaskCoverageSetLease for CoverageLease<'_, '_> {
    fn bytes(&self) -> u64 {
        self.0.bytes()
    }
}

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="staged_masked_filtered_flat_tests.rs"]
mod tests;
