//! UNRUN structural guard drafts; intentionally not source/render qualification.
use super::*;

#[test]
#[ignore = "UNRUN: observational epoch refuses unrelated stack/layer/pixel/selection edits before native reads"]
fn observational_epoch_is_readonly_before_wire_helpers_or_native_reads() {
    let (mut doc, q, guard, reads, _, _) = fixture(ColorMode::Rgb, SampleType::F32, SampleEncoding::NonlinearProfile);
    let metadata = q.reserve(QuotaKind::Index, 32768).unwrap();
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation = OperationIdentity {
        name: "gegl:box-blur".into(),
        stored_version: "0:0".into(),
        schema_epoch: photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH.into(),
    };
    node.arguments = vec![FilterArgument { name: "radius".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 }) }];
    node.composition.as_mut().unwrap().compatibility_epoch = photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH.into();
    // Deliberately structural internal owner: missing source-level proof must
    // never turn this observational epoch into mutation authority.
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, metadata)), q, &ctl()).unwrap();
    let mut s = Session::new();
    let state = crate::DocState::new_accounted(owner, None, 16, &ctl()).unwrap();
    s.publish_document_state(state, &ctl()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let revision = s.active().unwrap().revision;
    reads.store(0, Ordering::Relaxed);
    for (id, params) in [
        ("xcf.filter.move", json!({"filterId":"100","index":0})),
        ("xcf.filter.remove", json!({"filterId":"100"})),
        ("layer.setProps", json!({"blend":"multiply"})),
        ("edit.clear", json!({})),
        ("edit.fill", json!({})),
        ("select.all", json!({})),
    ] {
        if !matches!(id,"layer.setProps"|"edit.clear"|"edit.fill") { assert!(super::super::super::disabled_reason(&s, id, &Value::Null).unwrap().is_some()); }
        if id.starts_with("xcf.filter.") {
            assert!(Request::parse(&s, id, &params, &ctl()).is_err());
        }
        assert!(s.execute(id, params).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.journal, journal);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}
