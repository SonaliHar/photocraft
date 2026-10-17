//! Captured-origin wrappers around the existing profile parameter forms.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map, Value, json};
const ASSIGN: &str = "edit.assignProfile";
const CONVERT: &str = "edit.convertToProfile";
// Original registry forms, intentionally excluding target selectors as widgets.
const ASSIGN_SPEC: &str = r#"{"profile":"working|srgb|display-p3|adobe-rgb-compat|prophoto-compat|linear-srgb|rec2020|gray-gamma-2.2|sgray|lab-d50|coated-cmyk|none" (or a path to an .icc file)}"#;
const CONVERT_SPEC: &str = r#"{"profile":"srgb|display-p3|adobe-rgb-compat|prophoto-compat|linear-srgb|rec2020|gray-gamma-2.2|sgray|lab-d50|coated-cmyk|working" (or a path to an .icc file),"intent":"perceptual|relative|saturation|absolute"="relative","bpc":bool=true}"#;
#[derive(Clone, Debug)]
pub(crate) struct ProfileOrigin {
    dialog: u64,
    command: String,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
}
impl PartialEq for ProfileOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.dialog == other.dialog
            && self.command == other.command
            && self.document == other.document
            && self.revision == other.revision
            && match (self.snapshot.upgrade(), other.snapshot.upgrade()) {
                (Some(a), Some(b)) => a.shares_state(&b),
                (None, None) => true,
                _ => false,
            }
    }
}
fn supported(command: &str) -> bool {
    matches!(command, ASSIGN | CONVERT)
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or(tl!("Open a document before changing its profile."))?;
    if active.doc.id.0 == 0
        || app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before changing the profile.").into());
    }
    Ok(())
}
pub(crate) fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let prepared = (|| {
        if !supported(command) {
            return Err(tl!("This profile action is unavailable.").into());
        }
        idle(app)?;
        let spec = photocraft_engine::commands::find(command).ok_or(tl!("This profile action is unavailable."))?;
        Ok::<_, String>(spec.label)
    })();
    let label = match prepared {
        Ok(value) => value,
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            return None;
        }
    };
    let mut fixed = Map::new();
    fixed.insert("__profileOrigin".into(), json!(true));
    let display = if command == ASSIGN { ASSIGN_SPEC } else { CONVERT_SPEC };
    let id = crate::filter_dialog::open_with_spec(app, command, label, display, fixed);
    // open_with_spec supports filter previews; these original profile forms did
    // not preview. Keep their existing behavior without executing a proxy edit.
    if let Some(dialog) = app.ui.dialog_mut(id) {
        dialog.fields.remove("__preview");
    }
    let active = app.session.active()?;
    app.ui.profile_origin =
        Some(ProfileOrigin { dialog: id, command: command.into(), document: active.doc.id, revision: active.revision, snapshot: active.doc.downgrade() });
    Some(id)
}
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str).is_some_and(supported) || fields.get("__profileOrigin").and_then(Value::as_bool) == Some(true)
}
pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let target = app.ui.profile_origin.take().ok_or(tl!("The document changed. Reopen the profile dialog."))?;
    idle(app)?;
    let active = app.session.active().ok_or(tl!("The document changed. Reopen the profile dialog."))?;
    let snapshot = target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen the profile dialog."))?;
    if target.dialog != id
        || fields.get("__command").and_then(Value::as_str) != Some(target.command.as_str())
        || !supported(&target.command)
        || active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
    {
        return Err(tl!("The document changed. Reopen the profile dialog.").into());
    }
    if fields.len() > 16
        || fields.keys().any(|key| {
            !key.starts_with("__")
                && !matches!(key.as_str(), "profile" | "documentId" | "expectedRevision")
                && !(target.command == CONVERT && matches!(key.as_str(), "intent" | "bpc"))
        })
    {
        return Err(tl!("Check the profile settings and try again.").into());
    }
    let profile = fields
        .get("profile")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4096 && !value.contains('\0'))
        .ok_or(tl!("Choose a valid profile."))?;
    let mut params = Map::new();
    params.insert("profile".into(), json!(profile));
    if target.command == CONVERT {
        let intent = fields
            .get("intent")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "perceptual" | "relative" | "saturation" | "absolute"))
            .ok_or(tl!("Choose a valid rendering intent."))?;
        let bpc = fields.get("bpc").and_then(Value::as_bool).ok_or(tl!("Check the profile settings and try again."))?;
        params.insert("intent".into(), json!(intent));
        params.insert("bpc".into(), json!(bpc));
    }
    params.insert("documentId".into(), json!(target.document.0.to_string()));
    params.insert("expectedRevision".into(), json!(target.revision.to_string()));
    let result = app.run(&target.command, Value::Object(params))?;
    crate::filter_dialog::remember_profile(app, &target.command, fields);
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":3,"height":2})).unwrap();
        app
    }
    #[test]
    fn both_profile_forms_refuse_stale_tabs_revisions_and_restored_authority() {
        for command in [ASSIGN, CONVERT] {
            for change in 0..3 {
                let mut app = app();
                let id = open(&mut app, command).unwrap();
                let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
                match change {
                    0 => {
                        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
                    }
                    1 => {
                        app.session.execute("image.canvasSize", json!({"width":4,"height":2})).unwrap();
                    }
                    _ => {
                        app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
                        assert!(app.ui.profile_origin.is_none());
                    }
                }
                let before = app.session.active().unwrap().doc.clone();
                let revision = app.session.active().unwrap().revision;
                let preferences = app.session.prefs().dialogs.clone();
                assert!(confirm(&mut app, id, &fields).is_err());
                assert_eq!(app.session.prefs().dialogs, preferences);
                assert_eq!(app.session.active().unwrap().revision, revision);
                assert!(app.session.active().unwrap().doc.shares_state(&before));
            }
        }
    }
    #[test]
    fn existing_choices_defaults_and_no_preview_remain_without_target_widgets() {
        for command in [ASSIGN, CONVERT] {
            let mut app = app();
            let id = open(&mut app, command).unwrap();
            let fields = &app.ui.dialog_mut(id).unwrap().fields;
            assert!(!fields.contains_key("__preview"));
            assert!(!fields.contains_key("documentId"));
            assert!(!fields.contains_key("expectedRevision"));
            assert_eq!(fields["profile"], if command == ASSIGN { "working" } else { "srgb" });
            if command == CONVERT {
                assert_eq!(fields["intent"], "relative");
                assert_eq!(fields["bpc"], true);
            }
        }
    }
    #[test]
    fn high_id_origin_and_live_coalescing_are_not_serialized_field_authority() {
        let mut app = app();
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().id = photocraft_doc::DocId(9_007_199_254_740_995);
        let id = open(&mut app, ASSIGN).unwrap();
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(app.ui.profile_origin.as_ref().unwrap().document.0, 9_007_199_254_740_995);
        let before = app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().coalesce = Some("Own gesture".into());
        assert_eq!(confirm(&mut app, id, &fields).unwrap_err(), tl!("Finish pending edits before changing the profile."));
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
