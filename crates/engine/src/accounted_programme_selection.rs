//! Paid captured context for guarded programme pixel-selection replacement.
//! The immutable owner and complete Current recipe remain authoritative.
use crate::{
    DocumentSnapshot,
    accounted_programme_current::{self, ProgrammeKind},
    accounted_view::AccountedView,
    channel_cmds::ChannelView,
    document_history::DocumentHistory,
    source_policy::SourceIdentity,
};
use crate::{EngineError, Result, Session};
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{QuotaKind, QuotaLease},
};
fn kind(owner: &AccountedDocument) -> Option<ProgrammeKind> {
    accounted_programme_current::kind(owner)
}
fn recipe(owner: &AccountedDocument, expected: Option<ProgrammeKind>, c: &ReadControl<'_>) -> Result<()> {
    if kind(owner) != expected {
        return Err(BackingError::Unsupported.into());
    }
    if let Some(expected) = expected {
        owner.validate_programme_selection_replacement(c)?;
        let guard = owner.checked_programme_selection(c)?;
        if !guard.matches_owner(owner) || guard.storage_kind() != expected {
            return Err(BackingError::Integrity.into());
        }
        accounted_programme_current::validate(owner, expected, c)
    } else {
        if !owner.is_legacy_storage() {
            return Err(BackingError::Unsupported.into());
        }
        owner.validate_selection_replacement(c).map_err(Into::into)
    }
}
fn copy(v: &str) -> Result<String> {
    let mut s = String::new();
    s.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if s.capacity() > v.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    s.push_str(v);
    Ok(s)
}
fn flags(v: &[bool], ctl: &ReadControl<'_>) -> Result<Vec<bool>> {
    let mut out = Vec::new();
    out.try_reserve_exact(v.len()).map_err(|_| BackingError::Allocation)?;
    if out.capacity() > v.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for b in v {
        ctl.check(1)?;
        out.push(*b);
    }
    Ok(out)
}
fn channel_shape(v: &ChannelView) -> Result<()> {
    if v.color_hidden.len() > 32 || v.alpha_visible.len() > 4096 {
        return Err(BackingError::Length.into());
    }
    if v.layer_mask.is_some()
        || v.editing_mask.is_some()
        || !matches!(v.target, crate::channel_cmds::ChannelTarget::Composite | crate::channel_cmds::ChannelTarget::Color(_))
    {
        return Err(BackingError::Unsupported.into());
    }
    Ok(())
}
fn idle(session: &Session) -> Result<()> {
    if session.active().is_some_and(|s| s.coalesce.is_some() || s.floating.is_some() || s.doc.xcf_state.as_ref().is_some_and(|x| x.floating.is_some()))
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
    {
        return Err(EngineError::Other("accounted selection requires an idle synchronous color context without coalescing".into()));
    }
    Ok(())
}
pub(super) struct ProgrammeSelectionCapture {
    kind: Option<ProgrammeKind>,
    captured: DocumentSnapshot,
    old_history: DocumentHistory,
    view: AccountedView,
    revision: u64,
    view_revision: u64,
    active: Option<photocraft_doc::LayerId>,
    anchor: Option<photocraft_doc::LayerId>,
    channel: ChannelView,
    path: Option<String>,
    source: Option<SourceIdentity>,
    read_only: bool,
    history_states: u32,
    history_bytes: usize,
    _lease: QuotaLease,
}
impl ProgrammeSelectionCapture {
    pub(super) fn is_programme(&self) -> bool {
        self.kind.is_some()
    }
    pub(super) fn prepare(session: &Session, ctl: &ReadControl<'_>) -> Result<Self> {
        idle(session)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        let kind = kind(owner);
        if !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        recipe(owner, kind, ctl)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        if state.active_layer != view.target().active || state.layer_anchor != view.anchor() || !state.selected_layers.is_empty() {
            return Err(BackingError::Integrity.into());
        }
        channel_shape(&state.channel_view)?;
        if matches!(state.channel_view.target,crate::channel_cmds::ChannelTarget::Color(i) if i>=state.doc.mode.color_channels()) {
            return Err(BackingError::Bounds.into());
        }
        if state.path.as_ref().is_some_and(|v| v.len() > 4096)
            || state.source.as_ref().is_some_and(|s| s.label.len() > 4096 || s.path.as_ref().is_some_and(|v| v.len() > 4096))
        {
            return Err(BackingError::Length.into());
        }
        let strings = state
            .path
            .as_ref()
            .map_or(0, String::len)
            .checked_add(state.source.as_ref().map_or(0, |s| s.label.len() + s.path.as_ref().map_or(0, String::len)))
            .ok_or(BackingError::Length)?;
        let count = state.channel_view.color_hidden.len().checked_add(state.channel_view.alpha_visible.len()).ok_or(BackingError::Length)?;
        let envelope = strings
            .checked_add(count)
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(std::mem::size_of::<ProgrammeSelectionCapture>() + 512))
            .ok_or(BackingError::Length)?;
        let mut lease = owner.quota().reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
        ctl.check(strings.checked_add(count).ok_or(BackingError::Length)? as u64)?;
        let path = state.path.as_deref().map(copy).transpose()?;
        let source = state
            .source
            .as_ref()
            .map(|s| Ok::<_, EngineError>(SourceIdentity { label: copy(&s.label)?, path: s.path.as_deref().map(copy).transpose()?, origin: s.origin }))
            .transpose()?;
        let channel = ChannelView {
            target: state.channel_view.target,
            color_hidden: flags(&state.channel_view.color_hidden, ctl)?,
            alpha_visible: flags(&state.channel_view.alpha_visible, ctl)?,
            quick_mask_hidden: state.channel_view.quick_mask_hidden,
            layer_mask: None,
            editing_mask: None,
        };
        let actual = path
            .as_ref()
            .map_or(0, String::capacity)
            .checked_add(source.as_ref().map_or(0, |s| s.label.capacity() + s.path.as_ref().map_or(0, String::capacity)))
            .and_then(|v| v.checked_add(channel.color_hidden.capacity() + channel.alpha_visible.capacity()))
            .and_then(|v| v.checked_add(std::mem::size_of::<ProgrammeSelectionCapture>() + 512))
            .ok_or(BackingError::Length)?;
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Allocation)?;

        Ok(Self {
            kind,
            captured: state.doc.clone(),
            old_history: state.history.clone(),
            view: view.clone(),
            revision: state.revision,
            view_revision: state.view_revision,
            active: state.active_layer,
            anchor: state.layer_anchor,
            channel,
            path,
            source,
            read_only: state.source_read_only,
            history_states: session.prefs().performance.history_states,
            history_bytes: session.prefs().performance.history_budget_bytes(),
            _lease: lease,
        })
    }
    pub(super) fn validate(&self, session: &Session, ctl: &ReadControl<'_>) -> Result<()> {
        ctl.check(1)?;
        idle(session)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let view = state.accounted_view.as_ref().ok_or(BackingError::Integrity)?;
        channel_shape(&state.channel_view)?;
        ctl.check((self.channel.color_hidden.len() + self.channel.alpha_visible.len() + 1) as u64)?;
        for text in self.path.iter().chain(self.source.iter().flat_map(|s| std::iter::once(&s.label).chain(s.path.iter()))) {
            ctl.check(text.len() as u64)?;
        }
        if !state.doc.shares_state(&self.captured)
            || state.revision != self.revision
            || state.view_revision != self.view_revision
            || !view.shares_state(&self.view)
            || state.active_layer != self.active
            || state.layer_anchor != self.anchor
            || !state.selected_layers.is_empty()
            || state.channel_view != self.channel
            || state.path != self.path
            || state.source != self.source
            || state.source_read_only != self.read_only
            || !state.history.shares_accounted_history(&self.old_history)
            || state.history.max_states != self.old_history.max_states
            || state.history.max_bytes != self.old_history.max_bytes
            || session.prefs().performance.history_states != self.history_states
            || session.prefs().performance.history_budget_bytes() != self.history_bytes
        {
            return Err(EngineError::Other("the prepared accounted selection targets older Session state".into()));
        }

        let owner = state.doc.accounted().ok_or(BackingError::Unsupported)?;
        if kind(owner) != self.kind || !state.history.current()?.shares_state(&state.doc) {
            return Err(BackingError::Integrity.into());
        }
        recipe(owner, self.kind, ctl)?;
        ctl.check(1)?;
        Ok(())
    }
}
