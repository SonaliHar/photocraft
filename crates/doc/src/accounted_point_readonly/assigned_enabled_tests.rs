//! UNRUN structural activation drafts, not Box/Median source or renderer proof.
//! Reused fixture owns actual prepaid codec/model/native/mask/profile storage.
use super::*;
use photocraft_raster::Interrupt;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owner(revision: u64) -> AccountedDocument {
    super::super::tests::owner_at_revision(revision)
}
fn patch(owner: &AccountedDocument, enabled: bool) -> AssignedPointEnabledPatch {
    let state = owner.document().xcf_state.as_ref().unwrap();
    AssignedPointEnabledPatch {
        layer: owner.document().layers[0].id,
        filter: state.filter_stacks[0].nodes[0].id,
        expected_stack_revision: state.filter_stacks[0].revision,
        enabled,
    }
}
fn same_payloads(a: &AccountedDocument, b: &AccountedDocument) {
    assert!(b.is_readonly_assigned_point_storage());
    assert!(!b.is_readonly_point_storage());
    assert!(Arc::ptr_eq(a.document().icc_profile.as_ref().unwrap(), b.document().icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        a.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        b.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
    assert_eq!(a.document().layers[0].content, b.document().layers[0].content);
    let sa = a.document().xcf_state.as_ref().unwrap();
    let sb = b.document().xcf_state.as_ref().unwrap();
    assert_eq!(sa.items, sb.items);
    assert_eq!(sa.provenance, sb.provenance);
    assert_eq!(sa.opaque, sb.opaque);
    let ma = &sa.filter_masks[0];
    let mb = &sb.filter_masks[0];
    assert_eq!((ma.id, ma.owner, ma.bounds), (mb.id, mb.owner, mb.bounds));
    assert_eq!(ma.surface.default_bytes(), mb.surface.default_bytes());
    for ((ca, ta), (cb, tb)) in ma.surface.tiles().zip(mb.surface.tiles()) {
        assert_eq!(ca, cb);
        assert!(Arc::ptr_eq(ta, tb));
    }
}
#[test]
#[ignore = "UNRUN activation-only data copy and raw custody"]
fn changed_activation_only_updates_one_bool_and_one_stack_revision() {
    let original = owner(17);
    let changed = original.try_with_readonly_assigned_point_enabled(patch(&original, true), &ctl()).unwrap();
    assert!(!changed.shares_model(&original));
    same_payloads(&original, &changed);
    let a = &original.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    let b = &changed.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    assert_eq!(a.revision, 17);
    assert_eq!(b.revision, 18);
    assert!(!a.nodes[0].enabled);
    assert!(b.nodes[0].enabled);
    // External expected-value clone is covered by the fixture's original8MiB
    // Index prehold; it is test harness comparison, not production copy proof.
    let mut expected = a.nodes[0].clone();
    expected.enabled = true;
    assert_eq!(expected, b.nodes[0]);
    assert_eq!(original.document().name, changed.document().name);
    changed.validate_readonly_assigned_point_storage(&ctl()).unwrap();
}
#[test]
#[ignore = "UNRUN raw no-op, real full Index quota and revision boundary"]
fn same_bool_shares_at_full_quota_and_max_revision_but_change_cannot_overflow() {
    let original = owner(u64::MAX);
    let q = original.quota().clone();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    let same = original.try_with_readonly_assigned_point_enabled(patch(&original, false), &ctl()).unwrap();
    assert!(same.shares_model(&original));
    assert_eq!(q.usage().unwrap(), full);
    assert!(matches!(original.try_with_readonly_assigned_point_enabled(patch(&original, true), &ctl()), Err(BackingError::Length)));
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    // Engine document-revisionMAX pre-dispatch refusal remains a separate rule.
}
#[test]
#[ignore = "UNRUN class, singleton, stale IDs and malformed-current checks before noop"]
fn full_current_and_exact_singleton_ids_precede_noop_without_generic_grants() {
    let original = owner(4);
    let q = original.quota().clone();
    let before = q.usage().unwrap();
    for case in 0..4 {
        let mut request = patch(&original, false);
        match case {
            0 => request.layer = LayerId(0),
            1 => request.filter = FilterId(0),
            2 => request.expected_stack_revision = 3,
            _ => request.filter = FilterId(999),
        }
        assert!(original.try_with_readonly_assigned_point_enabled(request, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    for count in [0, 2] {
        let mut malformed = owner(4);
        let request = patch(&malformed, false);
        // Test-only structural setup within the genuine fixture prehold; no
        // mutable model or publication bypass is exposed by production APIs.
        let doc = &mut Arc::get_mut(&mut malformed.owner).unwrap().document;
        let nodes = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
        if count == 0 {
            nodes.clear();
            let state = doc.xcf_state.as_mut().unwrap();
            state.filter_masks.clear();
            state.items.retain(|item| matches!(item.item, ItemRef::Layer(_)));
        } else {
            let mut second = nodes[0].clone();
            second.id = FilterId(200);
            second.mask = None;
            nodes.push(second);
        }
        malformed.validate_readonly_assigned_point_storage(&ctl()).unwrap();
        assert!(malformed.try_with_readonly_assigned_point_enabled(request, &ctl()).is_err());
    }
    let mut malformed = owner(4);
    let request = patch(&malformed, false);
    Arc::get_mut(&mut malformed.owner).unwrap().document.icc_profile = None;
    assert!(malformed.try_with_readonly_assigned_point_enabled(request, &ctl()).is_err());
    let implicit = super::super::super::tests::owned(crate::SampleType::F32, crate::SampleEncoding::NonlinearProfile, 650);
    assert!(implicit.try_with_readonly_assigned_point_enabled(patch(&implicit, false), &ctl()).is_err());
    assert!(
        original
            .try_with_readonly_point_filter(
                PointFilterPatch {
                    layer: original.document().layers[0].id,
                    filter: FilterId(100),
                    expected_stack_revision: 4,
                    arguments: &[],
                    enabled: Some(false)
                },
                &ctl()
            )
            .is_err()
    );
    let noop = AccountedEdit::LayerVisibility { layer: original.document().layers[0].id, visible: true };
    assert!(original.try_edit(noop, &ctl()).is_err());
    let mut history = AccountedHistory::new(original.clone(), 4, &ctl()).unwrap();
    assert!(history.try_commit(noop, &ctl()).is_err());
    assert_eq!(history.len(), 1);
}
#[test]
#[ignore = "UNRUN quota, work and post-fee cancellation rollback"]
fn failed_private_activation_preserves_ledger_model_and_raw_revision() {
    let original = owner(9);
    let q = original.quota().clone();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    assert!(matches!(original.try_with_readonly_assigned_point_enabled(patch(&original, true), &ctl()), Err(BackingError::Quota)));
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    let failed = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(original.try_with_readonly_assigned_point_enabled(patch(&original, true), &failed), Err(BackingError::Work)));
    let cancel = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(original.try_with_readonly_assigned_point_enabled(patch(&original, true), &c), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    let state = original.document().xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_stacks[0].revision, 9);
    assert!(!state.filter_stacks[0].nodes[0].enabled);
}
#[test]
#[ignore = "UNRUN repeated activation/title flat source lifetime"]
fn repeated_activation_and_title_keep_original_receipts_without_copy_chains() {
    let original = owner(1);
    let q = original.quota().clone();
    let original_model = Arc::downgrade(&original.owner);
    let codec = Arc::downgrade(&original.owner.retention);
    let first = original.try_with_readonly_assigned_point_enabled(patch(&original, true), &ctl()).unwrap();
    let first_model = Arc::downgrade(&first.owner);
    let titled = first.try_with_document_name("renamed", &ctl()).unwrap();
    let title_model = Arc::downgrade(&titled.owner);
    let last = titled.try_with_readonly_assigned_point_enabled(patch(&titled, false), &ctl()).unwrap();
    same_payloads(&original, &last);
    assert_eq!(last.document().name, "renamed");
    assert_eq!(last.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 3);
    drop(first);
    drop(titled);
    assert!(first_model.upgrade().is_none());
    assert!(title_model.upgrade().is_none());
    drop(original);
    assert!(original_model.upgrade().is_some());
    assert!(codec.upgrade().is_some());
    drop(last);
    assert!(original_model.upgrade().is_none());
    assert!(codec.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), Default::default());
}
