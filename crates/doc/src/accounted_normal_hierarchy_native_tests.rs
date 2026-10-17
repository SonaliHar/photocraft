//! Ignored UNRUN/uncompiled structural native hierarchy contracts. Actual
//! prospectively paid runtime native providers; no CMS/Source/brush oracle.
use super::*;
fn plain_native(q: &BackingQuota, sample: SampleType) -> AccountedDocument {
    let (mut d, h) = fixture(q, sample);
    d.source_color_context.as_mut().unwrap().source_precision = match sample {
        SampleType::U8 => 150,
        SampleType::U16 => 250,
        SampleType::U32 => 350,
        SampleType::F32 => 650,
        SampleType::F64 => 750,
    };
    let state = d.xcf_state.as_mut().unwrap();
    for stack in &mut state.filter_stacks {
        stack.nodes.clear();
    }
    state.filter_masks.clear();
    state.items.retain(|i| matches!(i.item, ItemRef::Layer(_)));
    state.opaque.clear();
    let bytes = Arc::new(vec![0, 0x91, 3, 255]);
    state.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Layer(LayerId(2))),
        namespace: "structural/native-test".into(),
        identity: "source-bytes".into(),
        source_version: 22,
        order: 7,
        flags: 9,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
        rendering_required: false,
    });
    AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).unwrap()
}
fn pixel(sample: SampleType) -> Vec<u8> {
    match sample {
        SampleType::U8 => vec![17, 91, 203, 191],
        SampleType::U16 => [0x8001u16, 0x1235, 0xfffd, 0xc001].into_iter().flat_map(u16::to_ne_bytes).collect(),
        SampleType::U32 => [0x80000001u32, 0x12345679, 0xfffffffd, 0xc0000001].into_iter().flat_map(u32::to_ne_bytes).collect(),
        SampleType::F32 => [0.5f32, -0., f32::from_bits(1), 0.75].into_iter().flat_map(f32::to_ne_bytes).collect(),
        SampleType::F64 => [0.5 + 2f64.powi(-44), -0., f64::from_bits(1), -0.].into_iter().flat_map(f64::to_ne_bytes).collect(),
    }
}
fn capture(owner: &AccountedDocument, id: LayerId, x: i32, y: i32) -> Vec<u8> {
    let ordinal = target_ordinal(owner.document(), id, &ctl()).unwrap();
    let LayerContent::BackedRaster(b) = &layer_at(owner.document(), ordinal, &ctl()).unwrap().content else { panic!() };
    let mut out = vec![0; b.format().bytes_per_pixel()];
    b.checked_view().read_logical_pixel_native_into(x, y, &mut out, &ctl()).unwrap();
    out
}
#[test]
#[ignore = "UNRUN/uncompiled: actual nested nonfirst all5-depth native COW, exact defaults/source/canonical/emptyanchors and generic-path denial"]
fn nested_native_cow_preserves_actual_tree_and_original_owner() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let q = quota();
        let owner = plain_native(&q, sample);
        let bytes = pixel(sample);
        let rect = Rect::new(1, 2, 2, 3);
        let edit = AccountedEdit::NativePixels { layer: LayerId(2), rect, bytes: &bytes };
        owner.validate_readonly_normal_hierarchy_native_edit(edit, &ctl()).unwrap();
        assert!(owner.validate_imported_native_edit(edit, &ctl()).is_err());
        assert!(owner.try_edit(edit, &ctl()).is_err());
        let before = owner.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
        let row = before.row(3).unwrap();
        assert_eq!((row.id, row.parent), (LayerId(2), Some(2)));
        let LayerContent::BackedRaster(source) = &row.layer.content else { panic!() };
        let descriptor = source.descriptor();
        drop(before);
        let changed = owner.try_edit_readonly_normal_hierarchy_native(edit, &ctl()).unwrap();
        assert!(changed.is_readonly_normal_hierarchy_storage());
        assert!(!changed.shares_model(&owner));
        assert_eq!(capture(&changed, LayerId(2), 1, 2), bytes);
        assert!(capture(&owner, LayerId(2), 1, 2).iter().all(|v| *v == 0));
        assert!(capture(&changed, LayerId(2), 2, 2).iter().all(|v| *v == 0));
        assert_eq!(changed.document().xcf_state, owner.document().xcf_state);
        assert_eq!(changed.document().source_color_context, owner.document().source_color_context);
        assert!(Arc::ptr_eq(changed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
        let next = changed.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
        let old = owner.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
        for (a, b) in old.rows().iter().zip(next.rows()) {
            assert_eq!((a.id, a.parent, a.canonical_item_index, a.operative_bounds), (b.id, b.parent, b.canonical_item_index, b.operative_bounds));
            assert_eq!(
                (&a.layer.name, a.layer.visible, a.layer.opacity.to_bits(), a.layer.locks),
                (&b.layer.name, b.layer.visible, b.layer.opacity.to_bits(), b.layer.locks)
            );
            if a.id != LayerId(2) && !a.layer.is_group() {
                assert_eq!(a.layer.content, b.layer.content);
            }
        }
        let LayerContent::BackedRaster(backed) = &next.row(3).unwrap().layer.content else { panic!() };
        assert_eq!(backed.descriptor(), descriptor);
        assert_eq!(backed.default_bytes(), descriptor.default_bytes());
        drop(old);
        drop(next);
        assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
        assert!(changed.try_edit_readonly_normal_hierarchy_native(edit, &ctl()).unwrap().shares_model(&changed));
        let renamed = changed.try_with_document_name("painted current", &ctl()).unwrap();
        drop(changed);
        drop(owner);
        renamed.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
        assert_eq!(capture(&renamed, LayerId(2), 1, 2), bytes);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: stored-disabled filters/masks remain closed before arrays; real group/foreign/region targets refuse and inherited-lock data remains exact"]
fn native_target_and_plain_domain_are_not_class_marker_authority() {
    let q = quota();
    let owner = plain_native(&q, SampleType::F64);
    let bytes = pixel(SampleType::F64);
    let baseline = q.usage().unwrap();
    for edit in [
        AccountedEdit::NativePixels { layer: LayerId(10), rect: Rect::new(0, 0, 1, 1), bytes: &bytes },
        AccountedEdit::NativePixels { layer: LayerId(u64::MAX), rect: Rect::new(0, 0, 1, 1), bytes: &bytes },
        AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::EMPTY, bytes: &[] },
        AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::new(-1, 0, 0, 1), bytes: &bytes },
        AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::new(0, 0, 1, 1), bytes: &bytes[..31] },
        AccountedEdit::LayerName { layer: LayerId(2), name: "not a native patch" },
    ] {
        assert!(owner.try_edit_readonly_normal_hierarchy_native(edit, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let (mut d, h) = fixture(&q, SampleType::F32);
    for n in &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes {
        n.enabled = false;
    }
    let filtered = AccountedDocument::new_readonly_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).unwrap();
    let held = q.usage().unwrap();
    let all_index = q.reserve(QuotaKind::Index, (64u64 << 20) - held.index).unwrap();
    let all_resident = q.reserve(QuotaKind::Resident, (128u64 << 20) - q.usage().unwrap().resident).unwrap();
    assert_eq!(
        filtered
            .validate_readonly_normal_hierarchy_native_edit(
                AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::new(0, 0, 1, 1), bytes: &[0; 16] },
                &ctl()
            )
            .err(),
        Some(BackingError::Unsupported)
    );
    drop(all_resident);
    drop(all_index);
    // Pure data lookup retains ancestor policy; actual mutation policy is engine.
    let (mut d, h) = fixture(&q, SampleType::F32);
    let state = d.xcf_state.as_mut().unwrap();
    for stack in &mut state.filter_stacks {
        stack.nodes.clear();
    }
    state.filter_masks.clear();
    state.opaque.clear();
    state.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
    state.items[0].locks.alpha = true;
    d.layers[0].locks.pixels = true;
    let locked = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, h, q, &ctl()).unwrap();
    let access = locked.accounted_layer_access(LayerId(2), &ctl()).unwrap().unwrap();
    assert!(access.locks.pixels);
    assert!(access.imported_locks.alpha);
    assert_eq!(access.layer.id, LayerId(2));
}
#[test]
#[ignore = "UNRUN/uncompiled: independent fixed raw-noop Resident fee, real COW/lateWork/cancel refunds and original source unchanged"]
fn native_noop_and_changed_copy_use_actual_original_quota_and_control() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let q = quota();
        let owner = plain_native(&q, sample);
        let rect = Rect::new(0, 0, 1, 1);
        let zero = capture(&owner, LayerId(2), 0, 0);
        let bytes = pixel(sample);
        let edit = AccountedEdit::NativePixels { layer: LayerId(2), rect, bytes: &bytes };
        let noop = AccountedEdit::NativePixels { layer: LayerId(2), rect, bytes: &zero };
        let baseline = q.usage().unwrap();
        assert!(owner.try_edit_readonly_normal_hierarchy_native(noop, &ctl()).unwrap().shares_model(&owner));
        let full = q.reserve(QuotaKind::Resident, (128u64 << 20) - baseline.resident).unwrap();
        assert_eq!(owner.try_edit_readonly_normal_hierarchy_native(noop, &ctl()).err(), Some(BackingError::Quota));
        drop(full);
        assert_eq!(q.usage().unwrap(), baseline);
        let total = AtomicU64::new(0);
        let count = |n| {
            total.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let changed = owner.try_edit_readonly_normal_hierarchy_native(edit, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
        drop(changed);
        assert_eq!(q.usage().unwrap(), baseline);
        let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
        let work = |n| left.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
        assert!(owner.try_edit_readonly_normal_hierarchy_native(edit, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        // Observe a real newly acquired COW tile, beyond the fixed plan scratch.
        let cancel = |_| if q.usage().unwrap().resident > baseline.resident + 65536 { Err(BackingError::Cancelled) } else { Ok(()) };
        assert_eq!(
            owner.try_edit_readonly_normal_hierarchy_native(edit, &ReadControl { interrupt: &Interrupt::NONE, work: &cancel }).err(),
            Some(BackingError::Cancelled)
        );
        assert_eq!(q.usage().unwrap(), baseline);
        let all_index = q.reserve(QuotaKind::Index, (64u64 << 20) - baseline.index).unwrap();
        assert!(owner.try_edit_readonly_normal_hierarchy_native(edit, &ctl()).is_err());
        drop(all_index);
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(capture(&owner, LayerId(2), 0, 0), zero);
        assert_eq!(owner.document().name, "tree");
    }
}
