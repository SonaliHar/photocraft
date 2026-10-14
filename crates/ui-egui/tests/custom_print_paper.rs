//! #3820: edit custom paper through the real Print dialog, preview it, and print safely to PDF.
use egui::{Event, Key, Modifiers, PointerButton, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, file_ui::open_print};
use serde_json::{Map, Value, json};

fn app() -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80, "name": "Custom paper"})).unwrap();
    let id = open_print(&mut app);
    // Every UI path in these fixtures is safe even if an unexpected Enter confirms the dialog.
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    fields.insert("dryRun".into(), json!(true));
    // Host printer drivers can add arbitrary rows and scroll the controls these tests edit.
    // Exercise deterministic paper/position controls; driver choice validation has a unit test.
    fields.insert("__printers".into(), json!([]));
    fields.insert("__nativePrinters".into(), json!([]));
    fields.insert("__driverOptions".into(), json!([]));
    fields.insert("driverOptions".into(), json!({}));
    fields.insert("printer".into(), json!(""));
    (app, id)
}

fn show(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
        photocraft_ui_egui::dialogs::show(app, ui.ctx());
    }
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    harness_size(app, vec2(1000.0, 900.0))
}

fn harness_size(app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(size).with_max_steps(64).build_ui_state(show, app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run();
    h
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.run();
    if let Some(node) = h.query_by_label(label) {
        node.click();
    } else {
        h.get_by_value(label).click();
    }
    h.run();
}

fn shown(h: &Harness<'_, PhotocraftApp>, text: &str) -> bool {
    h.query_by_label(text).is_some() || h.query_by_value(text).is_some()
}

fn enter_length(h: &mut Harness<'_, PhotocraftApp>, label: &str, value: &str) {
    h.run();
    // Anchor the pointer to the real label; the existing length widget immediately follows it.
    let pos = h.get_by_label(label).rect().right_center() + vec2(35.0, 0.0);
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(value.into()));
    h.run_steps(1);
    h.key_press(Key::Tab);
    h.run();
}

fn fields(h: &Harness<'_, PhotocraftApp>, id: u64) -> Map<String, Value> {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

#[test]
fn custom_paper_ui_edits_reach_preview_and_pdf_and_restore_when_reopened() {
    let (mut app, id) = app();
    let output = std::env::temp_dir().join(format!("pc-custom-paper-{}.pdf", std::process::id()));
    let dialog = app.ui.dialog_mut(id).unwrap();
    dialog.fields.insert("output".into(), json!(output.to_string_lossy()));
    dialog.fields.insert("dryRun".into(), json!(true));
    let mut h = harness(app);
    click(&mut h, "letter");
    assert!(shown(&h, "a4"), "the real preset menu must be open");
    assert!(shown(&h, "Custom"), "the paper menu must offer editable custom dimensions");
    click(&mut h, "Custom");
    assert_eq!(fields(&h, id)["paper"], json!([612.0, 792.0]), "Custom starts at the current preset dimensions");
    click(&mut h, "Centimeters");
    click(&mut h, "Inches");
    enter_length(&mut h, "Paper Width", "7.25");
    enter_length(&mut h, "Paper Height", "9.25");
    let edited = fields(&h, id);
    assert_eq!(edited["paper"], json!([522.0, 666.0]), "the engine takes points, not the displayed units");
    click(&mut h, "Custom");
    click(&mut h, "a4");
    assert_eq!(fields(&h, id)["paper"], "a4");
    assert!(!shown(&h, "Paper Width"));
    click(&mut h, "a4");
    click(&mut h, "Custom");
    assert_eq!(fields(&h, id)["paper"], edited["paper"], "returning from a preset restores the custom dimensions");
    click(&mut h, "Inches");
    click(&mut h, "Millimeters");
    assert_eq!(fields(&h, id)["paper"], edited["paper"], "changing units must not change the physical paper");
    click(&mut h, "Millimeters");
    click(&mut h, "Inches");
    assert_eq!(fields(&h, id)["paper"], edited["paper"]);
    let preview = photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(edited.clone()), "file.print").unwrap();
    assert_eq!(preview.paper, (522.0, 666.0));
    click(&mut h, "Portrait");
    click(&mut h, "Landscape");
    let edited = fields(&h, id);
    assert_eq!(edited["paper"], json!([522.0, 666.0]), "orientation must not rewrite the unrotated custom dimensions");
    let preview = photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(edited), "file.print").unwrap();
    assert_eq!(preview.paper, (666.0, 522.0));
    click(&mut h, "Print");
    let pdf = std::fs::read(&output).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 666.00 522.00]"));
    assert_eq!(h.state().session.file_menu.last_print.as_ref().unwrap()["paper"], json!([522.0, 666.0]));
    let id = open_print(h.state_mut());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("dryRun".into(), json!(true));
    h.run_steps(3);
    assert!(shown(&h, "Custom"), "reopening array-valued settings must show Custom");
    assert_eq!(fields(&h, id)["paper"], json!([522.0, 666.0]));
    std::fs::remove_file(output).unwrap();
}

#[test]
fn remembered_custom_paper_is_shown_without_becoming_a_preset() {
    let (mut app, id) = app();
    app.ui.dialog_mut(id).unwrap().fields.insert("paper".into(), json!([432.0, 576.0]));
    let h = harness(app);
    assert!(shown(&h, "Custom"), "an array paper size must not display as letter");
    assert!(shown(&h, "Paper Width"));
    assert!(shown(&h, "Paper Height"));
    assert_eq!(fields(&h, id)["paper"], json!([432.0, 576.0]));
}

#[test]
fn invalid_custom_paper_is_rejected_without_printing_or_changing_the_document() {
    let (mut app, id) = app();
    let output = std::env::temp_dir().join(format!("pc-invalid-custom-paper-{}.pdf", std::process::id()));
    app.ui.dialog_mut(id).unwrap().fields.insert("output".into(), json!(output.to_string_lossy()));
    let before = app.session.active().unwrap().doc.clone();
    let mut h = harness(app);
    click(&mut h, "letter");
    click(&mut h, "Custom");
    enter_length(&mut h, "Paper Width", "0");
    let edited = fields(&h, id);
    assert!(photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(edited), "file.print").is_err());
    click(&mut h, "Print");
    let error = &h.state().ui.status;
    assert!(error.contains("paper is too small"), "{error}");
    assert_eq!(h.state().session.active().unwrap().doc, before);
    assert!(h.state().session.file_menu.last_print.is_none());
    assert!(!output.exists());
}

#[test]
fn disabling_center_preserves_position_and_preview_drag_updates_print_coordinates() {
    use egui::accesskit::Role;
    for (mirrored, driver_rows) in [(false, 0), (true, 0), (false, 4), (true, 4)] {
        let (mut app, id) = app();
        app.ui.dialog_mut(id).unwrap().fields.insert("emulsionDown".into(), json!(mirrored));
        let options: Vec<_> =
            (0..driver_rows).map(|i| json!({"key": format!("Option{i}"), "label": format!("Driver option {i}"), "choices": [["auto", "Automatic"]]})).collect();
        app.ui.dialog_mut(id).unwrap().fields.insert("__driverOptions".into(), json!(options));
        let doc_before = app.session.active().unwrap().doc.clone();
        let mut h = harness(app);
        let centered = photocraft_engine::print_cmds::layout(&doc_before, &Value::Object(fields(&h, id)), "file.print").unwrap();
        click(&mut h, "Center");
        let before = fields(&h, id);
        let positioned = photocraft_engine::print_cmds::layout(&doc_before, &Value::Object(before.clone()), "file.print").unwrap();
        assert_eq!(centered.rect, positioned.rect, "turning off Center must not jump to the page corner");
        h.run();
        let start = h.get_by_role_and_label(Role::Image, "Position and Size").rect().center();
        h.event(Event::PointerMoved(start));
        h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerMoved(start + vec2(12.0, 8.0)));
        h.run_steps(2);
        let during = fields(&h, id);
        h.run_steps(3);
        assert_eq!(fields(&h, id)["left"], during["left"], "holding still must not accumulate drag offsets");
        let end = start + vec2(24.0, 16.0);
        h.event(Event::PointerMoved(end));
        h.run_steps(2);
        h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        let after = fields(&h, id);
        let dx = after["left"].as_f64().unwrap() - before["left"].as_f64().unwrap();
        let dy = after["top"].as_f64().unwrap() - before["top"].as_f64().unwrap();
        assert!(if mirrored { dx < 0.0 } else { dx > 0.0 }, "{before:?} -> {after:?}");
        assert!(dy > 0.0);
        assert!((dx.abs() / dy - 1.5).abs() < 0.001, "drag uses page scale in both axes");
        assert_eq!(after["scale"], before["scale"]);
        assert_eq!(h.state().session.active().unwrap().doc, doc_before);
    }
}

#[test]
fn print_resolution_stays_correct_after_rotation_and_scale_changes() {
    let (mut app, id) = app();
    app.ui.dialog_mut(id).unwrap().fields.insert("scale".into(), json!(50.0));
    let mut h = harness(app);
    assert!(shown(&h, "Print Resolution: 144 PPI"));
    click(&mut h, "Auto-Rotate Image");
    assert!(shown(&h, "Print Resolution: 144 PPI"));
    enter_length(&mut h, "Scale", "200");
    assert!(shown(&h, "Print Resolution: 36 PPI"));
    assert_eq!(fields(&h, id)["scale"], json!(200.0));
}

#[test]
fn scale_to_fit_keeps_manual_size_fields_disabled() {
    let (mut app, id) = app();
    app.ui.dialog_mut(id).unwrap().fields.insert("scaleToFit".into(), json!(true));
    let mut h = harness(app);
    let before = fields(&h, id);
    enter_length(&mut h, "Width", "1");
    assert_eq!(fields(&h, id)["scaleToFit"], before["scaleToFit"]);
    assert_eq!(fields(&h, id)["scale"], before["scale"]);
}

#[test]
fn print_footer_stays_inside_short_windows() {
    let (mut app, id) = app();
    let options: Vec<_> =
        (0..20).map(|i| json!({"key": format!("Option{i}"), "label": format!("Driver option {i}"), "choices": [["auto", "Automatic"]]})).collect();
    app.ui.dialog_mut(id).unwrap().fields.insert("__driverOptions".into(), json!(options));
    let h = harness_size(app, vec2(800.0, 480.0));
    for label in ["Print", "Cancel"] {
        let rect = h.get_by_label(label).rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= 480.0, "{label}: {rect:?}");
    }
}

#[test]
fn grayscale_rebuilds_the_print_preview_texture() {
    let (app, _) = app();
    let doc = app.session.active().unwrap();
    let (document, revision) = (doc.doc.id.0, doc.revision);
    let mut h = harness(app);
    let colour = egui::Id::new(("print-thumb", document, revision, false));
    let gray = egui::Id::new(("print-thumb", document, revision, true));
    assert!(h.ctx.data(|data| data.get_temp::<std::sync::Arc<egui::TextureHandle>>(colour)).is_some());
    assert!(h.ctx.data(|data| data.get_temp::<std::sync::Arc<egui::TextureHandle>>(gray)).is_none());
    click(&mut h, "Print in Grayscale");
    assert!(h.ctx.data(|data| data.get_temp::<std::sync::Arc<egui::TextureHandle>>(gray)).is_some());
}
