//! Exact float literal controls; command execution owns renderer qualification.
//! No picker conversion, cache hash, pixel scan or source-path interpretation.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{DocId, LayerId, item_identity::PathId};
use photocraft_engine::{document_snapshot::WeakDocumentSnapshot, xcf_vector_style_cmds::current_color};
use serde_json::{Value, json};

const FILL: &str = "xcf.vector.fill.color.set";
const STROKE: &str = "xcf.vector.stroke.color.set";
const STALE: &str = "xcf.vector.color.ui.stale-target";
const LIMIT: &str = "xcf.vector.color.ui.limit";
struct Target {
    document: DocId,
    revision: u64,
    layer: LayerId,
    path: PathId,
    stroke: bool,
    original: [u32; 4],
    snapshot: WeakDocumentSnapshot,
}
fn context(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && !app.session.has_jobs()
        && app.session.active().is_some_and(|active| {
            active.floating.is_none()
                && active.doc.xcf_state.as_ref().is_none_or(|state| state.floating.is_none())
                && active.revision != u64::MAX
                && active.doc.accounted().is_none()
        })
}
fn capture(app: &PhotocraftApp, layer: LayerId, stroke: bool) -> Option<(Target, [f32; 4])> {
    let active = app.session.active()?;
    let (path, rgba) = current_color(&active.doc, layer, stroke)?;
    if path.0 == 0
        || layer.0 == 0
        || rgba.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || rgba.last().is_none_or(|alpha| alpha.to_bits() != 1.0_f32.to_bits())
    {
        return None;
    }
    Some((
        Target { document: active.doc.id, revision: active.revision, layer, path, stroke, original: rgba.map(f32::to_bits), snapshot: active.doc.downgrade() },
        rgba,
    ))
}
fn canonical_id(raw: u64) -> Result<String, String> {
    use std::fmt::Write;
    if raw == 0 {
        return Err(STALE.into());
    }
    let mut text = String::new();
    text.try_reserve_exact(20).map_err(|_| LIMIT)?;
    write!(&mut text, "{raw}").map_err(|_| LIMIT)?;
    Ok(text)
}
fn hex(bits: u32) -> Result<String, String> {
    use std::fmt::Write;
    let mut text = String::new();
    text.try_reserve_exact(8).map_err(|_| LIMIT)?;
    write!(&mut text, "{bits:08x}").map_err(|_| LIMIT)?;
    Ok(text)
}
fn params(layer: LayerId, path: PathId, rgba: [f32; 4]) -> Result<Value, String> {
    if rgba.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) || rgba.last().is_none_or(|alpha| alpha.to_bits() != 1.0_f32.to_bits()) {
        return Err("xcf.vector.color.ui.invalid-float".into());
    }
    let mut values = Vec::new();
    values.try_reserve_exact(4).map_err(|_| LIMIT)?;
    for value in rgba {
        values.push(Value::String(hex(value.to_bits())?));
    }
    Ok(json!({"layerId":canonical_id(layer.0)?,"pathId":canonical_id(path.0)?,"rgbaF32Bits":values}))
}
fn apply(app: &mut PhotocraftApp, target: &Target, rgba: [f32; 4]) -> Result<Value, String> {
    if !context(app) {
        return Err("xcf.vector.color.ui.context".into());
    }
    let active = app.session.active().ok_or(STALE)?;
    let original = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !original.shares_state(&active.doc) {
        return Err(STALE.into());
    }
    let (path, current) = current_color(&active.doc, target.layer, target.stroke).ok_or(STALE)?;
    if path != target.path || current.map(f32::to_bits) != target.original {
        return Err(STALE.into());
    }
    let params = params(target.layer, target.path, rgba)?;
    // No await, focus switch or direct mutation between the guard and command.
    app.run(if target.stroke { STROKE } else { FILL }, params)
}
fn parse_float(text: &str) -> Option<f64> {
    if text.len() > 64 {
        return None;
    }
    let value = text.parse::<f32>().ok()?;
    (value.is_finite() && (0.0..=1.0).contains(&value)).then(|| f64::from(value))
}
/// Existing localized headings/labels, no new view-state or localization bank.
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    for stroke in [false, true] {
        let Some((target, mut rgba)) = capture(app, layer, stroke) else { continue };
        let editable = context(app);
        ui.push_id(("xcf_vector_literal", layer.0, stroke), |ui| {
            ui.separator();
            ui.label(egui::RichText::new(if stroke { tl!("Stroke") } else { tl!("Fill") }).color(Tokens::get(ui.ctx()).text));
            ui.add_enabled_ui(editable, |ui| {
                for (value, label) in rgba.iter_mut().take(3).zip([tl!("Red"), tl!("Green"), tl!("Blue")]) {
                    ui.horizontal(|ui| {
                        let label = ui.label(egui::RichText::new(label).color(Tokens::get(ui.ctx()).text_dim));
                        ui.add(
                            egui::DragValue::new(value)
                                .speed(0.001)
                                .range(0.0..=1.0)
                                .clamp_existing_to_range(false)
                                .custom_parser(parse_float)
                                .custom_formatter(|value, _| (value as f32).to_string()),
                        )
                        .labelled_by(label.id);
                    });
                }
            });
        });
        // Compare raw words so untouched tiny values/signed zero do not traverse
        // an eight-bit or decimal rounding path. Opaque alpha is never editable.
        if editable
            && rgba.map(f32::to_bits) != target.original
            && let Err(error) = apply(app, &target, rgba)
        {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_parser_and_payload_retain_exact_float_words_without_byte_picker() {
        let values = [f32::from_bits(1), -0.0_f32, f32::from_bits(0x3eaa_aaab), 1.0];
        for value in values {
            assert_eq!((parse_float(&value.to_string()).unwrap() as f32).to_bits(), value.to_bits());
        }
        let request = params(LayerId(17), PathId(19), values).unwrap();
        assert_eq!(request["layerId"], "17");
        assert_eq!(request["pathId"], "19");
        assert_eq!(request["rgbaF32Bits"], json!(["00000001", "80000000", "3eaaaaab", "3f800000"]));
        for text in ["NaN", "inf", "-0.1", "1.0001", "not-a-number"] {
            assert!(parse_float(text).is_none());
        }
        assert!(params(LayerId(0), PathId(19), values).is_err());
        assert!(params(LayerId(17), PathId(0), values).is_err());
        assert!(params(LayerId(17), PathId(19), [0., 0., 0., 0.5]).is_err());
    }
    #[test]
    fn no_current_vector_produces_no_controls() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut document =
            photocraft_doc::Document::new("Own absent vector", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        document.layers.push(photocraft_doc::Layer::raster("Own ordinary raster", document.pixel_format()));
        let layer = document.layers.first().unwrap().id;
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(360., 260.)).build_ui_state(move |ui, app| properties(app, ui, layer), app);
        harness.run();
        assert!(harness.query_all_by_label("Red").next().is_none());
        assert!(harness.query_all_by_label("Green").next().is_none());
        assert!(harness.query_all_by_label("Blue").next().is_none());
    }
    fn retained_stroke_app() -> (PhotocraftApp, LayerId) {
        use photocraft_doc::{item_identity::ItemRef, xcf_items::EditableSource};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("retained qualified vector corpus"));
        let source = photocraft_io::import("source.xcf", &std::fs::read(root.join("vector-stroke-evidence/source.xcf")).unwrap()).unwrap().document;
        let owner = source
            .xcf_state
            .as_ref()
            .unwrap()
            .source_items
            .iter()
            .find(|item| matches!(item.source, EditableSource::Vector { source_modified: false, .. }))
            .unwrap()
            .owner;
        let ItemRef::Layer(layer) = owner else { panic!("qualified vector layer") };
        assert!(current_color(&source, layer, true).is_some());
        let mut session = photocraft_engine::Session::new();
        session.open_document(source, None).unwrap();
        (PhotocraftApp::new(session, crate::Services::default()), layer)
    }
    #[test]
    #[ignore = "retained qualified vector UI/native continuation draft; not executed"]
    fn actual_float_widget_dispatches_exact_registered_stroke_edit_with_native_undo_redo() {
        use egui_kittest::{Harness, kittest::Queryable};
        let (app, layer) = retained_stroke_app();
        let before = app.session.active().unwrap().doc.clone();
        let (_, color) = current_color(&before, layer, true).unwrap();
        let red = if color[0].to_bits() == 0.375_f32.to_bits() { 0.625_f32 } else { 0.375_f32 };
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let mut harness = Harness::builder().with_size(egui::vec2(360., 260.)).build_ui_state(move |ui, app| properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        for label in ["Stroke", "Red", "Green", "Blue"] {
            assert!(harness.query_all_by_label(label).next().is_some());
        }
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        assert_eq!(harness.state().session.active().unwrap().doc, before);
        harness.get_by_label("Red").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Red").type_text(&red.to_string());
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        let after = harness.state().session.active().unwrap().doc.clone();
        let (path, edited) = current_color(&after, layer, true).unwrap();
        let (original_path, original) = current_color(&before, layer, true).unwrap();
        assert_eq!(path, original_path);
        assert_eq!(edited.map(f32::to_bits), [red.to_bits(), original[1].to_bits(), original[2].to_bits(), original[3].to_bits()]);
        assert_eq!(after.paths, before.paths);
        assert_eq!(after.layers, before.layers);
        assert_eq!(after.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(after.xcf_state.as_ref().unwrap().source_items[0].cache, before.xcf_state.as_ref().unwrap().source_items[0].cache);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *after);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&harness.state().session.active().unwrap().doc, &Default::default()).unwrap(), native);
        harness.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(harness.state().session.active().unwrap().doc, after);
    }
    #[test]
    #[ignore = "retained qualified vector stale-target/native draft; not executed"]
    fn edit_then_undo_cannot_reuse_old_weak_target_or_clear_real_redo() {
        let (mut app, layer) = retained_stroke_app();
        let (target, mut rgba) = capture(&app, layer, true).unwrap();
        rgba[0] = if rgba[0].to_bits() == 0.375_f32.to_bits() { 0.625 } else { 0.375 };
        apply(&mut app, &target, rgba).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        assert!(apply(&mut app, &target, rgba).is_err());
        let active = app.session.active().unwrap();
        assert_eq!(active.revision, revision);
        assert!(active.history.can_redo());
        assert!(active.doc.shares_state(&before));
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), bytes);
    }
}
