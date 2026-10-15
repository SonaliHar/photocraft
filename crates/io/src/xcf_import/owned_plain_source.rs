//! Opt-in owned handoff for the existing staged plain-XCF class. This is not
//! the default open route and confers no additional source/render qualification.
//! Original decoded buffers move into private native stores; no f32 round trip.
//! Standard prepaid Arc/map allocation boundaries remain, not total RSS control.
use super::*;
use photocraft_doc::{BackedLayer, accounted_document::AccountedDocument};
use photocraft_raster::backing::{
    BackedRaster, BackingError, ReadControl,
    quota::{BackingQuota, QuotaBaseline, QuotaLimits, QuotaUsage},
};
use photocraft_xcf::{BudgetKind, OwnedReservationBatch, PhysicalUsage};
#[path = "owned_plain_source/census.rs"]
pub(in crate::xcf_import) mod census;
#[path = "owned_plain_source/existing_quota.rs"]
pub(in crate::xcf_import) mod existing_quota;
#[path = "owned_plain_source/store.rs"]
pub(in crate::xcf_import) mod store;
pub use existing_quota::import_owned_staged_plain_bytes_in_quota;
#[path = "owned_plain_source/reader.rs"]
mod reader;
pub use reader::import_owned_staged_plain_from_reader_in_quota;
fn invalid(message: &str) -> IoError {
    IoError::Unsupported(message.into())
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| invalid("owned XCF capacity overflow"))
}
fn backing_kind(error: BackingError) -> ErrorKind {
    match error {
        BackingError::Cancelled => ErrorKind::Cancelled,
        BackingError::Quota | BackingError::Allocation | BackingError::Work | BackingError::Length => ErrorKind::Limit,
        BackingError::Unsupported => ErrorKind::Unsupported,
        _ => ErrorKind::Structure,
    }
}
fn backing(error: BackingError) -> IoError {
    accounting_error(XcfError { kind: backing_kind(error), offset: 0, item: String::new(), message: format!("owned XCF backing: {error}") })
}
fn raster_error(error: photocraft_raster::RasterInputError) -> BackingError {
    use photocraft_raster::RasterInputError as E;
    match error {
        E::Cancelled => BackingError::Cancelled,
        E::AllocationFailed => BackingError::Allocation,
        E::LimitExceeded => BackingError::Quota,
        E::InvalidBounds => BackingError::Bounds,
        _ => BackingError::Integrity,
    }
}
// No model or Surface here: stores/model may retain this without an Arc cycle.
// Encoded storage dies before the genuine source receipts and quota baseline.
pub(in crate::xcf_import) struct Retention {
    _encoded: Arc<Vec<u8>>,
    _source: Option<OwnedReservationBatch>,
    _surfaces: OwnedReservationBatch,
    _baseline: QuotaBaseline,
    quota: BackingQuota,
}
/// Owned native model plus the unchanged mapper diagnostics. No owned resident
/// Document/Surface extraction, deep Clone, source grant or success requalification.
pub struct OwnedPlainXcfImport {
    document: AccountedDocument,
    warnings: Vec<String>,
    origin: ImportOrigin,
    assessment: Option<crate::xcf_assessment::ImportAssessment>,
    source_read_only: bool,
    preview_only: bool,
    _retention: Arc<Retention>,
}
impl std::fmt::Debug for OwnedPlainXcfImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedPlainXcfImport").field("document", &self.document).field("origin", &self.origin).finish_non_exhaustive()
    }
}
impl OwnedPlainXcfImport {
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
    /// Move the already charged model; diagnostics drop while its guard remains.
    pub fn into_accounted_document(self) -> AccountedDocument {
        self.document
    }
}
struct Bytes<'a>(&'a [u8]);
impl photocraft_xcf::Source for Bytes<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &photocraft_xcf::Budget<'_>) -> Result<(), XcfError> {
        budget.poll("owned plain source read")?;
        let start = usize::try_from(offset).map_err(|_| stage_error("owned source offset"))?;
        let end = start.checked_add(out.len()).ok_or_else(|| stage_error("owned source span"))?;
        let source = self.0.get(start..end).ok_or_else(|| stage_error("owned source short read"))?;
        // The parser bounds requests; still poll/account the actual copied bytes.
        for (input, output) in source.chunks(4096).zip(out.chunks_mut(4096)) {
            budget.consume_work(input.len() as u64, "owned source input copy")?;
            output.copy_from_slice(input);
        }
        Ok(())
    }
}
fn stage_error(message: &str) -> XcfError {
    XcfError { kind: ErrorKind::Structure, offset: 0, item: String::new(), message: message.into() }
}
fn combined(a: PhysicalUsage, b: PhysicalUsage) -> Result<PhysicalUsage, IoError> {
    Ok(PhysicalUsage {
        resident_bytes: add(a.resident_bytes, b.resident_bytes)?,
        index_bytes: add(a.index_bytes, b.index_bytes)?,
        scratch_bytes: add(a.scratch_bytes, b.scratch_bytes)?,
    })
}
fn layer_bounds(
    state: &photocraft_doc::XcfState,
    id: photocraft_doc::LayerId,
    c: &ReadControl<'_>,
    poll: &dyn Fn() -> Result<(), BackingError>,
) -> Result<Rect, BackingError> {
    let mut found = None;
    for item in &state.items {
        poll()?;
        c.check(1)?;
        if item.item == photocraft_doc::item_identity::ItemRef::Layer(id) {
            if found.is_some() {
                return Err(BackingError::Integrity);
            }
            found = Some(item.raster_bounds.ok_or(BackingError::Integrity)?);
        }
    }
    found.ok_or(BackingError::Integrity)
}
fn native_phase_work(doc: &Document, model: u64, payload: u64, source: u64, budget: &photocraft_xcf::Budget<'_>) -> Result<u64, IoError> {
    let items = doc.xcf_state.as_ref().ok_or_else(|| invalid("owned phase state"))?.items.len() as u64;
    let layers = doc.layers.len() as u64;
    let mut bytes = 0u64;
    let mut tiles = 0u64;
    for layer in &doc.layers {
        budget.consume_work(1, "owned native phase prospective geometry").map_err(accounting_error)?;
        let LayerContent::Raster(surface) = &layer.content else { return Err(invalid("owned phase raster")) };
        let count = surface.tile_count() as u64;
        tiles = add(tiles, count)?;
        let tile_bytes = 65536u64.checked_mul(surface.format().bytes_per_pixel() as u64).ok_or_else(|| invalid("owned phase tile bytes"))?;
        bytes = add(bytes, count.checked_mul(tile_bytes).ok_or_else(|| invalid("owned phase payload"))?)?;
    }
    let identity = layers.checked_mul(items).and_then(|n| n.checked_mul(2)).ok_or_else(|| invalid("owned phase identities"))?;
    let retained = add(add(model, payload)?, source)?.checked_mul(4).ok_or_else(|| invalid("owned phase retained work"))?;
    add(
        add(add(bytes, identity)?, retained)?,
        add(
            layers.checked_mul(128).ok_or_else(|| invalid("owned phase layer work"))?,
            add(tiles.checked_mul(4).ok_or_else(|| invalid("owned phase tile work"))?, 4096)?,
        )?,
    )
}
fn prepay_phase_work(budget: &photocraft_xcf::Budget<'_>, units: u64) -> Result<(), IoError> {
    // Precheck avoids a long debit loop for an already impossible request. The
    // subsequent actual debits, including failed-attempt work, are never refunded.
    budget.check(BudgetKind::Work, add(budget.used(BudgetKind::Work), units)?, "owned native phase work admission").map_err(accounting_error)?;
    let mut left = units;
    while left != 0 {
        let count = left.min(4096);
        budget.consume_work(count, "owned native phase prepaid work").map_err(accounting_error)?;
        left -= count;
    }
    Ok(())
}
/// Explicit owned-memory entry. The supplied Arc is retained, not cloned into a
/// second encoded buffer. The internal persistent import Budget has no external
/// alias; after preparation closes, all new allocation uses the aggregate quota.
/// Caller callbacks are job-local and never retained in the returned owner.
pub fn import_owned_staged_plain_bytes(
    name: &str,
    bytes: Arc<Vec<u8>>,
    options: &DecodeOptions<'_>,
    limits: &photocraft_xcf::ResourceLimits,
    control: &Interrupt<'_>,
) -> Result<OwnedPlainXcfImport, IoError> {
    import_impl(name, bytes, options, limits, control, None)
}
fn import_impl(
    name: &str,
    bytes: Arc<Vec<u8>>,
    options: &DecodeOptions<'_>,
    limits: &photocraft_xcf::ResourceLimits,
    control: &Interrupt<'_>,
    existing: Option<existing_quota::Context<'_, '_>>,
) -> Result<OwnedPlainXcfImport, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    if name.len() > 1 << 20 {
        return Err(invalid("owned import name ceiling"));
    }
    // Decoder, mapper, persistent headers and the eventual backing ledger share
    // the same lowered immutable resident ceiling from their first allocation.
    let aggregate = limits.get(BudgetKind::Resident).min(options.limits.resident_bytes as u64);
    let effective_limits = limits.lowered(&[("residentBytes", aggregate)]).map_err(accounting_error)?;
    let preparation = match &existing {
        Some(context) => Some(existing_quota::PreparationHold::reserve(&context.quota, &effective_limits, context.control)?),
        None => None,
    };
    // Move the input immediately after complete parent admission. On every
    // failure, source bytes drop before their real preparation fees, including
    // failures constructing the private persistent decoder Budget itself.
    struct PreparationInput {
        bytes: Option<Arc<Vec<u8>>>,
        hold: Option<existing_quota::PreparationHold>,
    }
    let mut preparation_input = PreparationInput { bytes: Some(bytes), hold: preparation };
    let cancelled = || control.cancelled() || options.cancelled.is_some_and(|f| f());
    let observer = |units| match &existing {
        Some(context) => context.control.check(units).map_err(|error| XcfError {
            kind: backing_kind(error),
            offset: 0,
            item: String::new(),
            message: format!("owned XCF enclosing work: {error}"),
        }),
        None => Ok(()),
    };
    let budget = match &existing {
        Some(_) => photocraft_xcf::Budget::new_persistent_with_work(effective_limits.clone(), Some(&cancelled), Some(&observer)),
        None => photocraft_xcf::Budget::new_persistent(effective_limits.clone(), Some(&cancelled)),
    }
    .map_err(accounting_error)?;
    // The decoder owns encoded length+resident length receipts. This real extra
    // reservation covers spare input capacity and the immutable Arc/Vec owner.
    let source_bytes = preparation_input.bytes.as_ref().ok_or_else(|| invalid("owned preparation source missing"))?;
    let extra = add(
        source_bytes.capacity().checked_sub(source_bytes.len()).ok_or_else(|| invalid("owned source capacity"))? as u64,
        (std::mem::size_of::<Vec<u8>>() + 128) as u64,
    )?;
    struct Input<'b, 'a> {
        bytes: Arc<Vec<u8>>,
        extra: photocraft_xcf::Reservation<'b, 'a>,
    }
    let name_bytes = (name.len() as u64).checked_mul(4).and_then(|n| n.checked_add(1024)).ok_or_else(|| invalid("owned name envelope"))?;
    let mut name_lease = budget.reserve(BudgetKind::Index, name_bytes, "owned mapped name copies").map_err(accounting_error)?;
    let mut input = Input {
        bytes: preparation_input.bytes.take().ok_or_else(|| invalid("owned preparation source already transferred"))?,
        extra: budget.reserve(BudgetKind::Resident, extra, "owned source spare capacity/header").map_err(accounting_error)?,
    };
    let completed = {
        let mut source = Bytes(input.bytes.as_slice());
        staged_source::complete_staged_plain_source(name, &mut source, control, options, &budget, photocraft_xcf::PlainSourceAccounting::BorrowedMemory)?
    };
    let measured = census::measure(&completed.result.document, &budget, control)?;
    let reports = census::reports(&completed.result, &budget)?;
    // A second explicit index receipt conservatively retains the real owned
    // model/map/report capacities alongside the mapper's original reservations.
    // It is not a fabricated receipt reconstructed from peak/used statistics.
    let index_bytes = add(measured.model, reports)?;
    let mut model_lease = budget.reserve(BudgetKind::Index, index_bytes, "owned exact model/index capacities").map_err(accounting_error)?;
    let mut payload_lease = budget.reserve(BudgetKind::Resident, measured.payload, "owned shared profile/raw capacities").map_err(accounting_error)?;
    let per_store = add(std::mem::size_of::<store::SurfaceStore<Retention>>() as u64, 128)?;
    let bridge_bytes = add(
        add(std::mem::size_of::<Retention>() as u64, std::mem::size_of::<OwnedPlainXcfImport>() as u64)?,
        add((measured.layers as u64).checked_mul(per_store).ok_or_else(|| invalid("owned store headers"))?, 4096)?,
    )?;
    let mut bridge_lease = budget.reserve(BudgetKind::Index, bridge_bytes, "owned native bridge headers").map_err(accounting_error)?;
    // One checked, nonrefundable phase allowance is spent on the actual import
    // Budget before a Sync ReadControl exists. It covers physical row/padding
    // bytes, BOTH layer/item lookup passes, bounded owner/model/header checks,
    // and a conservative four traversals of all measured model/raw/source bytes.
    // Original budget callbacks/deadline still run serially in each row/ID pass.
    let state = completed.result.document.xcf_state.as_ref().ok_or_else(|| invalid("owned source missing state"))?;
    let phase_work = native_phase_work(&completed.result.document, measured.model, measured.payload, input.bytes.len() as u64, &budget)?;
    prepay_phase_work(&budget, phase_work)?;
    let remaining = std::sync::atomic::AtomicU64::new(phase_work);
    let work = |units: u64| -> Result<(), BackingError> {
        remaining
            .try_update(std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Relaxed, |left| left.checked_sub(units))
            .map(|_| ())
            .map_err(|_| BackingError::Work)
    };
    let phase_poll = || {
        budget
            .poll("owned native handoff original controls")
            .map_err(|e| if e.kind == ErrorKind::Cancelled { BackingError::Cancelled } else { BackingError::Work })
    };
    let read = ReadControl { interrupt: control, work: &work };
    for layer in &completed.result.document.layers {
        let LayerContent::Raster(surface) = &layer.content else { return Err(invalid("owned source requires staged raster")) };
        store::validate(surface, layer_bounds(state, layer.id, &read, &phase_poll).map_err(backing)?, &read, &phase_poll).map_err(backing)?;
    }
    budget.poll("owned source preparation closed").map_err(accounting_error)?;
    let Input { bytes, ref mut extra } = input;
    let owned = completed.detach_owned(bytes, &mut [extra, &mut model_lease, &mut payload_lease, &mut bridge_lease, &mut name_lease], &budget)?;
    // No more import reservation/allocation phases beyond this point. Work polls
    // may still use the job-local budget while every allocation uses this quota.
    let mut physical = owned.surface_batch.physical_usage();
    let header = owned.surface_batch.ledger_header();
    if let Some(source) = &owned.source_batch {
        if !header.shares_ledger(source.ledger_header()) {
            return Err(invalid("owned source receipt ledger mismatch"));
        }
        physical = combined(physical, source.physical_usage())?;
    }
    physical = combined(physical, header.physical_usage())?;
    let (quota, baseline) = if let Some(context) = &existing {
        let hold = preparation_input.hold.as_mut().ok_or_else(|| invalid("owned preparation fees missing"))?;
        hold.retain_physical(physical, measured.layers, context.control)?;
        let hold = preparation_input.hold.take().ok_or_else(|| invalid("owned preparation fees already transferred"))?;
        let baseline = match hold.into_baseline() {
            Ok(baseline) => baseline,
            Err((error, hold)) => {
                // Restore every actual fee before this function drops the later
                // declared owned result/model/native surfaces on its error path.
                preparation_input.hold = Some(hold);
                return Err(error);
            }
        };
        (context.quota.clone(), baseline)
    } else {
        let quota_limits = QuotaLimits { resident: aggregate, scratch: limits.get(BudgetKind::Scratch), index: limits.get(BudgetKind::Index) };
        let (quota, baseline) = BackingQuota::new_with_aggregate_initial(
            quota_limits,
            aggregate,
            QuotaUsage { resident: physical.resident_bytes, index: physical.index_bytes, scratch: physical.scratch_bytes },
        )
        .map_err(|_| backing(BackingError::Quota))?;
        (quota, baseline)
    };
    let staged_source::OwnedStagedImport { result, encoded, source_batch, surface_batch } = owned;
    let retention = Arc::new(Retention { _encoded: encoded, _source: source_batch, _surfaces: surface_batch, _baseline: baseline, quota: quota.clone() });
    struct Promoting {
        result: ImportResult,
        retention: Arc<Retention>,
    }
    let mut staged = Promoting { result, retention };
    let state = staged.result.document.xcf_state.as_ref().ok_or_else(|| invalid("owned source state lost"))?;
    for layer in &mut staged.result.document.layers {
        phase_poll().map_err(backing)?;
        read.check(1).map_err(backing)?;
        let bounds = layer_bounds(state, layer.id, &read, &phase_poll).map_err(backing)?;
        let placeholder = LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: false, artboard: None });
        let LayerContent::Raster(surface) = std::mem::replace(&mut layer.content, placeholder) else { return Err(invalid("owned raster transferred twice")) };
        let descriptor = photocraft_raster::backing::NativePlaneDescriptor::new(surface.format(), bounds, surface.default_bytes()).map_err(backing)?;
        let store = Arc::new(store::SurfaceStore::new(surface, descriptor, staged.retention.clone(), quota.clone()));
        let backed = BackedRaster::new(store, quota.clone(), 1, &read).map_err(backing)?;
        layer.content = LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &read).map_err(|e| match e {
            photocraft_doc::BackedLayerError::Backing(e) => backing(e),
            _ => backing(BackingError::Integrity),
        })?);
        phase_poll().map_err(backing)?;
    }
    read.check(1).map_err(backing)?;
    let Promoting { result, retention } = staged;
    let ImportResult { document, warnings, origin, assessment, source_read_only, preview_only } = result;
    phase_poll().map_err(backing)?;
    let document = AccountedDocument::from_owned(document, retention.clone(), quota, &read).map_err(backing)?;
    phase_poll().map_err(backing)?;
    control.check().map_err(|_| IoError::Cancelled)?;
    Ok(OwnedPlainXcfImport { document, warnings, origin, assessment, source_read_only, preview_only, _retention: retention })
}

#[cfg(test)]
#[path = "owned_plain_source/tests.rs"]
mod tests;
