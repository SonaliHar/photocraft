//! UNRUN structural removal and exact inert raw-mask archive drafts.
use super::*;
use crate::{SampleEncoding, SampleType, xcf_provenance::OpaqueRecord};
use photocraft_geom::TileCoord;
use photocraft_raster::Interrupt;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn raw() -> (Document, Arc<dyn Any + Send + Sync>, BackingQuota) {
    let original = super::super::tests::owned(SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let quota = original.quota().clone();
    let fee = quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let mut doc = original.document().clone();
    let state = doc.xcf_state.as_mut().unwrap();
    let mut second = state.filter_stacks[0].nodes[0].clone();
    second.id = FilterId(200);
    second.mask = Some(ItemRef::Mask(MaskId(201)));
    second.enabled = true;
    second.opacity_f64_bits = (-0f64).to_bits();
    state.filter_stacks[0].revision = 7;
    state.filter_stacks[0].nodes.push(second);
    let mut mask = state.filter_masks[0].clone();
    mask.id = MaskId(201);
    mask.owner = FilterId(200);
    state.filter_masks.push(mask);
    let mut item = state.items.iter().find(|item| item.item == ItemRef::Mask(MaskId(101))).unwrap().clone();
    item.item = ItemRef::Mask(MaskId(201));
    state.items.push(item);
    for (index, mask) in [MaskId(101), MaskId(201)].into_iter().enumerate() {
        let bytes = Arc::new(vec![0, 0x91, index as u8, 255]);
        state.opaque.push(OpaqueRecord {
            owner: Some(ItemRef::Mask(mask)),
            namespace: "gimp/xcf/source-property".into(),
            identity: format!("raw/μ/{index}"),
            source_version: 22,
            order: index as u32,
            flags: 0x80000001,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
    }
    (doc, Arc::new((original, fee)), quota)
}
fn adopt(doc: Document, guard: Arc<dyn Any + Send + Sync>, q: BackingQuota) -> AccountedDocument {
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}
fn owner() -> AccountedDocument {
    let (doc, guard, q) = raw();
    adopt(doc, guard, q)
}
fn patch(owner: &AccountedDocument, id: FilterId) -> PointFilterRemovePatch {
    PointFilterRemovePatch {
        layer: owner.document().layers[0].id,
        filter: id,
        expected_stack_revision: owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision,
    }
}

#[test]
fn either_stable_id_removes_only_its_node_and_mask_and_archives_exact_original_record() {
    for (index, id, mask) in [(0, FilterId(100), MaskId(101)), (1, FilterId(200), MaskId(201))] {
        let original = owner();
        let removed = original.try_remove_readonly_point_filter(patch(&original, id), &ctl()).unwrap();
        let mut expected = original.document().clone(); // external expected-model oracle
        let state = expected.xcf_state.as_mut().unwrap();
        state.filter_stacks[0].nodes.remove(index);
        state.filter_stacks[0].revision = 8;
        state.filter_masks.retain(|value| value.id != mask);
        state.items.retain(|item| item.item != ItemRef::Mask(mask));
        let record = &mut state.opaque[index];
        record.owner = None;
        record.identity = format!("deleted-filter-mask/{}/{}", mask.0, record.identity);
        assert_eq!(removed.document(), &expected);
        let before = original.document().xcf_state.as_ref().unwrap();
        let after = removed.document().xcf_state.as_ref().unwrap();
        assert!(Arc::ptr_eq(&before.opaque[index].bytes, &after.opaque[index].bytes));
        assert_eq!(before.provenance, after.provenance);
        assert_eq!(before.current_filter_output_precision, after.current_filter_output_precision);
        let survivor = &after.filter_masks[0];
        let old = before.filter_masks.iter().find(|value| value.id == survivor.id).unwrap();
        assert!(Arc::ptr_eq(old.surface.tile(TileCoord::new(1, 0)).unwrap(), survivor.surface.tile(TileCoord::new(1, 0)).unwrap()));
        assert!(Arc::ptr_eq(original.document().icc_profile.as_ref().unwrap(), removed.document().icc_profile.as_ref().unwrap()));
        assert!(removed.quota().shares_ledger(original.quota()));
        let root = removed.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
        assert!(root.original.shares_model(&original));
        let empty = removed.try_remove_readonly_point_filter(patch(&removed, after.filter_stacks[0].nodes[0].id), &ctl()).unwrap();
        assert!(empty.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
    }
}

#[path = "accounted_point_readonly_empty_tests.rs"]
mod empty;
#[test]
fn required_non_source_orphan_and_archive_collisions_refuse_before_copy() {
    for case in 0..6 {
        let (mut doc, guard, q) = raw();
        let state = doc.xcf_state.as_mut().unwrap();
        match case {
            0 => state.opaque[0].rendering_required = true,
            1 => state.opaque[0].namespace = "other/audit".into(),
            2 => state.opaque[0].owner = Some(ItemRef::Mask(MaskId(999))),
            3 => {
                let mut inert = state.opaque[0].clone();
                inert.owner = None;
                inert.identity = "deleted-filter-mask/101/raw/μ/0".into();
                state.opaque.push(inert);
            }
            4 => {
                let duplicate = state.opaque[0].clone();
                state.opaque.push(duplicate);
            }
            _ => state.opaque[0].identity = "a".repeat(4097),
        }
        let original = adopt(doc, guard, q);
        let before = original.quota().usage().unwrap();
        assert!(original.try_remove_readonly_point_filter(patch(&original, FilterId(100)), &ctl()).is_err());
        assert_eq!(original.quota().usage().unwrap(), before);
        assert_eq!(original.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 2);
    }
}
#[test]
fn absence_does_not_remove_unrelated_masks_or_archive_same_identity_from_other_namespaces() {
    let (mut doc, guard, q) = raw();
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_stacks[0].nodes[1].mask = None;
    state.filter_masks.retain(|m| m.id != MaskId(201));
    state.items.retain(|i| i.item != ItemRef::Mask(MaskId(201)));
    state.opaque[1].owner = None;
    let original = adopt(doc, guard, q);
    let removed = original.try_remove_readonly_point_filter(patch(&original, FilterId(200)), &ctl()).unwrap();
    let before = original.document().xcf_state.as_ref().unwrap();
    let after = removed.document().xcf_state.as_ref().unwrap();
    assert_eq!(before.filter_masks, after.filter_masks);
    assert_eq!(before.items, after.items);
    assert_eq!(before.opaque, after.opaque);
    let (mut doc, guard, q) = raw();
    let state = doc.xcf_state.as_mut().unwrap();
    let mut unrelated = state.opaque[0].clone();
    unrelated.owner = None;
    unrelated.namespace = "other/inert-audit".into();
    unrelated.identity = "deleted-filter-mask/101/raw/μ/0".into();
    state.opaque.push(unrelated.clone());
    let original = adopt(doc, guard, q);
    let removed = original.try_remove_readonly_point_filter(patch(&original, FilterId(100)), &ctl()).unwrap();
    assert_eq!(removed.document().xcf_state.as_ref().unwrap().opaque.last(), Some(&unrelated));
    let (bytes, length) = prefix(MaskId(u64::MAX)).unwrap();
    assert_eq!(&bytes[..length], b"deleted-filter-mask/18446744073709551615/");
}
#[test]
fn stale_target_max_revision_quota_work_and_late_cancel_keep_source_and_refund() {
    let original = owner();
    let q = original.quota();
    let before = q.usage().unwrap();
    for case in 0..3 {
        let mut request = patch(&original, FilterId(100));
        match case {
            0 => request.filter = FilterId(999),
            1 => request.layer = LayerId(0),
            _ => request.expected_stack_revision = 8,
        };
        assert!(original.try_remove_readonly_point_filter(request, &ctl()).is_err());
    }
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(original.try_remove_readonly_point_filter(patch(&original, FilterId(100)), &deny), Err(BackingError::Work)));
    let late = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&late);
    assert!(matches!(
        original.try_remove_readonly_point_filter(patch(&original, FilterId(100)), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackingError::Cancelled)
    ));
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(original.try_remove_readonly_point_filter(patch(&original, FilterId(100)), &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let (mut doc, guard, q) = raw();
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].revision = u64::MAX;
    let full = adopt(doc, guard, q);
    assert!(matches!(full.try_remove_readonly_point_filter(patch(&full, FilterId(100)), &ctl()), Err(BackingError::Length)));
}
