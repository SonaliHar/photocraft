//! UNRUN genuine owned-XCF UI save-route drafts. No filesystem writes.
use super::*;
use crate::{PhotocraftApp, Services};
use photocraft_engine::Session;
use photocraft_raster::{Interrupt, backing::ReadControl};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn app(services: Services) -> PhotocraftApp {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "original.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "original.xcf", Some("authorized/original.xcf"), &control()).unwrap();
    PhotocraftApp::new(session, services)
}

#[test]
fn actual_owned_import_save_as_prepares_identity_and_one_record_then_notifies() {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let observed = writes.clone();
    let mut app = app(Services {
        accounted_native_save: Some(Box::new(move |snapshot, path, protection, ctl| {
            ctl.check(1).map_err(|e| e.to_string())?;
            assert_eq!(snapshot.name, "own.pcraft");
            assert_eq!(path, "own.pcraft");
            assert!(protection.sources.iter().any(|p| p == "authorized/original.xcf"));
            observed.borrow_mut().push(path.to_string());
            Ok(())
        })),
        native_save: Some(Box::new(|_, _, _| panic!("resident writer reached"))),
        ..Default::default()
    });
    let before = app.session.active().unwrap().doc.clone();
    app.session.prefs.edit(|p| {
        p.script_events = photocraft_engine::automate_cmds::ScriptEvents {
            enabled: true,
            bindings: vec![photocraft_engine::automate_cmds::ScriptBinding {
                event: "saveDocument".into(),
                name: "owned save notification".into(),
                steps: Some(serde_json::json!([])),
                script: None,
            }],
        }
    });
    let source = app.session.active().unwrap().source.clone();
    let count = app.session.journal.len();
    let undo = app.session.active().unwrap().history.can_undo();
    let result = app.save_to("own.pcraft".into(), None).unwrap();
    assert_eq!(result, serde_json::json!({"path":"own.pcraft","warnings":[]}));
    assert_eq!(writes.borrow().as_slice(), &["own.pcraft"]);
    let state = app.session.active().unwrap();
    assert_eq!(state.path.as_deref(), Some("own.pcraft"));
    assert_eq!(state.doc.name, "own.pcraft");
    assert_eq!(state.saved_revision, state.revision);
    assert_eq!(state.source, source);
    assert!(state.source_read_only);
    assert_eq!(state.doc.source_color_context, before.source_color_context);
    assert_eq!(state.doc.xcf_state, before.xcf_state);
    assert_eq!(state.history.can_undo(), undo);
    assert_eq!(app.session.journal.len(), count + 1);
    assert_eq!(app.session.journal.last().unwrap(), &("file.saveAs".into(), serde_json::json!({"path":"own.pcraft"})));
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("own.pcraft"));
    assert!(app.ui.status.contains("Saved"));
    assert!(!app.ui.status_error);
    assert_eq!(app.session.file_menu.event_log.last().unwrap()["event"], "saveDocument");
}

#[test]
fn failed_writer_preserves_actual_identity_history_journal_and_quota() {
    let mut app = app(Services { accounted_native_save: Some(Box::new(|_, _, _, _| Err("atomic writer refused".into()))), ..Default::default() });
    let state = app.session.active().unwrap();
    let (before, source, path, saved) = (state.doc.clone(), state.source.clone(), state.path.clone(), state.saved_revision);
    let quota = before.quota().unwrap().clone();
    let baseline = quota.usage().unwrap();
    let journal = app.session.journal.clone();
    assert_eq!(app.save_to("failed.pcraft".into(), None).unwrap_err(), "atomic writer refused");
    let state = app.session.active().unwrap();
    assert!(before.shares_state(&state.doc));
    assert_eq!((state.source.clone(), state.path.clone(), state.saved_revision), (source, path, saved));
    assert_eq!(app.session.journal, journal);
    assert_eq!(quota.usage().unwrap(), baseline);
    assert!(app.ui.recent_files.is_empty());
}

#[test]
fn accounted_automation_never_falls_back_to_interactive_writer() {
    let mut app = app(Services { accounted_native_save: Some(Box::new(|_, _, _, _| panic!("ambient authority reached"))), ..Default::default() });
    let before = app.session.active().unwrap().doc.clone();
    let journal = app.session.journal.clone();
    assert!(app.save_automation(Some("rooted.pcraft".into())).unwrap_err().contains("authority is not configured"));
    assert!(before.shares_state(&app.session.active().unwrap().doc));
    assert_eq!(app.session.journal, journal);
}

#[test]
fn accounted_copy_uses_original_snapshot_without_rename_clean_record_or_recent() {
    let mut app = app(Services {
        accounted_native_save: Some(Box::new(|snapshot, _, _, _| {
            assert_eq!(snapshot.name, "original.xcf");
            Ok(())
        })),
        ..Default::default()
    });
    let before = app.session.active().unwrap().doc.clone();
    app.session.prefs.edit(|p| {
        p.script_events = photocraft_engine::automate_cmds::ScriptEvents {
            enabled: true,
            bindings: vec![photocraft_engine::automate_cmds::ScriptBinding {
                event: "saveDocument".into(),
                name: "copy must not notify".into(),
                steps: Some(serde_json::json!([])),
                script: None,
            }],
        }
    });
    let journal = app.session.journal.clone();
    let state = app.session.active().unwrap();
    let (path, saved) = (state.path.clone(), state.saved_revision);
    app.write_document("copy.pcraft".into(), &Default::default(), true).unwrap();
    let state = app.session.active().unwrap();
    assert!(before.shares_state(&state.doc));
    assert_eq!((state.path.clone(), state.saved_revision), (path, saved));
    assert_eq!(app.session.journal, journal);
    assert!(app.ui.recent_files.is_empty());
    assert!(app.session.file_menu.event_log.is_empty());
}

#[test]
fn all_open_source_exclusions_are_present_before_writer_and_source_alias_refuses() {
    let mut app = app(Services {
        accounted_native_save: Some(Box::new(|_, _, p, _| {
            assert_eq!(p.sources, ["authorized/other.pcraft"]);
            Ok(())
        })),
        ..Default::default()
    });
    app.session.active_mut().unwrap().source.as_mut().unwrap().path = Some("authorized/other.pcraft".into());
    let before = app.session.active().unwrap().doc.clone();
    assert!(app.save_to("authorized/other.pcraft".into(), None).is_err());
    assert!(before.shares_state(&app.session.active().unwrap().doc));
    app.save_to("own.pcraft".into(), None).unwrap();
}
