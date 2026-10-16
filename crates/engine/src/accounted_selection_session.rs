//! Typed pixel-selection publication for a charged native document.
//! The incoming owner carries its real storage quota; JSON cannot supply it.
use crate::{EngineError, Result, Session};
use photocraft_doc::{DocId, accounted_selection::AccountedSelection};
use photocraft_raster::backing::{BackingError, ReadControl};

/// A worker or host captures this before preparing a private selection.
#[derive(Clone)]
pub struct AccountedSelectionTarget {
    pub document: DocId,
    pub revision: u64,
    pub view_revision: u64,
    snapshot: crate::WeakDocumentSnapshot,
    programme: Option<std::sync::Arc<programme::ProgrammeSelectionCapture>>,
}
impl AccountedSelectionTarget {
    /// Capture a paid immutable programme selection context using the original operation control.
    pub fn capture_with_control(session:&Session,c:&ReadControl<'_>)->Result<Self>{
        let mut target=Self::capture(session)?;
        let owner=session.active().and_then(|s|s.doc.accounted()).ok_or(BackingError::Unsupported)?;
        if crate::accounted_programme_current::kind(owner).is_some(){
            target.programme=Some(std::sync::Arc::new(programme::ProgrammeSelectionCapture::prepare(session,c)?));
        }
        Ok(target)
    }
    pub fn validate_with_control(&self,session:&Session,c:&ReadControl<'_>)->Result<()>{
        c.check(1)?;
        let state=session.active().ok_or(EngineError::NoDocument)?;
        if state.doc.id!=self.document||state.revision!=self.revision||state.view_revision!=self.view_revision||!self.snapshot.is_current(&state.doc){return Err(BackingError::Integrity.into())}
        if let Some(capture)=&self.programme{capture.validate(session,c)?;}else if state.doc.accounted().is_some_and(|o|crate::accounted_programme_current::kind(o).is_some()){return Err(BackingError::Unsupported.into())}
        Ok(())
    }
    pub fn capture(session: &Session) -> Result<Self> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        if state.doc.accounted().is_none() || state.doc.id.0 == 0 {
            return Err(BackingError::Unsupported.into());
        }
        Ok(Self { document: state.doc.id, revision: state.revision, view_revision: state.view_revision, snapshot: state.doc.downgrade(), programme: None })
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
        if target.programme.is_some()||self.active().and_then(|s|s.doc.accounted()).is_some_and(|o|crate::accounted_programme_current::kind(o).is_some()){target.validate_with_control(self,control)?;}
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
        let snapshot = if target.programme.is_some(){
            let owner=state.doc.accounted().ok_or(BackingError::Integrity)?;
            let kind=crate::accounted_programme_current::kind(owner).ok_or(BackingError::Unsupported)?;
            crate::accounted_programme_current::validate(owner,kind,control)?;
            let staged=owner.try_replace_programme_selection(selection,control)?;
            crate::accounted_programme_current::validate(&staged,kind,control)?;
            history.try_commit_prepared_accounted(owner,staged,label,view.target(),control)?
        }else{history.try_commit_accounted_selection(selection, label, view.target(), control)?};
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

impl std::fmt::Debug for AccountedSelectionTarget{
 fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("AccountedSelectionTarget").field("document",&self.document).field("revision",&self.revision).field("view_revision",&self.view_revision).finish_non_exhaustive()}
}
#[path="accounted_programme_selection.rs"]
mod programme;

#[cfg(all(test,feature="corpus",not(target_arch="wasm32")))]
#[path="accounted_programme_selection_tests.rs"]
mod tests;
