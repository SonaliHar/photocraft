//! Ignored UNRUN/uncompiled. Paid synthetic data-only integer tree custody;
//! these are not matching GIMP packets or parsed CMS/numerical oracles.
use super::*;
fn plain(q: &BackingQuota, sample: SampleType, version: u32, anchors: bool) -> (Document, Arc<Hold>) {
    // The inherited factory creates actual runtime-format providers/descriptors,
    // with real same-ledger Index/Resident guards before any model/native work.
    let (mut d, h) = fixture(q, sample);
    d.source_color_context.as_mut().unwrap().source_precision = match sample {
        SampleType::U8 => 150,
        SampleType::U16 => 250,
        _ => unreachable!(),
    };
    let state = d.xcf_state.as_mut().unwrap();
    state.provenance.format_version = version;
    for stack in &mut state.filter_stacks {
        stack.nodes.clear();
    }
    if !anchors {
        state.filter_stacks.clear();
    }
    state.filter_masks.clear();
    state.opaque.clear();
    state.items.retain(|i| matches!(i.item, ItemRef::Layer(_)));
    for item in &mut state.items {
        item.composition.as_mut().unwrap().compatibility_epoch = format!("xcf-{version}-normal");
    }
    (d, h)
}
fn integer_owner(q: &BackingQuota, sample: SampleType) -> AccountedDocument {
    let (d, h) = plain(q, sample, 22, true);
    AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN/uncompiled: exactU8/150 U16/250 explicitSource vs oldfloat/maskedSource, actual runtime defaults and retained empty anchors"]
fn exact_integer_roles_and_source_current_separation() {
    for sample in [SampleType::U8, SampleType::U16] {
        for version in [12, 13, 22] {
            for anchors in [false, true] {
                let q = quota();
                let initial = q.usage().unwrap();
                let (d, h) = plain(&q, sample, version, anchors);
                let expected = d.clone(); // external harness comparison under inherited prospective fixture bound
                let owner = AccountedDocument::new_readonly_plain_quantized_normal_hierarchy_storage(d, h, q.clone(), &ctl()).unwrap();
                owner.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
                assert!(owner.validate_readonly_normal_hierarchy_storage(&ctl()).is_err());
                let index = owner.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
                assert_eq!((index.len(), index.group_count(), index.leaf_count()), (6, 3, 3));
                assert_eq!(index.raw_nonempty_count(), 0);
                for row in index.rows() {
                    if let LayerContent::BackedRaster(b) = &row.layer.content {
                        assert_eq!(b.format(), PixelFormat::new(ColorMode::Rgb, sample, true));
                        assert_eq!(
                            b.default_bytes(),
                            match &layer_at(&expected, row.ordinal, &ctl()).unwrap().content {
                                LayerContent::BackedRaster(b) => b.default_bytes(),
                                _ => unreachable!(),
                            }
                        );
                    }
                }
                drop(index);
                let renamed = owner.try_with_document_name("integer tree", &ctl()).unwrap();
                assert_eq!(renamed.document().layers, expected.layers);
                assert_eq!(renamed.document().xcf_state, expected.xcf_state);
                if anchors {
                    assert_eq!(renamed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
                }
                assert!(Arc::ptr_eq(renamed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
                drop(expected);
                drop(owner);
                drop(renamed);
                assert_eq!(q.usage().unwrap(), initial);
                let (d, h) = plain(&q, sample, version, anchors);
                assert!(AccountedDocument::new_readonly_normal_hierarchy_storage(d, h, q.clone(), &ctl()).is_err());
                let (d, h) = plain(&q, sample, version, anchors);
                assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_storage(d, h, q.clone(), &ctl()).is_err());
            }
        }
    }
    let q = quota();
    let (d, h) = fixture(&q, SampleType::F32);
    assert!(AccountedDocument::new_readonly_plain_quantized_normal_hierarchy_storage(d, h, q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: actual disabled stored node and mask residue refuse before selected arrays; precision/encoding/profile/canonical negative"]
fn integer_plain_gate_precedes_reservation_and_repair() {
    for variant in 0..7 {
        let q = quota();
        let (mut d, _h) = plain(&q, SampleType::U16, 22, true);
        match variant {
            0 => {
                let (original, _extra) = fixture(&q, SampleType::U16);
                let mut node = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
                node.enabled = false;
                d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.push(node);
            }
            1 => {
                let (original, _extra) = fixture(&q, SampleType::U16);
                d.xcf_state.as_mut().unwrap().filter_masks.push(original.xcf_state.as_ref().unwrap().filter_masks[0].clone());
            }
            2 => d.source_color_context.as_mut().unwrap().source_precision = 150,
            3 => d.sample_encoding = Some(SampleEncoding::Linear),
            4 => d.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![1, 2, 3])),
            5 => d.mode = ColorMode::Grayscale,
            _ => {
                let stack = d.xcf_state.as_ref().unwrap().filter_stacks[0].clone();
                d.xcf_state.as_mut().unwrap().filter_stacks.resize(4097, stack);
            }
        }
        let requested = std::cell::Cell::new(false);
        assert!(
            d.checked_normal_hierarchy_mask_index_with_reservation(NormalHierarchyLimits::STANDARD, &Interrupt::NONE, &|_| Ok(()), |n| {
                requested.set(true);
                q.reserve(QuotaKind::Index, n).map_err(|_| BackingError::Quota)
            })
            .is_err()
        );
        assert!(!requested.get());
    }
    let q = quota();
    let (mut d, h) = plain(&q, SampleType::U8, 22, true);
    d.xcf_state.as_mut().unwrap().items[3].parent = None;
    assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).is_err());
    let (mut d, h) = plain(&q, SampleType::U16, 22, true);
    d.icc_profile = Some(Arc::new(vec![9, 8, 7]));
    assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: Current metadata/disclosure/empty target denials, truelocal cancellation and changed-copy late Work/Index atomic refunds"]
fn integer_copy_control_and_no_new_filter_or_pixel_authority() {
    for sample in [SampleType::U8, SampleType::U16] {
        let q = quota();
        let owner = integer_owner(&q, sample);
        let before = q.usage().unwrap();
        let noop = owner.try_with_document_name("tree", &ctl()).unwrap();
        assert!(noop.shares_model(&owner));
        drop(noop);
        let changed = owner
            .try_edit(AccountedEdit::LayerMetadata { layer: LayerId(11), name: Some("integer parent"), visible: Some(false), opacity: Some(-0.) }, &ctl())
            .unwrap();
        let expanded = changed.try_with_readonly_normal_hierarchy_group_expanded(None, true, true, &ctl()).unwrap();
        expanded.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
        assert_eq!(expanded.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
        assert!(expanded.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        assert!(expanded.document().xcf_state.as_ref().unwrap().filter_masks.is_empty());
        assert!(
            expanded
                .try_with_readonly_normal_hierarchy_filter(
                    PointFilterPatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 7, arguments: &[], enabled: Some(true) },
                    &ctl()
                )
                .is_err()
        );
        assert!(
            expanded
                .try_move_readonly_normal_hierarchy_filter(
                    PointFilterMovePatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 7, index: 0 },
                    &ctl()
                )
                .is_err()
        );
        assert!(
            expanded
                .try_remove_readonly_normal_hierarchy_filter(
                    PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 7 },
                    &ctl()
                )
                .is_err()
        );
        assert!(expanded.try_edit(AccountedEdit::LayerVisibility { layer: LayerId(2), visible: false }, &ctl()).is_err());
        drop(expanded);
        drop(changed);
        assert_eq!(q.usage().unwrap(), before);
        let total = AtomicU64::new(0);
        let count = |n| {
            total.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let changed = owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
        drop(changed);
        assert_eq!(q.usage().unwrap(), before);
        let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
        let work = |n| left.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
        assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
        assert_eq!(q.usage().unwrap(), before);
        let block = q.reserve(QuotaKind::Index, (64u64 << 20) - before.index).unwrap();
        assert!(owner.try_with_document_name("changed", &ctl()).is_err());
        drop(block);
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(owner.document().name, "tree");
    }
    let q = quota();
    let initial = q.usage().unwrap();
    let (d, h) = plain(&q, SampleType::U16, 13, true);
    let held = q.usage().unwrap();
    let calls = std::cell::Cell::new(0usize);
    let local = |_| {
        calls.set(calls.get() + 1);
        if q.usage().unwrap().index > held.index { Err(BackingError::Cancelled) } else { Ok(()) }
    };
    assert!(matches!(
        AccountedDocument::new_readonly_plain_quantized_normal_hierarchy_storage_with_local_work(d, h, q.clone(), &ctl(), &local),
        Err(BackingError::Cancelled)
    ));
    assert!(calls.get() > 1);
    assert_eq!(q.usage().unwrap(), initial);
}
