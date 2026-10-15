//! Paid, per-document metadata gesture key. A key is only coalescing policy;
//! it is never source, model, profile, layer, or view authority. All actual
//! current-owner and history validation is repeated by the typed transaction.
use super::*;
use photocraft_doc::DocId;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLease};
/// Constant-time routing hint over an actual owner. Full closed structural and
/// current profile/native validation remains mandatory; this grants no authority.
/// Layer-properties routing only; unlike the assigned helper this may include
/// readonly flat composites and must never be used for mask or Move admission.
pub fn owned_layer_metadata_candidate(owner:&AccountedDocument)->bool {
    owner.is_readonly_flat_composite_storage() || assigned_geometry_metadata_candidate(owner)
}
pub fn assigned_geometry_metadata_candidate(owner: &AccountedDocument) -> bool {
    if owner.is_readonly_gaussian_group_storage() { return true; }
    if owner.is_readonly_point_storage() || owner.is_readonly_threshold_storage() || owner.is_readonly_threshold_alpha_storage() {
        return false;
    }
    isolated_normal_group_metadata_candidate(owner)
        || (owner.document().layers.len() == 2
            && owner.document().layers.get(1).is_some_and(|layer| layer.mask.is_some())
            && owner.document().xcf_state.as_ref().is_some_and(|state| state.items.len() == 3))
}
fn isolated_normal_group_metadata_candidate(owner: &AccountedDocument) -> bool {
    owner.document().layers.len() == 2
        && owner.document().layers.get(1).is_some_and(|layer| matches!(&layer.content, photocraft_doc::LayerContent::Group(group) if group.children.len() == 2))
        && owner.document().xcf_state.as_ref().is_some_and(|state| matches!(state.items.len(), 4 | 5))
}
#[derive(Clone, Copy)]
pub(crate) enum MetadataPolicy {
    Plain,
    FlatComposite,
    AssignedGeometry,
    IsolatedNormalGroup,
    GaussianGroup,
}
impl MetadataPolicy {
    pub(crate) fn for_owner(owner: &AccountedDocument) -> Self {
        if owner.is_readonly_flat_composite_storage() {
            Self::FlatComposite
        } else if owner.is_readonly_gaussian_group_storage() {
            Self::GaussianGroup
        } else if isolated_normal_group_metadata_candidate(owner) {
            Self::IsolatedNormalGroup
        } else if assigned_geometry_metadata_candidate(owner) {
            Self::AssignedGeometry
        } else {
            Self::Plain
        }
    }
    pub(crate) fn needs_assigned_fence(self) -> bool {
        !matches!(self, Self::Plain)
    }
    pub(crate) fn validate_storage(self, owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
        match self {
            Self::Plain => Err(BackingError::Unsupported.into()),
            Self::FlatComposite => Ok(owner.validate_readonly_flat_composite_storage(c)?),
            Self::AssignedGeometry => Ok(owner.validate_assigned_analytic_geometry_storage(c)?),
            Self::IsolatedNormalGroup => Ok(owner.validate_assigned_isolated_normal_group_storage(c)?),
            Self::GaussianGroup => Ok(owner.validate_gaussian_group_storage(c)?),
        }
    }
    pub(crate) fn validate_current(self, owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
        match self {
            Self::Plain => Ok(()),
            Self::FlatComposite => crate::accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(owner,c),
            Self::AssignedGeometry => crate::accounted_xcf_open::analytic_geometry::validate_current_analytic_geometry_storage(owner, c),
            Self::IsolatedNormalGroup => crate::accounted_xcf_open::isolated_normal_group::validate_current_isolated_normal_group_storage(owner, c),
            Self::GaussianGroup => crate::accounted_xcf_open::gaussian_group::validate_current_isolated_gaussian_group_storage(owner,c),
        }
    }
}
struct KeyOwner {
    key: String,
    document: DocId,
    quota: BackingQuota,
    _lease: QuotaLease,
}
#[derive(Clone)]
pub(crate) struct PaidMetadataGesture {
    owner: Arc<KeyOwner>,
}
impl std::fmt::Debug for PaidMetadataGesture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaidMetadataGesture").field("document", &self.owner.document).finish_non_exhaustive()
    }
}
impl PaidMetadataGesture {
    pub(crate) fn matches(&self, key: &str, document: DocId, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<bool> {
        c.check((self.owner.key.len() + key.len() + 1) as u64)?;
        Ok(self.owner.document == document && self.owner.quota.shares_ledger(quota) && self.owner.key == key)
    }
    pub(crate) fn prepare(key: &str, document: DocId, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<Self> {
        if key.len() > 128 {
            return Err(BackingError::Length.into());
        }
        c.check(key.len() as u64)?;
        let bytes = key.len().checked_mul(2).and_then(|n| n.checked_add(std::mem::size_of::<KeyOwner>() + 128)).ok_or(BackingError::Length)?;
        let mut lease = quota.reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
        let mut owned = String::new();
        owned.try_reserve_exact(key.len()).map_err(|_| BackingError::Allocation)?;
        if owned.capacity() > key.len().checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation.into());
        }
        // Empty and NUL-containing keys are inert exact strings, as upstream.
        owned.push_str(key);
        let actual = owned.capacity().checked_add(std::mem::size_of::<KeyOwner>() + 128).ok_or(BackingError::Length)?;
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        c.check(1)?;
        Ok(Self { owner: Arc::new(KeyOwner { key: owned, document, quota: quota.clone(), _lease: lease }) })
    }
}
impl Session {
    /// Same exact per-document key continues the existing typed metadata anchor.
    /// Requested layer and changed fields do not form an implicit key prefix.
    /// Errors and actual no-ops preserve both the paid key and the redo branch.
    pub fn edit_accounted_metadata_with_coalesce(&mut self, label: &str, edit: AccountedEdit<'_>, key: Option<&str>, c: &ReadControl<'_>) -> Result<()> {
        c.check(1)?;
        if !matches!(edit, AccountedEdit::LayerMetadata { .. }) || key.is_some_and(|v| v.len() > 128) {
            return Err(BackingError::Unsupported.into());
        }
        if self.active_job().is_some() {
            return Err(EngineError::Other("the document has an active background job".into()));
        }
        let state = self.active_mut().ok_or(EngineError::NoDocument)?;
        if state.floating.is_some() || state.coalesce.is_some() {
            return Err(BackingError::Unsupported.into());
        }
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        owner.validate_imported_metadata_edit(edit, c)?;
        let policy = MetadataPolicy::for_owner(owner);
        if policy.needs_assigned_fence() && !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        policy.validate_current(owner, c)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        if state.channel_view.layer_mask.is_some()
            || state.channel_view.editing_mask.is_some()
            || !matches!(state.channel_view.target, crate::channel_cmds::ChannelTarget::Composite | crate::channel_cmds::ChannelTarget::Color(_))
        {
            return Err(BackingError::Unsupported.into());
        }
        if matches!(state.channel_view.target,crate::channel_cmds::ChannelTarget::Color(i) if i>=state.doc.mode.color_channels()) {
            return Err(BackingError::Bounds.into());
        }
        edit_locks(owner, edit, None, c)?;
        let matching_key = match (key, state._accounted_metadata_gesture.as_ref()) {
            (Some(key), Some(marker)) => marker.matches(key, state.doc.id, owner.quota(), c)?,
            _ => false,
        };
        // Upstream appends when no undo anchor remains, even for the same key.
        let continuation = matching_key && state.history.can_undo();
        let mut history = state.history.clone();
        history.set_current_layers_borrowed(view.target(), c)?;
        let snapshot = if continuation {
            let current_history = history.clone();
            history.try_continue_accounted_metadata(&current_history, owner, edit, label, view.target(), c)?
        } else {
            history.try_commit_accounted(edit, label, view.target(), c)?
        };
        if snapshot.shares_state(&state.doc) {
            c.check(1)?;
            return Ok(());
        }
        let current = snapshot.accounted().ok_or(BackingError::Integrity)?;
        policy.validate_current(current, c)?;
        let next_view = view.try_replace(current, state.active_layer, view.selected(), state.layer_anchor, c)?;
        let gesture = match key {
            None => None,
            Some(_) if matching_key => state._accounted_metadata_gesture.clone(),
            Some(key) => Some(PaidMetadataGesture::prepare(key, state.doc.id, owner.quota(), c)?),
        };
        let revision = state.revision.checked_add(1).ok_or(BackingError::Length)?;
        c.check(1)?;
        state.doc = snapshot;
        state.history = history;
        state.accounted_view = Some(next_view);
        state.revision = revision;
        state.last_damage = None;
        state.coalesce = None;
        state._accounted_metadata_gesture = gesture;
        Ok(())
    }
}
#[cfg(test)]
#[path = "accounted_metadata_gesture_tests.rs"]
mod tests;

#[cfg(all(test,feature="corpus",not(target_arch="wasm32")))]
#[path="accounted_gaussian_group_metadata_tests.rs"]
mod gaussian_group_tests;
