//! UNRUN two-node structural custody. These synthetic opaque recipes grant no
//! source/evaluator authority. Actual observed stack fixtures have one mask per
//! node; the optional-mask structural policy below is not an observation claim.
use super::*;
use crate::{
    SampleEncoding, SampleType,
    item_identity::{FilterId, MaskId},
};
use photocraft_geom::TileCoord;
use photocraft_raster::Interrupt;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn raw(mask_count: usize) -> (Document, Arc<dyn Any + Send + Sync>, BackingQuota) {
    let original = super::tests::owned(SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let q = original.quota().clone();
    // This genuine source guard plus an actual prospective metadata fee owns
    // the fixture's cloned maps/defaults/DTOs before any extra clone/append.
    let fee = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let mut doc = original.document().clone();
    let state = doc.xcf_state.as_mut().unwrap();
    let mut second = state.filter_stacks[0].nodes[0].clone();
    second.id = FilterId(200);
    second.opacity_f64_bits = 0.5f64.to_bits();
    second.operation.name = "opaque:second-point".into();
    second.mask = if mask_count == 2 { Some(ItemRef::Mask(MaskId(201))) } else { None };
    state.filter_stacks[0].revision = 7;
    state.filter_stacks[0].nodes.push(second);
    if mask_count == 2 {
        let mut mask = state.filter_masks[0].clone();
        mask.id = MaskId(201);
        mask.owner = FilterId(200);
        let old_item = state.items.iter().find(|item| matches!(item.item, ItemRef::Mask(_))).unwrap();
        let mut item = old_item.clone();
        item.item = ItemRef::Mask(mask.id);
        item.name = "second raw mask".into();
        state.filter_masks.push(mask);
        state.items.push(item);
    }
    if mask_count == 0 {
        state.filter_stacks[0].nodes[0].mask = None;
        state.filter_masks.clear();
        state.items.retain(|item| !matches!(item.item, ItemRef::Mask(_)));
    }
    // Relations are identity-based, never parallel-vector ordinal based.
    state.filter_masks.reverse();
    state.items.reverse();
    (doc, Arc::new((original, fee)), q)
}
fn owned(mask_count: usize) -> AccountedDocument {
    let (doc, guard, q) = raw(mask_count);
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}
fn free_curves() -> FilterValue {
    FilterValue::Config(
        (0..5)
            .map(|i| FilterArgument {
                name: format!("channel-{i}"),
                value: FilterValue::Curve { free: true, points_f64_bits: Vec::new(), point_types: Vec::new(), samples_f64_bits: vec![(-0f64).to_bits(); 256] },
            })
            .collect(),
    )
}

#[test]
fn two_nodes_optional_masks_copy_every_raw_field_and_share_original_native_payloads() {
    for count in 0..=2 {
        let owner = owned(count);
        owner.validate_readonly_point_storage(&ctl()).unwrap();
        let copied = owner.try_with_document_name("both nodes retained", &ctl()).unwrap();
        let mut expected = owner.document().clone(); // external expected-model harness capture
        expected.name = "both nodes retained".into();
        assert_eq!(copied.document(), &expected);
        let before = owner.document().xcf_state.as_ref().unwrap();
        let after = copied.document().xcf_state.as_ref().unwrap();
        assert_eq!(after.filter_stacks[0].nodes.len(), 2);
        assert_eq!(after.filter_masks.len(), count);
        assert_eq!(after.items.len(), 1 + count);
        assert_eq!(after.filter_stacks[0].nodes[1].opacity_f64_bits, 0.5f64.to_bits());
        assert!(copied.quota().shares_ledger(owner.quota()));
        assert!(Arc::ptr_eq(owner.document().icc_profile.as_ref().unwrap(), copied.document().icc_profile.as_ref().unwrap()));
        for (a, b) in before.filter_masks.iter().zip(&after.filter_masks) {
            assert_eq!(a.surface.default_bytes(), b.surface.default_bytes());
            assert!(Arc::ptr_eq(a.surface.tile(TileCoord::new(1, 0)).unwrap(), b.surface.tile(TileCoord::new(1, 0)).unwrap()));
            assert_eq!(a.surface.tile(TileCoord::new(1, 0)).unwrap().bytes().last(), Some(&0x91));
        }
        assert!(copied.try_edit(AccountedEdit::LayerName { layer: copied.document().layers[0].id, name: "generic" }, &ctl()).is_err());
    }
}

#[test]
fn duplicate_ids_or_shared_orphan_wrong_owner_and_item_relations_refuse_privately() {
    for case in 0..8 {
        let (mut doc, guard, q) = raw(2);
        let state = doc.xcf_state.as_mut().unwrap();
        match case {
            0 => state.filter_stacks[0].nodes[1].id = state.filter_stacks[0].nodes[0].id,
            1 => state.filter_stacks[0].nodes[1].mask = state.filter_stacks[0].nodes[0].mask,
            2 => state.filter_masks[1].id = state.filter_masks[0].id,
            3 => state.filter_masks[0].owner = FilterId(999),
            4 => state.filter_stacks[0].nodes[1].mask = None, // now an orphan actual mask
            5 => state.items[0].item = ItemRef::Mask(MaskId(999)),
            6 => {
                let extra = state.filter_stacks[0].nodes[1].clone();
                state.filter_stacks[0].nodes.push(extra);
            }
            _ => state.filter_stacks[0].nodes.clear(), // empty stack with orphan masks still refuses
        }
        assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}

#[test]
fn two_complete_free_tables_are_2560_words_and_filter_local_limits_still_apply() {
    let (mut doc, guard, q) = raw(2);
    for node in &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes {
        node.arguments = vec![FilterArgument { name: "config".into(), value: free_curves() }];
    }
    let owner = AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap();
    let state = owner.document().xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_stacks[0].nodes.iter().map(|n| point_argument_shape(&n.arguments, &ctl()).unwrap().f64_words).sum::<u64>(), 2560);
    let renamed = owner.try_with_document_name("all 2560 words retained", &ctl()).unwrap();
    assert_eq!(renamed.document().xcf_state, owner.document().xcf_state);
    let (mut doc, guard, q) = raw(2);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1].arguments = vec![
        FilterArgument { name: "config".into(), value: free_curves() },
        FilterArgument { name: "extra-word".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0)) },
    ];
    assert!(matches!(AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()), Err(BackingError::Length)));
}

#[test]
fn opacity_raw_unit_boundaries_and_all_copies_keep_flat_retention() {
    for bits in [0f64.to_bits(), (-0f64).to_bits(), 0.5f64.to_bits(), 1f64.to_bits()] {
        let (mut doc, guard, q) = raw(1);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1].opacity_f64_bits = bits;
        let owner = AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap();
        let copied = owner.try_with_document_name("unit opacity retained", &ctl()).unwrap();
        assert_eq!(copied.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].opacity_f64_bits, bits);
    }
    for bits in [f64::NAN.to_bits(), f64::INFINITY.to_bits(), (-0.1f64).to_bits(), 1.1f64.to_bits()] {
        let (mut doc, guard, q) = raw(0);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1].opacity_f64_bits = bits;
        assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).is_err());
    }
    let original = owned(2);
    let mut current = original.clone();
    for i in 0..10 {
        current = current.try_with_document_name(if i % 2 == 0 { "title A" } else { "title B" }, &ctl()).unwrap();
        let retained = current.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
        assert!(retained.original.shares_model(&original));
        assert_eq!(current.document().xcf_state, original.document().xcf_state);
    }
}

#[test]
fn two_node_copy_noop_quota_work_and_cancel_preserve_original_and_refund() {
    let owner = owned(2);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(owner.try_with_document_name(&owner.document().name, &ctl()).unwrap().shares_model(&owner));
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    drop(blocker);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(owner.try_with_document_name("changed", &deny), Err(BackingError::Work)));
    let cancelled = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(owner.try_with_document_name("changed", &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
}
