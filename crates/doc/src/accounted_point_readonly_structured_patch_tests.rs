//! UNRUN structured CURRENT storage drafts. Borrowed replacement buffers are
//! external test input; the resulting actual model copy owns same-quota fees.
use super::*;

fn fields(count: usize, prefix: &str, seed: u64) -> Vec<FilterArgument> {
    (0..count).map(|i| FilterArgument { name: format!("{prefix}-{i}"), value: FilterValue::Number(ExactNumber::Float64Bits(seed + i as u64)) }).collect()
}
fn config(channels: usize, values: usize) -> FilterValue {
    FilterValue::Config(
        (0..channels).map(|i| FilterArgument { name: format!("channel-{i}"), value: FilterValue::Config(fields(values, "word", (-0f64).to_bits())) }).collect(),
    )
}
fn curves() -> FilterValue {
    FilterValue::Config(
        (0..5)
            .map(|i| FilterArgument {
                name: format!("channel-{i}"),
                value: FilterValue::Curve {
                    free: true,
                    points_f64_bits: Vec::new(),
                    point_types: Vec::new(),
                    samples_f64_bits: (0..256).map(|n| if n == 0 { (-0f64).to_bits() } else { (f64::from(n) / 255.).to_bits() }).collect(),
                },
            })
            .collect(),
    )
}
fn current(value: FilterValue, revision: u64) -> AccountedDocument {
    let q = quota();
    let (mut doc, guard) = fixture(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let stack = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0];
    stack.revision = revision;
    stack.nodes[0].arguments =
        vec![FilterArgument { name: "config".into(), value }, FilterArgument { name: "preserve".into(), value: FilterValue::Bool(true) }];
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}
fn patch<'a>(owner: &AccountedDocument, args: &'a [PointStructuredArgumentPatch<'a>]) -> PointStructuredFilterPatch<'a> {
    PointStructuredFilterPatch {
        layer: owner.document().layers[0].id,
        filter: FilterId(100),
        expected_stack_revision: owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision,
        arguments: args,
        enabled: None,
    }
}
#[test]
fn nested_balance_and_levels_raw_words_copy_once_with_order_and_shared_source_custody() {
    for (channels, values) in [(3, 3), (5, 5)] {
        let owner = current(config(channels, values), 7);
        let mut replacement = config(channels, values);
        let FilterValue::Config(channels) = &mut replacement else { panic!() };
        let FilterValue::Config(words) = &mut channels[0].value else { panic!() };
        words[0].value = FilterValue::Number(ExactNumber::Float64Bits(0x7ff8000000001234));
        let args = [PointStructuredArgumentPatch { name: "config", value: &replacement }];
        let next = owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()).unwrap();
        let mut expected = owner.document().clone();
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.revision = 8;
        stack.nodes[0].arguments[0].value = replacement;
        assert_eq!(next.document(), &expected);
        assert!(next.quota().shares_ledger(owner.quota()));
        let a = &owner.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        let b = &next.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        assert!(Arc::ptr_eq(a.tile(TileCoord::new(1, 0)).unwrap(), b.tile(TileCoord::new(1, 0)).unwrap()));
        assert!(Arc::ptr_eq(owner.document().icc_profile.as_ref().unwrap(), next.document().icc_profile.as_ref().unwrap()));
    }
}
#[test]
fn five_free256_curves_preserve_raw_words_and_exact_noop_at_max_revision_full_quota() {
    let value = curves();
    let owner = current(value, u64::MAX);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let source = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value;
    let shape = point_value_shape(source, &ctl()).unwrap();
    assert_eq!(shape.f64_words, 1280);
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let args = [PointStructuredArgumentPatch { name: "config", value: source }];
    assert!(owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()).unwrap().shares_model(&owner));
    let mut changed = source.clone();
    let FilterValue::Config(channels) = &mut changed else { panic!() };
    let FilterValue::Curve { samples_f64_bits, .. } = &mut channels[0].value else { panic!() };
    samples_f64_bits[0] = 0f64.to_bits();
    let args = [PointStructuredArgumentPatch { name: "config", value: &changed }];
    assert!(matches!(owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()), Err(BackingError::Length)));
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let owner = current(curves(), 3);
    let next = owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()).unwrap();
    assert_eq!(next.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 4);
    assert_eq!(next.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value, changed);
}
#[test]
fn structured_shape_ids_duplicates_and_unsupported_payloads_refuse_before_copy() {
    let owner = current(config(3, 3), 2);
    let before = owner.quota().usage().unwrap();
    let old = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value;
    for mutation in 0..4 {
        let mut next = old.clone();
        let FilterValue::Config(channels) = &mut next else { panic!() };
        match mutation {
            0 => channels.swap(0, 1),
            1 => channels[0].name = "other".into(),
            2 => {
                channels.pop();
            }
            _ => channels[0].value = FilterValue::Bool(false),
        }
        let args = [PointStructuredArgumentPatch { name: "config", value: &next }];
        assert!(owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
    let good = [PointStructuredArgumentPatch { name: "config", value: old }];
    for mutation in 0..3 {
        let mut p = patch(&owner, &good);
        match mutation {
            0 => p.layer = crate::LayerId(0),
            1 => p.filter = FilterId(0),
            _ => p.expected_stack_revision = 3,
        };
        assert!(owner.try_with_readonly_point_structured_filter(p, &ctl()).is_err());
    }
    let duplicate = [good[0], good[0]];
    assert!(owner.try_with_readonly_point_structured_filter(patch(&owner, &duplicate), &ctl()).is_err());
    let unsupported = FilterValue::String("no string admission".into());
    let args = [PointStructuredArgumentPatch { name: "config", value: &unsupported }];
    assert!(owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()).is_err());
    assert_eq!(owner.quota().usage().unwrap(), before);
}
#[test]
fn f64_scalar_signed_zero_and_structured_work_cancel_quota_failures_stay_private() {
    let owner = current(FilterValue::Number(ExactNumber::Float64Bits((-0f64).to_bits())), 1);
    let zero = FilterValue::Number(ExactNumber::Float64Bits(0f64.to_bits()));
    let args = [PointStructuredArgumentPatch { name: "config", value: &zero }];
    let before = owner.quota().usage().unwrap();
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &work), Err(BackingError::Work)));
    let cancel = || owner.quota().usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &c), Err(BackingError::Cancelled)));
    assert_eq!(owner.quota().usage().unwrap(), before);
    let block = owner.quota().reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()).is_err());
    drop(block);
    let next = owner.try_with_readonly_point_structured_filter(patch(&owner, &args), &ctl()).unwrap();
    assert!(!next.shares_model(&owner));
    assert_eq!(next.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 2);
}
#[test]
fn alternating_structured_title_and_layer_copies_keep_flat_original_retention() {
    let original = current(config(3, 3), 0);
    let mut owner = original.clone();
    let mut previous = None;
    for i in 0..12 {
        let value = config(3, 3);
        let args = [PointStructuredArgumentPatch { name: "config", value: &value }];
        let mut p = patch(&owner, &args);
        p.enabled = Some(i % 2 == 0);
        owner = owner.try_with_readonly_point_structured_filter(p, &ctl()).unwrap();
        owner = owner.try_with_document_name(if i % 2 == 0 { "title A" } else { "title B" }, &ctl()).unwrap();
        owner = owner
            .try_with_readonly_layer_properties(
                ReadonlyLayerPropertiesPatch { layer: owner.document().layers[0].id, name: None, visible: Some(i % 2 == 0), opacity_f32_bits: None },
                &ctl(),
            )
            .unwrap();
        assert!(owner.owner.retention.downcast_ref::<OriginalRetention>().unwrap().original.shares_model(&original));
        let usage = owner.quota().usage().unwrap();
        if let Some(previous) = previous {
            assert_eq!(usage, previous);
        }
        previous = Some(usage);
    }
}
