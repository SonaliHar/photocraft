//! JPEG Options on Save As and Save a Copy (#5563): the quality it asks for, the scripted
//! `quality` that skips it, recorded actions, and the close prompt waiting behind it.
use super::*;
use crate::{FileDialogAnswer, jpeg_options_ui};
use egui_kittest::kittest::Queryable;
use std::{cell::RefCell, rc::Rc};

/// Each write: the path, and the JPEG quality (1–100) the export was asked for (as JSON).
type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The JSON a write records for a 0–12 `quality` (`None`: the export wasn't given one).
fn asked(quality: Option<u8>) -> Vec<u8> {
    serde_json::to_vec(&quality.map(jpeg_options_ui::to_percent)).unwrap()
}

/// One 8×8 RGB document whose exports record their JPEG quality, in a harness showing JPEG
/// Options and answering a save dialog with `to`.
fn jpeg_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(|_, _, settings| Ok((serde_json::to_vec(&settings.jpeg_quality).unwrap(), Vec::new()))));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            jpeg_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

/// The quality a new document's JPEG starts at: the encoder's default, as before the prompt.
fn default_quality() -> u8 {
    jpeg_options_ui::from_percent(photocraft_codecs::EncodeOptions::default().jpeg_quality)
}

/// Saves the frame to `$PHOTOCRAFT_JPEG_SNAPSHOTS/<name>.png` when set, to look at the prompt.
fn capture(h: &mut Prompted, name: &str) {
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_JPEG_SNAPSHOTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

/// Save As to a JPEG asks first, and writes only once answered, at the quality shown.
#[test]
fn save_as_to_a_jpeg_asks_for_its_quality() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    capture(&mut h, "jpeg-options");
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| (p.quality, p.copy)), Some((default_quality(), false)));
    assert!(h.query_by_label("JPEG Options").is_some());
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    assert!(h.state_mut().save_as(Some("other.jpg".into())).is_err(), "one save at a time");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("p.jpg".to_string(), asked(Some(default_quality())))]);
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("p.jpg"));
}

/// A preset sets its quality, the answer is the one written; Cancel writes nothing.
#[test]
fn jpeg_options_writes_the_choice_or_cancels() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    h.state_mut().jpeg_options.as_mut().unwrap().quality = 3;
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("p.jpg".to_string(), asked(Some(3)))]);

    h.state_mut().save_as(Some("c.jpg".into())).unwrap();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(writes.borrow().len(), 1, "cancelled: nothing more written");
    assert_eq!(h.state().ui.status, "Save cancelled");
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("p.jpg"));
}

/// Scripts choose with `quality` and are never asked; a bad one writes nothing, and other formats
/// aren't asked.
#[test]
fn scripted_quality_and_other_formats_skip_the_prompt() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.jpg", "quality": 6})).unwrap();
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(writes.borrow().as_slice(), &[("s.jpg".to_string(), asked(Some(6)))]);
    for bad in [json!(13), json!(-1), json!("6"), json!(true)] {
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.jpg", "quality": bad})).is_err(), "{bad}");
    }
    assert_eq!(writes.borrow().len(), 1, "a bad quality writes nothing");
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "p.png"})).unwrap();
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(writes.borrow()[1..], [("p.png".to_string(), asked(None))]);
}

/// Save a Copy to a JPEG asks too, and the copy leaves the document's path and unsaved state.
#[test]
fn save_a_copy_to_a_jpeg_asks_and_keeps_the_document_unsaved() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    make_dirty(h.state_mut(), 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| p.copy), Some(true));
    assert!(jpeg_options_ui::park(h.state_mut(), "other.jpg".into(), false, "file.saveAs").is_err(), "the waiting prompt is kept");
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| p.path.as_str()), Some("copy.jpg"));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.jpg".to_string(), asked(Some(default_quality())))]);
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty());
    assert_eq!(st.path, None);
}

/// An action being recorded keeps a JPEG Save As or Save a Copy with its quality, answered in JPEG
/// Options or scripted (#2032, #4373), and plays them back without asking.
#[test]
fn recorded_jpeg_save_keeps_its_quality_and_replays_without_asking() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    let ctx = h.ctx.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    h.state_mut().jpeg_options.as_mut().unwrap().quality = 4;
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().jpeg_options.as_mut().unwrap().quality = 12;
    h.key_press(Key::Enter);
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.jpg", "quality": 7})).unwrap();
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let expected = [("p.jpg".to_string(), asked(Some(4))), ("copy.jpg".to_string(), asked(Some(12))), ("s.jpg".to_string(), asked(Some(7)))];
    assert_eq!(writes.borrow().as_slice(), &expected);
    let steps = &h.state().session.actions.list[0].steps;
    assert_eq!(
        steps,
        &[
            ("file.saveAs".into(), json!({"path": "p.jpg", "quality": 4})),
            ("file.saveACopy".into(), json!({"path": "copy.jpg", "quality": 12})),
            ("file.saveAs".into(), json!({"path": "s.jpg", "quality": 7}))
        ]
    );
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().jpeg_options.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &expected);
}

/// Closing an unsaved document whose save goes to a JPEG waits behind JPEG Options, then closes.
#[test]
fn closing_after_a_jpeg_save_waits_for_jpeg_options() {
    let (mut h, writes) = jpeg_app("kept.jpg");
    // An edit that keeps the lone Background, so the JPEG holds the document and isn't a copy.
    h.state_mut().run("edit.fill", json!({"color": "#000000"})).unwrap();
    assert!(intercept(h.state_mut(), "file.close", &Value::Null));
    h.run_steps(2);
    h.key_press(Key::Y);
    h.run_steps(3);
    assert!(h.state().jpeg_options.is_some());
    assert_eq!(h.state().session.documents().len(), 1, "nothing closes before JPEG Options saves");
    assert!(h.query_by_label("(Y)es").is_none(), "the close prompt waits behind JPEG Options");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("kept.jpg".to_string(), asked(Some(default_quality())))]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty());
}
