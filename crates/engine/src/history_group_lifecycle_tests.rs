//! The ordinary resident history merge cursor is independent of model/history custody.
use super::*;
use serde_json::json;
fn grouped() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 3})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Own group", "coalesce": "own-history-group"})).unwrap();
    s.execute("layer.setProps", json!({"name": "Completed group", "coalesce": "own-history-group"})).unwrap();
    assert_eq!(s.active().unwrap().coalesce.as_deref(), Some("own-history-group"));
    s
}
#[test]
fn exact_group_finish_preserves_snapshot_revision_history_redo_and_journal() {
    let mut s = grouped();
    let state = s.active().unwrap();
    let doc = state.doc.clone();
    let revision = state.revision;
    let past = state.history.past_len();
    let redo = state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
    let journal = s.journal.clone();
    assert!(s.finish_history_group(doc.id, "own-history-group").unwrap());
    let state = s.active().unwrap();
    assert!(state.doc.shares_state(&doc));
    assert_eq!((state.revision, state.history.past_len()), (revision, past));
    assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), redo);
    assert_eq!(s.journal, journal);
    assert!(state.coalesce.is_none());
    assert!(!s.finish_history_group(doc.id, "own-history-group").unwrap());
    s.execute("layer.setProps", json!({"name": "New group", "coalesce": "own-history-group"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "a completed key cannot merge the next gesture");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(&*s.active().unwrap().doc, &*doc);
}
#[test]
fn wrong_key_document_and_invalid_key_never_clear_the_actual_group() {
    let mut s = grouped();
    let doc = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let past = s.active().unwrap().history.past_len();
    let other = photocraft_doc::Document::new("Other", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8).id;
    for (id, key) in [(other, "own-history-group"), (doc.id, "other-key"), (doc.id, ""), (doc.id, "bad\0key")] {
        assert!(s.finish_history_group(id, key).is_err());
        let state = s.active().unwrap();
        assert!(state.doc.shares_state(&doc));
        assert_eq!((state.revision, state.history.past_len()), (revision, past));
        assert_eq!(state.coalesce.as_deref(), Some("own-history-group"));
    }
    s.execute("edit.undo", json!({})).unwrap();
    let state = s.active().unwrap();
    let snapshot = state.doc.clone();
    let redo = state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
    let revision = state.revision;
    assert!(!s.finish_history_group(snapshot.id, "own-history-group").unwrap());
    assert!(s.active().unwrap().doc.shares_state(&snapshot));
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.active().unwrap().history.redo_labels().collect::<Vec<_>>(), redo);
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn active_job_refuses_group_finish_without_model_or_history_changes() {
    let mut s = grouped();
    let state = s.active().unwrap();
    let doc = state.doc.clone();
    let revision = state.revision;
    let past = state.history.past_len();
    let (release, wait) = std::sync::mpsc::sync_channel::<()>(0);
    let job = s
        .start_job(
            "own.lifecycle",
            json!({}),
            "Own held work",
            true,
            move |_| {
                wait.recv().map_err(|e| EngineError::Other(e.to_string()))?;
                Ok(())
            },
            |_, ()| Ok(serde_json::Value::Null),
        )
        .unwrap();
    let refused = s.finish_history_group(doc.id, "own-history-group");
    release.send(()).unwrap();
    let crate::jobs::Started::Job(id) = job else { panic!("native job must run as a job") };
    s.wait_job(id).unwrap();
    assert!(refused.is_err());
    let state = s.active().unwrap();
    assert!(state.doc.shares_state(&doc));
    assert_eq!((state.revision, state.history.past_len()), (revision, past));
    assert_eq!(state.coalesce.as_deref(), Some("own-history-group"));
}
