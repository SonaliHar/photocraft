//! UNRUN structural scalar-copy drafts; no operation/CMS/source qualification.
use super::*;
use crate::item_identity::FilterId;
use photocraft_raster::Interrupt;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owner(revision: u64) -> AccountedDocument {
    let mut owner = super::tests::owner_at_revision(revision);
    let node = &mut Arc::get_mut(&mut owner.owner).unwrap().document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    // Test-only setup remains inside the fixture's actual retained preholds.
    node.arguments[0].value = FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 });
    owner
}
fn request<'a>(owner: &AccountedDocument, arguments: &'a [PointArgumentPatch<'a>], enabled: Option<bool>) -> PointFilterPatch<'a> {
    let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    PointFilterPatch { layer: owner.document().layers[0].id, filter: stack.nodes[0].id, expected_stack_revision: stack.revision, arguments, enabled }
}
fn scalar(value: i32) -> PointArgumentPatch<'static> {
    PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Signed32(value) }
}
#[test]
#[ignore = "UNRUN same-type scalar and activation raw custody"]
fn scalar_and_activation_copy_only_two_words_and_one_revision() {
    let original = owner(7);
    let changed = original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(1000)], Some(true)), &ctl()).unwrap();
    assert!(!changed.shares_model(&original));
    assert!(changed.is_readonly_assigned_point_storage());
    let a = original.document();
    let b = changed.document();
    assert!(Arc::ptr_eq(a.icc_profile.as_ref().unwrap(), b.icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        a.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        b.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
    assert_eq!(a.layers, b.layers);
    let sa = a.xcf_state.as_ref().unwrap();
    let sb = b.xcf_state.as_ref().unwrap();
    assert_eq!(sa.items, sb.items);
    assert_eq!(sa.provenance, sb.provenance);
    assert_eq!(sa.opaque, sb.opaque);
    for (a, b) in sa.filter_masks.iter().zip(&sb.filter_masks) {
        assert_eq!((a.id, a.owner, a.bounds), (b.id, b.owner, b.bounds));
        assert_eq!(a.surface.default_bytes(), b.surface.default_bytes());
        assert_eq!(a.surface.tiles().count(), b.surface.tiles().count());
        for ((ca, ta), (cb, tb)) in a.surface.tiles().zip(b.surface.tiles()) {
            assert_eq!(ca, cb);
            assert!(Arc::ptr_eq(ta, tb));
        }
    }
    let mut expected = sa.filter_stacks[0].nodes[0].clone();
    expected.arguments[0].value = FilterValue::Number(ExactNumber::Signed { width: 32, value: 1000 });
    expected.enabled = true;
    assert_eq!(expected, sb.filter_stacks[0].nodes[0]);
    assert_eq!((sa.filter_stacks[0].revision, sb.filter_stacks[0].revision), (7, 8));
    changed.validate_readonly_assigned_point_storage(&ctl()).unwrap();
}
#[test]
#[ignore = "UNRUN structural raw integer domain distinct from engine semantics"]
fn signed_extremes_are_structural_words_without_operation_grant() {
    let original = owner(1);
    for value in [i32::MIN, -1, 0, 1001, i32::MAX] {
        let changed = original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(value)], None), &ctl()).unwrap();
        assert_eq!(
            changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value,
            FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(value) })
        );
    }
    // An implicit-class API is still closed even for a raw no-op.
    assert!(original.try_with_readonly_point_filter(request(&original, &[scalar(4)], Some(false)), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN exact noop and checked revision boundary under actual full quota"]
fn noop_shares_at_full_quota_and_max_revision_change_refuses() {
    let original = owner(u64::MAX);
    let q = original.quota().clone();
    let before = q.usage().unwrap();
    let _block = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    let same = original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(4)], Some(false)), &ctl()).unwrap();
    assert!(same.shares_model(&original));
    assert!(matches!(original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(5)], None), &ctl()), Err(BackingError::Length)));
    assert_eq!(q.usage().unwrap(), full);
}
#[test]
#[ignore = "UNRUN target, scalar name/type and singleton checks before noop"]
fn malformed_requests_and_current_cannot_bypass_noop_checks() {
    let original = owner(3);
    for case in 0..5 {
        let words = [scalar(4)];
        let mut patch = request(&original, &words, Some(false));
        match case {
            0 => patch.layer = LayerId(0),
            1 => patch.filter = FilterId(0),
            2 => patch.filter = FilterId(999),
            3 => patch.expected_stack_revision = 2,
            _ => patch.layer = LayerId(999),
        }
        assert!(original.try_with_readonly_assigned_point_filter(patch, &ctl()).is_err());
    }
    for words in [
        vec![scalar(4), scalar(4)],
        vec![PointArgumentPatch { name: "missing", value: PointArgumentValue::Signed32(4) }],
        vec![PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Float32Bits(4) }],
        vec![PointArgumentPatch { name: "raw-1", value: PointArgumentValue::Signed32(4) }],
        vec![PointArgumentPatch { name: "", value: PointArgumentValue::Signed32(4) }],
    ] {
        assert!(original.try_with_readonly_assigned_point_filter(request(&original, &words, Some(false)), &ctl()).is_err());
    }
    let mut malformed = owner(3);
    Arc::get_mut(&mut malformed.owner).unwrap().document.icc_profile = None;
    assert!(malformed.try_with_readonly_assigned_point_filter(request(&malformed, &[], Some(false)), &ctl()).is_err());
    let mut multiple = owner(3);
    let doc = &mut Arc::get_mut(&mut multiple.owner).unwrap().document;
    let nodes = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
    let mut extra = nodes[0].clone();
    extra.id = FilterId(200);
    extra.mask = None;
    nodes.push(extra);
    multiple.validate_readonly_assigned_point_storage(&ctl()).unwrap();
    assert!(multiple.try_with_readonly_assigned_point_filter(request(&multiple, &[], Some(false)), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN quota, work and cancellation rollback"]
fn failed_private_copy_preserves_ledger_and_original() {
    let original = owner(9);
    let q = original.quota().clone();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    assert!(matches!(original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(5)], Some(true)), &ctl()), Err(BackingError::Quota)));
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(5)], None), &work), Err(BackingError::Work)));
    let cancel = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&cancel);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(5)], None), &control), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(original.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 9);
}
#[test]
#[ignore = "UNRUN repeated scalar/title/activation copies retain only flat original"]
fn repeated_edits_keep_original_without_retaining_intermediate_copies() {
    let original = owner(1);
    let q = original.quota().clone();
    let source = Arc::downgrade(&original.owner.retention);
    let root = Arc::downgrade(&original.owner);
    let first = original.try_with_readonly_assigned_point_filter(request(&original, &[scalar(5)], Some(true)), &ctl()).unwrap();
    let intermediate = Arc::downgrade(&first.owner);
    let title = first.try_with_document_name("renamed", &ctl()).unwrap();
    let title_model = Arc::downgrade(&title.owner);
    let last = title.try_with_readonly_assigned_point_filter(request(&title, &[scalar(6)], Some(false)), &ctl()).unwrap();
    drop(first);
    drop(title);
    assert!(intermediate.upgrade().is_none());
    assert!(title_model.upgrade().is_none());
    drop(original);
    assert!(root.upgrade().is_some());
    assert!(source.upgrade().is_some());
    assert_eq!(last.document().name, "renamed");
    assert_eq!(last.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 3);
    drop(last);
    assert!(root.upgrade().is_none());
    assert!(source.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), Default::default());
}
