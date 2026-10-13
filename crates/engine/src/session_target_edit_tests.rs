//! Deferred regressions for target-scoped commits and revision exhaustion.
use super::*;
use photocraft_raster::Interrupt;
use serde_json::json;
use std::cell::Cell;

fn two_documents() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"name":"target","width":2,"height":2})).unwrap();
    session.execute("file.new", json!({"name":"source","width":2,"height":2})).unwrap();
    session
}

#[test]
fn target_commit_keeps_focus_and_source_and_creates_destination_undo() {
    let mut session = two_documents();
    let source = session.documents()[1].doc.clone();
    let target = session.documents()[0].doc.clone();
    let revision = session.documents()[0].revision;
    session.edit_document_with_interrupt(0, "Rename target", &Interrupt::NONE, |document, _| {
        document.name = "changed target".into();
        Ok(())
    }).unwrap();
    assert_eq!(session.active_index(), Some(1));
    assert!(session.documents()[1].doc.shares_state(&source));
    assert_eq!(session.documents()[0].revision, revision + 1);
    assert_eq!(session.documents()[0].doc.name, "changed target");
    assert!(session.set_active(0));
    assert!(session.undo());
    assert!(session.active().unwrap().doc.shares_state(&target));
}

#[test]
fn failed_or_late_cancelled_target_commit_preserves_both_documents_and_focus() {
    for cancel in [false, true] {
        let mut session = two_documents();
        let target = session.documents()[0].doc.clone();
        let source = session.documents()[1].doc.clone();
        let revision = session.documents()[0].revision;
        let edited = Cell::new(false);
        let signal = || cancel && edited.get();
        let control = Interrupt::cancel_only(&signal);
        let result: Result<()> = session.edit_document_with_interrupt(0, "Private target", &control, |document, _| {
            document.name = "discard this private stage".into();
            edited.set(true);
            if cancel { Ok(()) } else { Err(EngineError::Other("synthetic refusal".into())) }
        });
        assert!(result.is_err());
        assert_eq!(session.active_index(), Some(1));
        assert!(session.documents()[0].doc.shares_state(&target));
        assert!(session.documents()[1].doc.shares_state(&source));
        assert_eq!(session.documents()[0].revision, revision);
        assert!(!session.documents()[0].history.can_undo());
    }
}

#[test]
fn exhausted_revision_refuses_before_callback_without_overflow() {
    let mut session = two_documents();
    session.docs.get_mut(0).unwrap().revision = u64::MAX;
    let target = session.documents()[0].doc.clone();
    let called = Cell::new(false);
    let result: Result<()> = session.edit_document_with_interrupt(0, "No revision", &Interrupt::NONE, |_, _| {
        called.set(true);
        Ok(())
    });
    assert!(result.is_err());
    assert!(!called.get());
    assert!(session.documents()[0].doc.shares_state(&target));
    assert_eq!(session.active_index(), Some(1));
}

#[test]
fn exhausted_selection_revision_preserves_view_and_document() {
    let mut session = two_documents();
    let layer = session.active().unwrap().active_layer.unwrap();
    let source = session.active().unwrap().doc.clone();
    let selected = session.active().unwrap().selected_layers.clone();
    session.active_mut().unwrap().revision = u64::MAX;
    assert!(session.select_layer(layer).is_err());
    assert_eq!(session.active().unwrap().revision, u64::MAX);
    assert_eq!(session.active().unwrap().selected_layers, selected);
    assert!(session.active().unwrap().doc.shares_state(&source));
}

#[test]
fn prepared_selection_is_recorded_and_restored_with_document_history() {
    let mut session = two_documents();
    session.execute("layer.new.layer", json!({})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let prior_active = session.active().unwrap().active_layer;
    let prior_selected = session.active().unwrap().selected_layers.clone();
    let target = session.active().unwrap().doc.layers[0].id;
    session.edit_with_layer_target_with_interrupt("Prepared selection", &Interrupt::NONE,
        LayerTarget { active:Some(target), selected:vec![target] }, Some(target), |document, _| {
            document.name = "prepared selection document".into();
            Ok(())
        }).unwrap();
    assert_eq!(session.active().unwrap().selected_layers, vec![target]);
    assert_eq!(session.active().unwrap().active_layer, Some(target));
    assert!(session.undo());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().active_layer, prior_active);
    assert_eq!(session.active().unwrap().selected_layers, prior_selected);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().selected_layers, vec![target]);
    assert_eq!(session.active().unwrap().layer_anchor, Some(target));
}

#[test]
fn invalid_or_cancelled_prepared_selection_does_not_publish_document_or_view() {
    for cancel in [false,true] {
        let mut session = two_documents();
        let before = session.active().unwrap().doc.clone();
        let selected = session.active().unwrap().selected_layers.clone();
        let active = session.active().unwrap().active_layer;
        let anchor = session.active().unwrap().layer_anchor;
        let revision = session.active().unwrap().revision;
        let target = if cancel { active.unwrap() } else { LayerId(u64::MAX) };
        let edited = Cell::new(false);
        let signal = || cancel && edited.get();
        let control = Interrupt::cancel_only(&signal);
        let outcome = session.edit_with_layer_target_with_interrupt("Discard target", &control,
            LayerTarget { active:Some(target),selected:vec![target] }, Some(target), |document,_| {
                document.name = "discard".into();
                edited.set(true);
                Ok(())
            });
        assert!(outcome.is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().selected_layers, selected);
        assert_eq!(session.active().unwrap().active_layer, active);
        assert_eq!(session.active().unwrap().layer_anchor, anchor);
        assert_eq!(session.active().unwrap().revision, revision);
    }
}
