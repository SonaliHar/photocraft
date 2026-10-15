//! Engine publication fences for explicitly owned native models and history.
use crate::{
    DocState, EngineError, Result, Session,
    accounted_view::AccountedView,
    document_history::DocumentHistory,
    document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot},
};
use photocraft_doc::{
    accounted_document::{AccountedDocument, AccountedEdit, accounted_layer_access},
    item_identity::ItemRef,
};
use photocraft_raster::backing::{BackingError, ReadControl, quota::QuotaKind};
use std::sync::{Arc, Mutex};

#[path = "accounted_assigned_fence.rs"]
pub(crate) mod assigned_fence;
#[path = "accounted_metadata_gesture.rs"]
pub(crate) mod metadata_gesture;

#[path = "accounted_native_alpha.rs"]
mod native_alpha;
use native_alpha::VerifiedNativeAlpha;

impl DocState {
    pub(crate) fn new_accounted(document: AccountedDocument, path: Option<String>, history_states: usize, control: &ReadControl<'_>) -> Result<Self> {
        control.check(1)?;
        // Channel/mask/floating focus still needs its own qualified view owner.
        if !document.document().channels.is_empty() {
            return Err(BackingError::Unsupported.into());
        }
        let path_bytes = path.as_ref().map_or(0, String::capacity);
        if path.as_ref().is_some_and(|p| p.len() > 4096 || p.capacity() > 8192 || p.contains('\0')) {
            return Err(BackingError::Length.into());
        }
        let bytes = std::mem::size_of::<Self>().checked_add(path_bytes).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
        let metadata = document.quota().reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
        let view = initial_view(&document, control)?;
        let active = view.target().active;
        let history = DocumentHistory::new_accounted(document.clone(), history_states, "Open", view.target(), control)?;
        let snapshot = DocumentSnapshot::from_accounted(document)?;
        control.check(1)?;
        Ok(Self {
            doc: snapshot,
            surface_validation: Arc::new(Mutex::new(WeakDocumentSnapshot::new())),
            history,
            active_layer: active,
            selected_layers: Vec::new(),
            layer_anchor: active,
            path,
            source: None,
            source_read_only: false,
            revision: 1,
            view_revision: 0,
            saved_revision: 1,
            last_damage: None,
            coalesce: None,
            channel_view: Default::default(),
            isolated_layers: Vec::new(),
            symmetry: None,
            fx_collapsed: Vec::new(),
            show_only: None,
            floating: None,
            color_samplers: Vec::new(),
            snapshots: Vec::new(),
            next_snapshot: 1,
            accounted_view: Some(view),
            _accounted_metadata: Some(metadata),
            _accounted_save_identity: None,
            _accounted_metadata_gesture: None,
        })
    }
}

/// Restores layer focus from inert canonical state, without changing that state
/// or granting authority to its stored source/resource locators. The conversion
/// buffer and final view are separately paid while both allocations coexist.
fn initial_view(document: &AccountedDocument, control: &ReadControl<'_>) -> Result<AccountedView> {
    let doc = document.document();
    let Some(state) = &doc.xcf_state else {
        let active = doc.top_layer();
        return Ok(AccountedView::new(document, active, active.as_slice(), active, control)?);
    };
    if state.floating.is_some() || state.selection.mask_target.is_some() || !state.selection.selected_paths.is_empty() {
        return Err(BackingError::Unsupported.into());
    }
    let stored_active = match state.selection.active {
        Some(ItemRef::Layer(id)) => Some(id),
        None => None,
        Some(_) => return Err(BackingError::Unsupported.into()),
    };
    let count = state.selection.selected.len().max(1);
    if count > 4096 {
        return Err(BackingError::Length.into());
    }
    let bytes = count.checked_mul(std::mem::size_of::<photocraft_doc::LayerId>()).and_then(|n| n.checked_mul(2)).ok_or(BackingError::Length)?;
    let mut lease = document.quota().reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
    let mut selected = Vec::new();
    selected.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
    lease
        .reconcile_capacity(selected.capacity().checked_mul(std::mem::size_of::<photocraft_doc::LayerId>()).ok_or(BackingError::Length)? as u64)
        .map_err(|_| BackingError::Allocation)?;
    for (position, item) in state.selection.selected.iter().enumerate() {
        if position % 64 == 0 {
            control.check(64)?;
        }
        let ItemRef::Layer(id) = *item else { return Err(BackingError::Unsupported.into()) };
        selected.push(id);
    }
    if stored_active.is_some_and(|id| !selected.contains(&id)) {
        return Err(BackingError::Integrity.into());
    }
    let active = stored_active.or_else(|| selected.first().copied()).or_else(|| doc.top_layer());
    if selected.is_empty()
        && let Some(id) = active
    {
        selected.push(id);
    }
    Ok(AccountedView::new(document, active, &selected, active, control)?)
}

fn edit_locks(document: &AccountedDocument, edit: AccountedEdit<'_>, alpha: Option<&VerifiedNativeAlpha<'_>>, control: &ReadControl<'_>) -> Result<()> {
    let id = edit.layer_id();
    let doc = document.document();
    let access = accounted_layer_access(doc, id, control)?.ok_or(BackingError::Bounds)?;
    let layer = access.layer;
    let imported = access.imported_locks;
    match edit {
        AccountedEdit::LayerVisibility { visible, .. } if visible != layer.visible && imported.visibility => {
            Err(EngineError::Other("the imported layer is visibility-locked".into()))
        }
        AccountedEdit::LayerMetadata { visible: Some(visible), .. } if visible != layer.visible && imported.visibility => {
            Err(EngineError::Other("the imported layer is visibility-locked".into()))
        }
        AccountedEdit::NativePixels { .. }
            if access.locks.pixels
                || access.locks.all
                || imported.content
                || (access.locks.transparency || imported.alpha) && !alpha.is_some_and(|proof| proof.authorizes(document, edit)) =>
        {
            Err(EngineError::Other("native pixel replacement cannot change a content- or alpha-locked layer".into()))
        }
        _ => Ok(()),
    }
}

impl Session {
    /// The actual ledger retaining this session's document catalog. Open
    /// services must reuse it even when the active tab is resident or the last
    /// accounted document has closed; those catalog slots remain charged.
    pub fn accounted_open_quota(&self) -> Option<&photocraft_raster::backing::quota::BackingQuota> {
        self.accounted_slots.as_ref().map(|(quota, _)| quota)
    }

    /// Stages the document slots before moving any existing state. Allocation,
    /// quota and cancellation failures preserve the old active tab and owners.
    pub(crate) fn publish_document_state(&mut self, state: DocState, control: &ReadControl<'_>) -> Result<usize> {
        control.check(1)?;
        let incoming_quota = state.doc.quota().cloned();
        let current_quota = self.accounted_slots.as_ref().map(|(q, _)| q);
        if let (Some(incoming), Some(current)) = (&incoming_quota, current_quota)
            && !incoming.shares_ledger(current)
        {
            return Err(BackingError::Quota.into());
        }
        let quota = incoming_quota.or_else(|| current_quota.cloned());
        let index = self.docs.len();
        let count = index.checked_add(1).ok_or(BackingError::Length)?;
        if let Some(quota) = quota {
            if count > 4096 {
                return Err(BackingError::Length.into());
            }
            let requested = count
                .checked_mul(std::mem::size_of::<DocState>())
                .and_then(|n| n.checked_mul(2))
                .and_then(|n| n.checked_add(128))
                .ok_or(BackingError::Length)?;
            let mut lease = quota.reserve(QuotaKind::Index, requested as u64).map_err(|_| BackingError::Quota)?;
            let mut staged = Vec::new();
            staged.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
            let actual = staged.capacity().checked_mul(std::mem::size_of::<DocState>()).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
            lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
            control.check(count as u64)?;
            staged.append(&mut self.docs);
            staged.push(state);
            self.docs = staged; // drops the old slot allocation before its charge
            self.accounted_slots = Some((quota, lease));
        } else {
            self.docs.try_reserve(1).map_err(|_| BackingError::Allocation)?;
            control.check(1)?;
            self.docs.push(state);
        }
        self.active = Some(index);
        Ok(index)
    }

    pub fn add_accounted_document(&mut self, document: AccountedDocument, path: Option<String>, control: &ReadControl<'_>) -> Result<usize> {
        control.check(1)?;
        // A structural Point owner is not a current semantic/render receipt.
        // Only the typed source/native handoffs validate its operative recipe.
        if document.is_readonly_point_storage() || document.is_readonly_gaussian_group_storage()
            || crate::accounted_native_open::is_current_isolated_gaussian_group_candidate(document.document(),control)?
            || crate::accounted_native_open::is_current_isolated_normal_group_candidate(document.document(), control)? {
            return Err(BackingError::Unsupported.into());
        }
        if self.docs.iter().any(|st| st.doc.id == document.document().id) {
            return Err(EngineError::Other("an open document already owns this accounted identity".into()));
        }
        let history_states = self.prefs.get().performance.history_states.max(1) as usize;
        let state = DocState::new_accounted(document, path, history_states, control)?;
        self.publish_document_state(state, control)
    }

    /// Typed edits retain the quota and source owner. No mutable model callback
    /// can escape into this branch or mutate a model before history is prepared.
    pub fn edit_accounted(&mut self, label: &str, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<()> {
        self.edit_accounted_inner(label, edit, NativeAlphaCheck::OrdinaryLocks, control)
    }

    /// Fill-only raw-alpha preservation. The active genuine imported owner is
    /// independently checked here; a caller's preserve flag is not a proof.
    /// Content/all/pixel locks remain mandatory and ordinary NativePixels edits
    /// retain their existing transparency-lock refusal.
    pub(crate) fn edit_accounted_preserving_native_alpha(
        &mut self,
        label: &str,
        layer: photocraft_doc::LayerId,
        rect: photocraft_geom::Rect,
        bytes: &[u8],
        control: &ReadControl<'_>,
    ) -> Result<()> {
        self.edit_accounted_inner(label, AccountedEdit::NativePixels { layer, rect, bytes }, NativeAlphaCheck::VerifyRawPreservation, control)
    }

    fn edit_accounted_inner(&mut self, label: &str, edit: AccountedEdit<'_>, alpha_check: NativeAlphaCheck, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        if let AccountedEdit::MaskEnabled { layer, enabled } = edit
            && self.active().and_then(|state| state.doc.accounted()).is_some_and(metadata_gesture::assigned_geometry_metadata_candidate)
        {
            return self.edit_accounted_assigned_mask_enabled(label, layer, enabled, control).map(|_| ());
        }
        if matches!(edit, AccountedEdit::LayerMetadata { .. })
            && self.active().and_then(|state| state.doc.accounted()).is_some_and(metadata_gesture::assigned_geometry_metadata_candidate)
        {
            return self.edit_accounted_metadata_with_coalesce(label, edit, None, control);
        }
        if self.active_job().is_some() {
            return Err(EngineError::Other("the document has an active background job".into()));
        }
        let state = self.active_mut().ok_or(EngineError::NoDocument)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Unsupported)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        let document = state.doc.accounted().ok_or(BackingError::Integrity)?;
        let alpha = match alpha_check {
            NativeAlphaCheck::OrdinaryLocks => None,
            NativeAlphaCheck::VerifyRawPreservation => {
                if !state.history.current()?.shares_state(&state.doc) {
                    return Err(BackingError::Integrity.into());
                }
                Some(VerifiedNativeAlpha::verify(document, edit, control)?)
            }
        };
        edit_locks(document, edit, alpha.as_ref(), control)?;
        let mut history = state.history.clone();
        history.set_current_layers_borrowed(view.target(), control)?;
        let snapshot = history.try_commit_accounted(edit, label, view.target(), control)?;
        if snapshot.shares_state(&state.doc) {
            control.check(1)?;
            return Ok(());
        }
        let current = snapshot.accounted().ok_or(BackingError::Integrity)?;
        let selection = view.try_replace(current, state.active_layer, view.selected(), state.layer_anchor, control)?;
        let revision = state.revision.checked_add(1).ok_or(BackingError::Length)?;
        control.check(1)?;
        drop(alpha);
        state.doc = snapshot;
        state.history = history;
        state.accounted_view = Some(selection);
        state.revision = revision;
        state.last_damage = None;
        state.coalesce = None;
        state._accounted_metadata_gesture = None;
        Ok(())
    }
}

enum NativeAlphaCheck {
    OrdinaryLocks,
    VerifyRawPreservation,
}
