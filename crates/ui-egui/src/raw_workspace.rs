//! Camera Raw workspace (`photocraft --raw-workspace`).
//!
//! In this mode Camera Raw is the first thing a document shows: it opens by itself on the active
//! document and is drawn inside the main window (the way the web build draws it) instead of in an
//! operating-system window of its own, so a host that embeds the editor sees it as part of the editor.
//! OK or Cancel closes the dialog and leaves the ordinary editor on the same, still-open document; that
//! is the whole step from Camera Raw to the full editor, with nothing reopened or decoded again.
//!
//! It is a mode of the one editor, not a second program: it shares the engine, the document and every
//! Camera Raw code path, and changes only *where* the dialog is drawn and *when* it opens.

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_doc::DocId;
use serde_json::Value;

use crate::raw_recipe::{self, Targets};
use crate::{PhotocraftApp, camera_raw_ui};

#[derive(Default)]
pub struct State {
    /// `--raw-workspace` was given.
    pub enabled: bool,
    /// The document Camera Raw was last opened for automatically. Closing the dialog leaves the
    /// document in the full editor; it does not reopen until another document becomes active.
    pub(crate) opened_for: Option<DocId>,
    /// Documents the workspace's own flows made (a raw's Camera Raw dialog, the document "Open" develops or
    /// continues): Camera Raw is already settled for them and must not open again by itself.
    claimed: std::collections::HashSet<DocId>,
    /// "Open" is continuing a saved document: the next one an `app.open` produces is that document, in place of
    /// this one (the raw's).
    pub(crate) expect_continue: Option<DocId>,
    /// The raw being browsed in Camera Raw: its document exists only to feed the dialog. While it is the active
    /// document and nobody has pressed Open or Cancel, nothing draws it (a 45 MP canvas costs seconds, and the host
    /// shows its own picture of the photo meanwhile), including after the dialog was left with Done.
    pub(crate) browsing: Option<DocId>,
    /// The loading frame shown while the next photo's dialog is on its way.
    pub(crate) shell: Option<Shell>,
}

impl State {
    /// Whether Camera Raw is drawn inside the main window.
    pub fn embedded(&self) -> bool {
        self.enabled
    }
}

/// Whether the editor's own canvas has nothing to show right now: the Camera Raw workspace has the window, or a raw
/// that is only being browsed is the active document.
pub fn canvas_covered(app: &PhotocraftApp) -> bool {
    if !app.raw_workspace.embedded() {
        return false;
    }
    app.camera_raw.is_some()
        || app.pending_raw_open.is_some()
        || shell_active(app)
        || app.raw_workspace.browsing.is_some_and(|id| app.session.active().is_some_and(|st| st.doc.id == id))
}

/// How long a loading frame left standing after a photo was left waits for the host's next `app.open` before giving up.
const SHELL_GRACE: std::time::Duration = std::time::Duration::from_millis(2500);
/// The longest a loading frame stays up, whatever happens.
const SHELL_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// The Camera Raw frame that stands in for the dialog while the next photo is developed (see [`camera_raw_ui::draw_shell`]).
pub(crate) struct Shell {
    name: String,
    since: std::time::Instant,
    /// The photo was just left and the host is expected to open another any moment; if it does not, the frame goes.
    awaiting_open: bool,
    /// The document that was left: the frame only stands in while it is still the active one.
    left: Option<DocId>,
}

impl State {
    /// The loading frame goes away: the dialog is up, or there will be no dialog.
    pub(crate) fn end_shell(&mut self) {
        self.shell = None;
    }
}

/// Puts the loading frame up: `name` is the photo being opened (or, with `awaiting_open`, the one just left, whose
/// successor the host is about to open).
pub(crate) fn begin_shell(app: &mut PhotocraftApp, name: &str, awaiting_open: bool) {
    if app.raw_workspace.enabled {
        let left = if awaiting_open { app.session.active().map(|st| st.doc.id) } else { None };
        app.raw_workspace.shell = Some(Shell { name: name.to_string(), since: std::time::Instant::now(), awaiting_open, left });
    }
}

/// Whether the loading frame is what the window shows: the workspace is waiting for a photo's dialog, and none is open.
pub fn shell_active(app: &PhotocraftApp) -> bool {
    app.raw_workspace.enabled
        && app.camera_raw.is_none()
        && app.raw_workspace.shell.as_ref().is_some_and(|shell| {
            let age = shell.since.elapsed();
            let still_there = !shell.awaiting_open || shell.left == app.session.active().map(|st| st.doc.id);
            age < SHELL_LIMIT && still_there && !(shell.awaiting_open && age > SHELL_GRACE)
        })
}

/// Draws the loading frame when it is due; called every frame after the dialog (which, once open, replaces it).
pub fn show_shell(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !shell_active(app) {
        app.raw_workspace.shell = None;
        return;
    }
    if let Some(shell) = &app.raw_workspace.shell {
        camera_raw_ui::draw_shell(ctx, &shell.name);
    }
    // Wake up to notice a frame nobody is going to replace.
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}

/// Marks `document` as settled: no automatic Camera Raw for it.
pub(crate) fn claim(app: &mut PhotocraftApp, document: DocId) {
    if app.raw_workspace.claimed.len() > 256 {
        let open: std::collections::HashSet<DocId> = app.session.documents().iter().map(|st| st.doc.id).collect();
        app.raw_workspace.claimed.retain(|id| open.contains(id));
    }
    app.raw_workspace.claimed.insert(document);
}

/// An `app.open` job ended. If "Open" was continuing a saved document, that document is now the active one:
/// it is settled too, and takes over a host's binding of the raw it replaced. A raw opened for Camera Raw gets
/// its dialog queued.
pub fn on_open_event(app: &mut PhotocraftApp, document: Option<DocId>, done: bool) {
    if let (Some(replaced), true) = (app.raw_workspace.expect_continue.take(), done) {
        if let Some(id) = document.or_else(|| app.session.active().map(|st| st.doc.id)) {
            claim(app, id);
            crate::control_files::rebind(app, replaced, id);
        }
        app.raw_workspace.end_shell();
        return;
    }
    if done {
        queue_dialog(app, document);
    }
    // The frame stays only while a dialog is about to open on the photo that just landed.
    if !done || app.pending_raw_open.is_none() {
        app.raw_workspace.end_shell();
    }
}

/// Closes an open Camera Raw dialog before the document under it is replaced, saved or closed.
/// Settings the user changed apply as one history step; an untouched dialog leaves the document
/// exactly as it was (browsing past a photo must not edit, dirty or re-save it). A no-op outside the
/// workspace, where the host owns the dialog's lifetime.
pub fn settle(app: &mut PhotocraftApp) -> Result<(), String> {
    if !app.raw_workspace.enabled || app.camera_raw.is_none() {
        return Ok(());
    }
    camera_raw_ui::done(app).map(|_| ())
}

/// What a host asks for when it opens a raw in Camera Raw (`app.open {cameraRaw: {...}}`): the photo's
/// stored recipe to start from, where to keep the recipe and its render, and the saved full-editor
/// document that "Open" should continue instead of developing a new one.
#[derive(Clone, Debug, Default)]
pub struct OpenSpec {
    pub recipe: Option<CameraRaw>,
    pub targets: Targets,
    pub continue_from: Option<String>,
    /// The raw's size and modified time as the host saw them (`"<bytes>:<mtime_ns>"`): a raw prepared ahead of time is
    /// used only when it matches, so a file that changed in between is never shown from an old read.
    pub signature: Option<String>,
}

impl OpenSpec {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let object = value.as_object().ok_or("cameraRaw must be an object")?;
        if let Some(key) =
            object.keys().find(|key| !["recipe", "recipePath", "displayPath", "manifestPath", "continueFrom", "signature"].contains(&key.as_str()))
        {
            return Err(format!("unknown cameraRaw field `{key}`"));
        }
        let text = |name: &str| -> Result<Option<String>, String> {
            match object.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
                Some(_) => Err(format!("cameraRaw.{name} must be a nonempty string")),
            }
        };
        let recipe = match object.get("recipe") {
            None | Some(Value::Null) => None,
            Some(recipe) => Some(raw_recipe::from_json(recipe)?),
        };
        let targets = Targets { recipe: text("recipePath")?, display: text("displayPath")?, manifest: None };
        if targets.recipe.as_ref().is_some_and(|p| !p.to_ascii_lowercase().ends_with(".json")) {
            return Err("cameraRaw.recipePath must be a .json file".into());
        }
        if targets.display.as_ref().is_some_and(|p| !(p.to_ascii_lowercase().ends_with(".jpg") || p.to_ascii_lowercase().ends_with(".jpeg"))) {
            return Err("cameraRaw.displayPath must be a .jpg file".into());
        }
        let continue_from = text("continueFrom")?;
        if continue_from.as_ref().is_some_and(|p| !p.to_ascii_lowercase().ends_with(".pcraft")) {
            return Err("cameraRaw.continueFrom must be a .pcraft project".into());
        }
        let signature = text("signature")?;
        let manifest = text("manifestPath")?;
        if manifest.as_ref().is_some_and(|p| !p.to_ascii_lowercase().ends_with(".json")) {
            return Err("cameraRaw.manifestPath must be a .json file".into());
        }
        let mut targets = Targets { manifest: manifest.map(|manifest| crate::render_manifest::Request { manifest, source: signature.clone() }), ..targets };
        if continue_from.is_some() {
            // The photo has a full-editor document, and that is what a host shows of it: a render of the recipe would only
            // take the place of that document's pictures.
            targets.display = None;
            targets.manifest = None;
        }
        Ok(Self { recipe, targets, continue_from, signature })
    }
}

/// A raw developed ahead of time for Camera Raw (the photo after the one being looked at), so opening it later costs
/// next to nothing. One slot: at most one such photo is held (a 45 MP one is ~450 MB), and a newer request replaces it.
pub struct Prepared {
    pub path: String,
    pub signature: String,
    pub name: String,
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub imported: photocraft_io::ImportResult,
    /// Set when `imported` is a reduced-resolution stand-in: the size of the full development it stands in for.
    pub(crate) full_size: Option<(u32, u32)>,
    /// The dialog's preview proxy, built on the prefetch thread so opening the photo does no such work on the editor's.
    pub(crate) proxy: Option<camera_raw_ui::ProxyData>,
}

#[derive(Default)]
pub struct Prefetch {
    /// What the host asked for last (path, signature); anything else that finishes is thrown away.
    want: Option<(String, String)>,
    ready: Option<Prepared>,
    /// What the prefetch thread is developing right now (path, signature).
    in_flight: Option<(String, String)>,
}

pub type SharedPrefetch = std::sync::Arc<std::sync::Mutex<Prefetch>>;

/// The prepared raw for `path`, if one is ready and was made from the file the host still has (`signature`). Anything
/// prepared for another photo is dropped: the host has moved on.
pub fn take_prepared(shared: &SharedPrefetch, path: &str, signature: Option<&str>) -> Option<Prepared> {
    let mut slot = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ready = slot.ready.take()?;
    (ready.path == path && signature == Some(ready.signature.as_str())).then_some(ready)
}

/// For an `app.open` that found nothing ready: if this very photo is being developed ahead of time right now, wait for
/// that instead of developing it a second time beside it (the two then fought over the cores, and an open that arrived
/// half a second after the prefetch started took longer than with no prefetch at all). `None` when nothing is in flight
/// for it, the prefetch came to nothing, or `give_up` says to stop.
pub fn wait_prepared(shared: &SharedPrefetch, path: &str, signature: &str, give_up: impl Fn() -> bool) -> Option<Prepared> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        {
            let mut slot = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.ready.as_ref().is_some_and(|ready| ready.path == path && ready.signature == signature) {
                return slot.ready.take();
            }
            if !slot.in_flight.as_ref().is_some_and(|(p, s)| p == path && s == signature) {
                return None;
            }
        }
        if give_up() || std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// `app.prefetch {path, signature}`: develop `path` for Camera Raw in the background, at low priority, so the next
/// `app.open` of it with the same signature is nearly instant. Replies at once.
pub fn request_prefetch(app: &mut PhotocraftApp, path: &str, signature: &str) -> Result<Value, String> {
    if !app.raw_workspace.enabled {
        return Err("prefetch needs the Camera Raw workspace".into());
    }
    let read = app.services.background_automation_read.clone().ok_or("background automation read authority is not configured")?;
    let shared = app.jobs.workspace_prefetch.clone();
    {
        let mut slot = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let wanted = (path.to_string(), signature.to_string());
        if slot.want.as_ref() == Some(&wanted) {
            return Ok(serde_json::json!({"queued": false, "reason": "already requested"}));
        }
        slot.want = Some(wanted);
        slot.ready = None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::{Mutex, OnceLock, PoisonError};
        static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
        static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
        let (path, signature) = (path.to_string(), signature.to_string());
        let spawned = std::thread::Builder::new().name("camera-raw-prefetch".into()).spawn(move || {
            let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
            let still_wanted = |shared: &SharedPrefetch| {
                shared.lock().unwrap_or_else(PoisonError::into_inner).want.as_ref().is_some_and(|(p, s)| *p == path && *s == signature)
            };
            if !still_wanted(&shared) {
                return;
            }
            // From here until this thread is done, an open of the same photo waits for it (see `wait_prepared`).
            struct InFlight(SharedPrefetch);
            impl Drop for InFlight {
                fn drop(&mut self) {
                    self.0.lock().unwrap_or_else(PoisonError::into_inner).in_flight = None;
                }
            }
            shared.lock().unwrap_or_else(PoisonError::into_inner).in_flight = Some((path.clone(), signature.clone()));
            let _in_flight = InFlight(shared.clone());
            let Ok((name, bytes)) = read(&path) else { return };
            if !photocraft_io::raw::is_raw(&bytes) || !still_wanted(&shared) {
                return;
            }
            // One worker thread for the develop (it parallelises through rayon): whatever is open keeps the other cores.
            let pool = POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().num_threads(1).thread_name(|_| "camera-raw-prefetch-worker".into()).build().ok());
            let imported = match pool {
                Some(pool) => pool.install(|| develop_for_browsing(&name, &bytes)),
                None => develop_for_browsing(&name, &bytes),
            };
            let Ok(Browsed { imported, full_size, curve }) = imported else { return };
            let proxy = match pool {
                Some(pool) => pool.install(|| camera_raw_ui::proxy_for(&imported.document)),
                None => camera_raw_ui::proxy_for(&imported.document),
            }
            .map(|proxy| proxy.with_curve(curve));
            let mut slot = shared.lock().unwrap_or_else(PoisonError::into_inner);
            if slot.want.as_ref().is_some_and(|(p, s)| *p == path && *s == signature) {
                slot.ready = Some(Prepared { path, signature, name, bytes: std::sync::Arc::new(bytes), imported, full_size, proxy });
            }
        });
        if let Err(error) = spawned {
            return Err(format!("could not start the prefetch: {error}"));
        }
        Ok(serde_json::json!({"queued": true}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (read, shared);
        Err("prefetch needs native threads".into())
    }
}

/// A raw developed for browsing in Camera Raw (see [`develop_for_browsing`]).
pub(crate) struct Browsed {
    pub(crate) imported: photocraft_io::ImportResult,
    /// The size of the full development a reduced one stands in for.
    pub(crate) full_size: Option<(u32, u32)>,
    /// The file's default tone curve, for the dialog's raw-stage previews: worked out here, on the worker, because the dialog
    /// would otherwise decode the raw again on the editor's own thread when it opens (half a second for 45 MP).
    pub(crate) curve: Vec<[f32; 2]>,
}

/// Develops a raw for browsing in Camera Raw: reduced in size when the file allows (a second for 45 MP rather than three),
/// else in full.
pub(crate) fn develop_for_browsing(name: &str, bytes: &[u8]) -> Result<Browsed, photocraft_io::IoError> {
    if let Some(reduced) = photocraft_io::raw::import_raw_browse_reduced(name, bytes)? {
        return Ok(Browsed { imported: reduced.import, full_size: Some(reduced.full_size), curve: reduced.tone_curve });
    }
    Ok(Browsed { imported: photocraft_io::raw::import_raw(name, bytes)?, full_size: None, curve: photocraft_io::raw::tone_curve(bytes) })
}

/// A document the host opened for the Camera Raw workspace: for a raw developed from its sensor data, its bytes
/// (the dialog re-develops from them) and the host's request; without bytes, a file the host takes its own route
/// for (the retained-raw smart object, or a fallback after the sensor decode was unavailable), which must not get
/// an automatic Camera Raw dialog.
pub struct PendingRaw {
    pub path: String,
    pub bytes: Option<std::sync::Arc<Vec<u8>>>,
    pub spec: OpenSpec,
    pub(crate) proxy: Option<camera_raw_ui::ProxyData>,
    pub(crate) full_size: Option<(u32, u32)>,
}

/// A raw opened with `cameraRaw` has landed: queue its Camera Raw dialog on the document (shown next frame).
/// Called when an `app.open` job completes, before the host's reply goes out.
pub fn queue_dialog(app: &mut PhotocraftApp, document: Option<DocId>) {
    let pending = app.jobs.workspace_raw.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
    let Some(pending) = pending else { return };
    let Some(document) = document.or_else(|| app.session.active().map(|st| st.doc.id)) else { return };
    let Some(name) = app.session.documents().iter().find(|st| st.doc.id == document).map(|st| st.doc.name.clone()) else { return };
    // The dialog owns this document now (or the host does, without bytes): the layer-filter auto-open must not take it.
    app.raw_workspace.opened_for = Some(document);
    claim(app, document);
    let Some(bytes) = pending.bytes else { return };
    app.raw_workspace.browsing = Some(document);
    app.pending_raw_open = Some(camera_raw_ui::RawOpen {
        document,
        name,
        path: None,
        bytes: Some(bytes),
        initial: pending.spec.recipe,
        targets: pending.spec.targets,
        continue_from: pending.spec.continue_from,
        proxy: pending.proxy,
        full_size: pending.full_size,
    });
}

/// The path of the raw `app.prefetch` has developed and is holding for the host, if any: opening that photo costs next to
/// nothing, so a host need not wait for the selection to rest before asking for it.
fn prefetched_path(app: &PhotocraftApp) -> Option<String> {
    app.jobs.workspace_prefetch.lock().unwrap_or_else(std::sync::PoisonError::into_inner).ready.as_ref().map(|ready| ready.path.clone())
}

/// `ui.cameraRaw {open?, commit?, cancel?}`: the workspace's dialog for a host.
///
/// * no flags: the dialog's state;
/// * `open`: opens it on the active document when it is not open;
/// * `commit`: applies it as `settle` does (changed settings as one step, an untouched dialog just closes);
/// * `cancel`: closes it without applying anything.
///
/// Adjusting the open dialog is the editor's own `ui.menu.invoke {"id":"filter.cameraRaw","params":{"ui":{"set":{...}}}}`,
/// which it allows while the dialog is open.
pub fn control(app: &mut PhotocraftApp, ctx: &egui::Context, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::json;
    if let Some(object) = p.as_object()
        && let Some(key) = object.keys().find(|key| !["open", "commit", "cancel", "done", "wait"].contains(&key.as_str()))
    {
        return Err(format!("unknown ui.cameraRaw field `{key}`"));
    }
    let flag = |name: &str| -> Result<bool, String> {
        match p.get(name) {
            None => Ok(false),
            Some(value) => value.as_bool().ok_or_else(|| format!("{name} must be a boolean")),
        }
    };
    let (open, commit, cancel, done) = (flag("open")?, flag("commit")?, flag("cancel")?, flag("done")?);
    if u8::from(open) + u8::from(commit) + u8::from(cancel) + u8::from(done) > 1 {
        return Err("ui.cameraRaw takes one of open, commit, cancel or done".into());
    }
    let mut applied = false;
    let mut left = None;
    if open && app.camera_raw.is_none() {
        app.raw_workspace.opened_for = app.session.active().map(|state| state.doc.id);
        camera_raw_ui::open(app, ctx)?;
    }
    if done {
        // Leaving the photo: a raw keeps its settings as a recipe and nothing is developed. The host opens another
        // photo next; its Camera Raw frame is up from now, so the editor's own windows never show in between.
        let leaving_a_raw = camera_raw_ui::target_name(app) == Some("openRaw");
        left = Some(camera_raw_ui::done(app)?);
        if leaving_a_raw {
            begin_shell(app, "", true);
        }
    } else if commit {
        if camera_raw_ui::target_name(app) == Some("openRaw") {
            // "Open": develop the raw with the settings (or continue its saved document).
            let result = camera_raw_ui::commit(app)?;
            applied = true;
            left = Some(result);
        } else {
            applied = camera_raw_ui::changed(app);
            settle_any(app)?;
        }
    } else if cancel {
        camera_raw_ui::cancel(app);
    }
    let mut state = json!({"open": app.camera_raw.is_some(), "changed": camera_raw_ui::changed(app), "applied": applied,
        "workspace": app.raw_workspace.enabled, "target": camera_raw_ui::target_name(app), "prefetched": prefetched_path(app)});
    if let Some(recipe) = camera_raw_ui::recipe(app) {
        state["recipe"] = recipe;
    }
    state["source"] = json!(camera_raw_ui::source(app));
    if let Some(left) = left {
        state["result"] = left;
    }
    Ok(state)
}

/// [`settle`], also outside the workspace (an explicit host request).
fn settle_any(app: &mut PhotocraftApp) -> Result<(), String> {
    if app.camera_raw.is_none() {
        return Ok(());
    }
    if camera_raw_ui::changed(app) {
        camera_raw_ui::commit(app).map(|_| ())
    } else {
        camera_raw_ui::cancel(app);
        Ok(())
    }
}

/// Opens Camera Raw on the active document the first time it is active, in workspace mode. Called
/// every frame before the dialog is drawn; does nothing unless a document is ready and nothing else
/// holds the dialog.
pub fn auto_open(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.raw_workspace.enabled || app.camera_raw.is_some() || app.pending_raw_open.is_some() {
        return;
    }
    let Some(state) = app.session.active() else {
        // Between documents (a tab was closed): the next one, even a restored copy of the same photo
        // with the same id, is a new document and gets Camera Raw again.
        app.raw_workspace.opened_for = None;
        return;
    };
    let id = state.doc.id;
    if app.raw_workspace.opened_for == Some(id) || app.raw_workspace.claimed.contains(&id) {
        return;
    }
    app.raw_workspace.opened_for = Some(id);
    if let Err(error) = camera_raw_ui::open(app, ctx) {
        // The document has no pixels Camera Raw can work on (or the filter is unavailable); the full
        // editor is the right place for it.
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// The first time the editor draws a document's canvas (Open does, at a moment the user is waiting) it needs the GPU compositor,
/// whose shaders the driver compiles for about 12 s on an Intel UHD 620; built on the drawing thread, the whole window stops for
/// that long. In the workspace the build runs on a worker thread instead and the canvas draws on the CPU until it is ready.
///
/// It starts then, not ahead of time while browsing: the compile takes a core and the driver's own threads, which slowed every
/// photo swap that overlapped it (prepared 165 -> 189 ms, unprepared 1.72 -> 1.99 s on that laptop), and browsing must never wait
/// for background work. Called every frame; cheap.
pub fn gpu_builds_in_background(app: &mut PhotocraftApp) {
    if app.raw_workspace.enabled {
        app.gpu_background_build(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_document() -> (PhotocraftApp, egui::Context) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        (app, ctx)
    }

    #[test]
    fn nothing_opens_unless_the_mode_is_on() {
        let (mut app, ctx) = app_with_document();
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none());
        assert!(!app.raw_workspace.embedded());
    }

    #[test]
    fn camera_raw_opens_on_the_active_document_once() {
        let (mut app, ctx) = app_with_document();
        app.raw_workspace.enabled = true;
        assert!(app.raw_workspace.embedded());
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_some(), "Camera Raw did not open by itself");
        // OK/Cancel closes it; the document stays open in the full editor and is not reopened.
        let documents = app.session.documents().len();
        app.camera_raw = None;
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "closing Camera Raw must leave the full editor, not reopen it");
        assert_eq!(app.session.documents().len(), documents);
    }

    fn history(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.past_len()
    }

    fn adjust(app: &mut PhotocraftApp, ctx: &egui::Context, exposure: f64) {
        camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &serde_json::json!({"ui": {"set": {"exposure": exposure}}})).unwrap().unwrap();
    }

    #[test]
    fn settling_an_untouched_dialog_leaves_the_document_alone() {
        let (mut app, ctx) = app_with_document();
        app.raw_workspace.enabled = true;
        auto_open(&mut app, &ctx);
        let steps = history(&app);
        settle(&mut app).unwrap();
        assert!(app.camera_raw.is_none());
        assert_eq!(history(&app), steps, "browsing past a photo must not edit it");
    }

    #[test]
    fn settling_a_changed_dialog_applies_it_as_one_step() {
        let (mut app, ctx) = app_with_document();
        app.raw_workspace.enabled = true;
        auto_open(&mut app, &ctx);
        let steps = history(&app);
        adjust(&mut app, &ctx, 1.0);
        assert!(camera_raw_ui::changed(&app));
        settle(&mut app).unwrap();
        assert!(app.camera_raw.is_none());
        assert_eq!(history(&app), steps + 1);
    }

    #[test]
    fn settle_does_nothing_outside_the_workspace() {
        let (mut app, ctx) = app_with_document();
        camera_raw_ui::open(&mut app, &ctx).unwrap();
        adjust(&mut app, &ctx, 1.0);
        let steps = history(&app);
        settle(&mut app).unwrap();
        assert!(app.camera_raw.is_some(), "a dialog the user opened themselves is theirs to close");
        assert_eq!(history(&app), steps);
    }

    #[test]
    fn the_control_opens_commits_and_cancels() {
        let (mut app, ctx) = app_with_document();
        let state = control(&mut app, &ctx, &json!({})).unwrap();
        assert_eq!(state["open"], false);
        let opened = control(&mut app, &ctx, &json!({"open": true})).unwrap();
        assert_eq!((opened["open"].as_bool(), opened["changed"].as_bool()), (Some(true), Some(false)));
        let steps = history(&app);
        let untouched = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        assert_eq!((untouched["open"].as_bool(), untouched["applied"].as_bool()), (Some(false), Some(false)));
        assert_eq!(history(&app), steps);
        control(&mut app, &ctx, &json!({"open": true})).unwrap();
        adjust(&mut app, &ctx, 1.0);
        assert_eq!(control(&mut app, &ctx, &json!({})).unwrap()["changed"], true);
        let applied = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        assert_eq!((applied["open"].as_bool(), applied["applied"].as_bool()), (Some(false), Some(true)));
        assert_eq!(history(&app), steps + 1);
        control(&mut app, &ctx, &json!({"open": true})).unwrap();
        adjust(&mut app, &ctx, -1.0);
        let cancelled = control(&mut app, &ctx, &json!({"cancel": true})).unwrap();
        assert_eq!(cancelled["open"], false);
        assert_eq!(history(&app), steps + 1, "cancel applies nothing");
    }

    #[test]
    fn the_control_rejects_bad_requests() {
        let (mut app, ctx) = app_with_document();
        assert!(control(&mut app, &ctx, &json!({"bogus": true})).unwrap_err().contains("bogus"));
        assert!(control(&mut app, &ctx, &json!({"open": "yes"})).is_err());
        assert!(control(&mut app, &ctx, &json!({"open": true, "commit": true})).is_err());
        assert!(app.camera_raw.is_none(), "a rejected request opens nothing");
    }

    #[test]
    fn the_camera_raw_frame_stands_in_from_the_open_request_until_the_dialog_is_up() {
        use photocraft_engine::jobs::Started;
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        assert!(!shell_active(&app));
        let started = crate::control_files::open_with_camera_raw(&mut app, "IMG.dng", true, Some(spec(None))).unwrap();
        let Started::Job(job) = started else { panic!("expected a background open") };
        assert!(shell_active(&app), "up as soon as the host asks for the photo");
        assert!(canvas_covered(&app));
        let loading = crate::control::viewport(&app, &ctx)["cameraRaw"].clone();
        assert_eq!((loading["open"].as_bool(), loading["loading"].as_bool()), (Some(false), Some(true)), "{loading}");
        assert_eq!(loading["view"], loading["preview"]);
        app.session.wait_job(job).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert!(shell_active(&app), "still up while the dialog is queued");
        let raw = app.pending_raw_open.take().expect("the raw's dialog is queued");
        camera_raw_ui::open_raw(&mut app, &ctx, raw).unwrap();
        assert!(!shell_active(&app) && app.raw_workspace.shell.is_none(), "the dialog replaced it");
        assert_eq!(crate::control::viewport(&app, &ctx)["cameraRaw"]["open"], true);
    }

    #[test]
    fn a_photo_that_does_not_get_a_dialog_takes_the_frame_down() {
        let (mut app, ctx) = app_with_document();
        app.raw_workspace.enabled = true;
        // A failed or cancelled open.
        begin_shell(&mut app, "a.NEF", false);
        on_open_event(&mut app, None, false);
        assert!(app.raw_workspace.shell.is_none());
        // An open that finished without a dialog to queue (a file the editor could not develop).
        begin_shell(&mut app, "a.NEF", false);
        on_open_event(&mut app, None, true);
        assert!(app.raw_workspace.shell.is_none() && app.pending_raw_open.is_none());
        // An open that is not for Camera Raw at all.
        begin_shell(&mut app, "a.NEF", true);
        app.raw_workspace.end_shell();
        assert!(!shell_active(&app));
        // Outside the workspace there is no frame to show.
        app.raw_workspace.enabled = false;
        begin_shell(&mut app, "a.NEF", false);
        assert!(!shell_active(&app) && app.raw_workspace.shell.is_none());
        let _ = ctx;
    }

    #[test]
    fn leaving_a_raw_keeps_the_frame_up_for_the_next_photo_but_not_for_long() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        open_in_camera_raw(&mut app, &ctx, spec(None));
        assert!(!shell_active(&app), "the dialog itself is showing");
        control(&mut app, &ctx, &json!({"done": true})).unwrap();
        assert!(shell_active(&app), "no editor windows between one photo and the next");
        // If the host does not open another photo, the frame does not stay.
        let shell = app.raw_workspace.shell.as_mut().unwrap();
        shell.since = shell.since.checked_sub(std::time::Duration::from_secs(3)).unwrap();
        assert!(!shell_active(&app));
        ctx.run_ui(egui::RawInput::default(), |ui| show_shell(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.raw_workspace.shell.is_none());
    }

    #[test]
    fn the_loading_frame_is_drawn_with_the_dialogs_own_geometry() {
        let (mut app, ctx) = app_with_document();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        app.raw_workspace.enabled = true;
        let frame = |app: &mut PhotocraftApp, draw: &dyn Fn(&mut PhotocraftApp, &egui::Context)| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
            ctx.run_ui(input, |ui| draw(app, ui.ctx())).textures_delta.clear();
        };
        begin_shell(&mut app, "IMG.dng", false);
        frame(&mut app, &|app, ctx| show_shell(app, ctx));
        assert!(shell_active(&app), "the frame draws without opening anything");
        let promised = crate::control::viewport(&app, &ctx)["cameraRaw"]["view"].clone();
        camera_raw_ui::open(&mut app, &ctx).unwrap();
        frame(&mut app, &|app, ctx| camera_raw_ui::show(app, ctx));
        let drawn = app.camera_raw.as_ref().unwrap().viewport.expect("the dialog laid out its view");
        assert_eq!(drawn, camera_raw_ui::layout(ctx.content_rect(), false).view);
        let reported = crate::control::viewport(&app, &ctx)["cameraRaw"]["view"].clone();
        assert_eq!(promised, reported, "the picture a host laid over the frame does not move when the dialog takes over");
    }

    /// What the editor itself answers to a control request, as a host sends it.
    fn request(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: serde_json::Value) -> serde_json::Value {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, ctx, &req) {
            crate::control::Outcome::Done(value) => value,
            _ => panic!("{method}: expected an immediate reply"),
        }
    }

    #[test]
    fn the_open_dialog_is_adjusted_through_the_menu_route_and_ui_camera_raw_leaves_it() {
        let (mut app, ctx) = app_with_document();
        let set = |app: &mut PhotocraftApp, settings: serde_json::Value| {
            request(app, &ctx, "ui.menu.invoke", json!({"id": "filter.cameraRaw", "params": {"ui": {"set": settings}}}))
        };
        control(&mut app, &ctx, &json!({"open": true})).unwrap();
        let changed = set(&mut app, json!({"exposure": 1.0, "contrast": 25}));
        assert_eq!(changed["ok"], true, "{changed}");
        assert_eq!(control(&mut app, &ctx, &json!({})).unwrap()["changed"], true);
        // A bad request changes nothing, and `ui.cameraRaw` has no settings of its own: the editor's own route is the one.
        let bad = set(&mut app, json!({"nonsense": 1}));
        assert_eq!(bad["ok"], false, "{bad}");
        assert!(control(&mut app, &ctx, &json!({"set": {"exposure": 1.0}})).unwrap_err().contains("unknown ui.cameraRaw field `set`"));
        assert_eq!(control(&mut app, &ctx, &json!({})).unwrap()["changed"], true);
        // Leaving is `ui.cameraRaw`'s.
        let steps = history(&app);
        let applied = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        assert_eq!((applied["open"].as_bool(), applied["applied"].as_bool()), (Some(false), Some(true)));
        assert_eq!(history(&app), steps + 1);
    }

    #[test]
    fn a_photo_reopened_from_memory_with_the_same_id_gets_camera_raw_again() {
        let (mut app, ctx) = app_with_document();
        app.raw_workspace.enabled = true;
        auto_open(&mut app, &ctx);
        app.camera_raw = None;
        let id = app.session.active().unwrap().doc.id;
        let index = app.session.documents().iter().position(|state| state.doc.id == id).unwrap();
        app.session.close(index);
        auto_open(&mut app, &ctx); // the empty frame between the two
        let restored =
            photocraft_doc::Document::new("photo", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        app.session.add_document(restored.clone(), None);
        app.run("layer.new.layer", json!({})).unwrap();
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_some());
    }

    // --- raws opened in Camera Raw, with a recipe ----------------------------------------------------

    type Writes = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

    /// A workspace app whose scoped reader serves `files` by name and whose writer records what is written.
    fn host_app(files: Vec<(&'static str, Vec<u8>)>) -> (PhotocraftApp, Writes) {
        let writes: Writes = Default::default();
        let recorded = writes.clone();
        let files = std::sync::Arc::new(files);
        let services = crate::Services {
            background_automation_read: Some(std::sync::Arc::new(move |path: &str| {
                files.iter().find(|(name, _)| *name == path).map(|(name, bytes)| (name.to_string(), bytes.clone())).ok_or_else(|| format!("no file {path}"))
            })),
            background_automation_write: Some(std::sync::Arc::new(move |path: &str, bytes: &[u8]| {
                recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.raw_workspace.enabled = true;
        (app, writes)
    }

    /// [`host_app`] whose writer fails, as a full or read-only disk does, while the returned flag is set.
    fn host_app_with_failing_writes(files: Vec<(&'static str, Vec<u8>)>) -> (PhotocraftApp, Writes, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (mut app, writes) = host_app(files);
        let failing = std::sync::Arc::new(AtomicBool::new(false));
        let (fail, recorded) = (failing.clone(), writes.clone());
        app.services.background_automation_write = Some(std::sync::Arc::new(move |path: &str, bytes: &[u8]| {
            if fail.load(Ordering::SeqCst) {
                return Err("disk full".into());
            }
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        (app, writes, failing)
    }

    fn recipe_writes(writes: &Writes) -> Vec<f32> {
        let writes = writes.lock().unwrap();
        let recipes = writes.iter().filter(|(name, _)| name.ends_with(".cameraraw.json"));
        recipes.map(|(_, bytes)| raw_recipe::from_json(&serde_json::from_slice(bytes).unwrap()).unwrap().exposure).collect()
    }

    fn synthetic_raw() -> Vec<u8> {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        let (w, h) = (48, 32);
        let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
        spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
        spec.build()
    }

    fn spec(recipe: Option<f32>) -> OpenSpec {
        let mut spec = OpenSpec {
            targets: Targets { recipe: Some("IMG.dng.cameraraw.json".into()), display: Some("IMG.dng.photocraft.display.jpg".into()), manifest: None },
            ..Default::default()
        };
        if let Some(exposure) = recipe {
            spec.recipe = Some(CameraRaw { exposure, ..Default::default() });
        }
        spec
    }

    /// Opens the raw as a host would, lets the app notice it landed, and shows the queued dialog.
    fn open_in_camera_raw(app: &mut PhotocraftApp, ctx: &egui::Context, spec: OpenSpec) -> serde_json::Value {
        use photocraft_engine::jobs::Started;
        let started = crate::control_files::open_with_camera_raw(app, "IMG.dng", true, Some(spec)).unwrap();
        let Started::Job(job) = started else { panic!("expected a background open") };
        let result = app.session.wait_job(job).unwrap();
        crate::jobs_ui::tick(app, ctx);
        let raw = app.pending_raw_open.take().expect("the raw's dialog is queued");
        camera_raw_ui::open_raw(app, ctx, raw).unwrap();
        result
    }

    fn written(writes: &Writes, path: &str) -> Option<Vec<u8>> {
        writes.lock().unwrap().iter().rev().find(|(name, _)| name == path).map(|(_, bytes)| bytes.clone())
    }

    #[test]
    fn the_spec_names_the_manifest_and_a_photo_with_a_project_renders_no_recipe() {
        let parsed = OpenSpec::parse(&json!({"recipePath": "a.json", "displayPath": "a.jpg", "manifestPath": "a.renders.json", "signature": "5:6"})).unwrap();
        assert_eq!(parsed.targets.manifest, Some(crate::render_manifest::Request { manifest: "a.renders.json".into(), source: Some("5:6".into()) }));
        assert!(OpenSpec::parse(&json!({"manifestPath": "a.txt"})).unwrap_err().contains("manifestPath"));
        let with_project =
            OpenSpec::parse(&json!({"recipePath": "a.json", "displayPath": "a.jpg", "manifestPath": "a.renders.json", "continueFrom": "a.pcraft"})).unwrap();
        assert_eq!(with_project.targets.recipe.as_deref(), Some("a.json"), "the recipe is still kept");
        assert_eq!((with_project.targets.display, with_project.targets.manifest), (None, None));
    }

    #[test]
    fn a_raw_opens_in_camera_raw_with_its_stored_recipe() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        let result = open_in_camera_raw(&mut app, &ctx, spec(Some(1.5)));
        assert_eq!(result["cameraRaw"]["dialog"], true, "{result}");
        let d = app.camera_raw.as_ref().expect("dialog open");
        assert_eq!(d.params.exposure, 1.5, "the dialog starts from the stored recipe");
        assert_eq!(camera_raw_ui::target_name(&app), Some("openRaw"));
        assert!(!camera_raw_ui::changed(&app), "opening with a recipe is not a change");
        assert_eq!(app.session.documents().len(), 1);
        auto_open(&mut app, &ctx);
        assert_eq!(camera_raw_ui::target_name(&app), Some("openRaw"), "the filter auto-open must not take the raw's document");
    }

    #[test]
    fn done_keeps_the_recipe_and_its_render_and_develops_nothing() {
        let (mut app, writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        // A render path of its own: a newer render of the same path, from a test running beside this one, would replace it.
        let display = format!("IMG{}.dng.photocraft.display.jpg", crate::render_manifest::generation());
        let mut own = spec(None);
        own.targets.display = Some(display.clone());
        open_in_camera_raw(&mut app, &ctx, own);
        let document = app.session.active().unwrap().doc.id;
        let steps = history(&app);
        adjust(&mut app, &ctx, 2.0);
        let state = control(&mut app, &ctx, &json!({"done": true})).unwrap();
        assert_eq!((state["open"].as_bool(), state["result"]["changed"].as_bool()), (Some(false), Some(true)));
        assert_eq!(state["result"]["recipe"]["cameraRaw"]["exposure"], 2.0);
        assert!(app.camera_raw.is_none());
        assert_eq!(app.session.active().unwrap().doc.id, document, "the document was not re-developed or replaced");
        assert_eq!(history(&app), steps);
        let recipe = written(&writes, "IMG.dng.cameraraw.json").expect("recipe written");
        assert_eq!(raw_recipe::from_json(&serde_json::from_slice(&recipe).unwrap()).unwrap().exposure, 2.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while written(&writes, &display).is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(written(&writes, &display).is_some_and(|b| b.starts_with(&[0xFF, 0xD8])), "render written");
    }

    #[test]
    fn done_without_changes_writes_nothing() {
        let (mut app, writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(Some(1.0)));
        let state = control(&mut app, &ctx, &json!({"done": true})).unwrap();
        assert_eq!(state["result"]["changed"], false);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(writes.lock().unwrap().is_empty(), "browsing past an unchanged photo must not write");
    }

    #[test]
    fn resetting_a_recipe_to_defaults_overwrites_it() {
        let (mut app, writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(Some(1.0)));
        adjust(&mut app, &ctx, 0.0);
        control(&mut app, &ctx, &json!({"done": true})).unwrap();
        let recipe = written(&writes, "IMG.dng.cameraraw.json").expect("the reset is written");
        assert!(raw_recipe::from_json(&serde_json::from_slice(&recipe).unwrap()).unwrap().is_identity());
    }

    #[test]
    fn leaving_a_photo_in_the_workspace_keeps_the_recipe_without_the_host_asking() {
        let (mut app, writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        adjust(&mut app, &ctx, 1.0);
        settle(&mut app).unwrap();
        assert!(app.camera_raw.is_none());
        assert!(written(&writes, "IMG.dng.cameraraw.json").is_some());
    }

    #[test]
    fn done_that_cannot_keep_the_recipe_leaves_the_dialog_and_its_settings_until_it_can() {
        use std::sync::atomic::Ordering;
        let (mut app, writes, failing) = host_app_with_failing_writes(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        adjust(&mut app, &ctx, 2.0);
        failing.store(true, Ordering::SeqCst);
        for attempt in 0..3 {
            let error = control(&mut app, &ctx, &json!({"done": true})).unwrap_err();
            assert!(error.contains("disk full"), "attempt {attempt}: {error}");
            let d = app.camera_raw.as_ref().expect("the dialog stays");
            assert_eq!(d.params.exposure, 2.0, "attempt {attempt}: the settings stay");
            assert!(camera_raw_ui::changed(&app), "attempt {attempt}: still a change to keep (the dialog's starting point is untouched)");
            assert!(!app.camera_raw.as_ref().unwrap().busy(), "attempt {attempt}");
        }
        assert!(recipe_writes(&writes).is_empty());
        failing.store(false, Ordering::SeqCst);
        let state = control(&mut app, &ctx, &json!({"done": true})).unwrap();
        assert_eq!(state["result"]["changed"], true);
        assert!(app.camera_raw.is_none());
        assert_eq!(recipe_writes(&writes), [2.0], "kept once, once it could be");
    }

    #[test]
    fn switching_photos_when_the_recipe_cannot_be_kept_stays_on_the_photo() {
        use std::sync::atomic::Ordering;
        let (mut app, writes, failing) = host_app_with_failing_writes(vec![("IMG.dng", synthetic_raw()), ("NEXT.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        let document = app.session.active().unwrap().doc.id;
        adjust(&mut app, &ctx, 1.5);
        failing.store(true, Ordering::SeqCst);
        for _ in 0..2 {
            let error = crate::control_files::open_with_camera_raw(&mut app, "NEXT.dng", true, Some(spec(None))).unwrap_err();
            assert!(error.contains("disk full"), "{error}");
            let d = app.camera_raw.as_ref().expect("the dialog stays on the photo");
            assert_eq!(d.params.exposure, 1.5);
            assert!(!d.busy(), "not handed over");
            assert!(camera_raw_ui::changed(&app));
            assert_eq!((app.session.documents().len(), app.session.active().unwrap().doc.id), (1, document), "the next photo was not opened");
        }
        failing.store(false, Ordering::SeqCst);
        crate::control_files::open_with_camera_raw(&mut app, "NEXT.dng", true, Some(spec(None))).unwrap();
        assert_eq!(recipe_writes(&writes), [1.5]);
    }

    #[test]
    fn open_that_cannot_keep_changed_settings_opens_nothing() {
        use std::sync::atomic::Ordering;
        let (mut app, writes, failing) = host_app_with_failing_writes(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        let document = app.session.active().unwrap().doc.id;
        let browsing = app.raw_workspace.browsing;
        adjust(&mut app, &ctx, 1.0);
        failing.store(true, Ordering::SeqCst);
        let error = control(&mut app, &ctx, &json!({"commit": true})).unwrap_err();
        assert!(error.contains("disk full"), "{error}");
        assert_eq!(app.camera_raw.as_ref().expect("the dialog stays").params.exposure, 1.0);
        assert!(camera_raw_ui::changed(&app));
        assert_eq!(app.raw_workspace.browsing, browsing, "still browsing");
        assert_eq!(app.session.active().unwrap().doc.id, document, "nothing developed");
        failing.store(false, Ordering::SeqCst);
        control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        assert!(app.camera_raw.is_none());
        assert_eq!(recipe_writes(&writes), [1.0]);
    }

    #[test]
    fn cancel_closes_the_dialog_and_leaves_the_document_in_the_workspace() {
        let (mut app, writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        adjust(&mut app, &ctx, 1.0);
        control(&mut app, &ctx, &json!({"cancel": true})).unwrap();
        assert!(app.camera_raw.is_none());
        assert_eq!(app.session.documents().len(), 1, "the host owns the document");
        assert!(writes.lock().unwrap().is_empty(), "cancel keeps nothing");
    }

    #[test]
    fn open_continues_the_saved_document_instead_of_developing_a_new_one() {
        let saved =
            photocraft_doc::Document::new("saved edits", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let project = photocraft_io::export(&saved, "IMG.dng.pcraft", &Default::default()).unwrap().bytes;
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw()), ("IMG.dng.pcraft", project)]);
        let ctx = egui::Context::default();
        let mut with_project = spec(Some(1.0));
        with_project.continue_from = Some("IMG.dng.pcraft".into());
        open_in_camera_raw(&mut app, &ctx, with_project);
        let state = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        let job = state["result"]["job"].as_u64().expect("the saved document opens as a job");
        app.session.wait_job(photocraft_engine::jobs::JobId(job)).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1);
        // The saved project, named after its file (upstream #3571), at the size it was saved with: not a new development.
        let continued = &app.session.active().unwrap().doc;
        assert_eq!((continued.name.as_str(), continued.size.width, continued.size.height), ("IMG.dng.pcraft", 8, 8));
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "the saved document is the full editor's: Camera Raw must not open on it again");
    }

    // --- the host's binding (native Save) follows the photo -----------------------------------------------

    /// Binds the active document as a host does after its open (`app.bind`).
    fn bind(app: &mut PhotocraftApp) -> photocraft_doc::DocId {
        let id = app.session.active().unwrap().doc.id;
        app.jobs.host_target = Some(crate::control_files::HostTarget {
            doc: id,
            path: "IMG.dng.pcraft".into(),
            preview: "IMG.dng.photocraft.png".into(),
            display: None,
            manifest: None,
        });
        id
    }

    fn native_save_is_the_host_s(app: &PhotocraftApp) -> bool {
        crate::control_files::hosted_save(app, "file.save")
    }

    #[test]
    fn open_that_develops_the_raw_again_keeps_the_host_s_binding() {
        for background in [false, true] {
            let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
            app.background_jobs = background;
            let ctx = egui::Context::default();
            open_in_camera_raw(&mut app, &ctx, spec(None));
            let raw = bind(&mut app);
            adjust(&mut app, &ctx, 1.0); // exposure acts on the sensor data: Open develops the raw again
            let state = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
            if let Some(job) = state["result"]["job"].as_u64() {
                let _ = app.session.wait_job(photocraft_engine::jobs::JobId(job));
                for _ in 0..50 {
                    crate::jobs_ui::tick(&mut app, &ctx);
                    if app.raw_redevelop.is_none() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            let developed = app.session.active().unwrap().doc.id;
            assert_ne!(developed, raw, "background {background}: a new document");
            assert_eq!(app.jobs.host_target.as_ref().map(|t| t.doc), Some(developed), "background {background}");
            assert!(native_save_is_the_host_s(&app), "background {background}: Ctrl+S saves for the host, not Save As");
        }
    }

    #[test]
    fn open_of_a_stand_in_keeps_the_host_s_binding() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", big_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        let raw = bind(&mut app);
        control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        let developed = app.session.active().unwrap().doc.id;
        assert_ne!(developed, raw, "the full-size development replaced the stand-in");
        assert!(native_save_is_the_host_s(&app));
    }

    #[test]
    fn continuing_the_photo_s_saved_project_keeps_the_host_s_binding() {
        let saved =
            photocraft_doc::Document::new("saved edits", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let project = photocraft_io::export(&saved, "IMG.dng.pcraft", &Default::default()).unwrap().bytes;
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw()), ("IMG.dng.pcraft", project)]);
        let ctx = egui::Context::default();
        let mut with_project = spec(Some(1.0));
        with_project.continue_from = Some("IMG.dng.pcraft".into());
        open_in_camera_raw(&mut app, &ctx, with_project);
        let raw = bind(&mut app);
        let state = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        app.session.wait_job(photocraft_engine::jobs::JobId(state["result"]["job"].as_u64().unwrap())).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        let continued = app.session.active().unwrap().doc.id;
        assert_ne!(continued, raw);
        assert!(native_save_is_the_host_s(&app), "Ctrl+S saves the continued project for the host");
    }

    #[test]
    fn continuing_from_memory_keeps_the_binding_and_does_not_swallow_the_next_open() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw()), ("NEXT.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        // A save of the photo is still in flight: the editor holds its snapshot.
        let saved =
            photocraft_doc::Document::new("saved edits", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        app.jobs.host_snapshots.insert("IMG.dng.pcraft".into(), (std::sync::Arc::new(saved), 7));
        let mut with_project = spec(Some(1.0));
        with_project.continue_from = Some("IMG.dng.pcraft".into());
        open_in_camera_raw(&mut app, &ctx, with_project);
        let raw = bind(&mut app);
        let state = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        assert!(state["result"]["job"].is_null(), "restored at once: {state}");
        let continued = app.session.active().unwrap();
        assert_eq!((continued.doc.name.as_str(), continued.revision), ("saved edits", 7));
        assert_ne!(continued.doc.id, raw);
        assert!(native_save_is_the_host_s(&app));
        assert!(app.raw_workspace.expect_continue.is_none(), "the continuation has landed");
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "the continued document is settled");
        // The next photo opens in Camera Raw, not taken for the continuation.
        let next = open_in_camera_raw(&mut app, &ctx, spec(None));
        assert_eq!(next["cameraRaw"]["dialog"], true, "{next}");
        assert_eq!(camera_raw_ui::target_name(&app), Some("openRaw"));
    }

    #[test]
    fn another_photo_replacing_the_bound_one_does_not_take_its_binding() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw()), ("NEXT.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        let raw = bind(&mut app);
        let started = crate::control_files::open_with_camera_raw(&mut app, "NEXT.dng", true, Some(spec(None))).unwrap();
        if let photocraft_engine::jobs::Started::Job(job) = started {
            app.session.wait_job(job).unwrap();
        }
        crate::jobs_ui::tick(&mut app, &ctx);
        assert_ne!(app.session.active().unwrap().doc.id, raw);
        assert_eq!(app.jobs.host_target.as_ref().map(|t| t.doc), Some(raw), "the binding stays with the photo it was made for");
        assert!(!native_save_is_the_host_s(&app), "until the host binds the new photo");
    }

    /// A raw big enough to be browsed reduced: its longer side is more than twice the browsing size.
    fn big_raw() -> Vec<u8> {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        let (w, h) = (3700, 1900);
        let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
        spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
        spec.build()
    }

    #[test]
    fn a_large_raw_is_browsed_as_a_stand_in_but_the_dialog_speaks_for_the_photo() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", big_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        let dialog = app.camera_raw.as_ref().expect("a dialog");
        assert!(dialog.is_stand_in());
        let described = dialog.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview);
        assert_eq!(described["sourceSize"], json!([3700, 1900]), "the photo's size, not the stand-in's");
        let size = app.session.active().unwrap().doc.size;
        assert!(size.width < 3700 && size.width >= 1800, "the document is reduced: {}x{}", size.width, size.height);
    }

    #[test]
    fn open_develops_a_stand_in_in_full_even_when_nothing_was_adjusted() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", big_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        let before = app.session.active().unwrap().doc.id;
        control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        let after = app.session.active().expect("a document");
        assert_ne!(before, after.doc.id, "the stand-in is replaced by the real photo");
        assert_eq!((after.doc.size.width, after.doc.size.height), (3700, 1900));
        assert_eq!(app.session.documents().len(), 1);
        assert!(app.camera_raw.is_none());
    }

    #[test]
    fn a_small_raw_is_browsed_as_itself_and_open_keeps_its_document() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        assert!(!app.camera_raw.as_ref().unwrap().is_stand_in());
        let before = app.session.active().unwrap().doc.id;
        control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert_eq!(app.session.active().unwrap().doc.id, before, "nothing to redo: it was never reduced");
    }

    #[test]
    fn open_develops_the_raw_and_the_new_document_is_left_to_the_full_editor() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(Some(1.0)));
        let before = app.session.active().unwrap().doc.id;
        control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert!(app.camera_raw.is_none());
        let after = app.session.active().expect("a document").doc.id;
        assert_ne!(before, after, "exposure and white balance act on the sensor data, so the raw is developed again");
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "the developed document must not get the Camera Raw filter dialog");
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn a_failed_continue_does_not_leave_the_next_document_unclaimed_forever() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        let mut with_missing_project = spec(Some(1.0));
        with_missing_project.continue_from = Some("gone.pcraft".into());
        open_in_camera_raw(&mut app, &ctx, with_missing_project);
        let state = control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        let job = state["result"]["job"].as_u64().unwrap();
        assert!(app.session.wait_job(photocraft_engine::jobs::JobId(job)).is_err(), "the project does not exist");
        crate::jobs_ui::tick(&mut app, &ctx);
        assert!(app.raw_workspace.expect_continue.is_none(), "a failed open clears the expectation");
    }

    #[test]
    fn a_file_that_is_not_a_raw_reports_no_dialog_so_the_host_can_take_its_own_route() {
        let doc = photocraft_doc::Document::new("flat", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let project = photocraft_io::export(&doc, "x.pcraft", &Default::default()).unwrap().bytes;
        let (mut app, _writes) = host_app(vec![("x.pcraft", project)]);
        let ctx = egui::Context::default();
        let started = crate::control_files::open_with_camera_raw(&mut app, "x.pcraft", true, Some(spec(None))).unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("expected a job") };
        let result = app.session.wait_job(job).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert_eq!(result["cameraRaw"]["dialog"], false, "{result}");
        assert!(app.pending_raw_open.is_none());
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "a file the host takes its own route for gets no automatic Camera Raw");
    }

    #[test]
    fn camera_raw_requests_are_validated() {
        assert!(OpenSpec::parse(&json!({"bogus": 1})).unwrap_err().contains("bogus"));
        assert!(OpenSpec::parse(&json!("x")).is_err());
        assert!(OpenSpec::parse(&json!({"recipePath": "a.txt"})).is_err());
        assert!(OpenSpec::parse(&json!({"displayPath": "a.png"})).is_err());
        assert!(OpenSpec::parse(&json!({"continueFrom": "a.jpg"})).is_err());
        assert!(OpenSpec::parse(&json!({"recipe": {"version": 9, "cameraRaw": {}}})).is_err());
        let parsed = OpenSpec::parse(
            &json!({"recipe": {"version": 1, "cameraRaw": {"exposure": 1.0}}, "recipePath": "a.json", "displayPath": "a.jpg", "continueFrom": "a.pcraft"}),
        )
        .unwrap();
        assert_eq!(parsed.recipe.unwrap().exposure, 1.0);
        // A photo with a saved project is shown from the project's pictures: its recipe is kept, but not rendered over them.
        assert_eq!(
            (parsed.targets.recipe.as_deref(), parsed.targets.display.as_deref(), parsed.continue_from.as_deref()),
            (Some("a.json"), None, Some("a.pcraft"))
        );
        let without_project = OpenSpec::parse(&json!({"recipePath": "a.json", "displayPath": "a.jpg"})).unwrap();
        assert_eq!(without_project.targets.display.as_deref(), Some("a.jpg"));
    }

    #[test]
    fn camera_raw_needs_the_workspace() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        app.raw_workspace.enabled = false;
        assert!(crate::control_files::open_with_camera_raw(&mut app, "IMG.dng", true, Some(spec(None))).is_err());
        app.raw_workspace.enabled = true;
    }

    #[test]
    fn a_browsed_raws_canvas_is_not_drawn_until_open_or_cancel() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        assert!(!canvas_covered(&app), "nothing is open yet");
        open_in_camera_raw(&mut app, &ctx, spec(None));
        assert!(canvas_covered(&app), "the dialog covers it");
        control(&mut app, &ctx, &json!({"done": true})).unwrap();
        assert!(app.camera_raw.is_none());
        assert!(canvas_covered(&app), "left with Done, the browsed raw's canvas still has nothing to show");
        // Cancel in the dialog is the user asking for the full editor.
        open_in_camera_raw(&mut app, &ctx, spec(None));
        control(&mut app, &ctx, &json!({"cancel": true})).unwrap();
        assert!(!canvas_covered(&app), "after Cancel the full editor draws");
    }

    #[test]
    fn open_hands_the_window_to_the_full_editor() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(Some(1.0)));
        control(&mut app, &ctx, &json!({"commit": true})).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert!(!canvas_covered(&app));
    }

    #[test]
    fn the_cover_ends_when_another_document_is_active() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        open_in_camera_raw(&mut app, &ctx, spec(None));
        control(&mut app, &ctx, &json!({"done": true})).unwrap();
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        assert!(!canvas_covered(&app), "a different document is a different story");
    }

    #[test]
    fn outside_the_workspace_nothing_is_covered() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        app.raw_workspace.enabled = false;
        assert!(!canvas_covered(&app));
    }

    // --- the next raw, developed ahead of time ---------------------------------------------------------

    /// Counts the reads the host's scoped reader serves.
    fn count_reads(app: &mut PhotocraftApp) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        let inner = app.services.background_automation_read.clone().expect("a reader");
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = count.clone();
        app.services.background_automation_read = Some(std::sync::Arc::new(move |path: &str| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            inner(path)
        }));
        count
    }

    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn ready_path(app: &PhotocraftApp) -> Option<String> {
        app.jobs.workspace_prefetch.lock().unwrap().ready.as_ref().map(|r| r.path.clone())
    }

    fn open_with(app: &mut PhotocraftApp, path: &str, signature: Option<&str>) -> serde_json::Value {
        let mut spec = spec(None);
        spec.signature = signature.map(str::to_string);
        let started = crate::control_files::open_with_camera_raw(app, path, true, Some(spec)).unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("expected a job") };
        app.session.wait_job(job).unwrap()
    }

    #[test]
    fn a_prepared_raw_comes_with_the_dialogs_proxy_so_opening_it_builds_none() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        request_prefetch(&mut app, "IMG.dng", "10:1").unwrap();
        wait_until("the prefetch", || ready_path(&app).is_some());
        let prepared = take_prepared(&app.jobs.workspace_prefetch, "IMG.dng", Some("10:1")).expect("prepared");
        let proxy = prepared.proxy.expect("the proxy was built on the prefetch thread");
        let from_document = camera_raw_ui::proxy_for(&prepared.imported.document).expect("a proxy for the same document");
        assert_eq!(format!("{proxy:?}"), format!("{from_document:?}"));
    }

    /// Two raws, as a host browsing a folder opens them one after the other.
    fn two_photos() -> (PhotocraftApp, Writes, egui::Context) {
        let (app, writes) = host_app(vec![("A.dng", synthetic_raw()), ("B.dng", synthetic_raw())]);
        (app, writes, egui::Context::default())
    }

    fn open_photo(app: &mut PhotocraftApp, ctx: &egui::Context, path: &str) {
        use photocraft_engine::jobs::Started;
        let mut spec = spec(None);
        spec.targets = Targets { recipe: Some(format!("{path}.cameraraw.json")), display: Some(format!("{path}.photocraft.display.jpg")), manifest: None };
        let started = crate::control_files::open_with_camera_raw(app, path, true, Some(spec)).unwrap();
        let Started::Job(job) = started else { panic!("expected a background open") };
        app.session.wait_job(job).unwrap();
        crate::jobs_ui::tick(app, ctx);
        let raw = app.pending_raw_open.take().expect("the raw's dialog is queued");
        camera_raw_ui::open_raw(app, ctx, raw).unwrap();
    }

    fn dialog_name(app: &PhotocraftApp) -> String {
        let d = app.camera_raw.as_ref().expect("a dialog is open");
        d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)["openingRaw"]["name"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn the_dialog_stays_up_while_the_next_photo_takes_it_over() {
        use photocraft_engine::jobs::Started;
        let (mut app, writes, ctx) = two_photos();
        open_photo(&mut app, &ctx, "A.dng");
        assert_eq!(dialog_name(&app), "A.dng");
        adjust(&mut app, &ctx, 1.5);

        // The host asks for the next photo while the first one's dialog is up.
        let mut spec = spec(None);
        spec.targets = Targets { recipe: Some("B.dng.cameraraw.json".into()), display: None, manifest: None };
        let started = crate::control_files::open_with_camera_raw(&mut app, "B.dng", true, Some(spec)).unwrap();
        let Started::Job(job) = started else { panic!("expected a background open") };
        let dialog = app.camera_raw.as_ref().expect("the dialog did not close");
        assert!(dialog.busy(), "marked busy until the next photo takes over");
        assert!(!shell_active(&app), "no loading frame: the dialog itself stays");
        let reported = crate::control::viewport(&app, &ctx)["cameraRaw"].clone();
        assert_eq!((reported["open"].as_bool(), reported["busy"].as_bool()), (Some(true), Some(true)), "{reported}");
        let recipe = written(&writes, "A.dng.cameraraw.json").expect("what was adjusted on the first photo was kept as its recipe");
        assert_eq!(raw_recipe::from_json(&serde_json::from_slice(&recipe).unwrap()).unwrap().exposure, 1.5);

        app.session.wait_job(job).unwrap();
    }

    #[test]
    fn the_next_photos_proxy_comes_from_the_job_and_replaces_the_dialog_in_one_step() {
        use photocraft_engine::jobs::Started;
        let (mut app, _writes, ctx) = two_photos();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        open_photo(&mut app, &ctx, "A.dng");
        let mut spec = spec(None);
        spec.targets = Targets { recipe: Some("B.dng.cameraraw.json".into()), display: None, manifest: None };
        let started = crate::control_files::open_with_camera_raw(&mut app, "B.dng", true, Some(spec)).unwrap();
        let Started::Job(job) = started else { panic!("expected a background open") };
        app.session.wait_job(job).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert_eq!(dialog_name(&app), "A.dng", "still the old photo's dialog until the frame swaps it");
        assert!(app.pending_raw_open.as_ref().is_some_and(|raw| raw.proxy.is_some()), "queued, with the proxy the job built");
        // The frame the app draws takes it over: the way the dialog really opens, not a call that bypasses it.
        ctx.run_ui(egui::RawInput::default(), |ui| camera_raw_ui::show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.pending_raw_open.is_none());
        assert_eq!(dialog_name(&app), "B.dng");
        assert!(!app.camera_raw.as_ref().unwrap().busy(), "the new photo's dialog is ready");
        assert_eq!(app.session.documents().len(), 1, "the photo left behind was closed");
        assert_eq!(app.session.active().unwrap().doc.name, "B.dng");
        assert_eq!(crate::control::viewport(&app, &ctx)["cameraRaw"]["busy"], false);
    }

    #[test]
    fn a_dialog_that_is_not_the_open_time_one_is_settled_not_handed_over() {
        let (mut app, _writes, ctx) = two_photos();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        camera_raw_ui::open(&mut app, &ctx).unwrap();
        assert!(camera_raw_ui::leave_in_place(&mut app).is_err());
        assert!(app.camera_raw.as_ref().is_some_and(|d| !d.busy()), "a filter dialog is never marked busy");
    }

    #[test]
    fn a_raw_prepared_ahead_opens_without_reading_or_developing_again() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let reads = count_reads(&mut app);
        request_prefetch(&mut app, "IMG.dng", "10:1").unwrap();
        wait_until("the prefetch", || ready_path(&app).is_some());
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        let ctx = egui::Context::default();
        assert_eq!(control(&mut app, &ctx, &json!({})).unwrap()["prefetched"], "IMG.dng", "the host can see what is ready");
        let result = open_with(&mut app, "IMG.dng", Some("10:1"));
        assert_eq!(result["cameraRaw"]["dialog"], true, "{result}");
        assert!(control(&mut app, &ctx, &json!({})).unwrap()["prefetched"].is_null(), "and that it was used");
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1, "the open used what was prepared");
        assert!(ready_path(&app).is_none(), "the slot is emptied by the open");
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn an_open_that_arrives_while_the_photo_is_being_prepared_waits_for_it() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let reads = count_reads(&mut app);
        request_prefetch(&mut app, "IMG.dng", "10:1").unwrap();
        // Under way or already done, but not picked up: either way the open must not read and develop the file again.
        wait_until("the prefetch to start", || {
            let slot = app.jobs.workspace_prefetch.lock().unwrap();
            slot.in_flight.is_some() || slot.ready.is_some()
        });
        let result = open_with(&mut app, "IMG.dng", Some("10:1"));
        assert_eq!(result["cameraRaw"]["dialog"], true, "{result}");
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1, "the open used the prefetch that was in flight");
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn waiting_for_a_prefetch_gives_up_at_once_when_none_is_in_flight_for_the_photo() {
        let shared = SharedPrefetch::default();
        assert!(wait_prepared(&shared, "A.dng", "1:1", || false).is_none(), "nothing in flight");
        shared.lock().unwrap().in_flight = Some(("B.dng".into(), "1:1".into()));
        assert!(wait_prepared(&shared, "A.dng", "1:1", || false).is_none(), "another photo");
        shared.lock().unwrap().in_flight = Some(("A.dng".into(), "2:2".into()));
        assert!(wait_prepared(&shared, "A.dng", "1:1", || false).is_none(), "the same photo, changed since");
        shared.lock().unwrap().in_flight = Some(("A.dng".into(), "1:1".into()));
        assert!(wait_prepared(&shared, "A.dng", "1:1", || true).is_none(), "the open was cancelled");
        // A prefetch that ends without a result (an unreadable file) releases the open.
        let ending = shared.clone();
        let ended = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            ending.lock().unwrap().in_flight = None;
        });
        assert!(wait_prepared(&shared, "A.dng", "1:1", || false).is_none());
        ended.join().unwrap();
    }

    #[test]
    fn a_file_that_changed_since_it_was_prepared_is_read_again() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let reads = count_reads(&mut app);
        request_prefetch(&mut app, "IMG.dng", "10:1").unwrap();
        wait_until("the prefetch", || ready_path(&app).is_some());
        let result = open_with(&mut app, "IMG.dng", Some("10:2"));
        assert_eq!(result["cameraRaw"]["dialog"], true);
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 2, "a different signature means the file is read afresh");
        let result = open_with(&mut app, "IMG.dng", None);
        assert_eq!(result["cameraRaw"]["dialog"], true, "no signature, no shortcut");
    }

    #[test]
    fn a_newer_request_replaces_an_older_one() {
        let (mut app, _writes) = host_app(vec![("A.dng", synthetic_raw()), ("B.dng", synthetic_raw())]);
        request_prefetch(&mut app, "A.dng", "10:1").unwrap();
        request_prefetch(&mut app, "B.dng", "10:1").unwrap();
        wait_until("the newer prefetch", || ready_path(&app).is_some());
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(ready_path(&app).as_deref(), Some("B.dng"), "only the last request is kept");
        assert!(take_prepared(&app.jobs.workspace_prefetch, "A.dng", Some("10:1")).is_none());
    }

    #[test]
    fn asking_twice_for_the_same_file_does_not_develop_it_twice() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let reads = count_reads(&mut app);
        assert_eq!(request_prefetch(&mut app, "IMG.dng", "10:1").unwrap()["queued"], true);
        assert_eq!(request_prefetch(&mut app, "IMG.dng", "10:1").unwrap()["queued"], false);
        wait_until("the prefetch", || ready_path(&app).is_some());
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn a_file_that_is_not_a_raw_is_never_prepared() {
        let doc = photocraft_doc::Document::new("flat", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let project = photocraft_io::export(&doc, "x.pcraft", &Default::default()).unwrap().bytes;
        let (mut app, _writes) = host_app(vec![("x.pcraft", project)]);
        let reads = count_reads(&mut app);
        request_prefetch(&mut app, "x.pcraft", "10:1").unwrap();
        wait_until("the read", || reads.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(ready_path(&app).is_none());
    }

    #[test]
    fn prefetch_needs_the_workspace_and_a_reader() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        app.raw_workspace.enabled = false;
        assert!(request_prefetch(&mut app, "IMG.dng", "10:1").is_err());
        app.raw_workspace.enabled = true;
        app.services.background_automation_read = None;
        assert!(request_prefetch(&mut app, "IMG.dng", "10:1").is_err());
    }

    #[test]
    fn the_workspace_asks_the_gpu_canvas_to_build_in_the_background_only_when_it_is_on() {
        // No GPU canvas in a unit test: the call must be harmless either way.
        let (mut app, _ctx) = app_with_document();
        app.raw_workspace.enabled = false;
        gpu_builds_in_background(&mut app);
        app.raw_workspace.enabled = true;
        gpu_builds_in_background(&mut app);
        assert!(!app.warm_gpu_compositor(), "without a GPU canvas there is nothing to build");
    }

    #[test]
    fn the_signature_is_part_of_the_request() {
        let parsed = OpenSpec::parse(&json!({"signature": "123:456"})).unwrap();
        assert_eq!(parsed.signature.as_deref(), Some("123:456"));
        assert!(OpenSpec::parse(&json!({"signature": 5})).is_err());
    }

    #[test]
    fn the_next_document_gets_camera_raw_too() {
        let (mut app, ctx) = app_with_document();
        app.raw_workspace.enabled = true;
        auto_open(&mut app, &ctx);
        app.camera_raw = None;
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_some());
    }

    #[test]
    fn a_document_camera_raw_cannot_open_on_is_left_to_the_full_editor() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.raw_workspace.enabled = true;
        auto_open(&mut app, &ctx); // no document at all: nothing happens, nothing is recorded
        assert!(app.camera_raw.is_none() && !app.ui.status_error);
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        // A fresh document has a background that may or may not take the filter; whichever it does, it
        // must not panic or reopen on every frame.
        auto_open(&mut app, &ctx);
        let opened = app.camera_raw.is_some();
        app.camera_raw = None;
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "opened={opened}: a second frame reopened the dialog");
    }
}
