#![cfg(feature = "corpus")]
#[path = "common/video_options.rs"]
mod packet;
use photocraft_raster::Interrupt;
fn display_projection_icc(document: &photocraft_doc::Document, expected: &[u8], effective_icc: &[u8], label: &str) {
    // The source XCF has no assigned ICC. PhotoCraft retains its implicit sRGB;
    // GIMP's independent projection is interpreted with its exact effective ICC.
    assert!(document.source_color_context.as_ref().unwrap().original_icc.is_none());
    let actual_profile = photocraft_cms::Profile::parse(document.icc_profile.as_ref().unwrap()).unwrap();
    assert_eq!(document.icc_profile.as_ref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().to_bytes().as_slice());
    let effective = photocraft_cms::Profile::parse(effective_icc).unwrap();
    let target = photocraft_cms::Builtin::Srgb.profile();
    let actual_transform = photocraft_cms::Transform::new(&actual_profile, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
    let reference_transform = photocraft_cms::Transform::new(&effective, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
    let display = photocraft_compose::gimp_render::PreparedRenderer::new(document).unwrap().render_reduced(16, 16, None, &mut |_| Ok(())).unwrap();
    assert_eq!(display.px.len(), 256);
    for (pixel, raw) in display.px.iter().zip(expected.as_chunks::<32>().0.iter()) {
        let reference: Vec<_> = raw.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect();
        let mut actual_rgb = [0.; 3];
        let mut reference_rgb = [0.; 3];
        actual_transform.try_eval_f64(&pixel[..3].iter().map(|v| f64::from(*v)).collect::<Vec<_>>(), &mut actual_rgb).unwrap();
        reference_transform.try_eval_f64(&reference[..3], &mut reference_rgb).unwrap();
        assert!((f64::from(pixel[3]) - reference[3]).abs() <= 2. / 65535., "{label} display alpha");
        for c in 0..3 {
            assert!(
                (actual_rgb[c] * f64::from(pixel[3]) - reference_rgb[c] * reference[3]).abs() <= 2. / 255.,
                "{label} independent projection/effective-ICC vs implicit-sRGB display{c}"
            );
        }
    }
}
fn projection(document: &photocraft_doc::Document, expected: &[u8], label: &str) {
    let rendered = photocraft_compose::gimp_render::try_render(document, document.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(rendered.px.len(), 256);
    assert_eq!(expected.len(), 256 * 32);
    for (actual, raw) in rendered.px.iter().zip(expected.as_chunks::<32>().0.iter()) {
        let reference: Vec<_> = raw.as_chunks::<8>().0.iter().map(|value| f64::from_le_bytes(*value)).collect();
        assert!((actual[3] - reference[3]).abs() <= 1e-6, "{label} alpha");
        for channel in 0..3 {
            assert!((actual[channel] * actual[3] - reference[channel] * reference[3]).abs() <= 1e-6, "{label} projected channel{channel}");
        }
    }
    let roi = photocraft_compose::gimp_render::try_render(document, photocraft_geom::Rect::new(5, 7, 8, 11), &mut |_| Ok(())).unwrap();
    assert_eq!(roi.px.len(), 12);
    for (index, pixel) in roi.px.iter().enumerate() {
        assert_eq!(*pixel, rendered.px[(7 + index / 3) * 16 + 5 + index % 3]);
    }
}
#[test]
#[ignore = "immutable temp108 independently authored Video options controls; no corpus enrollment"]
fn actual_108_options_import_native_archives_and_reference() {
    let observations = packet::observations().unwrap();
    let root = packet::root();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap() {
        let source = packet::blob(&root, row).unwrap();
        let document = photocraft_io::import(row["file"].as_str().unwrap(), &source).unwrap().document;
        let native = packet::blob(&root, &row["current"]["layers"][0]["native"]).unwrap();
        assert_eq!(document.layers[0].surface().unwrap().try_read_interleaved(document.bounds()).unwrap(), native);
        let node = &document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(node.operation.schema_epoch, packet::EPOCH);
        let policy = node.composition.as_ref().unwrap();
        assert_eq!(policy.compatibility_epoch, packet::EPOCH);
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (2026996305, 2026996305, 2026996305));
        assert_eq!(node.enabled, row["role"] != "disabled");
        let expected = packet::blob(&root, &row["current"]["projection"]).unwrap();
        projection(&document, &expected, row["id"].as_str().unwrap());
        let icc = packet::blob(&root, &row["current"]["profiles"]["effective"]).unwrap();
        display_projection_icc(&document, &expected, &icc, row["id"].as_str().unwrap());
        let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened, document);
        projection(&reopened, &expected, row["id"].as_str().unwrap());
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&document, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &Interrupt::NONE)
            .unwrap();
        let zip64 =
            photocraft_format::load_from_stream(std::io::Cursor::new(stream.into_inner()), &Default::default(), Default::default(), &Interrupt::NONE).unwrap();
        assert_eq!(zip64, document);
        projection(&zip64, &expected, row["id"].as_str().unwrap());
        assert_eq!(packet::blob(&root, row).unwrap(), source);
        count += 1;
    }
    assert_eq!(count, 108);
}
#[test]
#[ignore = "immutable temp Video options current contexts and swapped epoch/policy controls"]
fn options_current_context_and_markers_fail_closed() {
    let observations = packet::observations().unwrap();
    let row = &observations["records"][0];
    let original = photocraft_io::import(row["file"].as_str().unwrap(), &packet::blob(&packet::root(), row).unwrap()).unwrap().document;
    for mutation in 0..23 {
        let mut bad = original.clone();
        match mutation {
            0 => {
                bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch =
                    photocraft_compose::gimp_video_degradation::VIDEO_DEGRADATION_EPOCH.into()
            }
            1 => {
                bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch =
                    "xcf-22-stock-point-replace-fields-v1".into()
            }
            2 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_blend_space = -1877441967,
            3 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_composite_space = 0,
            4 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = -1739226543,
            5 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "future".into(),
            6 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "future".into(),
            7 => {
                let node = &mut bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                node.operation.name = "gegl:invert-gamma".into();
                node.arguments.clear();
            }
            8 => {
                let node = &mut bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                node.operation.name = "gegl:invert-gamma".into();
                node.operation.schema_epoch = "xcf-22-stock-point-v1".into();
                node.arguments.clear();
            }
            9 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
            10 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_clipping = None,
            11 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits = 0.5f64.to_bits(),
            12 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
            13 => bad.xcf_state.as_mut().unwrap().filter_masks.clear(),
            14 => bad.xcf_state.as_mut().unwrap().current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Half,
            15 => bad.depth = photocraft_color::SampleType::U16,
            16 => bad.mode = photocraft_color::ColorMode::Grayscale,
            17 => bad.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear),
            18 => bad.size = photocraft_geom::Size::new(17, 16),
            19 => bad.icc_profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes()),
            20 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition = None,
            21 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().source_mode = 28,
            22 => bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().pass_through = true,
            _ => panic!("unexpected mutation ordinal"),
        }
        assert!(photocraft_compose::gimp_render::try_render(&bad, bad.bounds(), &mut |_| Ok(())).is_err(), "current mutation{mutation}");
    }
    assert!(photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&original, &Interrupt::cancel_only(&|| true)).is_err());
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let late = || {
        let old = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        old >= 2
    };
    assert!(photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&original, &Interrupt::cancel_only(&late)).is_err());
}
