//! Explicit actual-tree promotion; group projections are never retained as authority.
//! Runtime storage adoption is read-only: no source, renderer, edit or default
//! loader authority is inferred from the persisted node or receipt sizes.
use super::owned_plain_source::census;
use super::owned_plain_source::{existing_quota::PreparationHold, store};
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
    a.checked_add(b).ok_or_else(|| invalid("owned hierarchy capacity overflow"))
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
    accounting_error(XcfError { kind: kind(error), offset: 0, item: String::new(), message: format!("owned hierarchy: {error}") })
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
pub struct OwnedNormalHierarchyXcfImport {
    document: AccountedDocument,
    warnings: Vec<String>,
    origin: ImportOrigin,
    assessment: Option<crate::xcf_assessment::ImportAssessment>,
    source_read_only: bool,
    preview_only: bool,
    _retention: Arc<Retention>,
}
impl std::fmt::Debug for OwnedNormalHierarchyXcfImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedNormalHierarchyXcfImport").field("document_id", &self.document.document().id).finish_non_exhaustive()
    }
}
impl OwnedNormalHierarchyXcfImport {
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
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("hierarchy phase state"))?;
    let mut native = 0u64;
    let mut tiles = 0u64;
    // Real padded native tiles and defaults, including the retained resident mask.
    // No logical 8x6 estimate substitutes for those physical validation visits.
    let mut sources = staged_source::staged_normal_hierarchy::layers(doc);
    let mut masks = state.filter_masks.iter();
    loop {
        let surface = loop {
            match sources.next() {
                Some(layer) => {
                    budget.consume_work(1, "hierarchy physical-pass actual layer").map_err(accounting_error)?;
                    if let LayerContent::Raster(s) = &layer?.content {
                        break Some(s);
                    }
                }
                None => break masks.next().map(|m| &m.surface),
            }
        };
        let Some(surface) = surface else { break };
        budget.consume_work(1, "hierarchy phase physical census").map_err(accounting_error)?;
        let count = surface.tile_count() as u64;
        tiles = add(tiles, count)?;
        native = add(
            native,
            count
                .checked_mul(65536)
                .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel() as u64))
                .ok_or_else(|| invalid("hierarchy phase native bytes"))?,
        )?;
        native = add(native, surface.default_bytes().len() as u64)?;
    }
    let retained = add(add(model, payload)?, source)?.checked_mul(4).ok_or_else(|| invalid("hierarchy retained work"))?;
    add(
        add(native.checked_mul(2).ok_or_else(|| invalid("hierarchy native validation work"))?, retained)?,
        add(tiles.checked_mul(512).ok_or_else(|| invalid("hierarchy tile metadata work"))?, 8192)?,
    )
}
fn prepay(budget: &photocraft_xcf::Budget<'_>, units: u64) -> Result<(), IoError> {
    budget.check(BudgetKind::Work, add(budget.used(BudgetKind::Work), units)?, "hierarchy native phase admission").map_err(accounting_error)?;
    let mut left = units;
    while left != 0 {
        let n = left.min(4096);
        budget.consume_work(n, "hierarchy native phase prepaid").map_err(accounting_error)?;
        left -= n;
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum SourcePurpose {
    ByteZero,
    Masked,
}
/// Only the sealed actual hierarchy Source completion reaches this handoff.
pub(in crate::xcf_import) fn promote_source<'b, 'a, S: PromotionSource<'b, 'a>>(
    source: S,
    budget: &'b photocraft_xcf::Budget<'a>,
    quota: BackingQuota,
    control: &ReadControl<'_>,
    preparation: &mut Option<PreparationHold>,
) -> Result<OwnedNormalHierarchyXcfImport, IoError> {
    promote_with_purpose(source, budget, quota, control, preparation, SourcePurpose::ByteZero)
}
pub(in crate::xcf_import) fn promote_masked_source<'b, 'a, S: PromotionSource<'b, 'a>>(
    source: S,
    budget: &'b photocraft_xcf::Budget<'a>,
    quota: BackingQuota,
    control: &ReadControl<'_>,
    preparation: &mut Option<PreparationHold>,
) -> Result<OwnedNormalHierarchyXcfImport, IoError> {
    promote_with_purpose(source, budget, quota, control, preparation, SourcePurpose::Masked)
}
fn promote_with_purpose<'b, 'a, S: PromotionSource<'b, 'a>>(
    source: S,
    budget: &'b photocraft_xcf::Budget<'a>,
    quota: BackingQuota,
    control: &ReadControl<'_>,
    preparation: &mut Option<PreparationHold>,
    purpose: SourcePurpose,
) -> Result<OwnedNormalHierarchyXcfImport, IoError> {
    let masked = matches!(purpose, SourcePurpose::Masked);
    struct Incoming<'b, 'a, S> {
        source: S,
        model: Option<photocraft_xcf::Reservation<'b, 'a>>,
        payload: Option<photocraft_xcf::Reservation<'b, 'a>>,
        bridge: Option<photocraft_xcf::Reservation<'b, 'a>>,
    }
    let mut incoming = Incoming { source, model: None, payload: None, bridge: None };
    let _planner = budget.reserve(BudgetKind::Index, 8192, "hierarchy promotion cursor scratch").map_err(accounting_error)?;
    let measured = census::measure_normal_hierarchy(incoming.source.document(), budget, control.interrupt)?;
    // The holder's result is exposed only as borrowed fields; this sibling
    // report census does not acquire a public extraction or source grant.
    let reports = incoming.source.report_capacity_with_budget(&budget)?;
    incoming.model =
        Some(budget.reserve(BudgetKind::Index, add(measured.model, reports)?, "hierarchy actual model/report capacities").map_err(accounting_error)?);
    incoming.payload = Some(budget.reserve(BudgetKind::Resident, measured.payload, "hierarchy raw/profile capacities").map_err(accounting_error)?);
    let bridge_bytes = add(
        add(std::mem::size_of::<Retention>() as u64, std::mem::size_of::<OwnedNormalHierarchyXcfImport>() as u64)?,
        add(
            (std::mem::size_of::<store::SurfaceStore<Retention>>() as u64)
                .checked_mul(measured.layers as u64)
                .ok_or_else(|| invalid("filtered store headers"))?,
            16384,
        )?,
    )?;
    incoming.bridge = Some(budget.reserve(BudgetKind::Index, bridge_bytes, "hierarchy owned bridge headers").map_err(accounting_error)?);
    let mut phase = phase_work(incoming.source.document(), measured.model, measured.payload, incoming.source.original_bytes().len() as u64, &budget)?;
    let doc = incoming.source.document();
    let planning = |n: u64| {
        budget
            .consume_work(n.max(1), "hierarchy promotion requirements")
            .map_err(|e| if e.kind == ErrorKind::Cancelled { BackingError::Cancelled } else { BackingError::Work })
    };
    let coverage_requirements = if masked {
        Some(
            doc.checked_normal_hierarchy_mask_index_requirements(
                photocraft_doc::normal_hierarchy_index::NormalHierarchyLimits::STANDARD,
                control.interrupt,
                &planning,
            )
            .map_err(backing)?,
        )
    } else {
        None
    };
    let req = if let Some(ref combined) = coverage_requirements {
        combined.tree
    } else {
        doc.checked_normal_hierarchy_index_requirements(photocraft_doc::normal_hierarchy_index::NormalHierarchyLimits::STANDARD, control.interrupt, &planning)
            .map_err(backing)?
    };
    let n = add(add(req.layer_count as u64, req.node_count as u64)?, add(req.mask_count as u64, req.stack_count as u64)?)?;
    let joins = n.checked_mul(n).and_then(|v| v.checked_mul(256)).ok_or_else(|| invalid("hierarchy promotion joins"))?;
    phase = add(phase, add(joins, add(req.validation_work_units, add(req.index_bytes, req.validation_index_bytes)?)?)?)?;
    phase = add(phase, req.aggregate_mask_physical_bytes.checked_mul(4).ok_or_else(|| invalid("hierarchy mask work"))?)?;
    if let Some(ref combined) = coverage_requirements {
        // This final SOURCE factory constructs and scans its OWN combined token.
        // Earlier mapper/preparation scans are consumed work, never credits.
        let masks = req.mask_count as u64;
        let stacks = req.stack_count as u64;
        let layers = req.layer_count as u64;
        let nodes = req.node_count as u64;
        let associations = masks
            .checked_mul(stacks)
            .and_then(|n| n.checked_mul(layers))
            .and_then(|n| n.checked_add(masks.checked_mul(nodes)?))
            .and_then(|n| n.checked_add(masks.checked_mul(masks)?))
            .and_then(|n| n.checked_add(nodes.checked_mul(layers)?))
            .and_then(|n| n.checked_mul(512))
            .ok_or_else(|| invalid("hierarchy coverage associations"))?;
        // Exact repeated physical scan: 96/sample plus actual bytes/defaults,
        // tile polls and proof headers, obtained from this immutable Doc.
        let scans = combined.coverage_work_units;
        phase = add(phase, add(scans, add(associations, add(combined.validation_work_units, combined.proof_index_bytes)?)?)?)?;
    }
    prepay(&budget, phase)?;
    let remaining = AtomicU64::new(phase);
    let work = |units| {
        // Zero-unit enclosing checks preserve its dynamic controls without
        // charging the already-prepaid positive units a second time.
        control.check(0)?;
        remaining.try_update(Ordering::AcqRel, Ordering::Relaxed, |left| left.checked_sub(units)).map(|_| ()).map_err(|_| BackingError::Work)
    };
    let poll = || {
        budget
            .poll("hierarchy promotion original controls")
            .map_err(|error| if error.kind == ErrorKind::Cancelled { BackingError::Cancelled } else { BackingError::Work })
    };
    let read = ReadControl { interrupt: control.interrupt, work: &work };
    for layer in staged_source::staged_normal_hierarchy::layers(incoming.source.document()) {
        poll().map_err(backing)?;
        read.check(1).map_err(backing)?;
        let layer = layer?;
        if let LayerContent::Raster(surface) = &layer.content {
            store::validate(surface, incoming.source.document().bounds(), &read, &poll).map_err(backing)?;
        }
    }
    if let Some(state) = &incoming.source.document().xcf_state {
        for mask in &state.filter_masks {
            store::validate(&mask.surface, mask.bounds, &read, &poll).map_err(backing)?;
        }
    }
    let owned = incoming.source.detach_owned(
        &mut [
            incoming.model.as_mut().ok_or_else(|| invalid("hierarchy model fee missing"))?,
            incoming.payload.as_mut().ok_or_else(|| invalid("hierarchy payload fee missing"))?,
            incoming.bridge.as_mut().ok_or_else(|| invalid("hierarchy bridge fee missing"))?,
        ],
        &budget,
    )?;
    let mut physical = owned.surface_batch.physical_usage();
    let header = owned.surface_batch.ledger_header();
    if let Some(source) = &owned.source_batch {
        if !header.shares_ledger(source.ledger_header()) {
            return Err(invalid("hierarchy receipt ledger mismatch"));
        }
        physical = combined(physical, source.physical_usage())?;
    }
    physical = combined(physical, header.physical_usage())?;
    let hold = preparation.as_mut().ok_or_else(|| invalid("hierarchy preparation hold missing"))?;
    hold.retain_physical(physical, measured.layers, &read)?;
    let hold = preparation.take().ok_or_else(|| invalid("hierarchy preparation hold moved"))?;
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
    let bounds = promoting.result.document.bounds();
    transfer(&mut promoting.result.document.layers, bounds, &promoting.retention, &quota, &read, &poll)?;
    poll().map_err(backing)?;
    // Keep one already-paid Arc reference declared before all extracted report
    // locals, so adoption/poll failures cannot refund their physical owner early.
    let retention_guard = promoting.retention.clone();
    let Promoting { result, retention } = promoting;
    let ImportResult { document, warnings, origin, assessment, source_read_only, preview_only } = result;
    // This synchronous local callback can retain the original non-Sync job
    // Budget authority (including options.cancelled); no callback escapes.
    let local_work = |units| {
        poll()?;
        work(units)
    };
    let document = if masked {
        AccountedDocument::new_readonly_masked_normal_hierarchy_storage_with_local_work(document, retention.clone(), quota, &read, &local_work)
    } else {
        AccountedDocument::new_readonly_normal_hierarchy_storage_with_local_work(document, retention.clone(), quota, &read, &local_work)
    }
    .map_err(backing)?;
    poll().map_err(backing)?;
    control.interrupt.check().map_err(|_| IoError::Cancelled)?;
    let output = OwnedNormalHierarchyXcfImport { document, warnings, origin, assessment, source_read_only, preview_only, _retention: retention };
    drop(retention_guard);
    Ok(output)
}

/// Bounded safe mutable traversal, preserving actual Group children and order.
fn transfer(
    roots: &mut [Layer],
    bounds: Rect,
    retention: &Arc<Retention>,
    quota: &BackingQuota,
    read: &ReadControl<'_>,
    poll: &dyn Fn() -> Result<(), BackingError>,
) -> Result<(), IoError> {
    let mut frames: [Option<std::slice::IterMut<'_, Layer>>; 33] = std::array::from_fn(|_| None);
    if let Some(slot) = frames.get_mut(0) {
        *slot = Some(roots.iter_mut());
    }
    let mut depth = 0usize;
    loop {
        poll().map_err(backing)?;
        read.check(1).map_err(backing)?;
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        };
        match &mut layer.content {
            LayerContent::Group(g) => {
                if depth >= 32 {
                    return Err(invalid("hierarchy transfer group frames"));
                }
                if !g.children.is_empty() {
                    depth = depth.checked_add(1).filter(|d| *d <= 32).ok_or_else(|| invalid("hierarchy transfer depth"))?;
                    let slot = frames.get_mut(depth).ok_or_else(|| invalid("hierarchy transfer frame"))?;
                    *slot = Some(g.children.iter_mut());
                }
            }
            content @ LayerContent::Raster(_) => {
                let placeholder = LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: false, artboard: None });
                let LayerContent::Raster(surface) = std::mem::replace(content, placeholder) else { return Err(invalid("hierarchy native moved twice")) };
                let descriptor = NativePlaneDescriptor::new(surface.format(), bounds, surface.default_bytes()).map_err(backing)?;
                let store = Arc::new(store::SurfaceStore::new(surface, descriptor, retention.clone(), quota.clone()));
                let raster = BackedRaster::new(store, quota.clone(), 1, read).map_err(backing)?;
                *content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, read).map_err(|e| match e {
                    photocraft_doc::BackedLayerError::Backing(e) => backing(e),
                    _ => backing(BackingError::Integrity),
                })?);
            }
            _ => return Err(invalid("hierarchy unexpected native transfer content")),
        }
    }
    poll().map_err(backing)?;
    Ok(())
}
