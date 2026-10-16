//! Background jobs: long commands run on a worker thread, report progress and can be cancelled
//! (#210).
//!
//! A command opts in by doing its heavy work through [`run`] (or [`edit_job`] for a plain document
//! edit). The work closure gets a [`JobCtx`] (progress plus a cancellation flag) and runs against
//! a snapshot: documents are copy-on-write, so cloning one is O(layers). Its result is applied on
//! the UI thread as one undo step, through the same path as [`Session::execute`], so the journal,
//! history, damage and post-command hooks all behave as for a synchronous command.
//!
//! - [`Session::execute`] runs everything inline, as before: tests, the CLI and scripts see no
//!   difference.
//! - [`Session::start`] runs a job-capable command on a worker thread and returns its
//!   [`JobId`]; [`Session::poll_jobs`] (called every frame by the UI) applies finished jobs.
//!   On wasm `start` runs inline (same API, written once).
//! - [`Session::cancel_job`] sets the flag; the algorithms check it per tile, row band or
//!   iteration, and the result is discarded. The document is never touched by a worker.
//! - A panic in the worker becomes an error; the document is unchanged.
//! - While a job runs on a document, commands that would edit that document are disabled with a
//!   message naming the job (queries and document-independent commands still run).

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::{DocId, Document, LayerId};
use photocraft_raster::Interrupt;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, WeakDocumentSnapshot};

mod accounted_color_range;

/// Identifies one background job for the session's lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
pub struct JobId(pub u64);

/// What a job's work closure sees: report progress, check for cancellation. Cheap to clone; all
/// clones share the same state.
#[derive(Clone, Default)]
pub struct JobCtx {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    cancel: AtomicBool,
    /// `f32` bits of the progress fraction. Non-negative floats order like their bits, so
    /// `fetch_max` keeps progress monotonic even with several reporting threads.
    progress: AtomicU32,
    message: Mutex<String>,
    /// A paid accounted worker keeps its control-storage charge through the
    /// last JobCtx clone, including cancellation/draining after Running drops.
    _accounted_dispatch: Mutex<Option<Arc<photocraft_raster::backing::quota::QuotaLease>>>,
}

impl std::fmt::Debug for JobCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobCtx").field("progress", &self.fraction()).field("cancelled", &self.cancelled()).finish()
    }
}

impl JobCtx {
    fn retain_accounted_dispatch(&self, lease: Arc<photocraft_raster::backing::quota::QuotaLease>) {
        let mut held = self.shared._accounted_dispatch.lock().unwrap_or_else(PoisonError::into_inner);
        // Nested inline calls may share their original token. Retaining the
        // first paid owner until that token dies avoids replacing a live grant.
        if held.is_none() {
            *held = Some(lease);
        }
    }
    pub fn new() -> Self {
        Self::default()
    }

    /// Report progress in `0.0..=1.0` with a short status (empty keeps the previous one).
    /// Progress never goes backwards: a lower value than already reported is ignored.
    pub fn progress(&self, fraction: f32, message: &str) {
        if fraction.is_finite() {
            self.shared.progress.fetch_max(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
        if !message.is_empty() {
            let mut m = self.shared.message.lock().unwrap_or_else(PoisonError::into_inner);
            if *m != message {
                message.clone_into(&mut m);
            }
        }
    }

    /// Has the job been cancelled? Long loops check this and stop early.
    pub fn cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::Relaxed)
    }

    /// Ask the job to stop (idempotent).
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// `Err(Cancelled)` once cancelled, for `?` between stages.
    pub fn check(&self) -> Result<()> {
        if self.cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
    }

    /// Progress reported so far (`0.0..=1.0`).
    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.shared.progress.load(Ordering::Relaxed))
    }

    /// The latest status message.
    pub fn message(&self) -> String {
        self.shared.message.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Run `f` with an algorithm-level [`Interrupt`] whose progress maps onto `lo..hi` of this
    /// job (with `message`) and whose cancellation is this job's.
    pub fn stage<R>(&self, lo: f32, hi: f32, message: &str, f: impl FnOnce(&Interrupt) -> R) -> R {
        self.progress(lo, message);
        let cancel = || self.cancelled();
        let progress = |p: f32| self.progress(lo + (hi - lo) * p, "");
        f(&Interrupt::new(&cancel, &progress))
    }
}

/// Final state of a job.
#[derive(Clone, Debug, PartialEq)]
pub enum JobOutcome {
    /// Applied; the command's result.
    Done(Value),
    /// Failed (including a panic in the worker); the document is unchanged.
    Failed(String),
    /// Cancelled; the result (if any) was discarded and the document is unchanged.
    Cancelled,
}

/// A job that has ended, as reported by [`Session::poll_jobs`].
#[derive(Clone, Debug, PartialEq)]
pub struct JobEvent {
    pub id: JobId,
    pub command: String,
    pub label: String,
    pub document: Option<DocId>,
    pub outcome: JobOutcome,
}

/// A job's state for the UI, the control channel and MCP (`jobs.list`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: JobId,
    pub command: String,
    /// Shown in the progress UI ("Gaussian Blur").
    pub label: String,
    /// The document the job edits (and locks), if any.
    pub document: Option<DocId>,
    /// `running`, `done`, `failed` or `cancelled`.
    pub state: &'static str,
    pub progress: f32,
    pub message: String,
    pub elapsed_ms: f64,
    /// The command's result (done) or the error (failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

type Payload = Box<dyn Any + Send>;
type Apply = Box<dyn FnOnce(&mut Session, Payload) -> Result<Value> + Send>;

/// Wall-clock milliseconds since some fixed point (0 on wasm, where `Instant` is unavailable and
/// jobs run inline anyway).
fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

struct Running {
    id: JobId,
    command: String,
    label: String,
    params: Value,
    journal: bool,
    /// The typed accounted carrier publishes its prepaid journal itself.
    /// Only the private accounted dispatch can enable this path.
    prepaid_bookkeeping: bool,
    doc: Option<DocId>,
    /// The document as the job saw it: applying fails if it was edited meanwhile.
    snapshot: Option<WeakDocumentSnapshot>,
    ctx: JobCtx,
    started_ms: f64,
    coalesce: Option<String>,
    color_restrict: Option<usize>,
    rx: std::sync::mpsc::Receiver<Result<Payload>>,
    apply: Option<Apply>,
    #[cfg(not(target_arch = "wasm32"))]
    handle: Option<std::thread::JoinHandle<()>>,
    // Paid accounted dispatch metadata follows its physical strings/callbacks.
    // Generic executor/table/channel allocator internals keep their existing scope.
    _accounted_dispatch: Option<Arc<photocraft_raster::backing::quota::QuotaLease>>,
}

impl Running {
    fn info(&self) -> JobInfo {
        JobInfo {
            id: self.id,
            command: self.command.clone(),
            label: self.label.clone(),
            document: self.doc,
            state: "running",
            progress: self.ctx.fraction(),
            message: self.ctx.message(),
            elapsed_ms: now_ms() - self.started_ms,
            result: None,
            error: None,
        }
    }
}

/// The session's job table (see the module docs).
#[derive(Default)]
pub struct Jobs {
    next: u64,
    /// Set while [`Session::start`] runs a command: [`run`] spawns instead of running inline.
    spawn: bool,
    /// The job a command registered while `spawn` was set (picked up by `start`).
    pending: Option<Running>,
    running: Vec<Running>,
    /// Ended jobs not yet collected by [`Session::poll_jobs`].
    events: Vec<JobEvent>,
    /// The last few ended jobs, for `jobs.list`.
    recent: Vec<JobInfo>,
    /// Workers of cancelled jobs that may still be unwinding (native).
    #[cfg(not(target_arch = "wasm32"))]
    draining: Vec<std::thread::JoinHandle<()>>,
    /// The context inline job work runs under (see [`Session::set_inline_job_ctx`]).
    inline: Option<JobCtx>,
}

const RECENT: usize = 16;

impl Drop for Jobs {
    fn drop(&mut self) {
        // Workers own snapshots only; stop them so they don't burn CPU for a closed session.
        for r in self.running.iter().chain(self.pending.iter()) {
            r.ctx.cancel();
        }
    }
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jobs").field("running", &self.running.len()).finish()
    }
}

/// How [`Session::start`] ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Started {
    /// The command ran to completion inline (it isn't a job, or this is wasm).
    Done(Value),
    /// The command is running in the background.
    Job(JobId),
}

/// Commands that never edit the active document, allowed while a job runs on it.
fn independent_of_document(id: &str) -> bool {
    const PREFIXES: &[&str] = &["jobs.", "tools.", "view.", "window.", "help.", "brush.presets.", "gradient.presets."];
    const IDS: &[&str] = &["file.new", "file.close", "file.closeAll", "edit.preferences", "edit.colorSettings", "edit.keyboardShortcuts", "edit.menus"];
    PREFIXES.iter().any(|p| id.starts_with(p)) || IDS.contains(&id)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_path_with_svg_group_depth(name: &str, path: &str, ctx: &JobCtx, max_svg_group_depth: usize) -> Result<photocraft_io::ImportResult> {
    use std::io::{Seek, SeekFrom};
    ctx.check()?;
    // Only the already-authorized native OpenSource::Path lane reaches this
    // ambient open. Keep this same handle for classification, native streaming
    // and the content-first fallback; never reopen a stored resource locator.
    let mut file = std::fs::File::open(path).map_err(|error| EngineError::Other(format!("{path}: {error}")))?;
    ctx.check()?;
    let candidate =
        ctx.stage(0.0, 0.95, "Decoding", |control| photocraft_io::import_native_stream_candidate(name, &mut file, control)).map_err(|error| match error {
            photocraft_io::IoError::Cancelled => EngineError::Cancelled,
            error => EngineError::Other(error.to_string()),
        })?;
    if let Some(document) = candidate {
        ctx.check()?;
        return Ok(document);
    }
    ctx.check()?;
    file.seek(SeekFrom::Start(0)).map_err(|error| EngineError::Other(format!("{path}: {error}")))?;
    ctx.check()?;
    // Legacy formats keep the same size-authority/allocation semantics, with
    // cooperative per-chunk cancellation rather than a new small file cap.
    let size_hint = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let bytes = ctx.stage(0.0, 0.02, "Reading", |control| {
        photocraft_format::read::read_all_with_interrupt(&mut file, size_hint, control)
            .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("{path}: {error}")) })
    })?;
    ctx.check()?;
    let imported = ctx.stage(0.02, 0.95, "Decoding", |control| photocraft_io::import_with_svg_group_depth_and_interrupt(name, &bytes, max_svg_group_depth, control)).map_err(|error| match error {
        photocraft_io::IoError::Cancelled => EngineError::Cancelled,
        error => EngineError::Other(error.to_string()),
    })?;
    ctx.check()?;
    Ok(imported)
}

#[cfg(target_arch = "wasm32")]
fn read_path_with_svg_group_depth(_name: &str, path: &str, _ctx: &JobCtx, _max_svg_group_depth: usize) -> Result<photocraft_io::ImportResult> {
    Err(EngineError::Other(format!("cannot open paths on the web: {path}")))
}

#[cfg(test)]
fn read_path(name: &str, path: &str, ctx: &JobCtx) -> Result<photocraft_io::ImportResult> {
    read_path_with_svg_group_depth(name, path, ctx, usize::MAX)
}

/// Run a job-capable command's heavy part. `work` runs on a worker thread when the command was
/// started with [`Session::start`] (inline otherwise, and always on wasm); `apply` then runs on
/// the calling thread with its output and returns the command's result. With `lock_document`,
/// the active document is locked against other edits while the job runs, and applying fails if
/// it was edited anyway.
///
/// `work` must only read what it captured (clone the document `Arc` first): it never sees the
/// session.
pub fn run<T: Send + 'static>(
    s: &mut Session,
    label: &str,
    lock_document: bool,
    work: impl FnOnce(&JobCtx) -> Result<T> + Send + 'static,
    apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
) -> Result<Value> {
    if !s.jobs.spawn || cfg!(target_arch = "wasm32") {
        let t = work(&s.jobs.inline.clone().unwrap_or_default())?;
        return apply(s, t);
    }
    if s.jobs.pending.is_some() {
        return Err(EngineError::Other(format!("`{label}` started a second background job in one command")));
    }
    let (doc, snapshot) = match (lock_document, s.active()) {
        (true, Some(st)) => (Some(st.doc.id), Some(st.doc.downgrade())),
        (true, None) => return Err(EngineError::NoDocument),
        (false, _) => (None, None),
    };
    let ctx = JobCtx::new();
    let (tx, rx) = std::sync::mpsc::channel::<Result<Payload>>();
    let erased: Apply = Box::new(move |s, any| {
        let t = any.downcast::<T>().map_err(|_| EngineError::Other("internal error: job result type mismatch".into()))?;
        apply(s, *t)
    });
    let next = s.jobs.next.checked_add(1).ok_or_else(|| EngineError::Other("background job identities exhausted".into()))?;
    s.jobs.next = next;
    let id = JobId(next);
    #[cfg(not(target_arch = "wasm32"))]
    let handle = {
        let wctx = ctx.clone();
        let label_w = label.to_string();
        // The work closure is moved into the thread. If spawning fails (thread limits), there is
        // no way to get it back, so the error is reported instead.
        let spawned = std::thread::Builder::new().name(format!("photocraft-job-{}", id.0)).spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&wctx).map(|t| Box::new(t) as Payload)))
                .unwrap_or_else(|_| Err(EngineError::Other(format!("`{label_w}` failed with an internal error (logged); the document is unchanged"))));
            // The receiver is gone when the job was cancelled: nothing to report.
            let _ = tx.send(r);
        });
        Some(spawned.map_err(|e| EngineError::Other(format!("could not start a background job: {e}")))?)
    };
    #[cfg(target_arch = "wasm32")]
    {
        // Unreachable (wasm runs inline above); keep the types used.
        drop((work, tx));
    }
    s.jobs.pending = Some(Running {
        id,
        command: String::new(),
        label: label.to_string(),
        params: Value::Null,
        journal: true,
        prepaid_bookkeeping: false,
        doc,
        snapshot,
        ctx,
        started_ms: now_ms(),
        coalesce: s.coalesce_request.clone(),
        color_restrict: s.color_restrict,
        rx,
        apply: Some(erased),
        #[cfg(not(target_arch = "wasm32"))]
        handle,
        _accounted_dispatch: None,
    });
    Ok(json!({"job": id.0}))
}

/// A document edit whose body is the job: like [`Session::edit`] (one undo step named `label`),
/// but `f` runs on a worker against a copy of the active document when started in the
/// background. `finish` turns `f`'s output into the command's result.
pub fn edit_job<R: Send + 'static>(
    s: &mut Session,
    label: &str,
    f: impl FnOnce(&mut Document, &mut Option<LayerId>, &JobCtx) -> Result<R> + Send + 'static,
    finish: impl FnOnce(R) -> Value + Send + 'static,
) -> Result<Value> {
    if !s.jobs.spawn || cfg!(target_arch = "wasm32") {
        // Inline: exactly `Session::edit`, as before jobs existed.
        let ctx = s.jobs.inline.clone().unwrap_or_default();
        let r = s.edit(label, |doc, active| f(doc, active, &ctx))?;
        return Ok(finish(r));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    st.doc.validate_resident()?;
    let (base, active) = (st.doc.clone(), st.active_layer);
    let label_owned = label.to_string();
    run(
        s,
        label,
        true,
        move |ctx| {
            let mut doc = (*base).clone();
            let mut active = active;
            let r = f(&mut doc, &mut active, ctx)?;
            Ok((doc, active, r))
        },
        move |s, (doc, new_active, r)| {
            s.edit(&label_owned, move |d, active| {
                *d = doc;
                *active = new_active;
                Ok(())
            })?;
            Ok(finish(r))
        },
    )
}

/// Where a background open reads its file from.
#[derive(Clone, Debug)]
pub enum OpenSource {
    /// File contents already in memory (web pickers, drops, automation).
    Bytes(Arc<Vec<u8>>),
    /// A path the worker reads itself (native), so even the read stays off the UI thread.
    Path(String),
}

/// The command id open jobs report (there is no engine command: opening is the shell's).
pub const OPEN_JOB: &str = "file.open";

impl Session {
    /// Run the heavy part of inline (not background) job commands under `ctx`, so another thread
    /// can cancel them: a live preview whose parameters changed stops its stale computation
    /// (`Err(Cancelled)`, the document unchanged) instead of finishing it. `None` restores the
    /// default, a context nothing can cancel.
    pub fn set_inline_job_ctx(&mut self, ctx: Option<JobCtx>) {
        self.jobs.inline = ctx;
    }

    /// Run a command, in the background when it supports it (see [`jobs`](crate::jobs)). Errors
    /// like [`Session::execute`] for unknown, disabled or failing commands.
    pub fn start(&mut self, id: &str, params: Value) -> Result<Started> {
        self.dispatch(id, params, true)
    }

    /// Open a file in the background: read (for a path) and decode on a worker, with progress
    /// (per layer for PSD/PSB) and cancellation, then add the document like
    /// [`Session::open_document`] (Color Settings policies apply). `name` names the document.
    /// Results include import assessment. Profile questions return a private
    /// `{held,name,warnings,color,assessment}` until the prompt is answered;
    /// otherwise `{document,name,warnings,color,assessment}`. WASM runs inline.
    pub fn start_open(&mut self, name: &str, source: OpenSource) -> Result<Started> {
        let label = format!("Opening {name}");
        let source_path = match &source {
            OpenSource::Path(path) => Some(path.clone()),
            OpenSource::Bytes(_) => None,
        };
        let name_w = name.to_string();
        let name_a = name.to_string();
        let max_svg_group_depth = self.prefs().file_handling.rasterize_svg_groups_deeper_than as usize;
        self.start_job(
            OPEN_JOB,
            json!({"name": name}),
            &label,
            false,
            move |ctx| {
                ctx.progress(0.0, "Reading");
                let imported = match source {
                    OpenSource::Path(p) => read_path_with_svg_group_depth(&name_w, &p, ctx, max_svg_group_depth)?,
                    OpenSource::Bytes(bytes) => {
                        ctx.check()?;
                        ctx.progress(0.02, "Decoding");
                        ctx.stage(0.02, 0.95, "Decoding", |ctl| photocraft_io::import_with_svg_group_depth_and_interrupt(&name_w, &bytes, max_svg_group_depth, ctl)).map_err(|e| match e {
                            photocraft_io::IoError::Cancelled => EngineError::Cancelled,
                            e => EngineError::Other(e.to_string()),
                        })?
                    }
                };
                ctx.check()?;
                Ok((imported, ctx.clone()))
            },
            move |s, (r, ctx): (photocraft_io::ImportResult, JobCtx)| {
                ctx.check()?;
                let origin = r.origin;
                let read_only = r.source_read_only || origin.is_read_only();
                let raw = r.warnings.iter().any(|warning| warning.contains(photocraft_io::raw::DEVELOPED_NOTE));
                if let Some(question) = s.color.open_question(&r.document).filter(|_| !raw) {
                    ctx.check()?;
                    let held = ctx.stage(0.95, 1.0, "Preparing Profile Question", |control| s.hold_open_import(r.document, read_only, origin, control))?;
                    return Ok(json!({"held":held,"name":name_a,"warnings":r.warnings,"color":question,"assessment":r.assessment,"previewOnly":r.preview_only}));
                }
                let (index, color) = ctx.stage(0.95, 1.0, "Preparing Document", |control| s.open_document_with_interrupt(r.document, None, control))?;
                if let Some(state) = s.active_mut() {
                    state.source_read_only = read_only;
                    state.set_import_origin(origin, &name_a, source_path.clone());
                    if source_path.as_deref().is_some_and(crate::file_cmds::is_template) {
                        state.path = None;
                    }
                }
                Ok(json!({"document": index, "name": name_a, "warnings": r.warnings, "color": color, "assessment": r.assessment, "previewOnly": r.preview_only}))
            },
        )
    }

    /// Start a job that isn't an engine command (opening a file, a shell operation, a test's fake
    /// job): `work` runs on a worker, `apply` on this thread when [`Session::poll_jobs`] or
    /// [`Session::wait_job`] picks the result up. `command` and `params` identify it in
    /// [`JobInfo`]; it is not journaled. With `lock_document` the active document is locked as
    /// for a command. Runs inline on wasm.
    pub fn start_job<T: Send + 'static>(
        &mut self,
        command: &str,
        params: Value,
        label: &str,
        lock_document: bool,
        work: impl FnOnce(&JobCtx) -> Result<T> + Send + 'static,
        apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
    ) -> Result<Started> {
        self.jobs.spawn = true;
        let r = run(self, label, lock_document, work, apply);
        self.jobs.spawn = false;
        let pending = self.jobs.pending.take();
        match (r, pending) {
            (Err(e), Some(job)) => {
                job.ctx.cancel();
                Err(e)
            }
            (Err(e), None) => Err(e),
            (Ok(_), Some(mut job)) => {
                job.command = command.to_string();
                job.params = params;
                job.journal = false;
                let id = job.id;
                self.jobs.running.push(job);
                Ok(Started::Job(id))
            }
            (Ok(v), None) => Ok(Started::Done(v)),
        }
    }

    /// Apply finished background jobs (on this thread) and return every job that ended since
    /// the last call. Cheap when nothing runs; the UI calls it every frame.
    pub fn poll_jobs(&mut self) -> Vec<JobEvent> {
        let mut i = 0;
        while i < self.jobs.running.len() {
            match self.jobs.running.get(i).map(|r| r.rx.try_recv()) {
                Some(Ok(r)) => {
                    let job = self.jobs.running.remove(i);
                    self.finish_job(job, r);
                }
                Some(Err(std::sync::mpsc::TryRecvError::Empty)) => i += 1,
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    let job = self.jobs.running.remove(i);
                    let label = job.label.clone();
                    self.finish_job(job, Err(EngineError::Other(format!("`{label}` stopped unexpectedly; the document is unchanged"))));
                }
                None => break,
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.jobs.draining.retain(|h| !h.is_finished());
        std::mem::take(&mut self.jobs.events)
    }

    /// Block until job `id` ends, apply it, and return its result (the control channel's and
    /// MCP's default `wait`, and the CLI). `Err(Cancelled)` if it was cancelled.
    pub fn wait_job(&mut self, id: JobId) -> Result<Value> {
        if let Some(i) = self.jobs.running.iter().position(|r| r.id == id) {
            let job = self.jobs.running.remove(i);
            let r = job.rx.recv().unwrap_or_else(|_| Err(EngineError::Other(format!("`{}` stopped unexpectedly", job.label))));
            self.finish_job(job, r);
        }
        match self.jobs.recent.iter().rev().find(|j| j.id == id) {
            Some(j) if j.state == "done" => Ok(j.result.clone().unwrap_or(Value::Null)),
            Some(j) if j.state == "cancelled" => Err(EngineError::Cancelled),
            Some(j) => Err(EngineError::Other(j.error.clone().unwrap_or_else(|| "the job failed".into()))),
            None => Err(EngineError::Other(format!("no job {}", id.0))),
        }
    }

    /// Cancel job `id`: it stops at its next check, its result is discarded and the document
    /// stays unchanged. The document is unlocked at once. False if no such job is running.
    pub fn cancel_job(&mut self, id: JobId) -> bool {
        let Some(i) = self.jobs.running.iter().position(|r| r.id == id) else { return false };
        // A print worker can already have handed the job to the OS. Discarding its result
        // would falsely report cancellation; removal belongs to the system print queue.
        if matches!(self.jobs.running[i].command.as_str(), "file.print" | "file.printOneCopy") {
            return false;
        }
        let job = self.jobs.running.remove(i);
        job.ctx.cancel();
        self.end_job(&job, JobOutcome::Cancelled);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(h) = job.handle {
            self.jobs.draining.push(h);
        }
        true
    }

    /// Cancel every running job (e.g. before quitting).
    pub fn cancel_all_jobs(&mut self) {
        let ids: Vec<JobId> = self.jobs.running.iter().map(|r| r.id).collect();
        for id in ids {
            self.cancel_job(id);
        }
    }

    /// Is any job running? (Cheap: the UI asks every frame.)
    pub fn has_jobs(&self) -> bool {
        !self.jobs.running.is_empty()
    }

    /// Whether the current dispatch requested background edit preparation.
    pub(crate) fn command_is_background(&self) -> bool {
        self.jobs.spawn
    }

    /// Whether an enclosing dispatch has staged a job that is not published yet.
    pub(crate) fn command_has_pending_job(&self) -> bool {
        self.jobs.pending.is_some()
    }

    /// Source preparation only needs ownership, not an allocated JobInfo copy.
    pub(crate) fn command_has_job_on(&self, document: DocId) -> bool {
        self.jobs.pending.as_ref().is_some_and(|job| job.doc == Some(document))
            || self.jobs.running.iter().any(|job| job.doc == Some(document))
    }

    /// Running job `id`.
    pub fn job(&self, id: JobId) -> Option<JobInfo> {
        self.jobs.running.iter().find(|r| r.id == id).map(Running::info)
    }

    /// Running jobs.
    pub fn jobs(&self) -> Vec<JobInfo> {
        self.jobs.running.iter().map(Running::info).collect()
    }

    /// Running jobs followed by the last few that ended (newest last), for `jobs.list`.
    pub fn jobs_with_recent(&self) -> Vec<JobInfo> {
        self.jobs().into_iter().chain(self.jobs.recent.iter().cloned()).collect()
    }

    /// The running job that locks document `doc`, if any.
    pub fn job_on(&self, doc: DocId) -> Option<JobInfo> {
        self.jobs.running.iter().find(|r| r.doc == Some(doc)).map(Running::info)
    }

    /// The running job that locks the active document, if any.
    pub fn active_job(&self) -> Option<JobInfo> {
        self.active().and_then(|st| self.job_on(st.doc.id))
    }

    /// Wait for background workers of cancelled jobs to exit (tests and benchmarks measure
    /// cancel latency with it). Native only; returns at once on wasm.
    pub fn join_cancelled_jobs(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        for h in self.jobs.draining.drain(..) {
            let _ = h.join();
        }
    }

    /// Why command `id` can't run now because of a background job (None = no conflict).
    pub(crate) fn job_conflict(&self, id: &str, journal: bool) -> Option<String> {
        if !journal || independent_of_document(id) {
            return None;
        }
        let job = self.active_job()?;
        Some(format!("“{}” is still running on this document; wait for it to finish or cancel it (Esc)", job.label))
    }

    /// Cancel the jobs editing document `doc` (it is closing).
    pub(crate) fn cancel_jobs_on(&mut self, doc: DocId) {
        let ids: Vec<JobId> = self.jobs.running.iter().filter(|r| r.doc == Some(doc)).map(|r| r.id).collect();
        for id in ids {
            self.cancel_job(id);
        }
    }

    /// Run the command `id`: inline, or (with `background`) as a job when it is one.
    pub(crate) fn dispatch(&mut self, id: &str, params: Value, background: bool) -> Result<Started> {
        if id != "select.loadSelection" {
            self.stored_load_offer = None;
        }
        if id != "paint.colorReplacement" {
            self.clear_native_color_replacement();
        }
        let result = self.dispatch_inner(id, params, background);
        // Offers belong to one dispatch attempt, including a refusal before
        // the command handler runs. Failed authorization cannot retain pixels
        // for a later command to adopt.
        self.clear_native_color_replacement();
        self.stored_load_offer = None;
        result
    }

    fn dispatch_inner(&mut self, id: &str, params: Value, background: bool) -> Result<Started> {
        let spec = crate::commands::find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        // Params are named fields: anything but an object (or null, meaning none) is a caller
        // mistake, not "use every default".
        if !(params.is_object() || params.is_null()) {
            return Err(EngineError::BadParams { cmd: id.to_string(), msg: "params must be a JSON object".into() });
        }
        // Explicit cancellation owns only unpublished imports. It must remain
        // usable without inspecting, anchoring or repairing unrelated documents.
        if id == "file.open.discardPending" {
            crate::pending_open_cmds::dispatch_preflight(&params)?;
            if let Some(gate) = self.authorize { gate(id, &params)?; }
            self.precondition(spec, &params).map_err(|why| EngineError::Disabled(id.to_string(), why))?;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, &params)))
                .unwrap_or_else(|_| Err(EngineError::Other("pending-open cancellation failed with an internal error".into())));
            return result.map(Started::Done);
        }
        if id == "document.pixel.inspect" {
            crate::inspect_pixel_cmds::dispatch_preflight(&params)?;
        }
        if crate::accounted_commands::filtered_flat_mutation_refused(self,id) {
            return Err(EngineError::Disabled(id.into(),"this filtered-flat document is read-only".into()));
        }
        if crate::accounted_commands::observed_box_mutation_refused(self, id) {
            return Err(EngineError::Disabled(id.into(), "this observed Box geometry recipe is read-only".into()));
        }
        if id == "layer.translate" && self.active().is_some_and(|state| state.doc.accounted().is_some()) {
            crate::accounted_commands::imported_translation_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "layer.setProps" | "xcf.filter.move" | "xcf.filter.remove")
            && self.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_assigned_point_storage() && !(id == "layer.setProps" && crate::accounted_session::metadata_gesture::observed_box_layer_metadata_candidate(owner)))
        {
            return Err(EngineError::Disabled(id.into(), "this command has not yet been qualified for an assigned native filter document".into()));
        }
        if id == "layer.setProps"
            && self.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| {
                owner.is_readonly_threshold_alpha_storage() || owner.is_readonly_threshold_storage() || owner.is_readonly_point_storage()
                    || crate::owned_layer_metadata_candidate(owner)
            })
            && (background || self.jobs.spawn)
        {
            let point = self.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_point_storage());
            let flat=self.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_flat_composite_storage());
            let assigned = self.active().and_then(|state| state.doc.accounted()).is_some_and(crate::assigned_geometry_metadata_candidate);
            return Err(EngineError::BadParams {
                cmd: id.into(),
                msg: if flat { "owned flat composite layer properties require synchronous dispatch" }
                    else if point { "owned Point layer properties require synchronous dispatch" }
                    else if assigned { "owned assigned geometry layer properties require synchronous dispatch" }
                    else { "owned Threshold layer properties require synchronous dispatch" }.into(),
            });
        }
        if id=="xcf.composition.set" && self.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_flat_composite_storage()) && (background||self.jobs.spawn) {
            return Err(EngineError::BadParams{cmd:id.into(),msg:"owned flat composition requires synchronous dispatch".into()});
        }
        crate::brush_cmds::advanced_dispatch_preflight(self, id, &params)?;
        // The actual readonly-flat adapter runs the same parser below under
        // its real temporary fee and finite registered control, before clone,
        // journal or publication. Keep resident preflight/order unchanged.
        if id!="xcf.composition.set" || !self.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_flat_composite_storage()) {
            crate::xcf_composition_cmds::dispatch_preflight(self, id, &params)?;
        }
        if id == "select.object" && crate::selection_object_controlled::handles(&params) {
            crate::selection_object_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.quick" {
            crate::selection_quick_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.subject" {
            crate::selection_subject_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.colorRange" && !crate::selection_cmds::color_range_has_controlled_route(&params) {
            crate::selection_sample_policy::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "select.grow" | "select.similar") {
            crate::selection_grow_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "select.magicWand" {
            crate::selection_wand_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.focusArea" {
            crate::selection_focus_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.sky" {
            crate::selection_sky_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.refineEdge" {
            crate::selection_refine_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.colorRange" {
            if self.active().is_some_and(|state| state.doc.accounted().is_some()) {
                // The paid branch below runs its bounded synchronous native lane.
                // Resident preparation must not reject it before authorization.
                crate::accounted_color_range::validate_request(&params)?;
            } else {
                crate::selection_cmds::color_range_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
            }
        }
        if id == "channel.split" {
            crate::channel_cmds::split_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "channel.merge" {
            crate::channel_cmds::merge_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "channel.mergeSpot" {
            crate::channel_cmds::merge_spot_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.filter.graph.inspectPlan" {
            crate::xcf_filter_graph_cmds::dispatch_preflight(&params)?;
        }
        let xcf_query = crate::xcf_query_preflight::dispatch_preflight(id, &params)?;
        if id == "path.set" {
            let path = params.get("path").ok_or_else(|| EngineError::BadParams { cmd: id.into(), msg: "missing path".into() })?;
            crate::vector_cmds::preflight_path_value(path).map_err(|msg| EngineError::BadParams { cmd: id.into(), msg })?;
        }
        let params = if params.is_null() && (xcf_query || matches!(id, "document.pixel.inspect" | "image.imageSize" | "image.canvasSize" | "image.crop" | "image.trim" | "image.duplicate")) { json!({}) } else { params };
        if id == "image.imageSize" {
            crate::image_cmds::image_size_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.canvasSize" {
            crate::image_cmds::canvas_size_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.crop" {
            crate::image_cmds::crop_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        crate::image_cmds::depth_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::color_cmds::conversion_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        let plate_target = match id {
            "image.mode.multichannel" => Some(photocraft_doc::ColorMode::Multichannel),
            "image.mode.rgb" => Some(photocraft_doc::ColorMode::Rgb),
            "image.mode.grayscale" => Some(photocraft_doc::ColorMode::Grayscale),
            "image.mode.cmyk" => Some(photocraft_doc::ColorMode::Cmyk),
            "image.mode.lab" => Some(photocraft_doc::ColorMode::Lab),
            _ => None,
        };
        if let Some(target) = plate_target
            && (target == photocraft_doc::ColorMode::Multichannel
                || self.active().is_some_and(|state| state.doc.mode == photocraft_doc::ColorMode::Multichannel))
        {
            crate::multichannel_cmds::dispatch_preflight(self, &params, target, background || self.jobs.spawn)?;
        }
        if id == "edit.assignProfile" {
            crate::color_cmds::assignment_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.trim" {
            crate::image_cmds::trim_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.duplicate" {
            crate::image_cmds::duplicate_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.rotation.arbitrary" {
            crate::mode_cmds::rotation_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.mode.indexedColor" {
            crate::mode_cmds::indexed_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.mode.bitmap" {
            crate::mode_cmds::bitmap_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.mode.duotone" {
            crate::mode_cmds::duotone_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "image.mode.colorTable" {
            crate::mode_cmds::color_table_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "image.applyImage" | "image.calculations") {
            crate::channel_cmds::calculation_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "channel.duplicate" {
            crate::channel_cmds::duplicate_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.editInQuickMaskMode" {
            crate::channel_cmds::quick_mask_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.saveSelection" {
            crate::channel_cmds::save_selection_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.loadSelection" {
            crate::channel_cmds::load_selection_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "channel.new" | "channel.newSpot") {
            crate::channel_cmds::new_channel_dispatch_preflight(self, &params, id == "channel.newSpot")?;
        }
        if matches!(id, "channel.rename" | "channel.move" | "channel.delete") {
            crate::channel_cmds::management_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        let guide_operation = match id {
            "view.newGuide" => Some("new"),
            "view.moveGuide" => Some("move"),
            "view.deleteGuide" => Some("delete"),
            "view.clearGuides" => Some("clear"),
            _ => None,
        };
        if let Some(operation) = guide_operation {
            crate::edit_cmds::guide_dispatch_preflight(self, &params, operation, background || self.jobs.spawn)?;
        }
        if id == "path.rename" {
            crate::vector_cmds::path_name_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "view.newGuideLayout" | "view.newGuidesFromShape") {
            crate::file_cmds::guide_layout_dispatch_preflight(self, &params, id == "view.newGuidesFromShape", background || self.jobs.spawn)?;
        }
        if matches!(id, "path.set" | "path.delete") {
            crate::vector_cmds::path_storage_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "channel.options" {
            crate::channel_cmds::options_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "view.clearCanvasGuides" {
            crate::file_cmds::guide_cleanup_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "path.toSelection" {
            crate::vector_cmds::path_selection_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "channel.target" || id == "channel.setVisible" || id == "channel.target.composite" || id.starts_with("channel.target.slot") {
            crate::channel_cmds::view_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "layer.duplicate" {
            crate::layer_duplicate_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "layer.new.layer" {
            crate::new_layer::captured_preflight(self, &params)?;
        }
        if matches!(id, "paint.stroke" | "paint.pencil") {
            crate::brush_cmds::stroke_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "path.clippingPath.set" | "path.clippingPath.clear") {
            crate::vector_cmds::clipping_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "path.transform" {
            crate::vector_cmds::path_transform_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.toWorkPath" {
            crate::vector_cmds::trace_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "path.fill" {
            crate::vector_cmds::path_fill_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "path.stroke" {
            crate::vector_cmds::path_stroke_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "path.moveAnchors" | "path.moveHandle" | "path.bendSegment" | "path.convertPoint") {
            crate::path_edit_cmds::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "layer.layerMask.revealSelection" {
            crate::layer_mask_storage_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.inverse" {
            crate::selection_inverse_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "select.all" | "select.deselect") {
            crate::selection_inverse_controlled::storage_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "select.rect" {
            crate::selection_shapes_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.lasso" {
            crate::selection_polygon_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "select.transformSelection" {
            crate::selection_transform_controlled::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "select.modify.expand" | "select.modify.contract" | "select.modify.border" | "select.modify.smooth" | "select.modify.feather") {
            crate::selection_modify_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "select.modify.expand" | "select.modify.contract" | "select.modify.border") {
            crate::selection_morphology_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if id == "select.modify.smooth" {
            crate::selection_smooth_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "edit.fill" | "edit.clear")
            && !self.active().is_some_and(|state| state.doc.accounted().is_some())
        {
            crate::native_gray_fill_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        }
        crate::native_gray_invert_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_invert_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_gray_threshold_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_gray_posterize_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_threshold_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_posterize_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_gray_levels_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_gray_curves_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_gray_legacy_brightness_contrast_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_legacy_brightness_contrast_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_desaturate_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_gray_desaturate_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_levels_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_curves_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::native_raster_exposure_controlled::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        if matches!(id, "xcf.floating.anchor" | "xcf.floating.toLayer" | "xcf.floating.delete") {
            crate::xcf_floating::controlled::dispatch_preflight(self, id, &params)?;
        }
        crate::layer_mask_storage_controlled::actions_dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        if matches!(id, "edit.copy" | "edit.cut" | "edit.paste" | "edit.pasteSpecial.pasteInPlace" | "edit.pasteSpecial.pasteInto" | "edit.pasteSpecial.pasteOutside") {
            crate::channel_clip::native_gray_dispatch_preflight(self, &params, &photocraft_raster::Interrupt::NONE)?;
        }
        crate::xcf_item_set_manage_cmds::dispatch_preflight(self, id, &params, background || self.jobs.spawn)?;
        crate::xcf_vector_guard::pre_dispatch(self, id, &params)?;
        if id=="layer.renameLayer" && self.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage()) && (background||self.jobs.spawn) {
            return Err(EngineError::BadParams{cmd:id.into(),msg:"mixed rename requires synchronous dispatch".into()});
        }
        if id == "layer.renameLayer" && !self.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage()) {
            crate::layer_multi_cmds::rename_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        // These family-specific validators deliberately refuse unrelated ids.
        if matches!(id, "xcf.filter.move" | "xcf.filter.remove") {
            if self.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_filtered_flat_point_storage()) {
                if background||self.jobs.spawn {return Err(EngineError::BadParams{cmd:id.into(),msg:"mixed filter stack edit requires synchronous dispatch".into()})}
                // The dedicated mixed parser/transaction runs under the same
                // controlled paid dispatch below; do not use resident preflight.
            } else if self.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_point_storage()) {
                if id == "xcf.filter.move" {
                    crate::xcf_filter_cmds::pre_dispatch_point_move(self, id, &params, background || self.jobs.spawn)?;
                } else {
                    crate::xcf_filter_cmds::pre_dispatch_point_remove(self, id, &params, background || self.jobs.spawn)?;
                }
            } else {
                crate::xcf_filter_cmds::pre_dispatch_stack_change_with_background(self, id, &params, background || self.jobs.spawn)?;
            }
        }
        if matches!(id, "xcf.filter.set" | "xcf.filter.enable") {
            if self.active().is_some_and(|state| state.doc.accounted().is_some()) {
                // The closed accounted parser runs under the typed dispatch
                // control below. Preserve the caller's original start mode
                // before that dispatch can normalize job context.
                if background || self.jobs.spawn {
                    return Err(EngineError::BadParams { cmd: id.into(), msg: "accounted filter edits require synchronous dispatch".into() });
                }
            } else {
                crate::xcf_filter_cmds::pre_dispatch_set_change(self, id, &params, background || self.jobs.spawn)?;
            }
        }
        if id == "xcf.filter.mask.enabled.set" {
            crate::xcf_filter_mask_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.mask.geometry.set" {
            crate::xcf_mask_geometry_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.filter.insert" {
            if background || self.jobs.spawn {
                return Err(EngineError::BadParams { cmd: id.into(), msg: "filter insertion requires synchronous dispatch".into() });
            }
            crate::xcf_filter_insert_cmds::dispatch_preflight(self, &params)?;
        }
        if id == "xcf.indexed.pixel.set" {
            crate::xcf_indexed_pixel_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.indexed.palette.set" {
            crate::xcf_indexed_palette_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.text.name.set" {
            crate::xcf_text_transform_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.channel.name.set" {
            crate::xcf_channel_name_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.channel.locks.set" {
            crate::xcf_channel_locks_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.channel.indicates.set" {
            crate::xcf_channel_indicates_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.path.name.set" {
            crate::xcf_path_state_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if matches!(id, "xcf.vector.fill.color.set" | "xcf.vector.stroke.color.set") {
            if background || self.jobs.spawn {
                return Err(EngineError::BadParams { cmd: id.into(), msg: "vector color edits require synchronous dispatch".into() });
            }
            crate::xcf_vector_style_cmds::dispatch_preflight(self, id, &params)?;
        }
        if matches!(id, "xcf.filter.move" | "xcf.filter.remove") && (background || self.jobs.spawn) {
            return Err(EngineError::BadParams { cmd: id.into(), msg: "filter stack edits require synchronous dispatch".into() });
        }
        if id == "xcf.layer.affine.apply" {
            crate::xcf_wide_point_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == crate::xcf_item_set_create_cmds::CREATE {
            crate::xcf_item_set_create_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == crate::xcf_item_state_cmds::SET {
            crate::xcf_item_state_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.items.select" {
            crate::xcf_selection_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.items.visibility.set" {
            crate::xcf_selection_cmds::visibility_dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.channel.overlay.set" {
            crate::xcf_channel_state_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.mask.enabled.set" {
            crate::xcf_mask_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "layer.layerMask.enabled" {
            crate::xcf_mask_cmds::legacy_enabled_preflight(self, &params, background || self.jobs.spawn)?;
        }
        let accounted_dispatch = self.active().is_some_and(|state| state.doc.accounted().is_some()) && crate::accounted_commands::supports_for_session(self,id);
        // Validate immutable document identities once, before any enabled
        // predicate, implicit floating drop or history damage calculation.
        for state in self.documents() {
            // The typed branch retains each accounted owner and validates its
            // target through the paid snapshot API. Resident tabs still pass
            // the ordinary raster validation before any command can mutate.
            if !accounted_dispatch || state.doc.accounted().is_none() {
                state.validate_retained_surfaces().map_err(EngineError::Other)?;
            }
            if let Some(floating) = &state.floating {
                floating
                    .parts
                    .validate_retained_surfaces(&photocraft_raster::Interrupt::NONE)
                    .map_err(|error| EngineError::Other(format!("invalid floating raster: {error}")))?;
            }
        }
        if let Some(clipboard) = &self.clipboard {
            clipboard.surface.try_validate_tile_origins().map_err(|error| EngineError::Other(format!("invalid clipboard raster: {error}")))?;
        }
        // Untrusted sessions gate every command, including the ones a command runs on its own
        // behalf (`file.automate.conditionalModeChange` runs `image.mode.*`), before any side
        // effect.
        if let Some(gate) = self.authorize {
            gate(id, &params)?;
        }
        crate::xcf_link_cmds::dispatch_preflight_with_background(self, id, &params, background || self.jobs.spawn)?;
        if id == "xcf.link.substitute" {
            crate::xcf_link_substitution_cmds::dispatch_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if id == "xcf.link.recover" {
            crate::xcf_link_substitution_cmds::dispatch_recovery_preflight(self, &params, background || self.jobs.spawn)?;
        }
        if accounted_dispatch {
            if id == "edit.fill" && (background || self.jobs.spawn) {
                return Err(EngineError::BadParams { cmd: id.into(), msg: "accounted Fill requires synchronous dispatch".into() });
            }
            // The new flat composition parser is paid below on the registered
            // cumulative control. Eligibility here checks class/global context
            // only, without an unmetered duplicate parse.
            let enabled_params=if id=="xcf.composition.set" && self.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_flat_composite_storage()) {&Value::Null}else{&params};
            if let Some(why) = crate::accounted_commands::disabled_reason(self, id, enabled_params)? {
                return Err(EngineError::Disabled(id.to_string(), why.into()));
            }
            if let Some(why) = self.job_conflict(id, spec.journal) {
                return Err(EngineError::Disabled(id.to_string(), why));
            }
            if id == "select.colorRange" {
                return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.dispatch_accounted_color_range(params, background || self.jobs.spawn)
                }))
                .unwrap_or_else(|_| Err(EngineError::Other(format!("`{id}` failed with an internal error (logged); the document is unchanged"))));
            }
            // Bounded native pixel, filter and assigned-geometry property preparation share one
            // immutable registered work ledger; no per-cell allowance reset.
            let bounded_work = id=="layer.renameLayer" && self.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage())
                || matches!(id, "edit.clear" | "edit.fill" | "layer.translate")
                || id=="xcf.composition.set" && self.active().and_then(|state|state.doc.accounted()).is_some_and(|owner|owner.is_readonly_flat_composite_storage())
                || id == "xcf.mask.enabled.set"
                    && self.active().and_then(|state| state.doc.accounted()).is_some_and(crate::assigned_geometry_metadata_candidate)
                || id == "layer.setProps"
                    && self.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| {
                        owner.is_readonly_threshold_alpha_storage() || owner.is_readonly_threshold_storage() || owner.is_readonly_point_storage()
                            || crate::owned_layer_metadata_candidate(owner)
                    })
                || matches!(id, "xcf.filter.set" | "xcf.filter.enable" | "xcf.filter.move" | "xcf.filter.remove")
                    && self.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| {
                        owner.is_readonly_threshold_storage() || owner.is_readonly_point_storage()
                            || matches!(id, "xcf.filter.set" | "xcf.filter.enable")
                                && (owner.is_readonly_gaussian_group_storage() || owner.is_readonly_assigned_point_storage())
                            || matches!(id,"xcf.filter.set"|"xcf.filter.enable"|"xcf.filter.move"|"xcf.filter.remove") && owner.is_readonly_filtered_flat_point_storage()
                    });
            let registered_work = photocraft_io::accounted_operation_work::OperationWork::legacy(crate::accounted_clear::REGISTERED_WORK_LIMIT);
            // This synchronous boundary has no external Interrupt. The same
            // meter deadline remains active during interrupt-only CMS/mask loops;
            // externally controlled APIs retain their original caller control.
            let deadline = || registered_work.deadline_expired();
            let interrupt = photocraft_raster::Interrupt::cancel_only(&deadline);
            // The route probe itself debits this very ledger, including for
            // history/view commands whose old dispatch debit was a no-op.
            let probe_debit = |units| registered_work.check(units);
            let probe = photocraft_raster::backing::ReadControl { interrupt: &interrupt, work: &probe_debit };
            let pure_point_standard = matches!(id, "xcf.filter.set" | "xcf.filter.enable" | "xcf.filter.move" | "xcf.filter.remove"
                | "layer.setProps" | "layer.select" | "edit.undo" | "edit.redo" | "history.selectState")
                && if let Some(owner)=self.active().and_then(|state|state.doc.accounted()) {
                    crate::accounted_native_open::is_current_pure_point_work_candidate(owner,&probe)?
                } else { false };
            let mixed_rename_standard=id=="layer.renameLayer" && self.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage());
            // Preserve the original Box command selection exactly.
            let box_standard = (matches!(id, "xcf.filter.set" | "xcf.filter.enable")
                || id == "layer.translate" && self.active().and_then(|s|s.doc.accounted()).is_some_and(crate::observed_box_translation_candidate)
                || id == "layer.setProps" && self.active().and_then(|s|s.doc.accounted()).is_some_and(crate::accounted_session::metadata_gesture::observed_box_layer_metadata_candidate))
                && if let Some(owner)=self.active().and_then(|state|state.doc.accounted()) {
                    (owner.is_readonly_point_storage() || owner.is_readonly_assigned_point_storage())
                        && crate::accounted_native_open::is_current_box_work_candidate(owner.document(),&probe)?
                } else {false};
            if pure_point_standard || box_standard || mixed_rename_standard {
                let limits=photocraft_xcf::ResourceLimits::for_policy(Default::default()).map_err(|error|EngineError::Other(error.to_string()))?;
                registered_work.select_standard(&limits)?;
            }
            let debit = |units| {
                if !(bounded_work || pure_point_standard) { return Ok(()); }
                registered_work.check(units)
            };
            let control = photocraft_raster::backing::ReadControl { interrupt: &interrupt, work: &debit };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::accounted_commands::dispatch(self, id, &params, &control)))
                .unwrap_or_else(|_| Err(EngineError::Other(format!("`{id}` failed with an internal error (logged); the document is unchanged"))));
            // The carrier publishes a prepaid journal with the successful
            // typed edit. Its guarded contexts exclude the resident-only
            // Fade, action, slice and preset callbacks in after_command.
            return result.map(Started::Done);
        }
        if self.active().is_some_and(|state| state.doc.accounted().is_some()) {
            return Err(EngineError::Disabled(id.to_string(), "this command has not yet been qualified for an accounted native document".into()));
        }
        // Last Filter runs as the filter it repeats: journaled, recorded in an action, faded and
        // run as a job like that filter, so a recorded step replays the same filter anywhere
        // (#3089), as Transform Again records its transform (#1440). With no filter yet it falls
        // through to its own precondition.
        if id == "filter.lastFilter"
            && let Some((filter, p)) = crate::filters::last_filter(self).filter(|(f, _)| f != id)
        {
            return self.dispatch(&filter, p, background);
        }
        if id == "file.new" {
            crate::document_preset_cmds::validate_new_params(self, &params)?;
        }
        crate::param_check::check(spec, &params)?;
        crate::filters::validate_params(id, &params)?;
        // A floating selection drops before any other command (Undo puts it back instead).
        if let Some(v) = crate::float_cmds::before_command(self, id)? {
            return Ok(Started::Done(v));
        }
        // Pixel commands follow the Channels panel target unless the caller names one.
        let run_params = crate::channel_cmds::inject_target(self, id, crate::commands::inject_kind(id, params.clone()));
        if let Err(why) = self.explicit_layer_enabled(spec, &params).unwrap_or_else(|| self.precondition(spec, &run_params)) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        if let Some(why) = self.job_conflict(id, spec.journal) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        // Painting on or moving a hidden layer is refused, as in Photoshop (#571).
        if let Some(why) = crate::hidden_target::refusal(self, id, &params) {
            return Err(EngineError::Other(why.into()));
        }
        self.coalesce_request = params.get("coalesce").and_then(Value::as_str).map(str::to_string);
        self.color_restrict = crate::channel_cmds::color_restriction(self, id, &run_params);
        self.jobs.spawn = background;
        // Commands this one runs on its own behalf journal after this mark (#3067).
        let nested_from = self.journal.len();
        // A replayable command is recorded as one step, so recording pauses while it runs: an
        // action it plays (a script step) must not flush the steps it ran so far into the action
        // being recorded, nor move the recording mark past the entries it is about to replace.
        let recording = if spec.journal && crate::actions_cmds::replayable(id) { self.actions.recording.take() } else { None };
        // Last-resort guard (AGENTS.md, Never crash): a command that panics anyway fails with an
        // error instead of taking the app down. `edit` only commits a document after its closure
        // returns, so the documents are unchanged.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, &run_params)))
            .unwrap_or_else(|_| Err(EngineError::Other(format!("`{id}` failed with an internal error (logged); the document is unchanged"))));
        if self.actions.recording.is_none() {
            self.actions.recording = recording;
        }
        self.jobs.spawn = false;
        self.coalesce_request = None;
        self.color_restrict = None;
        let pending = self.jobs.pending.take();
        match (r, pending) {
            (Err(e), Some(job)) => {
                job.ctx.cancel();
                Err(e)
            }
            (Err(e), None) => Err(e),
            (Ok(_), Some(mut job)) => {
                job.command = id.to_string();
                job.params = params;
                job.journal = spec.journal;
                let jid = job.id;
                self.jobs.running.push(job);
                Ok(Started::Job(jid))
            }
            (Ok(v), None) => {
                // These commands inspect the currently installed snapshot.
                // View repair, preset persistence and edit bookkeeping belong
                // to an edit event, never to a read-only query.
                if xcf_query || matches!(id, "document.pixel.inspect" | "xcf.filter.graph.inspectPlan") {
                    return Ok(Started::Done(v));
                }
                self.after_command_result_nested(id, params, spec.journal, &v, Some(nested_from))?;
                Ok(Started::Done(v))
            }
        }
    }

    /// Bookkeeping after a command (or a job's apply) succeeded.
    /// Nested replayable commands replace the steps they dispatched in the journal.
    fn after_command_nested(&mut self, id: &str, params: Value, journal: bool, nested_from: Option<usize>) -> Result<()> {
        // A layer-mask view ends when another layer becomes active (#196).
        if let Some(st) = self.active_mut() {
            crate::mask_view_cmds::fix(st);
        }
        crate::edit_menu_cmds::after_command(self, id)?;
        crate::automate_cmds::after_command(self, id)?;
        self.sync_preset_store();
        if !journal {
            return Ok(());
        }
        if let Some(from) = nested_from.filter(|_| crate::actions_cmds::replayable(id)) {
            self.journal.truncate(from);
        }
        if !crate::brush_cmds::coalesce_journal(self, id, &params) {
            self.journal.push((id.to_string(), params));
        }
        Ok(())
    }

    #[cfg(test)]
    fn after_command_result(&mut self, id: &str, params: Value, journal: bool, result: &Value) -> Result<()> {
        self.after_command_result_nested(id, params, journal, result, None)
    }
    /// Cross-document channel edits publish their destination without
    /// switching tabs. Repair derived view/slice state on that destination.
    fn after_command_result_nested(&mut self, id: &str, params: Value, journal: bool, result: &Value, nested_from: Option<usize>) -> Result<()> {
        if !matches!(id, "channel.duplicate" | "select.saveSelection") {
            return self.after_command_nested(id, params, journal, nested_from);
        }
        let index = result.get("document").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
            .filter(|i| self.docs.get(*i).is_some())
            .ok_or_else(|| EngineError::Other(format!("`{id}` returned an invalid destination")))?;
        let viewed = self.active;
        self.active = Some(index);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.after_command_nested(id, params, journal, nested_from)))
            .unwrap_or_else(|_| Err(EngineError::Other(format!("`{id}` bookkeeping failed with an internal error"))));
        self.active = viewed;
        outcome
    }

    /// Apply (or report the failure of) a job whose worker has delivered `r`.
    fn finish_job(&mut self, mut job: Running, r: Result<Payload>) {
        let apply = job.apply.take();
        let outcome = match r {
            Err(EngineError::Cancelled) => JobOutcome::Cancelled,
            Err(e) => JobOutcome::Failed(e.to_string()),
            Ok(_) if job.ctx.cancelled() => JobOutcome::Cancelled,
            Ok(payload) => match self.apply_job(&job, apply, payload) {
                Ok(v) => JobOutcome::Done(v),
                Err(EngineError::Cancelled) => JobOutcome::Cancelled,
                Err(e) => JobOutcome::Failed(e.to_string()),
            },
        };
        self.end_job(&job, outcome);
    }

    fn apply_job(&mut self, job: &Running, apply: Option<Apply>, payload: Payload) -> Result<Value> {
        let Running { label, doc, snapshot, coalesce, color_restrict, command, params, journal, .. } = job;
        let prev_active = self.active;
        let target = match doc {
            Some(d) => {
                let i =
                    self.documents().iter().position(|st| st.doc.id == *d).ok_or_else(|| EngineError::Other(format!("`{label}`: the document was closed")))?;
                let unchanged = snapshot.as_ref().is_some_and(|w| self.documents().get(i).is_some_and(|st| w.is_current(&st.doc)));
                if !unchanged {
                    return Err(EngineError::Other(format!("`{label}`: the document changed while it ran; the result was discarded")));
                }
                self.active = Some(i);
                Some(i)
            }
            None => None,
        };
        self.coalesce_request = coalesce.clone();
        self.color_restrict = *color_restrict;
        // Apply-time hooks can dispatch commands too. Keep the same single-step recording
        // contract as synchronous dispatch without truncating work done while the worker ran.
        let nested_from = self.journal.len();
        let recording = if !job.prepaid_bookkeeping && *journal && crate::actions_cmds::replayable(command) { self.actions.recording.take() } else { None };
        let r = match apply {
            Some(f) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, payload)))
                .unwrap_or_else(|_| Err(EngineError::Other(format!("`{label}` failed with an internal error (logged); the document is unchanged")))),
            None => Err(EngineError::Other("internal error: job applied twice".into())),
        };
        if self.actions.recording.is_none() {
            self.actions.recording = recording;
        }
        self.coalesce_request = None;
        self.color_restrict = None;
        // Finish successful command bookkeeping on the document the job edited.
        // Restoring the viewed document first would attribute Fade and slice updates
        // to that document instead, when the user switched tabs mid-job.
        let bookkeeping = if !job.prepaid_bookkeeping && r.is_ok()
            && !matches!(
                command.as_str(),
                "document.pixel.inspect"
                    | "xcf.assess"
                    | "xcf.inspect"
                    | "xcf.source.inspect"
                    | "xcf.composition.inspect"
                    | "xcf.filter.inspect"
                    | "xcf.resource.inspect"
                    | "xcf.filter.graph.inspectPlan"
            )
        {
            match &r {
                Ok(value) => self.after_command_result_nested(command, params.clone(), *journal, value, Some(nested_from)),
                Err(_) => Ok(()),
            }
        } else {
            Ok(())
        };
        // Keep the user's active document unless the job opened a new one.
        // This also restores the selection when the apply step returned an error.
        if target.is_some() && self.active == target {
            self.active = prev_active.filter(|i| *i < self.docs.len()).or(self.active);
        }
        bookkeeping?;
        r
    }

    fn end_job(&mut self, job: &Running, outcome: JobOutcome) {
        let (state, result, error) = match &outcome {
            JobOutcome::Done(v) => ("done", Some(v.clone()), None),
            JobOutcome::Failed(e) => ("failed", None, Some(e.clone())),
            JobOutcome::Cancelled => ("cancelled", None, None),
        };
        let mut info = job.info();
        info.state = state;
        info.result = result;
        info.error = error;
        if state == "done" {
            info.progress = 1.0;
        }
        self.jobs.recent.push(info);
        if self.jobs.recent.len() > RECENT {
            let extra = self.jobs.recent.len() - RECENT;
            self.jobs.recent.drain(..extra);
        }
        self.jobs.events.push(JobEvent { id: job.id, command: job.command.clone(), label: job.label.clone(), document: job.doc, outcome });
    }
}

/// Engine commands for the job table (the control channel and MCP use them too).
pub fn specs() -> Vec<crate::commands::CommandSpec> {
    use crate::commands::CommandSpec;
    vec![
        CommandSpec {
            id: "jobs.list",
            label: "List Background Jobs",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: |_| Ok(()),
            run: |s, _| Ok(json!({"jobs": s.jobs_with_recent()})),
            journal: false,
        },
        CommandSpec {
            id: "jobs.cancel",
            label: "Cancel Background Job",
            menu: &[],
            shortcut: None,
            params: r##"{"job":u64? (default: every running job)}"##,
            enabled: |_| Ok(()),
            run: |s, p| match p.get("job") {
                None | Some(Value::Null) => {
                    let n = s.jobs.running.len();
                    s.cancel_all_jobs();
                    Ok(json!({"cancelled": n.saturating_sub(s.jobs.running.len())}))
                }
                Some(v) => {
                    let id = v.as_u64().ok_or_else(|| EngineError::BadParams { cmd: "jobs.cancel".into(), msg: "`job` must be a job id".into() })?;
                    if s.cancel_job(JobId(id)) {
                        Ok(json!({"cancelled": 1}))
                    } else if s.job(JobId(id)).is_some() {
                        Err(EngineError::Other("printing cannot be cancelled here; use the system print queue".into()))
                    } else {
                        Err(EngineError::Other(format!("no running job {id}")))
                    }
                }
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod destination_bookkeeping_tests {
    use super::*;
    use photocraft_doc::item_identity::LayerId;
    use photocraft_doc::slices::{Slice, SliceOrigin};
    use photocraft_geom::Rect;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn exhausted_job_identity_refuses_before_starting_worker() {
        let mut session = Session::new();
        session.jobs.next = u64::MAX;
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_called = called.clone();
        let outcome = session.start_job("own.identity.exhaustion", Value::Null, "Own exhausted job", false,
            move |_| {
                worker_called.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
            |_, ()| Ok(Value::Null));
        assert!(outcome.is_err());
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.jobs.next, u64::MAX);
        assert!(!session.has_jobs());
    }

    #[test]
    fn duplicate_repairs_destination_slices_without_touching_source_or_focus() {
        let mut session = Session::new();
        for name in ["destination", "source"] {
            session.execute("file.new", json!({"name":name,"width":2,"height":2})).unwrap();
            session.active_mut().unwrap().doc.try_resident_mut().unwrap().slices.list.push(Slice {
                id: 1,
                origin: SliceOrigin::Layer,
                layer: Some(LayerId(u64::MAX)),
                rect: Rect::new(0, 0, 1, 1),
                ..Slice::default()
            });
        }
        let source = session.documents()[1].doc.clone();
        session.after_command_result("channel.duplicate", json!({"document":0}), true, &json!({"document":0,"channel":0})).unwrap();
        assert_eq!(session.active_index(), Some(1));
        assert!(session.documents()[0].doc.slices.list.is_empty());
        assert!(session.documents()[1].doc.shares_state(&source));
        assert_eq!(session.journal.last().unwrap().0, "channel.duplicate");
    }
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus"))]
#[path = "jobs_point_tests.rs"]
mod point_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_path_stream_drafts {
    use super::*;
    #[test]
    fn cancelled_path_worker_refuses_before_opening_any_file() {
        let context = JobCtx::new();
        context.cancel();
        assert!(matches!(read_path("own.pcraft", "own-nonexistent-no-read-authority.pcraft", &context), Err(EngineError::Cancelled)));
    }
}
