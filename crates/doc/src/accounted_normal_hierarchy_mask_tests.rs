//! Ignored UNRUN/uncompiled structural contracts. The inherited fixture is paid
//! synthetic CURRENT data, never a CMS, GIMP Source or renderer oracle.
use super::*;
use crate::normal_hierarchy_mask_index::{CheckedNormalHierarchyMaskIndex, NormalHierarchyMaskIndexRequirements};
use crate::xcf_filter_masks::FilterMaskCoverageClass;
fn masked(q: &BackingQuota, sample: SampleType) -> (Document, Arc<Hold>) {
    let (mut d, h) = fixture(q, sample);
    let bytes = if sample == SampleType::F64 { 0x3ff0000009b25010u64.to_le_bytes().to_vec() } else { 0.375f32.to_le_bytes().to_vec() };
    let mask = &mut d.xcf_state.as_mut().unwrap().filter_masks[1];
    mask.surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap().bytes_mut()[..bytes.len()].copy_from_slice(&bytes);
    (d, h)
}
fn masked_owner(q: &BackingQuota, s: SampleType) -> AccountedDocument {
    let (d, h) = masked(q, s);
    AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN/uncompiled: complete actual tree+coverage binding, strict API separation and exact physical/native/source custody"]
fn combined_bindings_and_explicit_source_current_split() {
    for sample in [SampleType::F32, SampleType::F64] {
        let q = quota();
        let before = q.usage().unwrap();
        let (d, h) = masked(&q, sample);
        assert!(d.checked_normal_hierarchy_index_with(NormalHierarchyLimits::STANDARD, &q, &ctl()).is_err());
        let index = d.checked_normal_hierarchy_mask_index_with(NormalHierarchyLimits::STANDARD, &q, &ctl()).unwrap();
        assert_eq!((index.len(), index.group_count(), index.leaf_count(), index.max_depth()), (6, 3, 3, 3));
        assert_eq!(index.raw_nonempty_count(), 1);
        let s = d.xcf_state.as_ref().unwrap();
        assert_eq!(index.classification_for_mask(0, &s.filter_masks[0]).unwrap(), FilterMaskCoverageClass::ByteZeroSentinel);
        assert_eq!(index.classification_for_mask(1, &s.filter_masks[1]).unwrap(), FilterMaskCoverageClass::RawNonemptyCoverage);
        let bindings = index.bindings_for_stack(0).unwrap();
        assert_eq!((bindings[1].leaf_ordinal, bindings[1].node_index, bindings[1].filter_id), (3, 1, FilterId(101)));
        assert!(std::ptr::eq(bindings[1].mask, &s.filter_masks[1]));
        let foreign = d.clone(); // external comparison clone covered by inherited prospective guard
        assert!(!index.matches_document(&foreign));
        assert!(index.classification_for_mask(1, &foreign.xcf_state.as_ref().unwrap().filter_masks[1]).is_err());
        drop(foreign);
        drop(index);
        let owner = AccountedDocument::new_readonly_masked_normal_hierarchy_storage(d, h, q.clone(), &ctl()).unwrap();
        assert!(owner.validate_readonly_normal_hierarchy_storage(&ctl()).is_err());
        owner.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
        let renamed = owner.try_with_document_name("mask tree", &ctl()).unwrap();
        assert_eq!(renamed.document().layers, owner.document().layers);
        assert_eq!(renamed.document().xcf_state, owner.document().xcf_state);
        assert!(Arc::ptr_eq(renamed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
        drop(owner);
        renamed.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
        drop(renamed);
        assert_eq!(q.usage().unwrap(), before);
        let (d, h) = fixture(&q, sample);
        assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_storage(d, h, q.clone(), &ctl()).is_err());
        let (mut d, h) = masked(&q, sample);
        d.xcf_state.as_mut().unwrap().provenance.format_version = 12;
        assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_storage(d, h, q.clone(), &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: nonfirst inactive masked target compound guard, stable-ID move/remove/archive and retained empty class"]
fn targeted_copy_and_empty_continuation() {
    let q = quota();
    let owner = masked_owner(&q, SampleType::F64);
    let original = owner.document().clone(); // external harness, inherited prehold
    let args = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.75f32.to_bits()) }];
    let patch = PointFilterPatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 7, arguments: &args, enabled: Some(true) };
    assert!(owner.try_with_readonly_normal_hierarchy_filter_with_opacity(patch, Some(0.375f64.to_bits()), &ctl()).is_err());
    let changed = owner.try_with_readonly_normal_hierarchy_filter_with_opacity(patch, Some(1.0f64.to_bits()), &ctl()).unwrap();
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 8);
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_masks, original.xcf_state.as_ref().unwrap().filter_masks);
    let moved = changed
        .try_move_readonly_normal_hierarchy_filter(
            PointFilterMovePatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 8, index: 0 },
            &ctl(),
        )
        .unwrap();
    assert_eq!(moved.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id, FilterId(101));
    let metadata = moved
        .try_edit(AccountedEdit::LayerMetadata { layer: LayerId(11), name: Some("masked parent"), visible: Some(false), opacity: Some(-0.) }, &ctl())
        .unwrap();
    let expanded = metadata.try_with_readonly_normal_hierarchy_group_expanded(None, true, true, &ctl()).unwrap();
    expanded.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
    let removed = expanded
        .try_remove_readonly_normal_hierarchy_filter(PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 9 }, &ctl())
        .unwrap();
    let empty = removed
        .try_remove_readonly_normal_hierarchy_filter(PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 10 }, &ctl())
        .unwrap();
    assert!(empty.is_readonly_normal_hierarchy_storage());
    let state = empty.document().xcf_state.as_ref().unwrap();
    assert!(state.filter_masks.is_empty());
    assert!(state.filter_stacks[0].nodes.is_empty());
    assert_eq!(state.filter_stacks[0].revision, 11);
    for (a, b) in state.opaque.iter().zip(&original.xcf_state.as_ref().unwrap().opaque) {
        assert!(a.owner.is_none());
        assert!(Arc::ptr_eq(&a.bytes, &b.bytes));
        assert_eq!((&a.blake3, a.order, a.flags), (&b.blake3, b.order, b.flags));
    }
    assert_eq!(empty.document().source_color_context, original.source_color_context);
    assert_eq!(empty.document().icc_profile, original.icc_profile);
    empty.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
    assert!(empty.try_edit(AccountedEdit::LayerVisibility { layer: LayerId(2), visible: false }, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: signedzero-only/negative/padding/inactive-opacity/canonical refusal and 16/64 header admission before reservation"]
fn complete_prestate_and_preallocation_caps() {
    for variant in 0..6 {
        let q = quota();
        let (mut d, h) = masked(&q, SampleType::F64);
        let state = d.xcf_state.as_mut().unwrap();
        match variant {
            0 => {
                let b = state.filter_masks[1].surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap().bytes_mut();
                b[..8].copy_from_slice(&(-0f64).to_le_bytes());
            }
            1 => {
                state.filter_stacks[0].nodes[1].opacity_f64_bits = 0.375f64.to_bits();
            }
            2 => {
                *state.filter_masks[1].surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap().bytes_mut().last_mut().unwrap() = 1;
            }
            3 => {
                state.items[3].parent = None;
            }
            4 => {
                let b = state.filter_masks[1].surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap().bytes_mut();
                b[..8].copy_from_slice(&(-1f64).to_le_bytes());
            }
            _ => {
                state.filter_stacks[0].nodes[1].mask = state.filter_stacks[0].nodes[0].mask;
            }
        }
        assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q, &ctl()).is_err());
    }
    let q = quota();
    let (mut d, _h) = masked(&q, SampleType::F64);
    let requested = std::cell::Cell::new(false);
    // Header-only malformed topology deliberately proves caps precede allocation.
    let mask = d.xcf_state.as_ref().unwrap().filter_masks[1].clone();
    let tile = mask.surface.tiles().next().unwrap().1.clone();
    d.xcf_state.as_mut().unwrap().filter_masks = vec![mask; 5];
    let state = d.xcf_state.as_mut().unwrap();
    let node = state.filter_stacks[0].nodes[0].clone();
    state.filter_stacks[0].nodes.resize(5, node);
    let item = state.items[0].clone();
    state.items.resize(11, item);
    for mask in &mut state.filter_masks {
        for x in 1..31 {
            mask.surface.put_tiles([(photocraft_geom::TileCoord::new(x, 0), tile.clone())]);
        }
    }
    assert!(
        d.checked_normal_hierarchy_mask_index_with_reservation(NormalHierarchyLimits::STANDARD, &Interrupt::NONE, &|_| Ok(()), |n| {
            requested.set(true);
            q.reserve(QuotaKind::Index, n).map_err(|_| BackingError::Quota)
        })
        .is_err()
    );
    assert!(!requested.get());
}
#[test]
#[ignore = "UNRUN/uncompiled: actual generic Source Index receipt, repeated96/sample original work, dynamic local cancel and atomic copy refunds"]
fn original_control_unique_lease_and_copy_fences() {
    struct Receipt {
        lease: QuotaLease,
        _padding: [u8; 2048],
    }
    impl crate::normal_hierarchy_index::NormalHierarchyIndexLease for Receipt {
        fn bytes(&self) -> u64 {
            self.lease.bytes()
        }
    }
    let q = quota();
    let (d, _h) = masked(&q, SampleType::F64);
    let req: NormalHierarchyMaskIndexRequirements =
        d.checked_normal_hierarchy_mask_index_requirements(NormalHierarchyLimits::STANDARD, &Interrupt::NONE, &|_| Ok(())).unwrap();
    let large =
        d.checked_normal_hierarchy_mask_index_requirements_for_lease::<Receipt>(NormalHierarchyLimits::STANDARD, &Interrupt::NONE, &|_| Ok(())).unwrap();
    assert!(large.proof_index_bytes >= req.proof_index_bytes + 2000);
    assert!(req.coverage_work_units > 96 * 65536);
    let count = std::cell::Cell::new(0u64);
    let before = q.usage().unwrap();
    let token: CheckedNormalHierarchyMaskIndex<'_, Receipt> = d
        .checked_normal_hierarchy_mask_index_with_reservation(
            NormalHierarchyLimits::STANDARD,
            &Interrupt::NONE,
            &|n| {
                count.set(count.get().checked_add(n).unwrap());
                Ok(())
            },
            |n| Ok(Receipt { lease: q.reserve(QuotaKind::Index, n).unwrap(), _padding: [0; 2048] }),
        )
        .unwrap();
    assert!(count.get() >= req.coverage_work_units + req.validation_work_units);
    drop(token);
    assert_eq!(q.usage().unwrap(), before);
    let held = q.usage().unwrap();
    let calls = std::cell::Cell::new(0usize);
    assert!(matches!(
        d.checked_normal_hierarchy_mask_index_with_reservation(
            NormalHierarchyLimits::STANDARD,
            &Interrupt::NONE,
            &|_| {
                calls.set(calls.get() + 1);
                if q.usage().unwrap().index > held.index { Err(BackingError::Cancelled) } else { Ok(()) }
            },
            |n| q.reserve(QuotaKind::Index, n).map_err(|_| BackingError::Quota)
        ),
        Err(BackingError::Cancelled)
    ));
    assert!(calls.get() > 1);
    assert_eq!(q.usage().unwrap(), before);
    let owner = masked_owner(&q, SampleType::F32);
    let baseline = q.usage().unwrap();
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let next = owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(next);
    assert_eq!(q.usage().unwrap(), baseline);
    let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
    let work = |n| left.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let block = q.reserve(QuotaKind::Index, (64u64 << 20) - baseline.index).unwrap();
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(owner.document().name, "tree");
}
