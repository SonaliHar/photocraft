//! Cancellation must leave the installation and history boundary untouched.
use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size};
use photocraft_engine::{EngineError, Session};
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn session() -> Session {
    let mut doc = Document::new("Before", Size::new(4, 3), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("Pixels", doc.pixel_format()));
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
#[test]
fn cancellation_after_the_edit_closure_preserves_document_selection_revision_and_history() {
    let mut session = session();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let active = session.active().unwrap().active_layer;
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&cancel);
    let result = session.edit_with_interrupt("Cancelled", &ctl, |doc, selected| {
        doc.name = "Discard".into();
        *selected = None;
        cancelled.store(true, Ordering::Relaxed);
        Ok(())
    });
    assert!(matches!(result, Err(EngineError::Cancelled)));
    let state = session.active().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.active_layer, active);
    assert_eq!(state.revision, revision);
    assert_eq!(state.history.past_len(), 0);
}
#[test]
fn pre_cancelled_edit_does_not_run_the_closure() {
    let mut session = session();
    let ran = AtomicBool::new(false);
    let ctl = Interrupt::cancel_only(&|| true);
    let result = session.edit_with_interrupt("Cancelled", &ctl, |_, _| {
        ran.store(true, Ordering::Relaxed);
        Ok(())
    });
    assert!(matches!(result, Err(EngineError::Cancelled)));
    assert!(!ran.load(Ordering::Relaxed));
    assert_eq!(session.active().unwrap().history.past_len(), 0);
}
#[test]
fn successful_controlled_edit_retains_native_history_behavior() {
    let mut session = session();
    let before = session.active().unwrap().doc.clone();
    session
        .edit_with_interrupt("Rename", &Interrupt::NONE, |doc, _| {
            doc.layers.first_mut().unwrap().name = "After".into();
            Ok(())
        })
        .unwrap();
    let after = session.active().unwrap().doc.clone();
    // Document titles retain current save identity across undo; layer names are model edits.
    assert_eq!(after.layers[0].name, "After");
    assert_eq!(session.active().unwrap().history.past_len(), 1);
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc, after);
}
#[test]
fn cancellation_at_final_validation_poll_does_not_install_or_record() {
    let count = AtomicUsize::new(0);
    let trace = || {
        count.fetch_add(1, Ordering::Relaxed);
        false
    };
    let mut calibration = session();
    calibration
        .edit_with_interrupt("Rename", &Interrupt::cancel_only(&trace), |doc, _| {
            doc.name = "After".into();
            Ok(())
        })
        .unwrap();
    let last = count.load(Ordering::Relaxed) - 1;
    assert!(last >= 3);
    let mut session = session();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= last;
    let result = session.edit_with_interrupt("Rename", &Interrupt::cancel_only(&cancel), |doc, _| {
        doc.name = "After".into();
        Ok(())
    });
    assert!(matches!(result, Err(EngineError::Cancelled)));
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    assert!(!session.active().unwrap().history.can_redo());
}
