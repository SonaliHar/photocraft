//! UNRUN exact CURRENT storage/transaction drafts. Synthetic paid fixtures and
//! external expected clones are not numerical/GIMP/source-eligibility oracles.
use super::super::tests::{ctl, fixture, owned, quota};
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
fn request(owner: &AccountedDocument) -> GaussianGroupPatch {
    let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    GaussianGroupPatch {
        layer: owner.document().layers[1].id,
        filter: stack.nodes[0].id,
        expected_stack_revision: stack.revision,
        std_dev_x_f32_bits: None,
        std_dev_y_f32_bits: None,
        enabled: None,
    }
}
fn axis(owner: &AccountedDocument, name: &str) -> u32 {
    let value = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|a| a.name == name).unwrap().value;
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = value else { panic!("axis type") };
    *bits
}
fn expected_update(doc: &mut Document, p: GaussianGroupPatch) {
    let stack = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0];
    for arg in &mut stack.nodes[0].arguments {
        let next = match arg.name.as_str() {
            "std-dev-x" => p.std_dev_x_f32_bits,
            "std-dev-y" => p.std_dev_y_f32_bits,
            _ => None,
        };
        if let Some(bits) = next {
            arg.value = FilterValue::Number(ExactNumber::Float32Bits(bits));
        }
    }
    if let Some(enabled) = p.enabled {
        stack.nodes[0].enabled = enabled;
    }
    stack.revision += 1;
}
#[test]
fn independent_axis_boundaries_and_raw_signed_zero_do_not_invent_pair_or_disabled_aliases() {
    let cases = [
        (0f32.to_bits(), 15f32.to_bits()),
        (15f32.to_bits(), 0f32.to_bits()),
        ((-0f32).to_bits(), (-0f32).to_bits()),
        ((1f32.to_bits() - 1), 15f32.to_bits()),
        (15f32.to_bits(), 1f32.to_bits() + 1),
        (1f32.to_bits(), 1f32.to_bits() + 1),
    ];
    for edited in [false, true] {
        for (x, y) in cases {
            let q = quota();
            let before = q.usage().unwrap();
            let source = owned(&q, edited, true);
            let mut patch = request(&source);
            patch.std_dev_x_f32_bits = Some(x);
            patch.std_dev_y_f32_bits = Some(y);
            source.validate_gaussian_group_patch(patch, &ctl()).unwrap();
            let changed = source.try_with_gaussian_group_patch(patch, &ctl()).unwrap();
            assert_eq!(axis(&changed, "std-dev-x"), x);
            assert_eq!(axis(&changed, "std-dev-y"), y);
            assert!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
            assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 18);
            let mut expected = source.document().clone();
            expected_update(&mut expected, patch);
            assert_eq!(changed.document(), &expected);
            assert!(changed.try_with_gaussian_group_patch(request(&changed), &ctl()).unwrap().shares_model(&changed));
            drop(expected);
            drop(source);
            drop(changed);
            assert_eq!(q.usage().unwrap(), before);
        }
    }
}
#[test]
fn combined_axes_activation_one_revision_preserves_order_masks_native_profiles_and_flat_title_guard() {
    let q = quota();
    let before = q.usage().unwrap();
    let source = owned(&q, false, true);
    let mut patch = request(&source);
    patch.std_dev_x_f32_bits = Some(3.75f32.to_bits());
    patch.std_dev_y_f32_bits = Some(0f32.to_bits());
    patch.enabled = Some(false);
    let changed = source.try_with_gaussian_group_patch(patch, &ctl()).unwrap();
    let mut expected = source.document().clone();
    expected_update(&mut expected, patch);
    assert_eq!(changed.document(), &expected);
    let titled = changed.try_with_document_name("parameters.pcraft", &ctl()).unwrap();
    let p = request(&titled);
    let enabled = titled
        .try_with_gaussian_group_enabled(
            GaussianGroupEnabledPatch { layer: p.layer, filter: p.filter, expected_stack_revision: p.expected_stack_revision, enabled: true },
            &ctl(),
        )
        .unwrap();
    expected.name = "parameters.pcraft".into();
    expected.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = true;
    expected.xcf_state.as_mut().unwrap().filter_stacks[0].revision += 1;
    assert_eq!(enabled.document(), &expected);
    assert!(Arc::ptr_eq(&source.document().layers[1].mask.as_ref().unwrap().surface, &enabled.document().layers[1].mask.as_ref().unwrap().surface));
    assert!(Arc::ptr_eq(source.document().icc_profile.as_ref().unwrap(), enabled.document().icc_profile.as_ref().unwrap()));
    let a = &source.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let b = &enabled.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    for ((ca, ta), (cb, tb)) in a.tiles().zip(b.tiles()) {
        assert_eq!(ca, cb);
        assert!(Arc::ptr_eq(ta, tb));
    }
    assert!(enabled.owner.retention.downcast_ref::<OriginalRetention>().unwrap().original.shares_model(&source));
    drop(expected);
    drop(source);
    drop(changed);
    drop(titled);
    drop(enabled);
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn raw_bit_noop_at_full_quota_max_revision_but_changed_signed_zero_overflows_before_copy() {
    let q = quota();
    let (mut doc, guard) = fixture(&q, false, true);
    let stack = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0];
    stack.revision = u64::MAX;
    stack.nodes[0].arguments.iter_mut().find(|a| a.name == "std-dev-x").unwrap().value = FilterValue::Number(ExactNumber::Float32Bits((-0f32).to_bits()));
    let source = AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q.clone(), &ctl()).unwrap();
    let used = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (128 << 20) - used.resident).unwrap();
    let before = q.usage().unwrap();
    let mut patch = request(&source);
    patch.std_dev_x_f32_bits = Some((-0f32).to_bits());
    patch.enabled = Some(true);
    assert!(source.try_with_gaussian_group_patch(patch, &ctl()).unwrap().shares_model(&source));
    patch.std_dev_x_f32_bits = Some(0f32.to_bits());
    assert!(matches!(source.try_with_gaussian_group_patch(patch, &ctl()), Err(BackingError::Length)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(axis(&source, "std-dev-x"), (-0f32).to_bits());
    drop(blocker);
}
#[test]
fn invalid_axes_identity_and_disabled_only_sentinel_refuse_without_partial_revision_or_normalization() {
    let q = quota();
    let source = owned(&q, false, true);
    let before = q.usage().unwrap();
    for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.25f32).to_bits(), 15f32.to_bits() + 1] {
        let mut patch = request(&source);
        patch.std_dev_x_f32_bits = Some(bits);
        patch.std_dev_y_f32_bits = Some(2f32.to_bits());
        patch.enabled = Some(false);
        assert!(source.try_with_gaussian_group_patch(patch, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    for fault in 0..3 {
        let mut patch = request(&source);
        match fault {
            0 => patch.filter = FilterId(0),
            1 => patch.layer = source.document().layers[0].id,
            _ => patch.expected_stack_revision += 1,
        };
        assert!(source.try_with_gaussian_group_patch(patch, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let alternate = owned(&q, true, false);
    let mut patch = request(&alternate);
    patch.std_dev_x_f32_bits = Some(15f32.to_bits());
    let changed = alternate.try_with_gaussian_group_patch(patch, &ctl()).unwrap();
    assert!(!changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    let policy = changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].composition.as_ref().unwrap();
    assert_eq!([policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator], [-1474592175; 3]);
    let mut enable = request(&changed);
    enable.std_dev_y_f32_bits = Some(0f32.to_bits());
    enable.enabled = Some(true);
    assert!(matches!(changed.try_with_gaussian_group_patch(enable, &ctl()), Err(BackingError::Unsupported)));
    assert_eq!(axis(&changed, "std-dev-y"), 1.3f32.to_bits());
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 18);
}
#[test]
fn cumulative_work_quota_and_private_copy_cancellation_are_atomic_for_both_axes_and_activation() {
    let q = quota();
    let source = owned(&q, true, true);
    let before = q.usage().unwrap();
    let mut patch = request(&source);
    patch.std_dev_x_f32_bits = Some(15f32.to_bits());
    patch.std_dev_y_f32_bits = Some((-0f32).to_bits());
    patch.enabled = Some(false);
    let work = AtomicU64::new(0);
    let count = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = source.try_with_gaussian_group_patch(patch, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &count }).unwrap();
    let total = work.load(Ordering::Relaxed);
    assert!(total > 1);
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    let remaining = AtomicU64::new(total - 1);
    let bounded = |n| remaining.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Quota);
    assert!(source.try_with_gaussian_group_patch(patch, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &bounded }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let cancel = |_: u64| if q.usage().unwrap().index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(source.try_with_gaussian_group_patch(patch, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &cancel }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(axis(&source, "std-dev-x"), 0.8f32.to_bits());
    assert_eq!(axis(&source, "std-dev-y"), 1.3f32.to_bits());
    assert!(source.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    assert_eq!(source.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 17);
}
