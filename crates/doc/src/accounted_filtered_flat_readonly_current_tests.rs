//! Ignored UNRUN explicit CURRENT topology/custody. This supplies no producer,
//! native marker authority, CMS/profile equality or numerical acceptance proof.
use super::*;
fn empty_raw(sample: SampleType, counts: &[usize]) -> (Document, Arc<Guard>, BackingQuota) {
    let (mut d, g, q) = raw_remove(sample, counts);
    let s = d.xcf_state.as_mut().unwrap();
    for stack in &mut s.filter_stacks {
        stack.nodes.clear();
    }
    for r in &mut s.opaque {
        if let Some(ItemRef::Mask(mask)) = r.owner {
            r.owner = None;
            r.identity = format!("deleted-filter-mask/{}/{}", mask.0, r.identity);
        }
    }
    s.filter_masks.clear();
    s.items.retain(|i| matches!(i.item, ItemRef::Layer(_)));
    // All modifications remain under genuine original16MiB Index/Resident
    // preholds. This explicit factory fixture asserts no historical edit origin.
    assert!(measured(&d, &q, &ctl()).unwrap().index <= 16 << 20);
    (d, g, q)
}
fn current_empty(sample: SampleType) -> AccountedDocument {
    let (d, g, q) = empty_raw(sample, &[2]);
    AccountedDocument::new_readonly_filtered_flat_point_current_storage(d, g, q, &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: actual typed3to2to1to0 removal retains stackowner/revision and archives all maskrecords; empty title/compound metadata copies preserve raw/native/profile/source custody"]
fn complete_lifecycle_retains_current_identity() {
    for sample in [SampleType::F32, SampleType::F64] {
        let original = adopt_remove(sample, &[3]);
        let mut owner = original.clone();
        for expected in [2, 1, 0] {
            let p = removal(&owner, 0, 0);
            owner = owner.try_remove_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
            let s = owner.document().xcf_state.as_ref().unwrap();
            assert_eq!(s.filter_stacks.len(), 1);
            assert_eq!(s.filter_stacks[0].nodes.len(), expected);
            assert_eq!(s.filter_stacks[0].revision, 20 - expected as u64);
            assert_eq!(s.filter_masks.len(), expected);
            assert_eq!(s.items.len(), expected + 1);
            owner.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
            assert!(Arc::ptr_eq(&owner.owner.retention, &original.owner.retention));
            assert!(Arc::ptr_eq(owner.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
        }
        let s = owner.document().xcf_state.as_ref().unwrap();
        assert_eq!(s.filter_stacks[0].owner, ItemRef::Layer(owner.document().layers[0].id));
        for (old, new) in original.document().xcf_state.as_ref().unwrap().opaque.iter().zip(&s.opaque) {
            assert!(Arc::ptr_eq(&old.bytes, &new.bytes));
            assert_eq!((&old.blake3, old.order, old.flags), (&new.blake3, new.order, new.flags));
            if matches!(old.owner, Some(ItemRef::Mask(_))) {
                assert!(new.owner.is_none());
                assert!(new.identity.starts_with("deleted-filter-mask/"));
            }
        }
        assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(owner.document(), owner.quota(), &ctl()).is_err());
        AccountedDocument::readonly_filtered_flat_point_current_storage_requirements(owner.document(), owner.quota(), &ctl()).unwrap();
        let titled = owner.try_with_document_name("empty-current.pcraft", &ctl()).unwrap();
        let layer = titled.document().layers[0].id;
        let edited =
            titled.try_edit(AccountedEdit::LayerMetadata { layer, name: Some("empty root"), visible: Some(false), opacity: Some(-0.0) }, &ctl()).unwrap();
        assert_eq!(edited.document().layers[0].opacity.to_bits(), (-0f32).to_bits());
        assert_eq!(edited.document().xcf_state.as_ref().unwrap().filter_stacks[0], s.filter_stacks[0]);
        assert!(Arc::ptr_eq(&edited.owner.retention, &original.owner.retention));
        assert_eq!(edited.document().source_color_context, original.document().source_color_context);
        assert_eq!(edited.document().xcf_state.as_ref().unwrap().provenance, original.document().xcf_state.as_ref().unwrap().provenance);
    }
}
#[test]
#[ignore = "UNRUN: explicit CURRENT factory/header requirements admit retainedempty states while old Source entrypoints refuse; no stack/no root/foreign or duplicate empty owner/danglingmask remain invalid"]
fn factory_purpose_and_empty_owner_relationships() {
    for sample in [SampleType::F32, SampleType::F64] {
        let (d, g, q) = empty_raw(sample, &[2]);
        assert_eq!(
            AccountedDocument::readonly_filtered_flat_point_current_storage_requirements(&d, &q, &ctl()).unwrap(),
            AccountedDocument::owned_model_reservation_bytes()
        );
        // Negative external clone only; no successful incoming ownership claim.
        assert!(AccountedDocument::new_readonly_filtered_flat_point_storage(d.clone(), g.clone(), q.clone(), &ctl()).is_err());
        let owner = AccountedDocument::new_readonly_filtered_flat_point_current_storage(d, g, q, &ctl()).unwrap();
        assert!(owner.is_readonly_filtered_flat_point_storage());
        assert!(!owner.is_readonly_point_storage());
    }
    for variant in 0..5 {
        let (mut d, _g, q) = empty_raw(SampleType::F32, &[2, 2]);
        match variant {
            0 => d.xcf_state.as_mut().unwrap().filter_stacks.clear(),
            1 => d.xcf_state.as_mut().unwrap().filter_stacks[0].owner = ItemRef::Layer(LayerId(99999)),
            2 => {
                let s = d.xcf_state.as_mut().unwrap();
                s.filter_stacks[1].owner = s.filter_stacks[0].owner;
            }
            3 => d.layers.clear(),
            _ => {
                let s = d.xcf_state.as_mut().unwrap();
                s.opaque[0].owner = Some(ItemRef::Mask(MaskId(2000)));
            }
        }
        assert!(AccountedDocument::readonly_filtered_flat_point_current_storage_requirements(&d, &q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: empty CURRENT fullIndex raw noops share, unknown filter/move/remove and individual properties stay refused; no allplain source/fallback authority"]
fn empty_noop_and_closed_edit_routes() {
    let owner = current_empty(SampleType::F64);
    let before = owner.quota().usage().unwrap();
    let id = owner.document().layers[0].id;
    let blocker = owner.quota().reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    assert!(owner.try_with_document_name(&owner.document().name, &ctl()).unwrap().shares_model(&owner));
    assert!(owner.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: None }, &ctl()).unwrap().shares_model(&owner));
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    assert!(owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: owner.document().layers[0].visible }, &ctl()).is_err());
    let revision = owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision;
    assert!(
        owner
            .try_with_readonly_filtered_flat_point_filter(
                PointFilterPatch { layer: id, filter: FilterId(1000), expected_stack_revision: revision, arguments: &[], enabled: None },
                &ctl()
            )
            .is_err()
    );
    assert!(
        owner
            .try_move_readonly_filtered_flat_point_filter(
                PointFilterMovePatch { layer: id, filter: FilterId(1000), expected_stack_revision: revision, index: 0 },
                &ctl()
            )
            .is_err()
    );
    assert!(
        owner
            .try_remove_readonly_filtered_flat_point_filter(
                PointFilterRemovePatch { layer: id, filter: FilterId(1000), expected_stack_revision: revision },
                &ctl()
            )
            .is_err()
    );
    assert!(owner.try_with_imported_layer_translation(id, 0, 0, &ctl()).is_err());
    drop(blocker);
    assert_eq!(owner.quota().usage().unwrap(), before);
}
#[test]
#[ignore = "UNRUN: current factory lowerWork/zeroheaderquota and empty privatecopy lateWork/cancel remain atomic on same paid native/profile/source ledger"]
fn current_intake_and_copy_failures_preserve_custody() {
    let (d, g, q) = empty_raw(SampleType::F32, &[2]);
    let before = q.usage().unwrap();
    let work = |_| Err(BackingError::Work);
    assert!(
        AccountedDocument::readonly_filtered_flat_point_current_storage_requirements(&d, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &work })
            .is_err()
    );
    let blocker = q.reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    // Existing genuine prehold covers this bounded synthetic negative clone.
    assert!(AccountedDocument::new_readonly_filtered_flat_point_current_storage(d.clone(), g.clone(), q.clone(), &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let owner = AccountedDocument::new_readonly_filtered_flat_point_current_storage(d, g, q, &ctl()).unwrap();
    let q = owner.quota();
    let before = q.usage().unwrap();
    let total = AtomicU64::new(0);
    let work = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let copied = owner.try_with_document_name("new-empty-current", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    drop(copied);
    assert_eq!(q.usage().unwrap(), before);
    let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
    let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_with_document_name("new-empty-current", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let stop = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(owner.try_with_document_name("new-empty-current", &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
