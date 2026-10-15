//! Additional paid preparation fences only for assigned masked metadata.
use super::*;
use crate::{DocumentSnapshot, document_history::DocumentHistory, source_policy::SourceIdentity};
use photocraft_raster::backing::quota::QuotaLease;

pub(crate) struct AssignedFence {
    document: DocumentSnapshot,
    history: DocumentHistory,
    path: Option<String>,
    source: Option<SourceIdentity>,
    read_only: bool,
    history_states: u32,
    history_bytes: usize,
    _lease: QuotaLease,
}
fn strings(state: &crate::DocState) -> Result<usize> {
    let mut total = 0usize;
    for text in state.path.iter().chain(state.source.iter().flat_map(|source| std::iter::once(&source.label).chain(source.path.iter()))) {
        if text.len() > 4096 {
            return Err(BackingError::Length.into());
        }
        total = total.checked_add(text.len()).ok_or(BackingError::Length)?;
    }
    Ok(total)
}
fn copy(text: &str, control: &ReadControl<'_>) -> Result<String> {
    control.check(text.len() as u64)?;
    let mut result = String::new();
    result.try_reserve_exact(text.len()).map_err(|_| BackingError::Allocation)?;
    if result.capacity() > text.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    result.push_str(text);
    Ok(result)
}
impl AssignedFence {
    pub(crate) fn prepare<T>(session: &Session, control: &ReadControl<'_>) -> Result<Self> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Integrity)?;
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        let count = strings(state)?;
        let header = std::mem::size_of::<Self>().checked_add(std::mem::size_of::<T>()).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
        let envelope = count.checked_mul(2).and_then(|n| n.checked_add(header)).ok_or(BackingError::Length)?;
        let mut lease = owner.quota().reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
        let path = state.path.as_deref().map(|text| copy(text, control)).transpose()?;
        let source = state
            .source
            .as_ref()
            .map(|source| {
                Ok::<_, EngineError>(SourceIdentity {
                    label: copy(&source.label, control)?,
                    path: source.path.as_deref().map(|text| copy(text, control)).transpose()?,
                    origin: source.origin,
                })
            })
            .transpose()?;
        let actual = path
            .as_ref()
            .map_or(0, String::capacity)
            .checked_add(source.as_ref().map_or(0, |source| source.label.capacity() + source.path.as_ref().map_or(0, String::capacity)))
            .and_then(|n| n.checked_add(header))
            .ok_or(BackingError::Length)?;
        if actual > envelope {
            return Err(BackingError::Allocation.into());
        }
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;
        let result = Self {
            document: state.doc.clone(),
            history: state.history.clone(),
            path,
            source,
            read_only: state.source_read_only,
            history_states: session.prefs().performance.history_states,
            history_bytes: session.prefs().performance.history_budget_bytes(),
            _lease: lease,
        };
        control.check(1)?;
        Ok(result)
    }
    pub(crate) fn check(&self, session: &Session, control: &ReadControl<'_>) -> Result<()> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let count = strings(state)?;
        let retained = self
            .path
            .as_ref()
            .map_or(0, String::len)
            .checked_add(self.source.as_ref().map_or(0, |source| source.label.len() + source.path.as_ref().map_or(0, String::len)))
            .ok_or(BackingError::Length)?;
        control.check(count.checked_add(retained).and_then(|n| n.checked_add(1)).ok_or(BackingError::Length)? as u64)?;
        if !state.doc.shares_state(&self.document)
            || !state.history.current()?.shares_state(&state.doc)
            || !state.history.shares_accounted_history(&self.history)
            || state.history.max_states != self.history.max_states
            || state.history.max_bytes != self.history.max_bytes
            || state.path != self.path
            || state.source != self.source
            || state.source_read_only != self.read_only
            || session.prefs().performance.history_states != self.history_states
            || session.prefs().performance.history_budget_bytes() != self.history_bytes
        {
            return Err(EngineError::Other("the prepared assigned metadata targets older source, history or preferences".into()));
        }
        control.check(1)?;
        Ok(())
    }
}
