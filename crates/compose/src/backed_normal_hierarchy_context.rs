//! Paid actual-tree isolated Normal compiler; no flat-document aliases or fake roots.
use crate::{
    backed_normal::{BackedNormalError as E, NormalBandLimits},
    backed_normal_stack::OwnedNormalStackBand,
    backed_stock_point_context::{PlanBudget, bound_profile, root::RootPointPlan},
    gimp_composite::{CompositionError, WorkingSpace},
    gimp_render::PreparationBudget,
};
use photocraft_cms::Profile;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
use photocraft_doc::{
    Document, FilterOutputPrecision, LayerContent, LayerId, SampleEncoding,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
    normal_hierarchy_index::{CheckedNormalHierarchyIndex, NormalHierarchyIndexLease, NormalHierarchyLimits, NormalHierarchyRow},
    normal_hierarchy_mask_index::{CheckedNormalHierarchyMaskIndex, NormalHierarchyMaskBinding},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, NativePlaneDescriptor, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
#[path = "backed_normal_hierarchy_stack.rs"]
mod stack;
const SOURCE_RELEASED: bool = true;
fn scope() -> E {
    E::Unsupported("closed isolated Normal actual hierarchy")
}
fn length() -> E {
    E::Invalid("normal hierarchy size/count/capacity")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, mut n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(0)?;
    while n != 0 {
        let part = n.min(4096);
        b.check(part)?;
        n -= part;
        i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    }
    Ok(())
}
struct NativeBudget<'a, 'c> {
    quota: &'a BackingQuota,
    control: &'a ReadControl<'c>,
}
impl PlanBudget for NativeBudget<'_, '_> {
    type Lease = QuotaLease;
    fn reserve(&mut self, n: u64) -> Result<QuotaLease, E> {
        self.control.check(0).map_err(E::Backing)?;
        self.quota.reserve(QuotaKind::Resident, n).map_err(E::Quota)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.control.check(n).map_err(E::Backing)
    }
}
struct SharedBudget<'a, B>(&'a mut B);
impl<B: PreparationBudget> PlanBudget for SharedBudget<'_, B> {
    type Lease = B::Lease;
    fn reserve(&mut self, n: u64) -> Result<Self::Lease, E> {
        self.0.reserve_prepared_bytes(n).map_err(E::Color)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.0.consume_preparation_work(n).map_err(E::Color)
    }
}
fn preparation_error(e: E) -> CompositionError {
    match e {
        E::Color(e) => e,
        E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
        E::Allocation | E::Backing(BackingError::Allocation) => photocraft_raster::RasterInputError::AllocationFailed.into(),
        _ => CompositionError::Unsupported("normal hierarchy preparation refused"),
    }
}
/// Index reservation uses the original Source Index pool, never prepared Resident credit.
pub trait HierarchyPreparationBudget: PreparationBudget {
    fn reserve_hierarchy_index(&mut self, bytes: u64) -> Result<Self::Lease, CompositionError>;
}
struct SourceIndexLease<L> {
    _lease: L,
    bytes: u64,
}
impl<L> NormalHierarchyIndexLease for SourceIndexLease<L> {
    fn bytes(&self) -> u64 {
        self.bytes
    }
}
// Private common view shares the compiler, never converts a coverage token to a BYTEzero token.
trait TreeIndex<'doc> {
    fn rows(&self) -> &[NormalHierarchyRow<'doc>];
    fn max_depth(&self) -> usize;
    fn leaf_count(&self) -> usize;
    fn group_count(&self) -> usize;
    fn matches_document(&self, d: &Document) -> bool;
    fn bindings_for_stack(&self, _stack: usize) -> Option<&[NormalHierarchyMaskBinding<'doc>]> {
        None
    }
}
impl<'doc, L: NormalHierarchyIndexLease> TreeIndex<'doc> for CheckedNormalHierarchyIndex<'doc, L> {
    fn rows(&self) -> &[NormalHierarchyRow<'doc>] {
        self.rows()
    }
    fn max_depth(&self) -> usize {
        self.max_depth()
    }
    fn leaf_count(&self) -> usize {
        self.leaf_count()
    }
    fn group_count(&self) -> usize {
        self.group_count()
    }
    fn matches_document(&self, d: &Document) -> bool {
        self.matches_document(d)
    }
}
impl<'doc, L: NormalHierarchyIndexLease> TreeIndex<'doc> for CheckedNormalHierarchyMaskIndex<'doc, L> {
    fn rows(&self) -> &[NormalHierarchyRow<'doc>] {
        self.rows()
    }
    fn max_depth(&self) -> usize {
        self.max_depth()
    }
    fn leaf_count(&self) -> usize {
        self.leaf_count()
    }
    fn group_count(&self) -> usize {
        self.group_count()
    }
    fn matches_document(&self, d: &Document) -> bool {
        self.matches_document(d)
    }
    fn bindings_for_stack(&self, stack: usize) -> Option<&[NormalHierarchyMaskBinding<'doc>]> {
        self.bindings_for_stack(stack)
    }
}
enum CurrentIndex<'doc> {
    Zero(CheckedNormalHierarchyIndex<'doc>),
    Coverage(CheckedNormalHierarchyMaskIndex<'doc>),
}
impl<'doc> CurrentIndex<'doc> {
    fn row(&self, ordinal: usize) -> Option<&NormalHierarchyRow<'doc>> {
        match self {
            Self::Zero(v) => v.row(ordinal),
            Self::Coverage(v) => v.row(ordinal),
        }
    }
}
/// Bounded original Source preparation under the caller's original Index/work ledger.
pub struct LeasedNormalHierarchyPipeline<S, L> {
    _plan: Plan<L>,
    _source: S,
    _lease: L,
}
impl<S, L> LeasedNormalHierarchyPipeline<S, L> {
    pub fn new_budgeted<B: HierarchyPreparationBudget<Lease = L>>(doc: &Document, i: &Interrupt<'_>, b: &mut B, source: S) -> Result<Self, CompositionError> {
        if !SOURCE_RELEASED {
            return Err(CompositionError::Unsupported("normal hierarchy Source enrollment disabled"));
        }
        header(doc).map_err(preparation_error)?;
        let cell = std::cell::RefCell::new(b);
        let failed = std::cell::RefCell::new(None);
        let record = |e: CompositionError| {
            let cancelled = i.check().is_err();
            if failed.borrow().is_none() {
                *failed.borrow_mut() = Some(e);
            }
            if cancelled { BackingError::Cancelled } else { BackingError::Work }
        };
        let index = doc.checked_normal_hierarchy_index_with_reservation(
            NormalHierarchyLimits::STANDARD,
            i,
            &|n| cell.borrow_mut().consume_preparation_work(n).map_err(&record),
            &mut |n| cell.borrow_mut().reserve_hierarchy_index(n).map(|lease| SourceIndexLease { _lease: lease, bytes: n }).map_err(&record),
        );
        if let Some(e) = failed.into_inner() {
            return Err(e);
        }
        let index = index.map_err(|e| preparation_error(E::Backing(e)))?;
        let original = cell.into_inner();
        let (plan, lease) =
            compile(doc, &index, Purpose::Source, std::mem::size_of::<Self>() as u64, i, &mut SharedBudget(original)).map_err(preparation_error)?;
        Ok(Self { _plan: plan, _source: source, _lease: lease })
    }
    /// Distinct original Source role; only the complete actual-tree coverage token admits masks.
    pub fn new_masked_source_budgeted<B: HierarchyPreparationBudget<Lease = L>>(
        doc: &Document,
        i: &Interrupt<'_>,
        b: &mut B,
        source: S,
    ) -> Result<Self, CompositionError> {
        if !SOURCE_RELEASED {
            return Err(CompositionError::Unsupported("normal hierarchy Source enrollment disabled"));
        }
        header(doc).map_err(preparation_error)?;
        if doc.xcf_state.as_ref().map(|s| s.provenance.format_version) != Some(22) {
            return Err(CompositionError::Unsupported("masked hierarchy Source requires actual XCF22"));
        }
        let cell = std::cell::RefCell::new(b);
        let failed = std::cell::RefCell::new(None);
        let record = |e: CompositionError| {
            let cancelled = i.check().is_err();
            if failed.borrow().is_none() {
                *failed.borrow_mut() = Some(e);
            }
            if cancelled { BackingError::Cancelled } else { BackingError::Work }
        };
        let index = doc.checked_normal_hierarchy_mask_index_with_reservation(
            NormalHierarchyLimits::STANDARD,
            i,
            &|n| cell.borrow_mut().consume_preparation_work(n).map_err(&record),
            &mut |n| cell.borrow_mut().reserve_hierarchy_index(n).map(|lease| SourceIndexLease { _lease: lease, bytes: n }).map_err(&record),
        );
        if let Some(e) = failed.into_inner() {
            return Err(e);
        }
        let index = index.map_err(|e| preparation_error(E::Backing(e)))?;
        let mut budget = SharedBudget(cell.into_inner());
        // This completed-token accessor scans all entries; prepay its actual count.
        let masks = doc.xcf_state.as_ref().ok_or(CompositionError::Unsupported("hierarchy state"))?.filter_masks.len();
        charge(&mut budget, i, (masks as u64).checked_add(1).ok_or(CompositionError::Unsupported("hierarchy count"))?).map_err(preparation_error)?;
        if index.raw_nonempty_count() == 0 {
            return Err(CompositionError::Unsupported("masked hierarchy Source requires a raw nonempty mask"));
        }
        let (plan, lease) = compile(doc, &index, Purpose::MaskedSource, std::mem::size_of::<Self>() as u64, i, &mut budget).map_err(preparation_error)?;
        Ok(Self { _plan: plan, _source: source, _lease: lease })
    }
}
#[derive(Clone, Copy)]
enum Purpose {
    Source,
    Current,
    MaskedSource,
    MaskedCurrent,
}
impl Purpose {
    fn source_topology(self) -> bool {
        matches!(self, Self::Source | Self::MaskedSource)
    }
    fn allows_mixed_opacity(self) -> bool {
        matches!(self, Self::Current | Self::MaskedCurrent)
    }
}
struct Node<L> {
    id: LayerId,
    parent: Option<usize>,
    subtree_end: usize,
    bounds: Rect,
    visible: bool,
    opacity: f64,
    leaf: Option<Leaf<L>>,
}
struct Leaf<L> {
    descriptor: NativePlaneDescriptor,
    point: Option<RootPointPlan<L>>,
    _lease: Option<L>,
}
struct Plan<L> {
    nodes: Vec<Node<L>>,
    current: Profile,
    working: WorkingSpace,
    format: PixelFormat,
    bounds: Rect,
    max_depth: usize,
}
fn header(doc: &Document) -> Result<(), E> {
    let s = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    crate::gimp_semi_flatten::positive_origin(doc.bounds()).map_err(E::Color)?;
    if doc.layers.is_empty()
        || doc.layers.len() > 4096
        || s.items.len() > 8192
        || s.filter_stacks.len() > 4096
        || s.filter_masks.len() > 4096
        || s.opaque.len() > 4096
        || doc.mode != ColorMode::Rgb
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!((doc.depth, source.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750))
        || source.original_icc.is_some()
        || s.provenance.format != "xcf"
        || !matches!(s.provenance.format_version, 12 | 13 | 22)
        || s.current_filter_output_precision != FilterOutputPrecision::Canonical
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !s.source_items.is_empty()
        || !s.editable_indexed.is_empty()
        || !s.resources.is_empty()
        || !s.item_sets.is_empty()
        || s.floating.is_some()
        || s.selection.selected.len() > 4096
        || !s.selection.selected_paths.is_empty()
        || s.selection.mask_target.is_some()
    {
        return Err(scope());
    }
    Ok(())
}
fn unit(raw: &[u8], format: PixelFormat) -> Result<(), E> {
    if raw.len() != format.bytes_per_pixel() {
        return Err(length());
    }
    for c in 0..4 {
        let v = try_read_sample_f64(raw, format.sample, c).map_err(|_| length())?;
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(scope());
        }
    }
    Ok(())
}
fn qualify_node(
    node: &photocraft_doc::xcf_filters::FilterNode,
    purpose: Purpose,
    bounds: Rect,
    interrupt: &Interrupt<'_>,
    budget: &mut impl PlanBudget,
) -> Result<(), E> {
    // Canonical metadata comparisons and the fixed Semi/Hue/Vibrance guards are separately paid.
    charge(budget, interrupt, 512)?;
    if crate::gimp_stock_rgb::claims(node) {
        charge(budget, interrupt, crate::gimp_stock_rgb::CANONICAL_WORK)?;
        (if purpose.allows_mixed_opacity() { crate::gimp_stock_rgb::canonical_mixed_opacity } else { crate::gimp_stock_rgb::canonical })(
            node,
            SampleEncoding::NonlinearProfile,
            22,
            bounds,
        )
        .map_err(E::Color)?;
    } else if crate::gimp_affine::claims(node) {
        charge(budget, interrupt, 512)?;
        (if purpose.allows_mixed_opacity() { crate::gimp_affine::canonical_mixed_opacity } else { crate::gimp_affine::canonical })(
            node,
            SampleEncoding::NonlinearProfile,
            22,
            bounds,
        )
        .map_err(E::Color)?;
    } else if crate::gimp_invert::claims(node) {
        charge(budget, interrupt, 128)?;
        (if purpose.allows_mixed_opacity() { crate::gimp_invert::canonical_mixed_opacity } else { crate::gimp_invert::canonical })(
            node,
            SampleEncoding::NonlinearProfile,
            22,
            bounds,
        )
        .map_err(E::Color)?;
    } else if crate::gimp_exposure::claims(node) {
        charge(budget, interrupt, 512)?;
        if node.operation.schema_epoch == crate::gimp_exposure::LOADED_MIXED_EPOCH {
            let parameters = (if purpose.allows_mixed_opacity() {
                crate::gimp_exposure::canonical_loaded_mixed_opacity
            } else {
                crate::gimp_exposure::canonical_loaded_mixed
            })(node, SampleEncoding::NonlinearProfile, 22, bounds)
            .map_err(E::Color)?;
            let [_, raw_black] = parameters.raw_words();
            if purpose.source_topology() && !matches!(raw_black, 0x3dcc_cccd | 0xbdcc_cccd) {
                return Err(scope());
            }
        } else {
            (if purpose.allows_mixed_opacity() { crate::gimp_exposure::canonical_mixed_opacity } else { crate::gimp_exposure::canonical })(
                node,
                SampleEncoding::NonlinearProfile,
                22,
                bounds,
            )
            .map_err(E::Color)?;
        }
    } else if crate::gimp_semi_flatten::claims(node) {
        (if purpose.allows_mixed_opacity() { crate::gimp_semi_flatten::canonical_mixed_opacity } else { crate::gimp_semi_flatten::canonical })(
            node,
            SampleEncoding::NonlinearProfile,
            22,
            bounds,
        )
        .map_err(E::Color)?;
    } else if crate::gimp_hue_chroma::claims(node) {
        (if purpose.allows_mixed_opacity() { crate::gimp_hue_chroma::canonical_mixed_opacity } else { crate::gimp_hue_chroma::canonical })(
            node,
            SampleEncoding::NonlinearProfile,
            22,
            bounds,
        )
        .map_err(E::Color)?;
    } else if crate::gimp_vibrance::claims(node) {
        (if purpose.allows_mixed_opacity() { crate::gimp_vibrance::canonical_mixed_opacity } else { crate::gimp_vibrance::canonical })(
            node,
            SampleEncoding::NonlinearProfile,
            22,
            bounds,
        )
        .map_err(E::Color)?;
    } else {
        return Err(scope());
    }

    Ok(())
}
fn compile<'doc, P: TreeIndex<'doc>, B: PlanBudget>(
    doc: &'doc Document,
    index: &P,
    purpose: Purpose,
    outer: u64,
    i: &Interrupt<'_>,
    b: &mut B,
) -> Result<(Plan<B::Lease>, B::Lease), E> {
    charge(b, i, 1)?;
    header(doc)?;
    let s = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let count = index.rows().len();
    if !index.matches_document(doc) || count == 0 || count > 4096 || index.group_count() == 0 || index.leaf_count() == 0 || index.max_depth() > 32 {
        return Err(scope());
    }
    let bytes = count
        .checked_mul(2)
        .and_then(|n| n.checked_mul(std::mem::size_of::<Node<B::Lease>>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<Plan<B::Lease>>() + 128))
        .ok_or_else(length)?;
    let envelope = (bytes as u64).checked_add(outer).and_then(|n| n.checked_add(2 * 65536)).ok_or_else(length)?;
    let lease = b.reserve(envelope)?;
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(count).map_err(|_| E::Allocation)?;
    if nodes.capacity() > count.checked_mul(2).ok_or_else(length)? {
        return Err(E::Allocation);
    }
    let (current, basis) = bound_profile(doc, None, b, i)?;
    let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(|_| scope())?;
    charge(b, i, semantic.checked_mul(3).and_then(|n| n.checked_add(65536 * 64)).ok_or_else(length)?)?;
    let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, i).map_err(E::Color)?;
    let format = doc.pixel_format();
    let bounds = doc.bounds();
    let mut matched_stacks = 0usize;
    let mut found_nodes = 0usize;
    for row in index.rows() {
        charge(b, i, 512)?;
        let layer = row.layer;
        let item = s.items.get(row.canonical_item_index).ok_or_else(length)?;
        if row.ordinal != nodes.len()
            || row.id != layer.id
            || item.item != ItemRef::Layer(layer.id)
            || item.parent != row.parent.and_then(|p| index.rows().get(p).map(|r| ItemRef::Layer(r.id)))
            || item.visible != layer.visible
            || item.mask.is_some()
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.blend != photocraft_color::blend::BlendMode::Normal
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.link_group.is_some()
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.indexed_source.is_some()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
        {
            return Err(scope());
        }
        if item.name.len() > 1 << 20 || layer.name.len() > 1 << 20 {
            return Err(scope());
        }
        charge(b, i, (item.name.len() as u64).checked_add(layer.name.len() as u64).ok_or_else(length)?)?;
        if item.name != layer.name {
            return Err(scope());
        }
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        let resolved = policy.checked_resolved().map_err(|_| scope())?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        let normal_epoch = match s.provenance.format_version {
            12 => "xcf-12-normal",
            13 => "xcf-13-normal",
            22 => "xcf-22-normal",
            _ => return Err(scope()),
        };
        if policy.compatibility_epoch != normal_epoch
            || policy.source_mode != 28
            || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1)
            || policy.legacy
            || policy.pass_through
            || resolved.operator != GimpOperator::Union
            || resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || physical.pass_through
            || physical.blend_encoding != SampleEncoding::Linear
            || physical.composite_encoding != SampleEncoding::Linear
        {
            return Err(scope());
        }
        let mut leaf = None;
        match &layer.content {
            LayerContent::Group(group) => {
                if group.artboard.is_some() || item.raster_bounds.is_some() || row.stack_index.is_some() {
                    return Err(scope());
                }
            }
            LayerContent::BackedRaster(_) | LayerContent::Raster(_) => {
                if item.raster_bounds != Some(bounds) || row.operative_bounds != bounds || row.first_child.is_some() {
                    return Err(scope());
                }
                let descriptor = match &layer.content {
                    LayerContent::BackedRaster(v) => {
                        if v.operative_bounds() != bounds || v.format() != format {
                            return Err(scope());
                        }
                        v.descriptor()
                    }
                    LayerContent::Raster(v) => {
                        if v.format() != format {
                            return Err(scope());
                        }
                        NativePlaneDescriptor::new(format, bounds, v.default_bytes()).map_err(E::Backing)?
                    }
                    _ => return Err(scope()),
                };
                if descriptor.default_bytes().iter().any(|v| *v != 0) {
                    return Err(scope());
                }
                unit(descriptor.default_bytes(), format)?;
                let (point, fee) = if let Some(si) = row.stack_index {
                    let stack = s.filter_stacks.get(si).ok_or_else(length)?;
                    if stack.owner != ItemRef::Layer(layer.id) || stack.nodes.len() > 1024 || (!stack.nodes.is_empty() && s.provenance.format_version != 22) {
                        return Err(scope());
                    }
                    matched_stacks = matched_stacks.checked_add(1).ok_or_else(length)?;
                    found_nodes = found_nodes.checked_add(stack.nodes.len()).ok_or_else(length)?;
                    if found_nodes > 4096 {
                        return Err(scope());
                    }
                    let bindings = index.bindings_for_stack(si);
                    if matches!(purpose, Purpose::MaskedSource | Purpose::MaskedCurrent) && bindings.map(|v| v.len()) != Some(stack.nodes.len()) {
                        return Err(scope());
                    }
                    for (ni, node) in stack.nodes.iter().enumerate() {
                        let raw = if let Some(bindings) = bindings {
                            charge(b, i, 512)?;
                            let binding = bindings.get(ni).ok_or_else(length)?;
                            if binding.leaf_ordinal != row.ordinal || binding.stack_index != si || binding.node_index != ni || binding.filter_id != node.id {
                                return Err(scope());
                            }
                            binding.classification == photocraft_doc::xcf_filter_masks::FilterMaskCoverageClass::RawNonemptyCoverage
                        } else {
                            false
                        };
                        if raw {
                            crate::gimp_masked_mixed::canonical(node, bounds).map_err(E::Color)?;
                        } else {
                            qualify_node(node, purpose, bounds, i, b)?;
                        }
                    }
                    if stack.nodes.is_empty() {
                        (None, None)
                    } else if let Some(bindings) = bindings {
                        // Private temporary node-order references are prepaid before allocation.
                        let bytes = bindings
                            .len()
                            .checked_mul(2)
                            .and_then(|n| n.checked_mul(std::mem::size_of::<(&photocraft_doc::FilterMask, bool)>()))
                            .and_then(|n| n.checked_add(std::mem::size_of::<Vec<(&photocraft_doc::FilterMask, bool)>>() + 128))
                            .ok_or_else(length)?;
                        let binding_fee = b.reserve(bytes as u64)?;
                        let mut pairs = Vec::new();
                        pairs.try_reserve_exact(bindings.len()).map_err(|_| E::Allocation)?;
                        if pairs.capacity() > bindings.len().checked_mul(2).ok_or_else(length)? {
                            return Err(E::Allocation);
                        }
                        for binding in bindings {
                            charge(b, i, 1)?;
                            pairs
                                .push((binding.mask, binding.classification == photocraft_doc::xcf_filter_masks::FilterMaskCoverageClass::RawNonemptyCoverage));
                        }
                        let result = RootPointPlan::prepare_masked(stack, &pairs, format, bounds, purpose.allows_mixed_opacity(), i, b)?;
                        drop(pairs);
                        drop(binding_fee);
                        (Some(result.0), Some(result.1))
                    } else {
                        let (p, l) = RootPointPlan::prepare(stack, format, bounds, purpose.allows_mixed_opacity(), i, b)?;
                        (Some(p), Some(l))
                    }
                } else {
                    (None, None)
                };
                // Immediate payload-before-fee holder protects every later source validation error.
                leaf = Some(Leaf { descriptor, point, _lease: fee });
                if let LayerContent::Raster(surface) = &layer.content {
                    let pixels = (surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_add(1)).ok_or_else(length)?;
                    charge(b, i, pixels.checked_mul(64).ok_or_else(length)?)?;
                    for (_, tile) in surface.tiles() {
                        if tile.bytes().len() != 65536usize.checked_mul(format.bytes_per_pixel()).ok_or_else(length)? {
                            return Err(length());
                        }
                        for chunk in tile.bytes().chunks(format.bytes_per_pixel() * 64) {
                            i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
                            for raw in chunk.chunks_exact(format.bytes_per_pixel()) {
                                unit(raw, format)?;
                            }
                        }
                    }
                }
            }
            _ => return Err(scope()),
        }
        nodes.push(Node {
            id: layer.id,
            parent: row.parent,
            subtree_end: row.subtree_end,
            bounds: row.operative_bounds,
            visible: layer.visible,
            opacity: f64::from(layer.opacity),
            leaf,
        });
    }
    if matched_stacks != s.filter_stacks.len() || found_nodes != s.filter_masks.len() {
        return Err(scope());
    }
    for (pos, id) in s.selection.selected.iter().enumerate() {
        let mut present = false;
        for node in &nodes {
            charge(b, i, 1)?;
            present |= *id == ItemRef::Layer(node.id);
        }
        for prev in s.selection.selected.get(..pos).ok_or_else(length)? {
            charge(b, i, 1)?;
            if prev == id {
                return Err(scope());
            }
        }
        if !present {
            return Err(scope());
        }
    }
    if let Some(active) = s.selection.active {
        let mut selected = false;
        for id in &s.selection.selected {
            charge(b, i, 1)?;
            selected |= *id == active;
        }
        if !selected {
            return Err(scope());
        }
    }
    charge(b, i, 0)?;
    Ok((Plan { nodes, current, working, format, bounds, max_depth: index.max_depth() }, lease))
}
pub(crate) struct Context<'a> {
    owner: &'a AccountedDocument,
    plan: Plan<QuotaLease>,
    index: CurrentIndex<'a>,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}
impl<'a> Context<'a> {
    pub(crate) fn new(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        let max = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > max.max_rows || limits.max_pixels == 0 || limits.max_pixels > max.max_pixels {
            return Err(scope());
        }
        header(owner.document())?;
        owner.validate_readonly_normal_hierarchy_storage(c).map_err(E::Backing)?;
        let index = owner.checked_readonly_normal_hierarchy_index(c).map_err(E::Backing)?;
        let (plan, lease) = compile(
            owner.document(),
            &index,
            Purpose::Current,
            std::mem::size_of::<Self>() as u64,
            c.interrupt,
            &mut NativeBudget { quota: owner.quota(), control: c },
        )?;
        Ok(Self { owner, plan, index: CurrentIndex::Zero(index), limits, _lease: lease })
    }
    pub(crate) fn new_current(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        let max = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > max.max_rows || limits.max_pixels == 0 || limits.max_pixels > max.max_pixels {
            return Err(scope());
        }
        header(owner.document())?;
        owner.validate_readonly_normal_hierarchy_current_storage(c).map_err(E::Backing)?;
        let index = owner.checked_readonly_normal_hierarchy_current_index(c).map_err(E::Backing)?;
        let (plan, lease) = compile(
            owner.document(),
            &index,
            Purpose::MaskedCurrent,
            std::mem::size_of::<Self>() as u64,
            c.interrupt,
            &mut NativeBudget { quota: owner.quota(), control: c },
        )?;
        Ok(Self { owner, plan, index: CurrentIndex::Coverage(index), limits, _lease: lease })
    }
    pub(crate) fn bounds(&self) -> Rect {
        self.plan.bounds
    }
    pub(crate) fn current_profile(&self) -> &Profile {
        &self.plan.current
    }
    pub(crate) fn sample_encoding(&self) -> SampleEncoding {
        SampleEncoding::NonlinearProfile
    }
    pub(crate) fn validate_operative(&self, c: &ReadControl<'_>) -> Result<(), E> {
        stack::validate(self, c)
    }
    pub(crate) fn render_band(&self, r: Rect, c: &ReadControl<'_>) -> Result<OwnedNormalStackBand, E> {
        stack::render(self, r, c)
    }
}

#[cfg(test)]
#[path = "backed_normal_hierarchy_tests.rs"]
mod tests;
