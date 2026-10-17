//! Runtime target fencing for Color Range; persisted fields are view state only.
use crate::PhotocraftApp;
use photocraft_engine::{WeakDocumentSnapshot, channel_cmds::ChannelView};
use serde_json::{Map, Value};
#[derive(Clone, Debug)]
pub(crate) struct Origin {
    dialog: u64,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    layer: Option<photocraft_doc::LayerId>,
    view: ChannelView,
    shell_mask: bool,
    preview_failure: Option<(u64, String)>,
    pointer_down: bool,
}
impl PartialEq for Origin {
    fn eq(&self, other: &Self) -> bool {
        self.dialog == other.dialog
            && self.document == other.document
            && self.revision == other.revision
            && self.layer == other.layer
            && self.view == other.view
            && self.shell_mask == other.shell_mask
            && self.preview_failure == other.preview_failure
            && self.pointer_down == other.pointer_down
            && match (self.snapshot.upgrade(), other.snapshot.upgrade()) {
                (Some(a), Some(b)) => a.shares_state(&b),
                (None, None) => true,
                _ => false,
            }
    }
}
fn stale() -> String {
    tl!("The Color Range target changed. Reopen the dialog.").into()
}
fn invalid() -> String {
    tl!("Check the Color Range settings and try again.").into()
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or_else(|| tl!("Open a document before using Color Range.").to_owned())?;
    if active.doc.id.0 == 0
        || active.revision >= u64::MAX - 1
        || active.doc.resident_arc().is_none()
        || app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.floating.is_some()
        || active.coalesce.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before using Color Range.").into());
    }
    Ok(())
}
pub(crate) fn capture(app: &mut PhotocraftApp, dialog: u64) -> Result<(), String> {
    app.ui.color_range_origin = None;
    idle(app)?;
    let active = app.session.active().ok_or_else(stale)?;
    if active.channel_view.color_hidden.len() > 16 || active.channel_view.alpha_visible.len() > 10_000 {
        return Err(invalid());
    }
    app.ui.color_range_origin = Some(Origin {
        dialog,
        document: active.doc.id,
        revision: active.revision,
        snapshot: active.doc.downgrade(),
        layer: active.active_layer,
        view: active.channel_view.clone(),
        shell_mask: app.layer_mask_targeted(),
        preview_failure: None,
        pointer_down: false,
    });
    Ok(())
}
/// Transient control-pointer state belongs to the captured runtime dialog, never query fields.
pub(crate) fn pointer_event(app: &mut PhotocraftApp, down: bool, up: bool) -> bool {
    let Some(dialog) = app.ui.dialogs.last().filter(|dialog| crate::color_range_ui::owns(&dialog.fields)) else {
        return false;
    };
    let Some(origin) = app.ui.color_range_origin.as_mut().filter(|origin| origin.dialog == dialog.id) else {
        return false;
    };
    origin.pointer_down = !up && (down || origin.pointer_down);
    origin.pointer_down
}
fn decimal(value: &Value, nonzero: bool) -> bool {
    value.as_str().is_some_and(|text| {
        !text.is_empty()
            && text.len() <= 20
            && (text.len() == 1 || !text.starts_with('0'))
            && text.bytes().all(|byte| byte.is_ascii_digit())
            && text.parse::<u64>().is_ok_and(|value| !nonzero || value != 0)
    })
}
pub(crate) fn fields(fields: &Map<String, Value>) -> Result<(), String> {
    if fields.len() > 32 {
        return Err(invalid());
    }
    let mut points = 0usize;
    for (key, value) in fields {
        if key.len() > 128 {
            return Err(invalid());
        }
        let valid = match key.as_str() {
            "__colorRange" => value.as_bool() == Some(true),
            "__label" => value.as_str().is_some_and(|text| text.len() <= 4096),
            "select" => value.as_str().is_some_and(|text| crate::color_range_ui::SELECTS.iter().any(|(key, _)| *key == text)),
            "__tool" => matches!(value.as_str(), Some("sample" | "add" | "subtract")),
            "__view" => matches!(value.as_str(), Some("selection" | "image")),
            "__selectionPreview" => matches!(value.as_str(), Some("none" | "grayscale" | "blackMatte" | "whiteMatte" | "quickMask")),
            "localized" | "invert" | "sampleAllLayers" | "detectFaces" => value.is_boolean(),
            "fuzziness" | "range" | "toneFuzziness" | "shadowsLevel" | "highlightsLevel" | "midtonesLow" | "midtonesHigh" => {
                value.as_f64().is_some_and(|n| n.is_finite() && (n as f32).is_finite())
            }
            "mode" => matches!(value.as_str(), Some("replace" | "new" | "add" | "subtract" | "intersect")),
            "points" | "subtractPoints" => {
                let array = value.as_array().filter(|array| array.len() <= 4096).ok_or_else(invalid)?;
                points = points.checked_add(array.len()).filter(|count| *count <= 4096).ok_or_else(invalid)?;
                array
                    .iter()
                    .all(|point| point.as_array().is_some_and(|pair| pair.len() == 2 && pair.iter().all(|value| value.as_f64().is_some_and(f64::is_finite))))
            }
            "__order" => value.as_array().is_some_and(|array| array.len() <= 4096 && array.iter().all(|value| matches!(value.as_str(), Some("+" | "-")))),
            "documentId" => decimal(value, true),
            "expectedRevision" => decimal(value, false),
            _ => false,
        };
        if !valid {
            return Err(invalid());
        }
    }
    if fields.contains_key("documentId") != fields.contains_key("expectedRevision") {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) fn validate(app: &PhotocraftApp, dialog: u64, fields_value: &Map<String, Value>) -> Result<(), String> {
    fields(fields_value)?;
    idle(app)?;
    let origin = app.ui.color_range_origin.as_ref().ok_or_else(stale)?;
    let active = app.session.active().ok_or_else(stale)?;
    if origin.dialog != dialog
        || active.doc.id != origin.document
        || active.revision != origin.revision
        || !origin.snapshot.is_current(&active.doc)
        || active.active_layer != origin.layer
        || active.channel_view != origin.view
        || app.layer_mask_targeted() != origin.shell_mask
    {
        return Err(stale());
    }
    Ok(())
}
pub(crate) fn validate_frame(app: &PhotocraftApp, fields: &Map<String, Value>) -> Result<(), String> {
    let dialog = app.ui.dialogs.last().filter(|dialog| crate::color_range_ui::owns(&dialog.fields)).map(|dialog| dialog.id).ok_or_else(stale)?;
    validate(app, dialog, fields)
}
/// Failure memo belongs to this immutable target witness, never to serialized fields.
/// It prevents repeatedly attempting the same refused preview on pointer-only frames.
pub(crate) fn preview_failure(app: &PhotocraftApp, key: u64) -> Option<&str> {
    app.ui.color_range_origin.as_ref()?.preview_failure.as_ref().filter(|(failed, _)| *failed == key).map(|(_, error)| error.as_str())
}
pub(crate) fn remember_preview_failure(app: &mut PhotocraftApp, key: u64, error: &str) {
    if error.len() > 4096 {
        return;
    }
    let Some(origin) = app.ui.color_range_origin.as_mut() else {
        return;
    };
    let mut text = String::new();
    if text.try_reserve_exact(error.len()).is_err() {
        return;
    }
    text.push_str(error);
    origin.preview_failure = Some((key, text));
}
pub(crate) fn clear_preview_failure(app: &mut PhotocraftApp) {
    if let Some(origin) = app.ui.color_range_origin.as_mut() {
        origin.preview_failure = None;
    }
}
pub(crate) fn request(app: &PhotocraftApp, dialog: u64, fields: &Map<String, Value>, mut params: Value) -> Result<Value, String> {
    validate(app, dialog, fields)?;
    let origin = app.ui.color_range_origin.as_ref().ok_or_else(stale)?;
    let object = params.as_object_mut().filter(|object| object.len() <= 24).ok_or_else(invalid)?;
    object.insert("documentId".into(), Value::String(origin.document.0.to_string()));
    object.insert("expectedRevision".into(), Value::String(origin.revision.to_string()));
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":4,"height":4})).unwrap();
        app
    }
    #[test]
    fn query_options_and_foreground_remain_editable_but_target_and_restoration_do_not() {
        let mut app = app();
        let id = crate::color_range_ui::open(&mut app);
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("select".into(), json!("highlights"));
        fields.insert("invert".into(), json!(true));
        app.session.tools.foreground = [0.3, 0.4, 0.5, 1.0];
        assert!(validate(&app, id, &fields).is_ok());
        let request = request(&app, id, &fields, crate::color_range_ui::params(&fields)).unwrap();
        assert_eq!(request["documentId"], app.session.active().unwrap().doc.id.0.to_string());
        app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Color(0);
        assert!(validate(&app, id, &fields).is_err());
        app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(app.ui.color_range_origin.is_none());
        assert!(validate(&app, id, &fields).is_err());
    }
    #[test]
    fn bounded_fields_refuse_before_params_clone_and_revision_changes_stay_stale() {
        let mut app = app();
        let id = crate::color_range_ui::open(&mut app);
        let original = app.ui.dialog_mut(id).unwrap().fields.clone();
        let mut large = original.clone();
        large.insert("points".into(), json!(vec![[0, 0]; 4097]));
        assert!(fields(&large).is_err());
        let mut nested = original.clone();
        nested.insert("points".into(), json!([[0,{"x":1}]]));
        assert!(fields(&nested).is_err());
        let before = app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().revision += 1;
        assert!(crate::color_range_ui::confirm_for_dialog(&mut app, id, &original).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn failed_preview_memo_is_bounded_context_keyed_and_never_target_authority() {
        let mut app = app();
        let id = crate::color_range_ui::open(&mut app);
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        remember_preview_failure(&mut app, 7, "Own refused preview");
        assert_eq!(preview_failure(&app, 7), Some("Own refused preview"));
        assert!(preview_failure(&app, 8).is_none());
        remember_preview_failure(&mut app, 8, &"x".repeat(4097));
        assert!(preview_failure(&app, 8).is_none());
        app.session.active_mut().unwrap().revision += 1;
        assert!(validate(&app, id, &fields).is_err());
        clear_preview_failure(&mut app);
        assert!(preview_failure(&app, 7).is_none());
    }

    #[test]
    fn pointer_hold_is_runtime_only_and_restoration_never_supplies_it() {
        let mut app = app();
        let id = crate::color_range_ui::open(&mut app);
        let original = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert!(pointer_event(&mut app, true, false));
        assert!(pointer_event(&mut app, false, false));
        assert!(!pointer_event(&mut app, false, true));
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields, original);
        assert!(validate(&app, id, &original).is_ok());
        for value in [json!(true), json!(false), json!("true")] {
            let mut hostile = original.clone();
            hostile.insert("__pointerDown".into(), value);
            assert!(fields(&hostile).is_err());
        }
        assert!(pointer_event(&mut app, true, false));
        app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(!pointer_event(&mut app, false, false));
        assert!(validate(&app, id, &original).is_err());
    }
}
