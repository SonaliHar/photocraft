//! Post-pending-commit path fill dialog, preserving existing source selection.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map, Value, json};
const COMMAND: &str = "path.fill";
const DISPLAY_SPEC: &str = r#"{"color":text,"opacity":0..100=100,"feather":0..250=0,"antiAlias":bool=true}"#;
#[derive(Clone, Debug)]
pub(crate) struct PathFillOrigin {
    dialog: u64,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    name: String,
    path: Option<photocraft_doc::item_identity::PathId>,
    layer: Option<photocraft_doc::LayerId>,
}
impl PartialEq for PathFillOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.dialog == other.dialog
            && self.document == other.document
            && self.revision == other.revision
            && self.name == other.name
            && self.path == other.path
            && self.layer == other.layer
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
    tl!("Check the path fill settings and try again.").into()
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or(tl!("Open a document before filling a path."))?;
    if active.doc.id.0 == 0
        || app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || app.ui.pen.as_ref().is_some_and(|pen| pen.knots.len() >= 2)
    {
        return Err(tl!("Finish pending edits before filling a path.").into());
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
            return Err(tl!("The selected layer changed. Reopen Fill Path.").into());
        }
        let layer = Some(active.active_layer.filter(|id| id.0 != 0).ok_or(tl!("Select a pixel layer to fill first."))?);
        let label = photocraft_engine::commands::find(COMMAND).map(|spec| spec.label).ok_or(tl!("Fill Path is unavailable."))?;
        Ok::<_, String>((
            PathFillOrigin { dialog: 0, document: doc.id, revision: active.revision, snapshot: doc.downgrade(), name: name.into(), path, layer },
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
    fixed.insert("__pathFillOrigin".into(), json!(true));
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
    app.ui.path_fill_origin = Some(target);
    Some(id)
}
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(COMMAND) || fields.get("__pathFillOrigin").and_then(Value::as_bool) == Some(true)
}
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    crate::filter_dialog::body(ui, fields);
}
pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let target = app.ui.path_fill_origin.take().ok_or(tl!("The document changed. Reopen Fill Path."))?;
    idle(app)?;
    let active = app.session.active().ok_or(tl!("The document changed. Reopen Fill Path."))?;
    let snapshot = target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen Fill Path."))?;
    if target.dialog != id
        || fields.get("__command").and_then(Value::as_str) != Some(COMMAND)
        || active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
        || target.layer.is_some_and(|layer| active.active_layer != Some(layer))
    {
        return Err(tl!("The document or selected path changed. Reopen Fill Path.").into());
    }
    if fields.len() > 16
        || fields.keys().any(|key| {
            !key.starts_with("__")
                && !matches!(key.as_str(), "name" | "pathId" | "layer" | "color" | "opacity" | "feather" | "antiAlias" | "documentId" | "expectedRevision")
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
    let feather = fields.get("feather").and_then(Value::as_f64).filter(|value| value.is_finite() && value.abs() <= f64::from(f32::MAX)).ok_or_else(invalid)?;
    let anti_alias = fields.get("antiAlias").and_then(Value::as_bool).ok_or_else(invalid)?;
    let mut params = Map::new();
    params.insert("name".into(), json!(target.name));
    if let Some(path) = target.path {
        params.insert("pathId".into(), json!(path.0.to_string()));
    }
    let layer = target.layer.ok_or(tl!("Select a pixel layer to fill first."))?;
    params.insert("layer".into(), json!(layer.0));
    params.insert("color".into(), json!(color));
    params.insert("opacity".into(), json!(opacity));
    params.insert("feather".into(), json!(feather));
    params.insert("antiAlias".into(), json!(anti_alias));
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
    fn original_foreground_defaults_and_actual_fill_keep_path_and_target() {
        let mut app = app();
        path(&mut app, "Own Saved");
        app.session.tools.foreground = [0.25, 0.5, 0.75, 1.];
        let source = app.session.active().unwrap().doc.clone();
        let id = open(&mut app, &json!({"name":"Own Saved"})).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["color"], "#4080bf");
        assert_eq!(fields["opacity"], 100.);
        assert_eq!(fields["feather"], 0.);
        assert_eq!(fields["antiAlias"], true);
        assert!(!fields.contains_key("__preview"));
        assert!(!fields.contains_key("mode"));
        fields.insert("name".into(), json!("wrong-restored-selector"));
        confirm(&mut app, id, &fields).unwrap();
        let after = &app.session.active().unwrap().doc;
        assert_eq!(after.paths, source.paths);
        assert_ne!(after.layers, source.layers);
        app.session.undo();
        assert_eq!(&*app.session.active().unwrap().doc, &*source);
    }
    #[test]
    fn stale_source_tab_edit_and_restored_origin_never_fill_pixels() {
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
                    assert!(app.ui.path_fill_origin.is_none());
                }
            }
            let before = app.session.active().unwrap().doc.clone();
            let revision = app.session.active().unwrap().revision;
            assert!(confirm(&mut app, id, &fields).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            assert_eq!(app.session.active().unwrap().revision, revision);
        }
    }
    #[test]
    fn caller_finishes_pending_path_and_active_pixel_target_change_refuses() {
        let mut app = app();
        app.ui.tool_options.vector_mode = "path".into();
        crate::vector_ui::pen_down(&mut app, 2., 2., false);
        crate::vector_ui::pen_up(&mut app);
        crate::vector_ui::pen_down(&mut app, 20., 2., false);
        crate::vector_ui::pen_up(&mut app);
        assert!(open(&mut app, &json!({"name":"work"})).is_none());
        assert!(app.ui.pen.is_some());
        crate::vector_ui::pen_commit(&mut app, false);
        let id = open(&mut app, &json!({"name":"work"})).unwrap();
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        let before = app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().active_layer = None;
        assert!(confirm(&mut app, id, &fields).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
