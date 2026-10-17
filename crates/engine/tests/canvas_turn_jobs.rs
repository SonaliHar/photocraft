//! Unrun registered canvas-turn job/cancellation drafts; no source-domain widening.
use photocraft_doc::{Color, ColorMode, Document, PhysicalResolution, SampleType, Size};
use photocraft_engine::Session;
#[cfg(not(target_arch = "wasm32"))]
use photocraft_engine::jobs::Started;
use serde_json::json;

fn session() -> Session {
    let mut document = Document::with_background("Own canvas token", Size::new(3, 2), ColorMode::Rgb, SampleType::U16, Color::WHITE);
    document.set_physical_resolution(PhysicalResolution { x_dpi: 144.25, y_dpi: 287.75, unit: Default::default() }).unwrap();
    document.layers.first_mut().unwrap().surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.25, 0.5, 0.75, 0.5]).unwrap();
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    session
}
#[test]
fn immediate_turn_keeps_null_one_undo_focus_and_exact_axis_swap() {
    let mut session = session();
    let before = session.active().unwrap().doc.clone();
    let focus = session.active().unwrap().active_layer;
    let entries = session.active().unwrap().history.entry_labels().count();
    assert!(session.execute("image.imageRotation.90cw", json!({})).unwrap().is_null());
    let active = session.active().unwrap();
    assert_eq!(active.doc.size, Size::new(2, 3));
    assert_eq!(active.active_layer, focus);
    assert_eq!(active.history.entry_labels().count(), entries + 1);
    assert_eq!(active.doc.physical_resolution.as_ref().unwrap().x_dpi, 287.75);
    assert_eq!(active.doc.physical_resolution.as_ref().unwrap().y_dpi, 144.25);
    let native = photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *active.doc);
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before);
}
#[test]
fn theoretical_24mp_u16_payload_is_not_a_tripled_model_clone() {
    // Capacity arithmetic only: does not allocate/qualify a 24 MP document or
    // measure RSS/latency. The actual small-document visitor supplies the base
    // public model slice; tile-map allowances scale separately from tile bytes.
    let session = session();
    let doc = &session.active().unwrap().doc;
    let model = photocraft_io::retained_native_model_bytes(doc, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap() as u64;
    let side = u64::try_from(photocraft_geom::TILE_SIZE).unwrap();
    let tiles = 6000_u64.div_ceil(side) * 4000_u64.div_ceil(side);
    let payload = tiles * side * side * 8;
    let clone_metadata = model + tiles * 256;
    assert!(payload > 128 << 20);
    assert!(payload * 3 > 256 << 20);
    let peak = payload.checked_add(clone_metadata.checked_mul(3).unwrap()).unwrap();
    assert!(peak <= photocraft_raster::MAX_CHECKED_RASTER_BYTES);
    assert!(u64::MAX.checked_mul(3).is_none());
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cancelled_registered_job_keeps_native_model_and_real_redo() {
    let mut session = session();
    session
        .edit("Real redo", |doc, _| {
            doc.layers[0].name = "redo sentinel".into();
            Ok(())
        })
        .unwrap();
    assert!(session.undo());
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    let Started::Job(job) = session.start("image.imageRotation.90cw", json!({})).unwrap() else { panic!("native turn must use job-capable route") };
    assert!(session.cancel_job(job));
    session.join_cancelled_jobs();
    let events = session.poll_jobs();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, job);
    assert_eq!(events[0].document, Some(before.id));
    assert_eq!(events[0].command, "image.imageRotation.90cw");
    assert!(matches!(events[0].outcome, photocraft_engine::jobs::JobOutcome::Cancelled));
    assert!(session.poll_jobs().is_empty());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc.layers[0].name, "redo sentinel");
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn revision_only_target_drift_refuses_finished_job_without_installing() {
    let mut session = session();
    let before = session.active().unwrap().doc.clone();
    let entries = session.active().unwrap().history.entry_labels().count();
    let Started::Job(job) = session.start("image.imageRotation.180", json!({})).unwrap() else { panic!("native turn must use job-capable route") };
    session.active_mut().unwrap().revision += 1;
    assert!(session.wait_job(job).is_err());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.entry_labels().count(), entries);
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn background_turn_applies_to_original_tab_and_preserves_viewed_tab() {
    let mut session = session();
    let original_id = session.active().unwrap().doc.id;
    let Started::Job(job) = session.start("image.imageRotation.90ccw", json!({})).unwrap() else { panic!("native turn must use job-capable route") };
    session.add_document(Document::with_background("Own other tab", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE), None).unwrap();
    let other = session.active().unwrap().doc.clone();
    session.wait_job(job).unwrap();
    assert_eq!(session.active().unwrap().doc, other);
    let turned = session.documents().iter().find(|state| state.doc.id == original_id).unwrap();
    assert_eq!(turned.doc.size, Size::new(2, 3));
}
