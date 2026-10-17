//! One explicit Reduced replacement commits bytes and raster together.
#![cfg(feature = "corpus")]
use photocraft_doc::{
    Color, ColorMode, Document, SampleType, Size,
    item_identity::{ItemRef, ResourceId},
    xcf_items::EditableSource,
    xcf_resources::ResourceKind,
};
use photocraft_engine::{
    Session,
    xcf_resources::{RecoveryPolicy, ServiceResourceBytes, recover_authorized_bytes},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
use std::sync::Arc;
fn fixture() -> (Document, Vec<u8>, photocraft_doc::LayerId, ResourceId) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/transform-interpolation");
    let doc = photocraft_io::import("link-none.xcf", &std::fs::read(root.join("link-none.xcf")).unwrap()).unwrap().document;
    doc.validate_xcf_state().unwrap();
    let state = doc.xcf_state.as_ref().unwrap();
    let source = state.source_items.iter().find(|s| matches!(s.source, EditableSource::Linked { .. })).unwrap();
    let ItemRef::Layer(owner) = source.owner else { panic!("linked layer") };
    let EditableSource::Linked { resource, .. } = source.source else { panic!("linked image") };
    assert!(state.resources.iter().find(|r| r.id == resource).unwrap().content_hash.is_none());
    let bytes = std::fs::read(root.join("own-linked-resource.png")).unwrap();
    let mut image = photocraft_codecs::decode(&bytes).unwrap();
    assert_eq!(image.sample_type(), photocraft_codecs::SampleType::U8);
    assert_eq!(image.layout(), photocraft_codecs::ChannelLayout::Rgba);
    // Own synthetic replacement changes RGB throughout, preserving alpha/dimensions/ICC.
    for pixel in image.data_mut().as_chunks_mut::<4>().0 {
        for value in &mut pixel[..3] {
            *value = value.wrapping_add(41);
        }
    }
    let replacement = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    assert_ne!(blake3::hash(&replacement), blake3::hash(&bytes));
    (doc, replacement, owner, resource)
}
fn request(s: &Session, owner: photocraft_doc::LayerId, resource: ResourceId, bytes: &[u8]) -> Value {
    let st = s.active().unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    json!({"documentId":st.doc.id.0.to_string(),"expectedRevision":st.revision.to_string(),"layerId":owner.0.to_string(),"resourceId":resource.0.to_string(),"policy":"substitute","payloadHex":hex})
}
fn qualified_reference(doc: &Document, bytes: &[u8], owner: photocraft_doc::LayerId, resource: ResourceId) -> Document {
    let mut s = Session::new();
    s.open_document(doc.clone(), None).unwrap();
    recover_authorized_bytes(
        &mut s,
        resource,
        ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, bytes.to_vec(), true).unwrap(),
        RecoveryPolicy::AllowSubstitution,
        &Interrupt::NONE,
    )
    .unwrap();
    s.execute("xcf.link.refresh", json!({"layerId":owner.0.to_string()})).unwrap();
    photocraft_compose::gimp_render::PreparedRenderer::new(&s.active().unwrap().doc).unwrap();
    (*s.active().unwrap().doc).clone()
}
#[test]
fn existing_two_stage_lane_qualifies_the_same_explicit_png() {
    let (doc, bytes, owner, resource) = fixture();
    let current = qualified_reference(&doc, &bytes, owner, resource);
    assert!(current.layer(owner).unwrap().surface() != doc.layer(owner).unwrap().surface(), "replacement must change current raster");
    assert_eq!(
        current.xcf_state.as_ref().unwrap().resources.iter().find(|r| r.id == resource).unwrap().resolution.as_ref().unwrap().fidelity,
        photocraft_doc::xcf_resources::ResourceFidelity::Substituted
    );
    let archive = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&archive).unwrap(), current);
}
#[test]
fn registered_substitute_has_one_undo_for_both_resource_and_raster() {
    let (doc, bytes, owner, resource) = fixture();
    let expected = qualified_reference(&doc, &bytes, owner, resource);
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let history_count = s.active().unwrap().history.entry_labels().count();
    let native_before = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    let params = request(&s, owner, resource, &bytes);
    let result = s.execute("xcf.link.substitute", params);
    assert!(result.is_ok(), "registered replacement must exist and succeed: {result:?}");
    let result = result.unwrap();
    assert_eq!(result["assessment"], "Reduced");
    assert_eq!(result["byteIdentity"], "Substituted");
    assert_eq!(result["applied"], true);
    let after = s.active().unwrap().doc.clone();
    assert_eq!(*after, expected);
    assert_eq!(s.active().unwrap().revision, revision + 1);
    assert_eq!(s.active().unwrap().history.entry_labels().count(), history_count + 1);
    assert_eq!(after.icc_profile, before.icc_profile);
    assert_eq!(after.working_profile_interpretation, before.working_profile_interpretation);
    let original = &before.xcf_state.as_ref().unwrap().source_items;
    let current = &after.xcf_state.as_ref().unwrap().source_items;
    for (a, b) in original.iter().zip(current) {
        assert_eq!(a.cache, b.cache);
        assert_eq!(a.transform, b.transform);
    }
    let native_after = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native_after).unwrap(), *after);
    assert!(s.undo());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), native_before);
    assert!(!s.active().unwrap().history.can_undo());
    assert!(s.redo());
    assert!(s.active().unwrap().doc.shares_state(&after));
    assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), native_after);
}
fn floating_session() -> (Session, Document) {
    let doc = Document::with_background("Floating refusal", Size::new(3, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    s.edit("Real redo seed", |d, _| {
        d.name = "Restored redo".into();
        Ok(())
    })
    .unwrap();
    let redo = (*s.active().unwrap().doc).clone();
    assert!(s.undo());
    s.execute("select.all", json!({})).unwrap();
    // Selection is a history edit, so seed a real redo after selecting, before floating.
    s.edit("Real selected redo", |d, _| {
        d.name = redo.name.clone();
        Ok(())
    })
    .unwrap();
    let redo = (*s.active().unwrap().doc).clone();
    assert!(s.undo());
    s.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
    assert!(s.active().unwrap().floating.is_some());
    assert!(s.active().unwrap().history.can_redo());
    (s, redo)
}
fn floating_refusal(mut params: Value, coalesce: bool, stale: bool) {
    let (mut s, redo) = floating_session();
    if coalesce {
        params["coalesce"] = Value::Null;
    }
    if stale {
        s.active_mut().unwrap().floating.as_mut().unwrap().revision = 0;
    }
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let f = st.floating.clone().unwrap();
    let revisions = (st.revision, st.saved_revision);
    let past: Vec<_> = st.history.entry_labels().map(str::to_string).collect();
    let future: Vec<_> = st.history.redo_labels().map(str::to_string).collect();
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    let result = s.execute("xcf.link.substitute", params);
    assert!(result.is_err());
    let st = s.active().unwrap();
    assert!(st.doc.shares_state(&before));
    assert_eq!((st.revision, st.saved_revision), revisions);
    assert_eq!(st.history.entry_labels().collect::<Vec<_>>(), past);
    assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), future);
    let now = st.floating.as_ref().unwrap();
    assert_eq!((now.layer, now.offset, now.revision), (f.layer, f.offset, f.revision));
    assert!(Arc::ptr_eq(&now.parts, &f.parts));
    assert_eq!(photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap(), native);
    // Unknown command currently refuses before dispatch. Do not mistake this for new-route proof.
    let error = result.unwrap_err().to_string();
    let code = if coalesce { "xcf.link.substitute.coalescing" } else { "xcf.link.substitute.floating" };
    assert!(error.contains(code), "registered candidate must give named refusal: {error}");
    assert!(s.redo());
    assert_eq!(*s.active().unwrap().doc, redo);
}
#[test]
fn registered_floating_refusal_preserves_piece_and_real_redo() {
    floating_refusal(json!({"payloadHex":"invalid","expectedRevision":"0"}), false, false);
}
#[test]
fn registered_raw_coalesce_refusal_preserves_piece_and_real_redo() {
    floating_refusal(json!({}), true, false);
}

#[test]
fn registered_stale_floating_refusal_preserves_unmodified_some() {
    floating_refusal(json!({"payloadHex":"invalid","expectedRevision":"0"}), false, true);
}

fn seeded(doc: Document) -> Session {
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    s.edit("Actual retained redo", |d, _| {
        d.name = "Retained redo destination".into();
        Ok(())
    })
    .unwrap();
    assert!(s.undo());
    assert!(s.active().unwrap().history.can_redo());
    let rect = photocraft_doc::Rect::new(0, 0, 1, 1);
    let surface = photocraft_raster::Surface::try_from_interleaved(photocraft_doc::PixelFormat::RGBA8, rect, &[7, 21, 231, 255]).unwrap();
    s.clipboard = Some(photocraft_engine::edit_cmds::Clip::from_external_srgb(surface, rect).unwrap());
    s
}
fn authorized(bytes: Vec<u8>) -> ServiceResourceBytes {
    ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, bytes, true).unwrap()
}
fn atomic_refusal(mut s: Session, code: &str, call: impl FnOnce(&mut Session) -> photocraft_engine::Result<()>) {
    let expected_redo = {
        // Peek a cloned history rather than changing the request's revision via redo/undo.
        let st = s.active().unwrap();
        let mut h = st.history.clone();
        let ctl = photocraft_raster::backing::ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        h.redo(&st.doc, &ctl).unwrap().unwrap().snapshot().clone()
    };
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let meta = (st.revision, st.saved_revision, st.active_layer, st.layer_anchor, st.path.clone(), st.source.clone());
    let selected = st.selected_layers.clone();
    let undo: Vec<_> = st.history.entry_labels().map(str::to_string).collect();
    let redo: Vec<_> = st.history.redo_labels().map(str::to_string).collect();
    let coalesce = st.coalesce.clone();
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    let actions = s.actions.clone();
    let journal = s.journal.clone();
    let clip_before=s.clipboard.as_ref().map(|c|json!({"bounds":c.bounds,"format":c.surface.format(),"pixels":c.surface.try_read_interleaved(c.bounds).unwrap(),"icc":c.color.icc_profile.as_deref().map(|p|p.as_slice()),"mode":c.color.mode,"encoding":c.color.sample_encoding,"interpretation":c.color.interpretation,"composition":c.composition}));
    let old_tile = s.clipboard.as_ref().and_then(|c| c.surface.tiles().next().map(|(_, t)| t.clone()));
    let result = call(&mut s);
    let error = match result {
        Err(e) => e,
        Ok(()) => panic!("expected atomic refusal {code}"),
    };
    assert!(error.to_string().contains(code), "wanted {code}, got {error}");
    let st = s.active().unwrap();
    assert!(st.doc.shares_state(&before));
    assert_eq!(*st.doc, *before);
    assert_eq!((st.revision, st.saved_revision, st.active_layer, st.layer_anchor, st.path.clone(), st.source.clone()), meta);
    assert_eq!(st.selected_layers, selected);
    assert_eq!(st.coalesce, coalesce);
    assert_eq!(st.history.entry_labels().collect::<Vec<_>>(), undo);
    assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo);
    assert_eq!(s.actions, actions);
    assert_eq!(s.journal, journal);
    let clip_after=s.clipboard.as_ref().map(|c|json!({"bounds":c.bounds,"format":c.surface.format(),"pixels":c.surface.try_read_interleaved(c.bounds).unwrap(),"icc":c.color.icc_profile.as_deref().map(|p|p.as_slice()),"mode":c.color.mode,"encoding":c.color.sample_encoding,"interpretation":c.color.interpretation,"composition":c.composition}));
    assert_eq!(clip_after, clip_before);
    if let Some(tile) = old_tile {
        assert!(Arc::ptr_eq(&tile, s.clipboard.as_ref().unwrap().surface.tiles().next().unwrap().1));
    }
    assert!(st.floating.is_none());
    assert_eq!(photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap(), native);
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc, expected_redo);
}
#[test]
fn registered_parser_refusals_keep_native_state_and_real_redo() {
    let (doc, bytes, owner, resource) = fixture();
    for (field, value) in [
        ("documentId", json!(0)),
        ("documentId", json!("01")),
        ("documentId", json!("0")),
        ("documentId", json!("18446744073709551616")),
        ("layerId", json!("+1")),
        ("layerId", json!("0")),
        ("resourceId", json!(" 1")),
        ("resourceId", json!(1)),
        ("expectedRevision", json!("01")),
        ("expectedRevision", json!("-1")),
        ("expectedRevision", json!(1)),
        ("policy", json!("exact")),
        ("payloadHex", json!("")),
        ("payloadHex", json!("0")),
        ("payloadHex", json!("xx")),
        ("payloadHex", json!("ÃƒÆ’Ã‚Â©")),
        ("payloadHex", Value::Null),
    ] {
        let s = seeded(doc.clone());
        let mut p = request(&s, owner, resource, &bytes);
        p[field] = value;
        atomic_refusal(s, "xcf.link.substitute.parameters", |s| s.execute("xcf.link.substitute", p).map(|_| ()));
    }
    let s = seeded(doc);
    let mut p = request(&s, owner, resource, &bytes);
    p["locator"] = json!("file:///never-read");
    atomic_refusal(s, "xcf.link.substitute.parameters", |s| s.execute("xcf.link.substitute", p).map(|_| ()));
}
#[test]
fn registered_coalescing_all_raw_types_refuse_before_history() {
    let (doc, bytes, owner, resource) = fixture();
    for value in [Value::Null, json!("same"), json!(0), json!(false), json!({}), json!([])] {
        let s = seeded(doc.clone());
        let mut p = request(&s, owner, resource, &bytes);
        p["coalesce"] = value;
        atomic_refusal(s, "xcf.link.substitute.coalescing", |s| s.execute("xcf.link.substitute", p).map(|_| ()));
    }
}
#[test]
fn typed_target_edit_then_undo_stays_stale_even_when_snapshot_returns() {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, substitute_with_interrupt};
    let (doc, bytes, owner, resource) = fixture();
    let mut s = seeded(doc);
    let target = capture_target(&s, owner, resource).unwrap();
    let old = s.active().unwrap().doc.clone();
    s.edit("Later picker edit", |d, _| {
        d.layer_mut(owner).unwrap().name = "Later picker edit".into();
        Ok(())
    })
    .unwrap();
    assert!(s.undo());
    assert!(s.active().unwrap().doc.shares_state(&old));
    // atomic_refusal probes redo/undo too: revision is the authority, not restored pointer identity.
    atomic_refusal(s, "xcf.link.substitute.stale-target", |s| substitute_with_interrupt(s, target, authorized(bytes), &Interrupt::NONE).map(|_| ()));
}
#[test]
fn typed_target_switched_closed_and_same_id_reopened_refuse_without_refocus() {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, substitute_with_interrupt};
    for reopen in [false, true] {
        let (doc, bytes, owner, resource) = fixture();
        let mut s = seeded(doc.clone());
        let target = capture_target(&s, owner, resource).unwrap();
        if reopen {
            s.close(0).unwrap();
            s.open_document(doc, None).unwrap();
        } else {
            s.open_document(Document::with_background("Other tab", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE), None).unwrap();
        }
        let before: Vec<_> = s.documents().iter().map(|st| (st.doc.clone(), st.revision, st.saved_revision)).collect();
        let result = substitute_with_interrupt(&mut s, target, authorized(bytes), &Interrupt::NONE);
        assert!(matches!(&result,Err(e) if e.to_string().contains("xcf.link.substitute.stale-target")), "{result:?}");
        assert_eq!(s.documents().len(), before.len());
        for (st, (doc, rev, saved)) in s.documents().iter().zip(before) {
            assert!(st.doc.shares_state(&doc));
            assert_eq!((st.revision, st.saved_revision), (rev, saved));
        }
    }
}
#[test]
fn typed_success_returns_prepared_schema_and_preserves_original_model() {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, substitute_with_interrupt};
    let (doc, bytes, owner, resource) = fixture();
    let expected = qualified_reference(&doc, &bytes, owner, resource);
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    let target = capture_target(&s, owner, resource).unwrap();
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let r = substitute_with_interrupt(&mut s, target, authorized(bytes), &Interrupt::NONE).unwrap();
    assert_eq!(r.document, expected.id);
    assert_eq!(r.owner, owner);
    assert_eq!(r.resource, resource);
    assert_eq!(r.payload_hash, hash);
    assert_eq!(r.fidelity, photocraft_doc::xcf_resources::ResourceFidelity::Substituted);
    assert!(r.applied);
    assert_eq!(
        r.into_response(),
        json!({"documentId":expected.id.0.to_string(),"layerId":owner.0.to_string(),"resourceId":resource.0.to_string(),"payloadHash":hash,"byteIdentity":"Substituted","assessment":"Reduced","applied":true})
    );
    assert_eq!(*s.active().unwrap().doc, expected);
}
#[test]
fn typed_cancellation_all_observed_poll_boundaries_keeps_real_redo() {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, substitute_with_interrupt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (doc, bytes, owner, resource) = fixture();
    let mut measured = seeded(doc.clone());
    let target = capture_target(&measured, owner, resource).unwrap();
    let calls = AtomicUsize::new(0);
    let never = || {
        calls.fetch_add(1, Ordering::Relaxed);
        false
    };
    substitute_with_interrupt(&mut measured, target, authorized(bytes.clone()), &Interrupt::cancel_only(&never)).unwrap();
    let n = calls.load(Ordering::Relaxed);
    assert!(n > 10 && n < 4096, "bounded measured cancellation polls {n}");
    eprintln!("substitution complete route polls={n}; exhaustive cancellation thresholds=1..={n}");
    for threshold in 1..=n {
        let s = seeded(doc.clone());
        atomic_refusal(s, "cancelled", |s| {
            let target = capture_target(s, owner, resource)?;
            let count = AtomicUsize::new(0);
            let cancel = || count.fetch_add(1, Ordering::Relaxed) + 1 >= threshold;
            let result = substitute_with_interrupt(s, target, authorized(bytes.clone()), &Interrupt::cancel_only(&cancel));
            assert!(matches!(&result, Err(photocraft_engine::EngineError::Cancelled)), "threshold{threshold}: {result:?}");
            result.map(|_| ())
        });
    }
}
#[test]
fn locked_original_hash_and_nested_resource_associations_are_atomic_refusals() {
    use photocraft_doc::xcf_filters::{FilterArgument, FilterValue};
    let (doc, bytes, owner, resource) = fixture();
    for case in 0..5 {
        let mut changed = doc.clone();
        match case {
            0 => changed.layer_mut(owner).unwrap().locks.pixels = true,
            1 => changed.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().locks.content = true,
            2 => {
                changed.xcf_state.as_mut().unwrap().resources.iter_mut().find(|r| r.id == resource).unwrap().content_hash =
                    Some(blake3::hash(&bytes).to_hex().to_string())
            }
            _ => {
                let source = changed.xcf_state.as_mut().unwrap().source_items.iter_mut().find(|s| s.owner == ItemRef::Layer(owner)).unwrap();
                let EditableSource::Linked { configuration, .. } = &mut source.source else { panic!("linked") };
                let value = if case == 3 {
                    FilterValue::Resource(resource)
                } else {
                    FilterValue::Color {
                        components: vec![photocraft_doc::xcf_filters::ExactNumber::Float64Bits(0.25f64.to_bits()); 3],
                        encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
                        profile: Some(resource),
                    }
                };
                configuration
                    .push(FilterArgument { name: "own-extra".into(), value: FilterValue::Config(vec![FilterArgument { name: "own-ref".into(), value }]) });
            }
        }
        changed.validate_xcf_state().unwrap();
        let s = seeded(changed);
        let p = request(&s, owner, resource, &bytes);
        let code = match case {
            0 | 1 => "locked",
            2 => "original-identity",
            _ => "shared-resource",
        };
        atomic_refusal(s, &format!("xcf.link.substitute.{code}"), |s| s.execute("xcf.link.substitute", p).map(|_| ()));
    }
}
#[test]
fn malformed_png_wrong_authorized_kind_and_limit_are_atomic() {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, substitute_with_interrupt};
    let (doc, bytes, owner, resource) = fixture();
    for (kind, data, code) in [
        (ResourceKind::LinkedImage, b"not a PNG".to_vec(), "unsupported"),
        (ResourceKind::Pattern, bytes.clone(), "unsupported"),
        (ResourceKind::LinkedImage, Vec::new(), "limit"),
        (ResourceKind::LinkedImage, vec![0; (8 << 20) + 1], "limit"),
    ] {
        let s = seeded(doc.clone());
        atomic_refusal(s, &format!("xcf.link.substitute.{code}"), |s| {
            let target = capture_target(s, owner, resource)?;
            let input = ServiceResourceBytes::from_authorized_service(kind, data, true).unwrap();
            substitute_with_interrupt(s, target, input, &Interrupt::NONE).map(|_| ())
        });
    }
    assert!(matches!(
        ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, bytes.clone(), false),
        Err(photocraft_engine::xcf_resources::RecoveryError::Denied)
    ));
    assert!(matches!(
        ServiceResourceBytes::from_authorized_service(ResourceKind::Font, bytes, true),
        Err(photocraft_engine::xcf_resources::RecoveryError::UnsupportedFont)
    ));
}
#[test]
fn journal_and_action_recording_exclude_payload_on_success_and_error() {
    let (doc, bytes, owner, resource) = fixture();
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    s.execute("actions.record", json!({"name":"Own no payload"})).unwrap();
    let before = s.journal.clone();
    let mut malformed = request(&s, owner, resource, &bytes);
    malformed["payloadHex"] = json!("xx");
    assert!(s.execute("xcf.link.substitute", malformed).is_err());
    assert_eq!(s.journal, before);
    let p = request(&s, owner, resource, &bytes);
    s.execute("xcf.link.substitute", p).unwrap();
    assert_eq!(s.journal, before);
    assert_eq!(photocraft_engine::actions_cmds::pending_steps(&s, 0).count(), 0);
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list.len(), 1);
    assert!(s.actions.list[0].steps.is_empty());
    assert!(!photocraft_engine::actions_cmds::replayable("xcf.link.substitute"));
}
#[test]
fn background_and_layer_origin_slices_refuse_without_mutation() {
    let (doc, bytes, owner, resource) = fixture();
    let s = seeded(doc.clone());
    let p = request(&s, owner, resource, &bytes);
    atomic_refusal(s, "xcf.link.substitute.context", |s| s.start("xcf.link.substitute", p).map(|_| ()));
    let mut changed = doc;
    changed.slices.list.push(photocraft_doc::slices::Slice {
        id: 1,
        origin: photocraft_doc::slices::SliceOrigin::Layer,
        layer: Some(owner),
        rect: changed.bounds(),
        ..Default::default()
    });
    let s = seeded(changed);
    let p = request(&s, owner, resource, &bytes);
    atomic_refusal(s, "xcf.link.substitute.context", |s| s.execute("xcf.link.substitute", p).map(|_| ()));
}
#[test]
fn maximum_revision_refuses_before_unchecked_transaction_increment() {
    let (doc, bytes, owner, resource) = fixture();
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    s.active_mut().unwrap().revision = u64::MAX;
    let before = s.active().unwrap().doc.clone();
    let p = request(&s, owner, resource, &bytes);
    let result = s.execute("xcf.link.substitute", p);
    assert!(matches!(&result,Err(e) if e.to_string().contains("xcf.link.substitute.context")), "{result:?}");
    assert_eq!(s.active().unwrap().revision, u64::MAX);
    assert!(s.active().unwrap().doc.shares_state(&before));
}

#[test]
fn picker_request_bridge_validates_opaque_target_and_bounds_without_edit() {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, request_for_target};
    let (doc, bytes, owner, resource) = fixture();
    let mut s = seeded(doc);
    let target = capture_target(&s, owner, resource).unwrap();
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let p = request_for_target(&s, &target, &bytes).unwrap();
    assert_eq!(p, request(&s, owner, resource, &bytes));
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.active().unwrap().revision, revision);
    for rejected in [Vec::new(), vec![0; (8 << 20) + 1]] {
        let r = request_for_target(&s, &target, &rejected);
        assert!(matches!(&r,Err(e) if e.to_string().contains("xcf.link.substitute.limit")), "{r:?}");
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let maximum = vec![0; 8 << 20];
    let pmax = request_for_target(&s, &target, &maximum).unwrap();
    assert_eq!(pmax["payloadHex"].as_str().unwrap().len(), 16 << 20);
    drop(pmax);
    s.edit("Picker stale", |d, _| {
        d.layer_mut(owner).unwrap().name = "Stale picker".into();
        Ok(())
    })
    .unwrap();
    assert!(s.undo());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let r = request_for_target(&s, &target, &bytes);
    assert!(matches!(&r,Err(e) if e.to_string().contains("xcf.link.substitute.stale-target")), "{r:?}");
}
#[test]
fn registered_encoded_cap_refusal_and_typed_item_census_cap_preserve_state() {
    use photocraft_engine::xcf_link_substitution_cmds::capture_target;
    let (doc, bytes, owner, resource) = fixture();
    let s = seeded(doc.clone());
    let mut p = request(&s, owner, resource, &bytes);
    p["payloadHex"] = json!("00".repeat((8 << 20) + 1));
    atomic_refusal(s, "xcf.link.substitute.limit", |s| s.execute("xcf.link.substitute", p).map(|_| ()));
    let mut s = seeded(doc);
    let d = s.active_mut().unwrap().doc.try_resident_mut().unwrap();
    let state = d.xcf_state.as_mut().unwrap();
    let item = state.items[0].clone();
    state.items.resize(10_001, item);
    // Deliberately hostile duplicates: no valid-native continuation claim for this malformed DTO.
    let before = s.active().unwrap().doc.clone();
    let revisions = (s.active().unwrap().revision, s.active().unwrap().saved_revision);
    let target = capture_target(&s, owner, resource);
    assert!(matches!(&target,Err(e) if e.to_string().contains("xcf.link.substitute.limit")), "{target:?}");
    let p = request(&s, owner, resource, &bytes);
    let r = s.execute("xcf.link.substitute", p);
    assert!(matches!(&r,Err(e) if e.to_string().contains("xcf.link.substitute.limit")), "{r:?}");
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!((s.active().unwrap().revision, s.active().unwrap().saved_revision), revisions);
}
#[test]
fn native_zip64_continues_substituted_resource_and_current_raster() {
    let (doc, bytes, owner, resource) = fixture();
    let mut s = Session::new();
    s.open_document(doc, None).unwrap();
    let before = s.active().unwrap().doc.clone();
    let p = request(&s, owner, resource, &bytes);
    s.execute("xcf.link.substitute", p).unwrap();
    let after = s.active().unwrap().doc.clone();
    for doc in [&before, &after] {
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &Interrupt::NONE)
            .unwrap();
        let bytes = stream.into_inner();
        assert_eq!(photocraft_format::load_from_stream(std::io::Cursor::new(bytes), &Default::default(), Default::default(), &Interrupt::NONE).unwrap(), **doc);
    }
    assert!(s.undo());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.redo());
    assert!(s.active().unwrap().doc.shares_state(&after));
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn running_job_refuses_registered_and_typed_routes_without_consuming_job() {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, substitute_with_interrupt};
    let (doc, bytes, owner, resource) = fixture();
    let mut s = seeded(doc);
    let target = capture_target(&s, owner, resource).unwrap();
    let p = request(&s, owner, resource, &bytes);
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let journal = s.journal.clone();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let started = s
        .start_job(
            "own-substitution-blocker",
            json!({}),
            "Own bounded blocker",
            false,
            move |_| {
                rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))?;
                Ok(())
            },
            |_, ()| Ok(json!({"done":true})),
        )
        .unwrap();
    let photocraft_engine::jobs::Started::Job(job) = started else { panic!("native job") };
    assert!(s.has_jobs());
    let r = s.execute("xcf.link.substitute", p);
    assert!(matches!(&r,Err(e) if e.to_string().contains("xcf.link.substitute.context")), "{r:?}");
    let r = substitute_with_interrupt(&mut s, target, authorized(bytes), &Interrupt::NONE);
    assert!(matches!(&r,Err(e) if e.to_string().contains("xcf.link.substitute.context")), "{r:?}");
    assert!(s.has_jobs());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.journal, journal);
    tx.send(()).unwrap();
    s.wait_job(job).unwrap();
    assert!(!s.has_jobs());
    assert!(s.active().unwrap().history.can_redo());
}

#[test]
fn direct_second_link_and_graph_resource_are_refused_before_replacement() {
    use photocraft_doc::item_identity::GraphNodeId;
    use photocraft_doc::xcf_filters::{FilterArgument, FilterGraph, FilterValue, GraphNode, GraphVersionOrigin, OperationIdentity};
    let (doc, bytes, owner, resource) = fixture();
    for graph in [false, true] {
        let mut changed = doc.clone();
        if graph {
            let state = changed.xcf_state.as_mut().unwrap();
            let source = state.source_items.iter_mut().find(|s| s.owner == ItemRef::Layer(owner)).unwrap();
            let EditableSource::Linked { configuration, .. } = &mut source.source else { panic!("linked") };
            configuration.push(FilterArgument {
                name: "own-graph".into(),
                value: FilterValue::Graph(FilterGraph {
                    input: None,
                    nodes: vec![GraphNode {
                        id: GraphNodeId(50),
                        operation: OperationIdentity { name: "own:resource-consumer".into(), stored_version: "1".into(), schema_epoch: "own".into() },
                        version_origin: GraphVersionOrigin::Stored,
                        arguments: vec![FilterArgument { name: "resource".into(), value: FilterValue::Resource(resource) }],
                    }],
                    edges: vec![],
                    output: GraphNodeId(50),
                    original_pipeline: None,
                }),
            });
        } else {
            let old = changed.layer(owner).unwrap();
            let extra = photocraft_doc::Layer::new("Other linked consumer", old.content.clone());
            let extra_id = extra.id;
            changed.layers.push(extra);
            let mut state = changed.xcf_state.take().unwrap();
            let mut source = state.source_items.iter().find(|s| s.owner == ItemRef::Layer(owner)).unwrap().clone();
            let bounds = state.items.iter().find(|i| i.item == source.owner).unwrap().raster_bounds;
            state.reconcile_layers(&changed).unwrap();
            state.items.iter_mut().find(|i| i.item == ItemRef::Layer(extra_id)).unwrap().raster_bounds = bounds;
            source.owner = ItemRef::Layer(extra_id);
            state.source_items.push(source);
            changed.xcf_state = Some(state);
        }
        changed.validate_xcf_state().unwrap();
        let s = seeded(changed);
        let p = request(&s, owner, resource, &bytes);
        atomic_refusal(s, "xcf.link.substitute.shared-resource", |s| s.execute("xcf.link.substitute", p).map(|_| ()));
    }
}
fn synthetic_link(mode: ColorMode, depth: SampleType) -> (Document, Vec<u8>, photocraft_doc::LayerId, ResourceId) {
    use photocraft_doc::{
        XcfState,
        xcf_items::{SourceCache, SourceItem},
        xcf_resources::ResourceReference,
    };
    let mut d = Document::with_background("Own synthetic linked samples", Size::new(2, 1), mode, depth, Color::WHITE);
    d.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
    d.icc_profile = Some(if mode == ColorMode::Rgb {
        photocraft_cms::builtin::srgb().to_bytes()
    } else {
        photocraft_cms::builtin::Builtin::GrayGamma22.profile().to_bytes()
    });
    let owner = d.layers[0].id;
    let bounds = d.bounds();
    let format = d.pixel_format();
    let raw: Vec<u8> = if depth == SampleType::U8 {
        (0..format.bytes_per_pixel() * 2).map(|i| (i * 23 + 7) as u8).collect()
    } else {
        (0..format.channels() * 2).flat_map(|i| ((i * 4243 + 29) as u16).to_ne_bytes()).collect()
    };
    let image = photocraft_codecs::Image::from_raw(
        2,
        1,
        if mode == ColorMode::Rgb { photocraft_codecs::ChannelLayout::Rgba } else { photocraft_codecs::ChannelLayout::GrayA },
        if depth == SampleType::U8 { photocraft_codecs::SampleType::U8 } else { photocraft_codecs::SampleType::U16 },
        raw,
    )
    .unwrap()
    .with_icc(d.icc_profile.as_deref().cloned());
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let original = d.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    let mut state = XcfState::default();
    state.reconcile_layers(&d).unwrap();
    state.items[0].raster_bounds = Some(bounds);
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own substitute".into(),
        collection: None,
        internal: false,
        locator: Some("file:///inert-never-read".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(owner),
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: Some(SourceCache { blake3: blake3::hash(&original).to_hex().to_string(), bytes: original.into(), width: 2, height: 1, format }),
    });
    d.xcf_state = Some(state);
    d.validate_xcf_state().unwrap();
    (d, png, owner, ResourceId(1))
}
#[test]
fn registered_rgb_gray_u8_u16_controls_preserve_model_and_uppercase_hex() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16] {
            let (doc, bytes, owner, resource) = synthetic_link(mode, depth);
            let expected = qualified_reference(&doc, &bytes, owner, resource);
            let mut s = Session::new();
            s.open_document(doc, None).unwrap();
            let original = s.active().unwrap().doc.clone();
            let mut p = request(&s, owner, resource, &bytes);
            p["payloadHex"] = json!(p["payloadHex"].as_str().unwrap().to_ascii_uppercase());
            s.execute("xcf.link.substitute", p).unwrap();
            assert_eq!(*s.active().unwrap().doc, expected);
            assert!(s.undo());
            assert!(s.active().unwrap().doc.shares_state(&original));
            assert!(s.redo());
            assert_eq!(*s.active().unwrap().doc, expected);
            let native = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), expected);
        }
    }
}
#[test]
fn current_unqualified_encoding_refuses_without_changing_original_provenance() {
    let (mut doc, bytes, owner, resource) = fixture();
    doc.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear);
    doc.validate_xcf_state().unwrap();
    let s = seeded(doc);
    let p = request(&s, owner, resource, &bytes);
    atomic_refusal(s, "xcf.link.substitute.unsupported", |s| s.execute("xcf.link.substitute", p).map(|_| ()));
}
