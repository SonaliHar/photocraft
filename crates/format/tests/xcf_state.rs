//! Imported state must survive native migration without granting source filesystem authority.
use photocraft_doc::{
    ColorMode, Document, ImportCompositing, IndexedSource, Layer, LayerContent, PixelFormat, Rect, SampleEncoding, SampleType, Size, SourceColorContext,
};
use photocraft_format::{SaveOptions, load_from_bytes, save_to_bytes};
use photocraft_raster::Surface;
use std::sync::Arc;

#[test]
fn indexed_duplicates_survive_edit_save_and_undo() {
    let r = Rect::new(-1, 0, 1, 1);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let original = Surface::try_from_interleaved(fmt, r, &[10, 20, 30, 255, 10, 20, 30, 127]).unwrap();
    let palette = vec![[10, 20, 30], [10, 20, 30]];
    let source = IndexedSource::new(Arc::new(vec![0, 1]), Some(Arc::new(vec![255, 127])), palette.clone(), r, &original).unwrap();
    assert!(source.current_matches(&original, &palette, r));
    let mut edited = original.clone();
    edited.write_pixel(-1, 0, &[1.0, 0.0, 0.0, 1.0]);
    assert!(!source.current_matches(&edited, &palette, r));
    let mut d = Document::new("indexed", Size::new(2, 1), ColorMode::Indexed, SampleType::U8);
    let mut l = Layer::new("duplicates", LayerContent::Raster(edited));
    l.indexed_source = Some(source);
    d.layers.push(l);
    let reopened = load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(reopened, d);
    let source = reopened.layers[0].indexed_source.as_ref().unwrap();
    assert_eq!(source.indices().as_slice(), &[0, 1]);
    assert!(source.current_matches(&original, &palette, r));
    assert!(!source.current_matches(&original, &[[10, 20, 30]], r));
    assert!(!source.current_matches(&original, &palette, r.translate(1, 0)));
}

#[test]
fn source_interpretation_and_policy_roundtrip_all_depths() {
    for depth in SampleType::ALL {
        for encoding in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile, SampleEncoding::Perceptual] {
            let mut d = Document::new("source", Size::new(1, 1), ColorMode::Rgb, depth);
            d.sample_encoding = Some(encoding);
            d.source_color_context = Some(SourceColorContext { original_icc: Some(Arc::new(vec![1, 2, 3])), source_precision: 600 });
            let mut l = Layer::raster("layer", d.pixel_format());
            l.import_compositing = Some(ImportCompositing { blend_encoding: encoding, composite_encoding: encoding, pass_through: false });
            d.layers.push(l);
            assert_eq!(load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap(), d);
        }
    }
}

#[test]
fn invalid_index_planes_palette_and_fingerprints_rejected() {
    let r = Rect::new(0, 0, 1, 1);
    for (indices, alpha, palette, fingerprint) in [
        (vec![], None, vec![[0, 0, 0]], [1; 32]),
        (vec![1], None, vec![[0, 0, 0]], [1; 32]),
        (vec![0], Some(Arc::new(vec![])), vec![[0, 0, 0]], [1; 32]),
        (vec![0], None, vec![], [1; 32]),
        (vec![0], None, vec![[0, 0, 0]], [0; 32]),
    ] {
        assert!(IndexedSource::from_parts(Arc::new(indices), alpha, palette, r, fingerprint).is_err());
    }
}

#[test]
fn absent_fields_keep_legacy_defaults_and_unknown_policy_is_rejected() {
    let d = Document::new("old", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    assert_eq!(load_from_bytes(&save_to_bytes(&d, &SaveOptions::default()).unwrap()).unwrap(), d);
    assert!(serde_json::from_str::<ImportCompositing>(r#"{"blend_encoding":"unknown","composite_encoding":"linear","pass_through":false}"#).is_err());
    assert!(
        serde_json::from_str::<ImportCompositing>(r#"{"blend_encoding":"linear","composite_encoding":"linear","pass_through":false,"operator":42}"#).is_err()
    );
}

#[test]
fn older_native_manifest_without_import_fields_loads() {
    let mut d = Document::new("legacy", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    d.layers.push(Layer::raster("pixels", d.pixel_format()));
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-legacy-{}-{}", std::process::id(), d.id.0));
    photocraft_format::PcraftWriter::new().save_dir(&d, &dir, &SaveOptions::default()).unwrap();
    let path = dir.join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let doc = manifest["document"].as_object_mut().unwrap();
    doc.remove("sample_encoding");
    doc.remove("source_color_context");
    for layer in doc["layers"].as_array_mut().unwrap() {
        layer.as_object_mut().unwrap().remove("indexed_source");
        layer.as_object_mut().unwrap().remove("import_compositing");
    }
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(photocraft_format::load_path(&dir).unwrap(), d);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn native_fingerprint_cannot_claim_arbitrary_original_pixels() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let r = Rect::new(0, 0, 1, 1);
    let s = Surface::try_from_interleaved(format, r, &[10, 20, 30, 255]).unwrap();
    let source = IndexedSource::new(Arc::new(vec![0]), None, vec![[10, 20, 30]], r, &s).unwrap();
    assert!(IndexedSource::from_parts(source.indices().clone(), source.alpha().clone(), source.palette().to_vec(), r, [1; 32]).is_err());
    assert!(IndexedSource::new(Arc::new(vec![0]), None, vec![[11, 20, 30]], r, &s).is_err());
}

#[test]
fn indexed_provenance_verification_is_interruptible() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let r = Rect::new(0, 0, 257, 1);
    let pixels: Vec<u8> = (0..257).flat_map(|_| [10, 20, 30, 255]).collect();
    let s = Surface::try_from_interleaved(PixelFormat::RGBA8, r, &pixels).unwrap();
    let before = s.clone();
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= 4;
    let ctl = photocraft_raster::Interrupt::cancel_only(&cancel);
    assert!(IndexedSource::new_with_interrupt(Arc::new(vec![0; 257]), None, vec![[10, 20, 30]], r, &s, &ctl).is_err());
    assert_eq!(s, before);
    assert!(IndexedSource::new(Arc::new(vec![0; 257]), None, vec![[10, 20, 30]], r, &s).is_ok());
}

/// Native transport must retain stored Auto independently from its resolved snapshot.
/// This is data equality, not composition or producer qualification.
#[test]
fn stored_auto_and_resolved_composition_snapshot_roundtrip_all_depths() {
    use photocraft_doc::{
        XcfState,
        import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    };
    for depth in SampleType::ALL {
        for space in [GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpSpace::Lab, GimpSpace::PerceptualRgb] {
            let mut doc = Document::new("Retained Auto policy", Size::new(1, 1), ColorMode::Rgb, depth);
            doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
            doc.layers.push(Layer::raster("Unrelated layer metadata", doc.pixel_format()));
            doc.layers[0].visible = false;
            doc.layers[0].opacity = 0.375;
            let mut state = XcfState::default();
            state.reconcile_layers(&doc).unwrap();
            state.items[0].composition = Some(GimpComposition {
                compatibility_epoch: "synthetic-native-storage-contract".into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: 0,
                stored_operator: 0,
                resolved: Some(ResolvedGimpComposition { blend_space: space, composite_space: space, operator: GimpOperator::Union }),
                pass_through: false,
            });
            let expected = state.items[0].composition.clone().unwrap();
            expected.checked_resolved().unwrap();
            doc.xcf_state = Some(state);
            let saved = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
            let loaded = load_from_bytes(&saved).unwrap();
            assert_eq!(loaded, doc);
            let actual = loaded.xcf_state.as_ref().unwrap().items[0].composition.as_ref().unwrap();
            assert_eq!(actual, &expected);
            assert_eq!((actual.stored_blend_space, actual.stored_composite_space, actual.stored_operator), (0, 0, 0));
            assert_eq!(actual.checked_resolved().unwrap(), expected.resolved.as_ref().unwrap());
            assert_eq!(load_from_bytes(&save_to_bytes(&loaded, &SaveOptions::default()).unwrap()).unwrap(), doc);
        }
    }
}
