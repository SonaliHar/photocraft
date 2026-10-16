//! Ignored UNRUN structural removal/archive custody. Prepaid synthetic CURRENT
//! data supplies no producer/source/profile/evaluator/native numerical oracle.
use super::*;
use crate::xcf_provenance::OpaqueRecord;
fn raw_remove(sample: SampleType, counts: &[usize]) -> (Document, Arc<Guard>, BackingQuota) {
    let q = quota();
    let (mut d, g) = ordered_fixture(&q, sample, counts);
    let s = d.xcf_state.as_mut().unwrap();
    // Real existing16MiB Index/Resident preholds precede all these allocations.
    for (index, mask) in s.filter_masks.iter().enumerate() {
        let bytes = Arc::new(vec![0, 0x91, index as u8, 255]);
        s.opaque.push(OpaqueRecord {
            owner: Some(ItemRef::Mask(mask.id)),
            namespace: "gimp/xcf/source-property".into(),
            identity: format!("raw/\u{03bc}/{index}"),
            source_version: 22,
            order: index as u32,
            flags: 0x80000001,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
    }
    // Historical layer-owned effect property remains inert original source data.
    let mut root = s.opaque[0].clone();
    root.owner = Some(ItemRef::Layer(d.layers[0].id));
    root.identity = "original-effect-property".into();
    s.opaque.push(root);
    (d, g, q)
}
fn removal(owner: &AccountedDocument, stack: usize, index: usize) -> PointFilterRemovePatch {
    let s = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[stack];
    let ItemRef::Layer(layer) = s.owner else { panic!() };
    PointFilterRemovePatch { layer, filter: s.nodes[index].id, expected_stack_revision: s.revision }
}
fn adopt_remove(sample: SampleType, counts: &[usize]) -> AccountedDocument {
    let (d, g, q) = raw_remove(sample, counts);
    assert!(measured(&d, &q, &ctl()).unwrap().index <= 16 << 20);
    AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: first/middle/last nonfirst-root removal preserves exact survivor order/native/profile/source; inert Unicode identity archive retains byteArc/hash/order/flags"]
fn exact_target_archive_and_surviving_custody() {
    for sample in [SampleType::F32, SampleType::F64] {
        for index in [0, 1, 3] {
            let owner = adopt_remove(sample, &[3, 4]);
            let p = removal(&owner, 1, index);
            owner.validate_readonly_filtered_flat_point_filter_remove(p, &ctl()).unwrap();
            let changed = owner.try_remove_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
            let mut expected = owner.document().clone(); // External harness expectation only.
            let s = expected.xcf_state.as_mut().unwrap();
            let removed = s.filter_stacks[1].nodes.remove(index);
            let Some(ItemRef::Mask(mask)) = removed.mask else { panic!() };
            s.filter_stacks[1].revision += 1;
            s.filter_masks.retain(|m| m.id != mask);
            s.items.retain(|i| i.item != ItemRef::Mask(mask));
            for r in &mut s.opaque {
                if r.owner == Some(ItemRef::Mask(mask)) {
                    r.owner = None;
                    r.identity = format!("deleted-filter-mask/{}/{}", mask.0, r.identity);
                }
            }
            assert_eq!(changed.document(), &expected);
            assert!(changed.is_readonly_filtered_flat_point_storage());
            assert!(!changed.shares_model(&owner));
            assert!(Arc::ptr_eq(&changed.owner.retention, &owner.owner.retention));
            assert!(Arc::ptr_eq(changed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
            for (a, b) in owner.document().xcf_state.as_ref().unwrap().opaque.iter().zip(&changed.document().xcf_state.as_ref().unwrap().opaque) {
                assert!(Arc::ptr_eq(&a.bytes, &b.bytes));
                assert_eq!((&a.blake3, a.order, a.flags, a.source_version), (&b.blake3, b.order, b.flags, b.source_version));
            }
            let s = changed.document().xcf_state.as_ref().unwrap();
            for new in &s.filter_masks {
                let old = owner.document().xcf_state.as_ref().unwrap().filter_masks.iter().find(|m| m.id == new.id).unwrap();
                assert_eq!(old.surface.default_bytes(), new.surface.default_bytes());
                for ((a, x), (b, y)) in old.surface.tiles().zip(new.surface.tiles()) {
                    assert_eq!(a, b);
                    assert!(Arc::ptr_eq(x, y));
                }
            }
            assert!(s.items.iter().all(|i| i.item != ItemRef::Mask(mask)));
            assert!(s.filter_masks.iter().all(|m| m.id != mask));
            assert!(s.filter_stacks.iter().flat_map(|s| &s.nodes).all(|n| n.id != p.filter));
            assert!(changed.try_remove_readonly_filtered_flat_point_filter(p, &ctl()).is_err());
            assert!(changed.try_with_imported_layer_translation(p.layer, 0, 0, &ctl()).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN: CURRENT lastnode and single-root2to1 retain class but Source factory stays strict; wrongroot/filter/revision/MAX refuse"]
fn topology_and_identity_fences() {
    for counts in [&[1, 2][..], &[2][..]] {
        let owner = adopt_remove(SampleType::F32, counts);
        let changed = owner.try_remove_readonly_filtered_flat_point_filter(removal(&owner, 0, 0), &ctl()).unwrap();
        assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), counts[0] - 1);
        changed.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
        assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(changed.document(), changed.quota(), &ctl()).is_err());
    }
    for counts in [&[3][..], &[2, 1][..]] {
        let owner = adopt_remove(SampleType::F64, counts);
        let changed = owner.try_remove_readonly_filtered_flat_point_filter(removal(&owner, 0, 0), &ctl()).unwrap();
        assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), counts[0] - 1);
        changed.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
    }
    let owner = adopt_remove(SampleType::F32, &[2, 3]);
    let p = removal(&owner, 1, 1);
    let before = owner.quota().usage().unwrap();
    let mut bad = p;
    bad.layer = owner.document().layers[0].id;
    assert!(owner.validate_readonly_filtered_flat_point_filter_remove(bad, &ctl()).is_err());
    bad = p;
    bad.filter = FilterId(99999);
    assert!(owner.try_remove_readonly_filtered_flat_point_filter(bad, &ctl()).is_err());
    bad = p;
    bad.expected_stack_revision += 1;
    assert!(owner.try_remove_readonly_filtered_flat_point_filter(bad, &ctl()).is_err());
    assert_eq!(owner.quota().usage().unwrap(), before);
    let (mut d, g, q) = raw_remove(SampleType::F64, &[2, 3]);
    d.xcf_state.as_mut().unwrap().filter_stacks[1].revision = u64::MAX;
    let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap();
    assert!(matches!(owner.try_remove_readonly_filtered_flat_point_filter(removal(&owner, 1, 1), &ctl()), Err(BackingError::Length)));
}
#[test]
#[ignore = "UNRUN: non-source/too-long mask records, duplicate archive identity and existing ownerNone collision refuse; malformed dependency refused structurally"]
fn archival_collision_and_dependency_refusals() {
    for variant in 0..4 {
        let (mut d, g, q) = raw_remove(SampleType::F32, &[3]);
        let s = d.xcf_state.as_mut().unwrap();
        match variant {
            0 => s.opaque[1].namespace = "other/inert-audit".into(),
            1 => s.opaque[1].identity = "x".repeat(4097),
            2 => {
                let duplicate = s.opaque[1].clone();
                s.opaque.push(duplicate);
            }
            _ => {
                let mut archived = s.opaque[1].clone();
                let Some(ItemRef::Mask(mask)) = archived.owner else { panic!() };
                archived.owner = None;
                archived.identity = format!("deleted-filter-mask/{}/{}", mask.0, archived.identity);
                s.opaque.push(archived);
            }
        }
        let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap();
        let before = owner.quota().usage().unwrap();
        assert!(owner.try_remove_readonly_filtered_flat_point_filter(removal(&owner, 0, 1), &ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), before);
        assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 3);
    }
    let (mut d, _g, q) = raw_remove(SampleType::F32, &[3]);
    let s = d.xcf_state.as_mut().unwrap();
    s.filter_stacks[0].nodes[2].mask = s.filter_stacks[0].nodes[1].mask;
    assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(&d, &q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: archivegrowth/vector shifts prospectively paid; fullIndex/final-unit Work/copyfee cancellation refund unpublished copies without altering source owners"]
fn paid_archive_copy_failures_refund() {
    let owner = adopt_remove(SampleType::F64, &[3, 4]);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let p = removal(&owner, 1, 1);
    let blocker = q.reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    assert!(owner.try_remove_readonly_filtered_flat_point_filter(p, &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let total = AtomicU64::new(0);
    let work = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = owner.try_remove_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
    let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_remove_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let work = |_| if q.usage().map_err(|_| BackingError::Quota)?.index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(owner.try_remove_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_stacks[1].revision, 17);
}

#[path = "accounted_filtered_flat_readonly_current_tests.rs"]
mod current_tests;
