//! Current linked geometry: owned pixels, immutable original state, and atomic history.
#[path = "common/accounted_resource.rs"]
mod accounted_fixture;
use photocraft_doc::{
    Color, ColorMode, Document, Layer, LayerId, SampleEncoding, SampleType, Size, XcfState,
    item_identity::{ItemRef, ResourceId},
    xcf_items::{EditableSource, SourceCache, SourceItem},
    xcf_resources::{ResourceFidelity, ResourceKind, ResourceReference, ResourceResolution},
};
use photocraft_engine::{DocumentSnapshot, Session};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
fn fixture(mode: ColorMode, depth: SampleType, resolved: bool) -> (Session, LayerId, Vec<u8>) {
    let mut doc = Document::with_background("Own linked geometry", Size::new(2, 1), mode, depth, Color::WHITE);
    // This synthetic linked owner is editable, unlike a newly created Background.
    doc.layers[0].locks = Default::default();
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(if mode == ColorMode::Rgb {
        photocraft_cms::builtin::srgb().to_bytes()
    } else {
        photocraft_cms::builtin::Builtin::GrayGamma22.profile().to_bytes()
    });
    let owner = doc.layers[0].id;
    let format = doc.pixel_format();
    let channels = format.channels();
    let raw: Vec<u8> = if depth == SampleType::U8 {
        (0..channels * 2).map(|index| if index % channels == channels - 1 { if index < channels { 0 } else { 255 } } else { (index * 23 + 7) as u8 }).collect()
    } else {
        (0..channels * 2)
            .flat_map(|index| {
                let value = if index % channels == channels - 1 { if index < channels { 0 } else { u16::MAX } } else { (index * 4243 + 29) as u16 };
                value.to_ne_bytes()
            })
            .collect()
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
    let digest = blake3::hash(&png).to_hex().to_string();
    let original = doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap();
    let other = Layer::raster("Own ordinary active layer", format);
    let other_id = other.id;
    doc.layers.push(other);
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(doc.bounds());
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own PNG".into(),
        collection: None,
        internal: false,
        locator: Some("inert/original-locator".into()),
        content_hash: Some(digest.clone()),
        owned_bytes: resolved.then(|| png.into()),
        resolution: resolved.then_some(ResourceResolution { fidelity: ResourceFidelity::Exact, payload_hash: digest }),
    });
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(owner),
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: Some(SourceCache { blake3: blake3::hash(&original).to_hex().to_string(), bytes: original.into(), width: 2, height: 1, format }),
    });
    doc.xcf_state = Some(state);
    let mut session = Session::new();
    session.open_document(doc, None).unwrap();
    session.active_mut().unwrap().active_layer = Some(other_id);
    session.active_mut().unwrap().selected_layers = vec![other_id];
    (session, owner, raw)
}
fn enabled(session: &Session) -> bool {
    photocraft_engine::xcf_link_geometry_cmds::specs().iter().all(|spec| (spec.enabled)(session).is_ok())
}
// Draft for the source-payload-sharing wave; execution is deferred to the batch gate.
#[test]
fn metadata_history_shares_immutable_source_cache_and_owned_material() {
    let (mut session, owner, _) = fixture(ColorMode::Rgb, SampleType::U16, true);
    let before = session.active().unwrap().doc.clone();
    let state = before.xcf_state.as_ref().unwrap();
    let original_cache = state.source_items.first().unwrap().cache.as_ref().unwrap().bytes.clone();
    let original_material = state.resources.first().unwrap().owned_bytes.as_ref().unwrap().clone();
    session.execute("layer.renameLayer", json!({"layer":owner.0,"name":"Own metadata successor"})).unwrap();
    let state = session.active().unwrap().doc.xcf_state.as_ref().unwrap();
    assert!(std::sync::Arc::ptr_eq(&original_cache, &state.source_items.first().unwrap().cache.as_ref().unwrap().bytes));
    assert!(std::sync::Arc::ptr_eq(&original_material, state.resources.first().unwrap().owned_bytes.as_ref().unwrap()));
    let mut private = (*session.active().unwrap().doc).clone();
    let private_cache = &mut private.xcf_state.as_mut().unwrap().source_items.first_mut().unwrap().cache.as_mut().unwrap().bytes;
    std::sync::Arc::make_mut(private_cache)[0] ^= 1;
    assert_ne!(private_cache.as_slice(), original_cache.as_slice());
    assert_eq!(before.xcf_state.as_ref().unwrap().source_items.first().unwrap().cache.as_ref().unwrap().bytes.as_slice(), original_cache.as_slice());
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before);
}
fn bits(matrix: [f64; 9]) -> Value {
    json!(matrix.map(|value| format!("{:016x}", value.to_bits())))
}
#[test]
fn placement_preserves_native_depth_hidden_alpha_source_identity_and_history() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16] {
            let (mut session, owner, raw) = fixture(mode, depth, true);
            assert!(enabled(&session), "explicit linked target may differ from ordinary active layer");
            let before = session.active().unwrap().doc.clone();
            let original = &before.xcf_state.as_ref().unwrap().source_items[0];
            let params = json!({"layerId":owner.0.to_string(),"originPx":[-3,2]});
            assert_eq!(session.execute("xcf.link.setPlacement", params.clone()).unwrap()["changed"], true);
            let current = session.active().unwrap().doc.clone();
            let state = current.xcf_state.as_ref().unwrap();
            let bounds = state.items.iter().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds.unwrap();
            assert_eq!(bounds, photocraft_doc::Rect::new(-3, 2, -1, 3));
            assert_eq!(current.layer(owner).unwrap().surface().unwrap().try_read_interleaved(bounds).unwrap(), raw);
            assert_eq!(state.source_items[0].transform, original.transform);
            assert_eq!(state.source_items[0].cache, original.cache);
            assert_eq!(state.resources, before.xcf_state.as_ref().unwrap().resources);
            assert_eq!(state.provenance, before.xcf_state.as_ref().unwrap().provenance);
            assert_eq!(session.execute("xcf.link.setPlacement", params).unwrap()["changed"], false);
            assert!(session.active().unwrap().doc.shares_state(&current));
            let archive = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
            assert_eq!(photocraft_format::read_manifest(&archive).unwrap().minimum_reader_version, 7);
            assert_eq!(photocraft_format::load_from_bytes(&archive).unwrap(), *current);
            session.execute("edit.undo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&before));
            session.execute("edit.redo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&current));
        }
    }
}
#[test]
fn geometry_enabled_queries_reject_unresolved_ordinary_locked_and_accounted_documents() {
    let (mut session, owner, _) = fixture(ColorMode::Rgb, SampleType::U8, false);
    assert!(!enabled(&session));
    let before = session.active().unwrap().doc.clone();
    assert!(session.execute("xcf.link.setPlacement", json!({"layerId":owner.0.to_string(),"originPx":[0,0]})).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    let mut ordinary = Session::new();
    ordinary.open_document(Document::with_background("ordinary", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE), None).unwrap();
    assert!(!enabled(&ordinary));
    for position in [false, true] {
        let (mut session, owner, _) = fixture(ColorMode::Rgb, SampleType::U8, true);
        let state = session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap();
        let locks = &mut state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().locks;
        if position {
            locks.position = true;
        } else {
            locks.content = true;
        }
        assert!(!enabled(&session));
        let before = session.active().unwrap().doc.clone();
        assert!(
            photocraft_engine::xcf_link_geometry_cmds::set_placement_with_interrupt(
                &mut session,
                &json!({"layerId":owner.0.to_string(),"originPx":[1,1]}),
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    use photocraft_doc::accounted_document::AccountedDocument;
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    };
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 0, index: 1 << 20 });
    let retained = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let raw = accounted_fixture::document(&quota, &ctl);
    let accounted = AccountedDocument::from_owned(raw, std::sync::Arc::new(retained), quota, &ctl).unwrap();
    ordinary.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(accounted).unwrap();
    assert!(!enabled(&ordinary));
    let before = ordinary.active().unwrap().doc.clone();
    let result = photocraft_engine::xcf_link_geometry_cmds::set_placement_with_interrupt(
        &mut ordinary,
        &json!({"layerId":owner.0.to_string(),"originPx":[0,0]}),
        &Interrupt::NONE,
    );
    assert!(result.unwrap_err().to_string().contains("resident"));
    assert!(ordinary.active().unwrap().doc.shares_state(&before));
}
#[test]
fn hostile_transform_params_cancellation_and_consumer_refusal_are_atomic() {
    let (mut session, owner, _) = fixture(ColorMode::Rgb, SampleType::U8, true);
    let before = session.active().unwrap().doc.clone();
    let params = json!({"layerId":owner.0.to_string(),"matrixF64Bits":bits([1.,0.,0.,0.,1.,0.,0.,0.,1.]),"interpolation":"none"});
    let mut cases = vec![Value::Null, json!({}), json!({"layerId":"01"}), json!({"layerId":owner.0.to_string(),"originPx":[0,0]}), params.clone()];
    cases[4]["matrixF64Bits"][0] = json!("7ff0000000000000");
    for value in ["lohalo", "UNKNOWN", ""] {
        let mut case = params.clone();
        case["interpolation"] = json!(value);
        cases.push(case);
    }
    for value in [json!([i64::MAX, 0]), json!([0.5, 0]), json!([0, 0, 0]), json!("0,0")] {
        let mut case = params.clone();
        case["originPx"] = value;
        cases.push(case);
    }
    let mut uppercase = params.clone();
    uppercase["matrixF64Bits"][0] = json!("3FF0000000000000");
    cases.push(uppercase);
    let mut shrink = params.clone();
    shrink["matrixF64Bits"] = bits([0.5, 0., 0., 0., 1., 0., 0., 0., 1.]);
    shrink["interpolation"] = json!("nohalo");
    cases.push(shrink);
    let mut huge = params.clone();
    huge["matrixF64Bits"] = bits([1e300, 0., 0., 0., 1., 0., 0., 0., 1.]);
    cases.push(huge);
    let mut singular = params.clone();
    singular["matrixF64Bits"] = bits([0.; 9]);
    cases.push(singular);
    for case in cases {
        assert!(session.execute("xcf.link.setTransform", case).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    assert!(photocraft_engine::xcf_link_geometry_cmds::set_transform_with_interrupt(&mut session, &params, &Interrupt::cancel_only(&|| true)).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert!(
        photocraft_engine::xcf_link_geometry_cmds::set_placement_with_interrupt(
            &mut session,
            &json!({"layerId":owner.0.to_string(),"originPx":[1,1]}),
            &Interrupt::cancel_only(&|| true)
        )
        .is_err()
    );
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.execute("xcf.link.setTransform", params.clone()).unwrap()["changed"], true);
    let current = session.active().unwrap().doc.clone();
    assert_eq!(session.execute("xcf.link.setTransform", params).unwrap()["changed"], false);
    assert!(session.active().unwrap().doc.shares_state(&current));
}
#[test]
fn staged_and_final_fence_cancellation_keep_geometry_pixels_and_history_unpublished() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (mut baseline, owner, _) = fixture(ColorMode::Rgb, SampleType::U8, true);
    let polls = AtomicUsize::new(0);
    let count = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    photocraft_engine::xcf_link_geometry_cmds::set_placement_with_interrupt(
        &mut baseline,
        &json!({"layerId":owner.0.to_string(),"originPx":[1,1]}),
        &Interrupt::cancel_only(&count),
    )
    .unwrap();
    let complete_polls = polls.load(Ordering::Relaxed);
    assert!(complete_polls > 3);
    for fence in [complete_polls / 2, complete_polls - 1] {
        let (mut session, owner, _) = fixture(ColorMode::Rgb, SampleType::U8, true);
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= fence;
        assert!(
            photocraft_engine::xcf_link_geometry_cmds::set_placement_with_interrupt(
                &mut session,
                &json!({"layerId":owner.0.to_string(),"originPx":[1,1]}),
                &Interrupt::cancel_only(&cancel),
            )
            .is_err()
        );
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(!session.active().unwrap().history.can_undo());
    }
}
#[test]
fn current_geometry_response_and_inspection_use_exact_bit_strings_without_authority() {
    let (mut session, owner, _) = fixture(ColorMode::Rgb, SampleType::U8, true);
    let original = session.active().unwrap().doc.xcf_state.as_ref().unwrap().source_items[0].clone();
    let inspected = session.execute("xcf.inspect", json!({})).unwrap();
    assert!(inspected["sources"]["entries"][0]["source"]["current"].is_null());
    let params = json!({"layerId":owner.0.to_string(),"matrixF64Bits":bits([1.,-0.,0.,0.,1.,0.,0.,0.,1.]),"interpolation":"none"});
    let response = session.execute("xcf.link.setTransform", params.clone()).unwrap();
    assert_eq!(response["current"]["transform"]["matrixF64Bits"], params["matrixF64Bits"]);
    assert_eq!(response["current"]["transform"]["matrixF64Bits"][1], "8000000000000000");
    assert_eq!(response["current"]["transform"]["preOffset"], json!([0, 0]));
    assert_eq!(response["current"]["transform"]["interpolation"], "none");
    let inspected = session.execute("xcf.inspect", json!({})).unwrap();
    assert_eq!(inspected["sources"]["entries"][0]["source"]["current"], response["current"]);
    assert_eq!(inspected["resources"]["entries"][0]["accessGranted"], false);
    let source = &session.active().unwrap().doc.xcf_state.as_ref().unwrap().source_items[0];
    assert_eq!(source.transform, original.transform);
    assert_eq!(source.cache, original.cache);
}
