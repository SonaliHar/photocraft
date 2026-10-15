//! Resident compatibility and independently charged accounted undo/redo state.
//! Accounted metadata uses the model's persistent quota; pixel-byte estimates from
//! legacy resident tiles are deliberately unsupported for this branch.
use crate::document_snapshot::DocumentSnapshot;
use photocraft_doc::{
    Document,
    accounted_document::{AccountedDocument, AccountedEdit},
};
use photocraft_ops::{History, LayerTarget};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease, QuotaUsage},
};
use std::{mem::size_of, sync::Arc};

#[derive(Debug)]
pub struct ChargedLayerTarget {
    values: LayerTarget,
    lease: Option<QuotaLease>,
}
impl ChargedLayerTarget {
    pub fn values(&self) -> &LayerTarget {
        &self.values
    }
    /// The receiving state must keep the returned lease until values are dropped.
    pub fn into_parts(self) -> (LayerTarget, Option<QuotaLease>) {
        (self.values, self.lease)
    }
}
#[derive(Debug)]
pub struct HistoryChange {
    snapshot: DocumentSnapshot,
    target: ChargedLayerTarget,
}
impl HistoryChange {
    pub fn snapshot(&self) -> &DocumentSnapshot {
        &self.snapshot
    }
    pub fn target(&self) -> &LayerTarget {
        self.target.values()
    }
    pub fn into_parts(self) -> (DocumentSnapshot, ChargedLayerTarget) {
        (self.snapshot, self.target)
    }
}
#[derive(Debug)]
pub struct HistoryEntries {
    values: Vec<String>,
    _lease: Option<QuotaLease>,
}
impl HistoryEntries {
    pub fn values(&self) -> &[String] {
        &self.values
    }
}
#[derive(Debug)]
struct Metadata {
    label: String,
    target: LayerTarget,
    _lease: QuotaLease,
}
#[derive(Clone, Debug)]
struct Entry {
    document: AccountedDocument,
    metadata: Arc<Metadata>,
}
#[derive(Debug)]
struct Accounted {
    states: Vec<Entry>,
    cursor: usize,
    maximum: usize,
    _lease: QuotaLease,
}
#[derive(Clone, Debug)]
enum Branch {
    Resident(History),
    Accounted(Arc<Accounted>),
}
#[derive(Clone, Debug)]
pub struct DocumentHistory {
    pub max_states: usize,
    pub max_bytes: usize,
    branch: Branch,
}

fn envelope(bytes: usize, owner: usize) -> Result<u64, BackingError> {
    bytes.checked_mul(2).and_then(|n| n.checked_add(owner)).and_then(|n| u64::try_from(n).ok()).ok_or(BackingError::Length)
}
fn checked_actual(bytes: usize, owner: usize) -> Result<u64, BackingError> {
    bytes.checked_add(owner).and_then(|n| u64::try_from(n).ok()).ok_or(BackingError::Length)
}
fn metadata(q: &BackingQuota, label: &str, target: &LayerTarget, ctl: &ReadControl<'_>) -> Result<Metadata, BackingError> {
    if label.len() > 4096 || label.contains('\0') || target.selected.len() > 4096 {
        return Err(BackingError::Length);
    }
    ctl.check(1)?;
    let bytes = target.selected.len().checked_mul(size_of::<photocraft_doc::LayerId>()).and_then(|n| n.checked_add(label.len())).ok_or(BackingError::Length)?;
    let mut lease = q.reserve(QuotaKind::Index, envelope(bytes, size_of::<Metadata>() + 64)?).map_err(|_| BackingError::Quota)?;
    let mut name = String::new();
    name.try_reserve_exact(label.len()).map_err(|_| BackingError::Allocation)?;
    let mut selected = Vec::new();
    selected.try_reserve_exact(target.selected.len()).map_err(|_| BackingError::Allocation)?;
    let actual =
        selected.capacity().checked_mul(size_of::<photocraft_doc::LayerId>()).and_then(|n| n.checked_add(name.capacity())).ok_or(BackingError::Length)?;
    lease.reconcile_capacity(checked_actual(actual, size_of::<Metadata>() + 64)?).map_err(|_| BackingError::Allocation)?;
    ctl.check(label.len() as u64)?;
    name.push_str(label);
    for chunk in target.selected.chunks(64) {
        ctl.check(chunk.len() as u64)?;
        selected.extend_from_slice(chunk);
    }
    ctl.check(1)?;
    Ok(Metadata { label: name, target: LayerTarget { active: target.active, selected }, _lease: lease })
}
fn target_copy(q: &BackingQuota, value: &LayerTarget, ctl: &ReadControl<'_>) -> Result<ChargedLayerTarget, BackingError> {
    let Metadata { target, _lease, .. } = metadata(q, "", value, ctl)?;
    Ok(ChargedLayerTarget { values: target, lease: Some(_lease) })
}
fn slots(q: &BackingQuota, maximum: usize, ctl: &ReadControl<'_>) -> Result<Accounted, BackingError> {
    ctl.check(1)?;
    let bytes = maximum.checked_mul(size_of::<Entry>()).ok_or(BackingError::Length)?;
    let mut lease = q.reserve(QuotaKind::Index, envelope(bytes, size_of::<Accounted>() + 64)?).map_err(|_| BackingError::Quota)?;
    let mut states = Vec::new();
    states.try_reserve_exact(maximum).map_err(|_| BackingError::Allocation)?;
    let actual = states.capacity().checked_mul(size_of::<Entry>()).ok_or(BackingError::Length)?;
    lease.reconcile_capacity(checked_actual(actual, size_of::<Accounted>() + 64)?).map_err(|_| BackingError::Allocation)?;
    Ok(Accounted { states, cursor: 0, maximum, _lease: lease })
}
impl DocumentHistory {
    /// Resident-only controlled record/trim admission. Historical Arc payloads remain caller-held.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resident_record_requirements_with_extra_tiles(
        &self, before:&DocumentSnapshot, candidate:&DocumentSnapshot, label:&str,
        prior:&LayerTarget, layers:&LayerTarget, extra_candidate_tile_occurrences:u64,
        limits:photocraft_ops::ResidentHistoryLimits, ctl:&photocraft_raster::Interrupt<'_>,
    )->Result<photocraft_ops::ResidentHistoryRequirements,photocraft_raster::RasterInputError>{
        let Branch::Resident(history)=&self.branch else{return Err(photocraft_raster::RasterInputError::UnsupportedConversion)};
        before.resident_arc().ok_or(photocraft_raster::RasterInputError::UnsupportedConversion)?;
        candidate.resident_arc().ok_or(photocraft_raster::RasterInputError::UnsupportedConversion)?;
        let maximum=photocraft_ops::ResidentHistoryLimits::default();
        if limits.resident_bytes>maximum.resident_bytes || limits.work_units>maximum.work_units{return Err(photocraft_raster::RasterInputError::LimitExceeded)}
        let extra=(size_of::<Self>() as u64).checked_mul(3).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        let lower=photocraft_ops::ResidentHistoryLimits{resident_bytes:limits.resident_bytes.checked_sub(extra).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?,work_units:limits.work_units};
        let mut requirement=history.resident_record_requirements_with_extra_tiles(before,candidate,label,prior,layers,self.max_states,self.max_bytes,extra_candidate_tile_occurrences,lower,ctl)?;
        requirement.retained_bytes=requirement.retained_bytes.checked_add(size_of::<Self>() as u64).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        requirement.clone_bytes=requirement.clone_bytes.checked_add((size_of::<Self>() as u64).checked_mul(2).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        requirement.peak_bytes=requirement.peak_bytes.checked_add(extra).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        Ok(requirement)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_clone_record_trim_with_interrupt(
        &self,before:DocumentSnapshot,candidate:&DocumentSnapshot,label:&str,
        prior:&LayerTarget,layers:&LayerTarget,limits:photocraft_ops::ResidentHistoryLimits,
        ctl:&photocraft_raster::Interrupt<'_>,
    )->Result<Self,photocraft_raster::RasterInputError>{
        let Branch::Resident(history)=&self.branch else{return Err(photocraft_raster::RasterInputError::UnsupportedConversion)};
        self.resident_record_requirements_with_extra_tiles(&before,candidate,label,prior,layers,0,limits,ctl)?;
        let arc=before.resident_arc().ok_or(photocraft_raster::RasterInputError::UnsupportedConversion)?.clone();
        let extra=(size_of::<Self>() as u64).checked_mul(3).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        let lower=photocraft_ops::ResidentHistoryLimits{resident_bytes:limits.resident_bytes.checked_sub(extra).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?,work_units:limits.work_units};
        let result=history.try_clone_record_trim_with_interrupt(arc,candidate,label,prior,layers,self.max_states,self.max_bytes,lower,ctl)?;
        ctl.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
        Ok(Self::resident(result))
    }
    /// Prospective metadata census only; byte-budgeted or accounted history has a separate consumer.
    pub(crate) fn plain_clone_census_work(&self) -> Result<u64, photocraft_raster::RasterInputError> {
        if self.max_bytes != 0 {
            return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
        }
        let Branch::Resident(history) = &self.branch else {
            return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
        };
        history.plain_clone_census_work()
    }

    pub(crate) fn plain_clone_metadata_requirements(
        &self,
        byte_limit: u64,
        work_limit: u64,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<photocraft_ops::CloneMetadataRequirements, photocraft_raster::RasterInputError> {
        self.plain_clone_census_work()?;
        let Branch::Resident(history) = &self.branch else {
            return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
        };
        let extra = size_of::<Self>() as u64;
        let extra_bytes = extra.checked_mul(3).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        let extra_work = extra.checked_mul(8).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        let mut needs = history.plain_clone_metadata_requirements(
            byte_limit.checked_sub(extra_bytes).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?,
            work_limit.checked_sub(extra_work).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?,
            control,
        )?;
        needs.retained_bytes = needs.retained_bytes.checked_add(extra).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        needs.clone_bytes = needs.clone_bytes.checked_add(extra.checked_mul(2).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?)
            .ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        needs.clone_work_units = needs.clone_work_units.checked_add(extra_work).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        Ok(needs)
    }

    pub fn resident(history: History) -> Self {
        Self { max_states: history.max_states, max_bytes: history.max_bytes, branch: Branch::Resident(history) }
    }
    pub fn new_resident(max_states: usize) -> Self {
        Self::resident(History::new(max_states))
    }
    pub fn new_accounted(
        initial: AccountedDocument,
        max_states: usize,
        label: &str,
        layers: &LayerTarget,
        ctl: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        if max_states == 0 || max_states > 255 {
            return Err(BackingError::Length);
        }
        let maximum = max_states.checked_add(1).ok_or(BackingError::Length)?;
        let mut owner = slots(initial.quota(), maximum, ctl)?;
        let meta = Arc::new(metadata(initial.quota(), label, layers, ctl)?);
        owner.states.push(Entry { document: initial, metadata: meta });
        ctl.check(1)?;
        Ok(Self { max_states, max_bytes: 0, branch: Branch::Accounted(Arc::new(owner)) })
    }
    /// Clone shares physical history. A mutation copies only bounded slots;
    /// model/native payloads and label/selection metadata retain their own guards.
    fn update<T>(&mut self, ctl: &ReadControl<'_>, op: impl FnOnce(&mut Accounted) -> Result<T, BackingError>) -> Result<T, BackingError> {
        let Branch::Accounted(old) = &self.branch else { return Err(BackingError::Unsupported) };
        let q = old.states.get(old.cursor).ok_or(BackingError::Integrity)?.document.quota();
        let mut staged = slots(q, old.maximum, ctl)?;
        for chunk in old.states.chunks(64) {
            ctl.check(chunk.len() as u64)?;
            staged.states.extend_from_slice(chunk);
        }
        staged.cursor = old.cursor;
        let result = op(&mut staged)?;
        ctl.check(1)?;
        self.branch = Branch::Accounted(Arc::new(staged));
        Ok(result)
    }
    pub fn shares_accounted_history(&self, other: &Self) -> bool {
        match (&self.branch, &other.branch) {
            (Branch::Accounted(a), Branch::Accounted(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    fn sync_limits(&mut self) -> Result<(), BackingError> {
        match &mut self.branch {
            Branch::Resident(h) => {
                h.max_states = self.max_states;
                h.max_bytes = self.max_bytes;
                Ok(())
            }
            Branch::Accounted(h) => {
                if self.max_bytes != 0 {
                    return Err(BackingError::Unsupported);
                }
                if self.max_states == 0 || self.max_states >= h.maximum {
                    return Err(BackingError::Length);
                }
                Ok(())
            }
        }
    }
    pub fn current(&self) -> Result<DocumentSnapshot, BackingError> {
        match &self.branch {
            Branch::Resident(_) => Err(BackingError::Unsupported),
            Branch::Accounted(h) => DocumentSnapshot::from_accounted(h.states.get(h.cursor).ok_or(BackingError::Integrity)?.document.clone()),
        }
    }
    pub fn state(&self, index: usize) -> Result<Option<DocumentSnapshot>, BackingError> {
        match &self.branch {
            Branch::Resident(h) => h.state(index).map(DocumentSnapshot::from_resident).transpose(),
            Branch::Accounted(h) => {
                if index < h.cursor {
                    h.states.get(index).map(|e| DocumentSnapshot::from_accounted(e.document.clone())).transpose()
                } else {
                    Ok(None)
                }
            }
        }
    }
    pub fn past_len(&self) -> usize {
        match &self.branch {
            Branch::Resident(h) => h.past_len(),
            Branch::Accounted(h) => h.cursor,
        }
    }
    pub fn can_undo(&self) -> bool {
        self.past_len() > 0
    }
    pub fn can_redo(&self) -> bool {
        match &self.branch {
            Branch::Resident(h) => h.can_redo(),
            Branch::Accounted(h) => h.cursor.checked_add(1).is_some_and(|n| n < h.states.len()),
        }
    }
    pub fn undo_label(&self) -> Option<&str> {
        match &self.branch {
            Branch::Resident(h) => h.undo_label(),
            Branch::Accounted(h) => {
                if h.cursor > 0 {
                    h.states.get(h.cursor).map(|e| e.metadata.label.as_str())
                } else {
                    None
                }
            }
        }
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo_labels().next()
    }
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        let resident = match &self.branch {
            Branch::Resident(h) => Some(h.redo_labels()),
            _ => None,
        };
        let accounted = match &self.branch {
            Branch::Accounted(h) => h.states.get(h.cursor.saturating_add(1)..).unwrap_or(&[]),
            _ => &[],
        };
        resident.into_iter().flatten().chain(accounted.iter().map(|e| e.metadata.label.as_str()))
    }
    /// Legacy record is intentionally not a generic mutable accounted transaction.
    pub fn record(&mut self, label: &str, before: DocumentSnapshot, layers: LayerTarget, ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        let Branch::Resident(_) = &self.branch else { return Err(BackingError::Unsupported) };
        before.validate_resident()?;
        let arc = before.resident_arc().ok_or(BackingError::Unsupported)?.clone();
        ctl.check(1)?;
        self.sync_limits()?;
        let Branch::Resident(h) = &mut self.branch else { return Err(BackingError::Integrity) };
        h.record(label, arc, layers);
        Ok(())
    }
    pub fn try_commit_accounted(
        &mut self,
        edit: AccountedEdit<'_>,
        label: &str,
        layers: &LayerTarget,
        ctl: &ReadControl<'_>,
    ) -> Result<DocumentSnapshot, BackingError> {
        if !matches!(self.branch, Branch::Accounted(_)) {
            return Err(BackingError::Unsupported);
        }
        self.sync_limits()?;
        let max_states = self.max_states;
        let Branch::Accounted(owner) = &self.branch else { return Err(BackingError::Unsupported) };
        let current = owner.states.get(owner.cursor).ok_or(BackingError::Integrity)?;
        let meta = Arc::new(metadata(current.document.quota(), label, layers, ctl)?);
        let staged = current.document.try_edit(edit, ctl)?;
        let snapshot = DocumentSnapshot::from_accounted(staged.clone())?;
        // No-op edits preserve the shared physical history, redo, and target.
        if staged.shares_model(&current.document) {
            ctl.check(1)?;
            return Ok(snapshot);
        }
        self.update(ctl, |h| {
            let next = h.cursor.checked_add(1).ok_or(BackingError::Length)?;
            let drop_oldest = h.cursor >= max_states;
            let retained = next.checked_sub(usize::from(drop_oldest)).ok_or(BackingError::Integrity)?;
            if retained >= h.states.capacity() {
                return Err(BackingError::Integrity);
            }
            ctl.check(1)?;
            h.states.truncate(next);
            if drop_oldest {
                h.states.remove(0);
            }
            h.states.push(Entry { document: staged, metadata: meta });
            h.cursor = retained;
            Ok(snapshot)
        })
    }
    /// Record a genuine already-prepared model without copying or editing it.
    /// The typed caller must establish the patch and current renderer/source
    /// contract before this history-only handoff. It must stage this on a private
    /// history and fence the matching Session state before publication.
    ///
    /// An exact current-owner no-op preserves the physical history and redo
    /// before changed-only limits or metadata allocations are considered.
    pub(crate) fn try_commit_prepared_accounted(
        &mut self,
        expected: &AccountedDocument,
        staged: AccountedDocument,
        label: &str,
        layers: &LayerTarget,
        ctl: &ReadControl<'_>,
    ) -> Result<DocumentSnapshot, BackingError> {
        ctl.check(1)?;
        let Branch::Accounted(owner) = &self.branch else {
            return Err(BackingError::Unsupported);
        };
        let current = owner.states.get(owner.cursor).ok_or(BackingError::Integrity)?;
        if !current.document.shares_model(expected)
            || staged.document().id != expected.document().id
            || !staged.quota().shares_ledger(expected.quota())
        {
            return Err(BackingError::Integrity);
        }
        let snapshot = DocumentSnapshot::from_accounted(staged.clone())?;
        if staged.shares_model(&current.document) {
            ctl.check(1)?;
            return Ok(snapshot);
        }
        self.sync_limits()?;
        let max_states = self.max_states;
        let meta = Arc::new(metadata(staged.quota(), label, layers, ctl)?);
        // update prepays and copies only bounded Entry slots. Both the old
        // history and staged model retain their actual metadata/native owners.
        self.update(ctl, |history| {
            let current = history.states.get(history.cursor).ok_or(BackingError::Integrity)?;
            if !current.document.shares_model(expected) {
                return Err(BackingError::Integrity);
            }
            let next = history.cursor.checked_add(1).ok_or(BackingError::Length)?;
            let drop_oldest = history.cursor >= max_states;
            let retained = next.checked_sub(usize::from(drop_oldest)).ok_or(BackingError::Integrity)?;
            if retained >= history.states.capacity() {
                return Err(BackingError::Integrity);
            }
            ctl.check(1)?;
            history.states.truncate(next);
            if drop_oldest {
                history.states.remove(0);
            }
            history.states.push(Entry { document: staged, metadata: meta });
            history.cursor = retained;
            Ok(snapshot)
        })
    }

    /// Stage a save-identity replacement without recording an undo step.
    /// The caller must prepare this in a cloned history before external output,
    /// and fence/publish that history with the matching Session snapshot. Only a
    /// genuine charged model on the exact current ledger and document identity
    /// is accepted; this does not authorize a raw Document mutation callback.
    /// Labels, layer targets, cursor, and all prior undo/redo entries are retained.
    pub(crate) fn try_replace_accounted_current(
        &mut self,
        expected: &AccountedDocument,
        prepared: AccountedDocument,
        ctl: &ReadControl<'_>,
    ) -> Result<DocumentSnapshot, BackingError> {
        ctl.check(1)?;
        let Branch::Accounted(owner) = &self.branch else {
            return Err(BackingError::Unsupported);
        };
        let current = owner.states.get(owner.cursor).ok_or(BackingError::Integrity)?;
        if !current.document.shares_model(expected) || prepared.document().id != expected.document().id || !prepared.quota().shares_ledger(expected.quota()) {
            return Err(BackingError::Integrity);
        }
        let snapshot = DocumentSnapshot::from_accounted(prepared.clone())?;
        if prepared.shares_model(&current.document) {
            ctl.check(1)?;
            return Ok(snapshot);
        }
        // update reserves its complete slot/header envelope before allocation;
        // entries share genuine model and Metadata owners, never copied totals.
        self.update(ctl, |history| {
            let current = history.states.get_mut(history.cursor).ok_or(BackingError::Integrity)?;
            if !current.document.shares_model(expected) {
                return Err(BackingError::Integrity);
            }
            ctl.check(1)?;
            current.document = prepared;
            Ok(snapshot)
        })
    }

    /// Continue an already-authorized metadata gesture at the current history tip.
    /// Key/anchor continuity is the Session witness's responsibility. A paid view
    /// transition may supply a newly captured expected_history; this method does
    /// not equate a gesture key with a layer or freeze the original focus view.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_continue_accounted_metadata(
        &mut self,
        expected_history: &DocumentHistory,
        expected_current: &AccountedDocument,
        edit: AccountedEdit<'_>,
        _label: &str,
        layers: &LayerTarget,
        ctl: &ReadControl<'_>,
    ) -> Result<DocumentSnapshot, BackingError> {
        ctl.check(1)?;
        if !matches!(edit, AccountedEdit::LayerMetadata { .. }) {
            return Err(BackingError::Unsupported);
        }
        if !self.shares_accounted_history(expected_history) || self.max_states != expected_history.max_states || self.max_bytes != expected_history.max_bytes {
            return Err(BackingError::Integrity);
        }
        let Branch::Accounted(owner) = &self.branch else {
            return Err(BackingError::Unsupported);
        };
        let current = owner.states.get(owner.cursor).ok_or(BackingError::Integrity)?;
        if !current.document.shares_model(expected_current) {
            return Err(BackingError::Integrity);
        }
        let staged = current.document.try_edit(edit, ctl)?;
        let snapshot = DocumentSnapshot::from_accounted(staged.clone())?;
        // No-op requests never destroy redo or alter the current gesture marker,
        // even when the caller is at an undo destination or initial history slot.
        if staged.shares_model(&current.document) {
            ctl.check(1)?;
            return Ok(snapshot);
        }
        if owner.cursor == 0 || owner.cursor.checked_add(1) != Some(owner.states.len()) {
            return Err(BackingError::Integrity);
        }
        if layers.selected.len() > 4096 {
            return Err(BackingError::Length);
        }
        ctl.check(layers.selected.len() as u64)?;
        if current.metadata.target != *layers {
            return Err(BackingError::Integrity);
        }
        // A resident matching-key frame updates only the current target, keeping
        // the first changed frame's label. Retain its actual charged owner while
        // preparing the fresh metadata copy; caller label is intentionally unused.
        let first_metadata = Arc::clone(&current.metadata);
        self.sync_limits()?;
        let meta = Arc::new(metadata(staged.quota(), &first_metadata.label, layers, ctl)?);
        self.update(ctl, |history| {
            if history.cursor == 0 || history.cursor.checked_add(1) != Some(history.states.len()) {
                return Err(BackingError::Integrity);
            }
            let current = history.states.get_mut(history.cursor).ok_or(BackingError::Integrity)?;
            if !current.document.shares_model(expected_current) {
                return Err(BackingError::Integrity);
            }
            ctl.check(1)?;
            *current = Entry { document: staged, metadata: meta };
            Ok(snapshot)
        })
    }

    /// Typed charged pixel selection, separate from layer-target transactions.
    pub fn try_commit_accounted_selection(
        &mut self,
        selection: Option<&photocraft_doc::accounted_selection::AccountedSelection>,
        label: &str,
        layers: &LayerTarget,
        ctl: &ReadControl<'_>,
    ) -> Result<DocumentSnapshot, BackingError> {
        if !matches!(self.branch, Branch::Accounted(_)) {
            return Err(BackingError::Unsupported);
        }
        self.sync_limits()?;
        let max_states = self.max_states;
        let Branch::Accounted(owner) = &self.branch else { return Err(BackingError::Unsupported) };
        let current = owner.states.get(owner.cursor).ok_or(BackingError::Integrity)?;
        let meta = Arc::new(metadata(current.document.quota(), label, layers, ctl)?);
        let staged = current.document.try_replace_selection(selection, ctl)?;
        let snapshot = DocumentSnapshot::from_accounted(staged.clone())?;
        if staged.shares_model(&current.document) {
            ctl.check(1)?;
            return Ok(snapshot);
        }
        self.update(ctl, |h| {
            let next = h.cursor.checked_add(1).ok_or(BackingError::Length)?;
            let drop_oldest = h.cursor >= max_states;
            let retained = next.checked_sub(usize::from(drop_oldest)).ok_or(BackingError::Integrity)?;
            if retained >= h.states.capacity() {
                return Err(BackingError::Integrity);
            }
            ctl.check(1)?;
            h.states.truncate(next);
            if drop_oldest {
                h.states.remove(0);
            }
            h.states.push(Entry { document: staged, metadata: meta });
            h.cursor = retained;
            Ok(snapshot)
        })
    }
    fn travel(&mut self, current: &DocumentSnapshot, redo: bool, ctl: &ReadControl<'_>) -> Result<Option<HistoryChange>, BackingError> {
        if let Branch::Resident(h) = &mut self.branch {
            current.validate_resident()?;
            ctl.check(1)?;
            let arc = current.resident_arc().ok_or(BackingError::Unsupported)?.clone();
            let mut staged = h.clone();
            let value = if redo { staged.redo(arc) } else { staged.undo(arc) };
            let change = value
                .map(|(doc, values)| {
                    Ok::<_, BackingError>(HistoryChange { snapshot: DocumentSnapshot::from_resident(doc)?, target: ChargedLayerTarget { values, lease: None } })
                })
                .transpose()?;
            ctl.check(1)?;
            *h = staged;
            return Ok(change);
        }
        self.update(ctl, |h| {
            let now = h.states.get(h.cursor).ok_or(BackingError::Integrity)?;
            if !current.accounted().is_some_and(|d| d.shares_model(&now.document)) {
                return Err(BackingError::Integrity);
            }
            let destination = if redo { h.cursor.checked_add(1).filter(|n| *n < h.states.len()) } else { h.cursor.checked_sub(1) };
            let Some(destination) = destination else { return Ok(None) };
            let entry = h.states.get(destination).ok_or(BackingError::Integrity)?;
            let target = target_copy(entry.document.quota(), &entry.metadata.target, ctl)?;
            let snapshot = DocumentSnapshot::from_accounted(entry.document.clone())?;
            ctl.check(1)?;
            h.cursor = destination;
            Ok(Some(HistoryChange { snapshot, target }))
        })
    }
    pub fn undo(&mut self, current: &DocumentSnapshot, ctl: &ReadControl<'_>) -> Result<Option<HistoryChange>, BackingError> {
        self.travel(current, false, ctl)
    }
    pub fn redo(&mut self, current: &DocumentSnapshot, ctl: &ReadControl<'_>) -> Result<Option<HistoryChange>, BackingError> {
        self.travel(current, true, ctl)
    }
    pub fn set_current_layers(&mut self, layers: LayerTarget, ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        if let Branch::Resident(h) = &mut self.branch {
            ctl.check(1)?;
            h.set_current_layers(layers);
            return Ok(());
        }
        self.update(ctl, |h| {
            let e = h.states.get_mut(h.cursor).ok_or(BackingError::Integrity)?;
            e.metadata = Arc::new(metadata(e.document.quota(), &e.metadata.label, &layers, ctl)?);
            Ok(())
        })
    }
    /// Accounted view state can lend its selection without allocating an
    /// uncharged intermediate Vec. Resident callers retain the owned legacy API.
    pub fn set_current_layers_borrowed(&mut self, layers: &LayerTarget, ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        if let Branch::Accounted(h) = &self.branch
            && h.states.get(h.cursor).is_some_and(|e| e.metadata.target == *layers)
        {
            return ctl.check(1);
        }
        self.update(ctl, |h| {
            let e = h.states.get_mut(h.cursor).ok_or(BackingError::Integrity)?;
            e.metadata = Arc::new(metadata(e.document.quota(), &e.metadata.label, layers, ctl)?);
            Ok(())
        })
    }
    pub fn set_current_label(&mut self, label: &str, ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        if let Branch::Resident(h) = &mut self.branch {
            ctl.check(1)?;
            h.set_current_label(label);
            return Ok(());
        }
        self.update(ctl, |h| {
            let e = h.states.get_mut(h.cursor).ok_or(BackingError::Integrity)?;
            e.metadata = Arc::new(metadata(e.document.quota(), label, &e.metadata.target, ctl)?);
            Ok(())
        })
    }
    /// Borrowed labels avoid an uncharged panel-model copy.
    pub fn entry_labels(&self) -> impl Iterator<Item = &str> {
        let values = match &self.branch {
            Branch::Accounted(h) => h.states.get(..=h.cursor).unwrap_or(&[]),
            _ => &[],
        };
        let resident = match &self.branch {
            Branch::Resident(h) => Some(h),
            _ => None,
        };
        resident.into_iter().flat_map(History::entry_labels).chain(values.iter().map(|e| e.metadata.label.as_str()))
    }
    pub fn entries(&self, ctl: &ReadControl<'_>) -> Result<HistoryEntries, BackingError> {
        let Branch::Accounted(h) = &self.branch else {
            ctl.check(1)?;
            let Branch::Resident(h) = &self.branch else { return Err(BackingError::Integrity) };
            return Ok(HistoryEntries { values: h.entries(), _lease: None });
        };
        let count = h.cursor.checked_add(1).ok_or(BackingError::Length)?;
        let slots = count.checked_mul(size_of::<String>()).ok_or(BackingError::Length)?;
        let strings = self.entry_labels().try_fold(0usize, |n, s| n.checked_add(s.len()).ok_or(BackingError::Length))?;
        let quota = h.states.get(h.cursor).ok_or(BackingError::Integrity)?.document.quota();
        let mut lease = quota
            .reserve(QuotaKind::Index, envelope(slots.checked_add(strings).ok_or(BackingError::Length)?, size_of::<HistoryEntries>())?)
            .map_err(|_| BackingError::Quota)?;
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
        let mut actual = values.capacity().checked_mul(size_of::<String>()).ok_or(BackingError::Length)?;
        for s in self.entry_labels() {
            ctl.check(s.len() as u64)?;
            let mut v = String::new();
            v.try_reserve_exact(s.len()).map_err(|_| BackingError::Allocation)?;
            actual = actual.checked_add(v.capacity()).ok_or(BackingError::Length)?;
            if checked_actual(actual, size_of::<HistoryEntries>())? > lease.bytes() {
                return Err(BackingError::Allocation);
            };
            v.push_str(s);
            values.push(v);
        }
        lease.reconcile_capacity(checked_actual(actual, size_of::<HistoryEntries>())?).map_err(|_| BackingError::Allocation)?;
        ctl.check(1)?;
        Ok(HistoryEntries { values, _lease: Some(lease) })
    }
    pub fn clear_redo(&mut self, ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        if let Branch::Resident(h) = &mut self.branch {
            ctl.check(1)?;
            h.clear_redo();
            return Ok(());
        }
        self.update(ctl, |h| {
            h.states.truncate(h.cursor.checked_add(1).ok_or(BackingError::Length)?);
            Ok(())
        })
    }
    pub fn clear(&mut self, ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        if let Branch::Resident(h) = &mut self.branch {
            ctl.check(1)?;
            h.clear();
            return Ok(());
        }
        self.update(ctl, |h| {
            h.states.truncate(h.cursor.checked_add(1).ok_or(BackingError::Length)?);
            if h.cursor > 0 {
                h.states.drain(..h.cursor);
                h.cursor = 0;
            }
            Ok(())
        })
    }
    pub fn purge_last(&mut self, ctl: &ReadControl<'_>) -> Result<bool, BackingError> {
        if let Branch::Resident(h) = &mut self.branch {
            ctl.check(1)?;
            return Ok(h.purge_last());
        }
        self.update(ctl, |h| {
            h.states.truncate(h.cursor.checked_add(1).ok_or(BackingError::Length)?);
            if h.cursor == 0 {
                Ok(false)
            } else {
                h.states.remove(h.cursor - 1);
                h.cursor -= 1;
                Ok(true)
            }
        })
    }
    pub fn trim(&mut self, current: &DocumentSnapshot, ctl: &ReadControl<'_>) -> Result<usize, BackingError> {
        self.sync_limits()?;
        if let Branch::Resident(h) = &mut self.branch {
            current.validate_resident()?;
            ctl.check(1)?;
            return Ok(h.trim(current));
        }
        let max = self.max_states;
        self.update(ctl, |h| {
            let now = h.states.get(h.cursor).ok_or(BackingError::Integrity)?;
            if !current.accounted().is_some_and(|d| d.shares_model(&now.document)) {
                return Err(BackingError::Integrity);
            }
            let n = h.cursor.saturating_sub(max);
            if n > 0 {
                h.states.drain(..n);
                h.cursor -= n;
            }
            Ok(n)
        })
    }
    pub fn unique_bytes(&self, current: &Document) -> Result<usize, BackingError> {
        if current.has_backed_rasters().map_err(|_| BackingError::Bounds)? {
            return Err(BackingError::Unsupported);
        }
        match &self.branch {
            Branch::Resident(h) => Ok(h.unique_bytes(current)),
            _ => Err(BackingError::Unsupported),
        }
    }
    pub fn pixel_bytes(&self, current: &Document) -> Result<usize, BackingError> {
        if current.has_backed_rasters().map_err(|_| BackingError::Bounds)? {
            return Err(BackingError::Unsupported);
        }
        match &self.branch {
            Branch::Resident(h) => Ok(h.pixel_bytes(current)),
            _ => Err(BackingError::Unsupported),
        }
    }
    pub fn quota_usage(&self) -> Result<Option<QuotaUsage>, BackingError> {
        match &self.branch {
            Branch::Resident(_) => Ok(None),
            Branch::Accounted(h) => h.states.get(h.cursor).ok_or(BackingError::Integrity)?.document.quota().usage().map(Some).map_err(|_| BackingError::Quota),
        }
    }
}

#[cfg(test)]
#[path = "document_history_accounted_replacement_tests.rs"]
mod accounted_replacement_tests;

#[cfg(test)]
#[path = "document_history_accounted_continuation_tests.rs"]
mod accounted_continuation_tests;

#[cfg(test)]
#[path = "document_history_accounted_prepared_tests.rs"]
mod accounted_prepared_tests;
