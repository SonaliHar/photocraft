//! UNRUN data-only paid U32 structural custody; not parsed CMS or producer pixels.
use super::*;
fn plain_u32(q: &BackingQuota, version: u32) -> (Document, Arc<Hold>) {
    let (mut d, h) = fixture(q, SampleType::U32);
    d.source_color_context.as_mut().unwrap().source_precision = 350;
    let s = d.xcf_state.as_mut().unwrap();
    s.provenance.format_version = version;
    for stack in &mut s.filter_stacks {
        stack.nodes.clear();
    }
    s.filter_masks.clear();
    s.opaque.clear();
    s.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
    for row in &mut s.items {
        row.composition.as_mut().unwrap().compatibility_epoch = format!("xcf-{version}-normal");
    }
    (d, h)
}
#[test]
#[ignore = "UNRUN U32/350 sameclass/native13 plain Source/current and metadata/expanded copy with retained emptyanchors and raw runtime32 defaults; no filter permission"]
fn u32_source_current_and_copy_keep_native_shape_and_empty_revisions() {
    for version in [12, 13, 22] {
        let q = quota();
        let initial = q.usage().unwrap();
        let (d, h) = plain_u32(&q, version);
        let expected = d.clone(); // Existing prospective16MiB model guards precede this external comparison clone.
        let owner = AccountedDocument::new_readonly_plain_quantized_normal_hierarchy_storage(d, h, q.clone(), &ctl()).unwrap();
        owner.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
        assert!(owner.validate_readonly_normal_hierarchy_storage(&ctl()).is_err());
        let index = owner.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
        assert_eq!((index.len(), index.group_count(), index.leaf_count(), index.raw_nonempty_count()), (6, 3, 3, 0));
        for row in index.rows() {
            if let LayerContent::BackedRaster(b) = &row.layer.content {
                assert_eq!(b.format(), PixelFormat::new(ColorMode::Rgb, SampleType::U32, true));
                assert_eq!(
                    b.default_bytes(),
                    match &layer_at(&expected, row.ordinal, &ctl()).unwrap().content {
                        LayerContent::BackedRaster(v) => v.default_bytes(),
                        _ => unreachable!(),
                    }
                );
            }
        }
        drop(index);
        let changed = owner
            .try_edit(AccountedEdit::LayerMetadata { layer: LayerId(11), name: Some("U32 parent"), visible: Some(false), opacity: Some(-0.) }, &ctl())
            .unwrap();
        let expanded = changed.try_with_readonly_normal_hierarchy_group_expanded(None, true, true, &ctl()).unwrap();
        let renamed = expanded.try_with_document_name("U32 tree", &ctl()).unwrap();
        renamed.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
        let s = renamed.document().xcf_state.as_ref().unwrap();
        assert!(s.filter_masks.is_empty());
        assert!(s.filter_stacks.iter().all(|st| st.nodes.is_empty()));
        assert_eq!(s.filter_stacks[0].revision, 7);
        assert_eq!(s.filter_stacks, expected.xcf_state.as_ref().unwrap().filter_stacks);
        assert!(
            renamed
                .try_with_readonly_normal_hierarchy_filter(
                    PointFilterPatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 7, arguments: &[], enabled: Some(true) },
                    &ctl()
                )
                .is_err()
        );
        assert!(
            renamed
                .try_move_readonly_normal_hierarchy_filter(
                    PointFilterMovePatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 7, index: 0 },
                    &ctl()
                )
                .is_err()
        );
        assert!(
            renamed
                .try_remove_readonly_normal_hierarchy_filter(
                    PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 7 },
                    &ctl()
                )
                .is_err()
        );
        drop(expected);
        drop(owner);
        drop(changed);
        drop(expanded);
        drop(renamed);
        assert_eq!(q.usage().unwrap(), initial);
        let (d, h) = plain_u32(&q, version);
        assert!(AccountedDocument::new_readonly_normal_hierarchy_storage(d, h, q.clone(), &ctl()).is_err());
        let (d, h) = plain_u32(&q, version);
        assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_storage(d, h, q.clone(), &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN U32 wronglinear/perceptual precision or disablednode/mask refuses before Index allocation; original local Work/Cancelled and changed-copy quota refunds"]
fn u32_preindex_plain_gate_and_original_controls_remain_closed() {
    for fault in 0..5 {
        let q = quota();
        let (mut d, _h) = plain_u32(&q, 22);
        match fault {
            0 => d.source_color_context.as_mut().unwrap().source_precision = 300,
            1 => d.source_color_context.as_mut().unwrap().source_precision = 375,
            2 => {
                let (original, _extra) = fixture(&q, SampleType::U32);
                let mut n = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
                n.enabled = false;
                d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.push(n);
            }
            3 => {
                let (original, _extra) = fixture(&q, SampleType::U32);
                d.xcf_state.as_mut().unwrap().filter_masks.push(original.xcf_state.as_ref().unwrap().filter_masks[0].clone());
            }
            _ => d.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![1, 2, 3])),
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
    let (d, h) = plain_u32(&q, 22);
    let owner = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).unwrap();
    let base = q.usage().unwrap();
    let total = AtomicU64::new(0);
    drop(
        owner
            .try_with_document_name(
                "changed",
                &ReadControl {
                    interrupt: &Interrupt::NONE,
                    work: &|n| {
                        total.fetch_add(n, Ordering::Relaxed);
                        Ok(())
                    },
                },
            )
            .unwrap(),
    );
    assert_eq!(q.usage().unwrap(), base);
    let used = AtomicU64::new(0);
    assert!(
        owner
            .try_with_document_name(
                "changed",
                &ReadControl {
                    interrupt: &Interrupt::NONE,
                    work: &|n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total.load(Ordering::Relaxed) / 2 {
                        Err(BackingError::Work)
                    } else {
                        Ok(())
                    }
                }
            )
            .is_err()
    );
    assert_eq!(q.usage().unwrap(), base);
    let stop = || true;
    let i = Interrupt::cancel_only(&stop);
    assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let block = q.reserve(QuotaKind::Index, (64u64 << 20) - base.index).unwrap();
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), base);
    assert_eq!(owner.document().name, "tree");
}
