//! Ignored UNRUN ordered structural custody, not operation/HDR/source/render
//! qualification. Synthetic native stores and raw masks stay genuinely prepaid.
use super::*;
fn ordered_fixture(q: &BackingQuota, sample: SampleType, counts: &[usize]) -> (Document, Arc<Guard>) {
    assert!(!counts.is_empty() && counts.len() <= 5);
    let (mut d, g) = fixture(q, sample);
    let old = d.xcf_state.as_ref().unwrap();
    let stack = old.filter_stacks[0].clone();
    let mask = old.filter_masks[0].clone();
    let mask_item = old.items.iter().find(|i| i.item == ItemRef::Mask(mask.id)).unwrap().clone();
    d.layers.truncate(counts.len());
    // Additional test roots share an actually paid same-q native owner. Their
    // headers/names/canonical rows remain covered by the existing model prehold.
    while d.layers.len() < counts.len() {
        let mut layer = d.layers[0].clone();
        layer.id = LayerId(100 + d.layers.len() as u64);
        d.layers.push(layer);
    }
    let roots = d
        .layers
        .iter()
        .map(|l| {
            let mut row = old
                .items
                .iter()
                .find(|i| i.item == ItemRef::Layer(l.id))
                .unwrap_or_else(|| old.items.iter().find(|i| i.item == ItemRef::Layer(d.layers[0].id)).unwrap())
                .clone();
            row.item = ItemRef::Layer(l.id);
            row.name = l.name.clone();
            row.visible = l.visible;
            row
        })
        .collect::<Vec<_>>();
    let state = d.xcf_state.as_mut().unwrap();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.items = roots;
    state.selection.selected = d.layers.iter().map(|l| ItemRef::Layer(l.id)).collect();
    state.selection.active = state.selection.selected.first().copied();
    let mut ordinal = 0u64;
    for (root, count) in counts.iter().enumerate() {
        if *count == 0 {
            continue;
        }
        let owner = ItemRef::Layer(d.layers[root].id);
        let mut next = stack.clone();
        next.owner = owner;
        next.revision = 17;
        next.nodes.clear();
        for _ in 0..*count {
            let mut node = stack.nodes[0].clone();
            node.id = FilterId(1000 + ordinal);
            let mid = MaskId(2000 + ordinal);
            node.mask = Some(ItemRef::Mask(mid));
            node.enabled = ordinal % 2 == 0;
            node.arguments =
                vec![FilterArgument { name: "scalar".into(), value: FilterValue::Number(ExactNumber::Float32Bits((ordinal as f32 / 16.).to_bits())) }];
            let fid = node.id;
            next.nodes.push(node);
            let mut m = mask.clone();
            m.id = mid;
            m.owner = fid;
            state.filter_masks.push(m);
            let mut row = mask_item.clone();
            row.item = ItemRef::Mask(mid);
            row.mask.as_mut().unwrap().owner = owner;
            state.items.push(row);
            ordinal += 1;
        }
        state.filter_stacks.push(next);
    }
    (d, g)
}
fn owned_ordered(sample: SampleType, counts: &[usize]) -> AccountedDocument {
    let q = quota();
    let (d, g) = ordered_fixture(&q, sample, counts);
    assert!(measured(&d, &q, &ctl()).unwrap().index <= 16 << 20);
    AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap()
}
fn target<'a>(owner: &AccountedDocument, stack: usize, node: usize, args: &'a [PointArgumentPatch<'a>], enabled: Option<bool>) -> PointFilterPatch<'a> {
    let s = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[stack];
    let ItemRef::Layer(layer) = s.owner else { panic!() };
    PointFilterPatch { layer, filter: s.nodes[node].id, expected_stack_revision: s.revision, arguments: args, enabled }
}
#[test]
#[ignore = "UNRUN: one actual root with4 ordered nodes proves no3cap; paid nonfirstnode patch/title retains exact siblings/order/native/masks/profile/source"]
fn one_root_four_nodes_preserve_order_and_exact_target() {
    for sample in [SampleType::F32, SampleType::F64] {
        let owner = owned_ordered(sample, &[4]);
        assert_eq!(owner.document().layers.len(), 1);
        let args = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.75f32.to_bits()) }];
        let p = target(&owner, 0, 3, &args, Some(true));
        owner.validate_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
        let changed = owner.try_with_readonly_filtered_flat_point_filter(p, &ctl()).unwrap();
        let mut expected = owner.document().clone(); // external harness expectation
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.revision = 18;
        stack.nodes[3].enabled = true;
        stack.nodes[3].arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits()));
        assert_eq!(changed.document(), &expected);
        assert!(Arc::ptr_eq(&changed.owner.retention, &owner.owner.retention));
        let titled = changed.try_with_document_name("ordered.pcraft", &ctl()).unwrap();
        expected.name = "ordered.pcraft".into();
        assert_eq!(titled.document(), &expected);
        assert!(Arc::ptr_eq(&titled.owner.retention, &owner.owner.retention));
        assert!(Arc::ptr_eq(titled.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
        assert!(titled.try_with_readonly_filtered_flat_point_filter(target(&titled, 0, 3, &args, Some(true)), &ctl()).unwrap().shares_model(&titled));
        for (a, b) in owner.document().xcf_state.as_ref().unwrap().filter_masks.iter().zip(&titled.document().xcf_state.as_ref().unwrap().filter_masks) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.owner, b.owner);
            assert_eq!(a.surface.default_bytes(), b.surface.default_bytes());
            for ((ca, ta), (cb, tb)) in a.surface.tiles().zip(b.surface.tiles()) {
                assert_eq!(ca, cb);
                assert!(Arc::ptr_eq(ta, tb));
            }
        }
        let id = titled.document().layers[0].id;
        assert!(titled.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: true }, &ctl()).is_err());
        assert!(titled.try_with_imported_layer_translation(id, 0, 0, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: same-stack/cross-root nonfirst duplicate and mask ownership/cardinality failures, global4096 and strict singleton topology refuse"]
fn global_and_local_identity_limits_refuse_before_publication() {
    for variant in 0..7 {
        let q = quota();
        let (mut d, _g) = ordered_fixture(&q, SampleType::F32, &[2, 2]);
        let s = d.xcf_state.as_mut().unwrap();
        let mask_id = s.filter_masks[3].id;
        match variant {
            0 => s.filter_stacks[0].nodes[1].id = s.filter_stacks[0].nodes[0].id,
            1 => s.filter_stacks[1].nodes[1].id = s.filter_stacks[0].nodes[1].id,
            2 => s.filter_stacks[0].nodes[1].mask = s.filter_stacks[0].nodes[0].mask,
            3 => s.filter_masks[2].owner = s.filter_stacks[0].nodes[1].id,
            4 => s.items.iter_mut().find(|i| i.item == ItemRef::Mask(mask_id)).unwrap().mask.as_mut().unwrap().owner = ItemRef::Layer(d.layers[0].id),
            5 => {
                s.filter_masks.pop();
            }
            _ => s.filter_stacks[1].nodes.clear(),
        }
        assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(&d, &q, &ctl()).is_err());
    }
    let q = quota();
    let (d, _g) = ordered_fixture(&q, SampleType::F32, &[1]);
    assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(&d, &q, &ctl()).is_err());
    let q = quota();
    let (mut d, _g) = ordered_fixture(&q, SampleType::F32, &[1024, 1024, 1024, 1024, 1]);
    let s = d.xcf_state.as_mut().unwrap();
    let last = s.filter_masks.pop().unwrap().id;
    s.items.retain(|i| i.item != ItemRef::Mask(last));
    // All five stacks fit1024 and4096 masks pass their cheap cardinality guard;
    // actual node sum4097
    // must still fail before any native tile or per-node deep interpretation.
    assert!(matches!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(&d, &q, &ctl()), Err(BackingError::Length)));
}
fn fill_maps(d: &mut Document, counts: &[usize], poison: bool) {
    let state = d.xcf_state.as_mut().unwrap();
    let payload = state.filter_masks[0].surface.tiles().next().unwrap().1.clone();
    for (mask_index, (mask, count)) in state.filter_masks.iter_mut().zip(counts).enumerate() {
        let mut surface = Surface::try_with_default_bytes(mask.surface.format(), mask.surface.default_bytes()).unwrap();
        for i in 0..*count {
            let coord = if poison && mask_index == 0 && i == 0 { TileCoord::new(-1, 0) } else { TileCoord::new((i % 64) as i32, (i / 64) as i32) };
            surface.put_tiles(std::iter::once((coord, payload.clone())));
        }
        mask.surface = surface;
    }
}
fn widen(d: &mut Document, q: &BackingQuota, g: &Arc<Guard>) {
    d.size = Size::new(16384, 16384);
    let bounds = d.bounds();
    // Logical generated input is not a resident4GiB allocation or source-intake
    // allowance. This custody test reads no pixels and makes no render/native-save
    // or large-image completion claim; actual maps share prepaid physical payload.
    for layer in &mut d.layers {
        let format = PixelFormat::new(d.mode, d.depth, true);
        let descriptor = NativePlaneDescriptor::new(format, bounds, &vec![0; format.bytes_per_pixel()]).unwrap();
        let raster = BackedRaster::new(Arc::new(Store { descriptor, _guard: g.clone() }), q.clone(), 1, &ctl()).unwrap();
        layer.content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    }
    let s = d.xcf_state.as_mut().unwrap();
    for mask in &mut s.filter_masks {
        mask.bounds = bounds;
    }
    for item in &mut s.items {
        item.raster_bounds = Some(bounds);
        if let Some(m) = &mut item.mask {
            m.bounds = bounds;
        }
    }
}

#[test]
#[ignore = "UNRUN: per-root aggregate4096 maps precheck; separate roots may exceed4096 total physicaltiles without a global tilecap"]
fn physical_map_aggregate_is_per_root_and_precedes_tile_walk() {
    let q = quota();
    let (mut bad, g) = ordered_fixture(&q, SampleType::F32, &[2]);
    widen(&mut bad, &q, &g);
    fill_maps(&mut bad, &[2048, 2049], true);
    // Each map fits the logical grid cardinality, but one negative coordinate
    // would produce Bounds in a deep scan. Aggregate must report Length first.
    assert!(matches!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(&bad, &q, &ctl()), Err(BackingError::Length)));
    let mask = &bad.xcf_state.as_ref().unwrap().filter_masks[0];
    assert!(matches!(MaskPlan::new(&mask.surface, mask.bounds, &ctl()), Err(BackingError::Bounds)));
    let q = quota();
    let (mut d, g) = ordered_fixture(&q, SampleType::F32, &[1, 1]);
    widen(&mut d, &q, &g);
    fill_maps(&mut d, &[3000, 3000], false);
    assert!(measured(&d, &q, &ctl()).unwrap().index <= 16 << 20);
    let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap();
    owner.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_masks.iter().map(|m| m.surface.tile_count()).sum::<usize>(), 6000);
}
#[test]
#[ignore = "UNRUN: nonfirst ordered target MAX/noop and crossroot/stale rejection, sameledger wholecopy Work/cancel refunds preserve all source owners"]
fn ordered_target_failures_and_copy_rollback_keep_original() {
    let q = quota();
    let (mut d, g) = ordered_fixture(&q, SampleType::F64, &[3, 2]);
    d.xcf_state.as_mut().unwrap().filter_stacks[1].revision = u64::MAX;
    let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap();
    let before = owner.quota().usage().unwrap();
    let node = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[1].nodes[1];
    assert!(owner.try_with_readonly_filtered_flat_point_filter(target(&owner, 1, 1, &[], Some(node.enabled)), &ctl()).unwrap().shares_model(&owner));
    assert!(owner.try_with_readonly_filtered_flat_point_filter(target(&owner, 1, 1, &[], Some(!node.enabled)), &ctl()).is_err());
    let mut wrong = target(&owner, 0, 2, &[], None);
    wrong.filter = node.id;
    assert!(owner.try_with_readonly_filtered_flat_point_filter(wrong, &ctl()).is_err());
    wrong = target(&owner, 0, 2, &[], None);
    wrong.expected_stack_revision += 1;
    assert!(owner.validate_readonly_filtered_flat_point_filter(wrong, &ctl()).is_err());
    assert_eq!(owner.quota().usage().unwrap(), before);
    let p = target(&owner, 0, 2, &[], Some(!owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[2].enabled));
    let count = AtomicU64::new(0);
    let work = |n| {
        count.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = owner.try_with_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    drop(changed);
    assert_eq!(owner.quota().usage().unwrap(), before);
    let left = AtomicU64::new(count.load(Ordering::Relaxed) - 1);
    let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_with_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(owner.quota().usage().unwrap(), before);
    let work = |_| if owner.quota().usage().map_err(|_| BackingError::Quota)?.index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(owner.try_with_readonly_filtered_flat_point_filter(p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(owner.quota().usage().unwrap(), before);
}

#[path = "accounted_filtered_flat_readonly_move_tests.rs"]
mod move_tests;

#[path = "accounted_filtered_flat_readonly_remove_tests.rs"]
mod remove_tests;
