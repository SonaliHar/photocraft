//! Typed pixel-selection publication for a charged native document.
//! The incoming owner carries its real storage quota; JSON cannot supply it.
use crate::{EngineError, Result, Session};
use photocraft_doc::{DocId, accounted_selection::AccountedSelection};
use photocraft_raster::backing::{BackingError, ReadControl};

/// A worker or host captures this before preparing a private selection.
#[derive(Clone, Debug)]
pub struct AccountedSelectionTarget {
    pub document: DocId,
    pub revision: u64,
    pub view_revision: u64,
    snapshot: crate::WeakDocumentSnapshot,
}
impl AccountedSelectionTarget {
    pub fn capture(session: &Session) -> Result<Self> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        if state.doc.accounted().is_none() || state.doc.id.0 == 0 {
            return Err(BackingError::Unsupported.into());
        }
        Ok(Self { document: state.doc.id, revision: state.revision, view_revision: state.view_revision, snapshot: state.doc.downgrade() })
    }
}
impl Session {
    /// One staged history transaction; failures and proven no-ops preserve redo.
    /// Layer focus is restored independently, without detached charged vectors.
    pub fn replace_accounted_selection(
        &mut self,
        target: AccountedSelectionTarget,
        label: &str,
        selection: Option<&AccountedSelection>,
        control: &ReadControl<'_>,
    ) -> Result<()> {
        control.check(1)?;
        if self.active_job().is_some()
            || self.command_has_job_on(target.document)
            || self.command_has_pending_job()
            || self.coalesce_request.is_some()
            || self.color_restrict.is_some()
        {
            return Err(EngineError::Other("the accounted selection target is not idle".into()));
        }
        let state = self.active_mut().ok_or(EngineError::NoDocument)?;
        if state.doc.id != target.document
            || state.revision != target.revision
            || state.view_revision != target.view_revision
            || !target.snapshot.is_current(&state.doc)
        {
            return Err(EngineError::Other("the prepared accounted selection targets an older document or view".into()));
        }
        if state.floating.is_some() || state.coalesce.is_some() {
            return Err(EngineError::Other("the accounted selection target is not idle".into()));
        }
        let view = state.accounted_view.as_ref().ok_or(BackingError::Unsupported)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        let mut history = state.history.clone();
        history.set_current_layers_borrowed(view.target(), control)?;
        let snapshot = history.try_commit_accounted_selection(selection, label, view.target(), control)?;
        if snapshot.shares_state(&state.doc) {
            control.check(1)?;
            return Ok(());
        }
        let current = snapshot.accounted().ok_or(BackingError::Integrity)?;
        let next_view = view.try_replace(current, state.active_layer, view.selected(), state.layer_anchor, control)?;
        let revision = state.revision.checked_add(1).ok_or(BackingError::Length)?;
        control.check(1)?;
        state.doc = snapshot;
        state.history = history;
        state.accounted_view = Some(next_view);
        state.revision = revision;
        state.last_damage = None;
        state.coalesce = None;
        state._accounted_metadata_gesture = None;
        Ok(())
    }
}
