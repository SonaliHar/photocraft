//! A prepaid command journal owned across a worker and its typed publication.
//! This carries no borrowed request and grants no document-edit authority.
use super::*;

pub(crate) struct PreparedAccountedJobJournal {
    snapshot: crate::DocumentSnapshot,
    revision: u64,
    view_revision: u64,
    journal: Vec<(String, Value)>,
    lease: QuotaLease,
    overlap: QuotaLease,
    quota: BackingQuota,
}
impl PreparedAccountedCommand<'_> {
    pub(crate) fn into_job_journal(self) -> PreparedAccountedJobJournal {
        let Self { request: _, snapshot, revision, view_revision, journal, lease, overlap, quota } = self;
        PreparedAccountedJobJournal { snapshot, revision, view_revision, journal, lease, overlap, quota }
    }
}
impl PreparedAccountedJobJournal {
    /// Called before the worker's final typed edit. All checks remain fallible
    /// here; publish() below performs only the already-paid journal replacement.
    pub(crate) fn validate(&self, session: &Session, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        if let Some(reason) = global_refusal(session) {
            return Err(EngineError::Other(reason.into()));
        }
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let quota = state.doc.quota().ok_or(BackingError::Unsupported)?;
        if !quota.shares_ledger(&self.quota) {
            return Err(BackingError::Quota.into());
        }
        if state.revision != self.revision || state.view_revision != self.view_revision || !state.doc.shares_state(&self.snapshot) {
            return Err(EngineError::Other("the prepared accounted job targets an older document or view".into()));
        }
        let previous = self.journal.get(..self.journal.len().checked_sub(1).ok_or(BackingError::Integrity)?).ok_or(BackingError::Integrity)?;
        control.check(MAX_PAYLOAD as u64)?;
        if previous != session.journal.as_slice() {
            return Err(EngineError::Other("the command journal changed while the accounted job ran".into()));
        }
        control.check(1)?;
        Ok(())
    }

    pub(crate) fn publish(self, session: &mut Session, result: Value) -> Value {
        AccountedCommandOutcome { journal: self.journal, lease: self.lease, overlap: self.overlap, result }.publish(session)
    }
}

#[cfg(test)]
#[path = "job_journal_tests.rs"]
mod tests;
