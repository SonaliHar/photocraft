//! UNRUN declared reverse-canonical NLFloat policy. Synthetic paid CURRENT,
//! independent public math references; never producer RGB/Babl acceptance.
use super::*;
struct Extra {
    _prior: Arc<dyn Any + Send + Sync>,
    _index: QuotaLease,
    _resident: QuotaLease,
}
fn ordered(sample: SampleType, orders: &[&[u8]], enabled: bool) -> Fixture {
    assert!((1..=4).contains(&orders.len()));
    let mut kinds = [None; 4];
    for (kind, order) in kinds.iter_mut().zip(orders) {
        *kind = order.first().copied();
    }
    let mut f = fixture(sample, Size::new(8, 6), &kinds[..orders.len().max(2)], None);
    // Prospective model/arguments/Arc/BTree and one shared physical-mask payload
    // envelope, BEFORE any templates or additional nodes are cloned/allocated.
    let index = f.quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = f.quota.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    f.retention = Arc::new(Extra { _prior: f.retention, _index: index, _resident: resident });
    f.document.layers.truncate(orders.len());
    let mut prototypes = Vec::new();
    for (root, order) in orders.iter().enumerate() {
        if order.is_empty() {
            prototypes.push(None);
            continue;
        }
        let layer = f.document.layers[root].id;
        let state = f.document.xcf_state.as_ref().unwrap();
        let stack = state.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer)).unwrap();
        let node = stack.nodes[0].clone();
        let ItemRef::Mask(id) = node.mask.unwrap() else { panic!("mask") };
        let mask = state.filter_masks.iter().find(|m| m.id == id).unwrap().clone();
        let row = state.items.iter().find(|r| r.item == ItemRef::Mask(id)).unwrap().clone();
        prototypes.push(Some((stack.clone(), node, mask, row)));
    }
    let ids: Vec<_> = f.document.layers.iter().map(|l| l.id).collect();
    let state = f.document.xcf_state.as_mut().unwrap();
    state.items.retain(|r| matches!(r.item,ItemRef::Layer(id) if ids.contains(&id)));
    state.filter_stacks.clear();
    state.filter_masks.clear();
    for (root, order) in orders.iter().enumerate() {
        let Some((mut stack, template, mask_template, row_template)) = prototypes[root].take() else { continue };
        stack.nodes.clear();
        stack.nodes.try_reserve_exact(order.len()).unwrap();
        assert!(stack.nodes.capacity() <= order.len() * 2);
        for (index, kind) in order.iter().enumerate() {
            let mut node = template.clone();
            configure(&mut node, *kind);
            node.enabled = enabled;
            node.id = FilterId(LayerId::fresh().0);
            let id = MaskId(LayerId::fresh().0);
            node.mask = Some(ItemRef::Mask(id));
            let mut mask = mask_template.clone();
            mask.id = id;
            mask.owner = node.id;
            let mut row = row_template.clone();
            row.item = ItemRef::Mask(id);
            row.name = format!("ordered-mask-{root}-{index}");
            row.mask.as_mut().unwrap().owner = ItemRef::Layer(ids[root]);
            state.filter_masks.push(mask);
            state.items.push(row);
            stack.nodes.push(node);
        }
        state.filter_stacks.push(stack);
    }
    assert!(state.filter_masks.capacity() <= state.filter_masks.len().max(1) * 2 + 4);
    assert!(state.items.capacity() <= state.items.len() * 2 + 8);
    f
}
fn bridge(p: [f64; 4]) -> [f64; 4] {
    let out = p.map(|v| f64::from(v as f32));
    assert!(out.iter().all(|v| v.is_finite()));
    out
}
fn step(kind: u8, p: [f64; 4], derived: bool, nl: &photocraft_cms::owned_lab::Parts, linear: &photocraft_cms::owned_lab::Parts) -> [f64; 4] {
    assert!(p.iter().all(|v| v.is_finite()));
    assert!((0.0..=1.0).contains(&p[3]));
    if !derived {
        assert!(p.iter().all(|v| (0.0..=1.0).contains(v)));
    }
    let curve = photocraft_cms::srgb_trc();
    match kind {
        0 => {
            let input = [curve.eval64(p[0]), curve.eval64(p[1]), curve.eval64(p[2]), p[3]].map(|v| f64::from(v as f32));
            let operation = photocraft_algo::gimp::point_semi_flatten::SemiFlatten { color: [0.25, 0.5, 0.75, 0.375] };
            let mut out = [0.; 4];
            if derived {
                operation.try_eval_derived_pixel(&input, 3, &mut out).unwrap();
            } else {
                operation.try_eval_pixel(&input, 3, &mut out).unwrap();
            }
            [curve.eval_inverse64(out[0]), curve.eval_inverse64(out[1]), curve.eval_inverse64(out[2]), out[3]]
        }
        1 => physical(nl, p, 31.25, 22.5, -12.5),
        _ => {
            let input = [curve.eval64(p[0]) as f32, curve.eval64(p[1]) as f32, curve.eval64(p[2]) as f32, p[3] as f32];
            let sat = if derived {
                photocraft_algo::gimp::point_vibrance::saturation_linear_float_derived(input, 0.75)
            } else {
                photocraft_algo::gimp::point_vibrance::saturation_linear_float(input, 0.75)
            }
            .unwrap();
            let out = physical(linear, sat.map(f64::from), 0., 25., 0.);
            [
                f64::from(curve.eval_inverse64(out[0]) as f32),
                f64::from(curve.eval_inverse64(out[1]) as f32),
                f64::from(curve.eval_inverse64(out[2]) as f32),
                out[3],
            ]
        }
    }
}
fn parts() -> (photocraft_cms::owned_lab::Parts, photocraft_cms::owned_lab::Parts) {
    // External reference allocations, not realq preparation receipts.
    (
        photocraft_cms::owned_lab::construct_with_control(
            photocraft_cms::Builtin::Srgb.profile(),
            photocraft_cms::xcf_color::Encoding::NonlinearProfile,
            Default::default(),
            &|| false,
        )
        .unwrap(),
        photocraft_cms::owned_linear_srgb_lab::construct_with_control(Default::default(), &|| false).unwrap(),
    )
}
fn root_value(context: &Context<'_>, root: usize, x: i32, y: i32) -> [f64; 4] {
    let LayerContent::BackedRaster(backed) = &context.owner.document().layers[root].content else { panic!("backed") };
    let view = backed.checked_view();
    let pin = view.pin_tile(photocraft_geom::TileCoord { tx: 0, ty: 0 }, &ctl()).unwrap();
    let mut raw = [0u8; 32];
    let bpp = backed.format().bytes_per_pixel();
    pin.read_pixel_native_into(x, y, &mut raw[..bpp], &ctl()).unwrap();
    context.plan.entries[root].point.as_ref().unwrap().evaluate(context.owner.quota(), &raw[..bpp], (x, y), &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN explicit three noncommuting reversecanonical sequences, raw wide leaf then bottomfirst Normal; declaredmath only"]
fn named_sequences_are_not_family_batches_or_forward_order() {
    let (nl, linear) = parts();
    let orders: [&[u8]; 4] = [&[0, 1, 2], &[1, 2, 0], &[2, 0, 1], &[]];
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        let (owner, _, _, _) = adopt(ordered(sample, &orders, true));
        let context = Context::new(&owner, Default::default(), &ctl()).unwrap();
        context.validate_operative(&ctl()).unwrap();
        for x in [0u32, 2, 4, 7] {
            let a = input(0, x, 0, sample);
            let b = input(1, x, 0, sample);
            let c = input(2, x, 0, sample);
            // These fixed nested calls are an independent named sequence, not
            // a reference loop reversing the renderer's node container.
            let vib_hue_semi = step(0, bridge(step(1, bridge(step(2, a, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear);
            let semi_vib_hue = step(1, bridge(step(2, bridge(step(0, b, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear);
            let hue_semi_vib = step(2, bridge(step(0, bridge(step(1, c, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear);
            assert_eq!(root_value(&context, 0, x as i32, 0).map(f64::to_bits), vib_hue_semi.map(f64::to_bits));
            assert_eq!(root_value(&context, 1, x as i32, 0).map(f64::to_bits), semi_vib_hue.map(f64::to_bits));
            assert_eq!(root_value(&context, 2, x as i32, 0).map(f64::to_bits), hue_semi_vib.map(f64::to_bits));
            let expected = working
                .normal(
                    working
                        .normal(
                            working
                                .normal(working.normal([0.; 4], vib_hue_semi, 1., GimpOperator::Union).unwrap(), semi_vib_hue, 0.375, GimpOperator::Union)
                                .unwrap(),
                            hue_semi_vib,
                            0.5,
                            GimpOperator::Union,
                        )
                        .unwrap(),
                    input(3, x, 0, sample),
                    1.,
                    GimpOperator::Union,
                )
                .unwrap();
            assert_eq!(context.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
            if x == 2 {
                let wrong = step(2, bridge(step(1, bridge(step(0, a, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear);
                assert_ne!(vib_hue_semi.map(f64::to_bits), wrong.map(f64::to_bits));
            }
        }
    }
}
#[test]
#[ignore = "UNRUN disabled strict/derived proof without Float bridge, exact F64 all-disabled and trailingSemi wide inverse; no sourceHDR grant"]
fn disabled_stages_do_not_create_initial_or_final_float_boundaries() {
    let (nl, linear) = parts();
    let (owner, _, _, _) = adopt(ordered(SampleType::F64, &[&[0, 1, 2]], false));
    let context = Context::new(&owner, Default::default(), &ctl()).unwrap();
    context.validate_operative(&ctl()).unwrap();
    let native = input(0, 4, 0, SampleType::F64);
    assert_ne!(native[1].to_bits(), f64::from(native[1] as f32).to_bits());
    assert_ne!(native[3].to_bits(), f64::from(native[3] as f32).to_bits());
    assert_eq!(root_value(&context, 0, 4, 0).map(f64::to_bits), native.map(f64::to_bits));
    drop(context);
    let mut f = ordered(SampleType::F64, &[&[0, 0, 1]], false);
    f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1].enabled = true;
    let (owner, _, _, _) = adopt(f);
    let context = Context::new(&owner, Default::default(), &ctl()).unwrap();
    let expected = step(0, input(0, 2, 0, SampleType::F64), false, &nl, &linear);
    assert!(expected[..3].iter().any(|v| v.to_bits() != f64::from(*v as f32).to_bits()));
    assert_eq!(root_value(&context, 0, 2, 0).map(f64::to_bits), expected.map(f64::to_bits));
    // A fourth node is admitted; there is no accidental research-only3node cap.
    let (four, _, _, _) = adopt(ordered(SampleType::F64, &[&[0, 0, 0, 0]], false));
    let p = Context::new(&four, Default::default(), &ctl()).unwrap();
    assert_eq!(root_value(&p, 0, 4, 0).map(f64::to_bits), native.map(f64::to_bits));
    // Disabled malformed parameters and padding still refuse before publication.
    for fault in 0..2 {
        let mut f = ordered(SampleType::F64, &[&[0, 1]], false);
        if fault == 0 {
            f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1].arguments[0] = number("hue", 181.);
        } else {
            f.document.xcf_state.as_mut().unwrap().filter_masks[1]
                .surface
                .try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes())
                .unwrap();
        }
        if let Ok(owner) = AccountedDocument::new_readonly_filtered_flat_point_storage(f.document, f.retention, f.quota, &ctl()) {
            assert!(Context::new(&owner, Default::default(), &ctl()).is_err());
        }
    }
}
struct Budget {
    q: BackingQuota,
    used: u64,
    limit: u64,
}
impl PreparationBudget for Budget {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.used = self.used.checked_add(n).ok_or(CompositionError::Invalid("draft work overflow"))?;
        if self.used > self.limit { Err(CompositionError::Invalid("draft work denied")) } else { Ok(()) }
    }
}
#[test]
#[ignore = "UNRUN perstack1024/global4096 node caps independently precede physicalfee/scan, plus rootaggregate4096physicalmask cap; no alias pruning"]
fn actual_node_and_root_mask_caps_precede_scans() {
    let mut f = ordered(SampleType::F64, &[&[0, 0]], false);
    let q = f.quota.clone();
    let template = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
    let nodes = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
    nodes.clear();
    nodes.try_reserve_exact(1025).unwrap();
    assert!(nodes.capacity() <= 2050);
    for _ in 0..1025 {
        nodes.push(template.clone());
    }
    assert!(nodes.capacity() * std::mem::size_of::<photocraft_doc::xcf_filters::FilterNode>() < (8 << 20));
    let before = q.usage().unwrap();
    let mut budget = Budget { q: q.clone(), used: 0, limit: 1 << 20 };
    assert!(matches!(
        LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut budget, f.retention.clone()),
        Err(CompositionError::Unsupported(_))
    ));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(f.reads.load(Ordering::Relaxed), 0);
    assert_eq!(budget.used, 2); // Entry check + first actual stack; no physical work.
    // A separate global rejection: five DISTINCT actual roots and stack owners,
    // each stack <=1024. The intentionally incomplete mask/node identities
    // are never claimed valid: global admission must refuse BEFORE those scans.
    let mut f = ordered(SampleType::F64, &[&[0], &[0], &[0], &[0]], false);
    // ordered() already retains prospective Index8MiB+Resident8MiB before all
    // following native-owner/row/stack/node clones. Backed payloads stay shared.
    let mut fifth = f.document.layers[3].clone();
    let old_id = fifth.id;
    fifth.id = LayerId::fresh();
    let fifth_id = fifth.id;
    f.document.layers.push(fifth);
    let state = f.document.xcf_state.as_mut().unwrap();
    let mut row = state.items.iter().find(|r| r.item == ItemRef::Layer(old_id)).unwrap().clone();
    row.item = ItemRef::Layer(fifth_id);
    state.items.push(row);
    let mut fifth = state.filter_stacks[3].clone();
    fifth.owner = ItemRef::Layer(fifth_id);
    state.filter_stacks.push(fifth);
    for (stack, count) in state.filter_stacks.iter_mut().zip([1024, 1024, 1024, 1024, 1]) {
        let template = stack.nodes[0].clone();
        stack.nodes.clear();
        stack.nodes.try_reserve_exact(count).unwrap();
        assert!(stack.nodes.capacity() <= 2 * count);
        for _ in 0..count {
            stack.nodes.push(template.clone());
        }
    }
    assert!(state.filter_stacks.iter().all(|s| s.nodes.len() <= 1024));
    assert_eq!(state.filter_stacks.iter().map(|s| s.nodes.len()).sum::<usize>(), 4097);
    assert_eq!(f.document.layers.iter().map(|l| l.id).collect::<std::collections::HashSet<_>>().len(), 5);
    assert!(state.filter_stacks.iter().map(|s| s.nodes.capacity() * std::mem::size_of::<photocraft_doc::xcf_filters::FilterNode>()).sum::<usize>() < (8 << 20));
    let q = f.quota.clone();
    let before = q.usage().unwrap();
    let mut budget = Budget { q: q.clone(), used: 0, limit: 1 << 20 };
    assert!(matches!(
        LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut budget, f.retention.clone()),
        Err(CompositionError::Unsupported(_))
    ));
    assert_eq!(budget.used, 6); // Entry + all five actual stack counts, no census.
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(f.reads.load(Ordering::Relaxed), 0);
    let mut f = ordered(SampleType::F64, &[&[0, 0]], false);
    let tile = Arc::new(f.document.xcf_state.as_ref().unwrap().filter_masks[0].surface.try_default_tile().unwrap());
    for (index, n) in [2050, 2047].into_iter().enumerate() {
        f.document.xcf_state.as_mut().unwrap().filter_masks[index]
            .surface
            .put_tiles((0..n).map(|i| (photocraft_geom::TileCoord { tx: i, ty: 0 }, tile.clone())));
    }
    let q = f.quota.clone();
    let before = q.usage().unwrap();
    let mut budget = Budget { q: q.clone(), used: 0, limit: 1 << 20 };
    assert!(matches!(
        LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut budget, f.retention.clone()),
        Err(CompositionError::Unsupported(_))
    ));
    assert!(budget.used < 1 << 20);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(f.reads.load(Ordering::Relaxed), 0);
}
#[test]
#[ignore = "UNRUN every-stage fullpeak/source custody, cumulative latework/cancel and finalroot I/O no partialband on real sameledger"]
fn stage_owners_and_failures_keep_real_custody() {
    let (owner, rasters, fails, _) = adopt(ordered(SampleType::F64, &[&[0, 1, 2], &[1, 2, 0], &[2, 0, 1], &[]], true));
    let q = owner.quota().clone();
    let before = q.usage().unwrap();
    let mut budget = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let pipeline = LeasedFilteredFlatPointPipeline::new_budgeted(owner.document(), &Interrupt::NONE, &mut budget, owner.clone()).unwrap();
    assert!(q.usage().unwrap().resident >= before.resident + 9 * 65536);
    let work = budget.used;
    drop(pipeline);
    assert_eq!(q.usage().unwrap(), before);
    assert!(work > 0);
    let mut denied = Budget { q: q.clone(), used: 0, limit: work - 1 };
    assert!(LeasedFilteredFlatPointPipeline::new_budgeted(owner.document(), &Interrupt::NONE, &mut denied, owner.clone()).is_err());
    assert!(denied.used > denied.limit);
    assert_eq!(q.usage().unwrap(), before);
    let polls = AtomicU64::new(0);
    let count = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let interrupt = Interrupt::cancel_only(&count);
    let mut budget = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    drop(LeasedFilteredFlatPointPipeline::new_budgeted(owner.document(), &interrupt, &mut budget, owner.clone()).unwrap());
    let accepted = polls.load(Ordering::Relaxed);
    assert!(accepted > 1);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= accepted - 1;
    let interrupt = Interrupt::cancel_only(&cancel);
    let mut budget = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_budgeted(owner.document(), &interrupt, &mut budget, owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let prepared = PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).unwrap();
    prepared.validate_operative(&ctl()).unwrap();
    let rect = Rect::new(0, 0, 1, 1);
    drop(prepared.render_band(rect, &ctl()).unwrap());
    rasters[3].clear_cache().unwrap();
    let baseline = q.usage().unwrap();
    fails[3].store(true, Ordering::Relaxed);
    assert!(prepared.render_band(rect, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    fails[3].store(false, Ordering::Relaxed);
    drop(prepared.render_band(rect, &ctl()).unwrap());
    let baseline = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(prepared.render_band(rect, &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
}

#[path = "backed_filtered_flat_point_move_tests.rs"]
mod filter_move;

#[path = "backed_filtered_flat_point_remove_tests.rs"]
mod filter_remove;

#[path = "backed_filtered_flat_point_derived_current_tests.rs"]
mod derived_current;

#[path = "backed_filtered_flat_point_exposure_tests.rs"]
mod exposure;

#[path = "backed_filtered_flat_point_invert_tests.rs"]
mod invert;

#[path = "backed_filtered_flat_point_loaded_exposure_tests.rs"]
mod loaded_exposure;
