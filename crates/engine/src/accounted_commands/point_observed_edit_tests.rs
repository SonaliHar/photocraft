//! UNRUN paid synthetic CURRENT controls, not new GIMP source evidence.
use super::*;
#[path = "../../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "../../../ui-egui/src/canvas_observed_box_fixture.rs"]
#[allow(dead_code)]
mod observed;
use photocraft_doc::{
    SampleType,
    xcf_filters::{ExactNumber, FilterValue},
};
use serde_json::json;
fn session(sample: SampleType, assigned: bool) -> Session {
    let (base, _) = fixture::fixture(sample, assigned, 1);
    let owner = observed::with_raw(&base, [i32::MIN, 0, i32::MAX], &fixture::ctl());
    let mut s = Session::new();
    s.add_owned_native_point_archive(fixture::archive(&owner), "Observed CURRENT", None, &fixture::ctl()).unwrap();
    s
}
fn patch(s: &Session, radius: i32, enabled: Option<bool>) -> Value {
    let id = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let mut p = json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":32,"value":radius}}}}]});
    if let Some(enabled) = enabled {
        p.as_object_mut().unwrap().insert("enabled".into(), json!(enabled));
    }
    p
}
#[test]
#[ignore = "UNRUN: three genuine paid CURRENT/native Box roles, endpoint radii and mixed activation, one transaction/noopredo/raw policy/profile/mask/native persistence; no source or numerical acceptance claim"]
fn observed_radius_and_activation_are_one_paid_point_transaction() {
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        let mut s = session(sample, assigned);
        let original = s.active().unwrap().doc.clone();
        for (radius, enabled) in [(0, false), (0, true), (1, true), (1000, true), (5, false)] {
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let revision = s.active().unwrap().revision;
            let p = patch(&s, radius, Some(enabled));
            assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
            let changed = s.active().unwrap().doc.clone();
            let stack = &changed.xcf_state.as_ref().unwrap().filter_stacks[0];
            assert_eq!(stack.revision, before.xcf_state.as_ref().unwrap().filter_stacks[0].revision + 1);
            assert_eq!(stack.nodes[0].arguments[0].value, FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(radius) }));
            assert_eq!(stack.nodes[0].enabled, enabled);
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
            assert_eq!(fixture::native(&changed), fixture::native(&original));
            assert_eq!(changed.icc_profile, original.icc_profile);
            assert_eq!(changed.source_color_context, original.source_color_context);
            assert_eq!(changed.xcf_state.as_ref().unwrap().filter_masks, original.xcf_state.as_ref().unwrap().filter_masks);
            assert_eq!(observed::raw(&changed), [i32::MIN, 0, i32::MAX]);
            s.execute("edit.undo", Value::Null).unwrap();
            let old = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let FilterValue::Number(ExactNumber::Signed { value, .. }) = old.arguments[0].value else { panic!("radius") };
            let same = patch(&s, i32::try_from(value).unwrap(), Some(old.enabled));
            let retained = s.active().unwrap().history.clone();
            assert_eq!(s.execute("xcf.filter.set", same).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.shares_accounted_history(&retained));
            s.execute("edit.redo", Value::Null).unwrap();
            assert!(s.active().unwrap().doc.shares_state(&changed));
        }
        let current = s.active().unwrap().doc.clone();
        let id = current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        for (command, enabled) in [("xcf.filter.enable", true), ("xcf.filter.set", false)] {
            assert_eq!(s.execute(command, json!({"filterId":id,"enabled":enabled})).unwrap()["updated"], true);
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        }
        let current = s.active().unwrap().doc.clone();
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(fixture::archive(current.accounted().unwrap()), "continued", None, &fixture::ctl()).unwrap();
        fixture::exact(&current, &reopened.active().unwrap().doc);
        let p = patch(&reopened, 4, Some(true));
        assert_eq!(reopened.execute("xcf.filter.set", p).unwrap()["updated"], true);
    }
}
#[test]
#[ignore = "UNRUN: bounded observed wire, paid history mismatch on equal request, prepared stale/control refusals, synchronous jobs and unrelated mutation closure"]
fn observed_wire_and_noop_cannot_bypass_current_state_or_control() {
    for value in [-1, 1001] {
        assert!(!observed_box_wire_valid("xcf.filter.set", &patch(&session(SampleType::F32, false), value, None)));
    }
    for p in [
        json!({"filterId":"1","arguments":[]}),
        json!({"filterId":"01","enabled":true}),
        json!({"filterId":"1","enabled":true,"opacityF64Bits":"0x3ff0000000000000"}),
        json!({"filterId":"1","arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":64,"value":1}}}}]}),
        json!({"filterId":"1","arguments":[{"name":"other","value":{"Bool":true}}]}),
        json!({"filterId":"1","enabled":true,"coalesce":null}),
    ] {
        assert!(!observed_box_wire_valid("xcf.filter.set", &p));
    }
    for assigned in [false, true] {
        let mut s = session(SampleType::F32, assigned);
        let before = s.active().unwrap().doc.clone();
        let owner = before.accounted().unwrap().try_with_document_name("Paid divergent history", &fixture::ctl()).unwrap();
        let state = s.active().unwrap();
        let view = state
            .accounted_view
            .as_ref()
            .unwrap()
            .try_replace(&owner, state.active_layer, state.accounted_view.as_ref().unwrap().selected(), state.layer_anchor, &fixture::ctl())
            .unwrap();
        s.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(owner).unwrap();
        s.active_mut().unwrap().accounted_view = Some(view);
        let p = patch(&s, 4, Some(true));
        assert!(Request::parse_observed_box(&s, "xcf.filter.set", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).is_err());
        let mut s = session(SampleType::F32, assigned);
        let p = patch(&s, 0, Some(false));
        let before = s.active().unwrap().doc.clone();
        let prepared = Request::parse_observed_box(&s, "xcf.filter.set", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
        s.active_mut().unwrap().view_revision += 1;
        assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        let work = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(Request::parse_observed_box(&s, "xcf.filter.set", &p, &work).is_err());
        let stop = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&stop);
        assert!(Request::parse_observed_box(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        assert!(s.start("xcf.filter.set", p).is_err());
        assert!(!s.has_jobs());
        for (id, p) in [("layer.setProps", json!({"name":"closed"})), ("layer.translate", json!({"dx":1})), ("xcf.filter.remove", json!({"filterId":"1"}))] {
            assert!(s.execute(id, p).is_err());
        }
        s.active_mut().unwrap().revision = u64::MAX;
        let same = patch(&s, 4, Some(true));
        assert!(s.execute("xcf.filter.set", same).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    for lock in 1..=4 {
        let base = session(SampleType::F32, false);
        let owner = base.active().unwrap().doc.accounted().unwrap().clone();
        // External CURRENT lock control: pay before clone and keep every source,
        // profile and native owner. No producer-authored locked-file claim.
        let fee = owner.quota().reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = owner.document().clone();
        let layer = doc.layers[0].id;
        match lock {
            1 => doc.layers[0].locks.all = true,
            2 => doc.layers[0].locks.pixels = true,
            3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.content = true,
            4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.visibility = true,
            _ => {}
        }
        let current =
            AccountedDocument::new_readonly_point_storage(doc, std::sync::Arc::new((owner.clone(), fee)), owner.quota().clone(), &fixture::ctl()).unwrap();
        let mut s = Session::new();
        s.add_owned_native_point_archive(fixture::archive(&current), "Locked CURRENT", None, &fixture::ctl()).unwrap();
        let before = s.active().unwrap().doc.clone();
        let same = patch(&s, 4, Some(true));
        if lock == 4 {
            assert_eq!(s.execute("xcf.filter.set", same).unwrap()["updated"], false);
            let p = patch(&s, 1, None);
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], true);
            s.execute("edit.undo", Value::Null).unwrap();
        } else {
            assert!(s.execute("xcf.filter.set", same).is_err());
        }
        let changed = patch(&s, 0, Some(false));
        assert!(s.execute("xcf.filter.set", changed).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
}
