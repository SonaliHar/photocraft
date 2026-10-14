//! Explicit selected-layer original raw-storage preparation only.
//!
//! The independent IO owner, frozen current document and native layer/filter-mask
//! words remain separate. No selection coverage, mask multiplication, Gaussian,
//! working-domain, renderer, source qualification, or publication authority is
//! supplied. OriginalExact is the default. Explicit materialized Gray ICC
//! policy uses its own controlled proof, never absence or the RGB channel policy.
//!
//! One conservative lower resident/work lease covers full retained original and
//! current census envelopes, runtime view and identity-COW map copies. This is
//! admission, not actual RSS/private allocator measurement. The genuine current
//! census measures its documented public model/payload scopes; unexposed legacy
//! internals still require enclosing caller admission. Native identity-COW
//! internals remain the bounded raster provider allocation boundary.
use crate::{DocumentSnapshot, EngineError, Result, Session, WeakDocumentSnapshot, channel_cmds::ChannelView};
use photocraft_compose::{gimp_composite::CompositionError, gimp_render::PreparationBudget};
use photocraft_doc::{
    LayerId,
    item_identity::{FilterId, MaskId},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            resident_bytes: if cfg!(target_arch = "wasm32") { 64 << 20 } else { 256 << 20 },
            index_bytes: if cfg!(target_arch = "wasm32") { 16 << 20 } else { 64 << 20 },
            work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 },
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    /// Full conservative lease, not measured live allocator or RSS bytes.
    pub admitted_resident_bytes: u64,
    pub admitted_work_units: u64,
    pub original_owner_bytes: u64,
    pub original_owner_index_bytes: u64,
    pub current_raster_bytes: u64,
    pub current_model_bytes: u64,
    pub current_payload_bytes: u64,
    pub current_view_bytes: u64,
    pub native_copy_bytes: u64,
    /// Final genuine conservative held envelope, including unreleased scratch.
    pub held_resident_bytes: u64,
    pub held_index_bytes: u64,
    pub index_envelope_bytes: u64,
    pub prepaid_verification_work: u64,
    pub verification_work: u64,
    pub selection_work: u64,
    pub preparation_work: u64,
}
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "xcf.layer.stored.prepare".into(), msg: message.into() }
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| bad("raw layer accounting overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| bad("raw layer accounting overflow"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
struct Work {
    used: AtomicU64,
    exceeded: AtomicBool,
    limit: u64,
}
impl Work {
    fn charge(&self, units: u64) -> Result<()> {
        self.used.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(units).filter(|n| *n <= self.limit)).map_err(|_| {
            self.exceeded.store(true, Ordering::Relaxed);
            bad("raw layer preparation work limit")
        })?;
        Ok(())
    }
    fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used.load(Ordering::Relaxed))
    }
}
/// The verifier owns prepaid work. Its callbacks consume only this receipt;
/// metadata/poll work keeps using Work. No failures, drops or surplus refund it.
struct Prepaid {
    remaining: u64,
}
impl Prepaid {
    fn reserve(work: &Work, units: u64) -> Result<Self> {
        work.charge(units)?;
        Ok(Self { remaining: units })
    }
    fn take_exact(&mut self, units: u64) -> Result<Self> {
        self.remaining = self.remaining.checked_sub(units).ok_or_else(|| bad("layer proof exceeds prepaid upper"))?;
        Ok(Self { remaining: units })
    }
    fn consume(&mut self, units: u64) -> std::result::Result<(), photocraft_io::IoError> {
        self.remaining = self.remaining.checked_sub(units).ok_or_else(|| photocraft_io::IoError::Unsupported("layer proof prepaid work exhausted".into()))?;
        Ok(())
    }
    fn finish(&self) -> Result<()> {
        if self.remaining == 0 { Ok(()) } else { Err(bad("layer verifier did not consume exact prepaid receipt")) }
    }
}
fn idle(session: &Session) -> Result<()> {
    if session.documents().len() > 4096 {
        return Err(bad("document count limit"));
    }
    let current = session.active().ok_or(EngineError::NoDocument)?;
    if session.has_jobs()
        || session.command_has_pending_job()
        || session.command_is_background()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || current.floating.is_some()
        || current.coalesce.is_some()
        || current.revision == u64::MAX
        || current.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(bad("finish pending work before raw layer preparation"));
    }
    Ok(())
}
fn runtime_bytes(view: &ChannelView, selected: &[LayerId], capacity: bool) -> Result<u64> {
    if view.color_hidden.len() > 16 || view.alpha_visible.len() > 10_000 || selected.len() > 10_000 {
        return Err(bad("runtime view count limit"));
    }
    add(
        std::mem::size_of::<ChannelView>() as u64,
        add(
            add(
                if capacity { view.color_hidden.capacity() } else { view.color_hidden.len() } as u64,
                if capacity { view.alpha_visible.capacity() } else { view.alpha_visible.len() } as u64,
            )?,
            mul(selected.len() as u64, std::mem::size_of::<LayerId>() as u64)?,
        )?,
    )
}
fn copy_runtime(view: &ChannelView, selected: &[LayerId], maximum: u64, control: &Interrupt<'_>) -> Result<(ChannelView, Vec<LayerId>, u64)> {
    let mut colors = Vec::new();
    let mut alphas = Vec::new();
    let mut layers = Vec::new();
    colors.try_reserve_exact(view.color_hidden.len()).map_err(|_| bad("view allocation"))?;
    alphas.try_reserve_exact(view.alpha_visible.len()).map_err(|_| bad("view allocation"))?;
    layers.try_reserve_exact(selected.len()).map_err(|_| bad("selected layer allocation"))?;
    let bytes = add(
        std::mem::size_of::<ChannelView>() as u64,
        add(add(colors.capacity() as u64, alphas.capacity() as u64)?, mul(layers.capacity() as u64, std::mem::size_of::<LayerId>() as u64)?)?,
    )?;
    if bytes > maximum {
        return Err(bad("actual runtime capacities exceed remaining envelope"));
    }
    for v in &view.color_hidden {
        poll(control)?;
        colors.push(*v);
    }
    for v in &view.alpha_visible {
        poll(control)?;
        alphas.push(*v);
    }
    for v in selected {
        poll(control)?;
        layers.push(*v);
    }
    Ok((
        ChannelView {
            target: view.target,
            color_hidden: colors,
            alpha_visible: alphas,
            quick_mask_hidden: view.quick_mask_hidden,
            layer_mask: view.layer_mask,
            editing_mask: view.editing_mask,
        },
        layers,
        bytes,
    ))
}
fn inspector_limits(limits: Limits, held: u64, work: &Work, reserved_index: u64) -> Result<photocraft_format::PreparationLimits> {
    let available = limits.resident_bytes.checked_sub(held).ok_or_else(|| bad("raw layer resident envelope exhausted"))?.min(work.remaining());
    let available = usize::try_from(available).map_err(|_| bad("inspector address space"))?;
    let index = usize::try_from(limits.index_bytes.checked_sub(reserved_index).ok_or_else(|| bad("raw layer index envelope exhausted"))?)
        .map_err(|_| bad("inspector index address space"))?;
    let max = photocraft_io::NativeSaveLimits::default().preparation;
    Ok(photocraft_format::PreparationLimits {
        resident_bytes: available,
        index_bytes: index.min(available).min(max.index_bytes),
        manifest_bytes: available.min(max.manifest_bytes),
        object_bytes: available.min(max.object_bytes),
    })
}
fn io_error(error: photocraft_io::IoError, control: &Interrupt<'_>) -> EngineError {
    if control.cancelled() || matches!(&error, photocraft_io::IoError::Cancelled) { EngineError::Cancelled } else { EngineError::Other(error.to_string()) }
}
fn composition(error: CompositionError) -> EngineError {
    if matches!(error, CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled)) {
        EngineError::Cancelled
    } else {
        EngineError::Other(error.to_string())
    }
}

struct Origin {
    snapshot: DocumentSnapshot,
    weak: WeakDocumentSnapshot,
    revision: u64,
    active: Option<LayerId>,
    selected: Vec<LayerId>,
    view: ChannelView,
}
impl Origin {
    fn validate(&self, session: &Session, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        idle(session)?;
        let current = session.active().ok_or(EngineError::NoDocument)?;
        runtime_bytes(&current.channel_view, &current.selected_layers, false)?;
        if current.doc.id != self.snapshot.id
            || current.revision != self.revision
            || !self.weak.is_current(&current.doc)
            || !current.doc.shares_state(&self.snapshot)
            || current.active_layer != self.active
            || current.selected_layers != self.selected
            || current.channel_view != self.view
        {
            return Err(bad("raw layer preparation target changed"));
        }
        poll(control)
    }
}

/// The owner borrow keeps the entire independently decoded original backing and
/// reservations alive. Readonly surfaces are identity-COW copies of the proved
/// current native layer and filter mask, never coverage or evaluated appearance.
pub struct PreparedStoredLayer<'o, 's, 'b, 'a, L> {
    original: &'o photocraft_io::AssembledLayerSelection<'s, 'b, 'a>,
    origin: Origin,
    layer: LayerId,
    filter: FilterId,
    mask: MaskId,
    source_surface: Surface,
    source_bounds: Rect,
    mask_surface: Surface,
    mask_bounds: Rect,
    requirements: Requirements,
    work: Work,
    _lease: L,
}
impl<L> PreparedStoredLayer<'_, '_, '_, '_, L> {
    pub fn current_snapshot(&self) -> &DocumentSnapshot {
        &self.origin.snapshot
    }
    pub fn layer_id(&self) -> LayerId {
        self.layer
    }
    pub fn filter_id(&self) -> FilterId {
        self.filter
    }
    pub fn mask_id(&self) -> MaskId {
        self.mask
    }
    pub fn source_surface(&self) -> &Surface {
        &self.source_surface
    }
    pub fn source_bounds(&self) -> Rect {
        self.source_bounds
    }
    /// Original raw filter-mask storage, not layer-mask or selection coverage.
    pub fn filter_mask_surface(&self) -> &Surface {
        &self.mask_surface
    }
    pub fn filter_mask_bounds(&self) -> Rect {
        self.mask_bounds
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.original.original_bytes()
    }
    pub fn source_sha256(&self) -> &str {
        self.original.source_sha256()
    }
    pub fn requirements(&self) -> Requirements {
        let mut requirements = self.requirements;
        requirements.preparation_work = self.work.used.load(Ordering::Relaxed);
        requirements
    }
    /// A debit only: it cannot assert retained capacities, credit work, or create
    /// a fresh budget. The sibling numerical adapter computes its bounded field
    /// validation cost before comparisons under this same retained Work.
    pub(crate) fn charge_metadata_work(&self, bytes: u64, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        self.work.charge(bytes)
    }
    pub(crate) fn original_node(&self) -> Result<&photocraft_doc::xcf_filters::FilterNode> {
        self.original.original_node().map_err(|error| EngineError::Other(error.to_string()))
    }
    pub fn remaining_work_units(&self) -> u64 {
        self.work.remaining()
    }
    /// Immutable current-state fence only. The original opaque IO proof checked
    /// native correspondence once; no full scene validator/qualification runs.
    pub fn validate(&self, session: &Session, control: &Interrupt<'_>) -> Result<()> {
        self.work.charge(add(self.requirements.current_view_bytes, 1)?)?;
        let work = &self.work;
        let cancelled = || control.cancelled() || work.charge(1).is_err();
        let progress = |fraction| control.progress(fraction);
        let metered = Interrupt::new(&cancelled, &progress);
        let result = self.origin.validate(session, &metered);
        if self.work.exceeded.load(Ordering::Relaxed) {
            return Err(bad("raw layer preparation work limit"));
        }
        result
    }
}
// This budget cannot accept caller-asserted scene/census totals. Only the
// actual prepared reader's held ledger and concrete kernel requirements create it.
struct StageBudget<'a> {
    held: &'a mut u64,
    limit: u64,
    peak: u64,
    work: &'a mut Prepaid,
}
impl PreparationBudget for StageBudget<'_> {
    type Lease = ();
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> std::result::Result<(), CompositionError> {
        if bytes > self.peak {
            return Err(CompositionError::Invalid("layer Gaussian prospective peak"));
        }
        *self.held =
            self.held.checked_add(bytes).filter(|held| *held <= self.limit).ok_or(CompositionError::Invalid("layer Gaussian shared resident envelope"))?;
        Ok(())
    }
    fn consume_preparation_work(&mut self, units: u64) -> std::result::Result<(), CompositionError> {
        self.work.consume(units).map_err(|_| CompositionError::Invalid("layer Gaussian prepaid work"))
    }
}
impl<'o, 's, 'b, 'a, L> PreparedStoredLayer<'o, 's, 'b, 'a, L> {
    /// Concrete same-ledger transaction: no numeric scene totals, fresh external
    /// quota, or arbitrary unmetered callback can enter through this seam.
    pub(crate) fn into_gaussian_stage(
        mut self,
        session: &Session,
        config: photocraft_compose::gimp_layer_selection_gaussian_candidate::Config,
        control: &Interrupt<'_>,
    ) -> Result<(Self, photocraft_compose::gimp_layer_selection_gaussian_candidate::StagedOutput<()>)> {
        use photocraft_compose::gimp_layer_selection_gaussian_candidate as kernel;
        self.validate(session, control)?;
        // Numerical wrapper has exactly this tuple as its sole field. Its actual
        // inline delta is known here, including L's layout; no asserted header.
        let header = (std::mem::size_of::<(Self, kernel::StagedOutput<()>)>() as u64)
            .checked_sub(std::mem::size_of::<Self>() as u64)
            .ok_or_else(|| bad("stage header underflow"))?;
        let staged = (|| {
            let work = &self.work;
            let cancelled = || control.cancelled() || work.charge(1).is_err();
            let progress = |fraction| control.progress(fraction);
            let metered = Interrupt::new(&cancelled, &progress);
            let held = add(self.requirements.held_resident_bytes, header)?;
            let remaining = photocraft_compose::gimp_channel_gaussian_candidate::Limits {
                resident_bytes: self.requirements.admitted_resident_bytes.checked_sub(held).ok_or_else(|| bad("stage remaining resident envelope"))?,
                index_bytes: self
                    .requirements
                    .index_envelope_bytes
                    .checked_sub(self.requirements.held_index_bytes)
                    .ok_or_else(|| bad("stage remaining index envelope"))?,
                work_units: work.remaining(),
            };
            let input = kernel::Inputs { source: &self.source_surface, mask: &self.mask_surface, bounds: self.source_bounds, mask_bounds: self.mask_bounds };
            let required = kernel::requirements(input, config, remaining, &metered).map_err(composition)?;
            if required.work_units > work.remaining() {
                return Err(bad("stage remaining work envelope"));
            }
            let index = add(self.requirements.held_index_bytes, required.index_bytes)?;
            if add(held, required.peak_resident_bytes)? > self.requirements.admitted_resident_bytes || index > self.requirements.index_envelope_bytes {
                return Err(bad("combined raw and Gaussian stage envelope"));
            }
            let mut prepaid = Prepaid::reserve(work, required.work_units)?;
            self.requirements.held_resident_bytes = held;
            self.requirements.held_index_bytes = index;
            let mut local = StageBudget {
                held: &mut self.requirements.held_resident_bytes,
                limit: self.requirements.admitted_resident_bytes,
                peak: required.peak_resident_bytes,
                work: &mut prepaid,
            };
            let output = kernel::prepare(input, config, remaining, &mut local, &metered).map_err(composition)?.into_staged();
            local.work.finish()?;
            let actual = output.requirements();
            if actual.peak_resident_bytes > required.peak_resident_bytes
                || actual.index_bytes > required.index_bytes
                || actual.work_units > required.work_units
                || output.surface().resident_accounted_bytes()? > required.peak_resident_bytes
            {
                return Err(bad("Gaussian stage exceeds prospective requirements"));
            }
            poll(&metered)?;
            Ok(output)
        })();
        if self.work.exceeded.load(Ordering::Relaxed) {
            return Err(bad("raw layer preparation work limit"));
        }
        let output = staged?;
        self.validate(session, control)?;
        self.requirements.preparation_work = self.work.used.load(Ordering::Relaxed);
        Ok((self, output))
    }
}
fn verification_upper(source_work: u64, resident_limit: u64) -> Result<u64> {
    // Actual IO descriptor formula is source.work +8*authentic current census
    // retained bytes. That census must fit resident_limit under the same lease.
    add(source_work, mul(resident_limit, 8)?)
}
fn native_copy_envelope(format: photocraft_color::PixelFormat) -> Result<(u64, u64)> {
    if format.mode != photocraft_color::ColorMode::Grayscale
        || format.alpha
        || !matches!(format.sample, photocraft_color::SampleType::U8 | photocraft_color::SampleType::F64)
    {
        return Err(bad("original native Gray storage format"));
    }
    // IO closes each raw plane to 32x32 and at most one native tile. Price the
    // full native tile even for an empty sparse surface, plus the same 256-byte
    // per-map bookkeeping used by resident_accounted_bytes. This bounds current
    // equivalent zero tiles without assuming identical sparse representation.
    let index = mul(2, add(256, std::mem::size_of::<Surface>() as u64)?)?;
    let bodies = mul(
        2,
        add(
            mul(mul(photocraft_geom::TILE_SIZE as u64, photocraft_geom::TILE_SIZE as u64)?, format.bytes_per_pixel() as u64)?,
            format.bytes_per_pixel() as u64,
        )?,
    )?;
    Ok((add(bodies, index)?, index))
}
fn actual_copy_bytes(source: &Surface, mask: &Surface) -> Result<(u64, u64)> {
    if source.tile_count() > 1 || mask.tile_count() > 1 || source.format() != mask.format() {
        return Err(bad("proved native plane storage descriptor"));
    }
    let map = mul(add(source.tile_count() as u64, mask.tile_count() as u64)?, 256)?;
    let structs = mul(2, std::mem::size_of::<Surface>() as u64)?;
    Ok((add(add(source.resident_accounted_bytes()?, mask.resident_accounted_bytes()?)?, structs)?, add(map, structs)?))
}
/// Explicit raw-storage preparation from a genuine independently decoded owner.
/// It does not dispatch edits, prepare a selection, evaluate a filter, or grant
/// source/mask/profile authority to another consumer. All limits are lower-only.
#[allow(clippy::too_many_arguments)]
pub fn prepare<'o, 's, 'b, 'a, B: PreparationBudget>(
    session: &Session,
    layer: LayerId,
    owner: &'o photocraft_io::AssembledLayerSelection<'s, 'b, 'a>,
    limits: Limits,
    budget: &mut B,
    control: &Interrupt<'_>,
) -> Result<PreparedStoredLayer<'o, 's, 'b, 'a, B::Lease>> {
    prepare_with_profile_policy(session, layer, owner, photocraft_io::LayerProfilePolicy::OriginalExact, limits, budget, control)
}
/// Explicit opt-in only. The exact controlled IO/CMS proof of materialization
/// does not establish numerical layer/mask working-domain or coverage semantics.
pub fn prepare_with_profile_policy<'o, 's, 'b, 'a, B: PreparationBudget>(
    session: &Session,
    layer: LayerId,
    owner: &'o photocraft_io::AssembledLayerSelection<'s, 'b, 'a>,
    profile_policy: photocraft_io::LayerProfilePolicy,
    limits: Limits,
    budget: &mut B,
    control: &Interrupt<'_>,
) -> Result<PreparedStoredLayer<'o, 's, 'b, 'a, B::Lease>> {
    poll(control)?;
    idle(session)?;
    let max = Limits::default();
    if limits.resident_bytes > max.resident_bytes
        || limits.index_bytes > max.index_bytes
        || limits.work_units > max.work_units
        || layer.0 == 0
        || owner.layer_id() != layer
    {
        return Err(bad("raw layer input or lower-only limits"));
    }
    let current = session.active().ok_or(EngineError::NoDocument)?;
    let runtime_floor = runtime_bytes(&current.channel_view, &current.selected_layers, false)?;
    // Existing runtime backing is held while its copy is prepared, including
    // spare Vec capacity; it is not part of the document native census.
    let runtime_source =
        add(runtime_bytes(&current.channel_view, &[], true)?, mul(current.selected_layers.capacity() as u64, std::mem::size_of::<LayerId>() as u64)?)?;
    let runtime_reserved = add(runtime_source, runtime_floor)?;
    let header = std::mem::size_of::<PreparedStoredLayer<'_, '_, '_, '_, B::Lease>>() as u64;
    let (copy_upper, copy_index) = native_copy_envelope(owner.source_surface().format())?;
    if add(add(runtime_reserved, header)?, copy_upper)? > limits.resident_bytes || add(runtime_reserved, copy_index)? > limits.index_bytes {
        return Err(bad("raw storage and runtime envelope"));
    }
    budget.consume_preparation_work(limits.work_units).map_err(composition)?;
    let lease = budget.reserve_prepared_bytes(limits.resident_bytes).map_err(composition)?;
    let work = Work { used: AtomicU64::new(0), exceeded: AtomicBool::new(false), limit: limits.work_units };
    let cancelled = || control.cancelled() || work.charge(1).is_err();
    let progress = |fraction| control.progress(fraction);
    let metered = Interrupt::new(&cancelled, &progress);
    let staged = (|| {
        let original = owner.correspondence_requirements().map_err(|e| io_error(e, &metered))?;
        let retained_header = add(runtime_reserved, header)?;
        let reserved_index = add(add(runtime_reserved, copy_index)?, original.index_bytes)?;
        if add(add(original.resident_bytes, retained_header)?, copy_upper)? > limits.resident_bytes || reserved_index > limits.index_bytes {
            return Err(bad("complete original owner exceeds shared envelope"));
        }
        // Borrowed prospective CMS accounting precedes marker hashes, profile
        // clones and current census. Its documented builtin initialization is
        // the CMS provider boundary; the outer lease/token already remain held.
        let profile = photocraft_io::layer_profile_requirements(
            owner,
            &current.doc,
            profile_policy,
            photocraft_io::LayerCurrentLimits { resident_bytes: limits.resident_bytes, index_bytes: limits.index_bytes, work_units: work.remaining() },
            &metered,
        )
        .map_err(|error| io_error(error, &metered))?;
        if add(add(add(original.resident_bytes, retained_header)?, copy_upper)?, profile.resident_bytes)? > limits.resident_bytes {
            return Err(bad("prospective layer profile resident envelope"));
        }
        // Reserve the entire proof, including explicit profile verification,
        // before all current hierarchy/model/payload scans. No refunds/reset.
        let prepaid_units = add(verification_upper(original.work_units, limits.resident_bytes)?, profile.work_units)?;
        let mut proof_work = Prepaid::reserve(&work, prepaid_units)?;
        let resident = current.doc.resident_arc().ok_or(photocraft_raster::backing::BackingError::Unsupported)?;
        let backed = photocraft_doc::resident_structure::has_backed_rasters_with(resident, &metered)?;
        if backed {
            return Err(photocraft_raster::backing::BackingError::Unsupported.into());
        }
        let raster = current.doc.resident_raster_footprint_with(&metered)?;
        let raster_extra = add(raster.retained_bytes, mul(raster.clone_metadata_bytes, 4)?)?;
        let mut held = add(add(add(add(original.resident_bytes, profile.resident_bytes)?, retained_header)?, copy_upper)?, raster_extra)?;
        let model_limits = inspector_limits(limits, held, &work, reserved_index)?;
        let model = photocraft_io::retained_native_model_census(&current.doc, model_limits, &metered).map_err(|e| io_error(e, &metered))?;
        let model_bytes = model.retained_bytes() as u64;
        work.charge(model_bytes)?;
        held = add(held, model_bytes)?;
        let payload_limits = inspector_limits(limits, held, &work, reserved_index)?;
        let payload = photocraft_io::retained_native_payload_census(&current.doc, payload_limits, &metered).map_err(|e| io_error(e, &metered))?;
        let payload_bytes = payload.retained_bytes() as u64;
        work.charge(payload_bytes)?;
        let census = photocraft_io::NativeCurrentCensus::join(model, payload).map_err(|e| io_error(e, &metered))?;
        // Complete IO descriptor accounts current model/payload/maps and original
        // owner/index/scratch. Extra holds independent raster/COW/runtime/copies.
        let extra = add(add(retained_header, copy_upper)?, raster_extra)?;
        let selected_limits = photocraft_io::LayerCurrentLimits {
            resident_bytes: limits.resident_bytes.checked_sub(extra).ok_or_else(|| bad("selected raw layer resident envelope"))?,
            index_bytes: limits.index_bytes.checked_sub(add(runtime_reserved, copy_index)?).ok_or_else(|| bad("selected raw layer index envelope"))?,
            work_units: add(work.remaining(), proof_work.remaining)?,
        };
        let mut selected_resident = 0;
        let mut selected_index = 0;
        let selected = photocraft_io::select_layer_current_with_profile_policy(
            census,
            owner,
            profile_policy,
            selected_limits,
            &metered,
            &mut |resident, index| {
                let total = add(resident, extra).map_err(|e| photocraft_io::IoError::Unsupported(e.to_string()))?;
                let total_index = add(index, add(runtime_reserved, copy_index).map_err(|e| photocraft_io::IoError::Unsupported(e.to_string()))?)
                    .map_err(|e| photocraft_io::IoError::Unsupported(e.to_string()))?;
                if total > limits.resident_bytes || total_index > limits.index_bytes {
                    return Err(photocraft_io::IoError::Unsupported("selected raw layer exceeds shared envelope".into()));
                }
                selected_resident = total;
                selected_index = total_index;
                Ok(())
            },
            &mut |units| work.charge(units).map_err(|e| photocraft_io::IoError::Unsupported(e.to_string())),
        )
        .map_err(|e| io_error(e, &metered))?;
        let required = selected.requirements();
        if required.implicit_profile_peak_bytes > profile.resident_bytes || required.implicit_profile_work_units > profile.work_units {
            return Err(bad("actual layer profile exceeds prospective requirements"));
        }
        if !std::ptr::eq(selected.document(), &*current.doc)
            || selected.layer_id() != layer
            || selected.filter_id() != owner.filter_id()
            || selected.mask_id() != owner.mask_id()
            || required.verification_work_units > prepaid_units
        {
            return Err(bad("selected raw descriptor differs from captured source"));
        }
        let mut exact = proof_work.take_exact(required.verification_work_units)?;
        let proof = owner.verify_selected_layer_with_charge(&selected, &metered, &mut |units| exact.consume(units)).map_err(|e| io_error(e, &metered))?;
        exact.finish()?;
        if !std::ptr::eq(proof.document(), &*current.doc)
            || proof.layer_id() != layer
            || proof.filter_id() != owner.filter_id()
            || proof.mask_id() != owner.mask_id()
            || proof.source_sha256() != owner.source_sha256()
            || proof.source_blake3() != owner.source_blake3()
            || proof.original_bytes() != owner.original_bytes()
            || !std::ptr::eq(proof.original_node(), selected.original_node())
        {
            return Err(bad("opaque original layer proof differs from selected owner"));
        }
        let source = selected.source_surface();
        let mask = selected.filter_mask();
        let (copy_actual, index_actual) = actual_copy_bytes(source, &mask.surface)?;
        if copy_actual > copy_upper || index_actual > copy_index {
            return Err(bad("native COW copies exceed prospective envelope"));
        }
        let max_runtime = limits
            .resident_bytes
            .checked_sub(selected_resident.checked_sub(runtime_floor).ok_or_else(|| bad("runtime envelope underflow"))?)
            .ok_or_else(|| bad("runtime remaining envelope"))?;
        let max_runtime_index = limits
            .index_bytes
            .checked_sub(selected_index.checked_sub(runtime_floor).ok_or_else(|| bad("runtime index underflow"))?)
            .ok_or_else(|| bad("runtime remaining index envelope"))?;
        let (view, selected_layers, runtime_actual) =
            copy_runtime(&current.channel_view, &current.selected_layers, max_runtime.min(max_runtime_index), &metered)?;
        let actual_held = add(selected_resident.checked_sub(runtime_floor).ok_or_else(|| bad("runtime retained underflow"))?, runtime_actual)?;
        let actual_index = add(selected_index.checked_sub(runtime_floor).ok_or_else(|| bad("runtime retained index underflow"))?, runtime_actual)?;
        if actual_held > limits.resident_bytes || actual_index > limits.index_bytes {
            return Err(bad("actual runtime capacities exceed shared envelope"));
        }
        work.charge(copy_actual)?;
        // Exact-format identity path preserves raw U8/F64 words and native
        // defaults/padded tails, sharing Tile Arcs; no numeric read/conversion.
        let source_surface = source.try_convert_with(source.format(), &metered)?;
        let mask_surface = mask.surface.try_convert_with(mask.surface.format(), &metered)?;
        let (copied, copied_index) = actual_copy_bytes(&source_surface, &mask_surface)?;
        if copied > copy_upper || copied_index > copy_index {
            return Err(bad("actual native COW retained envelope"));
        }
        poll(&metered)?;
        let origin = Origin {
            snapshot: current.doc.clone(),
            weak: current.doc.downgrade(),
            revision: current.revision,
            active: current.active_layer,
            selected: selected_layers,
            view,
        };
        let requirements = Requirements {
            admitted_resident_bytes: limits.resident_bytes,
            admitted_work_units: limits.work_units,
            original_owner_bytes: original.resident_bytes,
            original_owner_index_bytes: original.index_bytes,
            current_raster_bytes: raster.retained_bytes,
            current_model_bytes: model_bytes,
            current_payload_bytes: payload_bytes,
            current_view_bytes: runtime_actual,
            native_copy_bytes: copied,
            held_resident_bytes: actual_held,
            held_index_bytes: actual_index,
            index_envelope_bytes: limits.index_bytes,
            prepaid_verification_work: prepaid_units,
            verification_work: required.verification_work_units,
            selection_work: required.selection_work_units,
            preparation_work: 0,
        };
        Ok((origin, source_surface, selected.source_bounds(), mask_surface, mask.bounds, requirements))
    })();
    if work.exceeded.load(Ordering::Relaxed) {
        return Err(bad("raw layer preparation work limit"));
    }
    let (origin, source_surface, source_bounds, mask_surface, mask_bounds, requirements) = staged?;
    let mut prepared = PreparedStoredLayer {
        original: owner,
        origin,
        layer,
        filter: owner.filter_id(),
        mask: owner.mask_id(),
        source_surface,
        source_bounds,
        mask_surface,
        mask_bounds,
        requirements,
        work,
        _lease: lease,
    };
    prepared.validate(session, control)?;
    prepared.requirements.preparation_work = prepared.work.used.load(Ordering::Relaxed);
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Size};
    fn session() -> Session {
        let mut session = Session::new();
        session.add_document(Document::new("raw origin", Size::new(32, 32), ColorMode::Grayscale, SampleType::F64), None).unwrap();
        session
    }
    // This is an origin-only fixture, never an original-plane proof.
    fn origin(session: &Session) -> Origin {
        let current = session.active().unwrap();
        Origin {
            snapshot: current.doc.clone(),
            weak: current.doc.downgrade(),
            revision: current.revision,
            active: current.active_layer,
            selected: current.selected_layers.clone(),
            view: current.channel_view.clone(),
        }
    }
    #[test]
    fn revision_or_channel_view_change_refuses_frozen_origin_without_mutation() {
        let mut session = session();
        let captured = origin(&session);
        assert!(captured.validate(&session, &Interrupt::NONE).is_ok());
        session.active_mut().unwrap().channel_view.quick_mask_hidden = true;
        assert!(captured.validate(&session, &Interrupt::NONE).is_err());
        session.active_mut().unwrap().channel_view.quick_mask_hidden = false;
        session.active_mut().unwrap().revision += 1;
        let before = session.active().unwrap().doc.clone();
        assert!(captured.validate(&session, &Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn same_document_id_is_not_snapshot_authority_and_cancel_never_changes_source() {
        let mut session = session();
        let captured = origin(&session);
        let mut replacement = Document::new("different", Size::new(32, 32), ColorMode::Grayscale, SampleType::F64);
        replacement.id = captured.snapshot.id;
        session.active_mut().unwrap().doc = DocumentSnapshot::from_resident(std::sync::Arc::new(replacement)).unwrap();
        assert!(captured.validate(&session, &Interrupt::NONE).is_err());
        let before = session.active().unwrap().doc.clone();
        assert!(matches!(captured.validate(&session, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn identity_cow_keeps_native_nan_and_signed_zero_without_numeric_ingress() {
        // Provider/copy admission fixture only, not an original-source proof.
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let payload_nan = 0x7ff8000000000142u64.to_le_bytes();
        let mut source = Surface::try_with_default_bytes(format, &payload_nan).unwrap();
        source.try_write_interleaved_with(Rect::new(0, 0, 1, 1), &(-0f64).to_le_bytes(), &Interrupt::NONE).unwrap();
        let mask = Surface::try_with_default_bytes(format, &0f64.to_le_bytes()).unwrap();
        let (upper, index) = native_copy_envelope(format).unwrap();
        let (actual, actual_index) = actual_copy_bytes(&source, &mask).unwrap();
        assert!(actual <= upper && actual_index <= index);
        let copied = source.try_convert_with(format, &Interrupt::NONE).unwrap();
        assert_eq!(copied.default_bytes(), &payload_nan);
        let mut word = [0u8; 8];
        copied.try_read_pixel_bytes(0, 0, &mut word).unwrap();
        assert_eq!(word, (-0f64).to_le_bytes());
        copied.try_read_pixel_bytes(-1, -1, &mut word).unwrap();
        assert_eq!(word, payload_nan);
        for ((coord, left), (copied_coord, right)) in source.tiles().zip(copied.tiles()) {
            assert_eq!(coord, copied_coord);
            assert!(std::sync::Arc::ptr_eq(left, right));
        }
        assert!(native_copy_envelope(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true)).is_err());
    }
    #[test]
    fn actual_inline_lease_is_part_of_preallocation_header_admission() {
        type Small = PreparedStoredLayer<'static, 'static, 'static, 'static, ()>;
        type Large = PreparedStoredLayer<'static, 'static, 'static, 'static, [u8; 4096]>;
        let small = std::mem::size_of::<Small>() as u64;
        let large = std::mem::size_of::<Large>() as u64;
        assert!(large >= small + 4096);
        let reserved = 100;
        let limit = add(small, reserved).unwrap();
        assert!(add(large, reserved).unwrap() > limit);
        // Size inspection does not construct or borrow a forged IO owner.
    }
    #[test]
    fn stage_uses_prepaid_remaining_work_and_keeps_raw_owners_charged() {
        use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianParams};
        use photocraft_compose::{gimp_channel_gaussian_candidate::DerivedPrecision, gimp_layer_selection_gaussian_candidate as kernel};
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let source = Surface::try_with_default_bytes(format, &0.25f64.to_le_bytes()).unwrap();
        let mask = Surface::try_with_default_bytes(format, &0f64.to_le_bytes()).unwrap();
        let inputs = kernel::Inputs { source: &source, mask: &mask, bounds: Rect::new(0, 0, 32, 32), mask_bounds: Rect::new(0, 0, 32, 32) };
        let config = kernel::Config {
            source_domain: kernel::SourceDomain::SrgbNonlinearF64,
            mask_domain: kernel::MaskDomain::NativeLinearF64,
            mask_policy: kernel::RawBinary::EmptySelectionUnrestricted,
            precision: DerivedPrecision::F64,
            gaussian: GaussianParams { std_dev_x: 1., std_dev_y: 1., abyss: GaussianAbyss::Clamp, clip_extent: true },
            filter: GaussianFilter::Auto,
            output: Rect::new(0, 0, 32, 32),
        };
        let limits = photocraft_compose::gimp_channel_gaussian_candidate::Limits::default();
        let required = kernel::requirements(inputs, config, limits, &Interrupt::NONE).unwrap();
        let work = Work { used: AtomicU64::new(13), exceeded: AtomicBool::new(false), limit: limits.work_units };
        let mut held = 100;
        let mut prepaid = Prepaid::reserve(&work, required.work_units).unwrap();
        let remaining = work.remaining();
        let mut local = StageBudget { held: &mut held, limit: limits.resident_bytes, peak: required.peak_resident_bytes, work: &mut prepaid };
        let output = kernel::prepare(inputs, config, limits, &mut local, &Interrupt::NONE).unwrap().into_staged();
        local.work.finish().unwrap();
        assert_eq!(*local.held, 100 + required.peak_resident_bytes);
        assert_eq!(work.remaining(), remaining);
        let mut value = [0.0];
        output.surface().try_read_pixel_f64(0, 0, &mut value).unwrap();
        assert!((value[0] - 0.25).abs() <= 2.0 / 255.0);
        assert_eq!(mask.default_bytes(), 0f64.to_le_bytes());
        drop(output);
        assert_eq!(*local.held, 100 + required.peak_resident_bytes); // no quota refund
        assert!(local.reserve_prepared_bytes(required.peak_resident_bytes + 1).is_err());
    }
    #[test]
    fn explicit_profile_verification_work_cannot_be_spent_by_current_census() {
        let profile = photocraft_io::LayerProfileRequirements { resident_bytes: 12, work_units: 9 };
        let upper = add(verification_upper(10, 8).unwrap(), profile.work_units).unwrap();
        let work = Work { used: AtomicU64::new(0), exceeded: AtomicBool::new(false), limit: 100 };
        let mut prepaid = Prepaid::reserve(&work, upper).unwrap();
        assert_eq!(work.remaining(), 17);
        work.charge(10).unwrap();
        let mut exact = prepaid.take_exact(add(42, profile.work_units).unwrap()).unwrap();
        exact.consume(51).unwrap();
        exact.finish().unwrap();
        assert_eq!(work.remaining(), 7);
        assert_eq!(prepaid.remaining, 32);
        assert!(work.charge(8).is_err());
        assert!(add(upper, u64::MAX).is_err());
    }
    #[test]
    fn quota_receipt_does_not_reset_refund_or_double_charge() {
        let work = Work { used: AtomicU64::new(0), exceeded: AtomicBool::new(false), limit: 100 };
        let mut upper = Prepaid::reserve(&work, 75).unwrap();
        work.charge(10).unwrap();
        let mut exact = upper.take_exact(60).unwrap();
        exact.consume(32).unwrap();
        exact.consume(28).unwrap();
        exact.finish().unwrap();
        assert_eq!(work.remaining(), 15);
        assert_eq!(upper.remaining, 15);
        assert!(upper.take_exact(16).is_err());
        drop(upper);
        assert_eq!(work.remaining(), 15);
        assert!(work.charge(16).is_err());
        assert!(add(u64::MAX, 1).is_err());
    }
}
