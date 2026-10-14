//! `app.render`: a photo's edits at full size, for a host that exports the photo or hands it on.
//!
//! Renders in the background from what the host names and never from the open session, which it leaves
//! alone: a saved project (the editor's in-memory snapshot of it while a save of it is in flight or failed,
//! else the file), or a Camera Raw recipe developed from the original raw exactly as "Open" develops it
//! ([`crate::raw_recipe::tuning`], then [`crate::raw_recipe::after_develop`] as one Camera Raw step). The
//! picture keeps the document's depth: 8- and 16-bit documents as PNG, 32-bit float ones as TIFF. The reply
//! describes the file that was written, read back from its bytes.

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_engine::{EngineError, jobs::Started};
use serde_json::{Value, json};

use crate::PhotocraftApp;

enum Source {
    Project { path: String, snapshot: Option<std::sync::Arc<photocraft_doc::Document>> },
    Recipe { source: String, params: Box<CameraRaw> },
}

/// Starts rendering what `p` names (`{out, project, snapshot?}` or `{out, source, recipe}`); `out` is the written file's path
/// under the write root without its extension. Unreadable recipes and missing parameters fail here, before any job starts.
pub fn start(app: &mut PhotocraftApp, p: &Value) -> Result<Started, String> {
    let text = |name: &str| p.get(name).and_then(Value::as_str).filter(|s| !s.is_empty());
    let out = text("out").ok_or("render requires `out`")?.to_string();
    let read = app.services.background_automation_read.clone().ok_or("background automation read authority is not configured")?;
    let write = app.services.background_automation_write.clone().ok_or("background automation write authority is not configured")?;
    let source = match (text("project"), p.get("recipe")) {
        (Some(project), None) => {
            if !project.to_ascii_lowercase().ends_with(".pcraft") {
                return Err("render `project` must be a .pcraft project".into());
            }
            let snapshot = text("snapshot").and_then(|key| app.jobs.host_snapshots.get(key)).map(|(doc, _)| doc.clone());
            Source::Project { path: project.to_string(), snapshot }
        }
        (None, Some(recipe)) => {
            let params = crate::raw_recipe::from_json(recipe)?;
            let source = text("source").ok_or("a recipe render requires the photo's `source`")?.to_string();
            Source::Recipe { source, params: Box::new(params) }
        }
        _ => return Err("render takes either a `project` or a `recipe`".into()),
    };
    app.session
        .start_job(
            "app.render",
            json!({"out": out}),
            "Rendering photo edits",
            false,
            move |ctx| {
                ctx.check()?;
                let doc = match source {
                    Source::Project { snapshot: Some(doc), .. } => (*doc).clone(),
                    Source::Project { path, snapshot: None } => {
                        let (name, bytes) = read(&path).map_err(EngineError::Other)?;
                        ctx.check()?;
                        photocraft_io::import(&name, &bytes).map_err(|e| EngineError::Other(e.to_string()))?.document
                    }
                    Source::Recipe { source, params } => develop(&read, &source, &params, ctx)?,
                };
                ctx.check()?;
                let float = doc.depth == photocraft_color::SampleType::F32;
                let target = format!("{out}.{}", if float { "tif" } else { "png" });
                let encoded = photocraft_io::export(&doc, &target, &Default::default()).map_err(|e| EngineError::Other(e.to_string()))?;
                ctx.check()?;
                write(&target, &encoded.bytes).map_err(EngineError::Other)?;
                let mut reply = describe_any(&encoded.bytes).ok_or_else(|| EngineError::Other(format!("{target}: the render is not a picture")))?;
                reply["out"] = json!(target);
                Ok(reply)
            },
            |_session, reply| Ok(reply),
        )
        .map_err(|e| e.to_string())
}

/// The original raw developed with `params` as "Open" develops it.
fn develop(
    read: &crate::BackgroundAutomationReadFn,
    source: &str,
    params: &CameraRaw,
    ctx: &photocraft_engine::jobs::JobCtx,
) -> photocraft_engine::Result<photocraft_doc::Document> {
    let (name, bytes) = read(source).map_err(EngineError::Other)?;
    if !photocraft_io::raw::is_raw(&bytes) {
        return Err(EngineError::Other(format!("{source}: a Camera Raw recipe applies to raw photos only")));
    }
    ctx.check()?;
    let (imported, white_balanced) =
        photocraft_io::raw::import_raw_tuned(&name, &bytes, &crate::raw_recipe::tuning(params)).map_err(|e| EngineError::Other(e.to_string()))?;
    ctx.check()?;
    let rest = crate::raw_recipe::after_develop(params, white_balanced);
    if rest.is_identity() {
        return Ok(imported.document);
    }
    // The same Camera Raw step Open applies, on a session of its own (the editor's is never touched).
    let mut session = photocraft_engine::Session::new();
    session.add_document(imported.document, None);
    let layer = session.active().and_then(|state| state.active_layer).ok_or_else(|| EngineError::Other("no layer to develop".into()))?;
    let mut step = crate::camera_raw_ui::non_defaults(&rest);
    step["layer"] = json!(layer.0);
    session.execute("filter.cameraRaw", step)?;
    session.active().map(|state| photocraft_doc::Document::clone(&state.doc)).ok_or_else(|| EngineError::Other("the developed document went away".into()))
}

/// What an encoded picture holds: [`crate::render_manifest::describe`] for PNG and JPEG, and a TIFF's first image.
fn describe_any(bytes: &[u8]) -> Option<Value> {
    crate::render_manifest::describe(bytes).or_else(|| describe_tiff(bytes))
}

fn describe_tiff(bytes: &[u8]) -> Option<Value> {
    let little = match bytes.get(0..4)? {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return None,
    };
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) });
    let u32_at = |at: usize| {
        bytes.get(at..at + 4).map(|b| if little { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    };
    let ifd = u32_at(4)? as usize;
    let (mut width, mut height, mut depth, mut icc) = (None, None, None, false);
    for entry in 0..u16_at(ifd)? as usize {
        let at = ifd + 2 + entry * 12;
        let (tag, kind, count) = (u16_at(at)?, u16_at(at + 2)?, u32_at(at + 4)?);
        // A SHORT or LONG value in the entry itself (the first of several, for BitsPerSample, is stored elsewhere).
        let value = |at: usize| if kind == 3 { u16_at(at).map(u32::from) } else { u32_at(at) };
        match tag {
            256 => width = value(at + 8),
            257 => height = value(at + 8),
            258 if count <= 2 => depth = value(at + 8),
            258 => depth = u16_at(u32_at(at + 8)? as usize).map(u32::from),
            34675 => icc = true,
            _ => {}
        }
    }
    Some(json!({"format": "tiff", "width": width?, "height": height?, "depth": depth?, "icc": icc}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;
    use std::sync::{Arc, Mutex};

    type Writes = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    fn app(files: Vec<(&'static str, Vec<u8>)>) -> (PhotocraftApp, Writes) {
        let writes: Writes = Default::default();
        let recorded = writes.clone();
        let files = Arc::new(files);
        let services = crate::Services {
            background_automation_read: Some(Arc::new(move |path: &str| {
                files.iter().find(|(name, _)| *name == path).map(|(name, bytes)| (name.to_string(), bytes.clone())).ok_or_else(|| format!("no file {path}"))
            })),
            background_automation_write: Some(Arc::new(move |path: &str, bytes: &[u8]| {
                recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
                Ok(())
            })),
            import: Some(Box::new(|name: &str, b: &[u8], depth: usize| {
                photocraft_io::import_with_svg_group_depth(name, b, depth).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        (PhotocraftApp::new(photocraft_engine::Session::new(), services), writes)
    }

    fn finish(app: &mut PhotocraftApp, started: Started) -> Result<Value, String> {
        match started {
            Started::Done(value) => Ok(value),
            Started::Job(job) => app.session.wait_job(job).map_err(|e| e.to_string()),
        }
    }

    fn document(depth: SampleType, value: f32) -> Document {
        let mut doc = Document::new("p", Size::new(20, 10), ColorMode::Rgb, depth);
        let mut surface = Surface::new(doc.pixel_format());
        surface.fill_rect(Rect::new(0, 0, 20, 10), &[value, value, value, 1.0]);
        doc.layers.push(Layer::new("bg", LayerContent::Raster(surface)));
        doc
    }

    fn synthetic_raw() -> Vec<u8> {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        let (w, h) = (48, 32);
        let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
        spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
        spec.build()
    }

    fn written(writes: &Writes, path: &str) -> Vec<u8> {
        writes.lock().unwrap().iter().find(|(name, _)| name == path).map(|(_, bytes)| bytes.clone()).expect(path)
    }

    #[test]
    fn a_saved_project_renders_at_its_size_and_depth_without_touching_the_session() {
        let project = photocraft_io::export(&document(SampleType::U16, 0.4), "p.pcraft", &Default::default()).unwrap().bytes;
        let (mut app, writes) = app(vec![("p.pcraft", project)]);
        let open = document(SampleType::U8, 0.9);
        app.session.add_document(open, None);
        let before = (app.session.documents().len(), app.session.active().unwrap().doc.id, app.session.active().unwrap().revision);
        let started = start(&mut app, &json!({"out": "p.render", "project": "p.pcraft"})).unwrap();
        let reply = finish(&mut app, started).unwrap();
        assert_eq!(reply["out"], "p.render.png");
        assert_eq!(
            (reply["format"].as_str(), reply["width"].as_u64(), reply["height"].as_u64(), reply["depth"].as_u64()),
            (Some("png"), Some(20), Some(10), Some(16)),
            "{reply}"
        );
        assert!(written(&writes, "p.render.png").starts_with(b"\x89PNG"));
        let after = (app.session.documents().len(), app.session.active().unwrap().doc.id, app.session.active().unwrap().revision);
        assert_eq!(before, after, "the open session is left alone");
    }

    #[test]
    fn a_project_still_being_saved_renders_from_the_editor_s_snapshot() {
        let (mut app, writes) = app(vec![]);
        app.jobs.host_snapshots.insert("p.pcraft".into(), (Arc::new(document(SampleType::U8, 0.2)), 3));
        let started = start(&mut app, &json!({"out": "p.render", "project": "p.pcraft", "snapshot": "p.pcraft"})).unwrap();
        let reply = finish(&mut app, started).unwrap();
        assert_eq!(reply["depth"], 8);
        assert!(!written(&writes, "p.render.png").is_empty());
    }

    #[test]
    fn a_float_document_renders_as_a_float_tiff() {
        let project = photocraft_io::export(&document(SampleType::F32, 0.5), "f.pcraft", &Default::default()).unwrap().bytes;
        let (mut app, writes) = app(vec![("f.pcraft", project)]);
        let started = start(&mut app, &json!({"out": "f.render", "project": "f.pcraft"})).unwrap();
        let reply = finish(&mut app, started).unwrap();
        assert_eq!((reply["out"].as_str(), reply["format"].as_str(), reply["depth"].as_u64()), (Some("f.render.tif"), Some("tiff"), Some(32)), "{reply}");
        assert!(!written(&writes, "f.render.tif").is_empty());
    }

    #[test]
    fn a_recipe_renders_exactly_what_open_develops() {
        let raw = synthetic_raw();
        let params = CameraRaw { exposure: 0.7, shadows: 40.0, contrast: 25.0, temperature: 12.0, ..Default::default() };
        let (mut app, writes) = app(vec![("IMG.dng", raw.clone())]);
        let recipe = crate::raw_recipe::to_json(&params);
        let started = start(&mut app, &json!({"out": "IMG.render", "source": "IMG.dng", "recipe": recipe})).unwrap();
        finish(&mut app, started).unwrap();
        let rendered = photocraft_codecs::decode(&written(&writes, "IMG.render.png")).unwrap();
        // Open in the dialog, with the same settings.
        app.open_bytes("IMG.dng", &raw).unwrap();
        let ctx = egui::Context::default();
        let pending = app.pending_raw_open.take().unwrap();
        crate::camera_raw_ui::open_raw(&mut app, &ctx, pending).unwrap();
        app.camera_raw.as_mut().unwrap().params = params.clone();
        crate::camera_raw_ui::commit(&mut app).unwrap();
        let opened = app.session.active().unwrap().doc.clone();
        let exported = photocraft_codecs::decode(&photocraft_io::export(&opened, "o.png", &Default::default()).unwrap().bytes).unwrap();
        assert_eq!(rendered.dimensions(), exported.dimensions());
        assert_eq!(rendered.data(), exported.data(), "the render is Open's development, pixel for pixel");
    }

    #[test]
    fn a_recipe_that_cannot_be_read_or_a_photo_that_is_not_raw_fails() {
        let (mut app, _writes) = app(vec![("flat.png", photocraft_io::export(&document(SampleType::U8, 0.5), "flat.png", &Default::default()).unwrap().bytes)]);
        assert!(start(&mut app, &json!({"out": "x", "source": "IMG.dng", "recipe": {"version": 9, "cameraRaw": {}}})).unwrap_err().contains("version"));
        assert!(start(&mut app, &json!({"out": "x", "source": "IMG.dng", "recipe": {"version": 1, "cameraRaw": {"exposure": "bright"}}})).is_err());
        assert!(start(&mut app, &json!({"out": "x", "project": "p.psd"})).is_err());
        assert!(start(&mut app, &json!({"project": "p.pcraft"})).unwrap_err().contains("out"));
        let started = start(&mut app, &json!({"out": "x", "source": "flat.png", "recipe": crate::raw_recipe::to_json(&CameraRaw::default())})).unwrap();
        assert!(finish(&mut app, started).unwrap_err().contains("raw photos only"));
        // Values are clamped where they are applied, except what the develop itself refuses: a failure, never the original.
        let (mut app, _writes) = self::app(vec![("IMG.dng", synthetic_raw())]);
        let started = start(&mut app, &json!({"out": "x", "source": "IMG.dng", "recipe": {"version": 1, "cameraRaw": {"exposure": 99}}})).unwrap();
        assert!(finish(&mut app, started).unwrap_err().contains("out of range"));
    }
}
