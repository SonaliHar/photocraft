//! Paid immutable actual-tree ordinals. Structural custody, never a source or evaluator grant.
pub use crate::xcf_filter_masks::MaskCoverageSetLease as NormalHierarchyIndexLease;
use crate::{Document, Layer, LayerContent, LayerId, Rect, item_identity::ItemRef};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError as E, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
#[derive(Clone, Copy)]
pub struct NormalHierarchyLimits {
    layers: usize,
    depth: usize,
    nodes: usize,
    masks: u64,
}
impl NormalHierarchyLimits {
    pub const STANDARD: Self = Self { layers: 4096, depth: 32, nodes: 4096, masks: 64 << 20 };
    pub fn try_lowered(layers: usize, depth: usize, nodes: usize, mask_bytes: u64) -> Result<Self, E> {
        if layers > 4096 || depth > 32 || nodes > 4096 || mask_bytes > 64 << 20 {
            return Err(E::Length);
        }
        Ok(Self { layers, depth, nodes, masks: mask_bytes })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct NormalHierarchyIndexRequirements {
    pub layer_count: usize,
    pub group_count: usize,
    pub leaf_count: usize,
    pub stack_count: usize,
    pub node_count: usize,
    pub mask_count: usize,
    pub max_depth: usize,
    pub aggregate_mask_physical_bytes: u64,
    pub index_bytes: u64,
    pub validation_index_bytes: u64,
    pub validation_work_units: u64,
}
pub struct NormalHierarchyRow<'a> {
    pub layer: &'a Layer,
    pub id: LayerId,
    pub ordinal: usize,
    pub parent: Option<usize>,
    pub first_child: Option<usize>,
    pub next_sibling: Option<usize>,
    pub subtree_end: usize,
    pub depth: usize,
    pub canonical_item_index: usize,
    pub operative_bounds: Rect,
    pub stack_index: Option<usize>,
}
pub struct CheckedNormalHierarchyIndex<'a, L: NormalHierarchyIndexLease = QuotaLease> {
    rows: Vec<NormalHierarchyRow<'a>>,
    document: &'a Document,
    requirements: NormalHierarchyIndexRequirements,
    _allocation: L,
}
impl<'a, L: NormalHierarchyIndexLease> CheckedNormalHierarchyIndex<'a, L> {
    pub fn rows(&self) -> &[NormalHierarchyRow<'a>] {
        &self.rows
    }
    pub fn row(&self, i: usize) -> Option<&NormalHierarchyRow<'a>> {
        self.rows.get(i)
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn max_depth(&self) -> usize {
        self.requirements.max_depth
    }
    pub fn leaf_count(&self) -> usize {
        self.requirements.leaf_count
    }
    pub fn group_count(&self) -> usize {
        self.requirements.group_count
    }
    pub fn requirements(&self) -> NormalHierarchyIndexRequirements {
        self.requirements
    }
    pub fn matches_document(&self, d: &Document) -> bool {
        std::ptr::eq(self.document, d)
    }
}
// Fixed cursors bound traversal before any input-sized allocation. Owned trees
// cannot cycle, but hostile depth/cardinality is still checked at every entry.
#[derive(Clone, Copy)]
struct Frame<'a> {
    layers: &'a [Layer],
    next: usize,
    parent: Option<LayerId>,
}
fn walk<'a>(
    d: &'a Document,
    lim: NormalHierarchyLimits,
    check: &dyn Fn(u64) -> Result<(), E>,
    mut visit: impl FnMut(&'a Layer, Option<LayerId>, usize) -> Result<(), E>,
) -> Result<(), E> {
    let mut frames: [Option<Frame<'a>>; 34] = [None; 34];
    *frames.get_mut(0).ok_or(E::Length)? = Some(Frame { layers: &d.layers, next: 0, parent: None });
    let mut depth = 0usize;
    let mut count = 0usize;
    loop {
        check(1)?;
        let frame = frames.get_mut(depth).ok_or(E::Length)?.as_mut().ok_or(E::Integrity)?;
        let Some(layer) = frame.layers.get(frame.next) else {
            if depth == 0 {
                break;
            }
            depth = depth.checked_sub(1).ok_or(E::Length)?;
            continue;
        };
        frame.next = frame.next.checked_add(1).ok_or(E::Length)?;
        let parent = frame.parent;
        count = count.checked_add(1).ok_or(E::Length)?;
        if count > lim.layers || depth > lim.depth {
            return Err(E::Length);
        }
        visit(layer, parent, depth)?;
        if let LayerContent::Group(g) = &layer.content {
            if depth >= lim.depth || g.artboard.is_some() {
                return Err(E::Unsupported);
            }
            let next = depth.checked_add(1).ok_or(E::Length)?;
            if next > lim.depth && !g.children.is_empty() {
                return Err(E::Length);
            }
            if !g.children.is_empty() {
                *frames.get_mut(next).ok_or(E::Length)? = Some(Frame { layers: &g.children, next: 0, parent: Some(layer.id) });
                depth = next;
            }
        }
    }
    Ok(())
}
pub(crate) fn requirements<L: NormalHierarchyIndexLease>(
    d: &Document,
    lim: NormalHierarchyLimits,
    check: &dyn Fn(u64) -> Result<(), E>,
) -> Result<NormalHierarchyIndexRequirements, E> {
    check(1)?;
    let s = d.xcf_state.as_ref().ok_or(E::Unsupported)?;
    if !d.channels.is_empty()
        || !d.paths.is_empty()
        || d.selection.is_some()
        || d.selection_channel_id.is_some()
        || d.quick_mask.is_some()
        || s.items.len() > 8192
        || s.filter_stacks.len() > 4096
        || s.filter_masks.len() > 4096
        || s.opaque.len() > 4096
        || s.selection.selected.len() > 4096
        || !s.resources.is_empty()
        || !s.source_items.is_empty()
        || !s.editable_indexed.is_empty()
        || !s.item_sets.is_empty()
        || s.floating.is_some()
        || !s.selection.selected_paths.is_empty()
        || s.selection.mask_target.is_some()
    {
        return Err(E::Unsupported);
    }
    let mut layers = 0usize;
    let mut groups = 0usize;
    let mut leaves = 0usize;
    let mut depth = 0usize;
    let mut text_bytes = 0u64;
    let mut add = |v: &str| -> Result<(), E> {
        check(1)?;
        if v.len() > 1 << 20 {
            return Err(E::Length);
        }
        text_bytes = text_bytes.checked_add(v.len() as u64).ok_or(E::Length)?;
        if text_bytes > 64 << 20 {
            return Err(E::Length);
        }
        Ok(())
    };
    add(&d.name)?;
    walk(d, lim, check, |l, _, n| {
        layers = layers.checked_add(1).ok_or(E::Length)?;
        depth = depth.max(n + usize::from(l.is_group()));
        add(&l.name)?;
        match &l.content {
            LayerContent::Group(_) => groups += 1,
            LayerContent::BackedRaster(_) | LayerContent::Raster(_) => leaves += 1,
            _ => return Err(E::Unsupported),
        }
        Ok(())
    })?;
    if groups == 0 || leaves == 0 {
        return Err(E::Unsupported);
    }
    if s.selection.selected.len() > layers || s.filter_stacks.len() > leaves {
        return Err(E::Length);
    }
    let mut nodes = 0usize;
    for stack in &s.filter_stacks {
        check(1)?;
        if stack.nodes.len() > 1024 {
            return Err(E::Length);
        }
        nodes = nodes.checked_add(stack.nodes.len()).ok_or(E::Length)?;
        if nodes > lim.nodes {
            return Err(E::Length);
        }
        for n in &stack.nodes {
            check(1)?;
            if n.arguments.len() > 10 || !n.auxiliary.is_empty() {
                return Err(E::Length);
            }
            for value in [n.source_display_name.as_deref(), n.source_icon.as_deref()].into_iter().flatten() {
                add(value)?;
            }
            add(&n.operation.name)?;
            add(&n.operation.stored_version)?;
            add(&n.operation.schema_epoch)?;
            if let Some(p) = &n.composition {
                add(&p.compatibility_epoch)?;
            }
            for a in &n.arguments {
                add(&a.name)?;
                match &a.value {
                    crate::xcf_filters::FilterValue::Number(_) | crate::xcf_filters::FilterValue::Bool(_) => {}
                    crate::xcf_filters::FilterValue::Enum { domain, .. } => add(domain)?,
                    crate::xcf_filters::FilterValue::Color { components, profile, .. } if components.len() == 4 && profile.is_none() => {}
                    _ => return Err(E::Unsupported),
                }
            }
        }
    }
    if nodes != s.filter_masks.len() || s.items.len() != layers.checked_add(nodes).ok_or(E::Length)? {
        return Err(E::Integrity);
    }
    let mut physical = 0u64;
    for m in &s.filter_masks {
        check(1)?;
        let f = m.surface.format();
        if f.mode != crate::ColorMode::Grayscale || f.alpha || !matches!(f.sample, crate::SampleType::F32 | crate::SampleType::F64) {
            return Err(E::Unsupported);
        }
        let bytes = (m.surface.tile_count() as u64)
            .checked_mul(256 * 256)
            .and_then(|n| n.checked_mul(f.bytes_per_pixel() as u64))
            .and_then(|n| n.checked_add(m.surface.default_bytes().len() as u64))
            .ok_or(E::Length)?;
        if bytes > 16 << 20 {
            return Err(E::Length);
        }
        physical = physical.checked_add(bytes).ok_or(E::Length)?;
        if physical > lim.masks {
            return Err(E::Length);
        }
    }
    for i in &s.items {
        add(&i.name)?;
        if let Some(p) = &i.composition {
            add(&p.compatibility_epoch)?;
        }
    }
    let mut payload = 0u64;
    for o in &s.opaque {
        add(&o.namespace)?;
        add(&o.identity)?;
        add(&o.blake3)?;
        payload = payload.checked_add(o.bytes.len() as u64).ok_or(E::Length)?;
        if payload > 256 << 20 {
            return Err(E::Length);
        }
    }
    if s.provenance.protected_source_fingerprints.len() > 4096 {
        return Err(E::Length);
    }
    for v in [&s.provenance.format, &s.provenance.container, &s.provenance.source_label] {
        add(v)?;
    }
    for v in [&s.provenance.producer, &s.provenance.release_id, &s.provenance.original_locator, &s.provenance.source_hash].into_iter().flatten() {
        add(v)?;
    }
    for v in &s.provenance.protected_source_fingerprints {
        add(v)?;
    }
    let mut resolution_bytes = 0u64;
    if let Some(r) = &d.physical_resolution
        && let crate::physical_resolution::PhysicalUnit::Custom(u) = &r.unit
    {
        for v in [&u.name, &u.symbol, &u.abbreviation] {
            add(v)?;
            resolution_bytes = resolution_bytes.checked_add((v.len() as u64).checked_mul(2).ok_or(E::Length)?).ok_or(E::Length)?;
        }
    }
    if let Some(icc) = &d.icc_profile {
        if icc.len() > 16 << 20 {
            return Err(E::Length);
        }
        payload = payload.checked_add(icc.len() as u64).ok_or(E::Length)?;
    }
    let records = layers
        .checked_add(s.items.len())
        .and_then(|n| n.checked_add(nodes))
        .and_then(|n| n.checked_add(s.opaque.len()))
        .and_then(|n| n.checked_add(s.provenance.protected_source_fingerprints.len()))
        .and_then(|n| n.checked_add(s.selection.selected.len()))
        .and_then(|n| n.checked_add(1))
        .ok_or(E::Length)? as u64;
    let scratch = records.checked_mul(512).and_then(|n| n.checked_add(resolution_bytes)).and_then(|n| n.checked_add(4096)).ok_or(E::Length)?;
    let index = (layers as u64)
        .checked_mul(std::mem::size_of::<NormalHierarchyRow<'_>>() as u64)
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(scratch))
        .and_then(|n| n.checked_add(std::mem::size_of::<CheckedNormalHierarchyIndex<'_, L>>() as u64 + 128))
        .ok_or(E::Length)?;
    let work = records
        .checked_mul(records)
        .and_then(|n| n.checked_mul(32))
        .and_then(|n| n.checked_add(text_bytes.checked_add(payload)?.checked_mul(64)?))
        .and_then(|n| n.checked_add(physical.checked_mul(32)?))
        .ok_or(E::Length)?;
    Ok(NormalHierarchyIndexRequirements {
        layer_count: layers,
        group_count: groups,
        leaf_count: leaves,
        stack_count: s.filter_stacks.len(),
        node_count: nodes,
        mask_count: nodes,
        max_depth: depth,
        aggregate_mask_physical_bytes: physical,
        index_bytes: index,
        validation_index_bytes: scratch,
        validation_work_units: work,
    })
}
pub(crate) fn pay(mut n: u64, check: &dyn Fn(u64) -> Result<(), E>) -> Result<(), E> {
    while n != 0 {
        let part = n.min(1 << 20);
        check(part)?;
        n = n.checked_sub(part).ok_or(E::Length)?;
    }
    check(0)
}
impl Document {
    pub fn checked_normal_hierarchy_index_requirements(
        &self,
        lim: NormalHierarchyLimits,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), E>,
    ) -> Result<NormalHierarchyIndexRequirements, E> {
        requirements::<QuotaLease>(self, lim, &|n| {
            interrupt.check().map_err(|_| E::Cancelled)?;
            work(n)?;
            interrupt.check().map_err(|_| E::Cancelled)
        })
    }
    pub fn checked_normal_hierarchy_index_requirements_for_lease<L: NormalHierarchyIndexLease>(
        &self,
        lim: NormalHierarchyLimits,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), E>,
    ) -> Result<NormalHierarchyIndexRequirements, E> {
        requirements::<L>(self, lim, &|n| {
            interrupt.check().map_err(|_| E::Cancelled)?;
            work(n)?;
            interrupt.check().map_err(|_| E::Cancelled)
        })
    }
    pub fn checked_normal_hierarchy_index_with(
        &self,
        lim: NormalHierarchyLimits,
        q: &BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<CheckedNormalHierarchyIndex<'_>, E> {
        self.checked_normal_hierarchy_index_with_reservation(lim, c.interrupt, &|n| c.check(n), |n| q.reserve(QuotaKind::Index, n).map_err(|_| E::Quota))
    }
    pub fn checked_normal_hierarchy_index_with_reservation<L: NormalHierarchyIndexLease>(
        &self,
        lim: NormalHierarchyLimits,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), E>,
        mut reserve: impl FnMut(u64) -> Result<L, E>,
    ) -> Result<CheckedNormalHierarchyIndex<'_, L>, E> {
        let check = |n| {
            interrupt.check().map_err(|_| E::Cancelled)?;
            work(n)?;
            interrupt.check().map_err(|_| E::Cancelled)
        };
        let req = requirements::<L>(self, lim, &check)?;
        let mut lease = reserve(req.index_bytes)?;
        if lease.bytes() < req.index_bytes {
            return Err(E::Allocation);
        }
        let mut rows: Vec<NormalHierarchyRow<'_>> = Vec::new();
        rows.try_reserve_exact(req.layer_count).map_err(|_| E::Allocation)?;
        let actual = (rows.capacity() as u64)
            .checked_mul(std::mem::size_of::<NormalHierarchyRow<'_>>() as u64)
            .and_then(|n| n.checked_add(req.validation_index_bytes))
            .and_then(|n| n.checked_add(std::mem::size_of::<CheckedNormalHierarchyIndex<'_, L>>() as u64 + 128))
            .ok_or(E::Length)?;
        if actual > req.index_bytes {
            return Err(E::Allocation);
        }
        lease.reconcile_capacity(actual)?;
        fill_rows(self, lim, &check, &mut rows)?;
        let state = self.xcf_state.as_ref().ok_or(E::Integrity)?;
        for mask in &state.filter_masks {
            check(1)?;
            if mask.bounds != self.bounds() || mask.surface.format().sample != self.depth {
                return Err(E::Integrity);
            }
            for bytes in std::iter::once(mask.surface.default_bytes()).chain(mask.surface.tiles().map(|(_, t)| t.bytes())) {
                for part in bytes.chunks(32768) {
                    check(part.len() as u64 + 1)?;
                    if part.iter().any(|b| *b != 0) {
                        return Err(E::Unsupported);
                    }
                }
            }
        }
        pay(req.validation_work_units, &check)?;
        self.validate_xcf_state_with_local_hierarchy_work(interrupt, &check)?;
        check(1)?;
        Ok(CheckedNormalHierarchyIndex { rows, document: self, requirements: req, _allocation: lease })
    }
}

pub(crate) fn fill_rows<'a>(
    doc: &'a Document,
    lim: NormalHierarchyLimits,
    check: &dyn Fn(u64) -> Result<(), E>,
    rows: &mut Vec<NormalHierarchyRow<'a>>,
) -> Result<(), E> {
    let state = doc.xcf_state.as_ref().ok_or(E::Integrity)?;
    walk(doc, lim, &check, |layer, parent_id, depth| {
        if layer.id.0 == 0 {
            return Err(E::Integrity);
        }
        let mut parent = None;
        for old in rows.iter() {
            check(1)?;
            if old.id == layer.id {
                return Err(E::Integrity);
            }
            if Some(old.id) == parent_id {
                parent = Some(old.ordinal);
            }
        }
        if parent_id.is_some() != parent.is_some() {
            return Err(E::Integrity);
        }
        let mut canonical = None;
        for (i, item) in state.items.iter().enumerate() {
            check(1)?;
            if item.item == ItemRef::Layer(layer.id) {
                if canonical.is_some() {
                    return Err(E::Integrity);
                }
                canonical = Some(i);
                check((item.name.len() as u64).checked_add(layer.name.len() as u64).ok_or(E::Length)?)?;
                if item.parent != parent_id.map(ItemRef::Layer) || item.visible != layer.visible || item.name != layer.name || item.mask.is_some() {
                    return Err(E::Integrity);
                }
                let expected = if layer.is_group() { None } else { Some(doc.bounds()) };
                if item.raster_bounds != expected {
                    return Err(E::Integrity);
                }
            }
        }
        let mut stack_index = None;
        for (i, stack) in state.filter_stacks.iter().enumerate() {
            check(1)?;
            if stack.owner == ItemRef::Layer(layer.id) {
                if stack_index.is_some() || layer.is_group() {
                    return Err(E::Integrity);
                }
                stack_index = Some(i);
            }
        }
        let ordinal = rows.len();
        if let Some(p) = parent {
            let p = rows.get_mut(p).ok_or(E::Integrity)?;
            if p.first_child.is_none() {
                p.first_child = Some(ordinal)
            }
        }
        for old in rows.iter_mut().rev() {
            check(1)?;
            if old.parent == parent {
                old.next_sibling = Some(ordinal);
                break;
            }
        }
        rows.push(NormalHierarchyRow {
            layer,
            id: layer.id,
            ordinal,
            parent,
            first_child: None,
            next_sibling: None,
            subtree_end: ordinal.checked_add(1).ok_or(E::Length)?,
            depth,
            canonical_item_index: canonical.ok_or(E::Integrity)?,
            operative_bounds: if layer.is_group() { Rect::new(0, 0, 0, 0) } else { doc.bounds() },
            stack_index,
        });
        Ok(())
    })?;
    for i in (0..rows.len()).rev() {
        check(1)?;
        let row = rows.get(i).ok_or(E::Integrity)?;
        let parent = row.parent;
        let end = row.subtree_end;
        let bounds = row.operative_bounds;
        if let Some(p) = parent {
            let p = rows.get_mut(p).ok_or(E::Integrity)?;
            p.subtree_end = p.subtree_end.max(end);
            if !bounds.is_empty() {
                p.operative_bounds = if p.operative_bounds.is_empty() { bounds } else { p.operative_bounds.union(&bounds) };
            }
        }
    }
    for (i, _) in state.filter_stacks.iter().enumerate() {
        let mut found = false;
        for row in rows.iter() {
            check(1)?;
            found |= row.stack_index == Some(i);
        }
        if !found {
            return Err(E::Integrity);
        }
    }
    // Complete actual node -> mask -> canonical policy ownership, including
    // unused/aliased-mask refusal. This is structural, never an operation grant.
    for (si, stack) in state.filter_stacks.iter().enumerate() {
        for (ni, node) in stack.nodes.iter().enumerate() {
            check(1)?;
            let Some(ItemRef::Mask(id)) = node.mask else { return Err(E::Integrity) };
            for (old_si, old_stack) in state.filter_stacks.iter().enumerate() {
                if old_si > si {
                    break;
                }
                check(1)?;
                for (old_ni, old) in old_stack.nodes.iter().enumerate() {
                    if old_si == si && old_ni >= ni {
                        break;
                    }
                    check(1)?;
                    if old.id == node.id || old.mask == node.mask {
                        return Err(E::Integrity);
                    }
                }
            }
            let mut found = None;
            for mask in &state.filter_masks {
                check(1)?;
                if mask.id == id {
                    if found.is_some() || mask.owner != node.id {
                        return Err(E::Integrity);
                    }
                    found = Some(mask);
                }
            }
            let mask = found.ok_or(E::Integrity)?;
            let mut item = None;
            for row in &state.items {
                check(1)?;
                if row.item == ItemRef::Mask(id) {
                    if item.is_some() {
                        return Err(E::Integrity);
                    }
                    item = Some(row);
                }
            }
            let item = item.ok_or(E::Integrity)?;
            let policy = item.mask.as_ref().ok_or(E::Integrity)?;
            if item.parent.is_some()
                || item.composition.is_some()
                || item.raster_bounds != Some(doc.bounds())
                || !item.visible
                || item.source_flags != 0
                || policy.owner != stack.owner
                || policy.bounds != mask.bounds
                || policy.offset != [0, 0]
                || policy.outside_f64_bits != 0
                || !policy.enabled
                || policy.shown
                || policy.editing
                || policy.linked
            {
                return Err(E::Integrity);
            }
        }
    }
    Ok(())
}
