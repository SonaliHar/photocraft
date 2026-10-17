//! Immutable, independently charged layer selection for accounted Session state.
//! Clones share the physical selection and its lease. No detached Vec is exposed.
use photocraft_doc::{LayerId, accounted_document::AccountedDocument};
use photocraft_ops::LayerTarget;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::{mem::size_of, sync::Arc};

const MAX_SELECTED: usize = 4096;
const MAX_WORK: u64 = 1 << 24;
struct Owner {
    target: LayerTarget,
    anchor: Option<LayerId>,
    quota: BackingQuota,
    _lease: QuotaLease,
}
/// Sharing this owner is allocation-free and retains the selection charge.
#[derive(Clone)]
pub struct AccountedView {
    owner: Arc<Owner>,
}
impl std::fmt::Debug for AccountedView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountedView").field("target", &self.owner.target).field("anchor", &self.owner.anchor).finish_non_exhaustive()
    }
}
struct Work<'a, 'i> {
    control: &'a ReadControl<'i>,
    pending: u64,
    total: u64,
}
impl Work<'_, '_> {
    fn compare(&mut self) -> Result<(), BackingError> {
        self.total = self.total.checked_add(1).ok_or(BackingError::Length)?;
        if self.total > MAX_WORK {
            return Err(BackingError::Length);
        }
        self.pending = self.pending.checked_add(1).ok_or(BackingError::Length)?;
        if self.pending == 64 {
            self.control.check(self.pending)?;
            self.pending = 0;
        }
        Ok(())
    }
    fn finish(&self) -> Result<(), BackingError> {
        self.control.check(self.pending.max(1))
    }
}
fn validate(
    document: &AccountedDocument,
    active: Option<LayerId>,
    selected: &[LayerId],
    anchor: Option<LayerId>,
    control: &ReadControl<'_>,
) -> Result<(), BackingError> {
    control.check(1)?;
    let layers = &document.document().layers;
    if selected.len() > MAX_SELECTED || layers.len() > MAX_SELECTED {
        return Err(BackingError::Length);
    }
    if active.is_none() && !selected.is_empty() {
        return Err(BackingError::Bounds);
    }
    let mut active_seen = active.is_none();
    let mut anchor_seen = anchor.is_none();
    // Nested lookup also verifies the borrowed canonical parent chain. Charge
    // its bounded scan against this operation's work budget, so a large ordered
    // selection cannot multiply repeated tree/item scans without a ceiling.
    let checked_work = std::sync::atomic::AtomicU64::new(0);
    let check = |units: u64| {
        let units_to_charge = units.checked_mul(64).ok_or(BackingError::Length)?;
        checked_work
            .try_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |current| {
                current.checked_add(units_to_charge).filter(|total| *total <= MAX_WORK)
            })
            .map_err(|_| BackingError::Length)?;
        control.check(units)
    };
    let bounded = ReadControl { interrupt: control.interrupt, work: &check };
    let mut work = Work { control: &bounded, pending: 0, total: 0 };
    for (position, id) in selected.iter().enumerate() {
        for prior in selected.iter().take(position) {
            work.compare()?;
            if prior == id {
                return Err(BackingError::Bounds);
            }
        }
        if document.accounted_layer_access(*id, &bounded)?.is_none() {
            return Err(BackingError::Bounds);
        }
        active_seen |= active == Some(*id);
        anchor_seen |= anchor == Some(*id);
    }
    work.finish()?;
    if !active_seen || !anchor_seen {
        return Err(BackingError::Bounds);
    }
    Ok(())
}
impl AccountedView {
    pub fn new(
        document: &AccountedDocument,
        active: Option<LayerId>,
        selected: &[LayerId],
        anchor: Option<LayerId>,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        validate(document, active, selected, anchor, control)?;
        Self::allocate(document, active, selected, anchor, control)
    }
    fn allocate(
        document: &AccountedDocument,
        active: Option<LayerId>,
        selected: &[LayerId],
        anchor: Option<LayerId>,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        let payload = selected.len().checked_mul(size_of::<LayerId>()).ok_or(BackingError::Length)?;
        let fixed = size_of::<Owner>().checked_add(64).ok_or(BackingError::Length)?;
        let envelope = payload.checked_mul(2).and_then(|v| v.checked_add(fixed)).ok_or(BackingError::Length)?;
        let mut lease =
            document.quota().reserve(QuotaKind::Index, u64::try_from(envelope).map_err(|_| BackingError::Length)?).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let mut values = Vec::new();
        values.try_reserve_exact(selected.len()).map_err(|_| BackingError::Allocation)?;
        for chunk in selected.chunks(64) {
            control.check(chunk.len() as u64)?;
            values.extend_from_slice(chunk);
        }
        let actual = values.capacity().checked_mul(size_of::<LayerId>()).and_then(|v| v.checked_add(fixed)).ok_or(BackingError::Length)?;
        lease.reconcile_capacity(u64::try_from(actual).map_err(|_| BackingError::Length)?).map_err(|_| BackingError::Allocation)?;
        let owner = Owner { target: LayerTarget { active, selected: values }, anchor, quota: document.quota().clone(), _lease: lease };
        control.check(1)?;
        Ok(Self { owner: Arc::new(owner) })
    }
    pub fn target(&self) -> &LayerTarget {
        &self.owner.target
    }
    pub fn selected(&self) -> &[LayerId] {
        &self.owner.target.selected
    }
    pub fn anchor(&self) -> Option<LayerId> {
        self.owner.anchor
    }
    pub fn quota(&self) -> &BackingQuota {
        &self.owner.quota
    }
    pub fn shares_state(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
    }
    pub fn try_replace(
        &self,
        document: &AccountedDocument,
        active: Option<LayerId>,
        selected: &[LayerId],
        anchor: Option<LayerId>,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        if !self.quota().shares_ledger(document.quota()) {
            return Err(BackingError::Unsupported);
        }
        validate(document, active, selected, anchor, control)?;
        if active == self.target().active && selected == self.selected() && anchor == self.anchor() {
            return Ok(self.clone());
        }
        Self::allocate(document, active, selected, anchor, control)
    }
}
