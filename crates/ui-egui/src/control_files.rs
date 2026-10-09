//! Host-editor file handoffs. Workers use only granted automation capabilities;
//! snapshots share COW tiles, so saving never locks the canvas or drops history.

use photocraft_engine::{EngineError, jobs::Started};
use serde_json::json;

use crate::PhotocraftApp;

pub fn hosted_save(app: &PhotocraftApp, command: &str) -> bool {
    command == "file.save" && app.jobs.host_target.as_ref().is_some_and(|(id, _, _)| app.session.active().is_some_and(|state| state.doc.id == *id))
}

#[cfg(test)]
pub fn open(app: &mut PhotocraftApp, path: &str) -> Result<Started, String> {
    open_with_mode(app, path, false)
}

#[cfg(test)]
pub fn open_with_mode(app: &mut PhotocraftApp, path: &str, replace: bool) -> Result<Started, String> {
    open_with_options(app, path, replace, false)
}

#[cfg(test)]
pub fn open_with_options(app: &mut PhotocraftApp, path: &str, replace: bool, raw_smart: bool) -> Result<Started, String> {
    open_with_sensor(app, path, replace, raw_smart, None)
}

pub fn open_with_sensor(app: &mut PhotocraftApp, path: &str, replace: bool, raw_smart: bool, sensor_path: Option<&str>) -> Result<Started, String> {
    if sensor_path.is_some() && !raw_smart {
        return Err("rawSensorPath requires rawSmartObject".into());
    }
    let sensor_path = sensor_path.map(str::to_string);
    crate::raw_develop_ui::settle(app);
    if app.raw_development.is_some() {
        return Err("Commit or cancel RAW Development before opening another document".into());
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
    app.session
        .start_job(
            "app.open",
            json!({"path": path}),
            "Opening photo",
            false,
            move |ctx| {
                ctx.check()?;
                let (name, bytes) = read(&path).map_err(EngineError::Other)?;
                let sensor = sensor_path.as_ref().map(|path| read(path).map_err(EngineError::Other)).transpose()?;
                let imported = if raw_smart {
                    photocraft_io::raw_smart::import_with_sensor(&name, &bytes, sensor.as_ref().map(|(name, bytes)| (name.as_str(), bytes.as_slice())))
                } else {
                    photocraft_io::import(&name, &bytes)
                }
                .map_err(|e| EngineError::Other(e.to_string()))?;
                ctx.check()?;
                Ok((name, imported))
            },
            move |session, (name, imported)| {
                // Automation never runs script events or ambient color-policy I/O.
                close_replaced(session, previous);
                session.add_document(imported.document, Some(applied_path.clone()));
                Ok(json!({"path": applied_path, "name": name, "warnings": imported.warnings}))
            },
        )
        .map_err(|e| e.to_string())
}

fn close_replaced(session: &mut photocraft_engine::Session, previous: Option<photocraft_doc::DocId>) {
    if let Some(index) = previous.and_then(|id| session.documents().iter().position(|state| state.doc.id == id)) {
        session.close(index);
    }
}

pub fn stash(app: &mut PhotocraftApp, path: &str, preview: &str) -> Result<Started, String> {
    crate::raw_develop_ui::settle(app);
    if app.raw_development.is_some() {
        return Err("Commit or cancel RAW Development before saving".into());
    }
    if !path.to_ascii_lowercase().ends_with(".pcraft") || !preview.to_ascii_lowercase().ends_with(".png") {
        return Err("stash requires a .pcraft project and a .png preview".into());
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
    let preview = preview.to_string();
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
                // The project and its preview are independent encodes of the same snapshot: run
                // them side by side. Both finish before either result is looked at.
                let (project, render) = std::thread::scope(|scope| {
                    let project = scope.spawn(|| photocraft_io::export(&doc, &path, &Default::default()));
                    let render = photocraft_io::export(&doc, &preview, &Default::default());
                    (project.join(), render)
                });
                let project = project.map_err(|_| EngineError::Other("project encoder stopped unexpectedly".into()))?.map_err(|e| EngineError::Other(e.to_string()))?;
                let render = render.map_err(|e| EngineError::Other(e.to_string()))?;
                ctx.check()?;
                // Each capability write is atomic. Report success only after both.
                write(&path, &project.bytes).map_err(EngineError::Other)?;
                write(&preview, &render.bytes).map_err(EngineError::Other)?;
                Ok((project.warnings, render.warnings))
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
        app.jobs.host_target = Some((id, "photo.pcraft".into(), "preview.png".into()));
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
        assert_eq!(app.session.active().unwrap().doc.name, "replacement");
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
