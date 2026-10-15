//! UNRUN activation-only storage regressions. Synthetic paid native fixtures
//! establish custody, not authored source/profile/kernel eligibility. Ordinary
//! expected Document clones are external test-oracle buffers.
use super::super::tests::{ctl, fixture, owned, quota};
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn request(owner: &AccountedDocument, enabled: bool) -> GaussianGroupEnabledPatch {
    let state = owner.document().xcf_state.as_ref().unwrap();
    let stack = &state.filter_stacks[0];
    GaussianGroupEnabledPatch { layer: owner.document().layers[1].id, filter: stack.nodes[0].id, expected_stack_revision: stack.revision, enabled }
}
#[test]
fn toggles_only_enabled_and_revision_preserving_exact_raw_tree_and_flat_title_retention() {
    for edited in [false, true] {
        let q = quota();
        let before = q.usage().unwrap();
        let source = owned(&q, edited, true);
        let patch = request(&source, false);
        source.validate_gaussian_group_enabled(patch, &ctl()).unwrap();
        let disabled = source.try_with_gaussian_group_enabled(patch, &ctl()).unwrap();
        let mut expected = source.document().clone();
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.nodes[0].enabled = false;
        stack.revision += 1;
        assert_eq!(disabled.document(), &expected);
        let titled = disabled.try_with_document_name("continued.pcraft", &ctl()).unwrap();
        let enabled = titled.try_with_gaussian_group_enabled(request(&titled, true), &ctl()).unwrap();
        expected.name = "continued.pcraft".into();
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.nodes[0].enabled = true;
        stack.revision += 1;
        assert_eq!(enabled.document(), &expected);
        enabled.validate_gaussian_group_storage(&ctl()).unwrap();
        assert!(Arc::ptr_eq(&source.document().layers[1].mask.as_ref().unwrap().surface, &enabled.document().layers[1].mask.as_ref().unwrap().surface));
        assert!(Arc::ptr_eq(source.document().icc_profile.as_ref().unwrap(), enabled.document().icc_profile.as_ref().unwrap()));
        let original_mask = &source.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        let current_mask = &enabled.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        assert_eq!(original_mask.default_bytes(), current_mask.default_bytes());
        for ((a, ta), (b, tb)) in original_mask.tiles().zip(current_mask.tiles()) {
            assert_eq!(a, b);
            assert!(Arc::ptr_eq(ta, tb));
        }
        assert!(enabled.owner.retention.downcast_ref::<OriginalRetention>().unwrap().original.shares_model(&source));
        drop(expected); // release external native/store Arc oracle first
        drop(source);
        drop(disabled);
        drop(titled);
        enabled.validate_gaussian_group_storage(&ctl()).unwrap();
        drop(enabled);
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
fn stable_ids_revision_and_actual_runtime_class_are_required_even_for_noop() {
    let q = quota();
    let source = owned(&q, false, true);
    let valid = request(&source, true);
    let before = q.usage().unwrap();
    for fault in 0..4 {
        let mut patch = valid;
        match fault {
            0 => patch.layer = source.document().layers[0].id,
            1 => patch.filter = FilterId(valid.filter.0 + 1),
            2 => patch.expected_stack_revision += 1,
            _ => patch.layer = LayerId(0),
        }
        assert!(source.validate_gaussian_group_enabled(patch, &ctl()).is_err());
        assert!(source.try_with_gaussian_group_enabled(patch, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let (doc, guard) = fixture(&q, false, true);
    let legacy = AccountedDocument::from_owned(doc, guard, q.clone(), &ctl()).unwrap();
    assert!(!legacy.is_readonly_gaussian_group_storage());
    assert!(legacy.try_with_gaussian_group_enabled(request(&legacy, true), &ctl()).is_err());
    // The opaque original tuple does not acquire a generic edit escape hatch.
    assert!(source.try_edit(AccountedEdit::LayerVisibility { layer: valid.layer, visible: false }, &ctl()).is_err());
    assert!(source.try_replace_selection(None, &ctl()).is_err());
}
#[test]
fn max_revision_noop_shares_at_full_quota_but_changed_overflow_never_allocates() {
    let q = quota();
    let (mut doc, guard) = fixture(&q, false, true);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].revision = u64::MAX;
    let source = AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q.clone(), &ctl()).unwrap();
    let used = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (128 << 20) - used.resident).unwrap();
    let before = q.usage().unwrap();
    assert!(source.try_with_gaussian_group_enabled(request(&source, true), &ctl()).unwrap().shares_model(&source));
    assert!(matches!(source.try_with_gaussian_group_enabled(request(&source, false), &ctl()), Err(BackingError::Length)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(source.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, u64::MAX);
    drop(blocker);
}
#[test]
fn alternate_disabled_tuple_is_retained_without_becoming_an_enable_alias() {
    let q = quota();
    let disabled = owned(&q, true, false);
    let before = q.usage().unwrap();
    assert!(disabled.try_with_gaussian_group_enabled(request(&disabled, false), &ctl()).unwrap().shares_model(&disabled));
    assert!(matches!(disabled.validate_gaussian_group_enabled(request(&disabled, true), &ctl()), Err(BackingError::Unsupported)));
    assert!(matches!(disabled.try_with_gaussian_group_enabled(request(&disabled, true), &ctl()), Err(BackingError::Unsupported)));
    assert_eq!(q.usage().unwrap(), before);
    let policy = disabled.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].composition.as_ref().unwrap();
    assert_eq!([policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator], [-1474592175; 3]);
    let titled = disabled.try_with_document_name("disabled.pcraft", &ctl()).unwrap();
    assert!(titled.try_with_gaussian_group_enabled(request(&titled, true), &ctl()).is_err());
    assert_eq!(titled.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 17);
}
#[test]
fn work_quota_and_private_copy_cancellation_leave_original_recipe_and_charges_atomic() {
    let q = quota();
    let source = owned(&q, false, true);
    let before = q.usage().unwrap();
    let patch = request(&source, false);
    let denied = |_: u64| Err(BackingError::Quota);
    assert!(source.try_with_gaussian_group_enabled(patch, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &denied }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let cancel_private = |_: u64| if q.usage().unwrap().index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(source.try_with_gaussian_group_enabled(patch, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &cancel_private }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let work = AtomicU64::new(0);
    let count = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let completed = source.try_with_gaussian_group_enabled(patch, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &count }).unwrap();
    let total = work.load(Ordering::Relaxed);
    assert!(total > 1);
    drop(completed);
    assert_eq!(q.usage().unwrap(), before);
    let remaining = AtomicU64::new(total - 1);
    let bounded = |n| remaining.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Quota);
    assert!(source.try_with_gaussian_group_enabled(patch, &ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &bounded }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert!(source.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    assert_eq!(source.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, patch.expected_stack_revision);
}
