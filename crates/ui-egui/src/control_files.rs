//! Host-editor file handoffs. Workers use only granted automation capabilities;
//! snapshots share COW tiles, so saving never locks the canvas or drops history.

use photocraft_engine::{EngineError, jobs::Started};
use serde_json::json;

use crate::PhotocraftApp;

/// What a host bound the open photo's document to (`app.bind`): where the editor's own Save writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct HostTarget {
    pub doc: photocraft_doc::DocId,
    pub path: String,
    pub preview: String,
    /// The screen-sized picture a host shows while browsing (see [`stash_with_display`]).
    pub display: Option<String>,
    /// The render manifest to keep (see `render_manifest`).
    pub manifest: Option<crate::render_manifest::Request>,
}

pub fn hosted_save(app: &PhotocraftApp, command: &str) -> bool {
    command == "file.save" && app.jobs.host_target.as_ref().is_some_and(|target| app.session.active().is_some_and(|state| state.doc.id == target.doc))
}

/// Document `from` was replaced by `to`, and `to` is the same photo: the raw developed again for "Open", or the photo's own
/// saved project continued in its place. A host's binding of `from` (`app.bind`) moves to `to`, so the editor's own Save keeps
/// saving the photo for the host instead of asking where to save it. A binding of any other document stays as it is.
pub(crate) fn rebind(app: &mut PhotocraftApp, from: photocraft_doc::DocId, to: photocraft_doc::DocId) {
    if let Some(target) = app.jobs.host_target.as_mut()
        && target.doc == from
    {
        target.doc = to;
    }
}

#[cfg(test)]
pub fn open(app: &mut PhotocraftApp, path: &str) -> Result<Started, String> {
    open_with_mode(app, path, false)
}

#[cfg(test)]
pub fn open_with_mode(app: &mut PhotocraftApp, path: &str, replace: bool) -> Result<Started, String> {
    open_with_camera_raw(app, path, replace, None)
}

pub fn open_with_camera_raw(app: &mut PhotocraftApp, path: &str, replace: bool, camera_raw: Option<crate::raw_workspace::OpenSpec>) -> Result<Started, String> {
    if camera_raw.is_some() && !app.raw_workspace.enabled {
        return Err("cameraRaw needs the Camera Raw workspace".into());
    }
    // Camera Raw workspace: the photo being left keeps what was adjusted, then the next one opens. When Camera Raw is
    // showing a raw and the next photo is one too, the dialog stays up and the next photo takes it over, so the window
    // never shows it closing and coming back.
    let in_place = camera_raw.is_some() && app.raw_workspace.enabled && crate::camera_raw_ui::target_name(app) == Some("openRaw");
    if in_place {
        crate::camera_raw_ui::leave_in_place(app)?;
    } else {
        crate::raw_workspace::settle(app)?;
    }
    let previous = if replace { app.session.active().map(|state| state.doc.id) } else { None };
    if let Some((doc, revision)) = app.jobs.host_snapshots.get(path) {
        let name = doc.name.clone();
        let doc = (**doc).clone();
        let revision = *revision;
        close_replaced(&mut app.session, previous);
        app.session.add_document(doc, Some(path.to_string()));
        if let Some(state) = app.session.active_mut() {
            // A pending stash owns this revision. Resetting it to zero makes
            // Save reject the very snapshot that is already being written.
            state.revision = revision;
        }
        app.sync_views();
        return Ok(Started::Done(json!({"path": path, "name": name, "warnings": []})));
    }
    let read = app.services.background_automation_read.clone().ok_or("background automation read authority is not configured")?;
    let path = path.to_string();
    let applied_path = path.clone();
    let pending_raw = app.jobs.workspace_raw.clone();
    let wants_dialog = camera_raw.is_some();
    let workspace = app.raw_workspace.enabled;
    if workspace {
        if !wants_dialog {
            app.raw_workspace.end_shell();
        } else if !in_place {
            // The photo's Camera Raw is on its way: its frame stands in for the editor's windows until the dialog opens.
            let name = std::path::Path::new(&path).file_name().and_then(|name| name.to_str()).unwrap_or(&path).to_string();
            crate::raw_workspace::begin_shell(app, &name, false);
        }
    }
    // A raw the host asked to have ready already is developed: opening it is then just adding the document.
    let prepared = camera_raw.as_ref().and_then(|spec| crate::raw_workspace::take_prepared(&app.jobs.workspace_prefetch, &path, spec.signature.as_deref()));
    // Not ready yet, but perhaps on its way: the worker waits for a prefetch of this photo that is in flight.
    let prefetch = app.jobs.workspace_prefetch.clone();
    let signature = camera_raw.as_ref().and_then(|spec| spec.signature.clone());
    app.session
        .start_job(
            "app.open",
            json!({"path": path}),
            "Opening photo",
            false,
            move |ctx| {
                ctx.check()?;
                let prepared =
                    prepared.or_else(|| signature.as_deref().and_then(|s| crate::raw_workspace::wait_prepared(&prefetch, &path, s, || ctx.cancelled())));
                ctx.check()?;
                if let Some(ready) = prepared {
                    return Ok((ready.name, ready.imported, Some(ready.bytes), ready.proxy, ready.full_size));
                }
                let (name, bytes) = read(&path).map_err(EngineError::Other)?;
                let camera_raw_import = wants_dialog && photocraft_io::raw::is_raw(&bytes);
                let mut full_size = None;
                let mut curve = None;
                let imported = if camera_raw_import {
                    // Browsing in Camera Raw: reduced in size when the file allows, so the photo is ready after little more than
                    // reading it (see `develop_for_browsing`).
                    crate::raw_workspace::develop_for_browsing(&name, &bytes).map(|browsed| {
                        full_size = browsed.full_size;
                        curve = Some(browsed.curve);
                        browsed.imported
                    })
                } else {
                    photocraft_io::import(&name, &bytes)
                }
                .map_err(|e| EngineError::Other(e.to_string()))?;
                ctx.check()?;
                // The dialog's proxy is built here, on the worker, not by the editor's own thread when the dialog opens.
                let proxy = if camera_raw_import { crate::camera_raw_ui::proxy_for(&imported.document) } else { None };
                let proxy = match (proxy, curve) {
                    (Some(proxy), Some(curve)) => Some(proxy.with_curve(curve)),
                    (proxy, _) => proxy,
                };
                let kept = camera_raw_import.then(|| std::sync::Arc::new(bytes));
                Ok((name, imported, kept, proxy, full_size))
            },
            move |session, (name, imported, kept, proxy, full_size)| {
                // Automation never runs script events or ambient color-policy I/O.
                close_replaced(session, previous);
                // Only a raw developed from its sensor data can open in Camera Raw; an embedded-preview
                // fallback is the host's cue to take its own route for that file.
                let dialog = wants_dialog && kept.is_some() && imported.warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE));
                session.add_document(imported.document, Some(applied_path.clone()));
                let mut result = json!({"path": applied_path, "name": name, "warnings": imported.warnings});
                if wants_dialog {
                    result["cameraRaw"] = json!({"dialog": dialog});
                }
                if workspace && wants_dialog {
                    // The host decided how this file opens. A file that could not be developed from its sensor data gets no
                    // dialog, and the automatic Camera Raw must leave the document alone (see `queue_dialog`).
                    let (bytes, spec, proxy, full_size) =
                        if dialog { (kept, camera_raw.unwrap_or_default(), proxy, full_size) } else { (None, Default::default(), None, None) };
                    *pending_raw.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(crate::raw_workspace::PendingRaw { path: applied_path.clone(), bytes, spec, proxy, full_size });
                }
                Ok(result)
            },
        )
        .map_err(|e| e.to_string())
}

fn close_replaced(session: &mut photocraft_engine::Session, previous: Option<photocraft_doc::DocId>) {
    if let Some(index) = previous.and_then(|id| session.documents().iter().position(|state| state.doc.id == id)) {
        session.close(index);
    }
}

/// Longest side and JPEG quality of the browsing preview a stash can write beside the project.
pub const DISPLAY_LONG_SIDE: u32 = 2560;
pub const DISPLAY_QUALITY: u8 = 90;

#[cfg(test)]
pub fn stash(app: &mut PhotocraftApp, path: &str, preview: &str) -> Result<Started, String> {
    stash_with_display(app, path, Some(preview), None)
}

/// The pool the hosted save encodes on: half of rayon's threads, its own. On rayon's shared pool the encoders took
/// every core for seconds, and whatever the window needed from that pool meanwhile (a canvas redraw) waited behind them.
/// Measured on a 45 MP 16-bit photo with the editor held to 8 logical processors, zooming while it saves: slowest answer
/// 97-901 ms before, 35-80 ms after; the save takes about a quarter longer. (The two large encoders also start their own
/// thread per core, which this does not bound: on 4 processors the stall is unchanged.)
#[cfg(not(target_arch = "wasm32"))]
fn save_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let threads = (rayon::current_num_threads() / 2).max(1);
        rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|index| format!("host-save-{index}")).build().ok()
    })
    .as_ref()
}

/// Runs the hosted save's encoders side by side on [`save_pool`]. An encoder that panics is an `Err` for that encoder
/// alone; the others still finish.
fn side_by_side<A: Send, B: Send, C: Send>(
    project: impl FnOnce() -> A + Send,
    render: impl FnOnce() -> B + Send,
    shown: Option<impl FnOnce() -> C + Send>,
) -> (std::thread::Result<A>, std::thread::Result<B>, Option<std::thread::Result<C>>) {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let project = move || catch_unwind(AssertUnwindSafe(project));
    let render = move || catch_unwind(AssertUnwindSafe(render));
    let shown = move || shown.map(|encode| catch_unwind(AssertUnwindSafe(encode)));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let all = move || {
            let (project, (render, shown)) = rayon::join(project, || rayon::join(render, shown));
            (project, render, shown)
        };
        match save_pool() {
            Some(pool) => pool.install(all),
            None => all(),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        (project(), render(), shown())
    }
}

/// [`stash`] that also writes `display`, a screen-sized sRGB JPEG of the same snapshot, for hosts
/// that show an edited photo from a cached render while browsing. It is made beside the project
/// encode (from the compositor's reduced render, not a full-size one) and written as soon as it is
/// ready, so a quick return to the photo can use it long before the project and full render land.
/// It is a convenience: a failure to make or write it never fails the save.
#[cfg(test)]
pub fn stash_with_display(app: &mut PhotocraftApp, path: &str, preview: Option<&str>, display: Option<&str>) -> Result<Started, String> {
    stash_with_outputs(app, path, preview, display, None)
}

/// The editor's own Save of a document a host bound ([`HostTarget`]): what the host's saves write.
pub fn save_bound(app: &mut PhotocraftApp, target: &HostTarget) -> Result<Started, String> {
    stash_with_outputs(app, &target.path, Some(&target.preview), target.display.as_deref(), target.manifest.clone())
}

/// [`stash_with_display`] that, with `manifest`, keeps the photo's render manifest: the full-size and screen-sized pictures are
/// written under names of this save's own (`render_manifest::versioned`), and once they and the project are written the manifest
/// is switched to them. Only pictures that were written are listed; a manifest that cannot be written fails the save.
pub fn stash_with_outputs(
    app: &mut PhotocraftApp,
    path: &str,
    preview: Option<&str>,
    display: Option<&str>,
    manifest: Option<crate::render_manifest::Request>,
) -> Result<Started, String> {
    crate::raw_workspace::settle(app)?;
    if manifest.as_ref().is_some_and(|request| !request.manifest.to_ascii_lowercase().ends_with(".json")) {
        return Err("stash `manifest` must be a .json file".into());
    }
    if preview.is_none() && display.is_none() {
        return Err("stash requires a preview or a display render".into());
    }
    if !path.to_ascii_lowercase().ends_with(".pcraft") || preview.is_some_and(|preview| !preview.to_ascii_lowercase().ends_with(".png")) {
        return Err("stash requires a .pcraft project and a .png preview".into());
    }
    if let Some(display) = display
        && !(display.to_ascii_lowercase().ends_with(".jpg") || display.to_ascii_lowercase().ends_with(".jpeg"))
    {
        return Err("stash `display` must be a .jpg file".into());
    }
    let write = app.services.background_automation_write.clone().ok_or("background automation write authority is not configured")?;
    let state = app.session.active().ok_or("no document")?;
    let doc = state.doc.clone();
    let revision = state.revision;
    if let Some(job) = app.jobs.host_inflight.get(path).copied()
        && app.session.job(job).is_some_and(|info| info.state == "running")
    {
        if app.jobs.host_snapshots.get(path).is_some_and(|(snapshot, saved)| snapshot.id == doc.id && *saved == revision) {
            return Ok(Started::Job(job));
        }
        return Err("The previous photo snapshot is still saving; the current edits are retained".into());
    }
    let path = path.to_string();
    // Numbered when this save is taken, so a save asked for later always has the higher number.
    let generation = manifest.as_ref().map(|_| crate::render_manifest::generation());
    let named = |target: Option<&str>| target.map(|target| generation.map_or_else(|| target.to_string(), |n| crate::render_manifest::versioned(target, n)));
    let preview = named(preview);
    let display = named(display);
    let applied_path = path.clone();
    let id = doc.id;
    let job_path = path.clone();
    app.jobs.host_snapshots.insert(path.clone(), (doc.clone(), revision));
    let result = app
        .session
        .start_job(
            "app.stash",
            json!({"path": path, "preview": preview}),
            "Saving photo edits",
            false,
            move |ctx| {
                ctx.check()?;
                // The project, its preview and the display render are independent encodes of the same
                // snapshot: run them side by side. All finish before any result is looked at.
                let (project, render, shown) = side_by_side(
                    || photocraft_io::export(&doc, &path, &Default::default()),
                    // The full-size render is the heaviest encode of a save; a host that autosaves while the photo is open leaves it out.
                    || preview.as_ref().map(|target| photocraft_io::export(&doc, target, &Default::default())),
                    // Small and quick: written the moment it exists, ahead of the large files.
                    display.as_ref().map(|target| {
                        || {
                            let jpeg = photocraft_io::export_display_jpeg(&doc, DISPLAY_LONG_SIDE, DISPLAY_QUALITY).map_err(|e| e.to_string())?;
                            write(target, &jpeg.bytes)?;
                            Ok::<_, String>(jpeg.bytes)
                        }
                    }),
                );
                let render = render.map_err(|_| EngineError::Other("preview encoder stopped unexpectedly".into()))?;
                let project =
                    project.map_err(|_| EngineError::Other("project encoder stopped unexpectedly".into()))?.map_err(|e| EngineError::Other(e.to_string()))?;
                let render = render.transpose().map_err(|e| EngineError::Other(e.to_string()))?;
                ctx.check()?;
                // Each capability write is atomic. Report success only after both. With a manifest the render goes first, under its
                // own name: the project then replaces the old one, and the manifest follows at once (until it does, a reader finds
                // the old manifest no longer matching the project, and treats its pictures as out of date).
                let write_render = || match (&preview, &render) {
                    (Some(target), Some(render)) => write(target, &render.bytes).map_err(EngineError::Other),
                    _ => Ok(()),
                };
                if manifest.is_some() {
                    write_render()?;
                    write(&path, &project.bytes).map_err(EngineError::Other)?;
                } else {
                    write(&path, &project.bytes).map_err(EngineError::Other)?;
                    write_render()?;
                }
                let mut warnings = project.warnings.clone();
                let shown = match shown {
                    Some(Ok(Err(error))) => {
                        warnings.push(format!("display preview not written: {error}"));
                        None
                    }
                    Some(Err(_)) => {
                        warnings.push("display preview not written: its encoder stopped unexpectedly".to_string());
                        None
                    }
                    Some(Ok(Ok(bytes))) => Some(bytes),
                    None => None,
                };
                if let (Some(request), Some(n)) = (&manifest, generation) {
                    use crate::render_manifest::{Made, content, publish, sha256_hex};
                    let made = Made::Project { size: project.bytes.len() as u64, sha256: sha256_hex(&project.bytes) };
                    let mut outputs: Vec<(&str, &str, &[u8])> = Vec::new();
                    if let (Some(target), Some(render)) = (&preview, &render) {
                        outputs.push(("png", target.as_str(), render.bytes.as_slice()));
                    }
                    if let (Some(target), Some(bytes)) = (&display, &shown) {
                        outputs.push(("display", target.as_str(), bytes.as_slice()));
                    }
                    publish(&write, request, n, &content(n, &made, request.source.as_deref(), &outputs)).map_err(EngineError::Other)?;
                }
                Ok((warnings, render.map(|render| render.warnings).unwrap_or_default()))
            },
            move |session, (mut warnings, render_warnings)| {
                if let Some(state) = session.active_mut().filter(|state| state.doc.id == id && state.revision == revision) {
                    state.path = Some(applied_path.clone());
                    state.saved_revision = revision;
                }
                warnings.extend(render_warnings);
                Ok(json!({"path": applied_path, "revision": revision, "warnings": warnings}))
            },
        )
        .map_err(|e| e.to_string())?;
    if let Started::Job(job) = result {
        app.jobs.host_inflight.insert(job_path, job);
        app.jobs.last_started = Some(job);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    fn app(write: crate::BackgroundAutomationWriteFn) -> PhotocraftApp {
        let services = crate::Services { background_automation_write: Some(write), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.add_document(Document::new("photo", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
        app
    }

    fn finish(app: &mut PhotocraftApp, started: Started) -> Result<Value, String> {
        match started {
            Started::Done(value) => Ok(value),
            Started::Job(job) => app.session.wait_job(job).map_err(|e| e.to_string()),
        }
    }

    use serde_json::Value;

    #[test]
    fn stash_returns_before_io_and_restore_uses_the_memory_snapshot() {
        let gate = Arc::new(AtomicBool::new(false));
        let writer_gate = gate.clone();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            while !writer_gate.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let revision = app.session.active().unwrap().revision;
        let started = stash(&mut app, "photo.jpg.pcraft", "photo.jpg.photocraft.png").unwrap();
        assert!(matches!(started, Started::Job(_)));
        assert!(app.session.active_job().is_none(), "a save must not lock editing");
        app.session.execute("file.close", json!({})).unwrap();
        let restored = open(&mut app, "photo.jpg.pcraft").unwrap();
        assert!(matches!(restored, Started::Done(_)), "restore must not wait for disk");
        assert_eq!(app.session.active().unwrap().doc.name, "photo");
        let restored_revision = app.session.active().unwrap().revision;
        let repeated_save = stash(&mut app, "photo.jpg.pcraft", "photo.jpg.photocraft.png");
        let dirty_while_saving = app.session.active().unwrap().is_dirty();
        gate.store(true, Ordering::Relaxed);
        finish(&mut app, started).unwrap();
        assert_eq!(restored_revision, revision, "restore must retain the pending snapshot's revision");
        assert!(matches!(repeated_save, Ok(Started::Job(_))), "Save should reuse the pending snapshot: {repeated_save:?}");
        assert!(dirty_while_saving);
        assert!(!app.session.active().unwrap().is_dirty(), "completion should mark the restored revision saved");
        assert_eq!(writes.lock().unwrap().len(), 2);
    }

    #[test]
    fn failed_save_retains_snapshot_and_does_not_mark_clean() {
        let mut app = app(Arc::new(|_, _| Err("disk full".into())));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let started = stash(&mut app, "photo.pcraft", "photo.png").unwrap();
        assert!(finish(&mut app, started).unwrap_err().contains("disk full"));
        crate::jobs_ui::tick(&mut app, &egui::Context::default());
        assert!(app.session.active().unwrap().is_dirty());
        app.session.execute("file.close", json!({})).unwrap();
        assert!(matches!(open(&mut app, "photo.pcraft").unwrap(), Started::Done(_)));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn no_authority_and_invalid_targets_fail_without_writing() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(open(&mut app, "../photo.jpg").is_err());
        assert!(stash(&mut app, "photo.jpg", "preview.png").is_err());
        assert!(stash(&mut app, "photo.pcraft", "preview.png").is_err());
    }

    #[test]
    fn hosted_save_uses_scoped_io_even_when_ambient_file_commands_are_denied() {
        let mut app = app(Arc::new(|_, _| Ok(())));
        app.services.automation_command = Some(Box::new(|_, _| Err("ambient IO denied".into())));
        app.services.automation_authorize = Some(|_, _| Err(EngineError::Other("ambient engine IO denied".into())));
        app.automation_input = true;
        let id = app.session.active().unwrap().doc.id;
        app.jobs.host_target = Some(HostTarget { doc: id, path: "photo.pcraft".into(), preview: "preview.png".into(), display: None, manifest: None });
        let ctx = egui::Context::default();
        let (request, _) = crate::ControlRequest::new("ui.menu.invoke", json!({"id": "file.save", "wait": false}));
        let response = crate::control::handle(&mut app, &ctx, &request);
        let crate::control::Outcome::Done(value) = response else { panic!("expected an immediate job response") };
        assert_eq!(value["ok"], true);
        assert!(app.session.authorize.is_none(), "control request leaked its authorization gate");
        let job = photocraft_engine::jobs::JobId(value["result"]["job"].as_u64().unwrap());
        app.session.wait_job(job).unwrap();
        assert!(!hosted_save(&app, "file.saveAs"));
        let (request, _) = crate::ControlRequest::new("ui.menu.invoke", json!({"id": "file.saveAs"}));
        let response = crate::control::handle(&mut app, &ctx, &request);
        let crate::control::Outcome::Done(value) = response else { panic!("expected an authorization error") };
        assert_eq!(value["ok"], false);
    }

    #[test]
    fn stash_with_display_writes_a_small_jpeg_beside_the_project() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let started = stash_with_display(&mut app, "photo.pcraft", Some("photo.png"), Some("photo.display.jpg")).unwrap();
        finish(&mut app, started).unwrap();
        let writes = writes.lock().unwrap();
        let display = writes.iter().find(|(path, _)| path == "photo.display.jpg").expect("display written");
        assert!(display.1.starts_with(&[0xFF, 0xD8]), "not a JPEG");
        assert_eq!(writes.len(), 3);
    }

    fn manifest_written(writes: &[(String, Vec<u8>)], path: &str) -> Option<Value> {
        writes.iter().rev().find(|(name, _)| name == path).map(|(_, bytes)| serde_json::from_slice(bytes).unwrap())
    }

    /// Binds the open document as a host does, with a manifest path of its own (the editor's record of what it published is
    /// process-wide, and tests run side by side). Returns the manifest path.
    fn bound(app: &mut PhotocraftApp) -> String {
        let manifest = format!("photo{}.photocraft.renders.json", crate::render_manifest::generation());
        let id = app.session.active().unwrap().doc.id;
        app.jobs.host_target = Some(HostTarget {
            doc: id,
            path: "photo.jpg.pcraft".into(),
            preview: "photo.jpg.photocraft.png".into(),
            display: Some("photo.jpg.photocraft.display.jpg".into()),
            manifest: Some(crate::render_manifest::Request { manifest: manifest.clone(), source: Some("12:34".into()) }),
        });
        manifest
    }

    #[test]
    fn native_save_of_a_bound_photo_writes_its_pictures_and_the_manifest_last() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let manifest_path = bound(&mut app);
        let target = app.jobs.host_target.clone().unwrap();
        let started = save_bound(&mut app, &target).unwrap();
        finish(&mut app, started).unwrap();
        let writes = writes.lock().unwrap();
        let (last, _) = writes.last().unwrap();
        assert_eq!(*last, manifest_path, "the manifest is switched after everything it names is written");
        let manifest = manifest_written(&writes, &manifest_path).unwrap();
        let n = manifest["generation"].as_u64().unwrap();
        assert_eq!(manifest["kind"], "project");
        assert_eq!(manifest["source"], "12:34");
        assert_eq!(manifest["renderer"]["algorithm"], crate::render_manifest::ALGORITHM);
        let png = format!("photo.jpg.photocraft.g{n}.png");
        let display = format!("photo.jpg.photocraft.display.g{n}.jpg");
        assert_eq!(manifest["outputs"]["png"]["file"], png.as_str());
        assert_eq!(manifest["outputs"]["display"]["file"], display.as_str());
        assert_eq!(manifest["outputs"]["png"]["format"], "png");
        assert_eq!(manifest["outputs"]["display"]["format"], "jpeg");
        assert!(writes.iter().any(|(path, _)| *path == png) && writes.iter().any(|(path, _)| *path == display));
        let project = writes.iter().find(|(path, _)| path == "photo.jpg.pcraft").unwrap();
        assert_eq!(manifest["project"]["sha256"], crate::render_manifest::sha256_hex(&project.1).as_str());
        assert_eq!(manifest["project"]["size"], project.1.len());
        assert!(!writes.iter().any(|(path, _)| path == "photo.jpg.photocraft.png"), "never the fixed name: a picture is not rewritten in place");
    }

    #[test]
    fn a_save_after_an_autosave_publishes_a_newer_generation_with_the_full_size_render() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let manifest_path = bound(&mut app);
        let target = app.jobs.host_target.clone().unwrap();
        // The host's autosave: no full-size render.
        let started = stash_with_outputs(&mut app, &target.path, None, target.display.as_deref(), target.manifest.clone()).unwrap();
        finish(&mut app, started).unwrap();
        let autosaved = manifest_written(&writes.lock().unwrap(), &manifest_path).unwrap();
        assert!(autosaved["outputs"].get("png").is_none(), "{autosaved}");
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let started = save_bound(&mut app, &target).unwrap();
        finish(&mut app, started).unwrap();
        let saved = manifest_written(&writes.lock().unwrap(), &manifest_path).unwrap();
        assert!(saved["generation"].as_u64() > autosaved["generation"].as_u64());
        assert!(saved["outputs"]["png"].is_object() && saved["outputs"]["display"].is_object());
    }

    #[test]
    fn a_render_that_cannot_be_written_publishes_no_manifest() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            if path.ends_with(".png") {
                return Err("disk full".into());
            }
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let manifest_path = bound(&mut app);
        let target = app.jobs.host_target.clone().unwrap();
        let started = save_bound(&mut app, &target).unwrap();
        assert!(finish(&mut app, started).unwrap_err().contains("disk full"));
        let writes = writes.lock().unwrap();
        assert!(manifest_written(&writes, &manifest_path).is_none());
        assert!(!writes.iter().any(|(path, _)| path == "photo.jpg.pcraft"), "nor a project the pictures do not match");
    }

    #[test]
    fn a_screen_sized_picture_that_cannot_be_written_is_left_out_of_the_manifest() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            if path.ends_with(".jpg") {
                return Err("read-only".into());
            }
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let manifest_path = bound(&mut app);
        let target = app.jobs.host_target.clone().unwrap();
        let started = save_bound(&mut app, &target).unwrap();
        finish(&mut app, started).unwrap();
        let manifest = manifest_written(&writes.lock().unwrap(), &manifest_path).unwrap();
        assert!(manifest["outputs"]["png"].is_object());
        assert!(manifest["outputs"].get("display").is_none(), "{manifest}");
    }

    #[test]
    fn a_failed_display_write_does_not_fail_the_save() {
        let mut app = app(Arc::new(|path, _| if path.ends_with(".jpg") { Err("read-only".into()) } else { Ok(()) }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let started = stash_with_display(&mut app, "photo.pcraft", Some("photo.png"), Some("photo.display.jpg")).unwrap();
        let result = finish(&mut app, started).expect("the project and render still save");
        assert!(result["warnings"].to_string().contains("display preview not written"), "{result}");
        assert!(!app.session.active().unwrap().is_dirty());
    }

    #[test]
    fn stash_without_a_preview_writes_the_project_and_the_display_jpeg_only() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let started = stash_with_display(&mut app, "photo.pcraft", None, Some("photo.display.jpg")).unwrap();
        finish(&mut app, started).unwrap();
        let writes = writes.lock().unwrap();
        let mut paths: Vec<&str> = writes.iter().map(|(path, _)| path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["photo.display.jpg", "photo.pcraft"], "no full-size render was encoded or written");
        assert!(!app.session.active().unwrap().is_dirty(), "the save still counts as a save");
    }

    #[test]
    fn stash_needs_a_preview_or_a_display_render() {
        let mut app = app(Arc::new(|_, _| Ok(())));
        let error = stash_with_display(&mut app, "photo.pcraft", None, None).unwrap_err();
        assert!(error.contains("preview or a display"), "{error}");
        assert!(stash_with_display(&mut app, "photo.pcraft", Some("photo.display.jpg"), None).is_err(), "a preview is a .png");
    }

    #[test]
    fn stash_display_must_be_a_jpeg() {
        let mut app = app(Arc::new(|_, _| Ok(())));
        assert!(stash_with_display(&mut app, "photo.pcraft", Some("photo.png"), Some("photo.png")).is_err());
        assert!(stash_with_display(&mut app, "photo.pcraft", Some("photo.png"), Some("photo.JPEG")).is_ok());
    }

    #[test]
    fn switching_photos_in_the_camera_raw_workspace_keeps_what_was_adjusted() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let mut app = app(Arc::new(move |path, bytes| {
            recorded.lock().unwrap().push((path.to_string(), bytes.len()));
            Ok(())
        }));
        let ctx = egui::Context::default();
        app.raw_workspace.enabled = true;
        app.session.execute("layer.new.layer", json!({})).unwrap();
        crate::raw_workspace::auto_open(&mut app, &ctx);
        assert!(app.camera_raw.is_some());
        crate::camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        // The host saves the photo it is leaving: the open dialog's changes land in that save.
        let started = stash(&mut app, "photo.pcraft", "photo.png").unwrap();
        finish(&mut app, started).unwrap();
        assert!(app.camera_raw.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        assert_eq!(writes.lock().unwrap().len(), 2);
    }

    #[test]
    fn stashing_past_an_untouched_camera_raw_dialog_does_not_edit_the_photo() {
        let mut app = app(Arc::new(|_, _| Ok(())));
        let ctx = egui::Context::default();
        app.raw_workspace.enabled = true;
        app.session.execute("layer.new.layer", json!({})).unwrap();
        crate::raw_workspace::auto_open(&mut app, &ctx);
        let steps = app.session.active().unwrap().history.past_len();
        let started = stash(&mut app, "photo.pcraft", "photo.png").unwrap();
        finish(&mut app, started).unwrap();
        assert!(app.camera_raw.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }

    #[test]
    fn stash_preserves_project_sample_depth() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let project = Arc::new(Mutex::new(Vec::new()));
            let recorded = project.clone();
            let mut app = app(Arc::new(move |path, bytes| {
                if path.ends_with(".pcraft") {
                    *recorded.lock().unwrap() = bytes.to_vec();
                }
                Ok(())
            }));
            app.session.execute("file.close", json!({})).unwrap();
            app.session.add_document(Document::new("depth", Size::new(4, 4), ColorMode::Rgb, depth), None);
            let started = stash(&mut app, "photo.pcraft", "preview.png").unwrap();
            finish(&mut app, started).unwrap();
            let imported = photocraft_io::import("photo.pcraft", &project.lock().unwrap()).unwrap();
            assert_eq!(imported.document.depth, depth);
        }
    }

    #[test]
    fn replacement_keeps_old_document_until_decode_succeeds() {
        let gate = Arc::new(AtomicBool::new(false));
        let reader_gate = gate.clone();
        let replacement = Document::new("replacement", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
        let bytes = photocraft_io::export(&replacement, "next.pcraft", &Default::default()).unwrap().bytes;
        let mut app = app(Arc::new(|_, _| Ok(())));
        let previous = app.session.active().unwrap().doc.id;
        app.services.background_automation_read = Some(Arc::new(move |_| {
            while !reader_gate.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Ok(("next.pcraft".into(), bytes.clone()))
        }));
        let started = open_with_mode(&mut app, "next.pcraft", true).unwrap();
        assert_eq!(app.session.active().unwrap().doc.id, previous);
        assert_eq!(app.session.documents().len(), 1);
        gate.store(true, Ordering::Relaxed);
        finish(&mut app, started).unwrap();
        // A saved project is named after the file it was saved under (upstream #3571), not after the document it held.
        assert_eq!(app.session.active().unwrap().doc.name, "next.pcraft");
        assert_ne!(app.session.active().unwrap().doc.id, previous, "the replacement is the open document");
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn failed_replacement_retains_previous_document_and_edits() {
        let mut app = app(Arc::new(|_, _| Ok(())));
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let previous = app.session.active().unwrap().doc.id;
        app.services.background_automation_read = Some(Arc::new(|_| Err("read failed".into())));
        let started = open_with_mode(&mut app, "missing.pcraft", true).unwrap();
        assert!(finish(&mut app, started).unwrap_err().contains("read failed"));
        assert_eq!(app.session.active().unwrap().doc.id, previous);
        assert!(app.session.active().unwrap().is_dirty());
        assert_eq!(app.session.documents().len(), 1);
    }
}
