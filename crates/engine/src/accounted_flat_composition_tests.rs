//! UNRUN paid CURRENT composition adapter on genuine existing source packets.
//! Native captures and prepaid CURRENT lock controls are external test harness
//! data; explicit CURRENT edits are not new producer/source observations.
use super::*;
use photocraft_doc::{
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_raster::backing::quota::QuotaKind;
fn session(i: usize) -> (Session, BackingQuota) {
    let q = quota();
    let imported = source(i, &q);
    let mut s = Session::new();
    s.add_owned_separable_blend_xcf_import(&imported, "flat", None, &ctl()).unwrap();
    drop(imported);
    (s, q)
}
fn wire(s: &Session, mode: i32, blend: &str, composite: &str, operator: &str) -> serde_json::Value {
    let state = s.active().unwrap();
    serde_json::json!({"layerId":state.doc.layers[1].id.0.to_string(),"sourceMode":mode,"legacy":false,"blendSpace":blend,"compositeSpace":composite,"operator":operator,"documentId":state.doc.id.0.to_string(),"expectedRevision":state.revision.to_string()})
}
fn space(s: GimpSpace) -> &'static str {
    match s {
        GimpSpace::LinearRgb => "linear-rgb",
        GimpSpace::ProfileRgb => "profile-rgb",
        GimpSpace::PerceptualRgb => "perceptual-rgb",
        GimpSpace::Lab => panic!("no Lab source grant"),
    }
}
fn current_control(lock: usize) -> (Session, BackingQuota) {
    let (s, q) = session(0);
    let original = s.active().unwrap().doc.accounted().unwrap().clone();
    let measured = photocraft_io::retained_native_model_census(original.document(), Default::default(), &Interrupt::NONE).unwrap();
    let index = q.reserve(QuotaKind::Index, (measured.retained_bytes() as u64) * 2 + 8192).unwrap();
    let resident = q.reserve(QuotaKind::Resident, (original.document().icc_profile.as_ref().map_or(0, Vec::capacity) as u64) * 2 + 8192).unwrap();
    let mut doc = original.document().clone();
    let id = doc.layers[1].id;
    match lock {
        1 => doc.layers[1].locks.all = true,
        2 => doc.layers[1].locks.pixels = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().locks.content = true,
        4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().locks.visibility = true,
        _ => {}
    }
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(actual.retained_bytes() as u64 <= index.bytes());
    assert!(doc.icc_profile.as_ref().map_or(0, Vec::capacity) as u64 <= resident.bytes());
    let owner = AccountedDocument::from_owned_readonly_flat_composite(doc, std::sync::Arc::new((original, index, resident)), q.clone(), &ctl()).unwrap();
    // Materialize the same interpretation as explicit CURRENT metadata before
    // publishing this paid lock control; the engine test below is a raw noop.
    let owner = owner
        .try_with_flat_composite_composition_patch(
            photocraft_doc::accounted_document::FlatCompositeCompositionPatch {
                layer: id,
                source_mode: 31,
                blend_space: GimpSpace::PerceptualRgb,
                composite_space: GimpSpace::LinearRgb,
                operator: GimpOperator::ClipToBackdrop,
            },
            &ctl(),
        )
        .unwrap();
    crate::accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(&owner, &ctl()).unwrap();
    let mut current = Session::new();
    let states = current.prefs().performance.history_states.max(1) as usize;
    let state = crate::DocState::new_accounted(owner, None, states, &ctl()).unwrap();
    current.publish_document_state(state, &ctl()).unwrap();
    drop(s);
    (current, q)
}
#[test]
#[ignore = "UNRUN: genuine46 supported source packets Auto->explicit same interpretation is one changed transaction, exact physical/canonical policy/native/profile/source and native continuation"]
fn actual_modes_and_raw_auto_to_explicit() {
    for i in 0..CASES.len() {
        let (mut s, q) = session(i);
        let state = s.active().unwrap();
        let before = state.doc.clone();
        let raw = native(&s);
        let source = state.source.clone();
        let id = state.doc.layers[1].id;
        let item = before.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap();
        let original = item.composition.as_ref().unwrap();
        let mode = original.source_mode;
        if matches!(mode, 46 | 47) {
            continue;
        }
        let r = original.resolved.as_ref().unwrap();
        let p = wire(&s, mode, space(r.blend_space), space(r.composite_space), "clip-to-backdrop");
        let rev = state.revision;
        let value = s.execute("xcf.composition.set", p).unwrap();
        assert_eq!(value["updated"], true);
        assert_eq!(value["layerId"], id.0.to_string());
        assert_eq!(value["sourceMode"], mode);
        assert_eq!(s.active().unwrap().revision, rev + 1);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Set GIMP Composition Policy"));
        let changed = s.active().unwrap().doc.clone();
        let policy = changed.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap().composition.as_ref().unwrap();
        assert_eq!(policy.compatibility_epoch, original.compatibility_epoch);
        assert_eq!(policy.resolved, original.resolved);
        assert_eq!(policy.stored_operator, 2);
        assert!(!policy.legacy);
        assert!(!policy.pass_through);
        assert_eq!(native(&s), raw);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(changed.icc_profile, before.icc_profile);
        assert_eq!(changed.source_color_context, before.source_color_context);
        let h = s.active().unwrap().history.clone();
        let p = wire(&s, mode, space(r.blend_space), space(r.composite_space), "clip-to-backdrop");
        assert_eq!(s.execute("xcf.composition.set", p).unwrap()["updated"], true);
        assert!(s.active().unwrap().doc.shares_state(&changed));
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
        assert_eq!(s.active().unwrap().revision, rev + 1);
        let bytes = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "Composition.pcraft", &ctl())
            .unwrap()
            .write_and_publish(&mut s, &ctl(), |doc, _| {
                photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|w| w.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut next = Session::new();
        next.add_owned_native_separable_blend_archive(archive, "native", None, &ctl()).unwrap();
        assert_eq!(next.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
        assert_eq!(native(&next), raw);
    }
}
#[test]
#[ignore = "UNRUN: explicit RGB spaces/all4operators use current public semantics, original label/onehistory and state-preserving updatedtrue noop retains redo"]
fn spaces_operators_noop_redo_and_captures() {
    for b in ["linear-rgb", "profile-rgb", "perceptual-rgb"] {
        for c in ["linear-rgb", "profile-rgb", "perceptual-rgb"] {
            for op in ["union", "clip-to-backdrop", "clip-to-layer", "intersection"] {
                let (mut s, _) = session(40);
                let p = wire(&s, 30, b, c, op);
                assert_eq!(s.execute("xcf.composition.set", p).unwrap()["updated"], true);
                assert_eq!(s.active().unwrap().history.past_len(), 1);
            }
        }
    }
    let (mut s, _) = session(0);
    let a = wire(&s, 30, "linear-rgb", "linear-rgb", "union");
    s.execute("xcf.composition.set", a).unwrap();
    let b = wire(&s, 28, "profile-rgb", "profile-rgb", "intersection");
    s.execute("xcf.composition.set", b).unwrap();
    assert!(s.undo_with_control(&ctl()).unwrap());
    let before = s.active().unwrap().doc.clone();
    let h = s.active().unwrap().history.clone();
    let rev = s.active().unwrap().revision;
    let a = wire(&s, 30, "linear-rgb", "linear-rgb", "union");
    assert_eq!(s.execute("xcf.composition.set", a).unwrap()["updated"], true);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert_eq!(s.active().unwrap().revision, rev);
    assert!(s.active().unwrap().history.can_redo());
    assert!(s.redo_with_control(&ctl()).unwrap());
    let mut p = wire(&s, 28, "profile-rgb", "profile-rgb", "intersection");
    p.as_object_mut().unwrap().remove("expectedRevision");
    assert!(s.execute("xcf.composition.set", p).is_err());
    s.active_mut().unwrap().revision = u64::MAX;
    let p = wire(&s, 28, "profile-rgb", "profile-rgb", "intersection");
    assert!(s.execute("xcf.composition.set", p).is_err());
}
#[test]
#[ignore = "UNRUN: inherited content/nativeall/pixels locks before rawnoop, visibilitylock not compositionban, exact20map excludesgrain/legacy/Lab and malformedwire"]
fn locks_and_exact_wire_refusals() {
    for lock in 1..=4 {
        let (mut s, _) = current_control(lock);
        let before = s.active().unwrap().doc.clone();
        let p = wire(&s, 31, "perceptual-rgb", "linear-rgb", "clip-to-backdrop");
        if lock < 4 {
            assert!(s.execute("xcf.composition.set", p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        } else {
            assert_eq!(s.execute("xcf.composition.set", p).unwrap()["updated"], true);
            assert!(s.active().unwrap().doc.shares_state(&before));
            let changed = wire(&s, 30, "linear-rgb", "linear-rgb", "union");
            s.execute("xcf.composition.set", changed).unwrap();
        }
    }
    let (mut s, _) = session(0);
    let base = wire(&s, 30, "linear-rgb", "linear-rgb", "union");
    for (key, value) in [
        ("sourceMode", serde_json::json!(46)),
        ("sourceMode", serde_json::json!(47)),
        ("sourceMode", serde_json::json!(3)),
        ("legacy", serde_json::json!(true)),
        ("blendSpace", serde_json::json!("lab")),
        ("layerId", serde_json::json!("01")),
        ("operator", serde_json::json!("auto")),
        ("extra", serde_json::Value::Null),
    ] {
        let before = s.active().unwrap().doc.clone();
        let mut p = base.clone();
        p.as_object_mut().unwrap().insert(key.into(), value);
        assert!(s.execute("xcf.composition.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
}
#[test]
#[ignore = "UNRUN: full source/history/preferences/view/channel/journal fences, cumulativework/cancellation/fullquota rollback, synchronous prestart"]
fn prepared_fences_and_failure_are_atomic() {
    for fault in 0..5 {
        let (mut s, _) = session(0);
        let before = s.active().unwrap().doc.clone();
        let p = wire(&s, 30, "linear-rgb", "linear-rgb", "union");
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.composition.set", &p, &ctl()).unwrap();
        match fault {
            0 => s.active_mut().unwrap().history.max_states += 1,
            1 => s.active_mut().unwrap().source_read_only = false,
            2 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            3 => s.active_mut().unwrap().view_revision += 1,
            _ => s.journal.push(("external".into(), serde_json::Value::Null)),
        };
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let (mut s, q) = session(0);
    let before = s.active().unwrap().doc.clone();
    let h = s.active().unwrap().history.clone();
    let j = s.journal.clone();
    let p = wire(&s, 30, "linear-rgb", "linear-rgb", "union");
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.composition.set", &p, &c).is_err());
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.composition.set", &p, &c).is_err());
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("xcf.composition.set", p.clone()).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert_eq!(s.journal, j);
    assert!(s.start("xcf.composition.set", p).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
