//! Origin-fenced existing Indexed Color/Bitmap forms; no new profile arithmetic.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map, Value, json};
const INDEXED: &str = "image.mode.indexedColor";
const BITMAP: &str = "image.mode.bitmap";
const COLOR_TABLE: &str = "image.mode.colorTable";
const DUOTONE: &str = "image.mode.duotone";
const COLOR_TABLE_SPEC: &str = r##"{"table":"custom|blackBody|grayscale|spectrum|systemMac|systemWindows|web"="custom","colors":json,"entries":json,"transparent":json} (colors: ["#rrggbb", …]; entries: {"index": "#rrggbb"}; transparent: index|null)"##;
const DUOTONE_SPEC: &str =
    r##"{"type":"duotone|monotone|tritone|quadtone"="duotone","inks":json} (inks: [{"name","color":"#rrggbb","curve":[[in,out],…] in 0..100}, …])"##;
const INDEXED_SPEC: &str = r#"{"palette":"selective|perceptual|adaptive|exact|systemMac|systemWindows|web|uniform"="selective","colors":2..256=256,"forced":"blackWhite|none|primaries|web"="blackWhite","transparency":bool=true,"dither":"diffusion|none|pattern|noise"="diffusion","amount":0..100=75} (flattens)"#;
const BITMAP_SPEC: &str = r#"{"method":"diffusion|threshold|pattern|halftone"="diffusion","frequency":1..999=53,"angle":-180..180=45,"shape":"round|ellipse|line|square|diamond|cross"="round"} (halftone frequency in lines/inch; flattens)"#;
#[derive(Clone, Debug)]
pub(crate) struct ModeConversionOrigin {
    dialog: u64,
    command: String,
    document: photocraft_doc::DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
}
impl PartialEq for ModeConversionOrigin {
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
    matches!(command, INDEXED | BITMAP | COLOR_TABLE | DUOTONE)
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or(tl!("Open a document before changing its color mode."))?;
    if active.doc.id.0 == 0
        || app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before changing the color mode.").into());
    }
    Ok(())
}
pub(crate) fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let prepared = (|| {
        if !supported(command) {
            return Err(tl!("This color mode action is unavailable.").into());
        }
        idle(app)?;
        let spec = photocraft_engine::commands::find(command).ok_or(tl!("This color mode action is unavailable."))?;
        let active = app.session.active().ok_or(tl!("Open a document before changing its color mode."))?;
        Ok::<_, String>((
            ModeConversionOrigin { dialog: 0, command: command.into(), document: active.doc.id, revision: active.revision, snapshot: active.doc.downgrade() },
            spec.label,
            active.doc.mode == photocraft_doc::ColorMode::Rgb,
            active.doc.color_table.as_ref().map(|table| table.transparent),
        ))
    })();
    let (mut target, label, rgb, transparent) = match prepared {
        Ok(value) => value,
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            return None;
        }
    };
    let mut fixed = Map::new();
    fixed.insert("__modeConversion".into(), json!(true));
    if command == INDEXED {
        fixed.insert("canonicalIndexed".into(), json!(false));
        fixed.insert("__canonicalIndexedRgb".into(), json!(rgb));
    }
    if command == COLOR_TABLE
        && let Some(transparent) = transparent
    {
        fixed.insert("transparent".into(), json!(transparent));
    }
    let display = match command {
        INDEXED => INDEXED_SPEC,
        BITMAP => BITMAP_SPEC,
        COLOR_TABLE => COLOR_TABLE_SPEC,
        _ => DUOTONE_SPEC,
    };
    let id = crate::filter_dialog::open_with_spec(app, command, label, display, fixed);
    // Color Table was not previewed; Duotone, Indexed and Bitmap were.
    if command == COLOR_TABLE
        && let Some(dialog) = app.ui.dialog_mut(id)
    {
        dialog.fields.remove("__preview");
    }
    target.dialog = id;
    app.ui.mode_conversion_origin = Some(target);
    Some(id)
}
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str).is_some_and(supported) || fields.get("__modeConversion").and_then(Value::as_bool) == Some(true)
}
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    crate::filter_dialog::body(ui, fields);
    if fields.get("__command").and_then(Value::as_str) == Some(INDEXED) {
        let mut keep = fields.get("canonicalIndexed").and_then(Value::as_bool).unwrap_or(false);
        // Presentation only; restored fields never grant engine eligibility.
        let rgb = fields.get("__canonicalIndexedRgb").and_then(Value::as_bool) == Some(true);
        let response = ui.add_enabled_ui(rgb, |ui| crate::widgets::checkbox(ui, &mut keep, tl!("Keep editable palette indices"))).inner;
        if response.changed() {
            fields.insert("canonicalIndexed".into(), json!(keep));
        }
        if !rgb {
            let tokens = crate::theme::Tokens::get(ui.ctx());
            ui.label(egui::RichText::new(tl!("Requires RGB color mode")).color(tokens.text_dim));
        }
    }
}
fn value_error() -> String {
    tl!("Check the color mode settings and try again.").into()
}
// Bound the existing flat JSON wire forms before cloning. Engine validation
// remains authoritative for palette ordinals, ink semantics and current carriers.
fn color_json(value: &Value) -> bool {
    match value {
        Value::String(text) => text.len() <= 64 && !text.contains('\0'),
        Value::Array(values) => (3..=4).contains(&values.len()) && values.iter().all(|value| value.as_f64().is_some_and(f64::is_finite)),
        _ => false,
    }
}
fn validate_json(key: &str, value: &Value) -> Result<(), String> {
    let valid = match key {
        "transparent" => value.is_null() || value.as_u64().is_some_and(|index| index < 256),
        "colors" => value.as_array().is_some_and(|values| !values.is_empty() && values.len() <= 256 && values.iter().all(color_json)),
        "entries" => value.as_object().is_some_and(|entries| {
            entries.len() <= 256
                && entries
                    .iter()
                    .all(|(index, color)| !index.is_empty() && index.len() <= 20 && index.bytes().all(|byte| byte.is_ascii_digit()) && color_json(color))
        }),
        "inks" => value.as_array().is_some_and(|inks| {
            inks.len() <= 4
                && inks.iter().all(|ink| {
                    ink.as_object().is_some_and(|object| {
                        object.len() <= 3
                            && object.iter().all(|(key, value)| match key.as_str() {
                                "name" => value.as_str().is_some_and(|name| name.len() <= 4096 && !name.contains('\0')),
                                "color" => color_json(value),
                                "curve" => value.as_array().is_some_and(|points| {
                                    points.len() <= 4096
                                        && points.iter().all(|point| {
                                            point.as_array().is_some_and(|pair| pair.len() == 2 && pair.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)))
                                        })
                                }),
                                _ => false,
                            })
                    })
                })
        }),
        _ => false,
    };
    if valid { Ok(()) } else { Err(value_error()) }
}
pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let target = app.ui.mode_conversion_origin.take().ok_or(tl!("The document changed. Reopen the color mode dialog."))?;
    idle(app)?;
    let active = app.session.active().ok_or(tl!("The document changed. Reopen the color mode dialog."))?;
    let snapshot = target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen the color mode dialog."))?;
    if target.dialog != id
        || fields.get("__command").and_then(Value::as_str) != Some(target.command.as_str())
        || !supported(&target.command)
        || active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
    {
        return Err(tl!("The document changed. Reopen the color mode dialog.").into());
    }
    let allowed = |key: &str| {
        matches!(key, "documentId" | "expectedRevision")
            || match target.command.as_str() {
                INDEXED => matches!(key, "palette" | "colors" | "forced" | "transparency" | "dither" | "amount" | "canonicalIndexed"),
                BITMAP => matches!(key, "method" | "frequency" | "angle" | "shape"),
                COLOR_TABLE => matches!(key, "table" | "colors" | "entries" | "transparent"),
                DUOTONE => matches!(key, "type" | "inks"),
                _ => false,
            }
    };
    if fields.len() > 20 || fields.keys().any(|key| !key.starts_with("__") && !allowed(key)) {
        return Err(value_error());
    }
    let mut params = Map::new();
    let choices: &[(&str, &[&str])] = if target.command == INDEXED {
        &[
            ("palette", &["selective", "perceptual", "adaptive", "exact", "systemMac", "systemWindows", "web", "uniform"]),
            ("forced", &["blackWhite", "none", "primaries", "web"]),
            ("dither", &["diffusion", "none", "pattern", "noise"]),
        ]
    } else if target.command == BITMAP {
        &[("method", &["diffusion", "threshold", "pattern", "halftone"]), ("shape", &["round", "ellipse", "line", "square", "diamond", "cross"])]
    } else if target.command == COLOR_TABLE {
        &[("table", &["custom", "blackBody", "grayscale", "spectrum", "systemMac", "systemWindows", "web"])]
    } else {
        &[("type", &["duotone", "monotone", "tritone", "quadtone"])]
    };
    for (key, values) in choices {
        let value = fields.get(*key).and_then(Value::as_str).filter(|value| values.contains(value)).ok_or_else(value_error)?;
        params.insert((*key).into(), json!(value));
    }
    let numbers: &[&str] = if target.command == INDEXED {
        &["colors", "amount"]
    } else if target.command == BITMAP {
        &["frequency", "angle"]
    } else {
        &[]
    };
    for key in numbers {
        let value = fields.get(*key).and_then(Value::as_f64).filter(|value| value.is_finite()).ok_or_else(value_error)?;
        params.insert((*key).into(), json!(value));
    }
    if target.command == INDEXED {
        for key in ["transparency", "canonicalIndexed"] {
            let value = fields.get(key).and_then(Value::as_bool).ok_or_else(value_error)?;
            params.insert(key.into(), json!(value));
        }
    }
    if target.command == COLOR_TABLE {
        for key in ["colors", "entries", "transparent"] {
            if let Some(value) = fields.get(key) {
                validate_json(key, value)?;
                params.insert(key.into(), value.clone());
            }
        }
    } else if target.command == DUOTONE
        && let Some(value) = fields.get("inks")
    {
        validate_json("inks", value)?;
        params.insert("inks".into(), value.clone());
    }
    params.insert("documentId".into(), json!(target.document.0.to_string()));
    params.insert("expectedRevision".into(), json!(target.revision.to_string()));
    app.filter_preview = None;
    app.run(&target.command, Value::Object(params))
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
    fn both_forms_refuse_stale_tabs_source_edits_and_restored_origins() {
        for command in [INDEXED, BITMAP] {
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
                        assert!(app.ui.mode_conversion_origin.is_none());
                    }
                }
                let before = app.session.active().unwrap().doc.clone();
                let revision = app.session.active().unwrap().revision;
                assert!(confirm(&mut app, id, &fields).is_err());
                assert_eq!(app.session.active().unwrap().revision, revision);
                assert!(app.session.active().unwrap().doc.shares_state(&before));
            }
        }
    }
    #[test]
    fn original_defaults_preview_and_new_opt_in_are_explicit_without_target_widgets() {
        for command in [INDEXED, BITMAP] {
            let mut app = app();
            let id = open(&mut app, command).unwrap();
            let fields = &app.ui.dialog_mut(id).unwrap().fields;
            assert_eq!(fields["__preview"], true);
            assert!(!fields.contains_key("documentId"));
            assert!(!fields.contains_key("expectedRevision"));
            if command == INDEXED {
                assert_eq!(fields["palette"], "selective");
                assert_eq!(fields["colors"].as_u64(), Some(256));
                assert_eq!(fields["forced"], "blackWhite");
                assert_eq!(fields["dither"], "diffusion");
                assert_eq!(fields["transparency"], true);
                assert_eq!(fields["amount"], 75.0);
                assert_eq!(fields["canonicalIndexed"], false);
            } else {
                assert_eq!(fields["method"], "diffusion");
                assert_eq!(fields["frequency"], 53.0);
                assert_eq!(fields["angle"], 45.0);
                assert_eq!(fields["shape"], "round");
                assert!(!fields.contains_key("canonicalIndexed"));
            }
        }
    }
    #[test]
    fn unknown_future_parameter_and_live_coalesce_refuse_without_pixel_changes() {
        let mut app = app();
        let id = open(&mut app, INDEXED).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("unknownFutureMode".into(), json!(true));
        let before = app.session.active().unwrap().doc.clone();
        assert!(confirm(&mut app, id, &fields).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let id = open(&mut app, BITMAP).unwrap();
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        app.session.active_mut().unwrap().coalesce = Some("Own gesture".into());
        assert_eq!(confirm(&mut app, id, &fields).unwrap_err(), tl!("Finish pending edits before changing the color mode."));
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn editable_index_hint_is_captured_rgb_only_not_restored_authority() {
        let mut app = app();
        let id = open(&mut app, INDEXED).unwrap();
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__canonicalIndexedRgb"], true);
        app.ui.close_dialog(id);
        app.session.execute("file.new", json!({"width":3,"height":2,"mode":"gray"})).unwrap();
        let id = open(&mut app, INDEXED).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["__canonicalIndexedRgb"], false);
        assert_eq!(fields["canonicalIndexed"], false);
        fields.insert("__canonicalIndexedRgb".into(), json!(true));
        fields.insert("canonicalIndexed".into(), json!(true));
        let before = app.session.active().unwrap().doc.clone();
        assert!(confirm(&mut app, id, &fields).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }

    #[test]
    fn color_table_and_duotone_refuse_stale_and_restored_dialogs() {
        for command in [COLOR_TABLE, DUOTONE] {
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
                    }
                }
                let before = app.session.active().unwrap().doc.clone();
                assert!(confirm(&mut app, id, &fields).is_err());
                assert!(app.session.active().unwrap().doc.shares_state(&before));
            }
        }
    }
    #[test]
    fn table_transparency_prefill_preserves_no_op_and_duotone_defaults_preview() {
        let mut app = app();
        app.session.execute(INDEXED, json!({"palette":"uniform","colors":4,"dither":"none","forced":"none"})).unwrap();
        app.session.execute(COLOR_TABLE, json!({"transparent":1})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let id = open(&mut app, COLOR_TABLE).unwrap();
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["table"], "custom");
        assert_eq!(fields["transparent"], 1);
        assert!(!fields.contains_key("__preview"));
        assert!(!fields.contains_key("colors"));
        assert!(!fields.contains_key("entries"));
        confirm(&mut app, id, &fields).unwrap();
        assert_eq!(app.session.active().unwrap().doc.color_table, before.color_table);
        let id = open(&mut app, DUOTONE).unwrap();
        let fields = &app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(fields["type"], "duotone");
        assert_eq!(fields["__preview"], true);
        assert!(!fields.contains_key("inks"));
        assert!(!fields.contains_key("documentId"));
        assert!(validate_json("inks", &json!([{"name":"Own","color":"#010203","curve":[[0.,0.],[100.,100.]]}])).is_ok());
        assert!(validate_json("entries", &json!({"1":"#010203"})).is_ok());
        assert!(validate_json("transparent", &json!(256)).is_err());
        assert!(validate_json("inks", &json!([{"newAuthority":true}])).is_err());
    }
}
