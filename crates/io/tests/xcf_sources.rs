use photocraft_doc::{
    LayerId, PixelFormat,
    item_identity::{ItemRef, ResourceId},
    xcf_items::EditableSource,
};
use photocraft_io::IoError;
use photocraft_io::xcf_sources as mapper;
use photocraft_raster::Interrupt;
use photocraft_xcf::{Drawable, LayerTransform, LinkedLayer, Plane, Sample};
#[test]
fn inert_link_cache_flags_transform_and_authority_are_exact() {
    let source = Drawable {
        width: 1,
        height: 1,
        alpha: true,
        link: Some(LinkedLayer { flags: 3, path: "../../missing-Ω.png".into(), width: 123, height: 456 }),
        transform: Some(LayerTransform { offset: (-7, 9), interpolation: 4, matrix: [1., -0., 2., 0., 1., 3., 0., 0., 1.] }),
        ..Default::default()
    };
    let plane = Plane { width: 1, height: 1, channels: 4, sample: Sample::U8, bytes: vec![1, 2, 3, 4] };
    let output = mapper::map_source(
        &source,
        Some(plane),
        mapper::SourceContext { owner: ItemRef::Layer(LayerId(99)), vector_path: None, format: PixelFormat::RGBA8, source_version: 25 },
        &Interrupt::NONE,
        &mut || ResourceId(101),
        &mut |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(output.item.cache.as_ref().unwrap().bytes.as_slice(), [1, 2, 3, 4]);
    assert_eq!(output.resources[0].locator.as_deref(), Some("../../missing-Ω.png"));
    assert!(output.resources[0].owned_bytes.is_none());
    assert!(output.resources[0].content_hash.is_none());
    assert_eq!(output.item.transform.as_ref().unwrap().matrix_f64_bits[1], (-0f64).to_bits());
    assert_eq!(output.item.transform.as_ref().unwrap().pre_offset, [-7, 9]);
    assert!(matches!(output.item.source, EditableSource::Linked { resource: ResourceId(101), last_verified_hash: None, .. }));
}
#[test]
fn invalid_cache_missing_path_unknown_flags_budget_and_cancel_reject() {
    let source = Drawable {
        width: 1,
        height: 1,
        alpha: true,
        link: Some(LinkedLayer { flags: 4, path: "own-inert://absent".into(), width: 0, height: 0 }),
        ..Default::default()
    };
    let context = || mapper::SourceContext { owner: ItemRef::Layer(LayerId(1)), vector_path: None, format: PixelFormat::RGBA8, source_version: 25 };
    assert!(mapper::map_source(&source, None, context(), &Interrupt::NONE, &mut || ResourceId(2), &mut |_| Ok(())).is_err());
    let mut good = source;
    good.link.as_mut().unwrap().flags = 0;
    assert!(mapper::map_source(&good, None, context(), &Interrupt::NONE, &mut || ResourceId(0), &mut |_| Ok(())).is_err());
    let mut nul = good.clone();
    nul.link.as_mut().unwrap().path.push('\0');
    assert!(mapper::map_source(&nul, None, context(), &Interrupt::NONE, &mut || ResourceId(2), &mut |_| Ok(())).is_err());
    assert!(
        mapper::map_source(&good, None, context(), &Interrupt::NONE, &mut || ResourceId(2), &mut |_| Err(IoError::Unsupported("budget denied".into())))
            .is_err()
    );
    let bad = Plane { width: 1, height: 1, channels: 4, sample: Sample::U16, bytes: vec![0; 8] };
    assert!(mapper::map_source(&good, Some(bad), context(), &Interrupt::NONE, &mut || ResourceId(2), &mut |_| Ok(())).is_err());
}
#[test]
#[ignore = "requires reviewed original full-XCF corpus (not publicly pinned)"]
fn original_source_and_modified_text_vector_link_cache_state_is_exact() {
    use sha2::{Digest, Sha256};
    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").join("photocraft-corpus/xcf");
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("full/coverage-fixtures.json")).unwrap()).unwrap();
    for name in ["text-basic", "text-painted", "text-transform", "vector-basic", "vector-modified", "link-transform"] {
        let fixture = manifest["fixtures"].as_array().unwrap().iter().find(|f| f["id"].as_str() == Some(&format!("feature002-gimp326-{name}"))).unwrap();
        for (role, state_role) in [("source", "reopened_state"), ("reference_edit", "edited_state")] {
            let artifact = fixture["artifacts"].as_array().unwrap().iter().find(|a| a["role"] == role).unwrap();
            let bytes = std::fs::read(base.join(artifact["path"].as_str().unwrap())).unwrap();
            assert_eq!(sha(&bytes), artifact["sha256"].as_str().unwrap());
            let state_artifact = fixture["artifacts"].as_array().unwrap().iter().find(|a| a["role"] == state_role).unwrap();
            let state_bytes = std::fs::read(base.join(state_artifact["path"].as_str().unwrap())).unwrap();
            assert_eq!(sha(&state_bytes), state_artifact["sha256"].as_str().unwrap());
            let state: serde_json::Value = serde_json::from_slice(&state_bytes).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let mut found = 0;
            for mut layer in file.layers {
                if layer.text.is_none() && layer.vector.is_none() && layer.link.is_none() {
                    continue;
                }
                let expected = state["layers"].as_array().unwrap().iter().find(|r| r["tattoo"].as_u64() == layer.tattoo.map(u64::from)).unwrap();
                let native = &expected["native"];
                let native_bytes = std::fs::read(base.join("full").join(native["file"].as_str().unwrap())).unwrap();
                assert_eq!(sha(&native_bytes), native["sha256"].as_str().unwrap());
                let plane = layer.plane.take().unwrap();
                let stored_cache_hash = sha(&plane.bytes);
                let format = PixelFormat::new(photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8, layer.alpha);
                let mut resource = 200;
                let mapped = mapper::map_source(
                    &layer,
                    Some(plane),
                    mapper::SourceContext {
                        owner: ItemRef::Layer(LayerId(99)),
                        vector_path: Some(photocraft_doc::item_identity::PathId(777)),
                        format,
                        source_version: file.version,
                    },
                    &Interrupt::NONE,
                    &mut || {
                        resource += 1;
                        ResourceId(resource)
                    },
                    &mut |_| Ok(()),
                )
                .unwrap()
                .unwrap();
                assert_eq!(sha(&mapped.item.cache.as_ref().unwrap().bytes), stored_cache_hash, "stored cache changed: {name}/{role}");
                let native_equal = mapped.item.cache.as_ref().unwrap().bytes.as_slice() == native_bytes.as_slice();
                // These exact source originals regenerate on stock reopen. Their
                // stored input cache must remain different, never become a display oracle.
                let regenerates = role == "source" && matches!(name, "vector-basic" | "link-transform");
                assert_eq!(native_equal, !regenerates, "source-bound regeneration behavior changed: {name}/{role}");
                println!(
                    "{name}/{role}: stored-cache equals fresh GIMP native = {native_equal}, source-modified={}",
                    layer.vector.as_ref().is_some_and(|v| v.source_modified)
                );
                if layer.text.is_some() {
                    assert!(native_equal, "fresh native cache differs: {name}/{role}");
                    assert_eq!(mapped.item.cache.as_ref().unwrap().width as u64, expected["size"][0].as_u64().unwrap());
                    assert_eq!(mapped.item.cache.as_ref().unwrap().height as u64, expected["size"][1].as_u64().unwrap());
                }
                if let EditableSource::Text { content, font, source_modified, .. } = &mapped.item.source {
                    assert_eq!(content, expected["text"]["text"].as_str().unwrap());
                    assert_eq!(*source_modified, layer.text_source_modified());
                    // The stored legacy request can be "Arial" while GIMP's installed
                    // font getter resolves "Arial Regular". Preserve the request; do not
                    // turn this independent render-machine identity into a substitution.
                    let requested = layer.text.as_ref().unwrap().font.as_ref().unwrap();
                    assert_eq!(
                        mapped.resources.iter().find(|r| r.id == *font).unwrap().name,
                        requested.name.as_deref().or(requested.postscript_name.as_deref()).unwrap()
                    );
                    assert!(expected["text"]["font"].as_str().is_some());
                    let raw = &layer.text.as_ref().unwrap().raw;
                    let original = std::fs::read(base.join("full").join(expected["parasites"]["gimp-text-layer"]["file"].as_str().unwrap())).unwrap();
                    assert_eq!(raw, &original);
                    assert_eq!(mapped.opaque.iter().find(|r| r.namespace == "gimp/xcf/text-parasite").unwrap().bytes.as_slice(), raw.as_slice());
                }
                assert!(mapped.resources.iter().all(|r| r.owned_bytes.is_none() && r.content_hash.is_none()));
                found += 1;
            }
            assert_eq!(found, 1, "{name}/{role}");
        }
    }
}
#[test]
fn cancelled_mapping_and_nonfinite_raw_cache_never_activate_resources() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let source = Drawable {
        width: 1,
        height: 1,
        alpha: true,
        link: Some(LinkedLayer { flags: 0, path: "https://invalid.example/owned-inert".into(), width: 0, height: 0 }),
        ..Default::default()
    };
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) > 2;
    let result = mapper::map_source(
        &source,
        None,
        mapper::SourceContext { owner: ItemRef::Layer(LayerId(1)), vector_path: None, format: PixelFormat::RGBA8, source_version: 25 },
        &Interrupt::cancel_only(&cancel),
        &mut || ResourceId(3),
        &mut |_| Ok(()),
    );
    assert!(matches!(result, Err(IoError::Cancelled)));
    for (sample, format, bytes) in [
        (
            Sample::F32,
            PixelFormat::RGBA32F,
            [f32::NAN.to_bits(), (-0f32).to_bits(), f32::INFINITY.to_bits(), 1f32.to_bits()].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>(),
        ),
        (
            Sample::F64,
            PixelFormat::RGBA64F,
            [f64::NAN.to_bits(), (-0f64).to_bits(), f64::INFINITY.to_bits(), 1f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>(),
        ),
    ] {
        let output = mapper::map_source(
            &source,
            Some(Plane { width: 1, height: 1, channels: 4, sample, bytes: bytes.clone() }),
            mapper::SourceContext { owner: ItemRef::Layer(LayerId(1)), vector_path: None, format, source_version: 25 },
            &Interrupt::NONE,
            &mut || ResourceId(3),
            &mut |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(output.item.cache.as_ref().unwrap().bytes.as_slice(), bytes);
        output.item.cache.as_ref().unwrap().validate().unwrap();
    }
}
#[test]
fn native_roundtrip_preserves_inert_link_configuration_cache_and_remaps_identity() {
    let source = Drawable {
        width: 1,
        height: 1,
        alpha: true,
        link: Some(LinkedLayer { flags: 3, path: "../../missing-own-resource.png".into(), width: 99, height: 77 }),
        ..Default::default()
    };
    let layer = photocraft_doc::Layer::raster("Own linked source", PixelFormat::RGBA8);
    let owner = ItemRef::Layer(layer.id);
    let mapped = mapper::map_source(
        &source,
        Some(Plane { width: 1, height: 1, channels: 4, sample: Sample::U8, bytes: vec![1, 2, 3, 4] }),
        mapper::SourceContext { owner, vector_path: None, format: PixelFormat::RGBA8, source_version: 25 },
        &Interrupt::NONE,
        &mut || ResourceId(177),
        &mut |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    let item = mapped.item.clone();
    let mut doc = photocraft_doc::Document::new("Own", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    doc.layers.push(layer);
    doc.xcf_state = Some(photocraft_doc::xcf_state::XcfState {
        items: vec![photocraft_doc::xcf_state::ItemState {
            item: owner,
            name: "Own linked source".into(),
            custom_name: true,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: Some(77),
            source_flags: 3,
            raster_bounds: Some(photocraft_geom::Rect::new(0, 0, 1, 1)),
            mask: None,
            composition: None,
        }],
        source_items: vec![mapped.item],
        resources: mapped.resources,
        opaque: mapped.opaque,
        ..Default::default()
    });
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let saved = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(saved.xcf_state.as_ref().unwrap().source_items[0], item);
    let fresh = photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert_ne!(fresh.layers[0].id, doc.layers[0].id);
    assert_eq!(fresh.xcf_state.as_ref().unwrap().source_items[0].owner, ItemRef::Layer(fresh.layers[0].id));
    assert_eq!(fresh.xcf_state.as_ref().unwrap().source_items[0].cache, item.cache);
}
#[test]
fn text_run_font_schema_recursion_and_modified_cache_fail_closed() {
    use photocraft_xcf::{ConfigNode, ConfigValue, FontReference, TextLayer, TextLayout, TextRun};
    let text = TextLayer {
        parasite_flags: 1,
        raw: vec![],
        text: Some("Ω".into()),
        markup: None,
        font: Some(FontReference { name: Some("Own inert font".into()), ..Default::default() }),
        configuration: vec![],
        unknown_fields: vec![],
        content: "Ω".into(),
        runs: vec![TextRun { start: 0, end: 2, style_ids: vec![] }],
        styles: vec![],
        layout: TextLayout { direction: Some("ltr".into()), ..Default::default() },
    };
    let source = Drawable { text: Some(text), ..Default::default() };
    let context = || mapper::SourceContext { owner: ItemRef::Layer(LayerId(1)), vector_path: None, format: PixelFormat::RGBA8, source_version: 19 };
    let mut bad = source.clone();
    bad.text.as_mut().unwrap().runs[0].start = 1;
    assert!(mapper::map_source(&bad, None, context(), &Interrupt::NONE, &mut || ResourceId(5), &mut |_| Ok(())).is_err());
    let mut bad = source.clone();
    bad.text.as_mut().unwrap().font = None;
    assert!(mapper::map_source(&bad, None, context(), &Interrupt::NONE, &mut || ResourceId(5), &mut |_| Ok(())).is_err());
    let mut bad = source.clone();
    bad.text.as_mut().unwrap().unknown_fields.push("required-unmapped-setting".into());
    assert!(mapper::map_source(&bad, None, context(), &Interrupt::NONE, &mut || ResourceId(5), &mut |_| Ok(())).is_err());
    let mut bad = source.clone();
    bad.text_flags = 2;
    assert!(mapper::map_source(&bad, None, context(), &Interrupt::NONE, &mut || ResourceId(5), &mut |_| Ok(())).is_err());
    let mut node = ConfigNode { start: 0, end: 1, value: ConfigValue::Atom("own".into()) };
    for _ in 0..30 {
        node = ConfigNode { start: 0, end: 1, value: ConfigValue::List(vec![node]) };
    }
    let mut bad = source;
    bad.text.as_mut().unwrap().configuration = vec![node];
    assert!(mapper::map_source(&bad, None, context(), &Interrupt::NONE, &mut || ResourceId(5), &mut |_| Ok(())).is_err());
}
#[test]
fn source_cache_model_alpha_precision_and_declared_extent_are_runtime_data() {
    use photocraft_doc::{ColorMode, SampleType};
    use photocraft_xcf::Model;
    for (model, mode, colors) in [(Model::Rgb, ColorMode::Rgb, 3), (Model::Gray, ColorMode::Grayscale, 1)] {
        for alpha in [false, true] {
            for (sample, storage) in [
                (Sample::U8, SampleType::U8),
                (Sample::U16, SampleType::U16),
                (Sample::U32, SampleType::U32),
                (Sample::F16, SampleType::F32),
                (Sample::F32, SampleType::F32),
                (Sample::F64, SampleType::F64),
            ] {
                let channels = colors + usize::from(alpha);
                let bytes = (0..channels * storage.bytes()).map(|i| (i as u8).wrapping_mul(37)).collect::<Vec<_>>();
                let source = Drawable {
                    width: 1,
                    height: 1,
                    alpha,
                    model,
                    link: Some(LinkedLayer { flags: 0, path: "missing-own-runtime-data.png".into(), width: 0, height: 0 }),
                    ..Default::default()
                };
                let mapped = mapper::map_source(
                    &source,
                    Some(Plane { width: 1, height: 1, channels, sample, bytes: bytes.clone() }),
                    mapper::SourceContext {
                        owner: ItemRef::Layer(LayerId(1)),
                        vector_path: None,
                        format: PixelFormat::new(mode, storage, alpha),
                        source_version: 25,
                    },
                    &Interrupt::NONE,
                    &mut || ResourceId(2),
                    &mut |_| Ok(()),
                )
                .unwrap()
                .unwrap();
                let cache = mapped.item.cache.unwrap();
                assert_eq!(cache.bytes.as_slice(), bytes);
                assert_eq!(cache.format, PixelFormat::new(mode, storage, alpha));
                cache.validate().unwrap();
            }
        }
    }
    let source = Drawable {
        width: 1,
        height: 1,
        alpha: true,
        link: Some(LinkedLayer { flags: 0, path: "missing-own-hostile-cache.png".into(), width: 0, height: 0 }),
        ..Default::default()
    };
    for case in 0..8 {
        let mut source = source.clone();
        let mut plane = Plane { width: 1, height: 1, channels: 4, sample: Sample::U8, bytes: vec![1, 2, 3, 4] };
        let mut format = PixelFormat::RGBA8;
        match case {
            0 => plane.channels = usize::MAX,
            1 => format = PixelFormat::GRAYA8,
            2 => format.alpha = false,
            3 => plane.width = 2,
            4 => {
                plane.bytes.pop();
            }
            5 => {
                source.width = 262145;
                plane.width = source.width;
            }
            6 => {
                source.model = Model::Indexed;
            }
            _ => {
                plane.sample = Sample::F16;
                plane.bytes = vec![0; 8];
                format = PixelFormat::RGBA32F;
            }
        }
        assert!(
            mapper::map_source(
                &source,
                Some(plane),
                mapper::SourceContext { owner: ItemRef::Layer(LayerId(1)), vector_path: None, format, source_version: 25 },
                &Interrupt::NONE,
                &mut || ResourceId(2),
                &mut |_| Ok(())
            )
            .is_err(),
            "case {case}"
        );
    }
}
