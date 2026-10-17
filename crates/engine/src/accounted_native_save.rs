//! Prepared file identity for the closed imported accounted class. The writer
//! is a host-authorized synchronous boundary, not path or source authority.
//! All fallible preparation and final fences precede it. Once it reports a
//! committed write, publication performs only moves and no cancellation poll.
use crate::{
    DocumentSnapshot, EngineError, Result, Session,
    accounted_commands::PreparedSaveJournalWitness,
    accounted_view::AccountedView,
    channel_cmds::{ChannelTarget, ChannelView},
    document_history::DocumentHistory,
    source_policy::SourceIdentity,
};
use photocraft_doc::LayerId;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{QuotaKind, QuotaLease},
};
fn copied(text: &str, c: &ReadControl<'_>) -> Result<String> {
    let mut value = String::new();
    value.try_reserve_exact(text.len()).map_err(|_| BackingError::Allocation)?;
    if value.capacity() > text.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for ch in text.chars() {
        c.check(ch.len_utf8() as u64)?;
        value.push(ch);
    }
    Ok(value)
}
fn flags(input: &[bool], c: &ReadControl<'_>) -> Result<Vec<bool>> {
    let mut output = Vec::new();
    output.try_reserve_exact(input.len()).map_err(|_| BackingError::Allocation)?;
    if output.capacity() > input.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for value in input {
        c.check(1)?;
        output.push(*value);
    }
    Ok(output)
}
fn text_shape(value: &str, c: &ReadControl<'_>) -> Result<()> {
    if value.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    c.check(value.len() as u64)?;
    if value.contains('\0') {
        return Err(BackingError::Length.into());
    }
    Ok(())
}
fn view_shape(view: &ChannelView) -> Result<()> {
    if view.layer_mask.is_some() || view.editing_mask.is_some() || !matches!(view.target, ChannelTarget::Composite | ChannelTarget::Color(_)) {
        return Err(BackingError::Unsupported.into());
    }
    if view.color_hidden.len() > 32 || view.alpha_visible.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    Ok(())
}
/// Move-only completion. Its output snapshot must be the one passed to the
/// actual writer; no raw Document clone or mutable callback is exposed.
pub struct PreparedAccountedNativeSave {
    snapshot: DocumentSnapshot,
    history: DocumentHistory,
    view: AccountedView,
    path: String,
    captured: DocumentSnapshot,
    old_history: DocumentHistory,
    old_view: AccountedView,
    revision: u64,
    view_revision: u64,
    active: Option<LayerId>,
    anchor: Option<LayerId>,
    old_path: Option<String>,
    source: Option<SourceIdentity>,
    source_read_only: bool,
    channel: ChannelView,
    journal: PreparedSaveJournalWitness,
    path_lease: QuotaLease,
    _temporary: QuotaLease,
}
impl PreparedAccountedNativeSave {
    pub fn prepare(session: &Session, path: &str, c: &ReadControl<'_>) -> Result<Self> {
        c.check(1)?;
        text_shape(path, c)?;
        if !std::path::Path::new(path).extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("pcraft")) {
            return Err(BackingError::Unsupported.into());
        }
        if session.has_jobs() {
            return Err(EngineError::Other("native accounted save requires jobs to finish".into()));
        }
        let state = session.active().ok_or(EngineError::NoDocument)?;
        if state.floating.is_some() || !state.selected_layers.is_empty() {
            return Err(BackingError::Unsupported.into());
        }
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        if state.doc.xcf_state.is_none() || state.doc.selection_channel_id.is_some() {
            return Err(BackingError::Unsupported.into());
        }
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() {
            return Err(BackingError::Integrity.into());
        }
        view_shape(&state.channel_view)?;
        if matches!(state.channel_view.target,ChannelTarget::Color(i) if i>=state.doc.mode.color_channels()) {
            return Err(BackingError::Bounds.into());
        }
        if let Some(value) = &state.path {
            text_shape(value, c)?;
        }
        if let Some(source) = &state.source {
            text_shape(&source.label, c)?;
            if let Some(value) = &source.path {
                text_shape(value, c)?;
            }
        }
        let strings = state
            .path
            .as_ref()
            .map_or(0, String::len)
            .checked_add(state.source.as_ref().map_or(0, |s| s.label.len() + s.path.as_ref().map_or(0, String::len)))
            .ok_or(BackingError::Length)?;
        let bytes = strings
            .checked_add(state.channel_view.color_hidden.len())
            .and_then(|n| n.checked_add(state.channel_view.alpha_visible.len()))
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 512))
            .ok_or(BackingError::Length)?;
        let temporary = owner.quota().reserve(QuotaKind::Index, bytes as u64).map_err(|_| BackingError::Quota)?;
        let path_bytes = path.len().checked_mul(2).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
        let mut path_lease = owner.quota().reserve(QuotaKind::Index, path_bytes as u64).map_err(|_| BackingError::Quota)?;
        let owned_path = copied(path, c)?;
        path_lease.reconcile_capacity((owned_path.capacity() + 128) as u64).map_err(|_| BackingError::Allocation)?;
        let old_path = state.path.as_deref().map(|v| copied(v, c)).transpose()?;
        let source = state
            .source
            .as_ref()
            .map(|s| {
                Ok::<_, EngineError>(SourceIdentity {
                    label: copied(&s.label, c)?,
                    path: s.path.as_deref().map(|v| copied(v, c)).transpose()?,
                    origin: s.origin,
                })
            })
            .transpose()?;
        let channel = ChannelView {
            target: state.channel_view.target,
            color_hidden: flags(&state.channel_view.color_hidden, c)?,
            alpha_visible: flags(&state.channel_view.alpha_visible, c)?,
            quick_mask_hidden: state.channel_view.quick_mask_hidden,
            layer_mask: None,
            editing_mask: None,
        };
        let journal = PreparedSaveJournalWitness::capture(session, owner.quota(), c)?;
        let name = path.rsplit(['/', '\\']).next().ok_or(BackingError::Bounds)?;
        let prepared = owner.try_with_document_name(name, c)?;
        let mut history = state.history.clone();
        let snapshot = history.try_replace_accounted_current(owner, prepared, c)?;
        let next_owner = snapshot.accounted().ok_or(BackingError::Integrity)?;
        let next_view = view.try_replace(next_owner, state.active_layer, view.selected(), state.layer_anchor, c)?;
        c.check(1)?;
        Ok(Self {
            snapshot,
            history,
            view: next_view,
            path: owned_path,
            captured: state.doc.clone(),
            old_history: state.history.clone(),
            old_view: view.clone(),
            revision: state.revision,
            view_revision: state.view_revision,
            active: state.active_layer,
            anchor: state.layer_anchor,
            old_path,
            source,
            source_read_only: state.source_read_only,
            channel,
            journal,
            path_lease,
            _temporary: temporary,
        })
    }
    pub fn snapshot(&self) -> &DocumentSnapshot {
        &self.snapshot
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    /// The closure must return Ok only once its protected atomic write commits.
    /// It cannot access this exclusively borrowed Session. Errors leave Session
    /// identity/history untouched; external-file guarantees belong to the writer.
    pub fn write_and_publish<R>(self, session: &mut Session, c: &ReadControl<'_>, write: impl FnOnce(&DocumentSnapshot, &str) -> Result<R>) -> Result<R> {
        c.check(1)?;
        if session.has_jobs() {
            return Err(BackingError::Unsupported.into());
        }
        self.journal.validate(session, c)?;
        let state = session.active_mut().ok_or(EngineError::NoDocument)?;
        view_shape(&state.channel_view)?;
        c.check((self.channel.color_hidden.len() + self.channel.alpha_visible.len() + 1) as u64)?;
        if !state.doc.shares_state(&self.captured)
            || state.revision != self.revision
            || state.view_revision != self.view_revision
            || state.history.max_states != self.old_history.max_states
            || state.history.max_bytes != self.old_history.max_bytes
            || !state.history.shares_accounted_history(&self.old_history)
            || !state.accounted_view.as_ref().is_some_and(|v| v.shares_state(&self.old_view))
            || state.active_layer != self.active
            || state.layer_anchor != self.anchor
            || !state.selected_layers.is_empty()
            || state.floating.is_some()
            || state.channel_view != self.channel
            || state.source_read_only != self.source_read_only
        {
            return Err(EngineError::Other("native-save.stale-accounted-target".into()));
        }
        let string_work =
            self.old_path.as_ref().map_or(0, String::len) + self.source.as_ref().map_or(0, |s| s.label.len() + s.path.as_ref().map_or(0, String::len));
        c.check((string_work as u64).checked_mul(2).ok_or(BackingError::Length)?)?;
        if state.path != self.old_path || state.source != self.source {
            return Err(EngineError::Other("native-save.stale-accounted-source".into()));
        }
        c.check(1)?;
        let result = write(&self.snapshot, &self.path)?;
        // No poll, allocation, callbacks, or fallible work follows committed write.
        state.doc = self.snapshot;
        state.history = self.history;
        state.accounted_view = Some(self.view);
        state.path = Some(self.path);
        state.saved_revision = self.revision;
        state._accounted_save_identity = Some(self.path_lease);
        Ok(result)
    }
}
#[cfg(test)]
#[path = "accounted_native_save_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;

#[cfg(test)]
#[path = "accounted_native_save_title_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod title_tests;
