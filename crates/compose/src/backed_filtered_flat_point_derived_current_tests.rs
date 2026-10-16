//! UNRUN explicit CURRENT topology; no original-source or recovered-family claim.
use super::*;
use photocraft_doc::accounted_document::PointFilterRemovePatch;
fn remove_first(o: &AccountedDocument) -> AccountedDocument {
    let s = &o.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    o.try_remove_readonly_filtered_flat_point_filter(
        PointFilterRemovePatch { layer: o.document().layers[0].id, filter: s.nodes[0].id, expected_stack_revision: s.revision },
        &ctl(),
    )
    .unwrap()
}
fn adopt_current(f: Fixture) -> AccountedDocument {
    AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()).unwrap()
}
fn emptied(sample: SampleType) -> AccountedDocument {
    let (owner, _, _, _) = adopt(ordered(sample, &[&[0, 1, 2]], true));
    remove_first(&remove_first(&remove_first(&owner)))
}
fn clear_nodes(f: &mut Fixture) {
    let s = f.document.xcf_state.as_mut().unwrap();
    for stack in &mut s.filter_stacks {
        stack.nodes.clear();
    }
    s.filter_masks.clear();
    s.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
    s.opaque.retain(|r| !matches!(r.owner, Some(ItemRef::Mask(_))));
}
#[test]
#[ignore = "UNRUN actual single3-to2-to1-to0 typed CURRENT removal, exact native/profile and retainedempty anchor, Source raw factory/preparation refusal vs CURRENT Normal reference"]
fn actual_current_continuation_is_separate_from_source() {
    let (nl, linear) = parts();
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        let (owner, _, _, _) = adopt(ordered(sample, &[&[0, 1, 2]], true));
        let original = owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].clone(); // external expected metadata
        let mut next = owner.clone();
        for remaining in [2, 1, 0] {
            Context::new(&next, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).unwrap();
            next = remove_first(&next);
            let s = next.document().xcf_state.as_ref().unwrap();
            assert_eq!(s.filter_stacks[0].nodes.len(), remaining);
            assert_eq!(s.filter_stacks[0].owner, original.owner);
            assert_eq!(s.filter_stacks[0].revision, original.revision + (3 - remaining) as u64);
            assert_eq!(s.filter_masks.len(), remaining);
            assert_eq!(s.items.len(), remaining + 1);
            assert_eq!(next.document().icc_profile, owner.document().icc_profile);
            assert_eq!(next.document().source_color_context, owner.document().source_color_context);
            let c = Context::new(&next, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in [0u32, 2, 4, 7] {
                let p = input(0, x, 0, sample);
                let expected = match remaining {
                    2 => step(1, bridge(step(2, p, false, &nl, &linear)), true, &nl, &linear),
                    1 => step(2, p, false, &nl, &linear),
                    _ => p,
                };
                if remaining > 0 {
                    assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                }
                let normal = working.normal([0.; 4], expected, 1., GimpOperator::Union).unwrap();
                assert_eq!(c.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), normal.map(f64::to_bits));
            }
            if remaining < 2 {
                assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(next.document(), next.quota(), &ctl()).is_err());
                let q = next.quota().clone();
                let before = q.usage().unwrap();
                let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                assert!(LeasedFilteredFlatPointPipeline::new_budgeted(next.document(), &Interrupt::NONE, &mut b, next.clone()).is_err());
                assert_eq!(q.usage().unwrap(), before);
            }
        }
        assert!(next.is_readonly_filtered_flat_point_storage());
        assert!(next.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
    }
}
#[test]
#[ignore = "UNRUN explicit current empty factories reject orphan/duplicate anchors, residualmask/canonical mismatch and inactive profile/native errors; no vacuous node authority"]
fn empty_anchors_and_full_inactive_proof() {
    for fault in 0..5 {
        let mut f = ordered(SampleType::F64, &[&[0, 1], &[]], false);
        clear_nodes(&mut f);
        let s = f.document.xcf_state.as_mut().unwrap();
        match fault {
            0 => s.filter_stacks[0].owner = ItemRef::Layer(LayerId::fresh()),
            1 => s.filter_stacks.push(s.filter_stacks[0].clone()),
            2 => s.items[0].raster_bounds = Some(Rect::new(0, 0, 7, 6)),
            3 => {
                f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
            }
            _ => {
                s.filter_stacks.clear();
            }
        }
        if fault == 0 || fault == 1 {
            // Independently exercise the PRIVATE Current compiler without
            // borrowing the L1 orphan/duplicate-owner invariant. Test-only.
            let q = f.quota.clone();
            let before = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), used: 0, limit: u64::MAX };
            assert!(
                crate::backed_filtered_flat_point_context::compile(&f.document, Purpose::Current, 0, &Interrupt::NONE, &mut SharedBudget(&mut budget)).is_err()
            );
            assert_eq!(q.usage().unwrap(), before);
        }
        if let Ok(o) = AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()) {
            assert!(Context::new(&o, Default::default(), &ctl()).is_err());
        }
    }
    // A removed-node mask cannot survive as an orphan active payload.
    let mut f = ordered(SampleType::F64, &[&[0, 1], &[]], false);
    f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.clear();
    assert!(AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()).is_err());
    let mut f = ordered(SampleType::F64, &[&[0, 1], &[]], false);
    clear_nodes(&mut f);
    f.document.layers[0].visible = false;
    f.document.layers[0].opacity = 0.;
    let id = f.document.layers[0].id;
    f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
    let fails = f.fails.clone();
    let rasters = f.rasters.clone();
    let o = adopt_current(f);
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    rasters[0].clear_cache().unwrap();
    fails[0].store(true, Ordering::Relaxed);
    assert!(c.validate_operative(&ctl()).is_err());
    assert!(c.render_band(Rect::new(0, 0, 1, 1), &ctl()).is_err());
    fails[0].store(false, Ordering::Relaxed);
}
#[test]
#[ignore = "UNRUN empty CURRENT retains profile/Space/normal/header custody and cumulative late Work/cancel refund, source rejection without preparing Parts"]
fn empty_still_pays_full_controlled_normal_request() {
    let o = emptied(SampleType::F64);
    let q = o.quota().clone();
    let before = q.usage().unwrap();
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    assert!(q.usage().unwrap().resident > before.resident);
    c.validate_operative(&ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    c.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    let total = used.load(Ordering::Relaxed);
    assert!(total > 1);
    let used = AtomicU64::new(0);
    let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) >= total { Err(BackingError::Work) } else { Ok(()) };
    assert!(matches!(c.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let polls = AtomicU64::new(0);
    let count = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let i = Interrupt::cancel_only(&count);
    c.validate_operative(&ReadControl { interrupt: &i, work: &|_| Ok(()) }).unwrap();
    let total = polls.load(Ordering::Relaxed);
    assert!(total > 1);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= total - 1;
    let i = Interrupt::cancel_only(&cancel);
    assert!(matches!(c.validate_operative(&ReadControl { interrupt: &i, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    drop(c);
    // Persistent native cache may remain charged; no pin-drop eviction assumption.
    assert!(q.usage().unwrap().resident < baseline.resident);
}
