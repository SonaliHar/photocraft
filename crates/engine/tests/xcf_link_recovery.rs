use photocraft_doc::{
    Color, ColorMode, Document, SampleType, Size, XcfState,
    item_identity::{ItemRef, ResourceId},
    xcf_items::{EditableSource, SourceCache, SourceItem},
    xcf_resources::*,
};
use photocraft_engine::{Session, xcf_resources::*};
use photocraft_raster::Interrupt;
use serde_json::json;
fn project(mode: ColorMode, depth: SampleType) -> (Session, Vec<u8>, Vec<u8>) {
    let mut doc = Document::with_background("Own linked image", Size::new(2, 1), mode, depth, Color::WHITE);
    doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(if mode == ColorMode::Rgb {
        photocraft_cms::builtin::srgb().to_bytes()
    } else {
        photocraft_cms::builtin::Builtin::GrayGamma22.profile().to_bytes()
    });
    let bounds = doc.bounds();
    let format = doc.pixel_format();
    let id = doc.layers[0].id;
    let raw: Vec<u8> = if depth == SampleType::U8 {
        (0..format.bytes_per_pixel() * 2).map(|i| (i * 23 + 7) as u8).collect()
    } else {
        (0..format.channels() * 2).flat_map(|i| ((i * 4243 + 29) as u16).to_ne_bytes()).collect()
    };
    let mut image = photocraft_codecs::Image::from_raw(
        2,
        1,
        if mode == ColorMode::Rgb { photocraft_codecs::ChannelLayout::Rgba } else { photocraft_codecs::ChannelLayout::GrayA },
        if depth == SampleType::U8 { photocraft_codecs::SampleType::U8 } else { photocraft_codecs::SampleType::U16 },
        raw.clone(),
    )
    .unwrap();
    image.icc = doc.icc_profile.as_deref().cloned();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let original = doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.items[0].raster_bounds = Some(bounds);
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own PNG".into(),
        collection: None,
        internal: false,
        locator: Some("file:///never-authorized".into()),
        content_hash: Some(blake3::hash(&png).to_hex().to_string()),
        owned_bytes: None,
        resolution: None,
    });
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(id),
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: Some(SourceCache { blake3: blake3::hash(&original).to_hex().to_string(), bytes: original.into(), width: 2, height: 1, format }),
    });
    doc.xcf_state = Some(state);
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    (s, png, raw)
}
#[test]
fn typed_link_refresh_uses_recovered_bytes_preserves_original_cache_and_native_history() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16] {
            let (mut s, png, raw) = project(mode, depth);
            let before = s.active().unwrap().doc.clone();
            let id = before.layers[0].id;
            assert!(s.execute("xcf.link.refresh", json!({"layerId":id.0.to_string()})).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            recover_authorized_bytes(
                &mut s,
                ResourceId(1),
                ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, png, true).unwrap(),
                RecoveryPolicy::Exact,
                &Interrupt::NONE,
            )
            .unwrap();
            let recovered = s.active().unwrap().doc.clone();
            assert!(
                photocraft_engine::xcf_link_cmds::refresh_with_interrupt(&mut s, &json!({"layerId":id.0.to_string()}), &Interrupt::cancel_only(&|| true))
                    .is_err()
            );
            assert!(s.active().unwrap().doc.shares_state(&recovered));
            let mut locked = (*recovered).clone();
            locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().locks.content = true;
            let mut denied = Session::new();
            denied.open_document(locked.clone(), None).unwrap();
            assert!(denied.execute("xcf.link.refresh", json!({"layerId":id.0.to_string()})).is_err());
            assert_eq!(*denied.active().unwrap().doc, locked);
            assert!(!denied.active().unwrap().history.can_undo());
            assert_eq!(s.execute("xcf.resource.inspect", json!({"resource":"1"})).unwrap()["resources"][0]["activated"], false);
            s.execute("xcf.link.refresh", json!({"layerId":id.0.to_string()})).unwrap();
            let after = s.active().unwrap().doc.clone();
            let inspected = s.execute("xcf.resource.inspect", json!({"resource":"1"})).unwrap();
            assert_eq!(inspected["resources"][0]["activated"], true);
            assert_eq!(inspected["resources"][0]["inert"], false);
            assert_eq!(after.layers[0].surface().unwrap().try_read_interleaved(after.bounds()).unwrap(), raw);
            assert_eq!(after.xcf_state.as_ref().unwrap().source_items[0].cache, before.xcf_state.as_ref().unwrap().source_items[0].cache);
            photocraft_compose::gimp_render::PreparedRenderer::new(&after).unwrap();
            assert_eq!(s.execute("xcf.link.refresh", json!({"layerId":id.0.to_string()})).unwrap()["refreshed"], false);
            let native = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *after);
            if mode == ColorMode::Rgb && depth == SampleType::U8 {
                let mut assigned = Session::new();
                assigned.open_document((*after).clone(), None).unwrap();
                assigned.execute("edit.assignProfile", json!({"profile":"linear-srgb"})).unwrap();
                let pending_profile = assigned.active().unwrap().doc.clone();
                assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&assigned.active().unwrap().doc).is_err());
                assigned.execute("xcf.link.refresh", json!({"layerId":id.0.to_string()})).unwrap();
                let refreshed_profile = assigned.active().unwrap().doc.clone();
                assert_ne!(refreshed_profile.layers[0].surface(), after.layers[0].surface());
                assert_eq!(refreshed_profile.xcf_state.as_ref().unwrap().source_items, after.xcf_state.as_ref().unwrap().source_items);
                assert_eq!(refreshed_profile.xcf_state.as_ref().unwrap().provenance, after.xcf_state.as_ref().unwrap().provenance);
                assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&assigned.active().unwrap().doc).is_ok());
                let native = photocraft_format::save_to_bytes(&refreshed_profile, &Default::default()).unwrap();
                assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *refreshed_profile);
                assigned.execute("edit.undo", json!({})).unwrap();
                assert!(assigned.active().unwrap().doc.shares_state(&pending_profile));
                assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&assigned.active().unwrap().doc).is_err());
                assigned.execute("edit.undo", json!({})).unwrap();
                assert_eq!(*assigned.active().unwrap().doc, *after);
                assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&assigned.active().unwrap().doc).is_ok());
            }
            let mut forged_current = (*after).clone();
            forged_current.layers[0].content = before.layers[0].content.clone();
            assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&forged_current).is_err());
            let mut stale_session = Session::new();
            stale_session.open_document(forged_current, None).unwrap();
            assert_eq!(stale_session.execute("xcf.resource.inspect", json!({"resource":"1"})).unwrap()["resources"][0]["activated"], false);
            s.execute("edit.undo", json!({})).unwrap();
            assert!(s.active().unwrap().doc.shares_state(&recovered));
            assert_eq!(s.execute("xcf.resource.inspect", json!({"resource":"1"})).unwrap()["resources"][0]["activated"], false);
            s.execute("edit.redo", json!({})).unwrap();
            assert!(s.active().unwrap().doc.shares_state(&after));
        }
    }
}

#[cfg(feature = "corpus")]
#[test]
fn real_projective_link_import_recovery_refresh_and_query_preserve_provenance() {
    for kind in ["none", "linear", "cubic"] {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/transform-interpolation");
        let source = std::fs::read(root.join(format!("link-{kind}.xcf"))).unwrap();
        let supplied = std::fs::read(root.join("own-linked-resource.png")).unwrap();
        let imported = photocraft_io::import(&format!("link-{kind}.xcf"), &source).unwrap();
        let original = imported.document;
        let state = original.xcf_state.as_ref().unwrap();
        let linked = state.source_items.iter().find(|entry| matches!(entry.source, EditableSource::Linked { .. })).unwrap();
        let ItemRef::Layer(layer) = linked.owner else { panic!("expected linked owner") };
        let EditableSource::Linked { resource, .. } = linked.source else { panic!("expected linked source") };
        let cache = linked.cache.clone();
        let transform = linked.transform.clone();
        let mut session = Session::new();
        session.open_document(original.clone(), None).unwrap();
        let original_snapshot = session.active().unwrap().doc.clone();
        let report = session.execute("xcf.assess", json!({})).unwrap();
        assert_eq!(report["fidelity"], "dependency_limited");
        assert!(report["diagnostics"].as_array().unwrap().iter().any(|entry| entry["code"] == "missing_resource"));
        assert!(session.active().unwrap().doc.shares_state(&original_snapshot));
        assert!(
            recover_authorized_bytes(
                &mut session,
                resource,
                ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, supplied.clone(), true).unwrap(),
                RecoveryPolicy::Exact,
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(session.active().unwrap().doc.shares_state(&original_snapshot));
        recover_authorized_bytes(
            &mut session,
            resource,
            ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, supplied, true).unwrap(),
            RecoveryPolicy::AllowSubstitution,
            &Interrupt::NONE,
        )
        .unwrap();
        let recovered = session.active().unwrap().doc.clone();
        assert_eq!(session.execute("xcf.assess", json!({})).unwrap()["fidelity"], "reduced");
        assert!(session.active().unwrap().doc.shares_state(&recovered));
        assert_eq!(session.execute("xcf.resource.inspect", json!({"resource":resource.0.to_string()})).unwrap()["resources"][0]["activated"], false);
        session.execute("xcf.link.refresh", json!({"layerId":layer.0.to_string()})).unwrap();
        let current = session.active().unwrap().doc.clone();
        assert_eq!(session.execute("xcf.assess", json!({})).unwrap()["fidelity"], "reduced");
        assert!(session.active().unwrap().doc.shares_state(&current));
        assert_eq!(session.execute("xcf.resource.inspect", json!({"resource":resource.0.to_string()})).unwrap()["resources"][0]["activated"], true);
        // Frozen independent GIMP native plane, including its off-canvas extent.
        let bounds = state.items.iter().find(|item| item.item == linked.owner).unwrap().raster_bounds.unwrap();
        let pixels = current.layer(layer).unwrap().surface().unwrap().try_read_interleaved(bounds).unwrap();
        let oracle = std::fs::read(root.join(format!("link-{kind}.source.0.native.bin"))).unwrap();
        assert_eq!(pixels.len(), oracle.len());
        assert!(
            pixels.iter().zip(&oracle).all(|(actual, expected)| actual.abs_diff(*expected) <= 2),
            "{kind} current CMS conversion exceeded independent native 2/255 channel floor"
        );
        assert_eq!(current.working_profile_interpretation, original.working_profile_interpretation);
        assert_eq!(current.icc_profile, original.icc_profile);
        let current_state = current.xcf_state.as_ref().unwrap();
        let current_link = current_state.source_items.iter().find(|entry| entry.owner == linked.owner).unwrap();
        assert_eq!(current_link.cache, cache);
        assert_eq!(current_link.transform, transform);
        assert_eq!(current_state.provenance, state.provenance);
        assert!(current_state.resources.iter().find(|entry| entry.id == resource).unwrap().content_hash.is_none());
        let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened, *current);
        assert_eq!(
            photocraft_io::xcf_assessment::assess_xcf_document(&reopened, &Interrupt::NONE).unwrap().unwrap().fidelity,
            photocraft_io::xcf_assessment::Fidelity::Reduced
        );
        let mut assigned = Session::new();
        assigned.open_document((*current).clone(), None).unwrap();
        assigned.execute("edit.assignProfile", json!({"profile":"linear-srgb"})).unwrap();
        let pending_profile = assigned.active().unwrap().doc.clone();
        assert!(assigned.execute("xcf.assess", json!({})).is_err());
        assigned.execute("xcf.link.refresh", json!({"layerId":layer.0.to_string()})).unwrap();
        let refreshed_profile = assigned.active().unwrap().doc.clone();
        assert_eq!(assigned.execute("xcf.assess", json!({})).unwrap()["fidelity"], "reduced");
        assert_ne!(refreshed_profile.layer(layer).unwrap().surface(), current.layer(layer).unwrap().surface());
        assert_eq!(refreshed_profile.xcf_state.as_ref().unwrap().source_items, current_state.source_items);
        assert_eq!(refreshed_profile.xcf_state.as_ref().unwrap().provenance, current_state.provenance);
        let native = photocraft_format::save_to_bytes(&refreshed_profile, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened, *refreshed_profile);
        assert_eq!(
            photocraft_io::xcf_assessment::assess_xcf_document(&reopened, &Interrupt::NONE).unwrap().unwrap().fidelity,
            photocraft_io::xcf_assessment::Fidelity::Reduced
        );
        assigned.execute("edit.undo", json!({})).unwrap();
        assert!(assigned.active().unwrap().doc.shares_state(&pending_profile));
        assert!(assigned.execute("xcf.assess", json!({})).is_err());
        assigned.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*assigned.active().unwrap().doc, *current);
        assert_eq!(assigned.execute("xcf.assess", json!({})).unwrap()["fidelity"], "reduced");
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&recovered));
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, original);
        assert_eq!(session.execute("xcf.assess", json!({})).unwrap()["fidelity"], "dependency_limited");
        session.execute("edit.redo", json!({})).unwrap();
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&current));
        assert_eq!(std::fs::read(root.join(format!("link-{kind}.xcf"))).unwrap(), source);
    }
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires independently captured RGBA8 Cubic packet; set PHOTOCRAFT_XCF_CUBIC_PACKET"]
fn canonical_cubic_link_recovery_consumes_independently_observed_mixed_and_hidden_native_material() {
    canonical_link_native_recovery("PHOTOCRAFT_XCF_CUBIC_PACKET", None, 2, 6);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires independently captured RGBA8 modern NoHalo packet; set PHOTOCRAFT_XCF_HALO_PACKET"]
fn canonical_nohalo_link_recovery_consumes_independently_observed_mixed_and_hidden_native_material() {
    canonical_link_native_recovery("PHOTOCRAFT_XCF_HALO_PACKET", Some("nohalo-u8_non_linear-rgb-"), 3, 4);
}

#[cfg(feature = "corpus")]
fn canonical_link_native_recovery(packet_env: &str, prefix: Option<&str>, interpolation: i32, expected_count: usize) {
    use photocraft_doc::{
        LayerContent, Rect,
        xcf_filters::{ExactNumber, FilterArgument, FilterValue},
    };
    use sha2::{Digest, Sha256};
    let base = std::path::PathBuf::from(std::env::var(packet_env).unwrap());
    let mut count = 0;
    for name in ["packet", "hidden-packet"] {
        let root = base.join(name);
        let packet: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let read = |entry: &serde_json::Value| {
            let bytes = std::fs::read(root.join(entry["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), entry["sha256"].as_str().unwrap());
            bytes
        };
        for record in packet["records"].as_array().unwrap() {
            if prefix.is_some_and(|value| !record["id"].as_str().unwrap().starts_with(value)) {
                continue;
            }
            let input = &record["fresh_source"];
            let edited = &record["fresh_edit"];
            let raw = read(&input["native"]);
            let oracle = read(&edited["native"]);
            let profile = read(&input["profiles"]["effective"]);
            let source_xcf = read(&record["source"]);
            let edit_xcf = read(&record["edit"]);
            let width = input["size"][0].as_u64().unwrap() as u32;
            let height = input["size"][1].as_u64().unwrap() as u32;
            let x = edited["offset"][0].as_i64().unwrap() as i32;
            let y = edited["offset"][1].as_i64().unwrap() as i32;
            let bounds = Rect::new(x, y, x + edited["size"][0].as_u64().unwrap() as i32, y + edited["size"][1].as_u64().unwrap() as i32);
            let mut doc = Document::with_background("Own canonical linked cubic evidence", Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
            doc.icc_profile = Some(std::sync::Arc::new(profile.clone()));
            doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
            let id = doc.layers[0].id;
            doc.layers[0].content = LayerContent::Raster(photocraft_raster::Surface::try_from_interleaved(doc.pixel_format(), bounds, &oracle).unwrap());
            let mut image =
                photocraft_codecs::Image::from_raw(width, height, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, raw).unwrap();
            image.icc = Some(profile);
            let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
            let mut state = XcfState::default();
            state.provenance.format = "xcf".into();
            state.provenance.format_version = 25;
            state.provenance.source_label = "Own synthetic canonical XCF resource state".into();
            state.reconcile_layers(&doc).unwrap();
            state.items[0].raster_bounds = Some(bounds);
            state.resources.push(ResourceReference {
                id: ResourceId(1),
                kind: ResourceKind::LinkedImage,
                name: "Own native source literals".into(),
                collection: None,
                internal: false,
                locator: Some("file:///inert-ungranted-source".into()),
                content_hash: Some(blake3::hash(&png).to_hex().to_string()),
                owned_bytes: None,
                resolution: None,
            });
            let argument = |name: &str, value: u32| FilterArgument {
                name: name.into(),
                value: FilterValue::Number(ExactNumber::Unsigned { width: 32, value: u64::from(value) }),
            };
            state.source_items.push(SourceItem {
                owner: ItemRef::Layer(id),
                source: EditableSource::Linked {
                    resource: ResourceId(1),
                    last_verified_hash: None,
                    current: None,
                    configuration: vec![argument("flags", 1), argument("source-width", width), argument("source-height", height)],
                },
                transform: Some(photocraft_doc::xcf_items::TransformState {
                    matrix_f64_bits: record["matrix"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap().to_bits()).collect::<Vec<_>>().try_into().unwrap(),
                    pre_offset: [input["offset"][0].as_i64().unwrap() as i32, input["offset"][1].as_i64().unwrap() as i32],
                    interpolation,
                }),
                cache: Some(SourceCache {
                    blake3: blake3::hash(&oracle).to_hex().to_string(),
                    bytes: oracle.clone().into(),
                    width: bounds.width(),
                    height: bounds.height(),
                    format: doc.pixel_format(),
                }),
            });
            doc.xcf_state = Some(state);
            let original = doc.clone();
            let mut session = Session::new();
            session.open_document(doc, None).unwrap();
            let original_snapshot = session.active().unwrap().doc.clone();
            assert_eq!(session.execute("xcf.assess", json!({})).unwrap()["fidelity"], "dependency_limited");
            recover_authorized_bytes(
                &mut session,
                ResourceId(1),
                ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, png, true).unwrap(),
                RecoveryPolicy::Exact,
                &Interrupt::NONE,
            )
            .unwrap();
            let pending = session.active().unwrap().doc.clone();
            assert_eq!(session.execute("xcf.resource.inspect", json!({"resource":"1"})).unwrap()["resources"][0]["activated"], false);
            assert!(
                photocraft_engine::xcf_link_cmds::refresh_with_interrupt(&mut session, &json!({"layerId":id.0.to_string()}), &Interrupt::cancel_only(&|| true))
                    .is_err()
            );
            assert!(session.active().unwrap().doc.shares_state(&pending));
            session.execute("xcf.link.refresh", json!({"layerId":id.0.to_string()})).unwrap();
            let current = session.active().unwrap().doc.clone();
            assert_eq!(session.execute("xcf.assess", json!({})).unwrap()["fidelity"], "full");
            assert_eq!(session.execute("xcf.resource.inspect", json!({"resource":"1"})).unwrap()["resources"][0]["activated"], true);
            let actual = current.layer(id).unwrap().surface().unwrap().try_read_interleaved(bounds).unwrap();
            assert_eq!(actual.len(), oracle.len());
            assert!(actual.iter().zip(&oracle).all(|(a, b)| a.abs_diff(*b) <= 2));
            let source = &current.xcf_state.as_ref().unwrap().source_items[0];
            let before = &original.xcf_state.as_ref().unwrap().source_items[0];
            assert_eq!(source.cache, before.cache);
            assert_eq!(source.transform, before.transform);
            let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(reopened, *current);
            assert_eq!(
                photocraft_io::xcf_assessment::assess_xcf_document(&reopened, &Interrupt::NONE).unwrap().unwrap().fidelity,
                photocraft_io::xcf_assessment::Fidelity::Full
            );
            session.execute("edit.undo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&pending));
            session.execute("edit.undo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&original_snapshot));
            session.execute("edit.redo", json!({})).unwrap();
            session.execute("edit.redo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&current));
            assert_eq!(read(&record["source"]), source_xcf);
            assert_eq!(read(&record["edit"]), edit_xcf);
            count += 1;
        }
    }
    assert_eq!(count, expected_count);
}
