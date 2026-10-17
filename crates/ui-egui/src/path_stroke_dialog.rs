//! Post-pending-commit path stroke dialog, preserving existing source selection.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map, Value, json};
const COMMAND: &str = "path.stroke";
const DISPLAY_SPEC: &str = r#"{"tool":"brush|pencil|eraser"="brush","size":1..500=10,"color":text,"opacity":0..100=100}"#;
/// Runtime UI witness cap only; this is not an image/engine format limit.
const BRUSH_WITNESS_BYTES: u64 = 8 << 20;
#[derive(Clone, Debug)]
pub(crate) struct PathStrokeOrigin {
    dialog: u64,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    name: String,
    path: Option<photocraft_doc::item_identity::PathId>,
    layer: Option<photocraft_doc::LayerId>,
    brush: std::sync::Arc<photocraft_paint::BrushSettings>,
    background: [f32; 4],
}
impl PartialEq for PathStrokeOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.dialog == other.dialog
            && self.document == other.document
            && self.revision == other.revision
            && self.name == other.name
            && self.path == other.path
            && self.layer == other.layer
            && self.brush == other.brush
            && self.background == other.background
            && match (self.snapshot.upgrade(), other.snapshot.upgrade()) {
                (Some(a), Some(b)) => a.shares_state(&b),
                (None, None) => true,
                _ => false,
            }
    }
}
fn work_alias(name: &str) -> bool {
    name.is_empty() || name.eq_ignore_ascii_case("work") || name == "Work Path"
}
fn invalid() -> String {
    tl!("Check the path stroke settings and try again.").into()
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or(tl!("Open a document before stroking a path."))?;
    if active.doc.id.0 == 0
        || app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || app.ui.pen.as_ref().is_some_and(|pen| pen.knots.len() >= 2)
    {
        return Err(tl!("Finish pending edits before stroking a path.").into());
    }
    Ok(())
}
fn identifier(value: &Value) -> Result<u64, String> {
    if let Some(id) = value.as_u64().filter(|id| *id != 0) {
        return Ok(id);
    }
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && !s.starts_with('0') && s.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
        .filter(|id| *id != 0)
        .ok_or_else(invalid)
}
/// Caller finishes its existing pending-pen commit FIRST. This function never
/// commits paths, parses path geometry or changes selection/source metadata.
pub(crate) fn open(app: &mut PhotocraftApp, initial: &Value) -> Option<u64> {
    let prepared = (|| {
        idle(app)?;
        let values = initial
            .as_object()
            .filter(|values| values.len() <= 3 && values.keys().all(|key| matches!(key.as_str(), "pathId" | "name" | "layer")))
            .ok_or_else(invalid)?;
        let active = app.session.active().ok_or_else(invalid)?;
        let doc = &active.doc;
        if doc.paths.len() > 10000 {
            return Err(invalid());
        }
        let name = match values.get("name") {
            None => "work",
            Some(value) => value.as_str().filter(|name| name.len() <= 4096 && !name.contains('\0')).ok_or_else(invalid)?,
        };
        let explicit = values.get("pathId").map(identifier).transpose()?.map(photocraft_doc::item_identity::PathId);
        // Match the existing engine's first-name-match behavior, then freeze its
        // actual ID. Labels never resolve a different object at confirmation.
        let path = if let Some(id) = explicit {
            if !doc.paths.iter().any(|path| path.id == id) {
                return Err(tl!("The selected saved path is unavailable.").into());
            }
            Some(id)
        } else if work_alias(name) || name == "layer" {
            None
        } else {
            Some(
                doc.paths
                    .iter()
                    .find(|path| path.name == name)
                    .map(|path| path.id)
                    .filter(|id| id.0 != 0)
                    .ok_or(tl!("The selected saved path is unavailable."))?,
            )
        };
        let explicit_layer = values.get("layer").map(identifier).transpose()?.map(photocraft_doc::LayerId);
        if explicit_layer.is_some() && explicit_layer != active.active_layer {
            return Err(tl!("The selected layer changed. Reopen Stroke Path.").into());
        }
        let layer = Some(active.active_layer.filter(|id| id.0 != 0).ok_or(tl!("Select a pixel layer to stroke first."))?);
        let label = photocraft_engine::commands::find(COMMAND).map(|spec| spec.label).ok_or(tl!("Stroke Path is unavailable."))?;
        let brush =
            photocraft_engine::vector_cmds::path_stroke_brush_snapshot(&app.session.tools.brush, BRUSH_WITNESS_BYTES, &photocraft_raster::Interrupt::NONE)
                .map_err(|_| tl!("The current brush cannot be retained within the Stroke Path dialog budget. Choose a smaller valid brush."))?;
        Ok::<_, String>((
            PathStrokeOrigin {
                dialog: 0,
                document: doc.id,
                revision: active.revision,
                snapshot: doc.downgrade(),
                name: name.into(),
                path,
                layer,
                brush: std::sync::Arc::new(brush),
                background: app.session.tools.background,
            },
            label,
        ))
    })();
    let (mut target, label) = match prepared {
        Ok(value) => value,
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            return None;
        }
    };
    let mut fixed = Map::new();
    fixed.insert("__pathStrokeOrigin".into(), json!(true));
    fixed.insert("name".into(), json!(target.name));
    if let Some(id) = target.path {
        fixed.insert("pathId".into(), json!(id.0.to_string()));
    }
    // Identical foreground serialization to the former canvas pen_dialog.
    let foreground = app.session.tools.foreground;
    let byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    fixed.insert("color".into(), json!(format!("#{:02x}{:02x}{:02x}", byte(foreground[0]), byte(foreground[1]), byte(foreground[2]))));
    let id = crate::filter_dialog::open_with_spec(app, COMMAND, label, DISPLAY_SPEC, fixed);
    if let Some(dialog) = app.ui.dialog_mut(id) {
        dialog.fields.remove("__preview");
    }
    target.dialog = id;
    app.ui.path_stroke_origin = Some(target);
    Some(id)
}
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(COMMAND) || fields.get("__pathStrokeOrigin").and_then(Value::as_bool) == Some(true)
}
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    crate::filter_dialog::body(ui, fields);
}
pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let target = app.ui.path_stroke_origin.take().ok_or(tl!("The document changed. Reopen Stroke Path."))?;
    idle(app)?;
    let active = app.session.active().ok_or(tl!("The document changed. Reopen Stroke Path."))?;
    let snapshot = target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen Stroke Path."))?;
    if target.dialog != id
        || fields.get("__command").and_then(Value::as_str) != Some(COMMAND)
        || active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
        || target.layer.is_some_and(|layer| active.active_layer != Some(layer))
    {
        return Err(tl!("The document or selected path changed. Reopen Stroke Path.").into());
    }
    // Bounded captured samples/settings are compared only on explicit Apply.
    // No per-frame hash/layout or hidden brush rewrite is needed.
    if *target.brush != app.session.tools.brush || target.background.iter().zip(app.session.tools.background).any(|(old, new)| old.to_bits() != new.to_bits()) {
        return Err(tl!("The brush settings changed. Reopen Stroke Path.").into());
    }
    if fields.len() > 16
        || fields.keys().any(|key| {
            !key.starts_with("__")
                && !matches!(key.as_str(), "name" | "pathId" | "layer" | "tool" | "size" | "color" | "opacity" | "documentId" | "expectedRevision")
        })
    {
        return Err(invalid());
    }
    let color = fields
        .get("color")
        .and_then(Value::as_str)
        .filter(|color| {
            (if color.starts_with('#') { matches!(color.len(), 7 | 9) } else { matches!(color.len(), 6 | 8) })
                && color.bytes().skip(usize::from(color.starts_with('#'))).all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or(tl!("Enter a valid hex color."))?;
    let opacity = fields.get("opacity").and_then(Value::as_f64).filter(|value| value.is_finite()).ok_or_else(invalid)?;
    let size = fields.get("size").and_then(Value::as_f64).filter(|value| value.is_finite() && (*value as f32).is_finite()).ok_or_else(invalid)?;
    let tool = fields.get("tool").and_then(Value::as_str).filter(|tool| matches!(*tool, "brush" | "pencil" | "eraser")).ok_or_else(invalid)?;
    let mut params = Map::new();
    params.insert("name".into(), json!(target.name));
    if let Some(path) = target.path {
        params.insert("pathId".into(), json!(path.0.to_string()));
    }
    let layer = target.layer.ok_or(tl!("Select a pixel layer to stroke first."))?;
    params.insert("layer".into(), json!(layer.0));
    params.insert("color".into(), json!(color));
    params.insert("opacity".into(), json!(opacity));
    params.insert("size".into(), json!(size));
    params.insert("tool".into(), json!(tool));
    params.insert("documentId".into(), json!(target.document.0.to_string()));
    params.insert("expectedRevision".into(), json!(target.revision.to_string()));
    app.run(COMMAND, Value::Object(params))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":32,"height":32})).unwrap();
        app
    }
    fn path(app: &mut PhotocraftApp, name: &str) {
        app.session.execute("path.set",json!({"name":name,"path":{"subpaths":[{"closed":true,"knots":[{"anchor":[2,2],"in":[2,2],"out":[2,2]},{"anchor":[20,2],"in":[20,2],"out":[20,2]},{"anchor":[20,20],"in":[20,20],"out":[20,20]}]}]}})).unwrap();
    }
    #[test]
    fn original_stroke_defaults_foreground_and_actual_saved_target_are_retained() {
        let mut app = app();
        path(&mut app, "Own Saved");
        app.session.tools.foreground = [0.25, 0.5, 0.75, 1.];
        let before = app.session.active().unwrap().doc.clone();
        let id = open(&mut app, &json!({"name":"Own Saved"})).unwrap();
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["tool"], "brush");
        assert_eq!(fields["size"], 10.);
        assert_eq!(fields["color"], "#4080bf");
        assert_eq!(fields["opacity"], 100.);
        assert!(!fields.contains_key("hardness"));
        assert!(!fields.contains_key("__preview"));
        confirm(&mut app, id, &fields).unwrap();
        assert_eq!(app.session.active().unwrap().doc.paths, before.paths);
        assert_ne!(app.session.active().unwrap().doc.layers, before.layers);
        app.session.undo();
        assert_eq!(&*app.session.active().unwrap().doc, &*before);
    }
    #[test]
    fn stale_source_tab_edit_and_restored_origin_never_stroke_pixels() {
        for change in 0..3 {
            let mut app = app();
            path(&mut app, "work");
            let id = open(&mut app, &json!({"name":"work"})).unwrap();
            let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
            match change {
                0 => {
                    app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
                }
                1 => {
                    app.session.execute("image.canvasSize", json!({"width":33,"height":32})).unwrap();
                }
                _ => {
                    app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
                    assert!(app.ui.path_stroke_origin.is_none());
                }
            }
            let before = app.session.active().unwrap().doc.clone();
            assert!(confirm(&mut app, id, &fields).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
        }
    }
    #[test]
    fn hidden_brush_change_and_pending_path_refuse_without_rewriting_source() {
        let mut app = app();
        path(&mut app, "work");
        let id = open(&mut app, &json!({"name":"work"})).unwrap();
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        let before = app.session.active().unwrap().doc.clone();
        app.session.tools.brush.hardness = 0.25;
        assert_eq!(confirm(&mut app, id, &fields).unwrap_err(), tl!("The brush settings changed. Reopen Stroke Path."));
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        app.ui.tool_options.vector_mode = "path".into();
        crate::vector_ui::pen_down(&mut app, 2., 2., false);
        crate::vector_ui::pen_up(&mut app);
        crate::vector_ui::pen_down(&mut app, 20., 2., false);
        crate::vector_ui::pen_up(&mut app);
        assert!(open(&mut app, &json!({"name":"work"})).is_none());
        assert!(app.ui.pen.is_some());
        crate::vector_ui::pen_commit(&mut app, false);
        assert!(open(&mut app, &json!({"name":"work"})).is_some());
    }
}
