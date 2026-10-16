//! Sealed actual-tree source planning. No group projection is authoritative.
use super::*;
use crate::xcf_import::staged_filtered_flat::work;
use photocraft_xcf::{PlaneDescriptor, Reservation};
pub(super) const SOURCE_RELEASED: bool = true; // Root fe413da0: packet852e / semantic5424 / resource1250.

/// Allocation-free depth-bounded actual Document traversal; never a fake flat Doc.
pub(in crate::xcf_import) struct Layers<'d> {
    frames: [Option<std::slice::Iter<'d, Layer>>; 33],
    depth: usize,
    done: bool,
}
pub(in crate::xcf_import) fn layers(doc: &Document) -> Layers<'_> {
    let mut frames = std::array::from_fn(|_| None);
    if let Some(slot) = frames.get_mut(0) {
        *slot = Some(doc.layers.iter());
    }
    Layers { frames, depth: 0, done: false }
}
impl<'d> Iterator for Layers<'d> {
    type Item = Result<&'d Layer, IoError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let next = self.frames.get_mut(self.depth).and_then(Option::as_mut).and_then(Iterator::next);
            if let Some(layer) = next {
                if let LayerContent::Group(g) = &layer.content {
                    if self.depth >= 32 {
                        self.done = true;
                        return Some(Err(IoError::Unsupported("hierarchy group frame depth".into())));
                    }
                    if !g.children.is_empty() {
                        let Some(depth) = self.depth.checked_add(1).filter(|d| *d <= 32) else {
                            self.done = true;
                            return Some(Err(IoError::Unsupported("hierarchy depth".into())));
                        };
                        self.depth = depth;
                        if let Some(slot) = self.frames.get_mut(depth) {
                            *slot = Some(g.children.iter());
                        }
                    }
                }
                return Some(Ok(layer));
            }
            if self.depth == 0 {
                self.done = true;
                return None;
            }
            self.depth -= 1;
        }
    }
}
/// Complete source path proof, before a nested path can affect priority.
fn paths<P>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<Option<(usize, usize, bool)>, IoError> {
    if f.layers.is_empty() || f.layers.len() > 4096 {
        return Ok(None);
    }
    // Fixed path cursor arrays are real temporary Index state even on cap miss.
    let _cursor = b.reserve(photocraft_xcf::BudgetKind::Index, 1024, "hierarchy path cursor").map_err(accounting_error)?;
    let mut next = [0u32; 33];
    let mut index = [0u32; 33];
    let mut groups = [false; 33];
    let mut previous = 0usize;
    let (mut ng, mut nl, mut nested) = (0usize, 0usize, false);
    for (ordinal, d) in f.layers.iter().enumerate() {
        work(i, b, 1024)?;
        if d.path.len() > 33 {
            return Ok(None);
        }
        let depth = d.path.len().saturating_sub(1);
        if depth > 32 || d.group && depth >= 32 || ordinal == 0 && depth != 0 || depth > previous.saturating_add(1) {
            return Ok(None);
        }
        if depth > previous {
            if let Some(n) = next.get_mut(depth) {
                *n = 0;
            }
        }
        for p in 0..depth {
            if !groups.get(p).copied().unwrap_or(false) || d.path.get(p) != index.get(p) {
                return Ok(None);
            }
        }
        let Some(expected) = next.get(depth).copied() else { return Ok(None) };
        if !d.path.is_empty() && d.path.last() != Some(&expected) {
            return Ok(None);
        }
        if let Some(n) = next.get_mut(depth) {
            *n = expected.checked_add(1).ok_or_else(|| IoError::Unsupported("hierarchy siblings".into()))?;
        }
        if let Some(n) = index.get_mut(depth) {
            *n = expected;
        }
        if let Some(g) = groups.get_mut(depth) {
            *g = d.group;
        }
        if d.group {
            ng += 1;
            nested |= depth != 0;
        } else {
            nl += 1;
        }
        previous = depth;
    }
    Ok(Some((ng, nl, nested)))
}
pub(super) fn intrinsic_old_partition<P>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !staged_accounted_group::hint(f) {
        return Ok(false);
    }
    work(i, b, 8192)?;
    let Some((_, _, nested)) = paths(f, i, b)? else { return Ok(false) };
    Ok(nested || f.layers.iter().any(|d| !d.group && !d.effects.is_empty()))
}
/// Explicit modern implicit plain identity, disjoint from every successful
/// old fixed source (assigned ICC>=132 and raw0,1,1). No validator retry.
pub(super) fn modern_plain_old_partition<P>(f:&XcfFile<P>,i:&Interrupt<'_>,b:&photocraft_xcf::Budget<'_>)->Result<bool,IoError>{
 if !staged_accounted_group::hint(f){return Ok(false);}
 work(i,b,512)?;
 if !matches!(f.version,12|13|22)||f.model!=Model::Rgb||f.encoding!=Encoding::NonlinearProfile||!f.icc.is_empty()
  ||!matches!((f.sample,f.precision),(Sample::U8,150)|(Sample::U16,250)|(Sample::U32,350)|(Sample::F32,650)|(Sample::F64,750))
  ||f.width==0||f.height==0||f.width>262144||f.height>262144||u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n|n>1<<30){return Ok(false);}
 let Some((groups,leaves,_))=paths(f,i,b)? else{return Ok(false)};
 if groups==0||leaves==0{return Ok(false);}
 for d in &f.layers {
  if !d.effects.is_empty()||d.mask.is_some()||d.model!=Model::Rgb||!d.alpha||d.mode!=28||(d.blend_space,d.composite_space,d.composite_mode)!=(0,-1,-1)||d.offset!=(0,0){return Ok(false);}
  if d.group {if d.group_flags!=1||d.plane.is_some(){return Ok(false)}}else if d.group_flags!=0||(d.width,d.height)!=(f.width,f.height){return Ok(false)}
 }
 Ok(true)
}
pub(super) fn hint<P>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if f.layers.len() > 4096 {
        return Ok(false);
    }
    work(i, b, f.layers.len() as u64 + 1)?;
    Ok(f.layers.iter().any(|d| d.group))
}
/// A real metadata plan precedes the FIRST Surface allocation. Cap misses never upgrade.
pub(super) fn descriptors<'b, 'a>(
    f: &XcfFile<PlaneDescriptor>,
    i: &Interrupt<'_>,
    b: &'b photocraft_xcf::Budget<'a>,
) -> Result<Option<super::staged_masked_filtered_flat::DescriptorPlan<'b, 'a>>, IoError> {
    if !matches!(f.version, 12 | 13 | 22)
        || f.model != Model::Rgb
        || !matches!((f.sample, f.precision), (Sample::U8,150)|(Sample::U16,250)|(Sample::U32,350)|(Sample::F32, 650) | (Sample::F64, 750))
        || f.encoding != Encoding::NonlinearProfile
        || !f.icc.is_empty()
        || f.width == 0
        || f.height == 0
        || f.width > 262144
        || f.height > 262144
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > 1 << 30)
    {
        return Ok(None);
    }
    let Some((ng, nl, _)) = paths(f, i, b)? else { return Ok(None) };
    if ng == 0 || nl == 0 {
        return Ok(None);
    }
    if staged_accounted_group::hint(f) && !intrinsic_old_partition(f, i, b)? && !modern_plain_old_partition(f,i,b)? {
        return Ok(None);
    }
    // Not a new Source admission: on semantic metadata miss preserve only
    // existing strict routes. No descriptor permission or retry is granted.
    if super::staged_normal_hierarchy_quantized::exact_depth(f)&&!super::staged_normal_hierarchy_quantized::plan_eligible(f,i,b)?{return Ok(None);}
    let tiles = u64::from(f.width).div_ceil(256).checked_mul(u64::from(f.height).div_ceil(256)).ok_or_else(|| IoError::Unsupported("hierarchy grid".into()))?;
    let one = tiles
        .checked_mul(65536)
        .and_then(|v| v.checked_mul(f.sample.source_bytes() as u64))
        .and_then(|v| v.checked_add(f.sample.source_bytes() as u64))
        .ok_or_else(|| IoError::Unsupported("hierarchy mask bytes".into()))?;
    let (mut nodes, mut count) = (0usize, 0usize);
    for d in &f.layers {
        work(i, b, 64)?;
        if d.group {
            if !d.effects.is_empty() || d.mask.is_some() || d.plane.is_some() {
                return Ok(None);
            }
            continue;
        }
        if d.effects.len() > 1024 || d.mask.is_some() {
            return Ok(None);
        }
        nodes = nodes.checked_add(d.effects.len()).ok_or_else(|| IoError::Unsupported("hierarchy nodes".into()))?;
        if nodes > 4096 {
            return Ok(None);
        }
        if !d.effects.is_empty() && (one > 16 << 20 || one.checked_mul(nodes as u64).is_none_or(|n| n > 64 << 20)) {
            return Ok(None);
        }
        count = count.checked_add(d.effects.len()).and_then(|n| n.checked_add(1)).ok_or_else(|| IoError::Unsupported("hierarchy planes".into()))?;
        for e in &d.effects {
            let Some(p) = e.mask.as_ref().and_then(|m| m.plane.as_ref()) else { return Ok(None) };
            if p.role != photocraft_xcf::PlaneRole::Mask
                || p.sample != f.sample
                || p.channels != 1
                || (p.width, p.height, p.offset) != (f.width, f.height, (0, 0))
            {
                return Ok(None);
            }
        }
        let Some(p) = &d.plane else { return Ok(None) };
        if (p.width, p.height, p.offset) != (f.width, f.height, (0, 0)) {
            return Ok(None);
        }
    }
    let cap = count.checked_mul(2).ok_or_else(|| IoError::Unsupported("hierarchy descriptor capacity".into()))?;
    let bytes = cap
        .checked_mul(std::mem::size_of::<PlaneDescriptor>())
        .and_then(|n| {
            n.checked_add(std::mem::size_of::<super::staged_masked_filtered_flat::DescriptorPlan<'b, 'a>>() + std::mem::size_of::<Layers<'_>>() + 4096)
        })
        .ok_or_else(|| IoError::Unsupported("hierarchy descriptor bytes".into()))?;
    let fee: Reservation<'b, 'a> = b.reserve(photocraft_xcf::BudgetKind::Resident, bytes as u64, "hierarchy descriptor plan").map_err(accounting_error)?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| IoError::Unsupported("hierarchy descriptors allocation".into()))?;
    if out.capacity() > cap {
        return Err(IoError::Unsupported("hierarchy descriptor capacity".into()));
    }
    for d in &f.layers {
        if d.group {
            continue;
        }
        for e in &d.effects {
            work(i, b, 32)?;
            out.push(e.mask.as_ref().and_then(|m| m.plane.as_ref()).ok_or_else(|| IoError::Unsupported("hierarchy descriptor mask".into()))?.clone());
        }
        work(i, b, 32)?;
        out.push(d.plane.as_ref().ok_or_else(|| IoError::Unsupported("hierarchy descriptor raster".into()))?.clone());
    }
    let mut coverage=false;
    if f.version==22 {
        for d in &f.layers {for e in &d.effects {
            work(i,b,512)?;
            coverage |= crate::xcf_filters::masked_mixed_source::parameters(e).is_some();
        }}
    }
    Ok(Some(super::staged_masked_filtered_flat::DescriptorPlan::hierarchy(out, fee, coverage)))
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
    // The publication guard is separate from metadata/decoder permissions. No Source
    // tuple is admitted without the explicit actual-packet compatibility release.
    if !SOURCE_RELEASED {
        return Err(IoError::Unsupported("normal hierarchy Source packet policy not released".into()));
    }
    validate_source_body(f, i, b, false,false)
}
pub(super) fn validate_source_body<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>, masked:bool,quantized:bool) -> Result<(), IoError> {
    work(i, b, 128)?;
    if quantized {if !super::staged_normal_hierarchy_quantized::exact_depth(f){return Err(IoError::Unsupported("integer hierarchy precision".into()));}super::staged_normal_hierarchy_quantized::preflight(f,i,b)?;}
    else if !matches!((f.sample,f.precision),(Sample::F32,650)|(Sample::F64,750)){return Err(IoError::Unsupported("float hierarchy precision".into()));}
    if masked && f.version!=22{return Err(IoError::Unsupported("masked hierarchy Source version".into()));}
    if !matches!(f.version, 12 | 13 | 22)
        || f.model != Model::Rgb
        || f.encoding != Encoding::NonlinearProfile
        || !matches!((f.sample, f.precision), (Sample::U8,150)|(Sample::U16,250)|(Sample::U32,350)|(Sample::F32, 650) | (Sample::F64, 750))
        || !f.icc.is_empty()
        || f.width == 0
        || f.height == 0
        || f.width > 262144
        || f.height > 262144
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > 1 << 30)
        || !f.channels.is_empty()
        || !f.paths.is_empty()
        || !f.palette.is_empty()
        || !f.item_sets.is_empty()
    {
        return Err(IoError::Unsupported("hierarchy Source header".into()));
    }
    let Some((ng, nl, _)) = paths(f, i, b)? else { return Err(IoError::Unsupported("hierarchy Source paths".into())) };
    if ng == 0 || nl == 0 {
        return Err(IoError::Unsupported("hierarchy Source topology".into()));
    }
    let mut nodes = 0usize;
    let n = f.layers.len() as u64;
    work(i, b, n.checked_mul(n).and_then(|v| v.checked_mul(64)).ok_or_else(|| IoError::Unsupported("hierarchy Source subtree work".into()))?)?;
    for (ordinal, d) in f.layers.iter().enumerate() {
        work(i, b, 64)?;
        if !d.opacity.is_finite()
            || !(0.0..=1.0).contains(&d.opacity)
            || d.model != Model::Rgb
            || d.mask.is_some()
            || d.text.is_some()
            || d.vector.is_some()
            || d.link.is_some()
            || d.legacy_linked
            || d.selection
            || d.transform.is_some()
            || d.floating_attachment.is_some()
            || d.text_flags != 0
            || !d.item_sets.is_empty()
        {
            return Err(IoError::Unsupported("hierarchy Source switches".into()));
        }
        for text in &d.unsupported {
            work(i, b, text.len() as u64 + 1)?;
            if text != "GEGL layer effects" {
                return Err(IoError::Unsupported("hierarchy Source unsupported semantics".into()));
            }
        }
        if d.group {
            if !d.effects.is_empty()
                || d.plane.is_some()
                || !d.alpha
                || d.group_flags != 1
                || d.offset != (0, 0)
                || d.mode != 28
                || (d.blend_space, d.composite_space, d.composite_mode) != (0, -1, -1)
            {
                return Err(IoError::Unsupported("hierarchy raw isolated Normal group".into()));
            }
            let depth = d.path.len().saturating_sub(1);
            let mut contains_leaf = false;
            for child in f.layers.iter().skip(ordinal + 1) {
                i.check().map_err(|_| IoError::Cancelled)?;
                b.poll("hierarchy Source subtree original controls").map_err(accounting_error)?;
                if child.path.len().saturating_sub(1) <= depth {
                    break;
                }
                contains_leaf |= !child.group;
            }
            let expected = if contains_leaf { (f.width, f.height) } else { (1, 1) };
            if (d.width, d.height) != expected {
                return Err(IoError::Unsupported("hierarchy raw group union/empty extent".into()));
            }
            continue;
        }
        if !d.alpha
            || (d.width, d.height, d.offset) != (f.width, f.height, (0, 0))
            || d.mode != 28
            || (d.blend_space, d.composite_space, d.composite_mode) != (0, -1, -1)
            || d.group_flags != 0
            || d.effects.len() > 1024
        {
            return Err(IoError::Unsupported("hierarchy Source leaf".into()));
        }
        nodes = nodes.checked_add(d.effects.len()).ok_or_else(|| IoError::Unsupported("hierarchy Source nodes".into()))?;
        if nodes > 4096 {
            return Err(IoError::Unsupported("hierarchy Source nodes".into()));
        }
        let p = d.plane.as_ref().ok_or_else(|| IoError::Unsupported("hierarchy Source pixels".into()))?;
        if !p.allows_normal_hierarchy_source() || !(if quantized{super::staged_normal_hierarchy_quantized::native(p,f.sample,f.width,f.height,i,b)?}else{staged_filtered_flat::native(p, f.sample, f.width, f.height, 4, false, i, Some(b))?}) {
            return Err(IoError::Unsupported("hierarchy Source nativeunit/plan".into()));
        }
        if masked { super::staged_normal_hierarchy_masked::validate_leaf(d,f,i,b)?; continue; }
        staged_filtered_flat::root_domain(d, i, b)?;
        for e in &d.effects {
            let m = e.mask.as_ref().ok_or_else(|| IoError::Unsupported("hierarchy Source mask".into()))?;
            if !staged_filtered_flat::mask(m, f, i, b)? {
                return Err(IoError::Unsupported("hierarchy Source mask envelope".into()));
            }
            let p = m.plane.as_ref().ok_or_else(|| IoError::Unsupported("hierarchy Source mask plane".into()))?;
            let mut zero = true;
            p.visit_native(i, &mut |bytes| {
                work(i, b, bytes.len() as u64 + 1)?;
                zero &= bytes.iter().all(|v| *v == 0);
                Ok(())
            })?;
            if !zero {
                return Err(IoError::Unsupported("hierarchy Source mask BYTEzero".into()));
            }
        }
    }
    if nodes != 0 && f.version != 22 {
        return Err(IoError::Unsupported("hierarchy filtered Source version".into()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "staged_normal_hierarchy_tests.rs"]
mod tests;
