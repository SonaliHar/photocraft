//! JPEG Options: the quality File › Save As and Save a Copy ask for before writing a JPEG, as
//! Photoshop's JPEG Options does (#5563): Quality 0–12 on Photoshop's scale, with its Low / Medium
//! / High / Maximum presets. Its Matte and Format Options (Baseline Optimized, Progressive) aren't
//! offered: the encoder writes baseline JPEGs.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. Scripts skip the prompt by passing `quality`.

use crate::{ExportSettings, PhotocraftApp, SaveAnswers};

/// The highest quality on Photoshop's JPEG scale.
pub const MAX_QUALITY: u8 = 12;

/// A save waiting on the JPEG Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// The quality chosen, 0–12, starting at the opened JPEG's own (see [`start_quality`]).
    pub quality: u8,
    /// Save a Copy, or a Save As of a document the JPEG can't hold: the document keeps its path
    /// and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as: `file.saveAs` or `file.saveACopy`.
    pub record: &'static str,
}

/// `true` when `path` names a JPEG (`.jpg`, `.jpeg`, `.jpe`…).
pub fn is_jpeg_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_codecs::from_extension(&e)) == Some(photocraft_codecs::Format::Jpeg)
}

/// Whether saving to `path` asks for JPEG Options.
pub fn wants_prompt(path: &str) -> bool {
    is_jpeg_path(path)
}

/// Photoshop's 0–12 quality as the encoder's 1–100, the way the engine's `quality` param maps it.
pub fn to_percent(quality: u8) -> u8 {
    let q = f64::from(quality.min(MAX_QUALITY));
    (q / f64::from(MAX_QUALITY) * 99.0 + 1.0).round() as u8
}

/// The encoder's 1–100 as the nearest 0–12 quality ([`to_percent`] reversed).
pub fn from_percent(percent: u8) -> u8 {
    let p = f64::from(percent.clamp(1, 100));
    ((p - 1.0) * f64::from(MAX_QUALITY) / 99.0).round() as u8
}

/// The quality the prompt starts at: the opened JPEG's own, as Photoshop's Save As starts there,
/// else the encoder's default (what a save wrote before the prompt).
fn start_quality(app: &PhotocraftApp) -> u8 {
    let percent = app.save_settings().jpeg_quality.unwrap_or(photocraft_codecs::EncodeOptions::default().jpeg_quality);
    from_percent(percent)
}

/// The JPEG quality a command or action step asks for (`"quality": 0..=12`, rounded to a whole
/// step as Photoshop's field does), which skips the prompt.
pub fn quality_param(params: &serde_json::Value, cmd: &str) -> Result<Option<u8>, String> {
    match params.get("quality") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|q| (0.0..=f64::from(MAX_QUALITY)).contains(q))
            .map(|q| Some(q.round() as u8))
            .ok_or_else(|| format!("{cmd}: `quality` must be a number from 0 to 12, got {v}")),
    }
}

/// Photoshop's quality presets and the quality each sets.
const PRESETS: [(Preset, &str, u8); 4] = [(Preset::Low, "Low", 3), (Preset::Medium, "Medium", 5), (Preset::High, "High", 8), (Preset::Maximum, "Maximum", 10)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Preset {
    Low,
    Medium,
    High,
    Maximum,
}

/// The preset a quality falls under: 0–4 Low, 5–7 Medium, 8–9 High, 10–12 Maximum.
fn preset_of(quality: u8) -> Preset {
    match quality {
        0..=4 => Preset::Low,
        5..=7 => Preset::Medium,
        8..=9 => Preset::High,
        _ => Preset::Maximum,
    }
}

/// Parks the save behind the prompt. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let doc = app.active_doc_id()?;
    let quality = start_quality(app);
    app.jpeg_options = Some(Prompt { doc, path, quality, copy, record });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.jpeg_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut quality = p.quality;
    let modal = egui::Modal::new(egui::Id::new("jpeg-options")).show(ctx, |ui| {
        ui.set_max_width(360.0);
        ui.label(egui::RichText::new(tl!("JPEG Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Image Options"));
        let mut preset = preset_of(quality);
        let options: Vec<(Preset, &str)> = PRESETS.iter().map(|(p, label, _)| (*p, tl!(label))).collect();
        if crate::widgets::dropdown(ui, "jpeg-options-preset", &mut preset, &options, 140.0) {
            quality = PRESETS.iter().find(|(p, ..)| *p == preset).map_or(quality, |(.., q)| *q);
        }
        let mut value = f32::from(quality);
        crate::widgets::slider_row_prec(ui, tl!("Quality"), &mut value, 0.0..=f32::from(MAX_QUALITY), "", None, Default::default(), Some(0));
        quality = value.round().clamp(0.0, f32::from(MAX_QUALITY)) as u8;
        ui.horizontal(|ui| {
            ui.small(tl!("smaller file"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.small(tl!("larger file")));
        });
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.quality = quality;
    if ok || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        let Some(p) = app.jpeg_options.take() else { return };
        let answers = SaveAnswers { jpeg_quality: Some(p.quality), ..SaveAnswers::default() };
        let settings: ExportSettings = answers.apply(app.save_settings());
        let mut step = serde_json::json!({"path": p.path});
        answers.record(&mut step);
        let written = app.with_document(p.doc, |app| app.write_document(p.path, &settings, p.copy));
        if written.is_ok() {
            // An action being recorded keeps the Save As or Save a Copy with this answer (#2032).
            app.session.journal.push((p.record.into(), step));
        }
        match written {
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
            }
            Ok(Some(_)) if !p.copy => crate::discard_ui::saved_document(app, ctx, p.doc),
            // A background save releases the close prompt when it finishes (`jobs_ui`).
            Ok(_) => {}
        }
    } else if cancel {
        app.jpeg_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jpeg_names() {
        assert!(is_jpeg_path("a.jpg"));
        assert!(is_jpeg_path("dir/sub/a.JPEG"));
        assert!(is_jpeg_path("C:\\x\\y.jpe"));
        assert!(!is_jpeg_path("a.jpg.psd"));
        assert!(!is_jpeg_path(".jpg"));
        assert!(!is_jpeg_path("jpg"));
        assert!(!is_jpeg_path("a.png"));
    }

    /// The 0–12 scale maps onto the encoder's 1–100 as the engine's `quality` does, and back.
    #[test]
    fn quality_scale_round_trips() {
        assert_eq!(to_percent(0), 1);
        assert_eq!(to_percent(12), 100);
        assert_eq!(to_percent(200), 100);
        assert_eq!(from_percent(0), 0);
        assert_eq!(from_percent(90), 11);
        assert_eq!(from_percent(255), 12);
        for q in 0..=MAX_QUALITY {
            assert_eq!(from_percent(to_percent(q)), q);
        }
    }

    #[test]
    fn presets_cover_the_scale() {
        for (preset, _, q) in PRESETS {
            assert_eq!(preset_of(q), preset);
        }
        assert_eq!(preset_of(0), Preset::Low);
        assert_eq!(preset_of(7), Preset::Medium);
        assert_eq!(preset_of(12), Preset::Maximum);
    }

    #[test]
    fn quality_params() {
        assert_eq!(quality_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(quality_param(&json!({"quality": null}), "file.saveAs"), Ok(None));
        assert_eq!(quality_param(&json!({"quality": 0}), "file.saveAs"), Ok(Some(0)));
        assert_eq!(quality_param(&json!({"quality": 12}), "file.saveAs"), Ok(Some(12)));
        assert_eq!(quality_param(&json!({"quality": 7.6}), "file.saveAs"), Ok(Some(8)));
        for bad in [json!(13), json!(-1), json!(12.1), json!("8"), json!(false), json!(1e300), json!(u64::MAX)] {
            assert!(quality_param(&json!({"quality": bad}), "file.saveAs").is_err(), "{bad}");
        }
    }
}
