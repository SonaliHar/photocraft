//! UNRUN topology-safe removal; declared math only, no producer oracle.
use super::*;
use photocraft_doc::{accounted_document::PointFilterRemovePatch, xcf_provenance::OpaqueRecord};
fn patch(o: &AccountedDocument, i: usize) -> PointFilterRemovePatch {
    let s = &o.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    PointFilterRemovePatch { layer: o.document().layers[0].id, filter: s.nodes[i].id, expected_stack_revision: s.revision }
}
fn fixture_records(sample: SampleType) -> Fixture {
    let mut f = ordered(sample, &[&[0, 1, 2], &[]], true);
    // The existing Index8MiB+Resident8MiB preholds precede all bounded records.
    let s = f.document.xcf_state.as_mut().unwrap();
    for i in 0..3 {
        let bytes = Arc::new(vec![0, 0x91, i as u8, 255]);
        s.opaque.push(OpaqueRecord {
            owner: s.filter_stacks[0].nodes[i].mask,
            namespace: "gimp/xcf/source-property".into(),
            identity: format!("remove/proof/{i}"),
            source_version: 22,
            order: i as u32,
            flags: 0x80000001,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
    }
    f
}
fn inactive(f: &mut Fixture) {
    f.document.layers[0].visible = false;
    f.document.layers[0].opacity = 0.;
    let id = f.document.layers[0].id;
    let s = f.document.xcf_state.as_mut().unwrap();
    s.items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
    for n in &mut s.filter_stacks[0].nodes {
        n.enabled = false;
    }
}
fn custody(a: &AccountedDocument, b: &AccountedDocument, index: usize) {
    assert_eq!(a.document().icc_profile, b.document().icc_profile);
    assert_eq!(a.document().source_color_context, b.document().source_color_context);
    assert_eq!(a.document().working_profile_interpretation, b.document().working_profile_interpretation);
    let x = a.document().xcf_state.as_ref().unwrap();
    let y = b.document().xcf_state.as_ref().unwrap();
    let removed = &x.filter_stacks[0].nodes[index];
    let ItemRef::Mask(mask) = removed.mask.unwrap() else { panic!("mask") };
    assert_eq!(y.filter_stacks[0].revision, x.filter_stacks[0].revision + 1);
    assert_eq!(y.filter_stacks[0].nodes, x.filter_stacks[0].nodes.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, n)| n.clone()).collect::<Vec<_>>());
    assert_eq!(&x.filter_stacks[1..], &y.filter_stacks[1..]);
    assert_eq!(y.filter_masks, x.filter_masks.iter().filter(|m| m.id != mask).cloned().collect::<Vec<_>>());
    assert_eq!(y.items, x.items.iter().filter(|r| r.item != ItemRef::Mask(mask)).cloned().collect::<Vec<_>>());
    assert_eq!(x.opaque.len(), y.opaque.len());
    assert_eq!(x.provenance, y.provenance);
    for r in &x.opaque {
        let mut expected = r.clone();
        if r.owner == Some(ItemRef::Mask(mask)) {
            expected.owner = None;
            expected.identity = format!("deleted-filter-mask/{}/{}", mask.0, r.identity);
        }
        let actual = y.opaque.iter().find(|v| v.namespace == expected.namespace && v.identity == expected.identity).unwrap();
        assert_eq!(actual, &expected);
        assert!(Arc::ptr_eq(&actual.bytes, &r.bytes));
    }
    for (a, b) in a.document().layers.iter().zip(&b.document().layers) {
        let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&a.content, &b.content) else { panic!("native") };
        assert_eq!(a.descriptor(), b.descriptor());
        assert_eq!(a.default_bytes(), b.default_bytes());
        let av = a.checked_view();
        let bv = b.checked_view();
        let ap = av.pin_tile(photocraft_geom::TileCoord { tx: 0, ty: 0 }, &ctl()).unwrap();
        let bp = bv.pin_tile(photocraft_geom::TileCoord { tx: 0, ty: 0 }, &ctl()).unwrap();
        let mut aa = [0u8; 32];
        let mut bb = [0u8; 32];
        let n = a.format().bytes_per_pixel();
        for yy in 0..6 {
            for xx in 0..8 {
                ap.read_pixel_native_into(xx, yy, &mut aa[..n], &ctl()).unwrap();
                bp.read_pixel_native_into(xx, yy, &mut bb[..n], &ctl()).unwrap();
                assert_eq!(&aa[..n], &bb[..n]);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN typed head/middle/tail survivor order with independent kernels/Normal and exact native/raw/profile/mask/archive custody; stale/unknown target refusals"]
fn survivor_order_and_custody() {
    let (nl, linear) = parts();
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        for index in 0..3 {
            let (owner, _, _, _) = adopt(fixture_records(sample));
            let old = Context::new(&owner, Default::default(), &ctl()).unwrap();
            old.validate_operative(&ctl()).unwrap();
            // The edit retains the original owner, not an unrelated prepared
            // renderer's construction-peak allocations.
            drop(old);
            let request = patch(&owner, index);
            owner.validate_readonly_filtered_flat_point_filter_remove(request, &ctl()).unwrap();
            let next = owner.try_remove_readonly_filtered_flat_point_filter(request, &ctl()).unwrap();
            custody(&owner, &next, index);
            let c = Context::new(&next, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in [0u32, 2, 4, 7] {
                let p = input(0, x, 0, sample);
                // Fixed named calls independent of renderer/canonical container traversal.
                let pre = step(0, bridge(step(1, bridge(step(2, p, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear);
                let post = match index {
                    0 => step(1, bridge(step(2, p, false, &nl, &linear)), true, &nl, &linear),
                    1 => step(0, bridge(step(2, p, false, &nl, &linear)), true, &nl, &linear),
                    _ => step(0, bridge(step(1, p, false, &nl, &linear)), true, &nl, &linear),
                };
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), post.map(f64::to_bits));
                if x == 2 {
                    assert_ne!(pre.map(f64::to_bits), post.map(f64::to_bits));
                }
                let expected = working
                    .normal(working.normal([0.; 4], post, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union)
                    .unwrap();
                assert_eq!(c.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
            }
            assert!(next.try_remove_readonly_filtered_flat_point_filter(request, &ctl()).is_err());
        }
    }
    // Historical topology refusal is superseded by explicit derived CURRENT;
    // this group keeps its original mutation references and unknown-ID refusal.
    for sample in [SampleType::F32, SampleType::F64] {
        let (owner, _, _, _) = adopt(ordered(sample, &[&[0, 1, 2]], true));
        Context::new(&owner, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).unwrap();
        let next = owner.try_remove_readonly_filtered_flat_point_filter(patch(&owner, 1), &ctl()).unwrap();
        let c = Context::new(&next, Default::default(), &ctl()).unwrap();
        c.validate_operative(&ctl()).unwrap();
        let expected = step(0, bridge(step(2, input(0, 2, 0, sample), false, &nl, &linear)), true, &nl, &linear);
        assert_eq!(root_value(&c, 0, 2, 0).map(f64::to_bits), expected.map(f64::to_bits));
        let mut invalid = patch(&next, 0);
        invalid.filter = FilterId(u64::MAX);
        assert!(next.try_remove_readonly_filtered_flat_point_filter(invalid, &ctl()).is_err());
    }
    let (owner, _, _, _) = adopt(ordered(SampleType::F64, &[&[0], &[]], true));
    let mut invalid = patch(&owner, 0);
    invalid.filter = FilterId(u64::MAX);
    assert!(owner.try_remove_readonly_filtered_flat_point_filter(invalid, &ctl()).is_err());
    let (owner, _, _, _) = adopt(ordered(SampleType::F64, &[&[0, 1]], true));
    let mut invalid = patch(&owner, 0);
    invalid.filter = FilterId(u64::MAX);
    assert!(owner.try_remove_readonly_filtered_flat_point_filter(invalid, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN full OLD hidden/zero/disabled physicalmask/profile/native proof remains required before structural removal; late Work/cancel and provider refund"]
fn old_inactive_proof_and_staged_failure_controls() {
    for fault in 0..2 {
        let mut f = fixture_records(SampleType::F64);
        inactive(&mut f);
        if fault == 0 {
            f.document.xcf_state.as_mut().unwrap().filter_masks[1]
                .surface
                .try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes())
                .unwrap();
        } else {
            f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
        }
        let (owner, _, _, _) = adopt(f);
        assert!(Context::new(&owner, Default::default(), &ctl()).is_err());
        if fault == 0 {
            // L1 is structural only. A clean staged result cannot replace OLD proof.
            let structural = owner.try_remove_readonly_filtered_flat_point_filter(patch(&owner, 1), &ctl()).unwrap();
            Context::new(&structural, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).unwrap();
            // Deliberately no engine publication: engine must reject failed OLD proof.
        }
    }
    let mut f = fixture_records(SampleType::F64);
    inactive(&mut f);
    let (owner, rasters, fails, _) = adopt(f);
    let old = Context::new(&owner, Default::default(), &ctl()).unwrap();
    old.validate_operative(&ctl()).unwrap();
    let next = owner.try_remove_readonly_filtered_flat_point_filter(patch(&owner, 1), &ctl()).unwrap();
    let c = Context::new(&next, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    assert_eq!(root_value(&c, 0, 4, 0).map(f64::to_bits), input(0, 4, 0, SampleType::F64).map(f64::to_bits));
    // Genuine native cache is warm before operation-only exact refund baselines.
    let q = next.quota();
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
    let interrupt = Interrupt::cancel_only(&count);
    c.validate_operative(&ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).unwrap();
    let total = polls.load(Ordering::Relaxed);
    assert!(total > 1);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= total - 1;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert!(matches!(c.validate_operative(&ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    rasters[0].clear_cache().unwrap();
    fails[0].store(true, Ordering::Relaxed);
    let baseline = q.usage().unwrap();
    assert!(old.validate_operative(&ctl()).is_err());
    assert!(c.render_band(Rect::new(0, 0, 1, 1), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    fails[0].store(false, Ordering::Relaxed);
}
