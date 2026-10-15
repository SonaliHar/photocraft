//! Window › Timelapse: records a painting session as numbered JPEG frames and renders them into
//! a video with ffmpeg.
//!
//! - A frame is considered when the document's revision moved and at least the interval has passed
//!   since the last frame. The worker writes it only when the picture changed: when the layers
//!   differ from the last frame's (a stroke, a filter, an undo), not for a selection, a guide,
//!   selecting a layer or showing a reference, and a skipped check doesn't restart the interval.
//!   It renders a copy of the document with every layer or
//!   group whose name starts with the reference prefix (default `REF_`) hidden, so references
//!   never appear in the video and the real document, its history and its saved state are left
//!   alone.
//! - Frames are flattened and shrunk to the video's size (at most [`VIDEO_MAX_SIDE`] on the long
//!   side) before they're saved, so a long session stays small: about 0.2–0.5 MB a frame.
//! - Frames and `session.json` live in `<document folder>/<document name>_timelapse/`, the same
//!   layout as the PS Timelapse UXP plug-in, so a session resumes across launches (and continues
//!   one started in Photoshop).
//! - Export runs ffmpeg from `PATH` in the background: `full_timelapse.mp4` (every frame at
//!   24 fps) and `sped_up_30s.mp4` (the whole session squeezed into 30 seconds).
//!
//! The recorder lives in [`crate::state::UiState::timelapse`]; everything else is in this module.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{RichText, vec2};
use photocraft_doc::{DocId, Document, Layer, LayerContent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

const MENU_ID: &str = "window.timelapse";
const FULL_FPS: u32 = 24;
const SPED_UP_SECONDS: f64 = 30.0;
/// Longest side of the exported video; frames keep the document's full size.
const VIDEO_MAX_SIDE: u32 = 1920;
/// Frames are re-encoded once more by ffmpeg, so 85 loses nothing visible and is about half of 95.
const FRAME_JPEG_QUALITY: u8 = 85;
const LOG_LINES: usize = 40;

/// `session.json`, field-compatible with the PS Timelapse plug-in. Unknown fields are kept.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    frame_count: u32,
    #[serde(default = "default_interval")]
    interval_seconds: u32,
    #[serde(default = "default_tag")]
    reference_tag: String,
    #[serde(flatten)]
    other: serde_json::Map<String, Value>,
}

fn default_interval() -> u32 {
    20
}

fn default_tag() -> String {
    "REF_".into()
}

impl SessionFile {
    fn fresh() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        Self {
            session_id: format!("{}_{:06x}", now.as_millis(), now.subsec_nanos() & 0xff_ffff),
            created_at: format!("{}", now.as_secs()),
            frame_count: 0,
            interval_seconds: default_interval(),
            reference_tag: default_tag(),
            other: serde_json::Map::new(),
        }
    }
}

/// The document a session records and where its frames go.
struct Attached {
    doc: DocId,
    name: String,
    folder: PathBuf,
    state: SessionFile,
    /// Bytes the session's frames take on disk.
    bytes: u64,
}

/// What a frame worker did.
enum FrameDone {
    /// Wrote the frame: its size in bytes, and what it showed (to compare the next one with).
    Written(u64, Arc<Document>),
    /// Nothing visible changed since the last frame, so none was written.
    Unchanged,
}

/// Where a frame worker leaves its result.
type FrameSlot = Arc<Mutex<Option<Result<FrameDone, String>>>>;

#[derive(Default)]
struct Recorder {
    open: bool,
    recording: bool,
    session: Option<Attached>,
    last_revision: u64,
    /// What the last frame showed (references hidden), to skip frames that would look the same.
    last_frame: Option<Arc<Document>>,
    last_capture: Option<Instant>,
    /// A frame being rendered: its number, the previous frame's time (restored when nothing
    /// changed) and the worker's result once done.
    capture: Option<(u32, Option<Instant>, FrameSlot)>,
    /// An ffmpeg export in progress: its latest status and whether it finished.
    export: Option<Arc<Mutex<(String, bool)>>>,
    /// Settings being edited in the panel (applied to the session on change).
    interval_edit: f32,
    tag_edit: String,
    log: Vec<String>,
    /// Take a frame as soon as no other is being written (Capture Frame Now, recording start).
    pending_capture: bool,
}

/// The app's recorder: shared by clones of the UI state, never saved with it, and equal to any
/// other (it's not part of the state that's compared or restored).
#[derive(Clone, Default)]
pub struct Timelapse(Arc<Mutex<Recorder>>);

impl Timelapse {
    fn handle(app: &PhotocraftApp) -> Arc<Mutex<Recorder>> {
        app.ui.timelapse.0.clone()
    }
}

impl std::fmt::Debug for Timelapse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Timelapse")
    }
}

impl PartialEq for Timelapse {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

fn lock_status(s: &Mutex<(String, bool)>) -> std::sync::MutexGuard<'_, (String, bool)> {
    s.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock(r: &Mutex<Recorder>) -> std::sync::MutexGuard<'_, Recorder> {
    r.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Recorder {
    fn log(&mut self, msg: impl Into<String>) {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
        self.log.insert(0, format!("[{h:02}:{m:02}:{s:02} UTC] {}", msg.into()));
        self.log.truncate(LOG_LINES);
    }

    /// Attach to the active document, loading or starting its session folder.
    fn attach(&mut self, app: &PhotocraftApp) -> Result<(), String> {
        let st = app.session.active().ok_or("No document is open.")?;
        if self.session.as_ref().is_some_and(|s| s.doc == st.doc.id) {
            return Ok(());
        }
        let path = st.path.as_deref().ok_or("Save the document first: frames are stored next to its file.")?;
        let (folder, name) = session_folder(Path::new(path)).ok_or("Could not work out a folder next to the document.")?;
        std::fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
        let file = folder.join("session.json");
        let mut state = match std::fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str::<SessionFile>(&text).map_err(|e| format!("session.json is unreadable ({e}); move it aside to start over."))?,
            Err(_) => SessionFile::fresh(),
        };
        // Never overwrite frames a stale count doesn't know about.
        state.frame_count = state.frame_count.max(highest_frame(&folder));
        let resumed = state.frame_count;
        self.interval_edit = state.interval_seconds as f32;
        self.tag_edit = state.reference_tag.clone();
        let bytes = frames_bytes(&folder);
        self.session = Some(Attached { doc: st.doc.id, name, folder, state, bytes });
        self.last_revision = st.revision;
        self.last_frame = None;
        self.last_capture = None;
        self.save()?;
        self.log(if resumed > 0 { format!("Resumed session: {resumed} frame(s) already captured.") } else { "Started a new timelapse session.".into() });
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        let Some(s) = &self.session else { return Ok(()) };
        let text = serde_json::to_string_pretty(&s.state).map_err(|e| e.to_string())?;
        std::fs::write(s.folder.join("session.json"), text).map_err(|e| format!("Could not write session.json: {e}"))
    }

    fn set_recording(&mut self, app: &PhotocraftApp, on: bool) {
        if on == self.recording {
            return;
        }
        if on {
            if let Err(e) = self.attach(app) {
                self.log(format!("Cannot start: {e}"));
                return;
            }
            self.recording = true;
            // The first frame shows where the session (re)starts.
            self.pending_capture = true;
            self.log("Recording started.");
        } else {
            self.recording = false;
            self.log("Recording stopped.");
        }
    }

    /// Collect a finished frame, then start the next one when it's due.
    fn tick(&mut self, app: &PhotocraftApp, ctx: &egui::Context, force: bool) {
        if let Some((n, before, slot)) = &self.capture {
            let done = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
            match done {
                None => return,
                Some(result) => {
                    let (n, before) = (*n, *before);
                    self.capture = None;
                    match result {
                        Ok(FrameDone::Written(size, frame)) => {
                            if let Some(s) = &mut self.session {
                                s.state.frame_count = s.state.frame_count.max(n);
                                s.bytes = s.bytes.saturating_add(size);
                            }
                            self.last_frame = Some(frame);
                            if let Err(e) = self.save() {
                                self.log(e);
                            }
                            self.log(format!("Captured frame {n}."));
                        }
                        Ok(FrameDone::Unchanged) => self.last_capture = before,
                        Err(e) => self.log(format!("Frame {n} failed: {e}")),
                    }
                }
            }
        }
        if !self.recording && !force {
            return;
        }
        let Some(session) = &self.session else { return };
        let Some(st) = app.session.documents().iter().find(|d| d.doc.id == session.doc) else {
            self.recording = false;
            self.session = None;
            self.log("The recorded document was closed; recording stopped.");
            return;
        };
        let interval = Duration::from_secs(u64::from(session.state.interval_seconds.max(1)));
        if !force {
            if st.revision == self.last_revision {
                return;
            }
            if let Some(wait) = self.last_capture.map(|t| interval.saturating_sub(t.elapsed())).filter(|w| !w.is_zero()) {
                // Changed, but too soon: look again once the interval is up.
                ctx.request_repaint_after(wait);
                return;
            }
        }
        let n = session.state.frame_count + 1;
        let path = session.folder.join(frame_name(n));
        let tag = session.state.reference_tag.clone();
        let doc = st.doc.clone();
        // A forced frame is always written; a timed one only when the picture changed.
        let previous = if force { None } else { self.last_frame.clone() };
        let slot = Arc::new(Mutex::new(None));
        let shared = slot.clone();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new().name("timelapse-frame".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture_frame(&doc, previous.as_deref(), &tag, &path)))
                .unwrap_or_else(|_| Err("frame worker panicked".into()));
            *shared.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
            repaint.request_repaint();
        });
        self.last_revision = st.revision;
        let before = self.last_capture.replace(Instant::now());
        match spawned {
            Ok(_) => self.capture = Some((n, before, slot)),
            Err(e) => self.log(format!("Could not start the frame worker: {e}")),
        }
    }

    fn start_export(&mut self, app: &PhotocraftApp) {
        if self.export.as_ref().is_some_and(|e| !lock_status(e).1) {
            return;
        }
        let Some(s) = &self.session else {
            self.log("Start or resume a session first.");
            return;
        };
        let frames = s.state.frame_count;
        if frames < 2 {
            self.log("Capture at least two frames before exporting.");
            return;
        }
        let size = app.session.documents().iter().find(|d| d.doc.id == s.doc).map(|d| d.doc.bounds()).map(|b| (b.width().max(1), b.height().max(1)));
        let (w, h) = video_size(size.unwrap_or((VIDEO_MAX_SIDE, VIDEO_MAX_SIDE)));
        let folder = s.folder.clone();
        let status = Arc::new(Mutex::new(("Rendering the full timelapse…".to_string(), false)));
        let shared = status.clone();
        let spawned = std::thread::Builder::new().name("timelapse-export".into()).spawn(move || {
            let set = |msg: String, done: bool| *shared.lock().unwrap_or_else(PoisonError::into_inner) = (msg, done);
            for (i, (out, args)) in ffmpeg_jobs(frames, w, h).into_iter().enumerate() {
                if i == 1 {
                    set("Rendering the 30-second version…".into(), false);
                }
                if let Err(e) = run_ffmpeg(&folder, &args) {
                    set(format!("{out}: {e}"), true);
                    return;
                }
            }
            set(format!("Done: full_timelapse.mp4 and sped_up_30s.mp4 in {}", folder.display()), true);
        });
        match spawned {
            Ok(_) => {
                self.log(format!("Exporting {frames} frames at {w}×{h}."));
                self.export = Some(status);
            }
            Err(e) => self.log(format!("Could not start the export: {e}")),
        }
    }
}

/// `<folder>/<stem>_timelapse` for a document saved at `path`, and the document's display name.
fn session_folder(path: &Path) -> Option<(PathBuf, String)> {
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let parent = path.parent()?;
    Some((parent.join(format!("{stem}_timelapse")), path.file_name()?.to_string_lossy().into_owned()))
}

fn frame_name(n: u32) -> String {
    format!("frame_{n:06}.jpg")
}

/// The highest `frame_NNNNNN.jpg` number in `folder` (0 when there is none).
fn highest_frame(folder: &Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(folder) else { return 0 };
    entries.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str()?.strip_prefix("frame_")?.strip_suffix(".jpg")?.parse::<u32>().ok()).max().unwrap_or(0)
}

/// Bytes taken by the `frame_*.jpg` files in `folder`.
fn frames_bytes(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("frame_") && n.ends_with(".jpg")))
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .fold(0, u64::saturating_add)
}

/// `bytes` for people: "840 KB", "148 MB", "2.4 GB".
fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

/// Hide every layer or group whose name starts with `tag` (case-insensitive), at any depth.
fn hide_references(layers: &mut [Layer], tag: &str) -> usize {
    let mut hidden = 0;
    for layer in layers {
        if layer.name.to_uppercase().starts_with(tag) {
            if layer.visible {
                layer.visible = false;
                hidden += 1;
            }
        } else if let LayerContent::Group(g) = &mut layer.content {
            hidden += hide_references(&mut g.children, tag);
        }
    }
    hidden
}

/// The document as one timelapse frame shows it: references hidden.
fn frame_document(doc: &Document, tag: &str) -> Document {
    let mut copy = doc.clone();
    let tag = tag.trim().to_uppercase();
    if !tag.is_empty() {
        hide_references(&mut copy.layers, &tag);
    }
    copy
}

/// `doc` flattened and fitted inside `max_side` pixels (never enlarged), through the same engine
/// commands as Export As, so depth, colour mode and profile are handled as a real export.
fn shrunk(doc: Document, max_side: u32) -> Result<Document, String> {
    let (w, h) = (doc.size.width.max(1), doc.size.height.max(1));
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    session.execute("layer.flattenImage", json!({})).map_err(|e| e.to_string())?;
    let scale = f64::from(max_side) / f64::from(w.max(h));
    if scale < 1.0 {
        let fit = |v: u32| (f64::from(v) * scale).round().max(1.0);
        session.execute("image.imageSize", json!({"width": fit(w), "height": fit(h), "resample": "bicubic"})).map_err(|e| e.to_string())?;
    }
    session.active().map(|d| (*d.doc).clone()).ok_or_else(|| "the frame document went missing".into())
}

/// Whether two frames would look the same: same canvas and identical layers. Selections, guides,
/// paths and the like live on the document too but never show in a frame.
fn looks_the_same(a: &Document, b: &Document) -> bool {
    a.size == b.size && a.mode == b.mode && a.depth == b.depth && a.icc_profile == b.icc_profile && a.layers == b.layers
}

/// Writes the frame for `doc` unless it would look exactly like `previous`.
fn capture_frame(doc: &Document, previous: Option<&Document>, tag: &str, path: &Path) -> Result<FrameDone, String> {
    let frame = frame_document(doc, tag);
    if previous.is_some_and(|p| looks_the_same(p, &frame)) {
        return Ok(FrameDone::Unchanged);
    }
    let size = write_frame(&frame, path)?;
    Ok(FrameDone::Written(size, Arc::new(frame)))
}

/// Writes one frame (references already hidden) and returns its size in bytes.
fn write_frame(frame: &Document, path: &Path) -> Result<u64, String> {
    let frame = shrunk(frame.clone(), VIDEO_MAX_SIDE)?;
    let mut opts = photocraft_io::ExportOptions::default();
    opts.encode.jpeg_quality = FRAME_JPEG_QUALITY;
    let bytes = photocraft_io::export(&frame, "jpg", &opts).map_err(|e| e.to_string())?.bytes;
    // Write beside the target and rename, so an interrupted write never leaves a broken frame.
    let tmp = path.with_extension("jpg.part");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))?;
    Ok(bytes.len() as u64)
}

/// The video frame size: the document fitted inside [`VIDEO_MAX_SIDE`], never enlarged, with even
/// sides (H.264 4:2:0 needs them).
fn video_size((w, h): (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(VIDEO_MAX_SIDE) / f64::from(w.max(h))).min(1.0);
    let even = |v: u32| ((f64::from(v) * scale).round() as u32 / 2 * 2).max(2);
    (even(w), even(h))
}

/// The two ffmpeg runs: (output name, arguments). Frames of another size (a canvas resized
/// mid-session) are fitted and letterboxed to `w`×`h`.
fn ffmpeg_jobs(frames: u32, w: u32, h: u32) -> Vec<(&'static str, Vec<String>)> {
    let fit = format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:white,setsar=1");
    let sped_fps = (f64::from(frames) / SPED_UP_SECONDS).max(0.1);
    let job = |rate: String, extra: &[&str], out: &str| {
        let mut args: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error", "-framerate"].iter().map(|s| s.to_string()).collect();
        args.push(rate);
        args.extend(["-i", "frame_%06d.jpg", "-vf", &fit, "-c:v", "libx264", "-pix_fmt", "yuv420p"].iter().map(|s| s.to_string()));
        args.extend(extra.iter().map(|s| s.to_string()));
        args.push(out.to_string());
        args
    };
    vec![
        ("full_timelapse.mp4", job(FULL_FPS.to_string(), &[], "full_timelapse.mp4")),
        ("sped_up_30s.mp4", job(format!("{sped_fps:.3}"), &["-r", "30", "-t", "30"], "sped_up_30s.mp4")),
    ]
}

fn run_ffmpeg(folder: &Path, args: &[String]) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(args).current_dir(folder).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "ffmpeg was not found. Install it (Windows: winget install ffmpeg, macOS: brew install ffmpeg) and restart PhotoCraft.".to_string()
        } else {
            format!("could not run ffmpeg: {e}")
        }
    })?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("ffmpeg failed: {}", err.lines().last().unwrap_or("no output").trim()))
    }
}

// ------------------------------------------------------------------ menu hooks

/// Ids this module owns.
pub fn handles(id: &str) -> bool {
    id == MENU_ID
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    handles(id).then(|| lock(&app.ui.timelapse.0).open)
}

/// Window › Timelapse toggles the panel; `{"record": bool}` starts or stops recording,
/// `{"capture": true}` takes a frame now and `{"export": true}` starts the video export (for the
/// control channel; the export's progress shows in the panel).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !handles(id) {
        return None;
    }
    let handle = Timelapse::handle(app);
    let mut r = lock(&handle);
    if let Some(on) = params.get("record").and_then(Value::as_bool) {
        r.open = true;
        r.set_recording(app, on);
    } else if params.get("capture").and_then(Value::as_bool) == Some(true) {
        if let Err(e) = r.attach(app) {
            return Some(Err(e));
        }
        r.pending_capture = true;
    } else if params.get("export").and_then(Value::as_bool) == Some(true) {
        if let Err(e) = r.attach(app) {
            return Some(Err(e));
        }
        r.open = true;
        r.start_export(app);
    } else {
        r.open = params.get("show").and_then(Value::as_bool).unwrap_or(!r.open);
    }
    let frames = r.session.as_ref().map_or(0, |s| s.state.frame_count);
    Some(Ok(json!({"open": r.open, "recording": r.recording, "frames": frames})))
}

// ------------------------------------------------------------------ panel

/// Start a requested frame once the previous one is written.
fn take_pending(r: &mut Recorder, app: &PhotocraftApp, ctx: &egui::Context) {
    if r.pending_capture && r.capture.is_none() {
        r.pending_capture = false;
        r.tick(app, ctx, true);
    }
}

/// Runs the recorder and draws its panel. Called every frame.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let handle = Timelapse::handle(app);
    let mut r = lock(&handle);
    r.tick(app, ctx, false);
    take_pending(&mut r, app, ctx);
    let exporting = r.export.as_ref().is_some_and(|e| !lock_status(e).1);
    if exporting || r.capture.is_some() || r.pending_capture {
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    if !r.open {
        return;
    }
    let t = Tokens::get(ctx);
    let mut close = false;
    let mut toggle = false;
    let mut capture_now = false;
    let mut export = false;
    let mut reveal = false;
    let mut settings_changed = false;
    let r = &mut *r;
    crate::analysis_ui::panel_window(app, ctx, "timelapse", tl!("Timelapse"), vec2(-40.0, 120.0), 280.0, |ui| {
        close = crate::analysis_ui::title_row(ui, tl!("Timelapse"));
        let (name, frames, bytes) = r.session.as_ref().map_or((tl!("no session").to_string(), 0, 0), |s| (s.name.clone(), s.state.frame_count, s.bytes));
        let row = |ui: &mut egui::Ui, label: &str, value: String| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(value).color(t.text)));
            });
        };
        row(ui, tl!("Document"), name);
        row(ui, tl!("Frames"), if bytes > 0 { format!("{frames} · {}", human_size(bytes)) } else { frames.to_string() });
        let state = if r.capture.is_some() {
            tl!("capturing…")
        } else if r.recording {
            tl!("recording")
        } else {
            tl!("stopped")
        };
        row(ui, tl!("State"), state.to_string());
        ui.add_space(6.0);
        let label = if r.recording { tl!("Stop Recording") } else { tl!("Start Recording") };
        if crate::widgets::primary_button(ui, label, ui.available_width()).clicked() {
            toggle = true;
        }
        if crate::widgets::secondary_button(ui, tl!("Capture Frame Now"), ui.available_width()).clicked() {
            capture_now = true;
        }
        ui.add_space(6.0);
        crate::widgets::section_label(ui, tl!("Min seconds between frames"));
        if crate::widgets::value_field(ui, &mut r.interval_edit, 1.0..=3600.0, "s", 80.0).changed() {
            settings_changed = true;
        }
        crate::widgets::section_label(ui, tl!("Reference layer prefix (not recorded)"));
        if crate::widgets::text_field(ui, egui::Id::new("timelapse-ref-tag"), &mut r.tag_edit, "REF_", ui.available_width(), true).changed() {
            settings_changed = true;
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(!exporting && frames >= 2, |ui| {
            if crate::widgets::secondary_button(ui, tl!("Export Video (MP4 + 30 s)"), ui.available_width()).clicked() {
                export = true;
            }
        });
        if let Some(status) = &r.export {
            let (msg, _) = lock_status(status).clone();
            ui.label(RichText::new(msg).color(t.text_dim).size(11.0));
        }
        ui.add_enabled_ui(r.session.is_some(), |ui| {
            if crate::widgets::secondary_button(ui, tl!("Open Frames Folder"), ui.available_width()).clicked() {
                reveal = true;
            }
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(110.0).auto_shrink([false, true]).show(ui, |ui| {
            for line in &r.log {
                ui.label(RichText::new(line).color(t.text_dim).size(10.5).monospace());
            }
        });
    });
    if close {
        r.open = false;
    }
    if settings_changed {
        let tag = r.tag_edit.trim().to_string();
        let interval = r.interval_edit.round().clamp(1.0, 3600.0) as u32;
        if let Some(s) = &mut r.session {
            s.state.interval_seconds = interval;
            s.state.reference_tag = tag;
        }
        if let Err(e) = r.save() {
            r.log(e);
        }
    }
    if toggle {
        let on = !r.recording;
        r.set_recording(app, on);
    }
    if capture_now {
        match r.attach(app) {
            Ok(()) => r.pending_capture = true,
            Err(e) => r.log(format!("Cannot capture: {e}")),
        }
    }
    take_pending(r, app, ctx);
    if export {
        r.start_export(app);
    }
    if reveal && let Some(s) = &r.session {
        let folder = s.folder.to_string_lossy().into_owned();
        let opened = app.services.open_url.as_ref().map(|open| open(&folder));
        if let Some(Err(e)) = opened {
            r.log(format!("Could not open the folder: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, visible: bool) -> Layer {
        let mut l = Layer::new(name, LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: true, artboard: None }));
        l.visible = visible;
        l
    }

    #[test]
    fn references_are_hidden_at_any_depth() {
        let mut group = named("Sketches", true);
        if let LayerContent::Group(g) = &mut group.content {
            g.children = vec![named("ref_pose", true), named("Lines", true)];
        }
        let mut layers = vec![named("REF_photo", true), group, named("Paint", true), named("REF_off", false)];
        assert_eq!(hide_references(&mut layers, "REF_"), 2);
        assert!(!layers[0].visible);
        assert!(layers[2].visible);
        assert!(matches!(&layers[1].content, LayerContent::Group(g) if !g.children[0].visible && g.children[1].visible));
    }

    #[test]
    fn a_frame_leaves_out_the_reference_and_the_document_alone() {
        let mut doc = Document::new("t", photocraft_geom::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let fmt = doc.pixel_format();
        let solid = |name: &str, rgb: [f32; 3]| {
            let mut layer = Layer::raster(name, fmt);
            let mut px = vec![1.0; fmt.channels()];
            px[..3].copy_from_slice(&rgb);
            if let LayerContent::Raster(s) = &mut layer.content {
                s.fill_rect(doc.bounds(), &px);
            }
            layer
        };
        // Bottom first: the reference sits on top of the paint.
        doc.layers = vec![solid("Paint", [1.0, 0.0, 0.0]), solid("ref_photo", [0.0, 0.0, 1.0])];
        let shown = photocraft_compose::flatten(&doc).get(4, 4);
        let frame = photocraft_compose::flatten(&frame_document(&doc, "REF_")).get(4, 4);
        assert_ne!(shown, frame, "the reference covers the paint in the document");
        assert!(frame[0] > 0.9 && frame[2] < 0.1, "frame shows the paint: {frame:?}");
        assert!(doc.layers.iter().all(|l| l.visible), "the document itself is untouched");
    }

    #[test]
    fn frames_are_flattened_and_shrunk_without_the_reference() {
        let mut doc = Document::new("t", photocraft_geom::Size::new(400, 100), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U16);
        let fmt = doc.pixel_format();
        let solid = |name: &str, rgb: [f32; 3]| {
            let mut layer = Layer::raster(name, fmt);
            let mut px = vec![1.0; fmt.channels()];
            px[..3].copy_from_slice(&rgb);
            if let LayerContent::Raster(s) = &mut layer.content {
                s.fill_rect(doc.bounds(), &px);
            }
            layer
        };
        doc.layers = vec![solid("Paint", [1.0, 0.0, 0.0]), solid("Paint 2", [1.0, 0.0, 0.0]), solid("REF_photo", [0.0, 0.0, 1.0])];
        let frame = shrunk(frame_document(&doc, "REF_"), 40).unwrap();
        assert_eq!((frame.size.width, frame.size.height), (40, 10));
        assert_eq!(frame.layers.len(), 1, "flattened");
        let px = photocraft_compose::flatten(&frame).get(20, 5);
        assert!(px[0] > 0.9 && px[2] < 0.1, "the reference stays out: {px:?}");
        let small = Document::new("s", photocraft_geom::Size::new(30, 20), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let kept = shrunk(small, 40).unwrap();
        assert_eq!((kept.size.width, kept.size.height), (30, 20), "never enlarged");
    }

    #[test]
    fn only_a_visible_change_makes_a_new_frame() {
        let mut doc = Document::new("t", photocraft_geom::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let fmt = doc.pixel_format();
        doc.layers = vec![Layer::raster("Paint", fmt), named("REF_photo", true)];
        let dir = std::env::temp_dir().join(format!("photocraft-timelapse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(frame_name(1));
        let Ok(FrameDone::Written(_, first)) = capture_frame(&doc, None, "REF_", &path) else { panic!("first frame is written") };

        // A selection and hiding the reference change the document, not the picture.
        let mut edited = doc.clone();
        edited.selection = Some(photocraft_raster::Surface::new(fmt));
        edited.layers[1].visible = false;
        assert!(matches!(capture_frame(&edited, Some(&first), "REF_", &path), Ok(FrameDone::Unchanged)));

        // A stroke does.
        if let LayerContent::Raster(s) = &mut edited.layers[0].content {
            s.fill_rect(photocraft_geom::Rect::new(0, 0, 2, 2), &[1.0, 0.0, 0.0, 1.0]);
        }
        assert!(matches!(capture_frame(&edited, Some(&first), "REF_", &path), Ok(FrameDone::Written(..))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human_size(840_000), "840 KB");
        assert_eq!(human_size(148_200_000), "148 MB");
        assert_eq!(human_size(2_400_000_000), "2.4 GB");
    }

    #[test]
    fn session_folder_sits_next_to_the_file() {
        let (folder, name) = session_folder(Path::new("C:/art/piece.v2.psd")).unwrap();
        assert_eq!(folder, Path::new("C:/art/piece.v2_timelapse"));
        assert_eq!(name, "piece.v2.psd");
    }

    #[test]
    fn video_size_fits_and_stays_even() {
        assert_eq!(video_size((4000, 3000)), (1920, 1440));
        assert_eq!(video_size((1001, 777)), (1000, 776));
        assert_eq!(video_size((3000, 9000)), (640, 1920));
    }

    #[test]
    fn reads_a_ps_timelapse_session_and_keeps_its_fields() {
        let text = r#"{"sessionId":"1_abc","createdAt":"2026-01-01T00:00:00Z","frameCount":42,"intervalSeconds":15,"referenceTag":"REF_","lastHistorySignature":"3:Brush:9"}"#;
        let s: SessionFile = serde_json::from_str(text).unwrap();
        assert_eq!((s.frame_count, s.interval_seconds), (42, 15));
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["lastHistorySignature"], "3:Brush:9");
    }

    #[test]
    fn sped_up_job_fits_thirty_seconds() {
        let jobs = ffmpeg_jobs(900, 1920, 1080);
        assert_eq!(jobs[1].1[jobs[1].1.iter().position(|a| a == "-framerate").unwrap() + 1], "30.000");
        assert!(jobs[0].1.contains(&"24".to_string()));
    }
}
