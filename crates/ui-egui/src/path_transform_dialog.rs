//! Post-pending-commit path transform dialog, preserving existing source selection.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map, Value, json};
const COMMAND: &str = "path.transform";
const DISPLAY_SPEC: &str = r#"{"translateX":-30000..30000=0,"translateY":-30000..30000=0,"scaleX":0.01..100=1,"scaleY":0.01..100=1,"angle":-360..360=0}"#;
#[derive(Clone, Debug)]
pub(crate) struct PathTransformOrigin {
    dialog: u64,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    name: String,
    path: Option<photocraft_doc::item_identity::PathId>,
    layer: Option<photocraft_doc::LayerId>,
}
impl PartialEq for PathTransformOrigin {
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
    tl!("Check the path transform settings and try again.").into()
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or(tl!("Open a document before transforming a path."))?;
    if active.doc.id.0 == 0
        || app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || app.ui.pen.as_ref().is_some_and(|pen| pen.knots.len() >= 2)
    {
        return Err(tl!("Finish pending edits before transforming a path.").into());
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
            return Err(tl!("The selected layer changed. Reopen Transform Path.").into());
        }
        let needs_layer = path.is_none() && (name == "layer" || (work_alias(name) && doc.work_path.is_none()));
        let layer = if needs_layer { Some(active.active_layer.ok_or(tl!("Select a layer with a path first."))?) } else { explicit_layer };
        let label = photocraft_engine::commands::find(COMMAND).map(|spec| spec.label).ok_or(tl!("Transform Path is unavailable."))?;
        Ok::<_, String>((
            PathTransformOrigin { dialog: 0, document: doc.id, revision: active.revision, snapshot: doc.downgrade(), name: name.into(), path, layer },
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
    fixed.insert("__pathTransformOrigin".into(), json!(true));
    fixed.insert("name".into(), json!(target.name));
    if let Some(id) = target.path {
        fixed.insert("pathId".into(), json!(id.0.to_string()));
    }
    let id = crate::filter_dialog::open_with_spec(app, COMMAND, label, DISPLAY_SPEC, fixed);
    if let Some(dialog) = app.ui.dialog_mut(id) {
        dialog.fields.remove("__preview");
        for (key, value) in [("translateX", 0.), ("translateY", 0.), ("scaleX", 1.), ("scaleY", 1.), ("angle", 0.)] {
            dialog.fields.insert(key.into(), json!(value));
        }
    }
    target.dialog = id;
    app.ui.path_transform_origin = Some(target);
    Some(id)
}
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(COMMAND) || fields.get("__pathTransformOrigin").and_then(Value::as_bool) == Some(true)
}
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    crate::filter_dialog::body(ui, fields);
}
pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let target = app.ui.path_transform_origin.take().ok_or(tl!("The document changed. Reopen Transform Path."))?;
    idle(app)?;
    let active = app.session.active().ok_or(tl!("The document changed. Reopen Transform Path."))?;
    let snapshot = target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen Transform Path."))?;
    if target.dialog != id
        || fields.get("__command").and_then(Value::as_str) != Some(COMMAND)
        || active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
        || target.layer.is_some_and(|layer| active.active_layer != Some(layer))
    {
        return Err(tl!("The document or selected path changed. Reopen Transform Path.").into());
    }
    if fields.len() > 16
        || fields.keys().any(|key| {
            !key.starts_with("__")
                && !matches!(
                    key.as_str(),
                    "name" | "pathId" | "layer" | "translateX" | "translateY" | "scaleX" | "scaleY" | "angle" | "documentId" | "expectedRevision"
                )
        })
    {
        return Err(invalid());
    }
    let mut params = Map::new();
    params.insert("name".into(), json!(target.name));
    if let Some(path) = target.path {
        params.insert("pathId".into(), json!(path.0.to_string()));
    }
    // Existing layer selector is retained only for the actual layer-path target.
    // Other fallback cases retain their original work-path semantics.
    if target.path.is_none()
        && target.name == "layer"
        && let Some(layer) = target.layer
    {
        params.insert("layer".into(), json!(layer.0));
    }
    for key in ["translateX", "translateY", "scaleX", "scaleY", "angle"] {
        let number = fields.get(key).and_then(Value::as_f64).filter(|number| number.is_finite() && number.abs() <= 1_000_000.).ok_or_else(invalid)?;
        params.insert(key.into(), json!(number));
    }
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
    fn original_controls_and_saved_selector_transform_only_captured_current_path() {
        let mut app = app();
        path(&mut app, "Own Saved");
        let id = open(&mut app, &json!({"name":"Own Saved"})).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["translateX"], 0.);
        assert_eq!(fields["translateY"], 0.);
        assert_eq!(fields["scaleX"], 1.);
        assert_eq!(fields["scaleY"], 1.);
        assert_eq!(fields["angle"], 0.);
        assert!(!fields.contains_key("__preview"));
        let saved = app.session.active().unwrap().doc.paths[0].id;
        assert_eq!(fields["pathId"], saved.0.to_string());
        fields.insert("translateX".into(), json!(5.));
        fields.insert("name".into(), json!("wrong-restored-selector"));
        fields.insert("pathId".into(), json!("0"));
        confirm(&mut app, id, &fields).unwrap();
        let current = &app.session.active().unwrap().doc.paths[0];
        assert_eq!(current.id, saved);
        assert_eq!(current.path.control_bounds().unwrap(), (7., 2., 25., 20.));
        app.session.undo();
        assert_eq!(app.session.active().unwrap().doc.paths[0].path.control_bounds().unwrap(), (2., 2., 20., 20.));
    }
    #[test]
    fn stale_source_tab_revision_and_restored_origin_refuse() {
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
                    assert!(app.ui.path_transform_origin.is_none());
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
    fn caller_commits_pending_path_before_origin_and_nonfinite_transform_refuses() {
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
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("scaleX".into(), Value::Null);
        let before = app.session.active().unwrap().doc.clone();
        assert!(confirm(&mut app, id, &fields).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
