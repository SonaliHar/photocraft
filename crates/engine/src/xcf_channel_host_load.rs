//! Trusted host-owned original bytes to the registered stored-selection Load.
//! No path, locator, grant, renderer or source-profile inference is accepted.
//! This is not a normal XCF import route. Its independent original owner stays
//! live through the existing single-attempt command offer and publication.
use crate::{DocumentSnapshot, EngineError, Result, Session, channel_cmds::ChannelView, jobs::JobCtx};
use photocraft_doc::{
    DocId, LayerId,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_io::{
    CurrentProfilePolicy, SelectedLimits,
    xcf_channel_candidate::{Ids, Limits as MappingLimits, Namespace},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Budget, BudgetKind, ResourceLimits, ResourcePolicy};
use serde_json::Value;
use std::{
    mem::size_of,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug)]
pub struct CapturedStoredTarget {
    pub document: DocId,
    pub revision: u64,
    pub channel: ChannelId,
}
#[derive(Clone, Copy, Debug)]
pub struct Quota {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
/// All quotas are explicit lower-only caller limits. A phase cannot borrow an
/// unused allowance from another phase. Full phase bounds stay charged until
/// the registered command returns, including on failure.
pub struct HostLoadLimits {
    pub aggregate: Quota,
    pub original: Quota,
    pub mapping: MappingLimits,
    pub current: photocraft_format::PreparationLimits,
    pub selected: SelectedLimits,
    pub publication: crate::xcf_channel_stored_load::Limits,
}
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "select.loadSelection".into(), msg: format!("stored original host input: {message}") }
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| bad("accounting overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| bad("accounting overflow"))
}
fn io(error: photocraft_io::IoError) -> EngineError {
    if matches!(error, photocraft_io::IoError::Cancelled) { EngineError::Cancelled } else { EngineError::Other(error.to_string()) }
}
fn xcf(error: photocraft_xcf::XcfError) -> EngineError {
    if error.kind == photocraft_xcf::ErrorKind::Cancelled { EngineError::Cancelled } else { EngineError::Other(error.to_string()) }
}
fn quota(limits: &ResourceLimits, value: Quota) -> Result<()> {
    if value.resident_bytes > limits.get(BudgetKind::Resident)
        || value.index_bytes > limits.get(BudgetKind::Index)
        || value.work_units > limits.get(BudgetKind::Work)
    {
        return Err(bad("lower-only quota required"));
    }
    Ok(())
}
fn resource_limits(base: &ResourceLimits, q: Quota) -> Result<ResourceLimits> {
    base.lowered(&[("residentBytes", q.resident_bytes), ("indexBytes", q.index_bytes), ("sampleWorkUnits", q.work_units)]).map_err(xcf)
}
struct PhaseWork {
    used: AtomicU64,
    exhausted: AtomicBool,
    limit: u64,
}
impl PhaseWork {
    fn new(limit: u64) -> Self {
        Self { used: AtomicU64::new(0), exhausted: AtomicBool::new(false), limit }
    }
    fn charge(&self, units: u64) -> Result<()> {
        if self.used.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(units).filter(|v| *v <= self.limit)).is_err() {
            self.exhausted.store(true, Ordering::Relaxed);
            return Err(bad("selected phase prepaid work exhausted"));
        }
        Ok(())
    }
    #[cfg(test)]
    fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used.load(Ordering::Relaxed))
    }
}
fn fence(session: &Session, target: CapturedStoredTarget) -> Result<()> {
    let current = session.active().ok_or(EngineError::NoDocument)?;
    if target.document.0 == 0
        || target.channel.0 == 0
        || target.revision == u64::MAX
        || current.doc.id != target.document
        || current.revision != target.revision
    {
        return Err(bad("captured document/revision/channel target changed"));
    }
    if session.documents().len() > 4096
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_is_background()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || current.coalesce.is_some()
        || current.floating.is_some()
        || current.doc.accounted().is_some()
        || current.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
    {
        return Err(bad("finish pending work before using an original stored channel"));
    }
    Ok(())
}
fn view_bytes(view: &ChannelView) -> Result<u64> {
    if view.color_hidden.len() > 16 || view.alpha_visible.len() > 10_000 {
        return Err(bad("view count limit"));
    }
    add(add(view.color_hidden.capacity() as u64, view.alpha_visible.capacity() as u64)?, add(view.color_hidden.len() as u64, view.alpha_visible.len() as u64)?)
}
fn copy_view(view: &ChannelView, allowance: u64, ctl: &Interrupt<'_>) -> Result<ChannelView> {
    let copy = |values: &[bool], available: u64| -> Result<Vec<bool>> {
        let mut out = Vec::new();
        out.try_reserve_exact(values.len()).map_err(|_| bad("view allocation"))?;
        if out.capacity() as u64 > available {
            return Err(bad("actual view capacity before initialization"));
        }
        for value in values {
            ctl.check().map_err(|_| EngineError::Cancelled)?;
            out.push(*value);
        }
        Ok(out)
    };
    let color_hidden = copy(&view.color_hidden, allowance)?;
    let remaining = allowance.checked_sub(color_hidden.capacity() as u64).ok_or_else(|| bad("view allocation envelope"))?;
    let alpha_visible = copy(&view.alpha_visible, remaining)?;
    if add(color_hidden.capacity() as u64, alpha_visible.capacity() as u64)? > allowance {
        return Err(bad("actual view capacity exceeds its reservation"));
    }
    Ok(ChannelView {
        target: view.target,
        color_hidden,
        alpha_visible,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
fn headers() -> Result<u64> {
    // Real generic wrapper/header sizes, plus bounded stack frames and small
    // scalar/string hash scratch. Native payload and historical Arc<Document>
    // backing are separately priced by the selected/publication quotas.
    let sizes = [
        size_of::<HostLoadLimits>(),
        size_of::<DocumentSnapshot>(),
        size_of::<ChannelView>(),
        size_of::<JobCtx>(),
        size_of::<photocraft_io::AssembledStoredChannel<'_, '_, '_>>(),
        size_of::<photocraft_io::SelectedCurrentChannel<'_>>(),
        size_of::<photocraft_io::StoredChannelCorrespondence<'_>>(),
        size_of::<Budget<'_>>() * 2,
        size_of::<photocraft_xcf::Reservation<'_, '_>>() * 8,
        size_of::<PhaseWork>(),
        65536,
    ];
    sizes.into_iter().try_fold(0, |n, s| add(n, s as u64))
}
fn validate_limits(limits: &HostLoadLimits, base: &ResourceLimits) -> Result<()> {
    quota(base, limits.aggregate)?;
    quota(base, limits.original)?;
    quota(base, Quota { resident_bytes: limits.selected.resident_bytes, index_bytes: limits.selected.index_bytes, work_units: limits.selected.work_units })?;
    let mapping = MappingLimits::default();
    let selected = SelectedLimits::default();
    let publication = crate::xcf_channel_stored_load::Limits::default();
    let current = photocraft_format::PreparationLimits::default();
    if limits.mapping.metadata_bytes > mapping.metadata_bytes
        || limits.mapping.work_units > mapping.work_units
        || limits.mapping.namespace_items > mapping.namespace_items
        || limits.selected.resident_bytes > selected.resident_bytes
        || limits.selected.index_bytes > selected.index_bytes
        || limits.selected.work_units > selected.work_units
        || limits.publication.resident_bytes > publication.resident_bytes
        || limits.publication.index_bytes > publication.index_bytes
        || limits.publication.work_units > publication.work_units
        || limits.current.resident_bytes > current.resident_bytes
        || limits.current.index_bytes > current.index_bytes
        || limits.current.manifest_bytes > current.manifest_bytes
        || limits.current.object_bytes > current.object_bytes
        || limits.current.resident_bytes as u64 > limits.selected.resident_bytes
        || limits.current.index_bytes as u64 > limits.selected.index_bytes
    {
        return Err(bad("all phase/inspector limits must be lower-only"));
    }
    Ok(())
}
struct HostEnvelope {
    resident: u64,
    index: u64,
    work: u64,
    view_upper: u64,
}
fn host_envelope(session: &Session, target: CapturedStoredTarget, source_bytes: usize, limits: &HostLoadLimits, base: &ResourceLimits) -> Result<HostEnvelope> {
    fence(session, target)?;
    if source_bytes == 0 || source_bytes > 16384 {
        return Err(bad("prospective original input must contain 1..16384 bytes"));
    }
    validate_limits(limits, base)?;
    let current = session.active().ok_or(EngineError::NoDocument)?;
    let view_upper = add(view_bytes(&current.channel_view)?, 1024)?;
    let overhead = add(headers()?, add(source_bytes as u64, view_upper)?)?;
    let resident = add(overhead, add(limits.original.resident_bytes, add(limits.selected.resident_bytes, limits.publication.resident_bytes)?)?)?;
    let index = add(overhead, add(limits.original.index_bytes, add(limits.selected.index_bytes, limits.publication.index_bytes)?)?)?;
    let work = add(limits.original.work_units, add(limits.selected.work_units, limits.publication.work_units)?)?;
    // Resource Budget Index consumes Resident too. Reuse this exact category
    // formula both before capability I/O and for real reservations at execution.
    if add(resident, index)? > limits.aggregate.resident_bytes || index > limits.aggregate.index_bytes || work > limits.aggregate.work_units {
        return Err(bad("complete host phase/header/view envelope exceeds aggregate"));
    }
    Ok(HostEnvelope { resident, index, work, view_upper })
}
/// Allocation-free successful admission of limits, hidden idle context and the
/// complete known prospective recipe. This is not a source/correspondence grant.
/// A host calls with its maximum prospective input BEFORE any capability read;
/// execution repeats the same checks using actual bytes and real reservations.
pub fn validate_host_request(session: &Session, target: CapturedStoredTarget, prospective_source_bytes: usize, limits: &HostLoadLimits) -> Result<()> {
    let base = ResourceLimits::for_policy(ResourcePolicy::default()).map_err(xcf)?;
    host_envelope(session, target, prospective_source_bytes, limits, &base).map(|_| ())
}
/// Explicit original record ordinal is not derived from a name, tattoo, current
/// channel index or canonical ID. A wrong ordinal fails genuine correspondence.
#[allow(clippy::too_many_arguments)]
pub fn load_from_original(
    session: &mut Session,
    target: CapturedStoredTarget,
    original: &[u8],
    original_channel_ordinal: usize,
    limits: HostLoadLimits,
    policy: CurrentProfilePolicy,
    context: &JobCtx,
) -> Result<Value> {
    context.check()?;
    fence(session, target)?; // before any source construction/allocation
    if original.is_empty() || original.len() > 16384 || original_channel_ordinal >= 10_000 {
        return Err(bad("bounded exact original input/ordinal required"));
    }
    let base = ResourceLimits::for_policy(ResourcePolicy::default()).map_err(xcf)?;
    let envelope = host_envelope(session, target, original.len(), &limits, &base)?;
    let cancelled = || context.cancelled();
    let grand = Budget::new(resource_limits(&base, limits.aggregate)?, Some(&cancelled));
    let current = session.active().ok_or(EngineError::NoDocument)?;
    let HostEnvelope { resident, index, work, view_upper } = envelope;
    // Whole simultaneous bound before decoder, indexes, snapshots or copies.
    let _resident = grand.reserve(BudgetKind::Resident, resident, "host original/full current/publication coexistence").map_err(xcf)?;
    let _index = grand.reserve(BudgetKind::Index, index, "host original/full current/publication indexes").map_err(xcf)?;
    grand.consume_work(work, "host original/selected/publication prepaid work").map_err(xcf)?;
    let snapshot = current.doc.clone(); // immutable strong ownership, no model clone
    let active: Option<LayerId> = current.active_layer;
    let phase = PhaseWork::new(limits.selected.work_units);
    phase.charge(view_upper)?;
    let view = context.stage(0.0, 0.05, "Capture Stored Channel Target", |ctl| copy_view(&current.channel_view, view_upper, ctl))?;
    let prepared = context.stage(0.05, 0.5, "Verify Original Stored Channel", |ctl| -> Result<_> {
        let cancelled = || ctl.cancelled() || phase.charge(1).is_err();
        let progress = |fraction| ctl.progress(fraction);
        let metered = Interrupt::new(&cancelled, &progress);
        let candidate = crate::xcf_channel_consumer::inspect_candidate(
            &snapshot,
            target.channel,
            crate::xcf_channel_consumer::ChannelConsumer::StoredSelectionRead,
            &metered,
        )?
        .ok_or_else(|| bad("current channel has no closed stored-source candidate"))?;
        let mask = match candidate.filter.mask {
            Some(ItemRef::Mask(id)) => id,
            _ => return Err(bad("closed candidate needs actual native filter mask identity")),
        };
        let ids = Ids { channel: target.channel, filter: candidate.filter.id, mask };
        // Independent ORIGINAL canonical projection, not an asserted current
        // namespace. Full current aliases/dependencies are checked by select_current.
        let source_channels = [target.channel];
        let namespace = photocraft_io::NamespaceBacking {
            namespace: Namespace { channels: &source_channels, filters: &[], masks: &[] },
            retained_bytes: size_of::<[ChannelId; 1]>() as u64,
        };
        let original_cancelled = || ctl.cancelled();
        let original_budget = Budget::new(resource_limits(&base, limits.original)?, Some(&original_cancelled));
        let mut options = photocraft_xcf::DecodeOptions::default();
        options.limits.input_bytes = original.len();
        options.limits.resident_bytes = usize::try_from(limits.original.resident_bytes).map_err(|_| bad("original resident address range"))?;
        let owner = photocraft_io::assemble_owned_stored_channel(
            original,
            original_channel_ordinal,
            ids,
            namespace,
            limits.mapping,
            &options,
            &original_budget,
            photocraft_xcf::SourceAccounting::Borrowed,
            ctl,
        )
        .map_err(io)?;
        // Fresh private decoder stays private throughout this call; no mutable
        // public decoded owner crosses the host boundary or stale charge receipt.
        let original_required = owner.correspondence_requirements().map_err(io)?;
        if original_budget.used(BudgetKind::Resident) > limits.original.resident_bytes
            || original_budget.used(BudgetKind::Index) > limits.original.index_bytes
            || original_required.resident_bytes > limits.original.resident_bytes
        {
            return Err(bad("actual complete original owner exceeded its prepaid phase"));
        }
        let profile = photocraft_io::selected_profile_requirements(&owner, &snapshot, policy, limits.selected, &metered).map_err(io)?;
        let prepaid = add(add(original_required.work_units, mul(limits.selected.resident_bytes, 8)?)?, profile.work_units)?;
        phase.charge(prepaid)?; // before any current hierarchy/model/payload census
        let model = photocraft_io::retained_native_model_census(&snapshot, limits.current, &metered).map_err(io)?;
        let model_bytes = model.retained_bytes() as u64;
        let mut payload_limits = limits.current;
        payload_limits.resident_bytes =
            payload_limits.resident_bytes.checked_sub(model.retained_bytes()).ok_or_else(|| bad("current model/payload coexistence"))?;
        let payload = photocraft_io::retained_native_payload_census(&snapshot, payload_limits, &metered).map_err(io)?;
        let census = photocraft_io::NativeCurrentCensus::join(model, payload).map_err(io)?;
        phase.charge(add(model_bytes, census.payload_retained_bytes() as u64)?)?;
        let descriptor = photocraft_io::select_current_with_profile_policy(
            census,
            &owner,
            policy,
            limits.selected,
            &metered,
            &mut |resident, index| {
                if resident > limits.selected.resident_bytes || index > limits.selected.index_bytes {
                    return Err(photocraft_io::IoError::Unsupported("host complete current/source quota".into()));
                }
                Ok(())
            },
            &mut |units| phase.charge(units).map_err(|error| photocraft_io::IoError::Unsupported(error.to_string())),
        )
        .map_err(io)?;
        let required = descriptor.requirements();
        if required.implicit_profile_resident_bytes > profile.resident_bytes
            || required.implicit_profile_work_units > profile.work_units
            || required.verification_work_units > prepaid
        {
            return Err(bad("actual selected proof exceeds prospective envelope"));
        }
        let mut proof_remaining = required.verification_work_units;
        let proof = owner
            .verify_selected_with_charge(&descriptor, &metered, &mut |units| {
                proof_remaining = proof_remaining.checked_sub(units).ok_or_else(|| photocraft_io::IoError::Unsupported("host prepaid proof overrun".into()))?;
                Ok(())
            })
            .map_err(io)?;
        if proof_remaining != 0
            || !std::ptr::eq(proof.document(), &*snapshot)
            || proof.channel_id() != target.channel
            || proof.source_sha256() != owner.source_sha256()
            || proof.original_bytes() != original
        {
            return Err(bad("exact independently decoded correspondence required"));
        }
        // The actual binding must borrow the proof's independently mapped node,
        // never the current candidate used only to capture canonical IDs.
        phase.charge(mul(original.len() as u64, 2)?)?;
        let binding = crate::xcf_channel_consumer::bind_source(proof.original_bytes(), proof.channel_id(), proof.original_node(), &metered)?;
        fence(session, target)?;
        let current = session.active().ok_or(EngineError::NoDocument)?;
        phase.charge(mul(view_upper, 2)?)?;
        if !current.doc.shares_state(&snapshot) || current.active_layer != active || current.channel_view != view {
            return Err(bad("captured original host target/view changed"));
        }
        metered.check().map_err(|_| EngineError::Cancelled)?;
        if phase.exhausted.load(Ordering::Relaxed) {
            return Err(bad("selected phase work exhausted"));
        }
        // Original owner, descriptor/proof, source bytes, census and all outer
        // real reservations stay live through this registered command attempt.
        crate::channel_cmds::load_stored_selection_with_binding(session, &binding, limits.publication, context)
    });
    // No cancellation check, accounting traversal or response allocation after
    // publication: a successful registered result must stay successful.
    if phase.exhausted.load(Ordering::Relaxed) && prepared.is_err() {
        return Err(bad("selected phase work exhausted"));
    }
    prepared
}

#[cfg(test)]
#[path = "xcf_channel_host_load_tests.rs"]
mod tests;
