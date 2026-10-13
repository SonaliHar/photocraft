//! Ask When Opening (Edit › Color Settings): a file whose embedded profile doesn't match the
//! working space (or that has none) is held back while a prompt asks what to do, as Photoshop
//! does: one prompt at a time, naming its file, before that file opens. OK applies the answer to
//! that file and opens it, then asks about the next; Cancel opens neither it nor the files still
//! to come. Files that finish reading while a prompt is up wait their turn, so they open in order.

use std::sync::Arc;

use photocraft_doc::{ColorMode, Document};
use serde_json::{Value, json};

use crate::PhotocraftApp;

#[path = "open_prompt_control.rs"]
mod control;

/// The field that marks a prompt as this module's.
const FIELD: &str = "__heldOpen";

/// A file read and decoded, waiting to open.
pub struct Held {
    pub doc: Document,
    /// Detected import provenance; this does not grant filesystem authority.
    pub origin: Option<photocraft_io::ImportOrigin>,
    /// The source is read-only (an Affinity file): Save doesn't write back to it.
    pub read_only: bool,
    pub name: String,
    pub path: Option<String>,
    pub warnings: Vec<String>,
    /// The file's bytes, for Camera Raw to develop from when there is no path.
    pub bytes: Option<Arc<Vec<u8>>>,
    /// The tab position a reopened file keeps.
    pub slot: Option<usize>,
    /// The profile question ([`photocraft_engine::color_cmds::ColorState::open_question`]), or
    /// `None` to open without asking.
    pub question: Option<Value>,
    /// The Color Settings policy was already applied (a background open added it before it had
    /// to wait); otherwise opening applies it.
    pub policy_applied: bool,
}

/// Prepared metadata separated from document ownership for atomic queue transfer.
pub(crate) struct HeldMetadata {
    pub origin: Option<photocraft_io::ImportOrigin>,
    pub read_only: bool,
    pub name: String,
    pub path: Option<String>,
    pub warnings: Vec<String>,
    pub bytes: Option<Arc<Vec<u8>>>,
    pub slot: Option<usize>,
    pub question: Option<Value>,
    pub policy_applied: bool,
}
impl HeldMetadata {
    fn with_document(self, doc: Document) -> Held {
        Held {
            doc,
            origin: self.origin,
            read_only: self.read_only,
            name: self.name,
            path: self.path,
            warnings: self.warnings,
            bytes: self.bytes,
            slot: self.slot,
            question: self.question,
            policy_applied: self.policy_applied,
        }
    }
}
/// An already-admitted reserved queue slot. Internal callers must transfer the
/// exact borrowed candidate immediately, without intervening queue changes.
pub(crate) struct PreparedArrival {
    admission: control::Admission,
    count: usize,
}
pub(crate) fn prepare_arrival(
    queue: &mut std::collections::VecDeque<Held>,
    doc: &Document,
    metadata: &HeldMetadata,
    clone_candidate: bool,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<PreparedArrival, String> {
    let admission = control::prepare(queue, control::Entry::metadata(doc, metadata), clone_candidate, interrupt)?;
    queue.try_reserve_exact(1).map_err(|_| "could not prepare this file for opening".to_string())?;
    admission.check_capacity(queue.capacity(), interrupt)?;
    Ok(PreparedArrival { admission, count: queue.len() })
}
impl PreparedArrival {
    pub(crate) fn recheck(&self, queue: &std::collections::VecDeque<Held>, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<(), String> {
        if queue.len() != self.count {
            return Err("pending files changed before opening".into());
        }
        self.admission.check_capacity(queue.capacity(), interrupt)
    }
    /// Ownership moves only after the final preparation fence. No second
    /// admission/serialization/clone can fail between transfer and queue push.
    pub(crate) fn commit(self, app: &mut PhotocraftApp, doc: Document, metadata: HeldMetadata) {
        app.held_opens.push_back(metadata.with_document(doc));
        advance(app);
    }
}
pub(crate) fn copy_string(source: &str) -> Result<String, String> {
    control::string(source, &photocraft_raster::Interrupt::NONE)
}
pub(crate) fn copy_question(question: Option<&Value>) -> Result<Option<Value>, String> {
    control::question(question, &photocraft_raster::Interrupt::NONE)
}
pub(crate) fn copy_warnings(value: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(items) = value.and_then(Value::as_array) else { return Ok(Vec::new()) };
    if items.len() > 100_000 {
        return Err("pending import warnings exceed model census limits".into());
    }
    let mut total = (items.len() as u64).checked_mul(std::mem::size_of::<String>() as u64).ok_or("pending import warning size overflow")?;
    for item in items {
        if let Some(text) = item.as_str() {
            total = total
                .checked_add(text.len() as u64)
                .filter(|bytes| *bytes <= photocraft_raster::MAX_CHECKED_RASTER_BYTES)
                .ok_or("pending import warnings exceed resident limits")?;
        }
    }
    let mut warnings = Vec::new();
    warnings.try_reserve_exact(items.len()).map_err(|_| "pending import warning allocation failed".to_string())?;
    for item in items {
        if let Some(text) = item.as_str() {
            warnings.push(copy_string(text)?);
        }
    }
    Ok(warnings)
}

/// Whether a file that just finished reading has to wait: one earlier in the queue does.
pub fn waiting(app: &PhotocraftApp) -> bool {
    !app.held_opens.is_empty()
}

/// Opens `held` now, or queues it behind the file whose prompt is up.
pub fn arrive(app: &mut PhotocraftApp, held: Held) {
    if let Err(error) = arrive_with_interrupt(app, held, &photocraft_raster::Interrupt::NONE) {
        app.failed(error);
    }
}

/// Fallible immediate arrival for prepared native imports. A queued prompt is
/// accepted without publication; its eventual answer retains the existing Result.
pub(crate) fn arrive_checked(app: &mut PhotocraftApp, held: Held) -> Result<(), String> {
    if waiting(app) || held.question.is_some() {
        arrive_with_interrupt(app, held, &photocraft_raster::Interrupt::NONE)
    } else {
        control::prepare(&app.held_opens, control::Entry::held(&held), false, &photocraft_raster::Interrupt::NONE)?;
        open(app, held, None).map(|_| ())
    }
}

/// Controlled queue preparation; cancellation/refusal never publishes a new held file.
pub(crate) fn arrive_with_interrupt(app: &mut PhotocraftApp, held: Held, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<(), String> {
    let admission = control::prepare(&app.held_opens, control::Entry::held(&held), false, interrupt)?;
    app.held_opens.try_reserve_exact(1).map_err(|_| "could not prepare this file for opening".to_string())?;
    admission.check_capacity(app.held_opens.capacity(), interrupt)?;
    app.held_opens.push_back(held);
    advance(app);
    Ok(())
}

/// Opens queued files in order until one needs its prompt, which is shown if it isn't yet.
fn advance(app: &mut PhotocraftApp) {
    while let Some(front) = app.held_opens.front() {
        if front.question.is_some() {
            if !app.ui.dialogs.iter().any(|d| d.fields.contains_key(FIELD)) {
                show(app);
            }
            return;
        }
        if let Some(held) = app.held_opens.pop_front() {
            // A failure is shown to the user (`open_failed`); the next file still opens.
            let _ = open(app, held, None);
        }
    }
}

/// Shows the Embedded Profile Mismatch (or missing profile) prompt for the first queued file.
fn show(app: &mut PhotocraftApp) {
    let Some(front) = app.held_opens.front() else { return };
    let Some(q) = &front.question else { return };
    // The mode reads as the catalogs name it (French "RVB"); the file name goes in last, so
    // nothing in it is taken for a placeholder.
    let mode = tl!(match front.doc.mode {
        ColorMode::Cmyk => "CMYK",
        ColorMode::Grayscale => "Gray",
        _ => "RGB",
    });
    let name = front.name.as_str();
    let working = crate::i18n::fmt(tl!("Working: {working}"), &[("working", q.get("working").and_then(Value::as_str).unwrap_or("?"))]);
    let missing = q.get("missing").and_then(Value::as_bool) == Some(true);
    let message = if missing {
        let what = crate::i18n::fmt(tl!("The document “{name}” does not have an embedded {mode} profile."), &[("mode", mode), ("name", name)]);
        format!("{what}\n{working}")
    } else {
        let what = crate::i18n::fmt(
            tl!("The document “{name}” has an embedded color profile that does not match the current {mode} working space."),
            &[("mode", mode), ("name", name)],
        );
        let embedded = crate::i18n::fmt(tl!("Embedded: {embedded}"), &[("embedded", q.get("embedded").and_then(Value::as_str).unwrap_or("?"))]);
        format!("{what}\n{embedded}\n{working}")
    };
    let action = q.get("action").and_then(Value::as_str).unwrap_or("preserve").to_string();
    crate::prefs_ui::open_mismatch(app, &message, &action, missing, FIELD);
}

/// Whether `fields` are one of this module's prompts.
pub fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(FIELD)
}

/// The prompt's answer: `Some(action)` (OK) opens the first queued file with it and moves on to
/// the next; `None` (Cancel) drops every queued file and stops the reads still running.
pub fn answer(app: &mut PhotocraftApp, action: Option<&str>) -> Result<Value, String> {
    let Some(action) = action else {
        let dropped = app.held_opens.len().checked_add(app.session.held_open_count()).ok_or("pending import cancellation count overflow")?;
        let response = json!({"cancelled": dropped});
        let name = match app.held_opens.front() {
            Some(held) => control::string(&held.name, &photocraft_raster::Interrupt::NONE)?,
            None => String::new(),
        };
        app.run("file.open.discardPending", json!({}))?;
        drop(std::mem::take(&mut app.held_opens));
        for open in &app.jobs.opens {
            app.session.cancel_job(open.job);
        }
        app.ui.status = crate::i18n::fmt(tl!("Cancelled opening {name}"), &[("name", &name)]);
        app.ui.status_error = false;
        return Ok(response);
    };
    let Some(held) = app.held_opens.pop_front() else { return Err("no file is waiting to open".into()) };
    let r = open(app, held, Some(action));
    advance(app);
    r
}

/// Opens a queued file, applying `action` (the prompt's answer) or else the Color Settings
/// policy, then does what any open does afterwards.
fn open(app: &mut PhotocraftApp, held: Held, action: Option<&str>) -> Result<Value, String> {
    let Held { mut doc, origin, read_only, name, path, warnings, bytes, slot, question, policy_applied } = held;
    let source_path = match &path {
        Some(path) => Some(control::string(path, &photocraft_raster::Interrupt::NONE)?),
        None => None,
    };
    let index = if question.is_some() || policy_applied {
        if let Some(action) = action
            && let Err(e) = app.session.color.resolve_open(&mut doc, action)
        {
            app.open_failed(&name, &e.to_string());
            return Err(e.to_string());
        }
        app.session.add_document(doc, None).map_err(|error| error.to_string())?
    } else {
        app.session.open_document(doc, None).map_err(|error| error.to_string())?.0
    };
    if let Some(st) = app.session.active_mut() {
        st.source_read_only = read_only;
        if let Some(origin) = origin {
            st.set_import_origin(origin, &name, source_path);
        }
    }
    let index = match slot {
        Some(slot) => app
            .run("document.move", json!({"document": index, "to": slot}))
            .ok()
            .and_then(|m| m.get("document").and_then(Value::as_u64))
            .map_or(index, |i| i as usize),
        None => index,
    };
    app.session.set_active(index);
    crate::jobs_ui::after_open(app, &name, path.as_deref(), &warnings, bytes.as_deref().map(Vec::as_slice));
    Ok(json!({"document": index, "action": action}))
}
