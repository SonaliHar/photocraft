//! UNRUN bounded configured wire, temporary custody and CURRENT transaction drafts.
use super::super::structured;
use super::*;
fn hex(v: f64) -> Value {
    json!(format!("0x{:016x}", v.to_bits()))
}
fn configuration(kind: u8, changed: bool) -> Value {
    let value = if kind == 0 {
        json!({"Config":["shadows","midtones","highlights"].map(|range|json!({"name":range,"value":{"Config":["cyan-red","magenta-green","yellow-blue"].map(|axis|json!({"name":axis,"value":{"Number":{"Float64Bits":hex(if changed && range=="shadows" && axis=="cyan-red" {0.125} else if changed { -0.0 } else {0.0})}}}))}}))})
    } else if kind == 1 {
        json!({"Config":["value","red","green","blue","alpha"].map(|channel|json!({"name":channel,"value":{"Config":[("low-input",0.),("high-input",1.),("gamma",if changed {1.25} else {1.}), ("low-output",0.),("high-output",1.)].map(|(name,v)|json!({"name":name,"value":{"Number":{"Float64Bits":hex(v)}}}))}}))})
    } else {
        json!({"Config":["value","red","green","blue","alpha"].map(|channel|json!({"name":channel,"value":{"Curve":{"free":true,"points_f64_bits":[],"point_types":[],"samples_f64_bits":(0..256).map(|i|hex(if changed && i==0 {-0.0} else if changed && i==128 {0.25} else {f64::from(i)/255.})).collect::<Vec<_>>()}}}))})
    };
    json!({"filterId":"100","arguments":[{"name":"config","value":value}]})
}
fn operation(kind: u8) -> &'static str {
    match kind {
        0 => "gimp:color-balance",
        1 => "gimp:levels",
        _ => "gimp:curves",
    }
}
#[test]
fn exact_structured_shapes_preserve_hex_words_and_only_their_deep_journal_allowance() {
    for kind in 0..3 {
        let p = configuration(kind, true);
        let plan = structured::preflight("xcf.filter.set", &p, &ctl()).unwrap().unwrap();
        assert_eq!(plan.depth, if kind == 2 { 9 } else { 11 });
        assert_eq!(journal_depth("xcf.filter.set", &p), plan.depth);
        super::super::super::Census::default().command_value("xcf.filter.set", &p, &ctl()).unwrap();
        super::super::super::Census::default().retained_command_value("xcf.filter.set", &p, &ctl()).unwrap();
        assert!(super::super::super::Census::default().command_value("layer.setProps", &p, &ctl()).is_err());
        let mut bad = p.clone();
        bad["arguments"][0]["value"]["Config"][0]["name"] = json!("wrong-selector");
        assert!(structured::preflight("xcf.filter.set", &bad, &ctl()).unwrap().is_none());
        assert_eq!(journal_depth("xcf.filter.set", &bad), 4);
    }
    let mut oversized = configuration(2, false);
    oversized["arguments"][0]["value"]["Config"][0]["value"]["Curve"]["samples_f64_bits"].as_array_mut().unwrap().push(hex(0.));
    assert!(structured::preflight("xcf.filter.set", &oversized, &ctl()).unwrap().is_none());
    let mut noncanonical = configuration(0, false);
    noncanonical["arguments"][0]["value"]["Config"][0]["value"]["Config"][0]["value"]["Number"]["Float64Bits"] = json!("0x7FF8000000000000");
    assert!(structured::preflight("xcf.filter.set", &noncanonical, &ctl()).unwrap().is_none());
}
#[test]
fn real_temporary_quota_covers_f64_config_and_curve_vectors_until_payload_drop() {
    for kind in 0..3 {
        let q = BackingQuota::new(QuotaLimits { resident: 4 << 20, index: 2 << 20, scratch: 0 });
        let baseline = q.usage().unwrap();
        let p = configuration(kind, true);
        let plan = structured::preflight("xcf.filter.set", &p, &ctl()).unwrap().unwrap();
        let temporary = structured::Temporary::parse(&p, operation(kind), plan, &q, &ctl()).unwrap();
        let shape = photocraft_doc::accounted_document::point_argument_shape(temporary.arguments(), &ctl()).unwrap();
        assert_eq!(
            shape.f64_words,
            if kind == 0 {
                9
            } else if kind == 1 {
                25
            } else {
                1280
            }
        );
        assert!(q.usage().unwrap().index > baseline.index);
        let FilterValue::Config(selectors) = &temporary.arguments()[0].value else { panic!() };
        if kind == 2 {
            let FilterValue::Curve { samples_f64_bits, .. } = &selectors[0].value else { panic!() };
            assert_eq!(samples_f64_bits[0], (-0.0f64).to_bits());
            assert_eq!(samples_f64_bits[128], 0.25f64.to_bits());
        }
        drop(temporary);
        assert_eq!(q.usage().unwrap(), baseline);
        let blocker = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        assert!(structured::Temporary::parse(&p, operation(kind), plan, &q, &ctl()).is_err());
        drop(blocker);
        assert_eq!(q.usage().unwrap(), baseline);
        let cancelled = || q.usage().unwrap().index > baseline.index;
        let interrupt = Interrupt::cancel_only(&cancelled);
        assert!(structured::Temporary::parse(&p, operation(kind), plan, &q, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
fn session_balance() -> (Session, BackingQuota) {
    let (mut doc, q, guard, _, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let metadata = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut parsed = crate::xcf_filter_cmds::paid_point_patches(&configuration(0, false), operation(0)).unwrap();
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation =
        OperationIdentity { name: operation(0).into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-color-balance-precision-v1".into() };
    node.arguments = vec![FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) }, parsed.remove(0)];
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = node.operation.schema_epoch.clone();
    policy.stored_blend_space = -113999279;
    policy.stored_composite_space = -113999279;
    policy.stored_operator = -113999279;
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, metadata)), q.clone(), &ctl()).unwrap();
    validate_recipe(&owner, &ctl()).unwrap();
    let mut s = Session::new();
    let state = crate::DocState::new_accounted(owner, None, 16, &ctl()).unwrap();
    s.publish_document_state(state, &ctl()).unwrap();
    (s, q)
}
#[test]
fn configured_current_transaction_one_undo_raw_noop_redo_and_refusal_preserve_native_owner() {
    let (mut s, _q) = session_balance();
    let before = s.active().unwrap().doc.clone();
    assert_eq!(s.execute("xcf.filter.set", configuration(0, true)).unwrap()["updated"], true);
    let after = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(after.layers[0].content, before.layers[0].content);
    assert_eq!(after.icc_profile, before.icc_profile);
    assert_eq!(after.source_color_context, before.source_color_context);
    assert!(s.undo_with_control(&ctl()).unwrap());
    let history = s.active().unwrap().history.clone();
    assert_eq!(s.execute("xcf.filter.set", configuration(0, false)).unwrap()["updated"], false);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert!(s.redo_with_control(&ctl()).unwrap());
    let current = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let mut outside = configuration(0, true);
    outside["arguments"][0]["value"]["Config"][0]["value"]["Config"][0]["value"]["Number"]["Float64Bits"] = hex(1.5);
    assert!(s.execute("xcf.filter.set", outside).is_err());
    assert!(s.active().unwrap().doc.shares_state(&current));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
}

#[test]
fn point_only_top_double_metadata_uses_lossless_hex_without_changing_resident_schema() {
    let (mut doc, q, _guard, _, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let _metadata = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let state = doc.xcf_state.as_mut().unwrap();
    let stack = &mut state.filter_stacks[0];
    for bits in [(-0.0f64).to_bits(), 0.5f64.to_bits() + 1, 0x7ff8123456789abc] {
        stack.nodes[0].arguments = vec![FilterArgument { name: "double".into(), value: FilterValue::Number(ExactNumber::Float64Bits(bits)) }];
        let point = crate::xcf_filter_cmds::build_point_storage_inspect_reply(stack.nodes[0].id, stack, &stack.nodes[0]).unwrap();
        let old = crate::xcf_filter_cmds::build_inspect_reply(stack.nodes[0].id, stack, &stack.nodes[0]).unwrap();
        assert_eq!(point["arguments"][0]["state"]["value"]["Number"]["Float64Bits"], json!(format!("0x{bits:016x}")));
        assert_eq!(old["arguments"][0]["state"]["value"]["Number"]["Float64Bits"], json!(bits));
    }
}

#[test]
fn aggregate_free1280_plus_top_double_is_rejected_during_borrowed_preflight() {
    let original = configuration(2, false);
    assert!(structured::preflight("xcf.filter.set", &original, &ctl()).unwrap().is_some());
    for first in [false, true] {
        let mut mixed = original.clone();
        let extra = json!({"name":"extra-double","value":{"Number":{"Float64Bits":hex(0.125)}}});
        let args = mixed["arguments"].as_array_mut().unwrap();
        if first {
            args.insert(0, extra);
        } else {
            args.push(extra);
        }
        // The borrowed scan admits neither a DTO nor a temporary quota fee.
        // All five Curve vectors and the independent scalar share one cap.
        assert!(structured::preflight("xcf.filter.set", &mixed, &ctl()).is_err());
        assert_eq!(journal_depth("xcf.filter.set", &mixed), 4);
        assert!(super::super::super::Census::default().command_value("xcf.filter.set", &mixed, &ctl()).is_err());
    }
}
