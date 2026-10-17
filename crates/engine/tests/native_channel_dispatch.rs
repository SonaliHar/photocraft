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

#[test]
fn native_split_accepts_empty_effects_enable_state_and_rejects_effect_payload_atomically() {
    for depth in [SampleType::U32, SampleType::F64] {
        let bytes: Vec<u8> = if depth == SampleType::U32 {
            [1u32, 0x80000001, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect()
        } else {
            [(-0.0f64).to_bits(), (1.0f64 / 3.0).to_bits() + 1, 0.875f64.to_bits() - 1, 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect()
        };
        let build = |enabled, effect| {
            let mut doc = Document::new("Neutral native", Size::new(1, 1), ColorMode::Rgb, depth);
            let mut layer = Layer::raster("Pixels", doc.pixel_format());
            assert!(layer.effects.enabled, "the ordinary Layer constructor is the positive fixture");
            layer.effects.enabled = enabled;
            if effect {
                layer.effects.items.push(photocraft_doc::Effect::ColorOverlay {
                    common: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                    color: photocraft_color::Color::rgb(0.0, 1.0, 0.0),
                });
            }
            layer.content =
                LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(), Rect::new(0, 0, 1, 1), &bytes, &Interrupt::NONE).unwrap());
            doc.layers.push(layer);
            let mut session = Session::new();
            session.add_document(doc, None).unwrap();
            session
        };
        let mut previous = None;
        for enabled in [true, false] {
            let mut session = build(enabled, false);
            let before = session.active().unwrap().doc.clone();
            let result = session.execute("channel.split", json!({"closeOriginal":false})).unwrap();
            assert_eq!(result["documents"], json!([1, 2, 3]));
            assert_eq!(&*session.documents()[0].doc, &*before);
            let output: Vec<Vec<u8>> =
                session.documents().iter().skip(1).map(|state| state.doc.layers[0].surface().unwrap().to_interleaved(Rect::new(0, 0, 1, 1))).collect();
            for (channel, actual) in output.iter().enumerate() {
                let width = depth.bytes();
                let mut expected = bytes[channel * width..(channel + 1) * width].to_vec();
                expected.extend_from_slice(&bytes[3 * width..4 * width]);
                assert_eq!(actual, &expected, "raw channel and alpha words");
            }
            if let Some(previous) = previous {
                assert_eq!(output, previous);
            }
            previous = Some(output);
        }
        for enabled in [true, false] {
            let mut session = build(enabled, true);
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let journal = session.journal.clone();
            assert!(session.execute("channel.split", json!({})).is_err());
            assert_eq!(session.documents().len(), 1);
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.journal, journal);
            assert!(!session.active().unwrap().history.can_undo());
        }
    }
}
