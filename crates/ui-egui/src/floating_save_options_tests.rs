//! The real native-child Close → Save As → format-options path.
use super::*;

#[derive(Clone, Copy, Debug)]
enum Format {
    Tiff,
    Targa,
}
impl Format {
    fn path(self) -> &'static str {
        match self {
            Self::Tiff => "/pics/floating-kept.tif",
            Self::Targa => "/pics/floating-kept.tga",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Tiff => "TIFF Options",
            Self::Targa => "Targa Options",
        }
    }
    fn open(self, app: &PhotocraftApp) -> bool {
        match self {
            Self::Tiff => app.tiff_options.is_some(),
            Self::Targa => app.targa_options.is_some(),
        }
    }
    fn expected_settings(self) -> (bool, Option<u8>) {
        match self {
            Self::Tiff => (true, None),
            Self::Targa => (true, Some(24)),
        }
    }
}
type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn configure(h: &mut Harness, format: Format) -> (photocraft_doc::DocId, Writes) {
    match format {
        Format::Tiff => {
            h.state_mut().session.edit_prefs(|p| p.file_handling.ask_before_saving_layered_tiff = true);
            h.state_mut().run("layer.new.layer", serde_json::json!({})).unwrap();
        }
        // Keep one Background so a Targa Save As really saves, rather than writes a copy.
        Format::Targa => {
            h.state_mut().run("edit.fill", serde_json::json!({"color":"#000000"})).unwrap();
        }
    }
    let closing = h.state().session.active().unwrap().doc.id;
    assert!(h.state().session.active().unwrap().is_dirty());
    assert!(h.state().session.active().unwrap().path.is_none());
    let writes: Writes = Rc::default();
    let written = writes.clone();
    h.state_mut().services.file_dialog = Some(crate::file_dialog::fake(vec![Some(crate::FileDialogAnswer::SaveTo(format.path().into()))]).0);
    h.state_mut().services.export = Some(Box::new(|doc, _, settings| {
        Ok((serde_json::to_vec(&(doc.id, settings.tiff_layers, settings.tga_bits.map(photocraft_io::TgaBits::bits))).unwrap(), Vec::new()))
    }));
    h.state_mut().services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.into(), bytes.to_vec()));
        Ok(())
    }));
    (closing, writes)
}
fn key(h: &mut Harness, b: &BackendRef, id: egui::ViewportId, key: egui::Key, ppp: f32) {
    focus(h, b, id, ppp);
    b.borrow_mut().events.insert(id, vec![plain_key(key)]);
    h.step();
    h.run_steps(3);
}
fn close_and_save(h: &mut Harness, b: &BackendRef, id: egui::ViewportId, format: Format, ppp: f32) {
    focus(h, b, id, ppp);
    b.borrow_mut().close.insert(id);
    h.step();
    h.run_steps(3);
    assert!(h.state().discard.is_some(), "the actual dirty-close event opens the stock confirmation");
    let save = if h.ctx.os() == egui::os::OperatingSystem::Mac { egui::Key::S } else { egui::Key::Y };
    key(h, b, id, save, ppp);
    assert!(format.open(h.state()), "Save As must reach the actual {format:?} options");
    assert!(h.state().discard.is_some(), "close remains parked until the format save succeeds");
}
fn text(shape: &egui::Shape, wanted: &str) -> bool {
    match shape {
        egui::Shape::Text(t) => t.galley.job.text == wanted,
        egui::Shape::Vec(shapes) => shapes.iter().any(|s| text(s, wanted)),
        _ => false,
    }
}
fn has_text(output: &egui::FullOutput, wanted: &str) -> bool {
    output.shapes.iter().any(|shape| text(&shape.shape, wanted))
}
fn model(h: &Harness) -> Vec<(std::sync::Arc<photocraft_doc::Document>, usize)> {
    h.state().session.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect()
}

#[test]
fn floating_close_save_options_are_visible_only_in_the_affected_child() {
    let mut failures = Vec::new();
    for format in [Format::Tiff, Format::Targa] {
        for (theme, ppp) in cases() {
            let (mut h, b, r, _integration) = harness(ppp, theme);
            documents(&mut h, &b, false, ppp);
            let (_, writes) = configure(&mut h, format);
            let before = model(&h);
            let windows = h.state().ui.windows.clone();
            let id = viewport(&h, 1);
            close_and_save(&mut h, &b, id, format, ppp);
            assert_eq!(model(&h), before, "choosing the path neither edits nor closes a document");
            assert_eq!(h.state().ui.windows, windows);
            assert!(writes.borrow().is_empty());
            let child = has_text(b.borrow().output.get(&id).unwrap(), format.label());
            let root = has_text(h.output(), format.label());
            capture(&mut h, &b, &r, &format!("save-options-{format:?}-placement-{ppp}x"));
            eprintln!("4551-SAVE-OPTIONS {format:?} {} {ppp}x child={child} root={root}", theme.id());
            if !child || root {
                failures.push(format!("{format:?} {} {ppp}x child={child} root={root}", theme.id()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

#[test]
fn child_cancel_returns_to_the_same_unsaved_confirmation_without_a_write() {
    let mut failures = Vec::new();
    for format in [Format::Tiff, Format::Targa] {
        for (theme, ppp) in cases() {
            let (mut h, b, r, _integration) = harness(ppp, theme);
            documents(&mut h, &b, false, ppp);
            let (_, writes) = configure(&mut h, format);
            let before = model(&h);
            let id = viewport(&h, 1);
            close_and_save(&mut h, &b, id, format, ppp);
            key(&mut h, &b, id, egui::Key::Escape, ppp);
            capture(&mut h, &b, &r, &format!("save-options-{format:?}-cancel-{ppp}x"));
            assert!(h.state().discard.is_some(), "the original unsaved confirmation is still pending");
            assert_eq!(model(&h), before);
            assert!(writes.borrow().is_empty());
            if format.open(h.state()) || !has_text(b.borrow().output.get(&id).unwrap(), "Unsaved changes") {
                failures.push(format!("{format:?} {} {ppp}x child Cancel must return to the original unsaved confirmation", theme.id()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

#[test]
fn child_accept_writes_and_closes_the_stable_document_after_tabs_reorder() {
    let mut failures = Vec::new();
    for format in [Format::Tiff, Format::Targa] {
        for (theme, ppp) in cases() {
            let (mut h, b, r, _integration) = harness(ppp, theme);
            documents(&mut h, &b, false, ppp);
            let (closing, writes) = configure(&mut h, format);
            let id = viewport(&h, 1);
            close_and_save(&mut h, &b, id, format, ppp);
            // A direct engine/control close shifts the parked document's index; a new tab
            // becomes active. Neither action may retarget this already-visible options prompt.
            h.state_mut().run("file.close", serde_json::json!({"document":0})).unwrap();
            h.state_mut().run("file.new", serde_json::json!({"name":"Retained after reorder","width":16,"height":16,"background":"white"})).unwrap();
            h.run_steps(3);
            assert_eq!(h.state().session.documents()[0].doc.id, closing);
            let retained = h.state().session.documents()[1].doc.clone();
            let retained_past = h.state().session.documents()[1].history.past_len();
            key(&mut h, &b, id, egui::Key::Enter, ppp);
            capture(&mut h, &b, &r, &format!("save-options-{format:?}-accepted-{ppp}x"));
            let (layers, bits) = format.expected_settings();
            let saved = writes.borrow().as_slice() == [(format.path().to_owned(), serde_json::to_vec(&(closing, layers, bits)).unwrap())];
            let completed = !format.open(h.state()) && h.state().discard.is_none();
            let docs = h.state().session.documents();
            let kept = docs.len() == 1 && docs[0].doc == retained && docs[0].history.past_len() == retained_past;
            if !saved || !completed || !kept {
                failures.push(format!(
                    "{format:?} {} {ppp}x saved={saved},completed={completed},retained_doc_exact={kept},writes={}",
                    theme.id(),
                    writes.borrow().len()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

#[test]
fn direct_format_options_block_child_input_until_cancel_then_editing_resumes() {
    let mut failures = Vec::new();
    for format in [Format::Tiff, Format::Targa] {
        for (theme, ppp) in cases() {
            let (mut h, b, r, _integration) = harness(ppp, theme);
            documents(&mut h, &b, false, ppp);
            let (_, writes) = configure(&mut h, format);
            let id = viewport(&h, 1);
            pencil(&mut h);
            focus(&mut h, &b, id, ppp);
            h.run_steps(2);
            let xf = canvas::ViewXform::active(h.state()).unwrap();
            let before_past = h.state().session.documents()[1].history.past_len();
            for events in stroke(xf.to_screen(78.0, 78.0), xf.to_screen(103.0, 78.0)) {
                b.borrow_mut().events.insert(id, events);
                h.step();
            }
            h.run_steps(3);
            let st = &h.state().session.documents()[1];
            assert_eq!(st.history.past_len(), before_past + 1, "positive native pointer control commits one stroke");
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(90, 78), [1.0, 0.0, 0.0, 1.0]);
            h.state_mut().save_as(Some(format.path().into())).unwrap();
            h.run_steps(3);
            assert!(format.open(h.state()) && h.state().discard.is_none(), "direct Save As also has a native options modal");
            let before = model(&h);
            for events in stroke(xf.to_screen(78.0, 120.0), xf.to_screen(103.0, 120.0)) {
                b.borrow_mut().events.insert(id, events);
                h.step();
            }
            h.run_steps(3);
            b.borrow_mut().events.insert(id, vec![command_key(egui::Key::J)]);
            h.step();
            h.run_steps(3);
            capture(&mut h, &b, &r, &format!("save-options-{format:?}-blocked-paint-{ppp}x"));
            assert!(writes.borrow().is_empty());
            if model(&h) != before {
                failures.push(format!("{format:?} {} {ppp}x pointer/keyboard edited beneath the format options", theme.id()));
                continue;
            }
            key(&mut h, &b, id, egui::Key::Escape, ppp);
            assert!(!format.open(h.state()) && h.state().discard.is_none());
            for events in stroke(xf.to_screen(78.0, 160.0), xf.to_screen(103.0, 160.0)) {
                b.borrow_mut().events.insert(id, events);
                h.step();
            }
            h.run_steps(3);
            let st = &h.state().session.documents()[1];
            assert_eq!(st.history.past_len(), before[1].1 + 1, "editing resumes after Cancel");
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(90, 160), [1.0, 0.0, 0.0, 1.0]);
            assert_eq!(h.state().session.documents()[0].doc, before[0].0);
            let layers = st.doc.layers.len();
            let past = st.history.past_len();
            b.borrow_mut().events.insert(id, vec![command_key(egui::Key::J)]);
            h.step();
            h.run_steps(3);
            assert_eq!(h.state().session.documents()[1].doc.layers.len(), layers + 1, "native keyboard editing resumes after Cancel");
            assert_eq!(h.state().session.documents()[1].history.past_len(), past + 1, "one native key performs one edit");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

#[test]
fn root_enter_does_not_accept_hidden_child_options_but_child_enter_saves() {
    let mut failures = Vec::new();
    for format in [Format::Tiff, Format::Targa] {
        for (theme, ppp) in cases() {
            let (mut h, b, r, _integration) = harness(ppp, theme);
            documents(&mut h, &b, false, ppp);
            let (closing, writes) = configure(&mut h, format);
            let id = viewport(&h, 1);
            close_and_save(&mut h, &b, id, format, ppp);
            let before = model(&h);
            let windows = h.state().ui.windows.clone();
            focus(&mut h, &b, egui::ViewportId::ROOT, ppp);
            h.input_mut().events.push(plain_key(egui::Key::Enter));
            h.step();
            h.run_steps(3);
            capture(&mut h, &b, &r, &format!("save-options-{format:?}-root-enter-blocked-{ppp}x"));
            let blocked =
                writes.borrow().is_empty() && format.open(h.state()) && h.state().discard.is_some() && model(&h) == before && h.state().ui.windows == windows;
            if !blocked {
                failures.push(format!("{format:?} {} {ppp}x ROOT Enter accepted hidden child options", theme.id()));
                continue;
            }
            key(&mut h, &b, id, egui::Key::Enter, ppp);
            let (layers, bits) = format.expected_settings();
            assert_eq!(
                writes.borrow().as_slice(),
                [(format.path().to_owned(), serde_json::to_vec(&(closing, layers, bits)).unwrap())],
                "the visible child still accepts exactly one save"
            );
            assert!(!format.open(h.state()) && h.state().discard.is_none());
            assert_eq!(model(&h), before[..1]);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
