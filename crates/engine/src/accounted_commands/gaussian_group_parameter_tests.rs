//! UNRUN CURRENT parameter transactions. Reuses genuine original-source and
//! separately prepaid CURRENT lock fixtures; captures are external harness data.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
use std::sync::atomic::{AtomicU64, Ordering};
fn parameters(filter: FilterId, x: Option<u32>, y: Option<u32>, enabled: Option<bool>) -> Value {
    // Reverse request order intentionally; the stored argument order must survive.
    let args: Vec<_> = [("std-dev-y", y), ("std-dev-x", x)]
        .into_iter()
        .filter_map(|(name, bits)| bits.map(|bits| serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":bits}}})))
        .collect();
    let mut p = serde_json::json!({"filterId":filter.0.to_string(),"arguments":args});
    if let Some(enabled) = enabled {
        p.as_object_mut().unwrap().insert("enabled".into(), Value::Bool(enabled));
    }
    p
}
fn axis(s: &Session, name: &str) -> u32 {
    match &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|a| a.name == name).unwrap().value {
        FilterValue::Number(ExactNumber::Float32Bits(bits)) => *bits,
        _ => panic!(),
    }
}
#[test]
fn exact_two_axis_wire_preserves_bits_and_existing_journal_depth() {
    for (x, y) in [(Some(0.0f32.to_bits()), None), (None, Some(15.0f32.to_bits())), (Some((-0.0f32).to_bits()), Some(0.0f32.to_bits()))] {
        let p = parameters(FilterId(1), x, y, Some(false));
        let fields = wire("xcf.filter.set", &p).unwrap();
        assert_eq!((fields.x, fields.y, fields.enabled, fields.parameters), (x, y, Some(false), true));
        assert!(wire("xcf.filter.enable", &p).is_none());
        let mut c = crate::accounted_commands::Census::default();
        c.command_value("xcf.filter.set", &p, &ctl()).unwrap();
        let mut old = crate::accounted_commands::Census::default();
        assert!(old.value(&p, 0, &ctl()).is_err());
    }
    assert!(wire("xcf.filter.set", &parameters(FilterId(1), None, None, Some(false))).is_none());
    for value in [-f32::MIN_POSITIVE, 15.000001, f32::INFINITY, f32::NAN] {
        assert!(wire("xcf.filter.set", &parameters(FilterId(1), Some(value.to_bits()), None, None)).is_none());
    }
    for p in [
        serde_json::json!({"filterId":"1","arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":0}}},{"name":"std-dev-x","value":{"Number":{"Float32Bits":0}}}]}),
        serde_json::json!({"filterId":"1","arguments":[{"name":"std-dev-x","value":{"Number":{"Float64Bits":"0x0000000000000000"}}}]}),
        serde_json::json!({"filterId":"1","arguments":[{"name":"filter","value":{"Enum":{"domain":"gegl:gaussian-blur::filter","value":0}}}]}),
        serde_json::json!({"filterId":"1","arguments":[{"name":"std-dev-y","value":{"Number":{"Float32Bits":0}}}],"opacityF64Bits":"0x3ff0000000000000"}),
    ] {
        assert!(wire("xcf.filter.set", &p).is_none());
    }
    let unrelated = parameters(FilterId(1), Some(0), None, None);
    let mut c = crate::accounted_commands::Census::default();
    assert!(c.command_value("unrelated.command", &unrelated, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: actual two sources independent0..15 axes/combined activation, stable storedorder/raw source/native/profile/masks, one revision/history/noopredo/native journal continuation"]
fn current_parameters_are_one_atomic_paid_change() {
    for case in 0..CASES.len() {
        for (x, y, enabled) in [(0.0f32, 1.3f32, None), (0.8, 0.0, None), (0.0, 0.0, None), (1.3, 0.8, Some(false)), (15.0, 15.0, None)] {
            let (mut s, q) = session(case);
            let initial = s.active().unwrap().doc.clone();
            let raw = native(&s);
            let source = s.active().unwrap().source.clone();
            let names: Vec<_> = initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().map(|a| a.name.clone()).collect();
            let stack_revision = initial.xcf_state.as_ref().unwrap().filter_stacks[0].revision;
            let revision = s.active().unwrap().revision;
            let p = parameters(id(&s), Some(x.to_bits()), Some(y.to_bits()), enabled);
            assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Set GIMP Group Filter Parameters"));
            let changed = s.active().unwrap().doc.clone();
            let state = changed.xcf_state.as_ref().unwrap();
            assert_eq!(state.filter_stacks[0].revision, stack_revision + 1);
            assert_eq!(axis(&s, "std-dev-x"), x.to_bits());
            assert_eq!(axis(&s, "std-dev-y"), y.to_bits());
            assert_eq!(state.filter_stacks[0].nodes[0].enabled, enabled.unwrap_or(true));
            assert_eq!(state.filter_stacks[0].nodes[0].arguments.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), names);
            assert_eq!(state.filter_stacks[0].nodes[0].composition, initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].composition);
            assert_eq!(state.filter_masks, initial.xcf_state.as_ref().unwrap().filter_masks);
            assert_eq!(native(&s), raw);
            assert_eq!(s.active().unwrap().source, source);
            assert_eq!(changed.icc_profile, initial.icc_profile);
            assert_eq!(changed.source_color_context, initial.source_color_context);
            let h = s.active().unwrap().history.clone();
            let rev = s.active().unwrap().revision;
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            assert_eq!(s.active().unwrap().revision, rev);
            assert!(s.active().unwrap().history.shares_accounted_history(&h));
            assert!(s.undo_with_control(&ctl()).unwrap());
            let h = s.active().unwrap().history.clone();
            let original = parameters(id(&s), Some(0.8f32.to_bits()), Some(1.3f32.to_bits()), None);
            assert_eq!(s.execute("xcf.filter.set", original).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.shares_accounted_history(&h));
            assert!(s.active().unwrap().history.can_redo());
            assert!(s.redo_with_control(&ctl()).unwrap());
            let request = serde_json::json!({"filterId":id(&s).0.to_string()});
            let reply = crate::accounted_filter_inspect::prepare(&s, &request, &ctl()).unwrap();
            let row = reply.value()["arguments"].as_array().unwrap().iter().find(|a| a["name"] == "std-dev-x").unwrap();
            assert_eq!(row["state"]["value"]["Number"]["Float32Bits"], x.to_bits());
            drop(reply);
            let bytes = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "ParametersGaussian.pcraft", &ctl())
                .unwrap()
                .write_and_publish(&mut s, &ctl(), |doc, _| {
                    photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                        .map(|w| w.into_inner())
                        .map_err(|e| EngineError::Other(e.to_string()))
                })
                .unwrap();
            let mut reopened = Session::new();
            reopened.add_owned_native_isolated_gaussian_group_archive(archive(&bytes, &q), "current", None, &ctl()).unwrap();
            assert_eq!(reopened.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
            assert_eq!(native(&reopened), raw);
        }
    }
}
#[test]
#[ignore = "UNRUN: raw signedzero is a change, bothzero remains enabled, label depends argumentspresence even activationonly changes"]
fn signed_zero_and_argument_presence_do_not_normalize() {
    let (mut s, _) = session(0);
    let p = parameters(id(&s), Some(0), Some(0), None);
    s.execute("xcf.filter.set", p).unwrap();
    assert!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    let p = parameters(id(&s), Some((-0.0f32).to_bits()), None, None);
    let revision = s.active().unwrap().revision;
    assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
    assert_eq!(s.active().unwrap().revision, revision + 1);
    assert_eq!(axis(&s, "std-dev-x"), (-0.0f32).to_bits());
    assert!(s.undo_with_control(&ctl()).unwrap());
    let h = s.active().unwrap().history.clone();
    let same = parameters(id(&s), Some(0), None, None);
    assert_eq!(s.execute("xcf.filter.set", same).unwrap()["updated"], false);
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert!(s.active().unwrap().history.can_redo());
    assert!(s.redo_with_control(&ctl()).unwrap());
    let combined = parameters(id(&s), Some((-0.0f32).to_bits()), None, Some(false));
    assert_eq!(s.execute("xcf.filter.set", combined).unwrap()["updated"], true);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Set GIMP Group Filter Parameters"));
    let p = enable(&s, true);
    assert_eq!(s.execute("xcf.filter.enable", p).unwrap()["updated"], true);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Set GIMP Group Filter Enabled"));
}
#[test]
#[ignore = "UNRUN: locked/alternate disabled tuple and mixed valid-invalid requests refuse atomically; finite cumulative stage work/quota preserves undo redo"]
fn parameter_refusals_and_cumulative_failures_have_no_prefix() {
    for lock in 1..=3 {
        let (mut s, _) = current_control(lock, false);
        let before = s.active().unwrap().doc.clone();
        let p = parameters(id(&s), Some(0.8f32.to_bits()), None, None);
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let (mut s, _) = current_control(4, false);
    let p = parameters(id(&s), Some(1.3f32.to_bits()), None, Some(true));
    assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], true);
    let before = s.active().unwrap().doc.clone();
    let p = parameters(id(&s), Some(0), None, Some(false));
    assert!(s.execute("xcf.filter.set", p).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let (mut s, _) = current_control(0, true);
    let p = parameters(id(&s), Some(0), None, None);
    s.execute("xcf.filter.set", p).unwrap();
    let before = s.active().unwrap().doc.clone();
    let p = parameters(id(&s), Some(15.0f32.to_bits()), None, Some(true));
    assert!(s.execute("xcf.filter.set", p).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let (mut s, q) = session(0);
    let changed = parameters(id(&s), Some(1.3f32.to_bits()), None, None);
    s.execute("xcf.filter.set", changed).unwrap();
    assert!(s.undo_with_control(&ctl()).unwrap());
    let before = s.active().unwrap().doc.clone();
    let h = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let p = parameters(id(&s), Some(0), Some(16.0f32.to_bits()), Some(false));
    assert!(s.execute("xcf.filter.set", p).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let spent = AtomicU64::new(0);
    let measure = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    validate_recipe(before.accounted().unwrap(), &ReadControl { interrupt: &Interrupt::NONE, work: &measure }).unwrap();
    let cost = spent.load(Ordering::Relaxed);
    let spent = AtomicU64::new(0);
    let work = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cost + cost / 2 { Err(BackingError::Work) } else { Ok(()) };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let p = parameters(id(&s), Some(15.0f32.to_bits()), Some(15.0f32.to_bits()), None);
    assert!(
        crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &c).and_then(|prepared| prepared.execute(&mut s, &c)).is_err()
    );
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert!(s.active().unwrap().history.can_redo());
    assert_eq!(s.journal, journal);
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("xcf.filter.set", p).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal, journal);
}
