#![cfg(feature = "corpus")]
//! Regression for a supported upgrade of a historical pinned effect boundary.
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
#[test]
fn pinned_legacy_effect_acceptance_matches_original_native_and_profile_matched_projection() {
    use photocraft_cms::{Builtin, Intent, Profile, Transform};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf");
    let source = std::fs::read(root.join("unsupported-gegl-effect.xcf")).unwrap();
    assert_eq!(hash(&source), "9f04009f355c58f42c4b186b642d67b42726a3a5bbbf0c268e6b9f772209a1e9");
    let png = std::fs::read(root.join("unsupported-gegl-effect.ref.png")).unwrap();
    assert_eq!(hash(&png), "18ee7030dc4a30062fb331c65aea14ad83ecd7f9183c308936566905e14d5f64");
    let native = std::fs::read(root.join("unsupported-gegl-effect.0.native.bin")).unwrap();
    assert_eq!(hash(&native), "522a6f906bbee4ed0e0239d1875998f015dff8dc7425b00b3d47217a7e9b5b34");
    let parsed = photocraft_xcf::decode(&source, &Default::default()).unwrap();
    assert_eq!((parsed.width, parsed.height), (65, 67));
    assert_eq!(parsed.layers.len(), 1);
    assert_eq!(parsed.layers[0].effects.len(), 1, "do not qualify silently dropped effects");
    let raw = &parsed.layers[0].effects[0];
    assert_eq!(raw.operation, "gegl:invert-linear");
    assert_eq!(raw.version.as_deref(), Some("0:0"));
    assert!(raw.arguments.is_empty());
    assert_eq!(raw.state.enabled, Some(true));
    assert_eq!(raw.state.opacity_u8, None);
    assert_eq!(raw.state.opacity_bits, Some(1f32.to_bits()));
    assert_eq!(raw.state.mode, Some(62));
    assert_eq!(raw.state.composite_mode, Some(1368949329));
    assert_eq!(raw.state.composite_space, Some(1368949329));
    assert_eq!(raw.state.blend_space, Some(1368949329));
    assert_eq!(raw.state.clipping, Some(0));
    assert_eq!(raw.state.region, Some(0));
    let raw_mask = raw.mask.as_ref().unwrap();
    assert_eq!((raw_mask.width, raw_mask.height, raw_mask.offset), (65, 67, (0, 0)));
    assert_eq!(raw_mask.model, photocraft_xcf::Model::Gray);
    assert!(!raw_mask.alpha);
    assert!(raw_mask.visible && raw_mask.mask_enabled);
    assert!(!raw_mask.mask_shown && !raw_mask.mask_editing && !raw_mask.legacy_linked);
    let mask_plane = raw_mask.plane.as_ref().unwrap();
    assert_eq!(mask_plane.sample, photocraft_xcf::Sample::U8);
    assert_eq!(mask_plane.channels, 1);
    assert_eq!(mask_plane.bytes.len(), 65 * 67);
    assert!(mask_plane.bytes.iter().all(|byte| *byte == 0));
    println!(
        "MASK {:?}",
        raw.mask.as_ref().map(|mask| (
            mask.width,
            mask.height,
            mask.offset,
            mask.plane.as_ref().map(|p| (p.sample, p.bytes.len(), p.bytes.iter().all(|byte| *byte == 0)))
        ))
    );
    let imported = photocraft_io::import("unsupported-gegl-effect.xcf", &source).unwrap();
    assert!(imported.origin.is_read_only());
    let doc = imported.document;
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(doc.depth, photocraft_doc::SampleType::U8);
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
    assert_eq!(doc.layers.len(), 1);
    let current = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(current.operation.name, raw.operation);
    assert_eq!(current.operation.stored_version, *raw.version.as_ref().unwrap());
    assert_eq!(current.operation.schema_epoch, "xcf-22-stock-invert-linear-precision-v1");
    assert!(current.arguments.is_empty() && current.enabled && current.auxiliary.is_empty());
    assert_eq!(current.opacity_f64_bits, 1f64.to_bits());
    assert_eq!(current.source_clipping, Some(0));
    assert_eq!(current.source_region, Some(0));
    let policy = current.composition.as_ref().unwrap();
    assert_eq!(policy.compatibility_epoch, "xcf-22-stock-point-replace-fields-v1");
    assert_eq!(policy.source_mode, 62);
    assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (1368949329, 1368949329, 1368949329));
    assert!(policy.resolved.is_none() && !policy.legacy && !policy.pass_through);
    let state = doc.xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_stacks.len(), 1);
    assert_eq!(state.filter_stacks[0].nodes.len(), 1);
    assert_eq!(state.filter_masks.len(), 1);
    let mask = &state.filter_masks[0];
    assert_eq!(mask.owner, current.id);
    assert_eq!(current.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(mask.id)));
    assert_eq!(mask.bounds, doc.bounds());
    assert!(mask.is_empty_selection_with(&photocraft_raster::Interrupt::NONE).unwrap());
    assert_eq!(mask.surface.try_read_interleaved(mask.bounds).unwrap(), mask_plane.bytes);
    assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
    let legacy = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&legacy).unwrap();
    assert_eq!(reopened, doc);
    let (stream, _) = photocraft_format::PcraftWriter::new()
        .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
        .unwrap();
    let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(streamed, doc);
    let reference = photocraft_codecs::decode(&png).unwrap();
    assert_eq!((reference.width(), reference.height()), (65, 67));
    let reference_profile = Profile::parse(reference.icc.as_ref().unwrap()).unwrap();
    let reference_to_srgb = Transform::new(&reference_profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
    let mut worst = 0f64;
    for candidate in [&doc, &reopened, &streamed] {
        let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(rendered.px.len(), 65 * 67);
        for (index, pixel) in rendered.px.iter().enumerate() {
            let x = index as u32 % 65;
            let y = index as u32 / 65;
            let input: [f64; 3] = std::array::from_fn(|channel| f64::from(reference.get(x, y, channel)));
            let mut expected = [0f64; 3];
            reference_to_srgb.try_eval_f64(&input, &mut expected).unwrap();
            let alpha = if reference.layout().has_alpha() { f64::from(reference.get(x, y, 3)) } else { 1. };
            assert!((pixel[3] - alpha).abs() <= 1. / 65535.);
            for channel in 0..3 {
                let delta = (pixel[channel] * pixel[3] - expected[channel] * alpha).abs();
                worst = worst.max(delta);
                assert!(delta <= 2. / 255., "historical effect pixel{index} channel{channel} delta{}", delta * 255.);
            }
        }
    }
    assert_eq!(std::fs::read(root.join("unsupported-gegl-effect.xcf")).unwrap(), source);
    assert_eq!(std::fs::read(root.join("unsupported-gegl-effect.ref.png")).unwrap(), png);
    println!("Legacy pinned effect native/legacy/ZIP64 exact; profile-matched PNG max {}/255", worst * 255.);
}
