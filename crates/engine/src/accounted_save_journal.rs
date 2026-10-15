//! Exact paid journal fence for synchronous save completion; never publishes a command.
use super::*;
// The common copier polls per Value; its String push_str itself does not debit
// bytes. Admit those actual bounded payload bytes and known slot copies before
// any destination allocation, independently of later comparison admission.
fn admit_copy(census: &Census, entries: usize, extra: usize, c: &ReadControl<'_>) -> Result<()> {
    let work = census
        .nodes
        .checked_mul(std::mem::size_of::<Value>())
        .and_then(|n| entries.checked_mul(std::mem::size_of::<(String, Value)>()).and_then(|v| n.checked_add(v)))
        .and_then(|n| n.checked_add(census.payload))
        .and_then(|n| n.checked_add(extra))
        .ok_or(BackingError::Length)?;
    c.check(u64::try_from(work).map_err(|_| BackingError::Length)?)?;
    Ok(())
}
pub(crate) struct PreparedSaveJournalWitness {
    journal: Vec<(String, Value)>,
    _lease: QuotaLease,
}
impl PreparedSaveJournalWitness {
    pub(crate) fn capture(session: &Session, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<Self> {
        c.check(1)?;
        if session.journal.len() > MAX_RECORDS || session.journal.capacity() > MAX_RECORDS * 2 {
            return Err(BackingError::Length.into());
        }
        let mut census = Census::default();
        census.add(
            0,
            session
                .journal
                .len()
                .checked_mul(std::mem::size_of::<(String, Value)>())
                .and_then(|n| n.checked_mul(2))
                .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 128))
                .ok_or(BackingError::Length)?,
        )?;
        for (label, value) in &session.journal {
            census.string(label)?;
            census.command_value(label, value, c)?;
        }
        admit_copy(&census, session.journal.len(), 0, c)?;
        let lease = quota.reserve(QuotaKind::Index, census.envelope).map_err(|_| BackingError::Quota)?;
        let mut journal = Vec::new();
        journal.try_reserve_exact(session.journal.len()).map_err(|_| BackingError::Allocation)?;
        if journal.capacity() > session.journal.len().checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation.into());
        }
        for (label, value) in &session.journal {
            journal.push((copied_string(label)?, copied_value(value, c)?));
        }
        c.check(1)?;
        Ok(Self { journal, _lease: lease })
    }
    pub(crate) fn validate(&self, session: &Session, c: &ReadControl<'_>) -> Result<()> {
        c.check(MAX_PAYLOAD as u64)?;
        if self.journal != session.journal {
            return Err(EngineError::Other("the journal changed after native save preparation".into()));
        }
        Ok(())
    }
}

/// Known native-save reply and optional exact user save record, prepared under
/// the same accounted quota. No record is synthesized for direct/automation IO.
pub struct PreparedAccountedSaveReply {
    journal: Vec<(String, Value)>,
    result: Value,
    snapshot: crate::DocumentSnapshot,
    revision: u64,
    view_revision: u64,
    old_len: usize,
    lease: QuotaLease,
    overlap: QuotaLease,
}
impl PreparedAccountedSaveReply {
    pub fn prepare(session: &Session, path: &str, record: Option<(&str, &Value)>, c: &ReadControl<'_>) -> Result<Self> {
        c.check(1)?;
        if path.len() > MAX_STRING || session.journal.len() > MAX_RECORDS || session.journal.capacity() > MAX_RECORDS * 2 {
            return Err(BackingError::Length.into());
        }
        c.check(path.len() as u64)?;
        if path.contains('\0') {
            return Err(BackingError::Length.into());
        }
        if let Some((id, _)) = record {
            if !matches!(id, "file.save" | "file.saveAs") {
                return Err(BackingError::Unsupported.into());
            }
        }
        if let Some((id, params)) = record {
            let object = params.as_object().ok_or(BackingError::Unsupported)?;
            let valid = if id == "file.save" { object.is_empty() } else { object.len() == 1 && object.get("path").and_then(Value::as_str) == Some(path) };
            if !valid {
                return Err(BackingError::Unsupported.into());
            }
        }
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let quota = state.doc.quota().ok_or(BackingError::Unsupported)?;
        let count = session.journal.len().checked_add(usize::from(record.is_some())).ok_or(BackingError::Length)?;
        if count > MAX_RECORDS {
            return Err(BackingError::Length.into());
        }
        let mut old = Census::default();
        old.add(
            0,
            session.journal.capacity().checked_mul(std::mem::size_of::<(String, Value)>()).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?,
        )?;
        let mut census = Census::default();
        census.add(
            0,
            count
                .checked_mul(std::mem::size_of::<(String, Value)>())
                .and_then(|n| n.checked_mul(2))
                .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 128))
                .ok_or(BackingError::Length)?,
        )?;
        for (label, value) in &session.journal {
            old.retained_string(label)?;
            old.retained_command_value(label, value, c)?;
            census.string(label)?;
            census.command_value(label, value, c)?;
        }
        if let Some((id, params)) = record {
            census.string(id)?;
            census.command_value(id, params, c)?;
        }
        // Two fixed map entries plus empty warnings vector and a bounded path String;
        // same standard serde map allocator boundary as existing command journals.
        census.string(path)?;
        census.add(0, 8192)?;
        let reply_work = std::mem::size_of::<Value>().checked_mul(2).and_then(|n| n.checked_add(12)).ok_or(BackingError::Length)?;
        admit_copy(&census, count, reply_work, c)?;
        let overlap = quota.reserve(QuotaKind::Index, old.envelope).map_err(|_| BackingError::Quota)?;
        let lease = quota.reserve(QuotaKind::Index, census.envelope).map_err(|_| BackingError::Quota)?;
        let mut journal = Vec::new();
        journal.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
        if journal.capacity() > count.checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation.into());
        }
        for (label, value) in &session.journal {
            journal.push((copied_string(label)?, copied_value(value, c)?));
        }
        if let Some((id, params)) = record {
            journal.push((copied_string(id)?, copied_value(params, c)?));
        }
        let mut response = serde_json::Map::new();
        response.insert(copied_string("path")?, Value::String(copied_string(path)?));
        response.insert(copied_string("warnings")?, Value::Array(Vec::new()));
        c.check(1)?;
        Ok(Self {
            journal,
            result: Value::Object(response),
            snapshot: state.doc.clone(),
            revision: state.revision,
            view_revision: state.view_revision,
            old_len: session.journal.len(),
            lease,
            overlap,
        })
    }
    pub fn validate(&self, session: &Session, c: &ReadControl<'_>) -> Result<()> {
        let state = session.active().ok_or(EngineError::NoDocument)?;
        c.check(MAX_PAYLOAD as u64)?;
        if state.revision != self.revision
            || state.view_revision != self.view_revision
            || !state.doc.shares_state(&self.snapshot)
            || self.journal.get(..self.old_len) != Some(session.journal.as_slice())
        {
            return Err(EngineError::Other("native-save.stale-reply".into()));
        }
        Ok(())
    }
    /// Called only following the matching successful typed save completion. Moves
    /// the paid journal/reply; no poll or fallible bookkeeping follows the write.
    pub fn publish(self, session: &mut Session) -> Value {
        let Self { journal, result, lease, overlap, .. } = self;
        AccountedCommandOutcome { journal, lease, overlap, result }.publish(session)
    }
}
