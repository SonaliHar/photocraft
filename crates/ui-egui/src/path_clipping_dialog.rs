//! Captured saved-path clipping metadata, after the caller's pending pen commit.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map, Value, json};

const COMMAND: &str = "path.clippingPath.set";

#[derive(Clone, Debug)]
pub(crate) struct PathClippingOrigin {
    dialog: u64,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    path: photocraft_doc::item_identity::PathId,
    name: String,
}

impl PartialEq for PathClippingOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.dialog == other.dialog
            && self.document == other.document
            && self.revision == other.revision
            && self.path == other.path
            && self.name == other.name
            && match (self.snapshot.upgrade(), other.snapshot.upgrade()) {
                (Some(a), Some(b)) => a.shares_state(&b),
                (None, None) => true,
                _ => false,
            }
    }
}

fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let state = app.session.active().ok_or(tl!("Open a document before setting a clipping path."))?;
    if state.doc.id.0 == 0
        || state.revision == u64::MAX
        || state.coalesce.is_some()
        || state.floating.is_some()
        || app.session.has_jobs()
        || app.ui.transform.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || app.ui.pen.as_ref().is_some_and(|pen| pen.knots.len() >= 2)
    {
        return Err(tl!("Finish pending edits before setting a clipping path.").into());
    }
    Ok(())
}

pub(crate) fn open(app: &mut PhotocraftApp, name: &str) -> Option<u64> {
    let prepared = (|| {
        idle(app)?;
        if name.is_empty() || name.len() > 2048 || name.contains('\0') {
            return Err(tl!("Select a saved path for clipping.").into());
        }
        let state = app.session.active().ok_or(tl!("Select a saved path for clipping."))?;
        if state.doc.paths.len() > 10_000 {
            return Err(tl!("The saved path count exceeds the editing limit.").into());
        }
        let mut matching = state.doc.paths.iter().filter(|path| path.name == name);
        let path = matching.next().filter(|path| path.id.0 != 0).ok_or(tl!("Select a saved path for clipping."))?;
        if matching.next().is_some() {
            return Err(tl!("The saved path name is ambiguous. Rename the path first.").into());
        }
        Ok::<_, String>(PathClippingOrigin {
            dialog: 0,
            document: state.doc.id,
            revision: state.revision,
            snapshot: state.doc.downgrade(),
            path: path.id,
            name: name.into(),
        })
    })();
    let mut origin = match prepared {
        Ok(origin) => origin,
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            return None;
        }
    };
    let mut fields = Map::new();
    fields.insert("name".into(), json!(origin.name));
    fields.insert("__pathClippingOrigin".into(), json!(true));
    let id = crate::filter_dialog::open_with_spec(app, COMMAND, "Clipping Path", r#"{"flatness":0..100=0}"#, fields);
    if let Some(dialog) = app.ui.dialog_mut(id) {
        dialog.fields.remove("__preview");
    }
    origin.dialog = id;
    app.ui.path_clipping_origin = Some(origin);
    Some(id)
}

pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(COMMAND) || fields.get("__pathClippingOrigin").and_then(Value::as_bool) == Some(true)
}

pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    crate::filter_dialog::body(ui, fields);
}

pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let origin = app.ui.path_clipping_origin.take().ok_or(tl!("The document changed. Reopen Clipping Path."))?;
    idle(app)?;
    let state = app.session.active().ok_or(tl!("The document changed. Reopen Clipping Path."))?;
    let snapshot = origin.snapshot.upgrade().ok_or(tl!("The document changed. Reopen Clipping Path."))?;
    if origin.dialog != id
        || fields.get("__command").and_then(Value::as_str) != Some(COMMAND)
        || state.doc.id != origin.document
        || state.revision != origin.revision
        || !state.doc.shares_state(&snapshot)
        || !state.doc.paths.iter().any(|path| path.id == origin.path && path.name == origin.name)
    {
        return Err(tl!("The document or saved path changed. Reopen Clipping Path.").into());
    }
    if fields.len() > 12 || fields.keys().any(|key| !key.starts_with("__") && !matches!(key.as_str(), "name" | "flatness")) {
        return Err(tl!("Check the clipping path settings and try again.").into());
    }
    let flatness = fields
        .get("flatness")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or(tl!("Flatness must be between 0 and 100."))?;
    app.run(COMMAND, json!({"name":origin.name,"flatness":flatness,"documentId":origin.document.0.to_string(),"expectedRevision":origin.revision.to_string()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        app.session.execute("path.set", json!({"name":"Saved","path":{"subpaths":[]}})).unwrap();
        app
    }
    #[test]
    fn captured_name_and_default_metadata_are_undoable() {
        let mut app = app();
        let original = app.session.active().unwrap().doc.clone();
        let id = open(&mut app, "Saved").unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["flatness"], 0.);
        assert!(!fields.contains_key("__preview"));
        fields.insert("name".into(), json!("Restored impostor"));
        confirm(&mut app, id, &fields).unwrap();
        assert_eq!(app.session.active().unwrap().doc.clipping_path.as_ref().unwrap().name, "Saved");
        assert!(app.session.undo());
        assert_eq!(&*app.session.active().unwrap().doc, &*original);
    }
    #[test]
    fn tab_switch_and_restored_ui_cannot_author_clipping_metadata() {
        for restored in [false, true] {
            let mut app = app();
            let id = open(&mut app, "Saved").unwrap();
            let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
            if restored {
                app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
                assert!(app.ui.path_clipping_origin.is_none());
            } else {
                app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
            }
            let before = app.session.active().unwrap().doc.clone();
            assert!(confirm(&mut app, id, &fields).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
        }
    }
}
