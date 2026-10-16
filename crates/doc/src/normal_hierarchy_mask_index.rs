//! Complete paid actual-tree and mask proof. No source operation authority.
use crate::accounted_document::SelectionPermit;
use crate::{
    Document, FilterMask,
    item_identity::FilterId,
    normal_hierarchy_index::{self as tree, NormalHierarchyIndexLease, NormalHierarchyIndexRequirements, NormalHierarchyLimits, NormalHierarchyRow},
    xcf_filter_masks::{CheckedFilterMaskCoverage, CoverageSetProof, FilterMaskCoverageClass},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError as E, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug)]
pub struct NormalHierarchyMaskIndexRequirements {
    pub tree: NormalHierarchyIndexRequirements,
    pub proof_index_bytes: u64,
    pub mask_entry_bytes: u64,
    pub node_binding_bytes: u64,
    pub stack_range_bytes: u64,
    pub validation_index_bytes: u64,
    pub validation_work_units: u64,
    /// Exact scalar/byte/header mask scan charge; paid again on every actual build.
    pub coverage_work_units: u64,
}
#[derive(Clone, Copy)]
pub struct NormalHierarchyMaskBinding<'doc> {
    pub mask: &'doc FilterMask,
    pub mask_ordinal: usize,
    pub leaf_ordinal: usize,
    pub stack_index: usize,
    pub node_index: usize,
    pub filter_id: FilterId,
    pub classification: FilterMaskCoverageClass,
}
pub struct CheckedNormalHierarchyMaskIndex<'doc, L: NormalHierarchyIndexLease = QuotaLease> {
    rows: Vec<NormalHierarchyRow<'doc>>,
    entries: Vec<CheckedFilterMaskCoverage<'doc>>,
    bindings: Vec<NormalHierarchyMaskBinding<'doc>>,
    ranges: Vec<Range<usize>>,
    document: &'doc Document,
    requirements: NormalHierarchyMaskIndexRequirements,
    raw_nonempty_count: usize,
    validation_claimed: AtomicBool,
    _allocation: L,
}
impl<'doc, L: NormalHierarchyIndexLease> CheckedNormalHierarchyMaskIndex<'doc, L> {
    pub fn rows(&self) -> &[NormalHierarchyRow<'doc>] {
        &self.rows
    }
    pub fn row(&self, i: usize) -> Option<&NormalHierarchyRow<'doc>> {
        self.rows.get(i)
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn max_depth(&self) -> usize {
        self.requirements.tree.max_depth
    }
    pub fn leaf_count(&self) -> usize {
        self.requirements.tree.leaf_count
    }
    pub fn group_count(&self) -> usize {
        self.requirements.tree.group_count
    }
    pub fn matches_document(&self, d: &Document) -> bool {
        std::ptr::eq(self.document, d)
    }
    pub fn raw_nonempty_count(&self) -> usize {
        self.raw_nonempty_count
    }
    pub fn classification_for_mask(&self, i: usize, m: &FilterMask) -> Result<FilterMaskCoverageClass, E> {
        let p = self.entries.get(i).ok_or(E::Bounds)?;
        if !p.covers(m) {
            return Err(E::Integrity);
        }
        classify(p)
    }
    pub fn bindings_for_stack(&self, i: usize) -> Option<&[NormalHierarchyMaskBinding<'doc>]> {
        self.bindings.get(self.ranges.get(i)?.clone())
    }
    fn claim(&self, d: &Document) -> bool {
        self.matches_document(d) && self.validation_claimed.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    }
}
impl<L: NormalHierarchyIndexLease> CoverageSetProof for CheckedNormalHierarchyMaskIndex<'_, L> {
    fn matches(&self, d: &Document) -> bool {
        self.matches_document(d)
    }
    fn covers(&self, d: &Document, i: usize, m: &FilterMask) -> bool {
        self.matches_document(d)
            && d.xcf_state.as_ref().and_then(|s| s.filter_masks.get(i)).is_some_and(|a| std::ptr::eq(a, m))
            && self.entries.get(i).is_some_and(|p| p.covers(m))
    }
}
fn classify(p: &CheckedFilterMaskCoverage<'_>) -> Result<FilterMaskCoverageClass, E> {
    if p.raw_nonempty() {
        Ok(FilterMaskCoverageClass::RawNonemptyCoverage)
    } else if p.logical_all_byte_zero() {
        Ok(FilterMaskCoverageClass::ByteZeroSentinel)
    } else {
        Err(E::Unsupported)
    }
}
fn vector_bytes<T>(n: usize) -> Result<u64, E> {
    (n as u64).checked_mul(std::mem::size_of::<T>() as u64).ok_or(E::Length)
}
/// Closed integer plain role check, before any selected index allocation.
/// Float callers incur no new work or role restriction here.
pub(crate) fn plain_quantized_header(d: &Document, check: &dyn Fn(u64) -> Result<(), E>) -> Result<bool, E> {
    if !matches!(d.depth, crate::SampleType::U8 | crate::SampleType::U16 | crate::SampleType::U32) {
        return Ok(false);
    }
    check(1)?;
    if d.mode != crate::ColorMode::Rgb
        || d.sample_encoding != Some(crate::SampleEncoding::NonlinearProfile)
        || d.source_color_context.as_ref().is_none_or(|source| {
            source.original_icc.is_some() || !matches!((d.depth, source.source_precision), (crate::SampleType::U8, 150) | (crate::SampleType::U16, 250) | (crate::SampleType::U32, 350))
        })
    {
        return Err(E::Unsupported);
    }
    let state = d.xcf_state.as_ref().ok_or(E::Unsupported)?;
    if state.filter_stacks.len() > 4096 {
        return Err(E::Length);
    }
    if !state.filter_masks.is_empty() {
        return Err(E::Unsupported);
    }
    for stack in &state.filter_stacks {
        check(1)?;
        if !stack.nodes.is_empty() {
            return Err(E::Unsupported);
        }
    }
    Ok(true)
}
fn requirements<L: NormalHierarchyIndexLease>(
    d: &Document,
    lim: NormalHierarchyLimits,
    check: &dyn Fn(u64) -> Result<(), E>,
) -> Result<NormalHierarchyMaskIndexRequirements, E> {
    requirements_selected::<L>(d, lim, check, None)
}
fn requirements_selected<L: NormalHierarchyIndexLease>(
    d: &Document, lim: NormalHierarchyLimits, check: &dyn Fn(u64) -> Result<(), E>, selection: Option<&SelectionPermit<'_>>,
) -> Result<NormalHierarchyMaskIndexRequirements, E> {
    plain_quantized_header(d, check)?;
    // This complete header prepass admits 16/64MiB before any proof allocation.
    let t = tree::requirements_selected::<L>(d, lim, check, selection)?;
    let entry = vector_bytes::<CheckedFilterMaskCoverage<'_>>(t.mask_count)?;
    let binding = vector_bytes::<NormalHierarchyMaskBinding<'_>>(t.node_count)?;
    let range = vector_bytes::<Range<usize>>(t.stack_count)?;
    let rows = vector_bytes::<NormalHierarchyRow<'_>>(t.layer_count)?;
    let proof = rows
        .checked_add(entry)
        .and_then(|v| v.checked_add(binding))
        .and_then(|v| v.checked_add(range))
        .and_then(|v| v.checked_mul(2))
        .and_then(|v| v.checked_add(t.validation_index_bytes))
        .and_then(|v| v.checked_add(std::mem::size_of::<CheckedNormalHierarchyMaskIndex<'_, L>>() as u64 + 128))
        .ok_or(E::Length)?;
    let mut scan = 0u64;
    for m in &d.xcf_state.as_ref().ok_or(E::Integrity)?.filter_masks {
        check(1)?;
        let sample = m.surface.format().sample.bytes() as u64;
        let tiles = m.surface.tile_count() as u64;
        let bytes = tiles.checked_mul(65536).and_then(|v| v.checked_mul(sample)).and_then(|v| v.checked_add(sample)).ok_or(E::Length)?;
        let work = bytes
            .checked_mul(2)
            .and_then(|v| v.checked_add((bytes / sample).checked_mul(96)?))
            .and_then(|v| v.checked_add(tiles.checked_mul(16)?))
            .and_then(|v| v.checked_add((std::mem::size_of::<FilterMask>() + std::mem::size_of::<CheckedFilterMaskCoverage<'_>>()) as u64))
            .ok_or(E::Length)?;
        scan = scan.checked_add(work).ok_or(E::Length)?;
    }
    Ok(NormalHierarchyMaskIndexRequirements {
        tree: t,
        proof_index_bytes: proof,
        mask_entry_bytes: entry,
        node_binding_bytes: binding,
        stack_range_bytes: range,
        validation_index_bytes: t.validation_index_bytes,
        validation_work_units: t.validation_work_units,
        coverage_work_units: scan,
    })
}
impl Document {
    pub(crate) fn checked_normal_hierarchy_mask_index_requirements_selected(
        &self, l: NormalHierarchyLimits, c: &ReadControl<'_>, selection: &SelectionPermit<'_>,
    ) -> Result<NormalHierarchyMaskIndexRequirements, E> {
        requirements_selected::<QuotaLease>(self, l, &|n| c.check(n), Some(selection))
    }
    pub(crate) fn checked_normal_hierarchy_mask_index_selected(
        &self, l: NormalHierarchyLimits, q: &BackingQuota, c: &ReadControl<'_>, selection: &SelectionPermit<'_>,
    ) -> Result<CheckedNormalHierarchyMaskIndex<'_>, E> {
        self.checked_normal_hierarchy_mask_index_with_reservation_selected(l,c.interrupt,&|n|c.check(n),|n|q.reserve(QuotaKind::Index,n).map_err(|_|E::Quota),Some(selection))
    }

    pub fn checked_normal_hierarchy_mask_index_requirements(
        &self,
        l: NormalHierarchyLimits,
        i: &Interrupt<'_>,
        w: &dyn Fn(u64) -> Result<(), E>,
    ) -> Result<NormalHierarchyMaskIndexRequirements, E> {
        self.checked_normal_hierarchy_mask_index_requirements_for_lease::<QuotaLease>(l, i, w)
    }
    pub fn checked_normal_hierarchy_mask_index_requirements_for_lease<L: NormalHierarchyIndexLease>(
        &self,
        l: NormalHierarchyLimits,
        i: &Interrupt<'_>,
        w: &dyn Fn(u64) -> Result<(), E>,
    ) -> Result<NormalHierarchyMaskIndexRequirements, E> {
        requirements::<L>(self, l, &|n| {
            i.check().map_err(|_| E::Cancelled)?;
            w(n)?;
            i.check().map_err(|_| E::Cancelled)
        })
    }
    pub fn checked_normal_hierarchy_mask_index_with(
        &self,
        l: NormalHierarchyLimits,
        q: &BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<CheckedNormalHierarchyMaskIndex<'_>, E> {
        self.checked_normal_hierarchy_mask_index_with_reservation(l, c.interrupt, &|n| c.check(n), |n| q.reserve(QuotaKind::Index, n).map_err(|_| E::Quota))
    }
    pub fn checked_normal_hierarchy_mask_index_with_reservation<L: NormalHierarchyIndexLease>(
        &self,
        l: NormalHierarchyLimits,
        i: &Interrupt<'_>,
        w: &dyn Fn(u64) -> Result<(), E>,
        reserve: impl FnMut(u64) -> Result<L, E>,
    ) -> Result<CheckedNormalHierarchyMaskIndex<'_, L>, E> {
        self.checked_normal_hierarchy_mask_index_with_reservation_selected(l, i, w, reserve, None)
    }
    pub(crate) fn checked_normal_hierarchy_mask_index_with_reservation_selected<L: NormalHierarchyIndexLease>(
        &self, l: NormalHierarchyLimits, i: &Interrupt<'_>, w: &dyn Fn(u64) -> Result<(), E>,
        mut reserve: impl FnMut(u64) -> Result<L, E>, selection: Option<&SelectionPermit<'_>>,
    ) -> Result<CheckedNormalHierarchyMaskIndex<'_, L>, E> {
        let check = |n| {
            i.check().map_err(|_| E::Cancelled)?;
            w(n)?;
            i.check().map_err(|_| E::Cancelled)
        };
        let req = requirements_selected::<L>(self, l, &check, selection)?;
        let mut allocation = reserve(req.proof_index_bytes)?;
        if allocation.bytes() < req.proof_index_bytes {
            return Err(E::Allocation);
        }
        let mut rows = Vec::new();
        let mut entries = Vec::new();
        let mut bindings = Vec::new();
        let mut ranges = Vec::new();
        rows.try_reserve_exact(req.tree.layer_count).map_err(|_| E::Allocation)?;
        entries.try_reserve_exact(req.tree.mask_count).map_err(|_| E::Allocation)?;
        bindings.try_reserve_exact(req.tree.node_count).map_err(|_| E::Allocation)?;
        ranges.try_reserve_exact(req.tree.stack_count).map_err(|_| E::Allocation)?;
        let actual = vector_bytes::<NormalHierarchyRow<'_>>(rows.capacity())?
            .checked_add(vector_bytes::<CheckedFilterMaskCoverage<'_>>(entries.capacity())?)
            .and_then(|n| n.checked_add(vector_bytes::<NormalHierarchyMaskBinding<'_>>(bindings.capacity()).ok()?))
            .and_then(|n| n.checked_add(vector_bytes::<Range<usize>>(ranges.capacity()).ok()?))
            .and_then(|n| n.checked_add(req.validation_index_bytes))
            .and_then(|n| n.checked_add(std::mem::size_of::<CheckedNormalHierarchyMaskIndex<'_, L>>() as u64 + 128))
            .ok_or(E::Length)?;
        if actual > req.proof_index_bytes {
            return Err(E::Allocation);
        }
        allocation.reconcile_capacity(actual)?;
        tree::fill_rows(self, l, &check, &mut rows)?;
        let state = self.xcf_state.as_ref().ok_or(E::Integrity)?;
        for mask in &state.filter_masks {
            check(1)?;
            if mask.bounds != self.bounds() || mask.surface.format().sample != self.depth {
                return Err(E::Integrity);
            }
            let p = mask.scan_checked_f32_coverage(&check)?;
            classify(&p)?;
            entries.push(p);
        }
        for (si, stack) in state.filter_stacks.iter().enumerate() {
            check(1)?;
            let mut leaf = None;
            for row in &rows {
                check(1)?;
                if row.stack_index == Some(si) {
                    if leaf.replace(row.ordinal).is_some() {
                        return Err(E::Integrity);
                    }
                }
            }
            let leaf = leaf.ok_or(E::Integrity)?;
            let start = bindings.len();
            for (ni, node) in stack.nodes.iter().enumerate() {
                let mut found = None;
                for (mi, mask) in state.filter_masks.iter().enumerate() {
                    check(1)?;
                    if node.mask == Some(crate::item_identity::ItemRef::Mask(mask.id)) && node.id == mask.owner {
                        if found.replace(mi).is_some() {
                            return Err(E::Integrity);
                        }
                    }
                }
                let mi = found.ok_or(E::Integrity)?;
                let mask = state.filter_masks.get(mi).ok_or(E::Integrity)?;
                let classification = classify(entries.get(mi).ok_or(E::Integrity)?)?;
                if classification == FilterMaskCoverageClass::RawNonemptyCoverage && node.opacity_f64_bits != 1.0f64.to_bits() {
                    return Err(E::Unsupported);
                }
                bindings.push(NormalHierarchyMaskBinding {
                    mask,
                    mask_ordinal: mi,
                    leaf_ordinal: leaf,
                    stack_index: si,
                    node_index: ni,
                    filter_id: node.id,
                    classification,
                });
            }
            ranges.push(start..bindings.len());
        }
        let mut raw_nonempty_count = 0usize;
        for proof in &entries {
            check(1)?;
            if proof.raw_nonempty() {
                raw_nonempty_count = raw_nonempty_count.checked_add(1).ok_or(E::Length)?;
            }
        }
        let token = CheckedNormalHierarchyMaskIndex {
            rows,
            entries,
            bindings,
            ranges,
            document: self,
            requirements: req,
            raw_nonempty_count,
            validation_claimed: AtomicBool::new(false),
            _allocation: allocation,
        };
        tree::pay(req.validation_work_units, &check)?;
        if !token.claim(self) {
            return Err(E::Integrity);
        }
        self.validate_xcf_state_with_local_hierarchy_coverage_work(i, &check, &token)?;
        check(1)?;
        Ok(token)
    }
}
