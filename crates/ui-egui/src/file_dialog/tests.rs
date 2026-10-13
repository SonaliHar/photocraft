use super::*;
use crate::{Services, menus};
use std::cell::RefCell;
use std::rc::Rc;

/// The requests the fake dialog service was given, with their replies: dialogs the user is still in.
type Open = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

/// An app with an untitled document, whose file dialogs stay open until the test answers them, and
/// whose writer records the paths it wrote.
fn app() -> (PhotocraftApp, Open, Rc<RefCell<Vec<String>>>) {
    let (open, written): (Open, Rc<RefCell<Vec<String>>>) = Default::default();
    let (held, w) = (open.clone(), written.clone());
    let services = Services {
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, open, written)
}

/// The user answers the open dialog, from the dialog's own thread.
fn answer(open: &Open, answer: Option<FileDialogAnswer>) {
    let (_, reply) = open.borrow_mut().pop().expect("a dialog is open");
    std::thread::spawn(move || reply.send(answer)).join().unwrap();
}

#[test]
fn bounded_resource_failure_preserves_document_history_and_real_redo() {
    let (mut app, open, _) = app();
    app.session
        .edit("Redo seed", |document, _| {
            document.name = "Redo resource guard".into();
            Ok(())
        })
        .unwrap();
    let redo = app.session.active().unwrap().doc.clone();
    assert!(app.session.undo());
    let before = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    app.pick_xcf_link_bytes(|_, _, _| panic!("failure must not invoke the continuation")).unwrap();
    let ctx = egui::Context::default();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::XcfLinkedImage, _)]));
    answer(&open, Some(FileDialogAnswer::Failed("xcf.resource.read-failed".into())));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, "xcf.resource.read-failed");
    assert!(app.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert!(app.session.redo());
    assert!(app.session.active().unwrap().doc.shares_state(&redo));
}

#[test]
fn bounded_resource_cancel_is_distinct_from_disconnected_service() {
    for cancelled in [true, false] {
        let (mut app, open, _) = app();
        app.ui.status = "Unchanged status".into();
        app.ui.status_error = false;
        app.pick_xcf_link_bytes(|_, _, _| panic!("no answer must not invoke the continuation")).unwrap();
        let ctx = egui::Context::default();
        app.poll_file_dialog(&ctx, None);
        if cancelled {
            answer(&open, None);
        } else {
            open.borrow_mut().clear();
        }
        app.poll_file_dialog(&ctx, None);
        assert_eq!(app.ui.status_error, !cancelled);
        assert_eq!(app.ui.status, if cancelled { "Unchanged status" } else { "xcf.resource.picker-unavailable" });
    }
}

#[test]
fn bounded_resource_rejects_paths_and_oversized_service_bytes() {
    for response in [
        FileDialogAnswer::Paths(vec!["inert-stored-locator.png".into()]),
        FileDialogAnswer::SaveTo("unused.png".into()),
        FileDialogAnswer::Contents("empty.png".into(), Vec::new()),
        FileDialogAnswer::Contents("oversized.png".into(), vec![0; (8 << 20) + 1]),
    ] {
        let (mut app, open, _) = app();
        let before = app.session.active().unwrap().doc.clone();
        app.pick_xcf_link_bytes(|_, _, _| panic!("invalid answer must not reach the command")).unwrap();
        let ctx = egui::Context::default();
        app.poll_file_dialog(&ctx, None);
        answer(&open, Some(response));
        app.poll_file_dialog(&ctx, None);
        assert!(app.ui.status_error);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert!(!app.file_dialog_open());
    }
}

#[test]
fn bounded_resource_accepts_the_exact_transport_cap_without_mutating() {
    let (mut app, open, _) = app();
    let before = app.session.active().unwrap().doc.clone();
    let reached = Rc::new(std::cell::Cell::new(false));
    let callback = reached.clone();
    app.pick_xcf_link_bytes(move |_, _, bytes| {
        assert_eq!(bytes.len(), 8 << 20);
        callback.set(true);
        Ok(Value::Null)
    })
    .unwrap();
    let ctx = egui::Context::default();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Contents("boundary.png".into(), vec![0; 8 << 20])));
    app.poll_file_dialog(&ctx, None);
    assert!(reached.get());
    assert!(app.session.active().unwrap().doc.shares_state(&before));
}

#[test]
fn bounded_resource_missing_service_is_a_failure_while_open_remains_cancelled() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    assert_eq!(app.open_dialog_file().unwrap_err(), CANCELLED);
    assert_eq!(app.pick_xcf_link_bytes(|_, _, _| Ok(Value::Null)).unwrap_err(), "xcf.resource.picker-unavailable");
    assert!(!app.file_dialog_open());
}

#[test]
fn the_app_keeps_running_while_a_dialog_is_open() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap(), json!({"fileDialog": "save"}));
    assert!(open.borrow().is_empty(), "shown at the end of the frame, which has the window to parent it to");
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested.ends_with(".psd")));
    // Frames go on while the user is in the dialog: nothing is written, commands still run, and
    // a second dialog is refused.
    for _ in 0..3 {
        app.poll_file_dialog(&ctx, None);
    }
    assert!(written.borrow().is_empty());
    assert!(app.file_dialog_open());
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err(), "a file dialog is already open");
    app.poll_file_dialog(&ctx, None);
    assert_eq!(open.borrow().len(), 1);
    // The answer wakes the app, and the save goes on from where it asked.
    for _ in 0..4 {
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    }
    assert!(!ctx.has_requested_repaint(), "idle");
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    assert!(ctx.has_requested_repaint());
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/a.psd"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/a.psd"));
    assert!(!st.is_dirty());
    assert!(!app.file_dialog_open());
}

#[test]
fn the_answer_goes_to_the_document_it_was_asked_for() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.session.set_active(1);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/first.psd".into())));
    app.poll_file_dialog(&ctx, None);
    let paths: Vec<_> = app.session.documents().iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, [Some("/pics/first.psd".to_string()), None]);
    assert_eq!(app.session.active_index(), Some(0));
    // Once that document is closed, the answer has nothing to save.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.close", json!({"document": 0})).unwrap();
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/again.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/first.psd"]);
    assert!(app.ui.status_error && app.session.documents()[0].path.is_none());
}

#[test]
fn cancel_and_a_dialog_that_could_not_show_end_quietly() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    // A reply dropped unanswered (the dialog failed to show) is a Cancel, not a dialog left open forever.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    open.borrow_mut().clear();
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    assert!(written.borrow().is_empty());
    // Without a dialog service, asking is a Cancel straight away.
    app.services.file_dialog = None;
    assert_eq!(app.open_dialog_file().unwrap_err(), CANCELLED);
    assert!(!app.file_dialog_open());
}

#[test]
fn an_answer_of_the_wrong_kind_is_an_error_not_a_crash() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Paths(vec!["/pics/a.psd".into()])));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error && app.ui.status == UNEXPECTED);
    assert!(written.borrow().is_empty());
    app.open_dialog_file().unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.session.documents().len(), 1);
}
