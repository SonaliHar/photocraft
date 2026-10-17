//! Paid immutable complete-mask structural proof; no operation/source authority.
use super::CheckedFilterMaskCoverage;
use crate::{Document, FilterMask, item_identity::ItemRef};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError as E, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
#[derive(Clone, Copy)]
pub struct MaskCoverageSetLimits {
    masks: usize,
    per_mask: u64,
    aggregate: u64,
}
impl MaskCoverageSetLimits {
    pub const STANDARD: Self = Self { masks: 4096, per_mask: 16 << 20, aggregate: 64 << 20 };
    pub fn try_lowered(masks: usize, per_mask: u64, aggregate: u64) -> Result<Self, E> {
        if masks > 4096 || per_mask > 16 << 20 || aggregate > 64 << 20 {
            return Err(E::Length);
        }
        Ok(Self { masks, per_mask, aggregate })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct MaskCoverageSetRequirements {
    pub mask_count: usize,
    pub aggregate_physical_bytes: u64,
    pub proof_index_bytes: u64,
    pub validation_index_bytes: u64,
    pub validation_work_units: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMaskCoverageClass {
    ByteZeroSentinel,
    RawNonemptyCoverage,
}
/// Trusted compiled allocation receipt. Each token must retain its own real
/// reservation; a shared borrowed envelope is not a new allocation receipt.
pub trait MaskCoverageSetLease {
    fn bytes(&self) -> u64;
    /// Default retains the whole actual envelope; implementations may shrink
    /// only their real reservation, never invent a refund or new ledger.
    fn reconcile_capacity(&mut self, actual: u64) -> Result<(), E> {
        if actual > self.bytes() { Err(E::Allocation) } else { Ok(()) }
    }
}
impl MaskCoverageSetLease for QuotaLease {
    fn bytes(&self) -> u64 {
        QuotaLease::bytes(self)
    }
    fn reconcile_capacity(&mut self, actual: u64) -> Result<(), E> {
        QuotaLease::reconcile_capacity(self, actual).map_err(|_| E::Quota)
    }
}
pub(crate) trait CoverageSetProof {
    fn matches(&self, doc: &Document) -> bool;
    fn covers(&self, doc: &Document, ordinal: usize, mask: &FilterMask) -> bool;
}
struct Entry<'a> {
    proof: CheckedFilterMaskCoverage<'a>,
    stack: usize,
    node: usize,
}
pub struct CheckedFilterMaskCoverageSet<'a, L: MaskCoverageSetLease = QuotaLease> {
    entries: Vec<Entry<'a>>,
    document: &'a Document,
    validation_claimed: std::sync::atomic::AtomicBool,
    _allocation: L,
}
impl<L: MaskCoverageSetLease> CheckedFilterMaskCoverageSet<'_, L> {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn classification_for_mask(&self, ordinal: usize, mask: &FilterMask) -> Result<FilterMaskCoverageClass, E> {
        let entry = self.entries.get(ordinal).ok_or(E::Bounds)?;
        if !entry.proof.covers(mask) {
            return Err(E::Integrity);
        }
        if entry.proof.raw_nonempty() {
            Ok(FilterMaskCoverageClass::RawNonemptyCoverage)
        } else if entry.proof.logical_all_byte_zero() {
            Ok(FilterMaskCoverageClass::ByteZeroSentinel)
        } else {
            Err(E::Unsupported)
        }
    }
    pub(crate) fn covers(&self, doc: &Document, ordinal: usize, mask: &FilterMask) -> bool {
        if !std::ptr::eq(self.document, doc) {
            return false;
        }
        let Some(state) = doc.xcf_state.as_ref() else { return false };
        let Some(entry) = self.entries.get(ordinal) else { return false };
        state.filter_masks.get(ordinal).is_some_and(|m| std::ptr::eq(m, mask))
            && entry.proof.covers(mask)
            && state
                .filter_stacks
                .get(entry.stack)
                .and_then(|s| s.nodes.get(entry.node))
                .is_some_and(|n| n.id == mask.owner && n.mask == Some(ItemRef::Mask(mask.id)))
    }
    pub(crate) fn claim_validation(&self, doc: &Document) -> bool {
        self.matches(doc)
            && self.validation_claimed.compare_exchange(false, true, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed).is_ok()
    }
    pub(crate) fn matches(&self, doc: &Document) -> bool {
        std::ptr::eq(self.document, doc) && doc.xcf_state.as_ref().is_some_and(|s| s.filter_masks.len() == self.entries.len())
    }
}
impl<L: MaskCoverageSetLease> CoverageSetProof for CheckedFilterMaskCoverageSet<'_, L> {
    fn matches(&self, doc: &Document) -> bool {
        CheckedFilterMaskCoverageSet::matches(self, doc)
    }
    fn covers(&self, doc: &Document, ordinal: usize, mask: &FilterMask) -> bool {
        CheckedFilterMaskCoverageSet::covers(self, doc, ordinal, mask)
    }
}
fn requirements<L: MaskCoverageSetLease>(
    doc: &Document,
    limits: MaskCoverageSetLimits,
    check: &dyn Fn(u64) -> Result<(), E>,
) -> Result<MaskCoverageSetRequirements, E> {
    check(1)?;
    let s = doc.xcf_state.as_ref().ok_or(E::Unsupported)?;
    if doc.layers.is_empty()
        || doc.layers.len() > 4096
        || s.filter_stacks.is_empty()
        || s.filter_stacks.len() > doc.layers.len()
        || s.filter_masks.len() > limits.masks
    {
        return Err(E::Length);
    }
    let mut nodes = 0usize;
    for stack in &s.filter_stacks {
        check(1)?;
        if stack.nodes.len() > 1024 {
            return Err(E::Length);
        }
        nodes = nodes.checked_add(stack.nodes.len()).ok_or(E::Length)?;
        if nodes > 4096 {
            return Err(E::Length);
        }
    }
    if nodes != s.filter_masks.len() {
        return Err(E::Integrity);
    }
    let mut bytes = 0u64;
    for mask in &s.filter_masks {
        check(1)?;
        let f = mask.surface.format();
        if f.mode != crate::ColorMode::Grayscale || f.alpha || !matches!(f.sample, crate::SampleType::F32 | crate::SampleType::F64) {
            return Err(E::Unsupported);
        }
        let n = (mask.surface.tile_count() as u64)
            .checked_mul(256 * 256)
            .and_then(|v| v.checked_mul(f.bytes_per_pixel() as u64))
            .and_then(|v| v.checked_add(mask.surface.default_bytes().len() as u64))
            .ok_or(E::Length)?;
        if n > limits.per_mask {
            return Err(E::Length);
        }
        bytes = bytes.checked_add(n).ok_or(E::Length)?;
        if bytes > limits.aggregate {
            return Err(E::Length);
        }
    }
    // The selected whole-Doc validator owns temporary BTree collections and
    // bounded nested searches. Price that separate phase before allocating any
    // scratch, and permit it once per completed token. No generic path changes.
    if !doc.channels.is_empty()
        || doc.quick_mask.is_some()
        || !doc.paths.is_empty()
        || !s.source_items.is_empty()
        || !s.resources.is_empty()
        || !s.editable_indexed.is_empty()
        || !s.item_sets.is_empty()
        || s.floating.is_some()
        || s.items.len() > 8192
        || s.opaque.len() > 4096
        || s.selection.selected.len() > doc.layers.len()
        || !s.selection.selected_paths.is_empty()
        || s.provenance.protected_source_fingerprints.len() > 4096
    {
        return Err(E::Unsupported);
    }
    for layer in &doc.layers {
        check(1)?;
        if layer.is_group() {
            return Err(E::Unsupported);
        }
    }
    let mut payload = 0u64;
    let mut add = |n: usize| -> Result<(), E> {
        check(1)?;
        payload = payload.checked_add(n as u64).ok_or(E::Length)?;
        if payload > 256 << 20 {
            return Err(E::Length);
        }
        Ok(())
    };
    if let Some(icc) = &doc.icc_profile {
        add(icc.len())?;
    }
    for item in &s.items {
        add(item.name.len())?;
    }
    for stack in &s.filter_stacks {
        check(1)?;
        for node in &stack.nodes {
            check(1)?;
            if node.arguments.len() > 10 || !node.auxiliary.is_empty() {
                return Err(E::Unsupported);
            }
            for v in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
                add(v.len())?;
            }
            for v in [&node.source_display_name, &node.source_icon].into_iter().flatten() {
                add(v.len())?;
            }
            for a in &node.arguments {
                add(a.name.len())?;
                match &a.value {
                    crate::xcf_filters::FilterValue::Number(_) | crate::xcf_filters::FilterValue::Bool(_) => {}
                    crate::xcf_filters::FilterValue::Enum { domain, .. } => add(domain.len())?,
                    crate::xcf_filters::FilterValue::Color { components, .. } if components.len() <= 16 => {
                        add(components.len().checked_mul(32).ok_or(E::Length)?)?
                    }
                    _ => return Err(E::Unsupported),
                }
            }
        }
    }
    for o in &s.opaque {
        add(o.bytes.len())?;
        add(o.namespace.len())?;
        add(o.identity.len())?;
        add(o.blake3.len())?;
    }
    for v in [&s.provenance.format, &s.provenance.container, &s.provenance.source_label] {
        add(v.len())?;
    }
    for v in [&s.provenance.producer, &s.provenance.release_id, &s.provenance.original_locator, &s.provenance.source_hash].into_iter().flatten() {
        add(v.len())?;
    }
    for v in &s.provenance.protected_source_fingerprints {
        add(v.len())?;
    }
    // effective_resolution() clones an optional custom unit in the unchanged
    // ordinary validator. Retain its actual prospective three-String overlap.
    let mut resolution_clone = 0u64;
    if let Some(r) = &doc.physical_resolution
        && let crate::physical_resolution::PhysicalUnit::Custom(unit) = &r.unit
    {
        for value in [&unit.name, &unit.symbol, &unit.abbreviation] {
            if value.len() > 1 << 20 {
                return Err(E::Length);
            }
            add(value.len())?;
            resolution_clone = resolution_clone.checked_add((value.len() as u64).checked_mul(2).ok_or(E::Length)?).ok_or(E::Length)?;
        }
        resolution_clone = resolution_clone.checked_add(std::mem::size_of::<crate::physical_resolution::PhysicalResolution>() as u64 + 256).ok_or(E::Length)?;
    }
    let records = (doc.layers.len() as u64)
        .checked_add(s.items.len() as u64)
        .and_then(|n| n.checked_add(nodes as u64))
        .and_then(|n| n.checked_add(s.filter_masks.len() as u64))
        .and_then(|n| n.checked_add(s.filter_stacks.len() as u64))
        .and_then(|n| n.checked_add(s.selection.selected.len() as u64))
        .and_then(|n| n.checked_add(s.opaque.len() as u64))
        .and_then(|n| n.checked_add(1))
        .ok_or(E::Length)?;
    let validation_index_bytes = records.checked_mul(512).and_then(|n| n.checked_add(4096)).and_then(|n| n.checked_add(resolution_clone)).ok_or(E::Length)?;
    let validation_work_units = records
        .checked_mul(records)
        .and_then(|n| n.checked_mul(32))
        .and_then(|n| n.checked_add(payload.checked_mul(64)?))
        .and_then(|n| n.checked_add(bytes.checked_mul(4)?))
        .ok_or(E::Length)?;
    let index = (s.filter_masks.len() as u64)
        .checked_mul(std::mem::size_of::<Entry<'_>>() as u64)
        .and_then(|v| v.checked_mul(2))
        .and_then(|v| v.checked_add(std::mem::size_of::<CheckedFilterMaskCoverageSet<'_, L>>() as u64 + 128))
        .ok_or(E::Length)?;
    Ok(MaskCoverageSetRequirements {
        mask_count: s.filter_masks.len(),
        aggregate_physical_bytes: bytes,
        proof_index_bytes: index.checked_add(validation_index_bytes).ok_or(E::Length)?,
        validation_index_bytes,
        validation_work_units,
    })
}
fn build<'a, L: MaskCoverageSetLease>(
    doc: &'a Document,
    limits: MaskCoverageSetLimits,
    reserve: &dyn Fn(u64) -> Result<L, E>,
    check: &dyn Fn(u64) -> Result<(), E>,
) -> Result<CheckedFilterMaskCoverageSet<'a, L>, E> {
    let req = requirements::<L>(doc, limits, check)?;
    let mut allocation = reserve(req.proof_index_bytes)?;
    if allocation.bytes() < req.proof_index_bytes {
        return Err(E::Quota);
    }
    check(req.proof_index_bytes)?;
    let mut work = req.validation_work_units;
    while work != 0 {
        let n = work.min(65536);
        check(n)?;
        work = work.checked_sub(n).ok_or(E::Length)?;
    }

    let mut entries = Vec::new();
    entries.try_reserve_exact(req.mask_count).map_err(|_| E::Allocation)?;
    let held = (entries.capacity() as u64)
        .checked_mul(std::mem::size_of::<Entry<'_>>() as u64)
        .and_then(|v| v.checked_add(std::mem::size_of::<CheckedFilterMaskCoverageSet<'_, L>>() as u64 + 128))
        .and_then(|v| v.checked_add(req.validation_index_bytes))
        .ok_or(E::Length)?;
    if held > req.proof_index_bytes {
        return Err(E::Allocation);
    }
    allocation.reconcile_capacity(held)?;
    let s = doc.xcf_state.as_ref().ok_or(E::Integrity)?;
    for (ordinal, mask) in s.filter_masks.iter().enumerate() {
        check(1)?;
        for previous in s.filter_masks.get(..ordinal).ok_or(E::Length)? {
            check(1)?;
            if previous.id == mask.id {
                return Err(E::Integrity);
            }
        }
        let mut found = None;
        for (si, stack) in s.filter_stacks.iter().enumerate() {
            check(1)?;
            let ItemRef::Layer(id) = stack.owner else { return Err(E::Integrity) };
            let mut owners = 0usize;
            for layer in &doc.layers {
                check(1)?;
                if layer.id == id && !layer.is_group() {
                    owners = owners.checked_add(1).ok_or(E::Length)?
                }
            }
            if owners != 1 {
                return Err(E::Integrity);
            }
            for (ni, node) in stack.nodes.iter().enumerate() {
                check(1)?;
                if node.mask == Some(ItemRef::Mask(mask.id)) && (node.id != mask.owner || found.replace((si, ni)).is_some()) {
                    return Err(E::Integrity);
                }
            }
        }
        let (stack, node) = found.ok_or(E::Integrity)?;
        let proof = mask.scan_checked_f32_coverage(check)?;
        if !proof.raw_nonempty() && !proof.logical_all_byte_zero() {
            return Err(E::Unsupported);
        }
        entries.push(Entry { proof, stack, node });
    }
    check(0)?;
    Ok(CheckedFilterMaskCoverageSet { entries, document: doc, validation_claimed: std::sync::atomic::AtomicBool::new(false), _allocation: allocation })
}
impl Document {
    pub fn checked_f32_filter_mask_coverage_set_requirements_with_local_work_for_lease<L: MaskCoverageSetLease>(
        &self,
        limits: MaskCoverageSetLimits,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), E>,
    ) -> Result<MaskCoverageSetRequirements, E> {
        requirements::<L>(self, limits, &|n| {
            interrupt.check().map_err(|_| E::Cancelled)?;
            work(n)?;
            interrupt.check().map_err(|_| E::Cancelled)
        })
    }
    /// Source-local allocator retains a distinct actual parent-budget lease.
    /// No callback escapes; all samples and identity facts are read here.
    pub fn checked_f32_filter_mask_coverages_with_reservation<L: MaskCoverageSetLease>(
        &self,
        limits: MaskCoverageSetLimits,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), E>,
        reserve: &dyn Fn(u64) -> Result<L, E>,
    ) -> Result<CheckedFilterMaskCoverageSet<'_, L>, E> {
        build(self, limits, reserve, &|n| {
            interrupt.check().map_err(|_| E::Cancelled)?;
            work(n)?;
            interrupt.check().map_err(|_| E::Cancelled)
        })
    }

    pub fn checked_f32_filter_mask_coverage_set_requirements_with_local_work(
        &self,
        limits: MaskCoverageSetLimits,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), E>,
    ) -> Result<MaskCoverageSetRequirements, E> {
        requirements::<QuotaLease>(self, limits, &|n| {
            interrupt.check().map_err(|_| E::Cancelled)?;
            work(n)?;
            interrupt.check().map_err(|_| E::Cancelled)
        })
    }

    pub fn checked_f32_filter_mask_coverage_set_requirements(
        &self,
        limits: MaskCoverageSetLimits,
        c: &ReadControl<'_>,
    ) -> Result<MaskCoverageSetRequirements, E> {
        requirements::<QuotaLease>(self, limits, &|n| c.check(n))
    }
    pub fn checked_f32_filter_mask_coverages_with(
        &self,
        limits: MaskCoverageSetLimits,
        q: &BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<CheckedFilterMaskCoverageSet<'_>, E> {
        build(self, limits, &|bytes| q.reserve(QuotaKind::Index, bytes).map_err(|_| E::Quota), &|n| c.check(n))
    }
    pub fn checked_f32_filter_mask_coverages_with_local_work(
        &self,
        limits: MaskCoverageSetLimits,
        q: &BackingQuota,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), E>,
    ) -> Result<CheckedFilterMaskCoverageSet<'_>, E> {
        build(self, limits, &|bytes| q.reserve(QuotaKind::Index, bytes).map_err(|_| E::Quota), &|n| {
            interrupt.check().map_err(|_| E::Cancelled)?;
            work(n)?;
            interrupt.check().map_err(|_| E::Cancelled)
        })
    }
}
