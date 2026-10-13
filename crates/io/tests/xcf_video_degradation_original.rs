#![cfg(feature = "corpus")]
fn packet() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/video-degradation-edit-evidence")
}
#[test]
#[ignore = "independent public serialized monitor source/edit/disabled controls"]
fn saved_video_wire_source_edit_disabled() {
    let root = packet();
    for name in ["source", "edit", "disabled"] {
        let bytes = std::fs::read(root.join(format!("{name}.xcf"))).unwrap();
        let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(f.layers.len(), 1);
        let l = &f.layers[0];
        assert_eq!(l.effects.len(), 1);
        let e = &l.effects[0];
        assert_eq!(e.operation, "gegl:video-degradation");
        assert_eq!(e.version.as_deref(), Some("0:0"));
        assert_eq!(e.arguments.len(), 3);
        eprintln!(
            "{name} header=({},{},{},{},{:?},{:?}) owner=({}x{},offset{:?},mode{},spaces{}/{}/{}) effect={:?} args={:?}",
            f.version,
            f.precision,
            f.width,
            f.height,
            f.model,
            f.sample,
            l.width,
            l.height,
            l.offset,
            l.mode,
            l.blend_space,
            l.composite_space,
            l.composite_mode,
            e.state,
            e.arguments
        );
        if let Some(mask) = e.mask.as_ref() {
            eprintln!(
                "mask {}x{} {:?} mode{} spaces{}/{}/{} enabled{} shown{} editing{} linked{} visible{}",
                mask.width,
                mask.height,
                mask.offset,
                mask.mode,
                mask.blend_space,
                mask.composite_space,
                mask.composite_mode,
                mask.mask_enabled,
                mask.mask_shown,
                mask.mask_editing,
                mask.legacy_linked,
                mask.visible
            );
        }
    }
}
#[test]
#[ignore = "independent original import/native/reopen/projection coordinate controls"]
fn original_video_source_import() {
    let root = packet();
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    assert_eq!(observations["records"].as_array().unwrap().len(), 3);
    for row in observations["records"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        let imported = photocraft_io::xcf_import::import("own video source", &bytes, &photocraft_raster::Interrupt::NONE).unwrap();
        let doc = imported.document;
        assert_eq!((doc.size.width, doc.size.height), (16, 16));
        let native = std::fs::read(root.join(row["current"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(native.len(), 16 * 16 * 16);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened, doc);
        let projection = std::fs::read(root.join(row["current"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(projection.len(), 16 * 16 * 32);
        for current in [&doc, &reopened] {
            let rendered = photocraft_compose::gimp_render::try_render(current, current.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px.len(), 256);
            let roi = photocraft_compose::gimp_render::try_render(current, photocraft_geom::Rect::new(5, 7, 8, 11), &mut |_| Ok(())).unwrap();
            assert_eq!(roi.px.len(), 12);
            for (index, pixel) in roi.px.iter().enumerate() {
                assert_eq!(*pixel, rendered.px[(7 + index / 3) * 16 + 5 + index % 3]);
            }
            for (pixel, expected) in rendered.px.iter().zip(projection.as_chunks::<32>().0.iter()) {
                let reference: Vec<_> = expected.as_chunks::<8>().0.iter().map(|value| f64::from_le_bytes(*value)).collect();
                assert!((pixel[3] - reference[3]).abs() <= 1e-6);
                for channel in 0..3 {
                    assert!(
                        (pixel[channel] * pixel[3] - reference[channel] * reference[3]).abs() <= 1e-6,
                        "{} projected pixel {:?} reference {:?}",
                        row["id"],
                        pixel,
                        reference
                    );
                }
            }
        }
        assert!(photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&doc, &photocraft_raster::Interrupt::cancel_only(&|| true)).is_err());
        assert!(
            photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid(
                "lower caller budget"
            )))
            .is_err()
        );
        let mut geometry = doc.clone();
        geometry.size.width = 17;
        assert!(photocraft_compose::gimp_render::try_render(&geometry, geometry.bounds(), &mut |_| Ok(())).is_err());
    }
}
