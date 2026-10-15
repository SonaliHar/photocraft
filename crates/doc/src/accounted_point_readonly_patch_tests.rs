//! UNRUN storage-only typed Point patch drafts. Unqualified synthetic operation
//! identity remains intentionally unchanged; no evaluator or source is granted.
use super::*;

fn current(color: bool) -> AccountedDocument {
    let q = quota();
    let (mut doc, guard) = fixture(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    if color {
        let mut components = Vec::with_capacity(13);
        components.extend([0x80000000, 0x7fc01234, 0x3f123456, 0x3f800000].into_iter().map(ExactNumber::Float32Bits));
        node.arguments[9].value = FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None };
    }
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].revision = 7;
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}
fn patch<'a>(owner: &AccountedDocument, arguments: &'a [PointArgumentPatch<'a>], enabled: Option<bool>) -> PointFilterPatch<'a> {
    PointFilterPatch {
        layer: owner.document().layers[0].id,
        filter: FilterId(100),
        expected_stack_revision: owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision,
        arguments,
        enabled,
    }
}
#[test]
fn mixed_raw_replacements_preserve_order_domains_outer_native_mask_profile_and_source() {
    let owner = current(true);
    let before = owner.document();
    let arguments = [
        PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Float32Bits(0x7fc05678) },
        PointArgumentPatch { name: "raw-1", value: PointArgumentValue::Bool(true) },
        PointArgumentPatch { name: "raw-2", value: PointArgumentValue::Enum(i32::MIN) },
    ];
    let new = owner.try_with_readonly_point_filter(patch(&owner, &arguments, Some(true)), &ctl()).unwrap();
    let mut expected = before.clone();
    let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
    stack.revision = 8;
    let node = &mut stack.nodes[0];
    node.enabled = true;
    node.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0x7fc05678));
    node.arguments[1].value = FilterValue::Bool(true);
    let FilterValue::Enum { value, .. } = &mut node.arguments[2].value else { panic!() };
    *value = i32::MIN;
    assert_eq!(new.document(), &expected);
    assert_eq!(before.xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
    let a = &before.xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let b = &new.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    assert!(Arc::ptr_eq(a.tile(TileCoord::new(1, 0)).unwrap(), b.tile(TileCoord::new(1, 0)).unwrap()));
    assert_eq!(a.default_bytes(), b.default_bytes());
    assert!(Arc::ptr_eq(before.icc_profile.as_ref().unwrap(), new.document().icc_profile.as_ref().unwrap()));
    let LayerContent::BackedRaster(a) = &before.layers[0].content else { panic!() };
    let LayerContent::BackedRaster(b) = &new.document().layers[0].content else { panic!() };
    assert_eq!(a.descriptor(), b.descriptor());
    assert!(new.quota().shares_ledger(owner.quota()));
}
#[test]
fn exact_noop_precedes_revision_increment_and_allocation_but_signed_zero_changes() {
    let owner = owned(SampleType::F64, SampleEncoding::Perceptual, 775);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let arguments = [PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Float32Bits((-0f32).to_bits()) }];
    assert!(owner.try_with_readonly_point_filter(patch(&owner, &arguments, Some(false)), &ctl()).unwrap().shares_model(&owner));
    let positive = [PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Float32Bits(0f32.to_bits()) }];
    assert!(matches!(owner.try_with_readonly_point_filter(patch(&owner, &positive, None), &ctl()), Err(BackingError::Length)));
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let owner = current(false);
    let next = owner.try_with_readonly_point_filter(patch(&owner, &positive, None), &ctl()).unwrap();
    assert_eq!(next.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 8);
}
#[test]
fn stale_ids_duplicates_missing_names_and_wrong_types_refuse_without_copy() {
    let owner = current(true);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let good = [PointArgumentPatch { name: "raw-1", value: PointArgumentValue::Bool(true) }];
    for case in 0..3 {
        let mut p = patch(&owner, &good, None);
        match case {
            0 => p.layer = crate::LayerId(0),
            1 => p.filter = FilterId(0),
            _ => p.expected_stack_revision = 8,
        };
        assert!(owner.try_with_readonly_point_filter(p, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    for args in [
        vec![good[0], good[0]],
        vec![PointArgumentPatch { name: "missing", value: PointArgumentValue::Bool(true) }],
        vec![PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Bool(true) }],
        vec![PointArgumentPatch { name: "raw-9", value: PointArgumentValue::Float32Bits(0) }],
        vec![good[0]; 11],
    ] {
        assert!(owner.try_with_readonly_point_filter(patch(&owner, &args, None), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let q2 = quota();
    let (mut doc, guard) = fixture(&q2, SampleType::U8, SampleEncoding::NonlinearProfile, 150);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1].name = "raw-0".into();
    let duplicate = AccountedDocument::new_readonly_point_storage(doc, guard, q2, &ctl()).unwrap();
    let args = [PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Float32Bits(0) }];
    assert!(duplicate.try_with_readonly_point_filter(patch(&duplicate, &args, None), &ctl()).is_err());
}
#[test]
fn exact_color_shape_and_spare_capacity_are_censused_and_copied_without_normalization() {
    let owner = current(true);
    let renamed = owner.try_with_document_name("color kept", &ctl()).unwrap();
    let a = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[9].value;
    let b = &renamed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[9].value;
    assert_eq!(a, b);
    let FilterValue::Color { components, .. } = a else { panic!() };
    assert_eq!(components.capacity(), 13);
    let FilterValue::Color { components: copied, .. } = b else { panic!() };
    assert!(copied.capacity() >= 4 && copied.capacity() <= 8);
    for case in 0..4 {
        let q = quota();
        let (mut doc, guard) = fixture(&q, SampleType::F32, SampleEncoding::Linear, 600);
        let mut components = vec![ExactNumber::Float32Bits(0); 4];
        let mut encoding = SampleEncoding::NonlinearProfile;
        match case {
            0 => {
                components.pop();
            }
            1 => components[0] = ExactNumber::Float64Bits(0),
            2 => encoding = SampleEncoding::Linear,
            _ => components.push(ExactNumber::Float32Bits(0)),
        }
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Color { components, encoding, profile: None };
        assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}

#[test]
fn color_word_patch_and_scalar_companion_preserve_representation_and_change_one_revision() {
    let owner = current(true);
    let words = [0, (-0f32).to_bits(), f32::NAN.to_bits(), f32::INFINITY.to_bits()];
    let args = [
        PointArgumentPatch { name: "raw-9", value: PointArgumentValue::Color4F32(words) },
        PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Float32Bits(0x3f100001) },
    ];
    let new = owner.try_with_readonly_point_filter(patch(&owner, &args, Some(true)), &ctl()).unwrap();
    let before = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    let after = &new.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    assert_eq!(after.revision, before.revision + 1);
    let mut expected = before.nodes[0].clone();
    expected.enabled = true;
    expected.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0x3f100001));
    let FilterValue::Color { components, encoding, profile } = &mut expected.arguments[9].value else { panic!() };
    for (component, bits) in components.iter_mut().zip(words) {
        *component = ExactNumber::Float32Bits(bits);
    }
    assert_eq!(*encoding, SampleEncoding::NonlinearProfile);
    assert!(profile.is_none());
    assert_eq!(after.nodes[0], expected);
    let FilterValue::Color { components: actual, .. } = &after.nodes[0].arguments[9].value else { panic!() };
    assert!(actual.capacity() >= 4 && actual.capacity() <= 8);
    assert_eq!(new.document().source_color_context, owner.document().source_color_context);
    let old_mask = &owner.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let new_mask = &new.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    assert!(Arc::ptr_eq(old_mask.tile(TileCoord::new(1, 0)).unwrap(), new_mask.tile(TileCoord::new(1, 0)).unwrap()));
}

#[test]
fn raw_color_noop_at_revision_max_and_full_quota_precedes_allocations_and_type_changes_refuse() {
    let q = quota();
    let (mut doc, guard) = fixture(&q, SampleType::F32, SampleEncoding::NonlinearProfile, 650);
    let words = [(-0f32).to_bits(), 0x7fc01234, 0x3f123456, 1f32.to_bits()];
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[9].value =
        FilterValue::Color { components: words.into_iter().map(ExactNumber::Float32Bits).collect(), encoding: SampleEncoding::NonlinearProfile, profile: None };
    let owner = AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).unwrap();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let args = [PointArgumentPatch { name: "raw-9", value: PointArgumentValue::Color4F32(words) }];
    assert!(owner.try_with_readonly_point_filter(patch(&owner, &args, None), &ctl()).unwrap().shares_model(&owner));
    let mut changed = words;
    changed[0] = 0;
    let changed = [PointArgumentPatch { name: "raw-9", value: PointArgumentValue::Color4F32(changed) }];
    assert!(matches!(owner.try_with_readonly_point_filter(patch(&owner, &changed, None), &ctl()), Err(BackingError::Length)));
    let wrong = [PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Color4F32(words) }];
    assert!(owner.try_with_readonly_point_filter(patch(&owner, &wrong, None), &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
fn color_patch_late_cancel_and_quota_failure_keep_original_words_and_refund() {
    let owner = current(true);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let args = [PointArgumentPatch { name: "raw-9", value: PointArgumentValue::Color4F32([0; 4]) }];
    let cancelled = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(
        owner.try_with_readonly_point_filter(patch(&owner, &args, None), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), before);
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(owner.try_with_readonly_point_filter(patch(&owner, &args, None), &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let FilterValue::Color { components, .. } = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[9].value else { panic!() };
    assert_eq!(components.first(), Some(&ExactNumber::Float32Bits(0x80000000)));
}
#[test]
fn work_cancel_after_fees_and_quota_failures_leave_current_owner_and_refund() {
    let owner = current(true);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let args = [PointArgumentPatch { name: "raw-1", value: PointArgumentValue::Bool(true) }];
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(owner.try_with_readonly_point_filter(patch(&owner, &args, None), &deny), Err(BackingError::Work)));
    let cancelled = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(owner.try_with_readonly_point_filter(patch(&owner, &args, None), &c), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    let block = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(owner.try_with_readonly_point_filter(patch(&owner, &args, None), &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
}
#[test]
fn alternating_patch_title_and_properties_keep_one_flat_original_root() {
    let original = current(true);
    let mut owner = original.clone();
    let mut previous = None;
    for i in 0..20 {
        owner = owner.try_with_readonly_point_filter(patch(&owner, &[], Some(i % 2 == 0)), &ctl()).unwrap();
        owner = owner.try_with_document_name(if i % 2 == 0 { "title A" } else { "title B" }, &ctl()).unwrap();
        owner = owner
            .try_with_readonly_layer_properties(
                ReadonlyLayerPropertiesPatch {
                    layer: owner.document().layers[0].id,
                    name: None,
                    visible: Some(i % 2 == 0),
                    opacity_f32_bits: Some((-0f32).to_bits()),
                },
                &ctl(),
            )
            .unwrap();
        let root = owner.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
        assert!(root.original.shares_model(&original));
        let usage = owner.quota().usage().unwrap();
        if let Some(before) = previous {
            assert_eq!(usage, before)
        }
        previous = Some(usage);
    }
    assert!(owner.try_edit(AccountedEdit::LayerName { layer: owner.document().layers[0].id, name: "generic" }, &ctl()).is_err());
}
