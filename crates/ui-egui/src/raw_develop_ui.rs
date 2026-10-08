//! RAW-source dialog adapter. Pixel Camera Raw previews and PSD descriptors stay independent.
use crate::PhotocraftApp;
use photocraft_doc::{DocId, LayerId, RawDemosaic, RawDevelopment, RawWhiteBalance, SmartSource};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
static PREVIEW_WORKER: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct Dialog {
    committing: Option<photocraft_engine::jobs::JobId>,
    cancelled: Arc<AtomicBool>,
    document: DocId,
    layer: LayerId,
    name: String,
    bytes: Arc<Vec<u8>>,
    pub settings: RawDevelopment,
    original: RawDevelopment,
    preview_settings: Option<RawDevelopment>,
    pending: Option<(RawDevelopment, mpsc::Receiver<Result<egui::ColorImage, String>>)>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
}

pub fn is_raw(app: &PhotocraftApp, params: &Value) -> bool {
    let layer = target_layer(app, params).ok();
    app.session.active().is_some_and(|state| layer.is_some_and(|id| photocraft_engine::raw_develop_cmds::smart(&state.doc, id).is_ok()))
}

fn target_layer(app: &PhotocraftApp, params: &Value) -> Result<LayerId, String> {
    match params.get("layer") {
        Some(value) => value.as_u64().map(LayerId).ok_or_else(|| "invalid layer id".into()),
        None => app.session.active().and_then(|state| state.active_layer).ok_or_else(|| "no active layer".into()),
    }
}

pub fn open(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let state = app.session.active().ok_or("no document")?;
    let layer = target_layer(app, params)?;
    if let Some(d) = &app.raw_development {
        if d.document == state.doc.id && d.layer == layer {
            return Ok(json!({"open":true,"sensorBacked":true}));
        }
        return Err("Commit or cancel RAW Development before editing another source".into());
    }
    let sm = photocraft_engine::raw_develop_cmds::smart(&state.doc, layer).map_err(|e| e.to_string())?;
    let settings = sm.raw_development.clone().ok_or("no RAW settings")?;
    let SmartSource::Embedded { file_name, .. } = &sm.source else { return Err("RAW development requires embedded source bytes".into()) };
    let SmartSource::Embedded { bytes, .. } = sm.raw_decode_source.as_ref().unwrap_or(&sm.source) else {
        return Err("RAW sensor data must be embedded".into());
    };
    app.raw_development = Some(Dialog {
        committing: None,
        cancelled: Arc::new(AtomicBool::new(false)),
        document: state.doc.id,
        layer,
        name: file_name.clone(),
        bytes: bytes.clone(),
        original: settings.clone(),
        settings,
        preview_settings: None,
        pending: None,
        texture: None,
        error: None,
    });
    Ok(json!({"open": true, "sensorBacked": true}))
}

/// Called by the host before stash/switch/close: commit the draft as one background edit.
pub fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    settle(app);
    let Some(d) = &app.raw_development else { return Ok(json!({"open": false})) };
    if let Some(job) = d.committing {
        app.jobs.last_started = Some(job);
        return Ok(json!({"job":job.0,"pending":true}));
    }
    if app.session.active().is_none_or(|state| state.doc.id != d.document) {
        return Err("the RAW document changed; the draft was not applied".into());
    }
    let params = json!({"layer": d.layer.0, "settings": d.settings});
    let result = app.run("layer.rawDevelopment.set", params)?;
    if let Some(job) = result.get("job").and_then(Value::as_u64) {
        if let Some(d) = &mut app.raw_development {
            d.committing = Some(photocraft_engine::jobs::JobId(job));
        }
    } else {
        app.raw_development = None;
    }
    Ok(result)
}

/// Keep a failed/cancelled commit's draft recoverable; close only after it applies.
pub fn settle(app: &mut PhotocraftApp) {
    let Some(job) = app.raw_development.as_ref().and_then(|d| d.committing) else { return };
    let Some(info) = app.session.jobs_with_recent().into_iter().find(|info| info.id == job) else {
        if let Some(d) = &mut app.raw_development {
            d.committing = None;
            d.error = Some("RAW development result is unavailable; settings are retained".into());
        }
        return;
    };
    if info.state == "done" {
        app.raw_development = None;
    } else if info.state != "running"
        && let Some(d) = &mut app.raw_development
    {
        d.committing = None;
        d.error = Some(info.error.clone().unwrap_or_else(|| "RAW development was cancelled; settings are retained".into()));
    }
}

pub fn control(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    settle(app);
    for flag in ["open", "commit", "cancel"] {
        if p.get(flag).is_some_and(|value| !value.is_boolean()) {
            return Err(format!("{flag} must be a boolean"));
        }
    }
    let settings = p.get("settings").map(|value| serde_json::from_value::<RawDevelopment>(value.clone()).map_err(|e| e.to_string())).transpose()?;
    if let Some(settings) = &settings {
        settings.validate()?;
    }
    if app.raw_development.as_ref().is_some_and(|d| d.committing.is_some()) && (settings.is_some() || p.get("cancel").and_then(Value::as_bool) == Some(true)) {
        return Err("RAW development is applying; wait or cancel its job first".into());
    }
    if p.get("open").and_then(Value::as_bool) == Some(true) {
        open(app, p)?;
    }
    if p.get("cancel").and_then(Value::as_bool) == Some(true) {
        app.raw_development = None;
        return Ok(json!({"open": false}));
    }
    if let Some(settings) = settings {
        app.raw_development.as_mut().ok_or("RAW development is not open")?.settings = settings;
    }
    if p.get("commit").and_then(Value::as_bool) == Some(true) {
        return commit(app);
    }
    Ok(match &app.raw_development {
        Some(d) => json!({"open": true, "applying":d.committing.is_some(), "settings": d.settings, "changed": d.settings != d.original, "sensorBacked": true,
            "previewReady": d.error.is_none() && d.texture.is_some() && d.preview_settings.as_ref() == Some(&d.settings), "previewPending": d.pending.is_some(), "error":d.error}),
        None => json!({"open": false}),
    })
}

fn preview(name: &str, bytes: &[u8], settings: &RawDevelopment) -> Result<egui::ColorImage, String> {
    let source = photocraft_io::raw_smart::develop(name, bytes, settings).map_err(|e| e.to_string())?.document;
    let transform =
        photocraft_cms::cached(&photocraft_engine::color_cmds::composite_profile(&source), photocraft_cms::Builtin::Srgb.profile(), Default::default())
            .map_err(|e| e.to_string())?;
    let surface = source.layers.first().and_then(|l| l.surface()).ok_or("no RAW pixels")?;
    let w = source.size.width;
    let h = source.size.height;
    let factor = w.max(h).div_ceil(900).max(1);
    let pw = w.div_ceil(factor);
    let ph = h.div_ceil(factor);
    let mut rgb = Vec::with_capacity((pw * ph) as usize * 4);
    for y in 0..ph {
        for x in 0..pw {
            rgb.extend(surface.rgba((x * factor) as i32, (y * factor) as i32));
        }
    }
    let mut output = vec![0.0; rgb.len()];
    transform.convert_f32(&rgb, 4, &mut output, 4, true);
    Ok(egui::ColorImage::new(
        [pw as usize, ph as usize],
        output
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let enc = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                egui::Color32::from_rgb(enc(p[0]), enc(p[1]), enc(p[2]))
            })
            .collect(),
    ))
}

impl Dialog {
    fn tick(&mut self, ctx: &egui::Context) {
        if let Some((settings, receiver)) = &self.pending {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("RAW preview worker stopped".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                if *settings == self.settings {
                    self.preview_settings = Some(settings.clone());
                    match result {
                        Ok(image) => {
                            self.texture = Some(ctx.load_texture("raw-source-preview", image, egui::TextureOptions::LINEAR));
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                self.pending = None;
            }
        }
        if self.pending.is_none() && self.preview_settings.as_ref() != Some(&self.settings) {
            let (tx, rx) = mpsc::channel();
            let name = self.name.clone();
            let bytes = self.bytes.clone();
            let settings = self.settings.clone();
            let cancelled = self.cancelled.clone();
            self.pending = Some((settings.clone(), rx));
            let work = move || {
                // One full-resolution preview worker at a time, including a recently
                // closed dialog. Obsolete queued work exits without developing pixels.
                let _guard = PREVIEW_WORKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if cancelled.load(Ordering::Relaxed) {
                    return;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preview(&name, &bytes, &settings)))
                    .unwrap_or_else(|_| Err("RAW preview failed".into()));
                if !cancelled.load(Ordering::Relaxed) {
                    let _ = tx.send(result);
                }
            };
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(error) = std::thread::Builder::new().name("raw-preview".into()).spawn(work) {
                self.error = Some(error.to_string());
                self.pending = None;
                self.preview_settings = Some(self.settings.clone());
            }
            #[cfg(target_arch = "wasm32")]
            work();
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    settle(app);
    let Some(d) = &mut app.raw_development else { return };
    if d.committing.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(40));
        return;
    }
    if app.session.active().is_none_or(|state| state.doc.id != d.document) {
        app.raw_development = None;
        return;
    }
    d.tick(ctx);
    let mut apply = false;
    let mut cancel = false;
    let response = egui::Modal::new(egui::Id::new("raw-development")).show(ctx, |ui| {
        ui.set_min_size(egui::vec2(900.0, 550.0));
        ui.heading(format!("RAW Development — {}", d.name));
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                if let Some(texture) = &d.texture {
                    let size = texture.size_vec2();
                    ui.add(egui::Image::new(texture).fit_to_exact_size(size * (600.0 / size.x.max(1.0)).min(500.0 / size.y.max(1.0)).min(1.0)));
                }
                if d.pending.is_some() {
                    ui.spinner();
                    ui.label("Developing from sensor data…");
                }
                if let Some(error) = &d.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
            ui.vertical(|ui| {
                ui.label("Original RAW retained in this smart object");
                ui.add(egui::Slider::new(&mut d.settings.exposure, -10.0..=10.0).text("Exposure (EV)"));
                egui::ComboBox::from_label("White balance")
                    .selected_text(match d.settings.white_balance {
                        RawWhiteBalance::AsShot => "As shot",
                        RawWhiteBalance::Auto => "Auto",
                        RawWhiteBalance::Multipliers { .. } => "Camera multipliers",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.settings.white_balance, RawWhiteBalance::AsShot, "As shot");
                        ui.selectable_value(&mut d.settings.white_balance, RawWhiteBalance::Auto, "Auto");
                        if ui.selectable_label(matches!(d.settings.white_balance, RawWhiteBalance::Multipliers { .. }), "Camera multipliers").clicked() {
                            d.settings.white_balance = RawWhiteBalance::Multipliers { rgb: [1.0; 3] };
                        }
                    });
                if let RawWhiteBalance::Multipliers { rgb } = &mut d.settings.white_balance {
                    for (value, label) in rgb.iter_mut().zip(["Red", "Green", "Blue"]) {
                        ui.add(egui::DragValue::new(value).range(0.01..=100.0).speed(0.05).prefix(format!("{label} ")));
                    }
                }
                egui::ComboBox::from_label("Demosaic").selected_text(format!("{:?}", d.settings.demosaic)).show_ui(ui, |ui| {
                    for (mode, label) in [(RawDemosaic::Ahd, "AHD"), (RawDemosaic::Mhc, "MHC"), (RawDemosaic::Bilinear, "Bilinear")] {
                        ui.selectable_value(&mut d.settings.demosaic, mode, label);
                    }
                });
                if ui.button("Reset").clicked() {
                    d.settings = RawDevelopment::default();
                }
                ui.separator();
                ui.label("Camera Raw Filter remains available for later pixel adjustments.");
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    apply = ui.button("OK").clicked();
                });
            });
        });
    });
    if cancel || response.should_close() {
        app.raw_development = None;
    } else if apply && let Err(error) = commit(app) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};

    fn app() -> PhotocraftApp {
        let bytes = DngSpec::cfa(32, 24, mosaic(&scene(32, 24), 32, [0, 1, 1, 2], 0, 65535)).build();
        let mut session = photocraft_engine::Session::new();
        session.add_document(photocraft_io::raw_smart::import("camera.dng", &bytes).unwrap().document, None);
        PhotocraftApp::new(session, Default::default())
    }

    #[test]
    fn scoped_open_applies_combined_settings_before_commit_and_rejects_linked_sources() {
        let mut app = app();
        app.services.automation_command = Some(Box::new(|_, _| Err("ambient filesystem denied".into())));
        let settings = RawDevelopment { exposure: -1.0, ..Default::default() };
        control(&mut app, &json!({"open":true,"settings":settings,"commit":true})).unwrap();
        assert!(app.raw_development.is_none());
        let state = app.session.active().unwrap();
        let photocraft_doc::LayerContent::Smart(sm) = &state.doc.layers[0].content else { panic!("not smart") };
        assert_eq!(sm.raw_development.as_ref(), Some(&settings));
        app.session
            .edit("link", |doc, _| {
                let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layers[0].content else { panic!("not smart") };
                sm.source = SmartSource::Linked { path: "outside.NEF".into() };
                Ok(())
            })
            .unwrap();
        assert!(control(&mut app, &json!({"open":true})).is_err());
        assert!(app.raw_development.is_none());
    }

    #[test]
    fn raw_contents_open_a_draft_and_cancel_keeps_document_exactly() {
        let mut app = app();
        let original = app.session.active().unwrap().doc.clone();
        crate::menus::invoke(&mut app, &egui::Context::default(), "layer.smartObjects.editContents", json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 1);
        assert_eq!(control(&mut app, &json!({})).unwrap()["open"], true);
        let settings = RawDevelopment { exposure: -2.0, ..Default::default() };
        control(&mut app, &json!({"settings": settings})).unwrap();
        control(&mut app, &json!({"cancel": true})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *original);
    }

    #[test]
    fn commit_is_one_edit_and_stale_draft_never_changes_another_photo() {
        let mut app = app();
        open(&mut app, &json!({})).unwrap();
        let before = app.session.active().unwrap().revision;
        let settings = RawDevelopment { exposure: -1.0, ..Default::default() };
        control(&mut app, &json!({"settings": settings})).unwrap();
        commit(&mut app).unwrap();
        assert!(app.raw_development.is_none());
        assert_eq!(app.session.active().unwrap().revision, before + 1);
        open(&mut app, &json!({})).unwrap();
        app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        let untouched = app.session.active().unwrap().doc.clone();
        assert!(commit(&mut app).is_err());
        assert_eq!(*app.session.active().unwrap().doc, *untouched);
    }

    #[test]
    fn failed_background_commit_keeps_draft_recoverable() {
        let mut app = app();
        open(&mut app, &json!({})).unwrap();
        let settings = RawDevelopment { exposure: -1.0, ..Default::default() };
        control(&mut app, &json!({"settings":settings})).unwrap();
        // Simulate damaged source data discovered by development, after opening the draft.
        app.session
            .edit("damage", |doc, _| {
                let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layers[0].content else { panic!("not smart") };
                sm.source = SmartSource::Embedded { file_name: "bad.dng".into(), bytes: Arc::new(vec![0]) };
                Ok(())
            })
            .unwrap();
        app.background_jobs = true;
        let result = commit(&mut app).unwrap();
        let job = photocraft_engine::jobs::JobId(result["job"].as_u64().unwrap());
        assert!(app.session.wait_job(job).is_err());
        settle(&mut app);
        assert_eq!(app.raw_development.as_ref().unwrap().settings, settings);
        assert!(app.raw_development.as_ref().unwrap().committing.is_none());
        assert!(app.raw_development.as_ref().unwrap().error.is_some());
    }

    #[test]
    fn completed_background_commit_closes_the_draft_and_unlocks_stash() {
        let mut app = app();
        open(&mut app, &json!({})).unwrap();
        let before = app.session.active().unwrap().revision;
        let settings = RawDevelopment { exposure: -1.0, ..Default::default() };
        control(&mut app, &json!({"settings":settings})).unwrap();
        app.background_jobs = true;
        let result = commit(&mut app).unwrap();
        let job = photocraft_engine::jobs::JobId(result["job"].as_u64().unwrap());
        assert!(app.raw_development.as_ref().unwrap().committing.is_some());
        assert_eq!(control(&mut app, &json!({})).unwrap()["applying"], true);
        app.session.wait_job(job).unwrap();
        settle(&mut app);
        assert!(app.raw_development.is_none());
        assert_eq!(app.session.active().unwrap().revision, before + 1);
        assert_eq!(control(&mut app, &json!({})).unwrap()["open"], false);
    }

    #[test]
    fn raw_import_and_stash_use_scoped_io_and_preserve_source_settings() {
        use photocraft_engine::jobs::Started;
        let mut app = app();
        let source = app.session.active().unwrap().doc.clone();
        let photocraft_doc::LayerContent::Smart(sm) = &source.layers[0].content else { panic!("not smart") };
        let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("not embedded") };
        let bytes = bytes.clone();
        app.services.background_automation_read = Some(Arc::new(move |_| Ok(("next.dng".into(), (*bytes).clone()))));
        let started = crate::control_files::open_with_options(&mut app, "next.dng", true, true).unwrap();
        if let Started::Job(job) = started {
            app.session.wait_job(job).unwrap();
        }
        open(&mut app, &json!({})).unwrap();
        let settings = RawDevelopment { exposure: -1.0, ..Default::default() };
        control(&mut app, &json!({"settings":settings})).unwrap();
        commit(&mut app).unwrap();
        let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = writes.clone();
        app.services.background_automation_write = Some(Arc::new(move |path, bytes| {
            captured.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        let started = crate::control_files::stash(&mut app, "next.dng.pcraft", "next.dng.photocraft.png").unwrap();
        if let Started::Job(job) = started {
            app.session.wait_job(job).unwrap();
        }
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 2);
        let doc = photocraft_format::load_from_bytes(&writes[0].1).unwrap();
        let photocraft_doc::LayerContent::Smart(saved) = &doc.layers[0].content else { panic!("not smart") };
        assert_eq!(saved.raw_development.as_ref(), Some(&settings));
        let SmartSource::Embedded { file_name, bytes } = &saved.source else { panic!("not embedded") };
        let SmartSource::Embedded { bytes: original, .. } = &sm.source else { panic!("not embedded") };
        assert_eq!(file_name, "next.dng");
        assert!(bytes == original);
    }
    #[test]
    fn sensor_adapter_reads_both_files_through_the_scoped_capability() {
        use photocraft_engine::jobs::Started;
        let mut app = app();
        let state = app.session.active().unwrap();
        let photocraft_doc::LayerContent::Smart(sm) = &state.doc.layers[0].content else { panic!("not smart") };
        let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("not embedded") };
        let sensor = bytes.clone();
        app.services.background_automation_read = Some(Arc::new(move |path| match path {
            "camera.NEF" => Ok(("camera.NEF".into(), b"original camera data".to_vec())),
            "sensor.dng" => Ok(("sensor.dng".into(), (*sensor).clone())),
            _ => Err("outside read grant".into()),
        }));
        let started = crate::control_files::open_with_sensor(&mut app, "camera.NEF", true, true, Some("sensor.dng")).unwrap();
        if let Started::Job(job) = started {
            app.session.wait_job(job).unwrap();
        }
        assert_eq!(app.session.documents().len(), 1);
        open(&mut app, &json!({})).unwrap();
        assert!(!app.raw_development.as_ref().unwrap().bytes.is_empty());
        control(&mut app, &json!({"cancel":true})).unwrap();
        let previous = app.session.active().unwrap().doc.id;
        let started = crate::control_files::open_with_sensor(&mut app, "camera.NEF", true, true, Some("../outside")).unwrap();
        if let Started::Job(job) = started {
            assert!(app.session.wait_job(job).is_err());
        }
        assert_eq!(app.session.active().unwrap().doc.id, previous);
        assert!(crate::control_files::open_with_sensor(&mut app, "camera.NEF", true, false, Some("sensor.dng")).is_err());
    }
}
