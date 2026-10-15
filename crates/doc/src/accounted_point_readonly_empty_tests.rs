//! UNRUN empty CURRENT structural custody; these synthetic recipes confer no
//! source or evaluator qualification. Expected model clones are external oracles.
use super::*;
use crate::xcf_precision::FilterOutputPrecision;

fn singleton() -> AccountedDocument {
    let original = owner();
    original.try_remove_readonly_point_filter(patch(&original, FilterId(200)), &ctl()).unwrap()
}
fn empty() -> AccountedDocument {
    let one = singleton();
    one.try_remove_readonly_point_filter(patch(&one, FilterId(100)), &ctl()).unwrap()
}

#[test]
fn last_remove_keeps_stack_identity_revision_and_every_raw_archive_payload() {
    let one = singleton();
    let result = one.try_remove_readonly_point_filter(patch(&one, FilterId(100)), &ctl()).unwrap();
    let mut expected = one.document().clone();
    let state = expected.xcf_state.as_mut().unwrap();
    let stack = &mut state.filter_stacks[0];
    stack.nodes.clear();
    stack.revision += 1;
    state.filter_masks.clear();
    state.items.retain(|item| !matches!(item.item, ItemRef::Mask(_)));
    for record in &mut state.opaque {
        if record.owner == Some(ItemRef::Mask(MaskId(101))) {
            record.owner = None;
            record.identity = format!("deleted-filter-mask/101/{}", record.identity);
        }
    }
    state.current_filter_output_precision = FilterOutputPrecision::Canonical;
    assert_eq!(result.document(), &expected);
    result.validate_readonly_point_storage(&ctl()).unwrap();
    let before = one.document().xcf_state.as_ref().unwrap();
    let after = result.document().xcf_state.as_ref().unwrap();
    assert_eq!(after.filter_stacks.len(), 1);
    assert_eq!(after.filter_stacks[0].owner, before.filter_stacks[0].owner);
    assert_eq!(after.filter_stacks[0].revision, 9);
    assert_eq!(after.items.len(), 1);
    assert!(after.opaque.iter().all(|record| record.owner.is_none()));
    for (a, b) in before.opaque.iter().zip(&after.opaque) {
        assert!(Arc::ptr_eq(&a.bytes, &b.bytes));
    }
    assert_eq!(one.document().layers, result.document().layers);
    assert!(Arc::ptr_eq(one.document().icc_profile.as_ref().unwrap(), result.document().icc_profile.as_ref().unwrap()));
    assert_eq!(one.document().source_color_context, result.document().source_color_context);
    assert!(result.quota().shares_ledger(one.quota()));
}

#[test]
fn empty_title_copies_keep_flat_original_custody_and_refund_all_derivative_fees() {
    let value = empty();
    let q = value.quota().clone();
    let baseline = q.usage().unwrap();
    let first = value.try_with_document_name("empty-a", &ctl()).unwrap();
    let second = first.try_with_document_name("empty-b", &ctl()).unwrap();
    let source = value.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
    let retained = second.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
    assert!(source.original.shares_model(&retained.original));
    assert_eq!(second.document().xcf_state, value.document().xcf_state);
    assert_eq!(second.document().layers, value.document().layers);
    assert!(second.is_readonly_point_storage());
    assert!(second.try_edit(AccountedEdit::LayerName { layer: second.document().layers[0].id, name: "generic" }, &ctl()).is_err());
    drop(first);
    drop(second);
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
fn empty_stack_requires_one_real_stack_no_masks_and_exact_layer_item() {
    for case in 0..4 {
        let (mut doc, guard, q) = raw();
        let state = doc.xcf_state.as_mut().unwrap();
        state.filter_stacks[0].nodes.clear();
        state.filter_masks.clear();
        state.items.retain(|item| !matches!(item.item, ItemRef::Mask(_)));
        for record in &mut state.opaque {
            record.owner = None;
        }
        match case {
            0 => state.filter_stacks.clear(),
            1 => state.filter_stacks.push(state.filter_stacks[0].clone()),
            2 => state.filter_stacks[0].owner = ItemRef::Layer(LayerId(999)),
            _ => state.items.push(state.items[0].clone()),
        }
        assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}

#[test]
fn last_remove_failures_and_empty_targets_preserve_source_and_actual_quota() {
    let one = singleton();
    let q = one.quota();
    let baseline = q.usage().unwrap();
    let mut stale = patch(&one, FilterId(100));
    stale.expected_stack_revision -= 1;
    assert!(one.try_remove_readonly_point_filter(stale, &ctl()).is_err());
    assert!(one.try_remove_readonly_point_filter(patch(&one, FilterId(999)), &ctl()).is_err());
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(one.try_remove_readonly_point_filter(patch(&one, FilterId(100)), &deny), Err(BackingError::Work)));
    let cancelled = || q.usage().unwrap().index > baseline.index;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(
        one.try_remove_readonly_point_filter(patch(&one, FilterId(100)), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackingError::Cancelled)
    ));
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(one.try_remove_readonly_point_filter(patch(&one, FilterId(100)), &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(one.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 1);
    let zero = empty();
    let request = patch(&zero, FilterId(100));
    let before = zero.quota().usage().unwrap();
    assert!(zero.try_remove_readonly_point_filter(request, &ctl()).is_err());
    assert!(
        zero.try_with_readonly_point_filter(
            PointFilterPatch {
                layer: request.layer,
                filter: request.filter,
                expected_stack_revision: request.expected_stack_revision,
                arguments: &[],
                enabled: Some(false)
            },
            &ctl()
        )
        .is_err()
    );
    assert!(
        zero.try_with_readonly_point_structured_filter(
            PointStructuredFilterPatch {
                layer: request.layer,
                filter: request.filter,
                expected_stack_revision: request.expected_stack_revision,
                arguments: &[],
                enabled: Some(false)
            },
            &ctl()
        )
        .is_err()
    );
    assert!(
        zero.try_move_readonly_point_filter(
            PointFilterMovePatch { layer: request.layer, filter: request.filter, expected_stack_revision: request.expected_stack_revision, index: 0 },
            &ctl()
        )
        .is_err()
    );
    assert_eq!(zero.quota().usage().unwrap(), before);
}

#[test]
fn last_remove_reconciles_raw_half_policy_without_changing_source_precision() {
    let (mut doc, guard, q) = raw();
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_stacks[0].nodes.remove(1);
    state.filter_masks.retain(|mask| mask.owner == FilterId(100));
    state.items.retain(|item| item.item != ItemRef::Mask(MaskId(201)));
    state.opaque.retain(|record| record.owner != Some(ItemRef::Mask(MaskId(201))));
    // Structural CURRENT metadata only: this deliberately does not assert the
    // synthetic recipe is qualified for Half evaluation or original import.
    state.current_filter_output_precision = FilterOutputPrecision::Half;
    let one = adopt(doc, guard, q);
    let zero = one.try_remove_readonly_point_filter(patch(&one, FilterId(100)), &ctl()).unwrap();
    assert_eq!(zero.document().xcf_state.as_ref().unwrap().current_filter_output_precision, FilterOutputPrecision::Canonical);
    assert_eq!(zero.document().source_color_context, one.document().source_color_context);
    assert_eq!(zero.document().layers, one.document().layers);
}
