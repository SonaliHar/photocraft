//! Ignored UNRUN structural movement only. The preheld virtual native fixture is
//! not a producer/source, evaluator, profile or numerical oracle.
use super::*;
fn movement(owner: &AccountedDocument, stack: usize, node: usize, index: usize) -> PointFilterMovePatch {
    let s = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[stack];
    let ItemRef::Layer(layer) = s.owner else { panic!() };
    PointFilterMovePatch { layer, filter: s.nodes[node].id, expected_stack_revision: s.revision, index }
}
#[test]
#[ignore = "UNRUN: forward/backward nonfirst-root movement equals remove/insert order, preserves complete node payload/mask/canonical/native/profile/source custody"]
fn both_directions_preserve_exact_payloads_and_other_roots() {
    for sample in [SampleType::F32, SampleType::F64] {
        for (from, to) in [(1, 4), (4, 0)] {
            let owner = owned_ordered(sample, &[2, 5]);
            let p = movement(&owner, 1, from, to);
            owner.validate_readonly_filtered_flat_point_filter_move(p, &ctl()).unwrap();
            let changed = owner.try_move_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
            let mut expected = owner.document().clone(); // external harness expectation only
            let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[1];
            let node = stack.nodes.remove(from);
            stack.nodes.insert(to, node);
            stack.revision += 1;
            assert_eq!(changed.document(), &expected);
            assert!(changed.is_readonly_filtered_flat_point_storage());
            assert!(!changed.shares_model(&owner));
            assert!(Arc::ptr_eq(&changed.owner.retention, &owner.owner.retention));
            assert!(Arc::ptr_eq(changed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
            for (old, new) in owner.document().xcf_state.as_ref().unwrap().filter_masks.iter().zip(&changed.document().xcf_state.as_ref().unwrap().filter_masks)
            {
                assert_eq!(old.id, new.id);
                assert_eq!(old.owner, new.owner);
                for ((a, x), (b, y)) in old.surface.tiles().zip(new.surface.tiles()) {
                    assert_eq!(a, b);
                    assert!(Arc::ptr_eq(x, y));
                }
            }
            // Stable IDs continue to select the moved node at its actual new index.
            let node = &changed.document().xcf_state.as_ref().unwrap().filter_stacks[1].nodes[to];
            let patch = PointFilterPatch {
                layer: p.layer,
                filter: p.filter,
                expected_stack_revision: p.expected_stack_revision + 1,
                arguments: &[],
                enabled: Some(node.enabled),
            };
            assert!(changed.try_with_readonly_filtered_flat_point_filter(patch, &ctl()).unwrap().shares_model(&changed));
            assert!(changed.try_with_imported_layer_translation(p.layer, 0, 0, &ctl()).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN: sameposition MAX/fullIndex shares only after complete prestate/target/revision proof; malformed target and requested destination refuse"]
fn noop_max_quota_and_identity_fences() {
    let q = quota();
    let (mut d, g) = ordered_fixture(&q, SampleType::F64, &[2, 4]);
    d.xcf_state.as_mut().unwrap().filter_stacks[1].revision = u64::MAX;
    let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap();
    let before = owner.quota().usage().unwrap();
    let blocker = owner.quota().reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    let p = movement(&owner, 1, 3, 3);
    assert!(owner.try_move_readonly_filtered_flat_point_filter(p, &ctl()).unwrap().shares_model(&owner));
    let mut bad = p;
    bad.index = 0;
    assert!(owner.try_move_readonly_filtered_flat_point_filter(bad, &ctl()).is_err());
    bad = p;
    bad.expected_stack_revision -= 1;
    assert!(owner.validate_readonly_filtered_flat_point_filter_move(bad, &ctl()).is_err());
    bad = p;
    bad.filter = owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
    assert!(owner.try_move_readonly_filtered_flat_point_filter(bad, &ctl()).is_err());
    bad = p;
    bad.index = 4;
    assert!(owner.try_move_readonly_filtered_flat_point_filter(bad, &ctl()).is_err());
    bad = p;
    bad.layer = LayerId(99999);
    assert!(owner.try_move_readonly_filtered_flat_point_filter(bad, &ctl()).is_err());
    let zero_work = |_| Err(BackingError::Work);
    assert!(owner.try_move_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &zero_work }).is_err());
    drop(blocker);
    assert_eq!(owner.quota().usage().unwrap(), before);
    // Actual one-node roots in a multi-root owner retain same-position behavior.
    let single = owned_ordered(SampleType::F32, &[1, 2]);
    assert!(single.try_move_readonly_filtered_flat_point_filter(movement(&single, 0, 0, 0), &ctl()).unwrap().shares_model(&single));
    // Corrupt unrelated canonical prestate cannot be repaired or bypassed by noop.
    let q = quota();
    let (mut d, _g) = ordered_fixture(&q, SampleType::F32, &[2, 2]);
    d.xcf_state.as_mut().unwrap().items[0].visible = !d.layers[0].visible;
    assert!(measured(&d, &q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: upstream1024 perstack bound succeeds at destination1023,1025 nodes refuse; global4096 remains independent and oldPoint stays separate"]
fn actual_stack_matches_upstream_bound() {
    let owner = owned_ordered(SampleType::F32, &[1024]);
    let p = movement(&owner, 0, 0, 1023);
    owner.validate_readonly_filtered_flat_point_filter_move(p, &ctl()).unwrap();
    let old = owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.iter().map(|n| n.id).collect::<Vec<_>>();
    let changed = owner.try_move_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
    let nodes = &changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
    assert_eq!(nodes.last().unwrap().id, old[0]);
    assert_eq!(nodes[..1023].iter().map(|n| n.id).collect::<Vec<_>>(), old[1..]);
    assert!(owner.try_move_readonly_point_filter(p, &ctl()).is_err());
    let q = quota();
    let (too_many, _guard) = ordered_fixture(&q, SampleType::F32, &[1025]);
    assert!(matches!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(&too_many, &q, &ctl()), Err(BackingError::Length)));
    // Four independent stacks still reach the4096 global bound; no global cap
    // was inadvertently narrowed to1024 with the per-stack guard.
    let q = quota();
    let (global, guard) = ordered_fixture(&q, SampleType::F32, &[1024, 1024, 1024, 1024]);
    assert!(measured(&global, &q, &ctl()).unwrap().index <= 16 << 20);
    let global = AccountedDocument::new_readonly_filtered_flat_point_storage(global, guard, q, &ctl()).unwrap();
    assert_eq!(global.document().xcf_state.as_ref().unwrap().filter_stacks.iter().map(|s| s.nodes.len()).sum::<usize>(), 4096);
    global.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
}
#[test]
#[ignore = "UNRUN: changed move denies fullIndex/late cumulative Work/copy-lease cancellation with exact quota refunds and original source owners intact"]
fn changed_copy_failures_are_atomic_and_refund() {
    let owner = owned_ordered(SampleType::F64, &[2, 5]);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let p = movement(&owner, 1, 4, 0);
    let blocker = q.reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    assert!(owner.try_move_readonly_filtered_flat_point_filter(p, &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let total = AtomicU64::new(0);
    let work = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = owner.try_move_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
    let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_move_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let work = |_| if q.usage().map_err(|_| BackingError::Quota)?.index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(owner.try_move_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_stacks[1].revision, 17);
}
