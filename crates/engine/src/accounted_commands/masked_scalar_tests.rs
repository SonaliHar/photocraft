//! Deferred genuine-source command contracts; no producer numerical output assertion.
use super::*;
#[path = "../accounted_masked_scalar_test_support.rs"]
mod fixture;
fn session(case: &serde_json::Value) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::Point(input) = &input else { panic!("actual Point") };
    let mut s = Session::new();
    s.add_owned_point_xcf_import(input, "masked-stock.xcf", fixture::path(case).to_str(), &fixture::ctl()).unwrap();
    s
}
fn argument(name: &str, value: f32) -> serde_json::Value {
    serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":value.to_bits()}}})
}
#[test]
#[ignore = "UNRUN genuine16 all/half ordinary Point Sources; exact0/0/2/4 tuple and combined enable, fourth Levels field, source/native/mask custody, undo and existing Point wire noops"]
fn masked_scalar_actual_sources_and_atomic_tuples() {
    for case in fixture::cases() {
        let mut s = session(&case);
        let old = s.active().unwrap().doc.clone();
        let node = &old.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id;
        let kind = crate::xcf_filter_cmds::masked_scalar_point_edit_kind(old.accounted().unwrap(), node).unwrap();
        assert!(crate::accounted_native_open::is_current_pure_point_work_candidate(old.accounted().unwrap(), &fixture::ctl()).unwrap());
        // Existing ordinary Point grammar, intentionally different from the mixed outer parser.
        s.execute("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"arguments":[],"enabled":true})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&old));
        s.execute("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3ff0000000000000"})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&old));
        let args = match kind {
            photocraft_compose::MaskedScalarKind::BrightnessContrast => vec![argument("contrast", 0.5), argument("brightness", 0.125)],
            photocraft_compose::MaskedScalarKind::GeglLevels => {
                vec![argument("in-low", 0.0625), argument("in-high", 0.9375), argument("out-low", 0.125), argument("out-high", 0.875)]
            }
            _ => vec![],
        };
        let revision = s.active().unwrap().revision;
        let params = if args.is_empty() {
            serde_json::json!({"filterId":id.0.to_string(),"enabled":false})
        } else {
            serde_json::json!({"filterId":id.0.to_string(),"arguments":args,"enabled":false})
        };
        s.execute("xcf.filter.set", params).unwrap();
        assert_eq!(s.active().unwrap().revision, revision + 1);
        let now = &s.active().unwrap().doc;
        fixture::payload(&old, now);
        let changed = &now.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(!changed.enabled);
        assert_eq!(changed.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), node.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>());
        if kind == photocraft_compose::MaskedScalarKind::GeglLevels {
            assert!(changed.arguments.iter().any(|a| a.name == "out-high"
                && a.value == photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(0.875f32.to_bits()))));
        }
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&old, &s.active().unwrap().doc);
        fixture::observed(&s.active().unwrap().doc, &case);
    }
}
#[test]
#[ignore = "UNRUN strict incoming types/domain/whole merged Levels tuple, no new opacity/Move authority, early Work/cancel and atomic refusal on genuine16 Sources"]
fn masked_scalar_refusals_keep_current_and_source() {
    for case in fixture::cases() {
        let mut s = session(&case);
        let old = s.active().unwrap().doc.clone();
        let n = &old.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = n.id.0.to_string();
        let mut bad = vec![
            serde_json::json!({"filterId":id}),
            serde_json::json!({"filterId":id,"arguments":[argument("unknown",0.0)]}),
            serde_json::json!({"filterId":id,"opacityF64Bits":"0x3fe0000000000000"}),
            serde_json::json!({"filterId":id,"arguments":[{"name":"brightness","value":{"Boolean":true}}]}),
        ];
        if n.operation.name == "gegl:levels" {
            // Each changed word is finite; the complete merged input interval is invalid.
            bad.push(serde_json::json!({"filterId":id,"arguments":[argument("in-low",0.99)]}));
        }
        if n.operation.name == "gegl:brightness-contrast" {
            bad.push(serde_json::json!({"filterId":id,"arguments":[argument("brightness",4.0)]}));
        }
        for params in bad {
            assert!(s.execute("xcf.filter.set", params).is_err());
            assert!(s.active().unwrap().doc.shares_state(&old));
        }
        assert!(s.execute("xcf.filter.move", serde_json::json!({"filterId":id,"index":0})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&old));
        let params = serde_json::json!({"filterId":id,"enabled":false});
        let deny = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        assert!(Request::parse(&s, "xcf.filter.enable", &params, &c).is_err());
        let cancelled = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(Request::parse(&s, "xcf.filter.enable", &params, &c).is_err());
        assert!(s.active().unwrap().doc.shares_state(&old));
    }
}
