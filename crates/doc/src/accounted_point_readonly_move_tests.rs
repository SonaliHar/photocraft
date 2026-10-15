//! UNRUN structural move drafts. Opaque CURRENT identities carry no evaluator
//! or source qualification; external expected-model captures are test oracles.
use super::*;
use crate::{SampleEncoding, SampleType};
use photocraft_geom::TileCoord;
use photocraft_raster::Interrupt;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owner(revision: u64) -> AccountedDocument {
    let original = super::super::tests::owned(SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let quota = original.quota().clone();
    // Actual CURRENT fixture clone/addition fees precede every new allocation;
    // the original genuine owner retains its source, native tiles and ICC.
    let fee = quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let mut document = original.document().clone();
    let stack = &mut document.xcf_state.as_mut().unwrap().filter_stacks[0];
    stack.revision = revision;
    let mut second = stack.nodes[0].clone();
    second.id = FilterId(200);
    second.mask = None;
    second.enabled = true;
    second.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0x7fc05678));
    second.opacity_f64_bits = (-0f64).to_bits();
    stack.nodes.push(second);
    AccountedDocument::new_readonly_point_storage(document, Arc::new((original, fee)), quota, &ctl()).unwrap()
}
fn patch(owner: &AccountedDocument, filter: FilterId, index: usize) -> PointFilterMovePatch {
    PointFilterMovePatch {
        layer: owner.document().layers[0].id,
        filter,
        expected_stack_revision: owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision,
        index,
    }
}

#[test]
fn each_direction_changes_only_node_order_and_one_revision_with_exact_shared_payloads() {
    for (id, destination) in [(FilterId(100), 1), (FilterId(200), 0)] {
        let original = owner(7);
        let moved = original.try_move_readonly_point_filter(patch(&original, id, destination), &ctl()).unwrap();
        let mut expected = original.document().clone();
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.nodes.swap(0, 1);
        stack.revision = 8;
        assert_eq!(moved.document(), &expected);
        assert_eq!(original.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
        let a = &original.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        let b = &moved.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        assert!(Arc::ptr_eq(a.tile(TileCoord::new(1, 0)).unwrap(), b.tile(TileCoord::new(1, 0)).unwrap()));
        assert_eq!(a.default_bytes(), b.default_bytes());
        assert!(Arc::ptr_eq(original.document().icc_profile.as_ref().unwrap(), moved.document().icc_profile.as_ref().unwrap()));
        assert!(moved.quota().shares_ledger(original.quota()));
    }
}

#[test]
fn exact_position_noop_precedes_allocation_and_max_revision_but_invalid_requests_refuse() {
    let original = owner(u64::MAX);
    let q = original.quota();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(original.try_move_readonly_point_filter(patch(&original, FilterId(200), 1), &ctl()).unwrap().shares_model(&original));
    assert!(matches!(original.try_move_readonly_point_filter(patch(&original, FilterId(200), 0), &ctl()), Err(BackingError::Length)));
    for case in 0..4 {
        let mut request = patch(&original, FilterId(200), 1);
        match case {
            0 => request.filter = FilterId(999),
            1 => request.index = 2,
            2 => request.layer = LayerId(0),
            _ => request.expected_stack_revision = 0,
        }
        assert!(original.try_move_readonly_point_filter(request, &ctl()).is_err());
    }
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
fn work_cancel_and_quota_failure_keep_original_then_success_keeps_flat_root() {
    let original = owner(7);
    let q = original.quota();
    let before = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(original.try_move_readonly_point_filter(patch(&original, FilterId(200), 0), &denied), Err(BackingError::Work)));
    let late = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&late);
    assert!(matches!(
        original.try_move_readonly_point_filter(patch(&original, FilterId(200), 0), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), before);
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(original.try_move_readonly_point_filter(patch(&original, FilterId(200), 0), &ctl()).is_err());
    drop(blocker);
    let mut current = original.clone();
    for _ in 0..4 {
        let destination = if current.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id == FilterId(200) { 1 } else { 0 };
        current = current.try_move_readonly_point_filter(patch(&current, FilterId(200), destination), &ctl()).unwrap();
        let root = current.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
        assert!(root.original.shares_model(&original));
    }
    drop(current);
    assert_eq!(q.usage().unwrap(), before);
}
