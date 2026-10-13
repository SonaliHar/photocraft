//! Unrun Multichannel token/target/native-state regression drafts.
use super::*;
use photocraft_doc::{PhysicalResolution, PhysicalUnit, Size};
use photocraft_raster::Interrupt;
fn session(depth: SampleType) -> Session {
    let mut document = Document::with_background("Own plate control", Size::new(3, 2), ColorMode::Rgb, depth, Color::WHITE);
    document.set_physical_resolution(PhysicalResolution { x_dpi: 144.25, y_dpi: 287.75, unit: PhysicalUnit::Millimeter }).unwrap();
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    session
}
#[test]
fn wide_native_and_explicit_current_context_refuse_before_plate_read_or_history() {
    for depth in [SampleType::U32, SampleType::F64] {
        let mut session = session(depth);
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        assert!(session.execute("image.mode.multichannel", json!({})).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
    }
    let original = session(SampleType::F32);
    let mut document = (*original.active().unwrap().doc).clone();
    document.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear);
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    let before = session.active().unwrap().doc.clone();
    assert!(session.execute("image.mode.multichannel", json!({})).is_err());
    assert_eq!(session.active().unwrap().doc, before);
}
#[test]
fn paired_target_and_ignored_profile_controls_are_bounded_without_profile_authority() {
    let mut session = session(SampleType::U16);
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    for params in [
        json!({"documentId":before.id.0.to_string()}),
        json!({"documentId":before.id.0.to_string(),"expectedRevision":revision}),
        json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
        json!({"profile":"x".repeat(4097)}),
        json!({"bpc":[]}),
        json!({"intent":"unknown"}),
    ] {
        assert!(session.execute("image.mode.multichannel", params).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
    }
    session.execute("image.mode.multichannel", json!({"documentId":before.id.0.to_string(),"expectedRevision":revision.to_string()})).unwrap();
    // Profile text retains its conventional ignored Multichannel meaning; it
    // cannot request file access or profile resolution from this plate route.
    session.execute("image.mode.rgb", json!({"profile":"not-a-readable-profile.icc","intent":"relative","bpc":false})).unwrap();
    assert_eq!(session.active().unwrap().doc.effective_resolution(), before.effective_resolution());
}
#[test]
fn cancel_before_preparation_keeps_native_model_and_real_redo() {
    let mut session = session(SampleType::U8);
    session
        .edit("Real redo", |document, _| {
            document.name = "redo retained".into();
            Ok(())
        })
        .unwrap();
    assert!(session.undo());
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    assert!(matches!(admission(&before, ColorMode::Multichannel, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(session.redo());
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn both_registered_job_directions_cancel_and_reject_revision_drift_atomically() {
    use crate::jobs::Started;
    let mut session = session(SampleType::U16);
    session
        .edit("Real redo", |document, _| {
            document.name = "redo retained".into();
            Ok(())
        })
        .unwrap();
    assert!(session.undo());
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let Started::Job(job) = session.start("image.mode.multichannel", json!({})).unwrap() else { panic!("native Multichannel job") };
    assert!(session.cancel_job(job));
    session.join_cancelled_jobs();
    assert!(session.poll_jobs().is_empty());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(session.redo());
    session.execute("image.mode.multichannel", json!({})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let Started::Job(job) = session.start("image.mode.rgb", json!({})).unwrap() else { panic!("native inverse plate job") };
    session.active_mut().unwrap().revision += 1;
    assert!(session.wait_job(job).is_err());
    assert_eq!(session.active().unwrap().doc, before);
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn plate_job_applies_to_original_tab_with_one_undo_and_preserves_focus() {
    use crate::jobs::Started;
    let mut session = session(SampleType::F32);
    let id = session.active().unwrap().doc.id;
    let original = session.active().unwrap().doc.clone();
    let Started::Job(job) = session.start("image.mode.multichannel", json!({})).unwrap() else { panic!("native Multichannel job") };
    session.add_document(Document::with_background("Own other tab", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE), None).unwrap();
    let other = session.active().unwrap().doc.clone();
    session.wait_job(job).unwrap();
    assert_eq!(session.active().unwrap().doc, other);
    let converted = session.documents().iter().find(|state| state.doc.id == id).unwrap();
    assert_eq!(converted.doc.mode, ColorMode::Multichannel);
    assert_eq!(converted.doc.effective_resolution(), original.effective_resolution());
    assert_eq!(converted.history.entry_labels().count(), 1);
}
