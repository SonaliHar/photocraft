//! Paid whole-document compiler for flat plain/pure-point roots. Native source
//! words and actual global identities stay in their original owners; no fake
//! singleton Document, derived native store, or per-root profile clone exists.
use crate::{
    backed_normal::{BackedNormalError as E, NormalBandLimits},
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
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, NativePlaneDescriptor, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
#[path = "backed_filtered_flat_point_source_opacity.rs"]
mod source_opacity;
#[path = "backed_filtered_flat_point_stack.rs"]
mod stack;
use crate::backed_normal_stack::OwnedNormalStackBand;
fn scope() -> E {
    E::Unsupported("closed readonly filtered flat pure-point roots")
}
fn length() -> E {
    E::Invalid("filtered flat size/count/capacity overflow")
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
fn text_equal(a: &str, z: &str, b: &mut impl PlanBudget, i: &Interrupt<'_>) -> Result<bool, E> {
    if a.len() > 1 << 20 || z.len() > 1 << 20 {
        return Err(scope());
    }
    charge(b, i, (a.len() as u64).checked_add(z.len() as u64).ok_or_else(length)?)?;
    Ok(a == z)
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
        _ => CompositionError::Unsupported("filtered flat preparation refused"),
    }
}
pub(super) struct Entry<L> {
    index: usize,
    id: LayerId,
    visible: bool,
    opacity: f64,
    descriptor: NativePlaneDescriptor,
    point: Option<RootPointPlan<L>>,
    _lease: Option<L>,
}
pub(super) struct Plan<L> {
    entries: Vec<Entry<L>>,
    current: Profile,
    working: WorkingSpace,
    format: PixelFormat,
    bounds: Rect,
}
/// Source/model custody and all root math are destroyed before preparation fees.
pub struct LeasedFilteredFlatPointPipeline<S, L> {
    _plan: Plan<L>,
    _source: S,
    _lease: L,
}
impl<S, L> LeasedFilteredFlatPointPipeline<S, L> {
    pub fn new_budgeted<B: PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let header = u64::try_from(std::mem::size_of::<Self>()).map_err(|_| CompositionError::Invalid("filtered flat source header"))?;
        let (plan, lease) = compile(doc, Purpose::Source, None, header, interrupt, &mut SharedBudget(budget)).map_err(preparation_error)?;
        Ok(Self { _plan: plan, _source: source_lease, _lease: lease })
    }
    /// Fresh original-XCF mapper contract only: retained property groups prove
    /// exact F32 opacity promotion before Source-topology preparation. This is
    /// not CURRENT provenance recovery or an ordinary Point source endpoint.
    pub fn new_opacity_source_budgeted<B: PreparationBudget<Lease=L>>(
        doc:&Document, interrupt:&Interrupt<'_>, budget:&mut B, source_lease:S,
    )->Result<Self,CompositionError> {
        let header=u64::try_from(std::mem::size_of::<Self>()).map_err(|_|CompositionError::Invalid("filtered flat opacity source header"))?;
        let(plan,lease)=compile(doc,Purpose::SourceOpacity,None,header,interrupt,&mut SharedBudget(budget)).map_err(preparation_error)?;
        Ok(Self{_plan:plan,_source:source_lease,_lease:lease})
    }
}
/// Separate fresh masked Source endpoint; original quota and work ledger are explicit.
pub struct LeasedMaskedFilteredFlatPointPipeline<S,L>{_plan:Plan<L>,_source:S,_lease:L}
struct SourceProofLease<L>{_lease:L,bytes:u64}
impl<L> photocraft_doc::xcf_filter_masks::MaskCoverageSetLease for SourceProofLease<L>{
 fn bytes(&self)->u64{self.bytes}
}
impl<S,L> LeasedMaskedFilteredFlatPointPipeline<S,L>{
 pub fn new_budgeted<B:crate::backed_masked_filtered_flat_point_context::MaskedFilterPreparationBudget<Lease=L>>(doc:&Document,interrupt:&Interrupt<'_>,budget:&mut B,source_lease:S)->Result<Self,CompositionError>{
 let cell=std::cell::RefCell::new(budget);let failed=std::cell::RefCell::new(None);
 let record=|error:CompositionError|{let cancelled=interrupt.check().is_err();if failed.borrow().is_none(){*failed.borrow_mut()=Some(error);}if cancelled{BackingError::Cancelled}else{BackingError::Work}};
 let proofs=doc.checked_f32_filter_mask_coverages_with_reservation(photocraft_doc::xcf_filter_masks::MaskCoverageSetLimits::STANDARD,interrupt,&|units|cell.borrow_mut().consume_preparation_work(units).map_err(&record),&|bytes|cell.borrow_mut().reserve_filter_mask_proof_index(bytes).map(|lease|SourceProofLease{_lease:lease,bytes}).map_err(&record));
 if let Some(error)=failed.into_inner(){return Err(error);}
 let proofs=proofs.map_err(|e|preparation_error(E::Backing(e)))?;
 let original=cell.into_inner();let mut budget=SharedBudget(&mut *original);
 let bindings=MaskBindings::build(doc,&proofs,interrupt,&mut budget).map_err(preparation_error)?;
 let(plan,lease)=compile(doc,Purpose::MaskedSource,Some(bindings),std::mem::size_of::<Self>()as u64,interrupt,&mut budget).map_err(preparation_error)?;
 Ok(Self{_plan:plan,_source:source_lease,_lease:lease})
 }
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
fn numeric_zero(bytes: &[u8], sample: SampleType, i: &Interrupt<'_>) -> Result<(), E> {
    let size = sample.bytes();
    if bytes.len() % size != 0 {
        return Err(length());
    }
    for block in bytes.chunks(size * 64) {
        i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
        for raw in block.chunks_exact(size) {
            let v = try_read_sample_f64(raw, sample, 0).map_err(|_| length())?;
            if !v.is_finite() || v != 0. {
                return Err(scope());
            }
        }
    }
    Ok(())
}
struct MaskBindings<'a,L>{rows:Vec<(&'a photocraft_doc::FilterMask,bool)>,offsets:Vec<usize>,_fee:L}
impl<'a,L> MaskBindings<'a,L>{
 fn build<B:PlanBudget<Lease=L>,P:photocraft_doc::xcf_filter_masks::MaskCoverageSetLease>(doc:&'a Document,proofs:&photocraft_doc::xcf_filter_masks::CheckedFilterMaskCoverageSet<'_,P>,i:&Interrupt<'_>,b:&mut B)->Result<Self,E>{
 doc.validate_xcf_state_with_checked_filter_masks(proofs,i).map_err(|_|if i.check().is_err(){E::Backing(BackingError::Cancelled)}else{scope()})?;
 let state=doc.xcf_state.as_ref().ok_or_else(scope)?;
 let count=state.filter_masks.len();let offsets=state.filter_stacks.len().checked_add(1).ok_or_else(length)?;
 let bytes=count.checked_mul(2).and_then(|n|n.checked_mul(std::mem::size_of::<(&photocraft_doc::FilterMask,bool)>())).and_then(|n|n.checked_add(offsets.checked_mul(2)?.checked_mul(std::mem::size_of::<usize>())?)).and_then(|n|n.checked_add(std::mem::size_of::<Self>()+128)).ok_or_else(length)?;
 let fee=b.reserve(bytes as u64)?;
 let mut result=Self{rows:Vec::new(),offsets:Vec::new(),_fee:fee};
 result.rows.try_reserve_exact(count).map_err(|_|E::Allocation)?;result.offsets.try_reserve_exact(offsets).map_err(|_|E::Allocation)?;
 if result.rows.capacity()>count.checked_mul(2).ok_or_else(length)?||result.offsets.capacity()>offsets.checked_mul(2).ok_or_else(length)?{return Err(E::Allocation);}
 let mut nonempty=0usize;
 for stack in &state.filter_stacks{result.offsets.push(result.rows.len());for node in &stack.nodes{
 let mut found=None;
 for(ordinal,mask)in state.filter_masks.iter().enumerate(){charge(b,i,1)?;if node.mask==Some(ItemRef::Mask(mask.id)){if found.is_some()||mask.owner!=node.id{return Err(scope());}found=Some((ordinal,mask));}}
 let(ordinal,mask)=found.ok_or_else(scope)?;let class=proofs.classification_for_mask(ordinal,mask).map_err(E::Backing)?;
 let raw=class==photocraft_doc::xcf_filter_masks::FilterMaskCoverageClass::RawNonemptyCoverage;
 nonempty=nonempty.checked_add(usize::from(raw)).ok_or_else(length)?;
 if result.rows.len()>=count{return Err(scope());}result.rows.push((mask,raw));
 }}result.offsets.push(result.rows.len());if result.rows.len()!=count{return Err(scope());}
 // Source-only nonempty requirement is checked by compile; CURRENT may retain empty anchors.
 let _=nonempty;
 Ok(result)
 }
 fn stack(&self,index:usize)->Result<&[(&'a photocraft_doc::FilterMask,bool)],E>{let start=*self.offsets.get(index).ok_or_else(length)?;let end=*self.offsets.get(index.checked_add(1).ok_or_else(length)?).ok_or_else(length)?;self.rows.get(start..end).ok_or_else(length)}
 fn at(&self,stack:usize,node:usize)->Result<(&'a photocraft_doc::FilterMask,bool),E>{self.stack(stack)?.get(node).copied().ok_or_else(length)}
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose { Source, SourceOpacity, Current, MaskedSource, MaskedCurrent }
impl Purpose {
    fn source_topology(self)->bool { matches!(self,Self::Source|Self::SourceOpacity|Self::MaskedSource) }
    fn allows_mixed_opacity(self)->bool { matches!(self,Self::SourceOpacity|Self::Current|Self::MaskedCurrent) }
    fn masked(self)->bool { matches!(self,Self::MaskedSource|Self::MaskedCurrent) }
}
fn compile<B: PlanBudget>(doc: &Document, purpose: Purpose, mask_bindings:Option<MaskBindings<'_,B::Lease>>, header: u64, interrupt: &Interrupt<'_>, budget: &mut B) -> Result<(Plan<B::Lease>, B::Lease), E> {
    charge(budget, interrupt, 1)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    let count = doc.layers.len();
    let filtered = state.filter_stacks.len();
    let bounds = doc.bounds();
    crate::gimp_semi_flatten::positive_origin(bounds).map_err(E::Color)?;
    if !(1..=4096).contains(&count)
        || filtered == 0
        || filtered > count
        || state.filter_masks.len() > 4096
        || state.items.len() > 8192
        || doc.mode != ColorMode::Rgb
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!((doc.depth, source.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750))
        || source.original_icc.is_some()
        || state.provenance.format != "xcf"
        || state.provenance.format_version != 22
        || state.current_filter_output_precision != FilterOutputPrecision::Canonical
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
        || state.selection.selected.len() > count
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
    {
        return Err(scope());
    }
    let mut nodes = 0usize;
    for stack in &state.filter_stacks {
        charge(budget, interrupt, 1)?;
        if (purpose.source_topology() && stack.nodes.is_empty()) || stack.nodes.len() > 1024 { return Err(scope()); }
        nodes = nodes.checked_add(stack.nodes.len()).ok_or_else(length)?;
        if nodes > 4096 { return Err(scope()); }
    }
    if (purpose.source_topology() && (nodes == 0 || (count == 1 && nodes < 2))) || state.filter_masks.len() != nodes || state.items.len() != count.checked_add(nodes).ok_or_else(length)? { return Err(scope()); }
    if purpose == Purpose::SourceOpacity { source_opacity::verify(doc, interrupt, budget)?; }
    if purpose.masked()!=mask_bindings.is_some(){return Err(scope());}
    if let Some(bindings)=&mask_bindings {
        charge(budget,interrupt,nodes as u64)?;
        if purpose==Purpose::MaskedSource&&!bindings.rows.iter().any(|(_,raw)|*raw){return Err(scope());}
    }
    // All dynamic retained state is admitted before Vec reserve/fill or profile
    // construction; source adapters retain the full conservative bound.
    let metadata = count
        .checked_mul(2)
        .and_then(|n| n.checked_mul(std::mem::size_of::<Entry<B::Lease>>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<Plan<B::Lease>>()))
        .ok_or_else(length)?;
    let envelope = u64::try_from(metadata).map_err(|_| length())?.checked_add(header).and_then(|n| n.checked_add(2 * 65536)).ok_or_else(length)?;
    let lease = budget.reserve(envelope)?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(count).map_err(|_| E::Allocation)?;
    if entries.capacity() > count.checked_mul(2).ok_or_else(length)? {
        return Err(E::Allocation);
    }
    if !purpose.masked() {
    // Preserve the original root's physical-mask allowance across all its
    // ordered nodes; no per-node multiplier. Admit counts before census/scans.
    for stack in &state.filter_stacks {
        let mut root_tiles = 0usize;
        for node in &stack.nodes {
            charge(budget, interrupt, 1)?;
            let Some(ItemRef::Mask(id)) = node.mask else { return Err(scope()) };
            let mut found = None;
            for mask in &state.filter_masks {
                charge(budget, interrupt, 1)?;
                if mask.id == id { if found.is_some() { return Err(scope()); } found = Some(mask); }
            }
            root_tiles = root_tiles.checked_add(found.ok_or_else(scope)?.surface.tile_count()).ok_or_else(length)?;
            if root_tiles > 4096 { return Err(scope()); }
        }
    }
    // O(1) per-mask cap and checked global actual work before physical scans.
    let mut physical_work = 0u64;
    for mask in &state.filter_masks {
        charge(budget, interrupt, 1)?;
        if mask.surface.tile_count() > 4096 {
            return Err(scope());
        }
        let pixels = (mask.surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_add(1)).ok_or_else(length)?;
        physical_work = physical_work.checked_add(pixels.checked_mul(32).ok_or_else(length)?).ok_or_else(length)?;
    }
    charge(budget, interrupt, physical_work)?;
    }
    let (current, basis) = bound_profile(doc, None, budget, interrupt)?;
    let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(|_| scope())?;
    charge(budget, interrupt, semantic.checked_mul(3).and_then(|n| n.checked_add(65536 * 64)).ok_or_else(length)?)?;
    let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, interrupt).map_err(E::Color)?;
    let format = doc.pixel_format();
    let mut found_filters = 0usize;
    let mut found_stacks = 0usize;
    for (index, layer) in doc.layers.iter().enumerate() {
        charge(budget, interrupt, 1)?;
        if layer.id.0 == 0
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
        for prior in doc.layers.get(..index).ok_or_else(length)? {
            charge(budget, interrupt, 1)?;
            if prior.id == layer.id {
                return Err(scope());
            }
        }
        let mut item = None;
        for row in &state.items {
            charge(budget, interrupt, 1)?;
            if row.item == ItemRef::Layer(layer.id) {
                if item.is_some() {
                    return Err(scope());
                }
                item = Some(row)
            }
        }
        let item = item.ok_or_else(scope)?;
        if item.parent.is_some()
            || item.mask.is_some()
            || item.raster_bounds != Some(bounds)
            || item.visible != layer.visible
            || !text_equal(&item.name, &layer.name, budget, interrupt)?
        {
            return Err(scope());
        }
        let composition = item.composition.as_ref().ok_or_else(scope)?;
        let resolved = composition.checked_resolved().map_err(|_| scope())?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if composition.compatibility_epoch != "xcf-22-normal"
            || composition.source_mode != 28
            || (composition.stored_blend_space, composition.stored_composite_space, composition.stored_operator) != (0, -1, -1)
            || composition.legacy
            || composition.pass_through
            || resolved.operator != GimpOperator::Union
            || resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || physical.pass_through
            || physical.blend_encoding != SampleEncoding::Linear
            || physical.composite_encoding != SampleEncoding::Linear
        {
            return Err(scope());
        }
        let descriptor = match &layer.content {
            LayerContent::BackedRaster(backed) => {
                if backed.operative_bounds() != bounds || backed.format() != format {
                    return Err(scope());
                }
                backed.descriptor()
            }
            LayerContent::Raster(surface) => {
                if surface.format() != format {
                    return Err(scope());
                }
                NativePlaneDescriptor::new(format, bounds, surface.default_bytes()).map_err(E::Backing)?
            }
            _ => return Err(scope()),
        };
        if descriptor.default_bytes().iter().any(|v| *v != 0) {
            return Err(scope());
        }
        unit(descriptor.default_bytes(), format)?;
        let mut root_stack = None;
        for (stack_index,stack) in state.filter_stacks.iter().enumerate() {
            charge(budget, interrupt, 1)?;
            if stack.owner == ItemRef::Layer(layer.id) {
                if root_stack.is_some() {
                    return Err(scope());
                }
                root_stack = Some((stack_index,stack))
            }
        }
        let (point, root_lease) = if let Some((stack_index,stack)) = root_stack {
            found_stacks = found_stacks.checked_add(1).ok_or_else(length)?;
            for (node_index,node) in stack.nodes.iter().enumerate() {
                let nonempty=if let Some(bindings)=&mask_bindings {bindings.at(stack_index,node_index)?.1}else{false};
                if node.id.0 == 0 {
                    return Err(scope());
                }
                // Exact family canonical policies are unchanged, separately selected.
                if nonempty {
                    charge(budget,interrupt,512)?;
                    if !crate::gimp_masked_mixed::claims(node){return Err(scope());}
                    crate::gimp_masked_mixed::canonical(node,bounds).map_err(E::Color)?;
                } else if crate::gimp_stock_rgb::claims(node) {
                    charge(budget,interrupt,crate::gimp_stock_rgb::CANONICAL_WORK)?;
                    (if purpose.allows_mixed_opacity(){crate::gimp_stock_rgb::canonical_mixed_opacity}else{crate::gimp_stock_rgb::canonical})(node,SampleEncoding::NonlinearProfile,22,bounds).map_err(E::Color)?;
                } else if crate::gimp_affine::claims(node) {
                    charge(budget,interrupt,512)?;
                    (if purpose.allows_mixed_opacity(){crate::gimp_affine::canonical_mixed_opacity}else{crate::gimp_affine::canonical})(node,SampleEncoding::NonlinearProfile,22,bounds).map_err(E::Color)?;
                } else if crate::gimp_invert::claims(node) {
                    charge(budget,interrupt,128)?;
                    (if purpose.allows_mixed_opacity() { crate::gimp_invert::canonical_mixed_opacity } else { crate::gimp_invert::canonical })(node,SampleEncoding::NonlinearProfile,22,bounds).map_err(E::Color)?;
                } else if crate::gimp_exposure::claims(node) {
                    charge(budget,interrupt,512)?;
                    if node.operation.schema_epoch==crate::gimp_exposure::LOADED_MIXED_EPOCH { let parameters=(if purpose.allows_mixed_opacity() { crate::gimp_exposure::canonical_loaded_mixed_opacity } else { crate::gimp_exposure::canonical_loaded_mixed })(node,SampleEncoding::NonlinearProfile,22,bounds).map_err(E::Color)?; let [_,raw_black]=parameters.raw_words(); if purpose.source_topology() && !matches!(raw_black,0x3dcc_cccd|0xbdcc_cccd) {return Err(scope());} } else { (if purpose.allows_mixed_opacity() { crate::gimp_exposure::canonical_mixed_opacity } else { crate::gimp_exposure::canonical })(node,SampleEncoding::NonlinearProfile,22,bounds).map_err(E::Color)?; }
                } else if crate::gimp_semi_flatten::claims(node) {
                    (if purpose.allows_mixed_opacity() { crate::gimp_semi_flatten::canonical_mixed_opacity } else { crate::gimp_semi_flatten::canonical })(node, SampleEncoding::NonlinearProfile, 22, bounds).map_err(E::Color)?;
                } else if crate::gimp_hue_chroma::claims(node) {
                    (if purpose.allows_mixed_opacity() { crate::gimp_hue_chroma::canonical_mixed_opacity } else { crate::gimp_hue_chroma::canonical })(node, SampleEncoding::NonlinearProfile, 22, bounds).map_err(E::Color)?;
                } else if crate::gimp_vibrance::claims(node) {
                    (if purpose.allows_mixed_opacity() { crate::gimp_vibrance::canonical_mixed_opacity } else { crate::gimp_vibrance::canonical })(node, SampleEncoding::NonlinearProfile, 22, bounds).map_err(E::Color)?;
                } else {
                    return Err(scope());
                }
                for other in &state.filter_stacks {
                    for retained in &other.nodes {
                        charge(budget, interrupt, 1)?;
                        if !std::ptr::eq(retained, node) && (retained.id == node.id || retained.mask == node.mask) { return Err(scope()); }
                    }
                }
                let Some(ItemRef::Mask(mask_id)) = node.mask else { return Err(scope()) };
                let mut mask = None;
                for m in &state.filter_masks {
                    charge(budget, interrupt, 1)?;
                    if m.id == mask_id {
                        if mask.is_some() {
                            return Err(scope());
                        }
                        mask = Some(m)
                    }
                }
                let mask = mask.ok_or_else(scope)?;
                let mut metadata = None;
                for row in &state.items {
                    charge(budget, interrupt, 1)?;
                    if row.item == ItemRef::Mask(mask_id) {
                        if metadata.is_some() {
                            return Err(scope());
                        }
                        metadata = Some(row)
                    }
                }
                let metadata = metadata.ok_or_else(scope)?;
                let m = metadata.mask.as_ref().ok_or_else(scope)?;
                if mask_id.0 == 0
                    || mask.owner != node.id
                    || mask.bounds != bounds
                    || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
                    || metadata.parent.is_some()
                    || metadata.composition.is_some()
                    || metadata.raster_bounds != Some(bounds)
                    || !metadata.visible
                    || metadata.source_flags != 0
                    || m.owner != ItemRef::Layer(layer.id)
                    || m.bounds != bounds
                    || m.offset != [0, 0]
                    || m.outside_f64_bits != 0f64.to_bits()
                    || !m.enabled
                    || m.shown
                    || m.editing
                    || m.linked
                {
                    return Err(scope());
                }
                if !purpose.masked() {
                numeric_zero(mask.surface.default_bytes(), doc.depth, interrupt)?;
                for (coordinate, tile) in mask.surface.tiles() {
                    if tile.bytes().len() != 65536usize.checked_mul(doc.depth.bytes()).ok_or_else(length)? {
                        return Err(length());
                    }
                    crate::gimp_semi_flatten::mask_tile(bounds, *coordinate).map_err(E::Color)?;
                    numeric_zero(tile.bytes(), doc.depth, interrupt)?;
                }
                }
            }
            found_filters = found_filters.checked_add(stack.nodes.len()).ok_or_else(length)?;
            if stack.nodes.is_empty() {
                // Explicit CURRENT empty anchor: native wide samples use the
                // existing plain Normal path; no lost operation is inferred.
                (None, None)
            } else {
                let(plan,fee)=if let Some(bindings)=&mask_bindings {
                    RootPointPlan::prepare_masked(stack,bindings.stack(stack_index)?,format,bounds,purpose.allows_mixed_opacity(),interrupt,budget)?
                }else{RootPointPlan::prepare(stack, format, bounds, purpose.allows_mixed_opacity(), interrupt, budget)?};
                (Some(plan), Some(fee))
            }
        } else {
            (None, None)
        };
        // Install payload-before-fee field order before any further fallible
        // source Surface work/validation can unwind this root's owned Parts.
        let entry = Entry { index, id: layer.id, visible: layer.visible, opacity: f64::from(layer.opacity), descriptor, point, _lease: root_lease };
        // Source-stage actual retained native Surface proof, including defaults
        // and padding. The native provider path is validated by the current walk.
        if let LayerContent::Raster(surface) = &layer.content {
            let pixels = (surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_add(1)).ok_or_else(length)?;
            charge(budget, interrupt, pixels.checked_mul(64).ok_or_else(length)?)?;
            for (_, tile) in surface.tiles() {
                if tile.bytes().len() != 65536usize.checked_mul(format.bytes_per_pixel()).ok_or_else(length)? {
                    return Err(length());
                }
                for block in tile.bytes().chunks(format.bytes_per_pixel() * 64) {
                    interrupt.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
                    for pixel in block.chunks_exact(format.bytes_per_pixel()) {
                        unit(pixel, format)?
                    }
                }
            }
        }
        entries.push(entry);
    }
    if found_filters != nodes || found_stacks != filtered {
        return Err(scope());
    }
    for (index, id) in state.selection.selected.iter().enumerate() {
        let mut present = false;
        for entry in &entries {
            charge(budget, interrupt, 1)?;
            present |= ItemRef::Layer(entry.id) == *id;
        }
        for previous in state.selection.selected.get(..index).ok_or_else(length)? {
            charge(budget, interrupt, 1)?;
            if previous == id {
                return Err(scope());
            }
        }
        if !present {
            return Err(scope());
        }
    }
    if let Some(id) = state.selection.active {
        let mut present = false;
        for entry in &entries {
            charge(budget, interrupt, 1)?;
            present |= ItemRef::Layer(entry.id) == id;
        }
        let mut selected = false;
        for selected_id in &state.selection.selected {
            charge(budget, interrupt, 1)?;
            selected |= *selected_id == id;
        }
        if !present || !selected {
            return Err(scope());
        }
    }
    // Every registry slot must belong to one checked actual root or its mask.
    for row in &state.items {
        charge(budget, interrupt, 1)?;
        if !matches!(row.item, ItemRef::Layer(_) | ItemRef::Mask(_)) {
            return Err(scope());
        }
    }
    charge(budget, interrupt, 0)?;
    Ok((Plan { entries, current, working, format, bounds }, lease))
}
pub(crate) struct Context<'a> {
    owner: &'a AccountedDocument,
    plan: Plan<QuotaLease>,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}
impl<'a> Context<'a> {
    pub(crate) fn new(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        let maximum = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > maximum.max_rows || limits.max_pixels == 0 || limits.max_pixels > maximum.max_pixels {
            return Err(scope());
        }
        owner.validate_readonly_filtered_flat_point_storage(c).map_err(E::Backing)?;
        let header = u64::try_from(std::mem::size_of::<Self>()).map_err(|_| length())?;
        let (plan, lease) = compile(owner.document(), Purpose::Current, None, header, c.interrupt, &mut NativeBudget { quota: owner.quota(), control: c })?;
        Ok(Self { owner, plan, limits, _lease: lease })
    }
    pub(crate) fn new_masked(owner:&'a AccountedDocument,limits:NormalBandLimits,c:&ReadControl<'_>)->Result<Self,E>{
        let maximum=NormalBandLimits::default();
        if limits.max_rows==0||limits.max_rows>maximum.max_rows||limits.max_pixels==0||limits.max_pixels>maximum.max_pixels{return Err(scope());}
        owner.validate_readonly_masked_filtered_flat_point_storage(c).map_err(E::Backing)?;
        let proofs=owner.document().checked_f32_filter_mask_coverages_with(photocraft_doc::xcf_filter_masks::MaskCoverageSetLimits::STANDARD,owner.quota(),c).map_err(E::Backing)?;
        let mut budget=NativeBudget{quota:owner.quota(),control:c};
        let bindings=MaskBindings::build(owner.document(),&proofs,c.interrupt,&mut budget)?;
        let(plan,lease)=compile(owner.document(),Purpose::MaskedCurrent,Some(bindings),std::mem::size_of::<Self>()as u64,c.interrupt,&mut budget)?;
        Ok(Self{owner,plan,limits,_lease:lease})
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
    pub(crate) fn render_band(&self, r: Rect, c: &ReadControl<'_>) -> Result<OwnedNormalStackBand, E> {
        stack::render(self, r, c)
    }
    pub(crate) fn validate_operative(&self, c: &ReadControl<'_>) -> Result<(), E> {
        stack::validate(self, c)
    }
}

#[cfg(test)]
#[path="backed_filtered_flat_point_tests.rs"]
mod tests;
