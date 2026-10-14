//! Edit › Generative Fill…: the prompt dialog in front of `edit.generativeFill`.
//!
//! Describe what to put in the selection (empty removes it), pick the provider, and Generate. The
//! provider's credentials live under "Provider settings"; they are saved through `ai.settings`
//! (the preferences, never shown again) the moment Generate is pressed. The fields are the
//! command's params, so `ui.dialog.set` / `ui.dialog.confirm` drive it like any dialog.

use egui::vec2;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

pub const COMMAND: &str = "edit.generativeFill";
const MARK: &str = "__genFill";

/// Is this dialog the Generative Fill dialog?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// The provider list of `ai.settings` and the active provider id.
fn state(app: &PhotocraftApp) -> (String, Vec<Value>) {
    let v = photocraft_engine::genfill_cmds::settings_state(&app.session);
    let provider = v.get("provider").and_then(Value::as_str).unwrap_or("cloudflare").to_string();
    let providers = v.get("providers").and_then(Value::as_array).cloned().unwrap_or_default();
    (provider, providers)
}

/// Open the dialog; an error says why Generative Fill can't run now (no selection).
pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    if let Some(Err(why)) = photocraft_engine::commands::find(COMMAND).map(|c| (c.enabled)(&app.session)) {
        return Err(why);
    }
    let (provider, _) = state(app);
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Generative Fill…"));
    f.insert("prompt".into(), json!(""));
    f.insert("provider".into(), json!(provider));
    f.insert("showSettings".into(), json!(false));
    for k in ["apiKey", "accountId", "baseUrl", "model"] {
        f.insert(k.into(), json!(""));
    }
    Ok(app.ui.open_dialog(DialogKind::Command, f))
}

fn get_str(f: &Map<String, Value>, k: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// One line of advice for each provider.
fn hint(id: &str) -> &'static str {
    match id {
        "cloudflare" => "Free daily allowance. Needs your Cloudflare account id and an API token with Workers AI access.",
        "sdwebui" => "Runs on this computer. Start AUTOMATIC1111 or Forge with the --api option.",
        "gemini" => "Needs a Google AI Studio API key.",
        "openai" => "Works with OpenAI and with any server that speaks its images/edits API (set the base URL).",
        "stability" => "Needs a Stability AI API key (paid credits).",
        _ => "",
    }
}

/// A small explanatory line under the controls, wrapped inside the controls' column.
fn note(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::NONE.inner_margin(egui::Margin { left: 100, right: 0, top: 0, bottom: 0 }).show(ui, |ui| {
        ui.label(egui::RichText::new(text).size(11.5).color(color));
    });
}

/// The text in front of the settings fields (a real label, so screen readers and tests see it).
fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.allocate_ui_with_layout(vec2(96.0, 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(text).size(12.0).color(t.text_dim));
    });
}

fn text_row(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, text: &str, hint_text: &str, password: bool) {
    ui.horizontal(|ui| {
        label(ui, text);
        let mut v = get_str(f, key);
        let edit = egui::TextEdit::singleline(&mut v).hint_text(hint_text).password(password).desired_width(ui.available_width().max(160.0));
        if ui.add(edit).changed() {
            f.insert(key.into(), json!(v));
        }
    });
}

/// The dialog body.
pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let (_, providers) = state(app);
    ui.spacing_mut().item_spacing.y = 6.0;

    let mut prompt = get_str(f, "prompt");
    let field = ui.add(
        egui::TextEdit::multiline(&mut prompt)
            .hint_text("Describe what to generate. Leave empty to remove the selected content.")
            .desired_rows(3)
            .desired_width(f32::INFINITY),
    );
    if field.changed() {
        f.insert("prompt".into(), json!(prompt));
    }
    if !ui.is_sizing_pass() && f.get("__focused").is_none() {
        field.request_focus();
        f.insert("__focused".into(), json!(true));
    }

    let mut provider = get_str(f, "provider");
    ui.horizontal(|ui| {
        label(ui, "Provider:");
        let opts: Vec<(String, &str)> = providers.iter().filter_map(|p| Some((p.get("id")?.as_str()?.to_string(), p.get("label")?.as_str()?))).collect();
        if crate::widgets::dropdown(ui, "genfill-provider", &mut provider, &opts, 320.0) {
            f.insert("provider".into(), json!(provider));
            for k in ["apiKey", "accountId", "baseUrl", "model"] {
                f.insert(k.into(), json!(""));
            }
        }
    });
    let info = providers.iter().find(|p| p.get("id").and_then(Value::as_str) == Some(provider.as_str()));
    let configured = info.and_then(|p| p.get("configured")).and_then(Value::as_bool).unwrap_or(false);
    let typed_key = !get_str(f, "apiKey").trim().is_empty();
    note(ui, hint(&provider), t.text_faint);
    if !configured && !typed_key && !f.contains_key("__autoOpened") {
        // Credentials missing: open the settings once so the next step is obvious.
        f.insert("__autoOpened".into(), json!(true));
        f.insert("showSettings".into(), json!(true));
    }

    let mut show = f.get("showSettings").and_then(Value::as_bool).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.add_space(100.0);
        if crate::widgets::checkbox(ui, &mut show, "Provider settings").changed() {
            f.insert("showSettings".into(), json!(show));
        }
    });
    if show && let Some(info) = info {
        let needs_key = info.get("needsKey").and_then(Value::as_bool).unwrap_or(true);
        let needs_account = info.get("needsAccount").and_then(Value::as_bool).unwrap_or(false);
        let source = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or("none").to_string();
        let default = |k: &str| info.get("defaults").and_then(|d| d.get(k)).and_then(Value::as_str).unwrap_or_default().to_string();
        if needs_key {
            let saved = match source("keySource").as_str() {
                "saved" => "Saved. Type to replace.",
                "environment" => "Using the environment variable. Type to override.",
                _ => "Paste your API key",
            };
            text_row(ui, f, "apiKey", "API key:", saved, true);
        }
        if needs_account {
            let saved = if source("accountSource") == "none" { "Your account id" } else { "Saved. Type to replace." };
            text_row(ui, f, "accountId", "Account id:", saved, false);
        }
        let base = info.get("baseUrl").and_then(Value::as_str).filter(|b| !b.is_empty()).map(str::to_string).unwrap_or_else(|| default("baseUrl"));
        text_row(ui, f, "baseUrl", "Server URL:", &base, false);
        let model = info.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()).map(str::to_string).unwrap_or_else(|| default("model"));
        text_row(ui, f, "model", "Model:", if model.is_empty() { "(server default)" } else { &model }, false);
        note(ui, "Keys are stored in a separate file in your user config folder, not in preferences. Empty fields keep what is saved.", t.text_faint);
    }
    if !configured
        && !typed_key
        && let Some(why) = info.and_then(|p| p.get("missing")).and_then(Value::as_str)
    {
        note(ui, why, t.text_dim);
    }
    note(ui, "Only a crop around the selection is sent, and only when you press Generate.", t.text_faint);
}

/// The `ai.settings` params that save what the user typed (`None` when nothing changed).
fn settings_params(f: &Map<String, Value>) -> Value {
    let provider = get_str(f, "provider");
    let mut cfg = Map::new();
    cfg.insert("provider".into(), json!(provider));
    for k in ["apiKey", "accountId", "baseUrl", "model"] {
        let v = get_str(f, k);
        if !v.trim().is_empty() {
            cfg.insert(k.into(), json!(v.trim()));
        }
    }
    json!({ "provider": provider, "configure": cfg })
}

/// Generate: save the provider choice and anything typed, then run the command.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    app.run("ai.settings", settings_params(f))?;
    app.run(COMMAND, json!({ "prompt": get_str(f, "prompt"), "provider": get_str(f, "provider") }))
}

/// The options-bar button next to Select Subject: Generative Fill… on the current selection.
pub fn options_button(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let has_selection = app.session.active().is_some_and(|st| st.doc.selection.is_some());
    let clicked = ui.add_enabled_ui(has_selection, |ui| crate::widgets::secondary_button(ui, tl!("Generative Fill…"), 0.0).clicked()).inner;
    if clicked {
        let _ = crate::menus::invoke(app, ui.ctx(), COMMAND, json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn app_with_selection() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.session
            .edit("select", |doc, _| {
                let mut sel = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
                sel.fill_rect(photocraft_geom::Rect::new(8, 8, 24, 24), &[1.0]);
                doc.selection = Some(sel);
                Ok(())
            })
            .unwrap();
        app
    }

    #[test]
    fn the_menu_needs_a_selection_then_opens_the_dialog() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        let e = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap_err();
        assert!(e.contains("selection"), "{e}");
        let mut app = app_with_selection();
        let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        let id = r["dialog"].as_u64().unwrap();
        assert!(app.ui.dialogs.iter().any(|d| d.id == id && owns(&d.fields)));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "nothing is generated until Generate");
    }

    #[test]
    fn generate_without_credentials_explains_what_is_missing() {
        let mut app = app_with_selection();
        let id = open(&mut app).unwrap();
        let d = app.ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
        d.fields.insert("provider".into(), json!("openai"));
        d.fields.insert("prompt".into(), json!("a lighthouse"));
        let e = crate::dialogs::confirm(&mut app, id).unwrap_err();
        assert!(e.contains("API key"), "{e}");
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
        // The provider choice was remembered even though the run could not start.
        assert_eq!(photocraft_engine::genfill_cmds::settings(&app.session).provider.map(|p| p.id()), Some("openai"));
    }

    #[test]
    fn the_dialog_shows_the_prompt_and_provider_controls() {
        let mut app = app_with_selection();
        open(&mut app).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    crate::dialogs::show(app, &ctx);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        assert!(h.query_by_label("Provider:").is_some());
        assert!(h.query_by_label("Generate").is_some() && h.query_by_label("Cancel").is_some());
        assert!(h.query_by_label("Provider settings").is_some());
    }

    #[test]
    fn settings_params_skip_empty_fields() {
        let mut f = Map::new();
        f.insert("provider".into(), json!("gemini"));
        f.insert("apiKey".into(), json!("  abc "));
        f.insert("accountId".into(), json!(""));
        let p = settings_params(&f);
        assert_eq!(p["provider"], "gemini");
        assert_eq!(p["configure"]["apiKey"], "abc");
        assert!(p["configure"].get("accountId").is_none());
    }
}
