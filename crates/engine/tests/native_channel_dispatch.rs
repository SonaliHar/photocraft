//! Registered workflow drafts, intentionally unrun during implementation-first work.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde_json::json;

#[test]
fn registered_native_split_then_explicit_merge_retains_component_words() {
    for depth in [SampleType::U32, SampleType::F64] {
        let bytes = match depth {
            SampleType::U32 => [1u32, 123_456_789, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>(),
            SampleType::F64 => {
                [(-0.0f64).to_bits(), (1.0f64 / 3.0).to_bits() + 1, 0.875f64.to_bits() - 1, 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect()
            }
            _ => panic!("native fixture depth"),
        };
        let mut doc = Document::new("Own channel workflow", Size::new(1, 1), ColorMode::Rgb, depth);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        layer.content = LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(), Rect::new(0, 0, 1, 1), &bytes, &Interrupt::NONE).unwrap());
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let split = session.execute("channel.split", json!({})).unwrap();
        assert_eq!(split["documents"], json!([0, 1, 2]));
        assert_eq!(session.documents().len(), 3);
        session.set_active(0);
        let merged = session.execute("channel.merge", json!({"mode":"rgb","documents":[0,1,2]})).unwrap();
        assert_eq!(merged["document"], 0);
        assert_eq!(session.documents().len(), 1);
        let state = session.active().unwrap();
        assert_eq!(state.doc.mode, ColorMode::Rgb);
        assert_eq!(state.doc.depth, depth);
        let layer = state.doc.layers.first().unwrap();
        assert_eq!(layer.surface().unwrap().to_interleaved(Rect::new(0, 0, 1, 1)), bytes);
        assert!(layer.locks.transparency && layer.locks.position);
    }
}
