use photocraft_doc::{
    LayerId, PixelFormat,
    item_identity::{ItemRef, PathId, ResourceId},
};
use photocraft_io::xcf_modified_sources as mapper;
use photocraft_io::xcf_sources::SourceContext;
use photocraft_raster::Interrupt;
use photocraft_xcf::{Drawable, Encoding, Plane, Sample};
#[test]
fn unmodified_missing_or_conflicting_source_never_uses_cache_as_renderer() {
    let source = Drawable { width: 1, height: 1, alpha: true, ..Default::default() };
    let plane = Plane { width: 1, height: 1, channels: 4, sample: Sample::U8, bytes: vec![1, 2, 3, 4] };
    let context = mapper::ModifiedSourceContext {
        mapping: SourceContext { owner: ItemRef::Layer(LayerId(1)), vector_path: Some(PathId(2)), format: PixelFormat::RGBA8, source_version: 19 },
        source_precision: 150,
        source_encoding: Encoding::NonlinearProfile,
        assigned_profile_present: false,
    };
    assert!(mapper::map_modified_source(&source, plane, context, &Interrupt::NONE, &mut || ResourceId(3), &mut |_| Ok(())).is_err());
}

fn context(source: &Drawable, version: u32, precision: u32, encoding: Encoding, assigned_profile_present: bool) -> mapper::ModifiedSourceContext {
    mapper::ModifiedSourceContext {
        mapping: SourceContext {
            owner: ItemRef::Layer(LayerId(77)),
            vector_path: source.vector.as_ref().map(|_| PathId(88)),
            format: PixelFormat::RGBA8,
            source_version: version,
        },
        source_precision: precision,
        source_encoding: encoding,
        assigned_profile_present,
    }
}
#[test]
#[ignore = "requires reviewed independently authored source cache corpus"]
fn originals_use_only_explicit_modified_buffer_and_preserve_native_cache() {
    use sha2::{Digest, Sha256};
    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf");
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("full/coverage-fixtures.json")).unwrap()).unwrap();
    let mut positives = 0;
    let mut rejected_unmodified = 0;
    for name in ["text-painted", "vector-modified"] {
        let fixture = manifest["fixtures"].as_array().unwrap().iter().find(|f| f["id"].as_str() == Some(&format!("feature002-gimp326-{name}"))).unwrap();
        for (role, state_role) in [("source", "reopened_state"), ("reference_edit", "edited_state")] {
            let artifacts = fixture["artifacts"].as_array().unwrap();
            let artifact = artifacts.iter().find(|a| a["role"] == role).unwrap();
            let bytes = std::fs::read(base.join(artifact["path"].as_str().unwrap())).unwrap();
            assert_eq!(sha(&bytes), artifact["sha256"].as_str().unwrap());
            let state_artifact = artifacts.iter().find(|a| a["role"] == state_role).unwrap();
            let state_bytes = std::fs::read(base.join(state_artifact["path"].as_str().unwrap())).unwrap();
            assert_eq!(sha(&state_bytes), state_artifact["sha256"].as_str().unwrap());
            let state: serde_json::Value = serde_json::from_slice(&state_bytes).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let mut source = file.layers.into_iter().find(|d| d.text.is_some() || d.vector.is_some()).unwrap();
            let expected = state["layers"].as_array().unwrap().iter().find(|r| r["tattoo"].as_u64() == source.tattoo.map(u64::from)).unwrap();
            let native = std::fs::read(base.join("full").join(expected["native"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(sha(&native), expected["native"]["sha256"].as_str().unwrap());
            if name == "vector-modified" || role == "reference_edit" {
                assert!(
                    mapper::validate_modified_source(
                        &source,
                        &context(&source, file.version, file.precision, file.encoding, !file.icc.is_empty()),
                        &Interrupt::NONE
                    )
                    .unwrap_err()
                    .to_string()
                    .contains("regeneration")
                );
                rejected_unmodified += 1;
                continue;
            }
            let plane = source.plane.take().unwrap();
            let stored_hash = blake3::hash(&plane.bytes).to_hex().to_string();
            let mut resource = 100;
            let output = mapper::map_modified_source(
                &source,
                plane,
                context(&source, file.version, file.precision, file.encoding, !file.icc.is_empty()),
                &Interrupt::NONE,
                &mut || {
                    resource += 1;
                    ResourceId(resource)
                },
                &mut |_| Ok(()),
            )
            .unwrap();
            let cache = output.mapped.item.cache.as_ref().unwrap();
            cache.validate().unwrap();
            assert_eq!(cache.bytes.as_slice(), native);
            assert_eq!(cache.blake3, stored_hash);
            assert_eq!(cache.width as u64, expected["size"][0].as_u64().unwrap());
            assert_eq!(cache.height as u64, expected["size"][1].as_u64().unwrap());
            assert!(output.warning.contains("regeneration is unavailable"));
            assert!(output.mapped.resources.iter().all(|r| r.owned_bytes.is_none() || r.kind == photocraft_doc::xcf_resources::ResourceKind::ColorProfile));
            assert!(output.mapped.resources.iter().all(|r| r.locator.is_none()));
            if name == "text-painted" {
                assert_eq!(output.kind, mapper::ModifiedSourceKind::Text);
            } else {
                assert_eq!(output.kind, mapper::ModifiedSourceKind::Vector);
            }
            positives += 1;
        }
    }
    assert_eq!((positives, rejected_unmodified), (1, 3));
}
#[test]
#[ignore = "requires reviewed modified text original"]
fn hostile_descriptors_flags_contexts_and_budgets_reject_before_publication() {
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-painted.xcf")).unwrap();
    let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    let original = file.layers.into_iter().find(|d| d.text.is_some()).unwrap();
    for variant in 0..12 {
        let mut source = original.clone();
        let mut ctx = context(&source, file.version, file.precision, file.encoding, false);
        match variant {
            0 => source.text_flags |= 4,
            1 => source.text_flags &= !2,
            2 => source.offset.0 = i32::MAX,
            3 => ctx.source_precision = 650,
            4 => ctx.source_encoding = Encoding::Linear,
            5 => ctx.assigned_profile_present = true,
            6 => ctx.mapping.source_version = 26,
            7 => ctx.mapping.format = PixelFormat::RGBA32F,
            8 => source.mode = 30,
            9 => source.composite_space = 2,
            10 => source.unsupported.push("unknown layer property 777".into()),
            11 => source.opacity = 0.5,
            _ => panic!("variant"),
        }
        assert!(mapper::validate_modified_source(&source, &ctx, &Interrupt::NONE).is_err(), "variant{variant}");
    }
    let mut source = original.clone();
    let mut plane = source.plane.take().unwrap();
    plane.bytes.pop();
    assert!(
        mapper::map_modified_source(
            &source,
            plane,
            context(&source, file.version, file.precision, file.encoding, false),
            &Interrupt::NONE,
            &mut || ResourceId(3),
            &mut |_| Ok(())
        )
        .is_err()
    );
    let mut source = original.clone();
    let mut plane = source.plane.take().unwrap();
    plane.sample = Sample::U16;
    assert!(
        mapper::map_modified_source(
            &source,
            plane,
            context(&source, file.version, file.precision, file.encoding, false),
            &Interrupt::NONE,
            &mut || ResourceId(3),
            &mut |_| Ok(())
        )
        .is_err()
    );
    let mut source = original.clone();
    let plane = source.plane.take().unwrap();
    assert!(
        mapper::map_modified_source(
            &source,
            plane,
            context(&source, file.version, file.precision, file.encoding, false),
            &Interrupt::NONE,
            &mut || ResourceId(3),
            &mut |_| Err(photocraft_io::IoError::Unsupported("caller budget denied".into()))
        )
        .unwrap_err()
        .to_string()
        .contains("caller budget denied")
    );
    assert!(matches!(
        mapper::validate_modified_source(&original, &context(&original, file.version, file.precision, file.encoding, false), &Interrupt::cancel_only(&|| true)),
        Err(photocraft_io::IoError::Cancelled)
    ));
}

#[test]
#[ignore = "requires reviewed modified text original"]
fn original_modified_text_native_roundtrip_retains_cache_resources_and_fresh_owner() {
    use photocraft_doc::{
        ColorMode, Document, Layer, SampleType,
        xcf_state::{ItemState, XcfState},
    };
    use photocraft_geom::{Rect, Size};
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf");
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("full/coverage-fixtures.json")).unwrap()).unwrap();
    let fixture = manifest["fixtures"].as_array().unwrap().iter().find(|row| row["id"] == "feature002-gimp326-text-painted").unwrap();
    let artifact = fixture["artifacts"].as_array().unwrap().iter().find(|row| row["role"] == "source").unwrap();
    let bytes = std::fs::read(base.join(artifact["path"].as_str().unwrap())).unwrap();
    let digest: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest, artifact["sha256"].as_str().unwrap());
    let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    let mut source = file.layers.into_iter().find(|layer| layer.text.is_some()).unwrap();
    let mut layer = Layer::raster(&source.name, PixelFormat::RGBA8);
    let owner = ItemRef::Layer(layer.id);
    let bounds = Rect::new(
        source.offset.0,
        source.offset.1,
        source.offset.0 + i32::try_from(source.width).unwrap(),
        source.offset.1 + i32::try_from(source.height).unwrap(),
    );
    let plane = source.plane.take().unwrap();
    layer.content = photocraft_doc::LayerContent::Raster(photocraft_raster::Surface::try_from_interleaved(PixelFormat::RGBA8, bounds, &plane.bytes).unwrap());
    let mut ctx = context(&source, file.version, file.precision, file.encoding, !file.icc.is_empty());
    ctx.mapping.owner = owner;
    let mapped = mapper::map_modified_source(&source, plane, ctx, &Interrupt::NONE, &mut || ResourceId(101), &mut |_| Ok(())).unwrap().mapped;
    let expected = mapped.item.clone();
    let resources = mapped.resources.clone();
    let mut doc = Document::new("Own modified text", Size::new(file.width, file.height), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(layer);
    doc.xcf_state = Some(XcfState {
        items: vec![ItemState {
            item: owner,
            name: source.name,
            custom_name: true,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: source.tattoo,
            source_flags: source.text_flags,
            raster_bounds: Some(bounds),
            mask: None,
            composition: None,
        }],
        source_items: vec![mapped.item],
        resources: mapped.resources,
        opaque: mapped.opaque,
        ..Default::default()
    });
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let saved = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(saved.xcf_state.as_ref().unwrap().source_items[0], expected);
    assert_eq!(saved.xcf_state.as_ref().unwrap().resources, resources);
    let fresh = photocraft_format::load_from_bytes_with(&native, &photocraft_format::LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let fresh_item = &fresh.xcf_state.as_ref().unwrap().source_items[0];
    assert_eq!(fresh_item.owner, ItemRef::Layer(fresh.layers[0].id));
    assert_ne!(fresh_item.owner, expected.owner);
    assert_eq!(fresh_item.cache, expected.cache);
    assert_eq!(fresh_item.source, expected.source);
}
