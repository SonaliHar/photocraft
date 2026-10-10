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
    /// "Open" is continuing a saved document: the next one an `app.open` produces is that document.
    pub(crate) expect_continue: bool,
    /// The raw being browsed in Camera Raw: its document exists only to feed the dialog. While it is the active
    /// document and nobody has pressed Open or Cancel, nothing draws it (a 45 MP canvas costs seconds, and the host
    /// shows its own picture of the photo meanwhile), including after the dialog was left with Done.
    pub(crate) browsing: Option<DocId>,
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
        || app.raw_workspace.browsing.is_some_and(|id| app.session.active().is_some_and(|st| st.doc.id == id))
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
/// it is settled too. A raw opened for Camera Raw gets its dialog queued.
pub fn on_open_event(app: &mut PhotocraftApp, document: Option<DocId>, done: bool) {
    if std::mem::take(&mut app.raw_workspace.expect_continue) && done {
        if let Some(id) = document.or_else(|| app.session.active().map(|st| st.doc.id)) {
            claim(app, id);
        }
        return;
    }
    if done {
        queue_dialog(app, document);
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
        if let Some(key) = object.keys().find(|key| !["recipe", "recipePath", "displayPath", "continueFrom", "signature"].contains(&key.as_str())) {
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
        let targets = Targets { recipe: text("recipePath")?, display: text("displayPath")? };
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
}

#[derive(Default)]
pub struct Prefetch {
    /// What the host asked for last (path, signature); anything else that finishes is thrown away.
    want: Option<(String, String)>,
    ready: Option<Prepared>,
}

pub type SharedPrefetch = std::sync::Arc<std::sync::Mutex<Prefetch>>;

/// The prepared raw for `path`, if one is ready and was made from the file the host still has (`signature`). Anything
/// prepared for another photo is dropped: the host has moved on.
pub fn take_prepared(shared: &SharedPrefetch, path: &str, signature: Option<&str>) -> Option<Prepared> {
    let mut slot = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ready = slot.ready.take()?;
    (ready.path == path && signature == Some(ready.signature.as_str())).then_some(ready)
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
            let Ok((name, bytes)) = read(&path) else { return };
            if !photocraft_io::raw::is_raw(&bytes) || !still_wanted(&shared) {
                return;
            }
            // One worker thread for the develop (it parallelises through rayon): whatever is open keeps the other cores.
            let pool = POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().num_threads(1).thread_name(|_| "camera-raw-prefetch-worker".into()).build().ok());
            let imported = match pool {
                Some(pool) => pool.install(|| photocraft_io::raw::import_raw_browse(&name, &bytes)),
                None => photocraft_io::raw::import_raw_browse(&name, &bytes),
            };
            let Ok(imported) = imported else { return };
            let mut slot = shared.lock().unwrap_or_else(PoisonError::into_inner);
            if slot.want.as_ref().is_some_and(|(p, s)| *p == path && *s == signature) {
                slot.ready = Some(Prepared { path, signature, name, bytes: std::sync::Arc::new(bytes), imported });
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

/// A document the host opened for the Camera Raw workspace: for a raw developed from its sensor data, its bytes
/// (the dialog re-develops from them) and the host's request; without bytes, a file the host takes its own route
/// for (the retained-raw smart object, or a fallback after the sensor decode was unavailable), which must not get
/// an automatic Camera Raw dialog.
pub struct PendingRaw {
    pub path: String,
    pub bytes: Option<std::sync::Arc<Vec<u8>>>,
    pub spec: OpenSpec,
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
    });
}

/// `ui.cameraRaw {open?, commit?, cancel?}`: the workspace's dialog for a host.
///
/// * no flags: the dialog's state;
/// * `open`: opens it on the active document when it is not open;
/// * `commit`: applies it as `settle` does (changed settings as one step, an untouched dialog just closes);
/// * `cancel`: closes it without applying anything.
pub fn control(app: &mut PhotocraftApp, ctx: &egui::Context, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::json;
    if let Some(object) = p.as_object()
        && let Some(key) = object.keys().find(|key| !["open", "set", "commit", "cancel", "done", "wait"].contains(&key.as_str()))
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
    if let Some(set) = p.get("set") {
        // Adjusting the open dialog. `ui.menu.invoke filter.cameraRaw {ui: {set}}` is refused while Camera Raw
        // is open (menu commands are blocked behind a modal), so the workspace has its own route to the same code.
        if app.camera_raw.is_none() {
            return Err("Camera Raw is not open".into());
        }
        camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &json!({"ui": {"set": set}})).ok_or("Camera Raw is not open")??;
    }
    if done {
        // Leaving the photo: a raw keeps its settings as a recipe and nothing is developed.
        left = Some(camera_raw_ui::done(app)?);
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
        "workspace": app.raw_workspace.enabled, "target": camera_raw_ui::target_name(app)});
    if let Some(recipe) = camera_raw_ui::recipe(app) {
        state["recipe"] = recipe;
    }
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
    if !app.raw_workspace.enabled || app.camera_raw.is_some() || app.pending_raw_open.is_some() || app.raw_development.is_some() {
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
    fn the_control_adjusts_the_open_dialog() {
        let (mut app, ctx) = app_with_document();
        assert!(control(&mut app, &ctx, &json!({"set": {"exposure": 1.0}})).unwrap_err().contains("not open"));
        control(&mut app, &ctx, &json!({"open": true})).unwrap();
        let state = control(&mut app, &ctx, &json!({"set": {"exposure": 1.0, "contrast": 25}})).unwrap();
        assert_eq!(state["changed"], true);
        // A bad request changes nothing.
        assert!(control(&mut app, &ctx, &json!({"set": {"nonsense": 1}})).unwrap_err().contains("nonsense"));
        assert!(control(&mut app, &ctx, &json!({"set": 3})).is_err());
        assert_eq!(control(&mut app, &ctx, &json!({})).unwrap()["changed"], true);
        // The menu route is refused behind the open dialog (upstream's modal policy), which is why `set` exists.
        assert!(!crate::menus::modal_allows(&app, "filter.cameraRaw"));
        // Settings and the leaving action can be one call; so can opening and adjusting.
        let steps = history(&app);
        let applied = control(&mut app, &ctx, &json!({"set": {"exposure": -1.0}, "commit": true})).unwrap();
        assert_eq!((applied["open"].as_bool(), applied["applied"].as_bool()), (Some(false), Some(true)));
        assert_eq!(history(&app), steps + 1);
        let again = control(&mut app, &ctx, &json!({"open": true, "set": {"exposure": 0.5}})).unwrap();
        assert_eq!((again["open"].as_bool(), again["changed"].as_bool()), (Some(true), Some(true)));
        assert!(control(&mut app, &ctx, &json!({"open": true, "set": {"exposure": 1.0}, "cancel": true})).is_err());
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

    fn synthetic_raw() -> Vec<u8> {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        let (w, h) = (48, 32);
        let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
        spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
        spec.build()
    }

    fn spec(recipe: Option<f32>) -> OpenSpec {
        let mut spec = OpenSpec {
            targets: Targets { recipe: Some("IMG.dng.cameraraw.json".into()), display: Some("IMG.dng.photocraft.display.jpg".into()) },
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
        let started = crate::control_files::open_with_sensor(app, "IMG.dng", true, false, None, Some(spec)).unwrap();
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
        open_in_camera_raw(&mut app, &ctx, spec(None));
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
        while written(&writes, "IMG.dng.photocraft.display.jpg").is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(written(&writes, "IMG.dng.photocraft.display.jpg").is_some_and(|b| b.starts_with(&[0xFF, 0xD8])), "render written");
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
        assert_eq!(app.session.active().unwrap().doc.name, "saved edits");
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "the saved document is the full editor's: Camera Raw must not open on it again");
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
        assert!(!app.raw_workspace.expect_continue, "a failed open clears the expectation");
    }

    #[test]
    fn a_file_that_is_not_a_raw_reports_no_dialog_so_the_host_can_take_its_own_route() {
        let doc = photocraft_doc::Document::new("flat", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let project = photocraft_io::export(&doc, "x.pcraft", &Default::default()).unwrap().bytes;
        let (mut app, _writes) = host_app(vec![("x.pcraft", project)]);
        let ctx = egui::Context::default();
        let started = crate::control_files::open_with_sensor(&mut app, "x.pcraft", true, false, None, Some(spec(None))).unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("expected a job") };
        let result = app.session.wait_job(job).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        assert_eq!(result["cameraRaw"]["dialog"], false, "{result}");
        assert!(app.pending_raw_open.is_none());
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none(), "a file the host takes its own route for gets no automatic Camera Raw");
    }

    #[test]
    fn the_retained_raw_route_gets_no_automatic_camera_raw() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let ctx = egui::Context::default();
        let started = crate::control_files::open_with_sensor(&mut app, "IMG.dng", true, true, None, None).unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("expected a job") };
        app.session.wait_job(job).unwrap();
        crate::jobs_ui::tick(&mut app, &ctx);
        auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_none() && app.pending_raw_open.is_none());
        assert_eq!(app.session.documents().len(), 1);
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
        assert_eq!(
            (parsed.targets.recipe.as_deref(), parsed.targets.display.as_deref(), parsed.continue_from.as_deref()),
            (Some("a.json"), Some("a.jpg"), Some("a.pcraft"))
        );
    }

    #[test]
    fn camera_raw_needs_the_workspace() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        app.raw_workspace.enabled = false;
        assert!(crate::control_files::open_with_sensor(&mut app, "IMG.dng", true, false, None, Some(spec(None))).is_err());
        app.raw_workspace.enabled = true;
        assert!(crate::control_files::open_with_sensor(&mut app, "IMG.dng", true, true, None, Some(spec(None))).is_err(), "not with a smart-object import");
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
        let started = crate::control_files::open_with_sensor(app, path, true, false, None, Some(spec)).unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("expected a job") };
        app.session.wait_job(job).unwrap()
    }

    #[test]
    fn a_raw_prepared_ahead_opens_without_reading_or_developing_again() {
        let (mut app, _writes) = host_app(vec![("IMG.dng", synthetic_raw())]);
        let reads = count_reads(&mut app);
        request_prefetch(&mut app, "IMG.dng", "10:1").unwrap();
        wait_until("the prefetch", || ready_path(&app).is_some());
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        let result = open_with(&mut app, "IMG.dng", Some("10:1"));
        assert_eq!(result["cameraRaw"]["dialog"], true, "{result}");
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1, "the open used what was prepared");
        assert!(ready_path(&app).is_none(), "the slot is emptied by the open");
        assert_eq!(app.session.documents().len(), 1);
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
