//! Native viewport output and real child-key regressions for the hidden close prompt (#4551).
use super::*;

fn contains_text(shape: &egui::Shape, wanted: &str) -> bool {
    match shape {
        egui::Shape::Text(text) => text.galley.job.text == wanted,
        egui::Shape::Vec(shapes) => shapes.iter().any(|shape| contains_text(shape, wanted)),
        _ => false,
    }
}

fn has_prompt(output: &egui::FullOutput) -> bool {
    output.shapes.iter().any(|shape| contains_text(&shape.shape, "Unsaved changes"))
}

fn discard_key(h: &Harness) -> egui::Key {
    if h.ctx.os() == egui::os::OperatingSystem::Mac { egui::Key::D } else { egui::Key::N }
}

fn child_key(h: &mut Harness, backend: &BackendRef, id: egui::ViewportId, key: egui::Key, ppp: f32) -> egui::FullOutput {
    focus(h, backend, id, ppp);
    backend.borrow_mut().events.insert(id, vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
    h.step();
    let answer_pass = h.output().clone();
    h.run_steps(3);
    answer_pass
}

#[test]
fn dirty_close_prompt_is_drawn_in_the_affected_native_float() {
    let mut failures = Vec::new();
    for (theme, ppp) in cases() {
        eprintln!("4551-PROMPT-VISIBILITY BEGIN {} {ppp}", theme.id());
        let (mut h, backend, renderer, _integration) = harness(ppp, theme);
        documents(&mut h, &backend, false, ppp);
        h.state_mut().run("layer.new.layer", serde_json::json!({})).unwrap();
        let id = viewport(&h, 1);
        focus(&mut h, &backend, id, ppp);
        assert!(h.state().session.active().unwrap().is_dirty());
        let before = h.state().session.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect::<Vec<_>>();
        let windows = h.state().ui.windows.clone();
        backend.borrow_mut().close.insert(id);
        h.step();
        h.run_steps(3);
        assert!(h.state().discard.is_some(), "the real dirty-close request asks for confirmation");
        assert_eq!(h.state().session.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect::<Vec<_>>(), before);
        assert_eq!(h.state().ui.windows, windows);
        let child = backend.borrow().output.get(&id).map(has_prompt).expect("actual affected-child output");
        let root = has_prompt(h.output());
        capture(&mut h, &backend, &renderer, &format!("prompt-placement-{ppp}x"));
        if !child || root {
            failures.push(format!("{} {ppp}x: affected-child prompt={child}, root prompt={root}", theme.id()));
        }
        eprintln!("4551-PROMPT-VISIBILITY DONE {} {ppp} child={child} root={root}", theme.id());
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

#[test]
fn native_float_cancel_and_discard_answer_the_stable_document() {
    for (theme, ppp) in cases() {
        let (mut h, backend, renderer, _integration) = harness(ppp, theme);
        documents(&mut h, &backend, false, ppp);
        h.state_mut().run("layer.new.layer", serde_json::json!({})).unwrap();
        let id = viewport(&h, 1);
        let closing = h.state().session.documents()[1].doc.id;
        focus(&mut h, &backend, id, ppp);
        let before = h.state().session.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect::<Vec<_>>();
        let windows = h.state().ui.windows.clone();
        backend.borrow_mut().close.insert(id);
        h.step();
        h.run_steps(3);
        let _ = child_key(&mut h, &backend, id, egui::Key::Escape, ppp);
        capture(&mut h, &backend, &renderer, &format!("prompt-child-cancel-{ppp}x"));
        assert!(h.state().discard.is_none(), "the affected child's Escape cancels its prompt");
        assert_eq!(h.state().ui.windows, windows);
        assert_eq!(h.state().session.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect::<Vec<_>>(), before);
        backend.borrow_mut().close.insert(id);
        h.step();
        h.run_steps(3);
        h.state_mut().run("file.close", serde_json::json!({"document": 0})).unwrap();
        h.state_mut().run("file.new", serde_json::json!({"name": "Retained document", "width": 256, "height": 256, "background": "white"})).unwrap();
        h.run_steps(3);
        assert_eq!(h.state().session.documents()[0].doc.id, closing);
        let retained = h.state().session.documents()[1].doc.clone();
        let key = discard_key(&h);
        let _ = child_key(&mut h, &backend, id, key, ppp);
        assert!(h.state().discard.is_none());
        assert_eq!(h.state().session.documents().len(), 1);
        assert_eq!(h.state().session.documents()[0].doc, retained);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
        assert!(!h.state().ui.windows.iter().any(|w| crate::document_windows::viewport(w.id) == id));
        capture(&mut h, &backend, &renderer, &format!("prompt-child-discard-reorder-{ppp}x"));
    }
}

#[test]
fn quitting_from_child_confirmations_closes_the_root_viewport() {
    for (theme, ppp) in cases() {
        let (mut h, backend, renderer, _integration) = harness(ppp, theme);
        documents(&mut h, &backend, false, ppp);
        for document in 0..2 {
            h.state_mut().session.set_active(document);
            h.state_mut().run("layer.new.layer", serde_json::json!({})).unwrap();
        }
        let first = viewport(&h, 0);
        let second = viewport(&h, 1);
        focus(&mut h, &backend, egui::ViewportId::ROOT, ppp);
        h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        h.step();
        h.run_steps(3);
        assert!(h.state().discard.is_some() && !h.state().allow_close, "the native root-close request is held for dirty documents");
        let key = discard_key(&h);
        let _ = child_key(&mut h, &backend, first, key, ppp);
        assert_eq!(h.state().session.documents().len(), 1, "the first child answers its own document");
        assert!(h.state().discard.is_some() && !h.state().allow_close);
        capture(&mut h, &backend, &renderer, &format!("prompt-quit-second-{ppp}x"));
        let answer_pass = child_key(&mut h, &backend, second, key, ppp);
        assert!(h.state().session.documents().is_empty());
        assert!(h.state().discard.is_none() && h.state().allow_close);
        assert!(
            answer_pass.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::Close)),
            "the last child's answer must close ROOT, rather than merely its floating viewport"
        );
    }
}

#[test]
fn saving_from_a_native_child_waits_for_the_existing_save_service() {
    use std::sync::{Arc, Mutex};
    for (theme, ppp) in cases() {
        let (mut h, backend, renderer, _integration) = harness(ppp, theme);
        documents(&mut h, &backend, false, ppp);
        h.state_mut().run("layer.new.layer", serde_json::json!({})).unwrap();
        let id = viewport(&h, 1);
        let closing = h.state().session.documents()[1].doc.id;
        let retained = h.state().session.documents()[0].doc.clone();
        h.state_mut().session.active_mut().unwrap().path = Some("/pics/floating-confirmation.psd".into());
        h.state_mut().background_jobs = true;
        let written = Arc::new(Mutex::new(Vec::new()));
        let writes = written.clone();
        h.state_mut().services.save_file = Some(Arc::new(move |doc, path, _, _| {
            writes.lock().unwrap().push((doc.id, path.to_string()));
            Ok(Vec::new())
        }));
        focus(&mut h, &backend, id, ppp);
        backend.borrow_mut().close.insert(id);
        h.step();
        h.run_steps(3);
        let key = if h.ctx.os() == egui::os::OperatingSystem::Mac { egui::Key::S } else { egui::Key::Y };
        let _ = child_key(&mut h, &backend, id, key, ppp);
        let started = std::time::Instant::now();
        while h.state().saving() && started.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::yield_now();
            h.step();
        }
        h.run_steps(3);
        assert_eq!(
            *written.lock().unwrap(),
            [(closing, "/pics/floating-confirmation.psd".to_string())],
            "the child's Save reaches the normal service exactly once"
        );
        assert!(!h.state().saving() && h.state().discard.is_none());
        assert_eq!(h.state().session.documents().len(), 1);
        assert_eq!(h.state().session.documents()[0].doc, retained);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
        capture(&mut h, &backend, &renderer, &format!("prompt-child-save-{ppp}x"));
    }
}

#[test]
fn root_escape_does_not_reopen_a_pending_primary_close() {
    let mut failures = Vec::new();
    for (theme, ppp) in cases() {
        let (mut h, backend, renderer, _integration) = harness(ppp, theme);
        documents(&mut h, &backend, false, ppp);
        h.state_mut().run("layer.new.layer", serde_json::json!({})).unwrap();
        let id = viewport(&h, 1);
        focus(&mut h, &backend, id, ppp);
        backend.borrow_mut().close.insert(id);
        h.step();
        h.run_steps(3);
        assert!(h.state().discard.is_some(), "the first native close is held for the dirty document");
        let before = h.state().session.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect::<Vec<_>>();
        let windows = h.state().ui.windows.clone();
        focus(&mut h, &backend, egui::ViewportId::ROOT, ppp);
        h.input_mut().events.push(plain_key(egui::Key::Escape));
        backend.borrow_mut().close.insert(id);
        h.step();
        let cancel_close = backend
            .borrow()
            .output
            .values()
            .chain(std::iter::once(h.output()))
            .any(|output| output.viewport_output.get(&id).is_some_and(|view| view.commands.contains(&egui::ViewportCommand::CancelClose)));
        h.run_steps(3);
        let prompt_canceled = h.state().discard.is_none();
        assert_eq!(h.state().session.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect::<Vec<_>>(), before);
        assert_eq!(h.state().ui.windows, windows);
        capture(&mut h, &backend, &renderer, &format!("prompt-root-cancel-repeat-{ppp}x"));
        eprintln!("4551-PROMPT-ROOT-CANCEL {} {ppp} canceled={prompt_canceled} CancelClose={cancel_close}", theme.id());
        if !prompt_canceled || !cancel_close {
            failures.push(format!("{} {ppp}x: prompt canceled={prompt_canceled}, CancelClose={cancel_close}", theme.id()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
