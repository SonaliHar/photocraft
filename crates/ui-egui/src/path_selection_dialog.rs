//! Post-pending-commit path selection dialog, preserving existing source selection.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map, Value, json};
const COMMAND: &str = "path.toSelection";
const DISPLAY_SPEC: &str = r#"{"feather":0..250=0,"antiAlias":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"#;
#[derive(Clone, Debug)]
pub(crate) struct PathSelectionOrigin {
    dialog: u64,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    name: String,
    path: Option<photocraft_doc::item_identity::PathId>,
    layer: Option<photocraft_doc::LayerId>,
}
impl PartialEq for PathSelectionOrigin {
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
    tl!("Check the path selection settings and try again.").into()
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or(tl!("Open a document before making a path selection."))?;
    if active.doc.id.0 == 0
        || app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || app.ui.pen.as_ref().is_some_and(|pen| pen.knots.len() >= 2)
    {
        return Err(tl!("Finish pending edits before making a path selection.").into());
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
            return Err(tl!("The selected layer changed. Reopen Make Selection.").into());
        }
        let needs_layer = path.is_none() && (name == "layer" || (work_alias(name) && doc.work_path.is_none()));
        let layer = if needs_layer { Some(active.active_layer.ok_or(tl!("Select a layer with a path first."))?) } else { explicit_layer };
        let label = photocraft_engine::commands::find(COMMAND).map(|spec| spec.label).ok_or(tl!("Make Selection is unavailable."))?;
        Ok::<_, String>((
            PathSelectionOrigin { dialog: 0, document: doc.id, revision: active.revision, snapshot: doc.downgrade(), name: name.into(), path, layer },
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
    fixed.insert("__pathSelectionOrigin".into(), json!(true));
    fixed.insert("name".into(), json!(target.name));
    if let Some(id) = target.path {
        fixed.insert("pathId".into(), json!(id.0.to_string()));
    }
    let id = crate::filter_dialog::open_with_spec(app, COMMAND, label, DISPLAY_SPEC, fixed);
    if let Some(dialog) = app.ui.dialog_mut(id) {
        dialog.fields.remove("__preview");
    }
    target.dialog = id;
    app.ui.path_selection_origin = Some(target);
    Some(id)
}
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(COMMAND) || fields.get("__pathSelectionOrigin").and_then(Value::as_bool) == Some(true)
}
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    crate::filter_dialog::body(ui, fields);
}
pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let target = app.ui.path_selection_origin.take().ok_or(tl!("The document changed. Reopen Make Selection."))?;
    idle(app)?;
    let active = app.session.active().ok_or(tl!("The document changed. Reopen Make Selection."))?;
    let snapshot = target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen Make Selection."))?;
    if target.dialog != id
        || fields.get("__command").and_then(Value::as_str) != Some(COMMAND)
        || active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
        || target.layer.is_some_and(|layer| active.active_layer != Some(layer))
    {
        return Err(tl!("The document or selected path changed. Reopen Make Selection.").into());
    }
    if fields.len() > 16
        || fields.keys().any(|key| {
            !key.starts_with("__") && !matches!(key.as_str(), "name" | "pathId" | "feather" | "antiAlias" | "mode" | "documentId" | "expectedRevision")
        })
    {
        return Err(invalid());
    }
    let feather = fields.get("feather").and_then(Value::as_f64).filter(|value| value.is_finite() && value.abs() <= f64::from(f32::MAX)).ok_or_else(invalid)?;
    let anti_alias = fields.get("antiAlias").and_then(Value::as_bool).ok_or_else(invalid)?;
    let mode = fields.get("mode").and_then(Value::as_str).filter(|value| matches!(*value, "replace" | "add" | "subtract" | "intersect")).ok_or_else(invalid)?;
    let mut params = Map::new();
    params.insert("name".into(), json!(target.name));
    if let Some(path) = target.path {
        params.insert("pathId".into(), json!(path.0.to_string()));
    }
    params.insert("feather".into(), json!(feather));
    params.insert("antiAlias".into(), json!(anti_alias));
    params.insert("mode".into(), json!(mode));
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
    fn original_work_selector_defaults_and_selection_are_retained() {
        let mut app = app();
        path(&mut app, "work");
        let id = open(&mut app, &json!({"name":"work"})).unwrap();
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["name"], "work");
        assert_eq!(fields["feather"], 0.);
        assert_eq!(fields["antiAlias"], true);
        assert_eq!(fields["mode"], "replace");
        assert!(!fields.contains_key("__preview"));
        confirm(&mut app, id, &fields).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_some());
    }
    #[test]
    fn saved_selector_stays_canonical_and_stale_or_restored_dialogs_refuse() {
        for change in 0..3 {
            let mut app = app();
            path(&mut app, "Own Saved");
            let id = open(&mut app, &json!({"name":"Own Saved"})).unwrap();
            let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
            assert_eq!(fields["pathId"], app.session.active().unwrap().doc.paths[0].id.0.to_string());
            match change {
                0 => {
                    app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
                }
                1 => {
                    app.session.execute("image.canvasSize", json!({"width":33,"height":32})).unwrap();
                }
                _ => {
                    app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
                    assert!(app.ui.path_selection_origin.is_none());
                }
            }
            let before = app.session.active().unwrap().doc.clone();
            assert!(confirm(&mut app, id, &fields).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
        }
    }
    #[test]
    fn pending_path_must_be_committed_by_the_existing_caller_before_capture() {
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
        assert!(app.ui.pen.is_none());
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["name"], "work");
    }
}
