//! Explicit same-catalog promotion of the sealed source-selected Point class.
//! Runtime storage adoption is read-only: no source, renderer, edit or default
//! loader authority is inferred from the persisted node or receipt sizes.
use super::owned_plain_source::{existing_quota::PreparationHold, store};
#[path = "owned_point_source/census.rs"]
mod census;
use super::owned_xcf_source::PromotionSource;
use super::*;
use photocraft_doc::{BackedLayer, accounted_document::AccountedDocument};
use photocraft_raster::backing::{
    BackedRaster, BackingError, NativePlaneDescriptor, ReadControl,
    quota::{BackingQuota, QuotaBaseline},
};
use photocraft_xcf::{BudgetKind, OwnedReservationBatch, PhysicalUsage};
use std::sync::atomic::{AtomicU64, Ordering};

fn invalid(message: &str) -> IoError {
    IoError::Unsupported(message.into())
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| invalid("owned Point capacity overflow"))
}
fn kind(error: BackingError) -> ErrorKind {
    match error {
        BackingError::Cancelled => ErrorKind::Cancelled,
        BackingError::Quota | BackingError::Allocation | BackingError::Work | BackingError::Length => ErrorKind::Limit,
        BackingError::Unsupported => ErrorKind::Unsupported,
        _ => ErrorKind::Structure,
    }
}
fn backing(error: BackingError) -> IoError {
    accounting_error(XcfError { kind: kind(error), offset: 0, item: String::new(), message: format!("owned Point: {error}") })
}
fn combined(a: PhysicalUsage, b: PhysicalUsage) -> Result<PhysicalUsage, IoError> {
    Ok(PhysicalUsage {
        resident_bytes: add(a.resident_bytes, b.resident_bytes)?,
        index_bytes: add(a.index_bytes, b.index_bytes)?,
        scratch_bytes: add(a.scratch_bytes, b.scratch_bytes)?,
    })
}
// No Document or Surface in this owner, so stores/model cannot form an Arc cycle.
// Every source allocation dies before its actual decoder/surface/baseline fees.
struct Retention {
    _encoded: Arc<Vec<u8>>,
    _source: Option<OwnedReservationBatch>,
    _surfaces: OwnedReservationBatch,
    _baseline: QuotaBaseline,
    quota: BackingQuota,
}
impl store::RetainedSource for Retention {
    fn actual_quota(&self) -> &BackingQuota {
        &self.quota
    }
}
pub struct OwnedPointXcfImport {
    document: AccountedDocument,
    warnings: Vec<String>,
    origin: ImportOrigin,
    assessment: Option<crate::xcf_assessment::ImportAssessment>,
    source_read_only: bool,
    preview_only: bool,
    _retention: Arc<Retention>,
}
impl std::fmt::Debug for OwnedPointXcfImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedPointXcfImport").field("document_id", &self.document.document().id).finish_non_exhaustive()
    }
}
impl OwnedPointXcfImport {
    pub fn document(&self) -> &AccountedDocument {
        &self.document
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn origin(&self) -> ImportOrigin {
        self.origin
    }
    pub fn assessment(&self) -> Option<&crate::xcf_assessment::ImportAssessment> {
        self.assessment.as_ref()
    }
    pub fn source_read_only(&self) -> bool {
        self.source_read_only
    }
    pub fn preview_only(&self) -> bool {
        self.preview_only
    }
    pub fn into_accounted_document(self) -> AccountedDocument {
        self.document
    }
}
fn phase_work(doc: &Document, model: u64, payload: u64, source: u64, budget: &photocraft_xcf::Budget<'_>) -> Result<u64, IoError> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("point phase state"))?;
    let mut native = 0u64;
    let mut tiles = 0u64;
    // Real padded native tiles and defaults, including the retained resident mask.
    // No logical 8x6 estimate substitutes for those physical validation visits.
    for surface in doc
        .layers
        .iter()
        .filter_map(|layer| match &layer.content {
            LayerContent::Raster(s) => Some(s),
            _ => None,
        })
        .chain(state.filter_masks.iter().map(|mask| &mask.surface))
    {
        budget.consume_work(1, "point phase physical census").map_err(accounting_error)?;
        let count = surface.tile_count() as u64;
        tiles = add(tiles, count)?;
        native = add(
            native,
            count
                .checked_mul(65536)
                .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64))
                .ok_or_else(|| invalid("point phase native bytes"))?,
        )?;
        native = add(native, surface.default_bytes().len() as u64)?;
    }
    let retained = add(add(model, payload)?, source)?.checked_mul(4).ok_or_else(|| invalid("point retained work"))?;
    add(
        add(native.checked_mul(2).ok_or_else(|| invalid("point native validation work"))?, retained)?,
        add(tiles.checked_mul(512).ok_or_else(|| invalid("point tile metadata work"))?, 8192)?,
    )
}
fn prepay(budget: &photocraft_xcf::Budget<'_>, units: u64) -> Result<(), IoError> {
    budget.check(BudgetKind::Work, add(budget.used(BudgetKind::Work), units)?, "point native phase admission").map_err(accounting_error)?;
    let mut left = units;
    while left != 0 {
        let n = left.min(4096);
        budget.consume_work(n, "point native phase prepaid").map_err(accounting_error)?;
        left -= n;
    }
    Ok(())
}
/// Only a sealed, successfully admitted selected completion enters this path.
/// The selected factory must restrict its actual classified domain to the existing
/// supported Point branches before this consuming custody handoff. This is not
/// a caller-supplied Document or a source/renderer grant from census statistics.
#[derive(Clone,Copy)]
enum PointStorage { Implicit,Assigned,FilteredFlat,MaskedFilteredFlat }
pub(in crate::xcf_import) fn promote_source<'b, 'a, S: PromotionSource<'b, 'a>>(
    source: S,
    budget: &'b photocraft_xcf::Budget<'a>,
    quota: BackingQuota,
    control: &ReadControl<'_>,
    preparation: &mut Option<PreparationHold>,
) -> Result<OwnedPointXcfImport, IoError> {
    promote_with_storage(source,budget,quota,control,preparation,PointStorage::Implicit)
}
pub(in crate::xcf_import) fn promote_assigned_source<'b,'a,S:PromotionSource<'b,'a>>(source:S,budget:&'b photocraft_xcf::Budget<'a>,quota:BackingQuota,control:&ReadControl<'_>,preparation:&mut Option<PreparationHold>)->Result<OwnedPointXcfImport,IoError>{
    promote_with_storage(source,budget,quota,control,preparation,PointStorage::Assigned)
}
pub(in crate::xcf_import) fn promote_filtered_flat_source<'b,'a,S:PromotionSource<'b,'a>>(source:S,budget:&'b photocraft_xcf::Budget<'a>,quota:BackingQuota,control:&ReadControl<'_>,preparation:&mut Option<PreparationHold>)->Result<OwnedPointXcfImport,IoError>{
 promote_with_storage(source,budget,quota,control,preparation,PointStorage::FilteredFlat)
}
pub(in crate::xcf_import) fn promote_masked_filtered_flat_source<'b,'a,S:PromotionSource<'b,'a>>(source:S,budget:&'b photocraft_xcf::Budget<'a>,quota:BackingQuota,control:&ReadControl<'_>,preparation:&mut Option<PreparationHold>)->Result<OwnedPointXcfImport,IoError>{
 promote_with_storage(source,budget,quota,control,preparation,PointStorage::MaskedFilteredFlat)
}
fn promote_with_storage<'b,'a,S:PromotionSource<'b,'a>>(source:S,budget:&'b photocraft_xcf::Budget<'a>,quota:BackingQuota,control:&ReadControl<'_>,preparation:&mut Option<PreparationHold>,storage:PointStorage)->Result<OwnedPointXcfImport,IoError>{
    struct Incoming<'b, 'a, S> {
        source: S,
        model: Option<photocraft_xcf::Reservation<'b, 'a>>,
        payload: Option<photocraft_xcf::Reservation<'b, 'a>>,
        bridge: Option<photocraft_xcf::Reservation<'b, 'a>>,
    }
    let mut incoming = Incoming { source, model: None, payload: None, bridge: None };
    let filtered=matches!(storage,PointStorage::FilteredFlat|PointStorage::MaskedFilteredFlat);
    let measured = if filtered {census::measure_filtered_flat(incoming.source.document(),budget,control.interrupt)?}else{census::measure(incoming.source.document(), budget, control.interrupt)?};
    // The holder's result is exposed only as borrowed fields; this sibling
    // report census does not acquire a public extraction or source grant.
    let reports = incoming.source.report_capacity_with_budget(&budget)?;
    incoming.model = Some(budget.reserve(BudgetKind::Index, add(measured.model, reports)?, "point actual model/report capacities").map_err(accounting_error)?);
    incoming.payload = Some(budget.reserve(BudgetKind::Resident, measured.payload, "point raw/profile capacities").map_err(accounting_error)?);
    let bridge_bytes = add(
        add(std::mem::size_of::<Retention>() as u64, std::mem::size_of::<OwnedPointXcfImport>() as u64)?,
        add((std::mem::size_of::<store::SurfaceStore<Retention>>() as u64).checked_mul(if filtered{measured.layers as u64}else{1}).ok_or_else(||invalid("filtered store headers"))?, 4096)?,
    )?;
    incoming.bridge = Some(budget.reserve(BudgetKind::Index, bridge_bytes, "point owned bridge headers").map_err(accounting_error)?);
    let mut phase = phase_work(incoming.source.document(), measured.model, measured.payload, incoming.source.original_bytes().len() as u64, &budget)?;
    if filtered {
      let doc=incoming.source.document();
      let state=doc.xcf_state.as_ref().ok_or_else(||invalid("filtered phase state"))?;
      let mut n=add(doc.layers.len() as u64,state.items.len() as u64)?;
      n=add(n,state.filter_masks.len() as u64)?;
      n=add(n,state.filter_stacks.len() as u64)?;
      for stack in &state.filter_stacks {n=add(n,stack.nodes.len() as u64)?;}
      phase=add(phase,n.checked_mul(n).and_then(|v|v.checked_mul(64)).ok_or_else(||invalid("filtered phase joins"))?)?;
    }
    if matches!(storage,PointStorage::MaskedFilteredFlat){
        let doc=incoming.source.document();
        let work=|n:u64|budget.consume_work(n.max(1),"masked mixed promotion requirements").map_err(|e|if e.kind==ErrorKind::Cancelled{BackingError::Cancelled}else{BackingError::Work});
        let req=doc.checked_f32_filter_mask_coverage_set_requirements_with_local_work(photocraft_doc::xcf_filter_masks::MaskCoverageSetLimits::STANDARD,control.interrupt,&work).map_err(backing)?;
        phase=add(phase,add(req.validation_work_units,req.proof_index_bytes)?)?;
        let state=doc.xcf_state.as_ref().ok_or_else(||invalid("masked mixed promotion state"))?;
        let masks=state.filter_masks.len()as u64;let stacks=state.filter_stacks.len()as u64;let roots=doc.layers.len()as u64;
        let nodes=state.filter_stacks.iter().try_fold(0u64,|n,s|add(n,s.nodes.len()as u64))?;
        let joins=masks.checked_mul(stacks).and_then(|n|n.checked_mul(roots)).and_then(|n|n.checked_add(masks.checked_mul(nodes)?.checked_mul(4)?)).and_then(|n|n.checked_add(masks.checked_mul(masks)?)).and_then(|n|n.checked_mul(4)).ok_or_else(||invalid("masked mixed promotion associations"))?;
        phase=add(phase,add(joins,add(measured.model,measured.payload)?.checked_mul(4).ok_or_else(||invalid("masked mixed promotion metadata work"))?)?)?;
        for mask in &state.filter_masks{
            budget.consume_work(1,"masked mixed promotion physical requirements").map_err(accounting_error)?;
            let sample=mask.surface.format().bytes_per_pixel()as u64;let tiles=mask.surface.tile_count()as u64;
            let bytes=tiles.checked_mul(65536).and_then(|n|n.checked_mul(sample)).and_then(|n|n.checked_add(mask.surface.default_bytes().len()as u64)).ok_or_else(||invalid("masked mixed promotion physical bytes"))?;
            let samples=bytes.checked_div(sample).ok_or_else(||invalid("masked mixed promotion sample width"))?;
            let units=bytes.checked_mul(2).and_then(|n|n.checked_add(samples.checked_mul(96)?)).and_then(|n|n.checked_add(tiles.checked_mul(16)?)).and_then(|n|n.checked_add(1024)).ok_or_else(||invalid("masked mixed promotion coverage work"))?;
            phase=add(phase,units)?;
        }
    }
    prepay(&budget, phase)?;
    let remaining = AtomicU64::new(phase);
    let work = |units| remaining.try_update(Ordering::AcqRel, Ordering::Relaxed, |left| left.checked_sub(units)).map(|_| ()).map_err(|_| BackingError::Work);
    let poll = || {
        budget
            .poll("point promotion original controls")
            .map_err(|error| if error.kind == ErrorKind::Cancelled { BackingError::Cancelled } else { BackingError::Work })
    };
    let read = ReadControl { interrupt: control.interrupt, work: &work };
    for layer in &incoming.source.document().layers {
        let LayerContent::Raster(surface) = &layer.content else { return Err(invalid("point staged layer")) };
        let bounds = if filtered {incoming.source.document().bounds()}else{census::raster_bounds(incoming.source.document(), layer.id)?};
        store::validate(surface, bounds, &read, &poll).map_err(backing)?;
    }
    if let Some(state) = &incoming.source.document().xcf_state {
        for mask in &state.filter_masks {
            store::validate(&mask.surface, mask.bounds, &read, &poll).map_err(backing)?;
        }
    }
    let owned = incoming.source.detach_owned(
        &mut [
            incoming.model.as_mut().ok_or_else(|| invalid("point model fee missing"))?,
            incoming.payload.as_mut().ok_or_else(|| invalid("point payload fee missing"))?,
            incoming.bridge.as_mut().ok_or_else(|| invalid("point bridge fee missing"))?,
        ],
        &budget,
    )?;
    let mut physical = owned.surface_batch.physical_usage();
    let header = owned.surface_batch.ledger_header();
    if let Some(source) = &owned.source_batch {
        if !header.shares_ledger(source.ledger_header()) {
            return Err(invalid("point receipt ledger mismatch"));
        }
        physical = combined(physical, source.physical_usage())?;
    }
    physical = combined(physical, header.physical_usage())?;
    let hold = preparation.as_mut().ok_or_else(|| invalid("point preparation hold missing"))?;
    hold.retain_physical(physical, measured.layers, &read)?;
    let hold = preparation.take().ok_or_else(|| invalid("point preparation hold moved"))?;
    let baseline = match hold.into_baseline() {
        Ok(baseline) => baseline,
        Err((error, hold)) => {
            *preparation = Some(hold);
            return Err(error);
        }
    };
    let staged_source::OwnedStagedImport { result, encoded, source_batch, surface_batch } = owned;
    let retention = Arc::new(Retention { _encoded: encoded, _source: source_batch, _surfaces: surface_batch, _baseline: baseline, quota: quota.clone() });
    struct Promoting {
        result: ImportResult,
        retention: Arc<Retention>,
    }
    let mut promoting = Promoting { result, retention };
    // Resolve the singleton's original operative rectangle before taking a
    // mutable layer borrow; it may differ from the canvas and cross zero.
    let id = promoting.result.document.layers.first().ok_or_else(|| invalid("point layer missing"))?.id;
    let bounds = if filtered {promoting.result.document.bounds()}else{census::raster_bounds(&promoting.result.document, id)?};
    for layer in &mut promoting.result.document.layers {
        poll().map_err(backing)?;
        read.check(1).map_err(backing)?;
        let placeholder = LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: false, artboard: None });
        let LayerContent::Raster(surface) = std::mem::replace(&mut layer.content, placeholder) else {
            return Err(invalid("point raster transferred twice"));
        };
        let descriptor = NativePlaneDescriptor::new(surface.format(), bounds, surface.default_bytes()).map_err(backing)?;
        let store = Arc::new(store::SurfaceStore::new(surface, descriptor, promoting.retention.clone(), quota.clone()));
        let raster = BackedRaster::new(store, quota.clone(), 1, &read).map_err(backing)?;
        layer.content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &read).map_err(|error| match error {
            photocraft_doc::BackedLayerError::Backing(error) => backing(error),
            _ => backing(BackingError::Integrity),
        })?);
    }
    poll().map_err(backing)?;
    // Keep one already-paid Arc reference declared before all extracted report
    // locals, so adoption/poll failures cannot refund their physical owner early.
    let retention_guard = promoting.retention.clone();
    let Promoting { result, retention } = promoting;
    let ImportResult { document, warnings, origin, assessment, source_read_only, preview_only } = result;
    let document = if matches!(storage,PointStorage::MaskedFilteredFlat){AccountedDocument::new_readonly_masked_filtered_flat_point_storage(document,retention.clone(),quota,&read)}else if filtered {AccountedDocument::new_readonly_filtered_flat_point_storage(document,retention.clone(),quota,&read)} else if matches!(storage,PointStorage::Assigned) { AccountedDocument::new_readonly_assigned_point_storage(document,retention.clone(),quota,&read) } else { AccountedDocument::new_readonly_point_storage(document, retention.clone(), quota, &read) }.map_err(backing)?;
    poll().map_err(backing)?;
    control.interrupt.check().map_err(|_| IoError::Cancelled)?;
    let output = OwnedPointXcfImport { document, warnings, origin, assessment, source_read_only, preview_only, _retention: retention };
    drop(retention_guard);
    Ok(output)
}

#[cfg(test)]
#[path = "owned_point_source/tests.rs"]
mod tests;
