//! Bounded preparation for an explicit stored-channel selection replacement.
//! No registered route or mutation is supplied here: default Load and its raw
//! admission stay unchanged. SourceBinding is an explicit original-byte witness,
//! not a renderer, and selection-only publication needs a separately reviewed
//! dispatcher/transaction boundary. The frozen snapshot owns source storage.
use crate::{
    DocumentSnapshot, EngineError, Result, Session,
    channel_cmds::ChannelView,
    xcf_channel_consumer::{SourceBinding, read_stored},
    xcf_item_locks::{EditKind, effective_item_locks},
};
use photocraft_doc::{
    Document, LayerId,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Caller limits may only lower the existing standard preparation envelope.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        let standard = photocraft_io::NativeSaveLimits::default().preparation;
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES.min(standard.resident_bytes as u64), index_bytes: standard.index_bytes as u64, work_units: 1u64 << 32 }
    }
}
impl Limits {
    fn validate(self) -> Result<()> {
        let standard = Self::default();
        if self.resident_bytes > standard.resident_bytes || self.index_bytes > standard.index_bytes || self.work_units > standard.work_units {
            return Err(bad("stored preparation limits must be lower-only"));
        }
        Ok(())
    }
}
struct Work {
    used: AtomicU64,
    exceeded: AtomicBool,
    limit: u64,
}
impl Work {
    fn charge(&self, units: u64) -> Result<()> {
        if self.used.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(units).filter(|next| *next <= self.limit)).is_err() {
            self.exceeded.store(true, Ordering::Relaxed);
            return Err(bad("stored preparation work limit"));
        }
        Ok(())
    }
    fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used.load(Ordering::Relaxed))
    }
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| bad("stored preparation accounting overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| bad("stored preparation accounting overflow"))
}
fn structural_view_bytes(view: &ChannelView, capacities: bool) -> Result<u64> {
    if view.color_hidden.len() > 16 || view.alpha_visible.len() > 10_000 {
        return Err(bad("selection view count limit"));
    }
    let colors = if capacities { view.color_hidden.capacity() } else { view.color_hidden.len() };
    let alphas = if capacities { view.alpha_visible.capacity() } else { view.alpha_visible.len() };
    add(std::mem::size_of::<PreparedStoredLoad>() as u64, add(colors as u64, alphas as u64)?)
}
fn repeated_validation_work(document: &Document, model: u64, payload: u64) -> Result<u64> {
    // Payload census deduplicates Arc storage, while typed validation can hash
    // the same retained allocation for several original records/owners. Charge
    // each possible owner separately instead of assuming one shared-byte visit.
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing stored validation state"))?;
    let mut repeats = 3u64; // source validation plus initial/final target scans
    for count in [state.source_items.len(), state.resources.len(), state.opaque.len(), state.editable_indexed.len(), state.filter_masks.len()] {
        repeats = add(repeats, count as u64)?;
    }
    mul(add(model, payload)?, repeats)
}
fn inspector_limits(limits: Limits, held: u64, work: &Work, view_reservation: u64) -> Result<photocraft_format::PreparationLimits> {
    let remaining = limits.resident_bytes.checked_sub(held).ok_or_else(|| bad("stored preparation resident limit"))?;
    // The native inspectors have no work argument. Their index/retained-byte
    // envelope is lowered to remaining declared work too, and their token polls
    // share this operation's meter. No fresh independent default budget.
    let available = remaining.min(work.remaining());
    let available = usize::try_from(available).map_err(|_| bad("stored inspector limit range"))?;
    let standard = photocraft_io::NativeSaveLimits::default().preparation;
    let index_remaining = limits.index_bytes.checked_sub(view_reservation).ok_or_else(|| bad("stored view index limit"))?;
    Ok(photocraft_format::PreparationLimits {
        resident_bytes: available,
        index_bytes: standard.index_bytes.min(usize::try_from(index_remaining).map_err(|_| bad("stored index range"))?).min(available),
        manifest_bytes: standard.manifest_bytes.min(available),
        object_bytes: standard.object_bytes.min(available),
    })
}
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "select.loadSelection".into(), msg: message.into() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn idle(session: &Session) -> Result<()> {
    if session.documents().len() > 4096 {
        return Err(bad("stored preparation document count limit"));
    }
    let current = session.active().ok_or(EngineError::NoDocument)?;
    if session.has_jobs()
        || session.command_has_pending_job()
        || session.command_is_background()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || current.coalesce.is_some()
        || current.floating.is_some()
        || current.revision == u64::MAX
        || current.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(bad("finish pending work before preparing a stored selection read"));
    }
    Ok(())
}
fn view_copy(view: &ChannelView, control: &Interrupt<'_>) -> Result<ChannelView> {
    if view.color_hidden.len() > 16 || view.alpha_visible.len() > 10_000 {
        return Err(bad("selection view count limit"));
    }
    let mut colors = Vec::new();
    let mut alphas = Vec::new();
    colors.try_reserve_exact(view.color_hidden.len()).map_err(|_| bad("selection view allocation"))?;
    alphas.try_reserve_exact(view.alpha_visible.len()).map_err(|_| bad("selection view allocation"))?;
    for value in &view.color_hidden {
        poll(control)?;
        colors.push(*value);
    }
    for value in &view.alpha_visible {
        poll(control)?;
        alphas.push(*value);
    }
    Ok(ChannelView {
        target: view.target,
        color_hidden: colors,
        alpha_visible: alphas,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
fn target(document: &Document, source: ChannelId, bounds: Rect, format: photocraft_color::PixelFormat, control: &Interrupt<'_>) -> Result<(ChannelId, Rect)> {
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("stored selection needs current typed state"))?;
    let id =
        document.selection_channel_id.filter(|id| id.0 != 0 && *id != source).ok_or_else(|| bad("stored selection needs an independent existing identity"))?;
    if document.channels.len() > 10_000
        || state.items.len() > 10_000
        || state.filter_stacks.len() > 10_000
        || state.source_items.len() > 10_000
        || state.opaque.len() > 100_000
    {
        return Err(bad("stored selection target count limit"));
    }
    for channel in &document.channels {
        poll(control)?;
        if channel.id == id {
            return Err(bad("selection identity aliases a saved channel"));
        }
    }
    if document.quick_mask.as_ref().is_some_and(|channel| channel.id == id) {
        return Err(bad("selection identity aliases Quick Mask"));
    }
    let selection = document.selection.as_ref().ok_or_else(|| bad("stored selection needs existing native storage"))?;
    if selection.format() != format {
        return Err(bad("stored source and target native Gray depth differ"));
    }
    let reference = ItemRef::Channel(id);
    let mut found = None;
    for item in &state.items {
        poll(control)?;
        if item.item == reference && found.replace(item).is_some() {
            return Err(bad("duplicate selection owner"));
        }
    }
    let item = found.ok_or_else(|| bad("missing current selection metadata"))?;
    let old = item.raster_bounds.filter(|bounds| !bounds.is_empty()).ok_or_else(|| bad("missing current selection bounds"))?;
    photocraft_raster::checked_region_byte_len(old, format)?;
    if item.source_flags != 0 || item.parent.is_some() || item.mask.is_some() || item.composition.is_some() {
        return Err(bad("selection metadata is not a plain stored target"));
    }
    for value in &state.source_items {
        poll(control)?;
        if value.owner == reference {
            return Err(bad("selection is an immutable source owner"));
        }
    }
    let mut visits = 0usize;
    for stack in &state.filter_stacks {
        poll(control)?;
        if stack.owner == reference {
            return Err(bad("selection is a filter-stack owner"));
        }
        visits = visits.checked_add(stack.nodes.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("selection dependency work limit"))?;
        for node in &stack.nodes {
            poll(control)?;
            visits = visits.checked_add(node.auxiliary.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("selection dependency work limit"))?;
            if node.mask == Some(reference) || node.auxiliary.iter().any(|(_, owner)| *owner == reference) {
                return Err(bad("selection is an operative filter input"));
            }
        }
    }
    for record in &state.opaque {
        poll(control)?;
        if record.rendering_required && (record.owner.is_none() || record.owner == Some(reference)) {
            return Err(bad("selection has unresolved rendering dependencies"));
        }
    }
    let locks = effective_item_locks(document, reference)?;
    locks.require(EditKind::Content)?;
    if old != bounds {
        locks.require(EditKind::Position)?;
    }
    poll(control)?;
    Ok((id, old))
}
#[derive(Clone, Copy, Debug)]
pub struct PreparationRequirements {
    /// Conservative known retained ownership plus a future metadata-edit COW envelope.
    pub resident_bytes: u64,
    pub work_units: u64,
    /// Conservative peak inspector scratch-index allowance plus actual view capacity.
    pub index_bytes: u64,
}
/// Prepared private storage ownership, never a persisted grant or commit token.
pub struct PreparedStoredLoad {
    snapshot: DocumentSnapshot,
    revision: u64,
    active: Option<LayerId>,
    view: ChannelView,
    source: ChannelId,
    selection: ChannelId,
    bounds: Rect,
    old_bounds: Rect,
    requirements: PreparationRequirements,
}
impl PreparedStoredLoad {
    pub fn source_channel(&self) -> ChannelId {
        self.source
    }
    pub fn selection_channel(&self) -> ChannelId {
        self.selection
    }
    pub fn source_bounds(&self) -> Rect {
        self.bounds
    }
    pub fn previous_selection_bounds(&self) -> Rect {
        self.old_bounds
    }
    pub fn requirements(&self) -> PreparationRequirements {
        self.requirements
    }
    /// Exact immutable current storage held by the original prepared snapshot.
    /// No filter is evaluated, merged or applied to these bytes.
    pub fn stored_surface(&self) -> Result<&Surface> {
        self.snapshot
            .channels
            .iter()
            .find(|channel| channel.id == self.source)
            .map(|channel| &channel.surface)
            .ok_or_else(|| bad("prepared stored source disappeared"))
    }
    /// Caller must check immediately before any future registered transaction.
    /// This method never edits selection/history and cannot bypass Load admission.
    pub fn validate(&self, session: &Session, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        idle(session)?;
        let current = session.active().ok_or(EngineError::NoDocument)?;
        if current.doc.id != self.snapshot.id
            || current.revision != self.revision
            || !current.doc.shares_state(&self.snapshot)
            || current.active_layer != self.active
            || current.channel_view != self.view
        {
            return Err(bad("stored selection target changed during preparation"));
        }
        let surface = self.stored_surface()?;
        let (id, old) = target(&current.doc, self.source, self.bounds, surface.format(), control)?;
        if id != self.selection || old != self.old_bounds {
            return Err(bad("stored selection identity or geometry changed"));
        }
        poll(control)
    }
}
/// Same-document preparation only, matching the observed public selection load.
/// The original binding is consumed only while borrowed; no source bytes, raw
/// records or typed filter config are copied into a background job or journal.
pub fn prepare(session: &Session, binding: &SourceBinding<'_>, control: &Interrupt<'_>) -> Result<PreparedStoredLoad> {
    prepare_with_limits(session, binding, Limits::default(), control)
}
/// Poll work, model/payload bytes and repeated validation work share one ledger.
/// Charges are conservative known operations, not CPU time or all legacy heap/RSS.
pub fn prepare_with_limits(session: &Session, binding: &SourceBinding<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<PreparedStoredLoad> {
    limits.validate()?;
    let work = Work { used: AtomicU64::new(0), exceeded: AtomicBool::new(false), limit: limits.work_units };
    let cancelled = || control.cancelled() || work.charge(1).is_err();
    let progress = |fraction| control.progress(fraction);
    let metered = Interrupt::new(&cancelled, &progress);
    let result = (|| {
        work.charge(mul(session.documents().len() as u64, 2)?)?;
        poll(&metered)?;
        idle(session)?;
        let current = session.active().ok_or(EngineError::NoDocument)?;
        current.doc.validate_resident()?;
        let requested_view = structural_view_bytes(&current.channel_view, false)?;
        if requested_view > limits.resident_bytes || requested_view > limits.index_bytes {
            return Err(bad("stored view preparation limit"));
        }
        work.charge(requested_view)?;
        // Metadata-only raster census is controlled before any inspector scratch
        // or view copy; the existing snapshot/provider is already owned by Session.
        let raster = current.doc.resident_raster_footprint_with(&metered)?;
        let mut held = add(add(raster.retained_bytes, mul(raster.clone_metadata_bytes, 4)?)?, requested_view)?;
        let mut model_limits = inspector_limits(limits, held, &work, requested_view)?;
        // The model is retained in the four-copy metadata envelope. Lower the
        // scanner before it allocates its identity index, not only afterwards.
        model_limits.resident_bytes /= 4;
        model_limits.index_bytes = model_limits.index_bytes.min(model_limits.resident_bytes);
        model_limits.manifest_bytes = model_limits.manifest_bytes.min(model_limits.resident_bytes);
        model_limits.object_bytes = model_limits.object_bytes.min(model_limits.resident_bytes);
        let mut scratch = model_limits.index_bytes as u64;
        let model = photocraft_io::retained_native_model_bytes(&current.doc, model_limits, &metered)
            .map_err(|error| if metered.cancelled() { EngineError::Cancelled } else { EngineError::Other(error.to_string()) })? as u64;
        work.charge(model)?;
        held = add(held, mul(model, 4)?)?;
        let payload_limits = inspector_limits(limits, held, &work, requested_view)?;
        scratch = scratch.max(payload_limits.index_bytes as u64);
        let payload = photocraft_io::retained_native_payload_bytes(&current.doc, payload_limits, &metered)
            .map_err(|error| if metered.cancelled() { EngineError::Cancelled } else { EngineError::Other(error.to_string()) })? as u64;
        work.charge(payload)?;
        held = add(held, payload)?;
        if add(held, scratch)? > limits.resident_bytes {
            return Err(bad("stored preparation resident/index peak limit"));
        }
        // Reserve repeated byte/comparison work for source typed validation and
        // both target scans before entry, including shared payload revalidation.
        // Per-loop polls
        // additionally debit the same meter; there is no refund on refusal.
        work.charge(repeated_validation_work(&current.doc, model, payload)?)?;
        let read = read_stored(&current.doc, binding, &metered)?;
        let (selection, old_bounds) = target(&current.doc, read.channel, read.bounds, read.surface.format(), &metered)?;
        let view = view_copy(&current.channel_view, &metered)?;
        let actual_view = structural_view_bytes(&view, true)?;
        let resident_bytes = add(add(held.checked_sub(requested_view).ok_or_else(|| bad("stored view accounting underflow"))?, actual_view)?, scratch)?;
        let index_bytes = add(actual_view, scratch)?;
        if resident_bytes > limits.resident_bytes || index_bytes > limits.index_bytes {
            return Err(bad("actual stored view/index capacity exceeds limits"));
        }
        let mut prepared = PreparedStoredLoad {
            snapshot: current.doc.clone(),
            revision: current.revision,
            active: current.active_layer,
            view,
            source: read.channel,
            selection,
            bounds: read.bounds,
            old_bounds,
            requirements: PreparationRequirements { resident_bytes, index_bytes, work_units: 0 },
        };
        prepared.validate(session, &metered)?;
        poll(&metered)?;
        prepared.requirements.work_units = work.used.load(Ordering::Relaxed);
        Ok(prepared)
    })();
    if work.exceeded.load(Ordering::Relaxed) {
        return Err(bad("stored preparation work limit"));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_are_lower_only_and_view_capacity_is_reserved() {
        let standard = Limits::default();
        assert!(Limits { resident_bytes: standard.resident_bytes + 1, ..standard }.validate().is_err());
        assert!(Limits { work_units: standard.work_units + 1, ..standard }.validate().is_err());
        let mut view = ChannelView::default();
        view.alpha_visible.try_reserve_exact(37).unwrap();
        view.alpha_visible.push(true);
        assert!(structural_view_bytes(&view, true).unwrap() > structural_view_bytes(&view, false).unwrap());
        let work = Work { used: AtomicU64::new(0), exceeded: AtomicBool::new(false), limit: 128 };
        let limits = Limits { resident_bytes: 1024, index_bytes: 128, work_units: 128 };
        let scanner = inspector_limits(limits, 256, &work, 32).unwrap();
        assert_eq!(scanner.resident_bytes, 128);
        assert_eq!(scanner.index_bytes, 96);
        work.charge(100).unwrap();
        let scanner = inspector_limits(limits, 256, &work, 32).unwrap();
        assert_eq!(scanner.resident_bytes, 28);
        assert!(work.charge(29).is_err());
        assert!(work.exceeded.load(Ordering::Relaxed));
    }
    fn document() -> Document {
        let mut doc = Document::new("Own stored target", photocraft_doc::Size::new(32, 32), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F64);
        doc.selection = Some(
            Surface::try_with_default_bytes(
                photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, doc.depth, false),
                &(-0f64).to_ne_bytes(),
            )
            .unwrap(),
        );
        doc.selection_channel_id = Some(ChannelId(17));
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items.iter_mut().find(|item| item.item == ItemRef::Channel(ChannelId(17))).unwrap().raster_bounds = Some(Rect::new(0, 0, 32, 32));
        doc.xcf_state = Some(state);
        doc
    }
    #[test]
    fn target_requires_existing_independent_identity_and_exact_native_depth() {
        let mut doc = document();
        let format = doc.selection.as_ref().unwrap().format();
        let before = doc.clone();
        assert_eq!(target(&doc, ChannelId(19), Rect::new(0, 0, 32, 32), format, &Interrupt::NONE).unwrap().0, ChannelId(17));
        assert_eq!(doc, before);
        assert!(target(&doc, ChannelId(17), Rect::new(0, 0, 32, 32), format, &Interrupt::NONE).is_err());
        assert!(target(&doc, ChannelId(19), Rect::new(0, 0, 32, 32), photocraft_color::PixelFormat::GRAY8, &Interrupt::NONE).is_err());
        doc.selection_channel_id = None;
        assert!(target(&doc, ChannelId(19), Rect::new(0, 0, 32, 32), format, &Interrupt::NONE).is_err());
    }
    #[test]
    fn target_content_lock_and_cancel_do_not_mutate_native_storage() {
        let mut doc = document();
        let format = doc.selection.as_ref().unwrap().format();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(ChannelId(17))).unwrap().locks.content = true;
        let before = doc.clone();
        assert!(target(&doc, ChannelId(19), Rect::new(0, 0, 32, 32), format, &Interrupt::NONE).is_err());
        assert_eq!(doc, before);
        let stop = || true;
        assert!(matches!(target(&doc, ChannelId(19), Rect::new(0, 0, 32, 32), format, &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
    }
}
