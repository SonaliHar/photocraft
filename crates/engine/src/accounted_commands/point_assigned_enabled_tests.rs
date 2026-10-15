//! UNRUN assigned-only activation controls. Real original imports and paid
//! title copies are used; intentional Session incoherence is a test condition,
//! never a publication path or source proof.
use super::*;
#[path = "../../../ui-egui/src/canvas_assigned_box_fixture.rs"]
#[allow(dead_code)]
mod fixture;
fn session() -> Session {
    let (path, bytes) = fixture::cases().remove(0);
    let quota = BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let input = photocraft_io::import_owned_xcf_from_reader_in_quota(
        path.file_name().unwrap().to_str().unwrap(),
        &mut std::io::Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        quota,
        &fixture::ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::Point(imported) = input else { panic!("Point carrier") };
    let mut session = Session::new();
    session.add_owned_point_xcf_import(&imported, "assigned Box", None, &fixture::ctl()).unwrap();
    session
}
fn params(session: &Session, enabled: bool) -> Value {
    let id = session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    serde_json::json!({"filterId":id,"enabled":enabled})
}
fn locked_session(lock: usize) -> Session {
    let original = session();
    let owner = original.active().unwrap().doc.accounted().unwrap().clone();
    let q = owner.quota().clone();
    // Prepaid external CURRENT metadata clone; original source/native/profile
    // owners remain in retention. This is not a GIMP-authored locked source.
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = owner.document().clone();
    let layer = doc.layers[0].id;
    match lock {
        1 => doc.layers[0].locks.all = true,
        2 => doc.layers[0].locks.pixels = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.content = true,
        4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.visibility = true,
        _ => {}
    }
    let current = AccountedDocument::new_readonly_assigned_point_storage(doc, std::sync::Arc::new((owner, fee)), q, &fixture::ctl()).unwrap();
    let mut s = Session::new();
    s.add_owned_native_point_archive(fixture::archive(&current), "Locked CURRENT", None, &fixture::ctl()).unwrap();
    s
}
#[test]
#[ignore = "UNRUN: exact assigned-only activation wire, real controlled parser, prepared stale/cancel/work and paid history-current mismatch even for equal activation"]
fn assigned_activation_wire_and_equal_history_integrity_are_independent() {
    for id in ["xcf.filter.set", "xcf.filter.enable"] {
        assert!(assigned_enabled_wire_valid(id, &serde_json::json!({"filterId":"1","enabled":false})));
        for value in [
            serde_json::json!({"filterId":"01","enabled":true}),
            serde_json::json!({"filterId":"1","enabled":null}),
            serde_json::json!({"filterId":"1","enabled":true,"arguments":[]}),
            serde_json::json!({"filterId":"1","enabled":true,"opacityF64Bits":"0x3ff0000000000000"}),
            serde_json::json!({"filterId":"1","enabled":true,"coalesce":null}),
        ] {
            assert!(!assigned_enabled_wire_valid(id, &value));
        }
    }
    let mut s = session();
    let p = params(&s, true);
    let before = s.active().unwrap().doc.clone();
    let titled = before.accounted().unwrap().try_with_document_name("Genuinely paid different CURRENT model", &fixture::ctl()).unwrap();
    let state = s.active().unwrap();
    let view = state
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&titled, state.active_layer, state.accounted_view.as_ref().unwrap().selected(), state.layer_anchor, &fixture::ctl())
        .unwrap();
    s.active_mut().unwrap().accounted_view = Some(view);
    s.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(titled).unwrap();
    let mismatched = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.len();
    assert!(!history.current().unwrap().shares_state(&mismatched));
    assert!(Request::parse_assigned_enabled(&s, "xcf.filter.enable", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&mismatched));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal.len(), journal);
    let mut s = session();
    let p = params(&s, false);
    let before = s.active().unwrap().doc.clone();
    let prepared = Request::parse_assigned_enabled(&s, "xcf.filter.enable", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let stop = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(Request::parse_assigned_enabled(&s, "xcf.filter.enable", &p, &c).is_err());
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(Request::parse_assigned_enabled(&s, "xcf.filter.enable", &p, &c).is_err());
}
#[test]
#[ignore = "UNRUN: real assigned Box full current+staged activation, one history/revision, redo-preserving equal activation, paid reply/journal and unsupported background preflight"]
fn assigned_activation_reuses_one_point_transaction_and_refuses_start() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let before_stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
    let old = s.active().unwrap().revision;
    let past = s.active().unwrap().history.past_len();
    let p = params(&s, false);
    assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
    let disabled = s.active().unwrap().doc.clone();
    let mut expected = before.xcf_state.clone().unwrap();
    expected.filter_stacks[0].nodes[0].enabled = false;
    expected.filter_stacks[0].revision = before_stack.revision + 1;
    assert_eq!(disabled.xcf_state, Some(expected));
    assert_eq!(s.active().unwrap().revision, old + 1);
    assert_eq!(s.active().unwrap().history.past_len(), past + 1);
    assert_eq!(fixture::native(&before), fixture::native(&disabled));
    assert_eq!(before.source_color_context, disabled.source_color_context);
    assert_eq!(before.icc_profile, disabled.icc_profile);
    s.execute("edit.undo", Value::Null).unwrap();
    let history = s.active().unwrap().history.clone();
    let revision = s.active().unwrap().revision;
    let same = params(&s, true);
    assert_eq!(s.execute("xcf.filter.enable", same.clone()).unwrap()["updated"], false);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.active().unwrap().revision, revision);
    s.execute("edit.redo", Value::Null).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&disabled));
    assert!(s.start("xcf.filter.enable", same).is_err());
    assert!(!s.has_jobs());
    assert!(s.active().unwrap().doc.shares_state(&disabled));
    for lock in 1..=4 {
        let mut locked = locked_session(lock);
        let before = locked.active().unwrap().doc.clone();
        let equal = params(&locked, true);
        if lock == 4 {
            assert_eq!(locked.execute("xcf.filter.enable", equal).unwrap()["updated"], false);
        } else {
            assert!(locked.execute("xcf.filter.enable", equal).is_err());
        }
        let changed = params(&locked, false);
        assert!(locked.execute("xcf.filter.set", changed).is_err());
        assert!(locked.active().unwrap().doc.shares_state(&before));
    }
    let mut max = session();
    max.active_mut().unwrap().revision = u64::MAX;
    let equal = params(&max, true);
    assert!(max.execute("xcf.filter.enable", equal).is_err());
}
