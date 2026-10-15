//! UNRUN exact Signed32 wire, native Posterize transaction and journal drafts.
use super::*;
fn signed(value: i64, width: u64) -> Value {
    json!({"filterId":"100","arguments":[{"name":"levels","value":{"Number":{"Signed":{"width":width,"value":value}}}}]})
}
#[test]
fn signed32_wire_and_per_entry_depth_six_do_not_relax_other_shapes() {
    for value in [i32::MIN, 2, 256, i32::MAX] {
        let p = signed(i64::from(value), 32);
        let fields = wire("xcf.filter.set", &p).unwrap();
        assert!(matches!(fields.arguments[0].unwrap().value,PointArgumentValue::Signed32(v) if v==value));
        assert_eq!(journal_depth("xcf.filter.set", &p), 6);
        let mut census = super::super::super::Census::default();
        census.command_value("xcf.filter.set", &p, &ctl()).unwrap();
        let mut census = super::super::super::Census::default();
        census.retained_command_value("xcf.filter.set", &p, &ctl()).unwrap();
        assert!(super::super::super::Census::default().command_value("layer.setProps", &p, &ctl()).is_err());
    }
    for p in [
        signed(2, 16),
        signed(2, 64),
        signed(i64::from(i32::MAX) + 1, 32),
        signed(i64::from(i32::MIN) - 1, 32),
        json!({"filterId":"100","arguments":[{"name":"levels","value":{"Number":{"Signed":{"width":32,"value":2,"extra":false}}}}]}),
    ] {
        assert!(wire("xcf.filter.set", &p).is_none());
        assert_eq!(journal_depth("xcf.filter.set", &p), 4);
    }
    let mut both = color_patch([0; 4]);
    both["arguments"].as_array_mut().unwrap().push(signed(2, 32)["arguments"][0].clone());
    assert_eq!(journal_depth("xcf.filter.set", &both), 7);
    assert_eq!(journal_depth("xcf.filter.set", &patch(0)), 5);
}
fn posterize_session() -> (Session, BackingQuota) {
    let (mut doc, q, guard, reads, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let metadata = q.reserve(QuotaKind::Index, 32768).unwrap();
    doc.size = Size::new(32, 32);
    doc.depth = SampleType::U8;
    doc.source_color_context.as_mut().unwrap().source_precision = 150;
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let descriptor = NativePlaneDescriptor::new(format, doc.bounds(), &[0; 4]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, 8192).unwrap();
    let raw = [29u8, 127, 223, 255].repeat(32 * 32);
    let store = Arc::new(Store { descriptor, bytes: raw, reads, quota: q.clone(), _fee: fee });
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap(), descriptor, &ctl()).unwrap());
    let bounds = doc.bounds();
    let state = doc.xcf_state.as_mut().unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(bounds);
        if let Some(mask) = &mut item.mask {
            mask.bounds = bounds;
        }
    }
    state.filter_masks[0].bounds = bounds;
    state.filter_masks[0].surface = Surface::new(PixelFormat::GRAY8);
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: "gimp:posterize".into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-point-replace".into() };
    node.arguments = vec![FilterArgument { name: "levels".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 }) }];
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = "xcf-22-stock-point-replace".into();
    policy.stored_blend_space = -261651887;
    policy.stored_composite_space = -261651887;
    policy.stored_operator = -261651887;
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, metadata)), q.clone(), &ctl()).unwrap();
    validate_recipe(&owner, &ctl()).unwrap();
    let mut s = Session::new();
    let state = crate::DocState::new_accounted(owner, None, 16, &ctl()).unwrap();
    s.publish_document_state(state, &ctl()).unwrap();
    (s, q)
}
#[test]
fn registered_posterize_signed_patch_has_exact_noop_redo_native_custody_and_recipe_refusal() {
    let (mut s, _q) = posterize_session();
    let original = s.active().unwrap().doc.clone();
    assert_eq!(s.execute("xcf.filter.set", signed(7, 32)).unwrap()["updated"], true);
    let changed = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert!(matches!(
        changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value,
        FilterValue::Number(ExactNumber::Signed { width: 32, value: 7 })
    ));
    assert_eq!(changed.layers[0].content, original.layers[0].content);
    assert_eq!(changed.source_color_context, original.source_color_context);
    assert_eq!(changed.icc_profile, original.icc_profile);
    assert!(s.undo_with_control(&ctl()).unwrap());
    let history = s.active().unwrap().history.clone();
    assert_eq!(s.execute("xcf.filter.set", signed(4, 32)).unwrap()["updated"], false);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert!(s.redo_with_control(&ctl()).unwrap());
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let revision = s.active().unwrap().revision;
    let journal = s.journal.clone();
    for p in [
        signed(1, 32),
        signed(257, 32),
        signed(-2, 32),
        signed(7, 16),
        json!({"filterId":"100","arguments":[{"name":"levels","value":{"Number":{"Float32Bits":7f32.to_bits()}}}]}),
    ] {
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.journal, journal);
    }
}
