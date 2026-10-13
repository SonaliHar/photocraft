//! Read-only queries must not normalize unrelated runtime view state.
use photocraft_doc::{ColorMode, Document, Layer, LayerId, SampleType, Size};
use photocraft_engine::{Session, mask_view_cmds::{LayerMaskView, MaskViewMode}};
use serde_json::json;

#[test]
fn native_pixel_inspection_preserves_stale_mask_view_and_edit_history() {
    let mut document = Document::new("Own read-only carrier", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let layer = Layer::raster("Native samples", document.pixel_format());
    let id = layer.id;
    document.layers.push(layer);
    let mut session = Session::new();
    session.open_document(document, None).unwrap();
    // A stale view can exist between event delivery and normal edit repair.
    // The query must report pixels without consuming that separate event.
    session.active_mut().unwrap().channel_view.layer_mask = Some(LayerMaskView { layer: LayerId(u64::MAX), mode: MaskViewMode::Gray });
    let before = session.active().unwrap().doc.clone();
    let view = session.active().unwrap().channel_view.clone();
    let revision = session.active().unwrap().revision;
    let history = session.active().unwrap().history.entry_labels().count();
    let journal = session.journal.clone();
    let result = session.execute("document.pixel.inspect", json!({"x":0,"y":0,"layerId":id.0.to_string(),"view":"native"})).unwrap();
    assert!(result.is_object());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().channel_view, view);
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.entry_labels().count(), history);
    assert_eq!(session.journal, journal);
    assert!(session.execute("document.pixel.inspect", json!({"view":"invalid"})).is_err());
    assert_eq!(session.active().unwrap().channel_view, view);
    assert!(session.active().unwrap().doc.shares_state(&before));
}
