//! UNRUN typed L1 moves over actual paid mixed owners; declared math only.
use super::*;
use photocraft_doc::accounted_document::PointFilterMovePatch;
fn request(owner: &AccountedDocument, from: usize, to: usize) -> PointFilterMovePatch {
    let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    PointFilterMovePatch { layer: owner.document().layers[0].id, filter: stack.nodes[from].id, expected_stack_revision: stack.revision, index: to }
}
fn payload(old: &AccountedDocument, new: &AccountedDocument) {
    let a = old.document();
    let b = new.document();
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    let x = a.xcf_state.as_ref().unwrap();
    let y = b.xcf_state.as_ref().unwrap();
    assert_eq!(x.items, y.items);
    assert_eq!(x.filter_masks, y.filter_masks);
    assert_eq!(&x.filter_stacks[1..], &y.filter_stacks[1..]);
    let before = &x.filter_stacks[0];
    let after = &y.filter_stacks[0];
    assert_eq!(after.revision, before.revision + 1);
    for node in &before.nodes {
        assert_eq!(after.nodes.iter().find(|n| n.id == node.id).unwrap(), node);
    }
    for (a, b) in a.layers.iter().zip(&b.layers) {
        let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&a.content, &b.content) else { panic!("native") };
        assert_eq!(a.descriptor(), b.descriptor());
        assert_eq!(a.default_bytes(), b.default_bytes());
        // Actual checked samples, including F64 tiny/near-one words, remain native.
        let av = a.checked_view();
        let bv = b.checked_view();
        let ap = av.pin_tile(photocraft_geom::TileCoord { tx: 0, ty: 0 }, &ctl()).unwrap();
        let bp = bv.pin_tile(photocraft_geom::TileCoord { tx: 0, ty: 0 }, &ctl()).unwrap();
        let mut aa = [0u8; 32];
        let mut bb = [0u8; 32];
        let n = a.format().bytes_per_pixel();
        for x in [0, 2, 4, 7] {
            ap.read_pixel_native_into(x, 0, &mut aa[..n], &ctl()).unwrap();
            bp.read_pixel_native_into(x, 0, &mut bb[..n], &ctl()).unwrap();
            assert_eq!(&aa[..n], &bb[..n]);
        }
    }
}
#[test]
#[ignore = "UNRUN L1 actual stable-ID move preserves native/raw/profile/masks; independent noncommuting pre/post named kernels and Normal references; no producer equivalence"]
fn typed_move_changes_only_canonical_order_and_declared_composition() {
    let (nl, linear) = parts();
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        let (owner, _, _, _) = adopt(ordered(sample, &[&[0, 1, 2], &[]], true));
        let old = Context::new(&owner, Default::default(), &ctl()).unwrap();
        old.validate_operative(&ctl()).unwrap();
        // Expected old pixels are external oracle values; the original model
        // stays retained while its independent renderer is dropped before edit.
        let old_pixels = [0, 2, 4, 7].map(|x| root_value(&old, 0, x, 0));
        drop(old);
        let patch = request(&owner, 0, 2);
        owner.validate_readonly_filtered_flat_point_filter_move(patch, &ctl()).unwrap();
        let moved = owner.try_move_readonly_filtered_flat_point_filter(patch, &ctl()).unwrap();
        payload(&owner, &moved);
        let before = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
        let after = &moved.document().xcf_state.as_ref().unwrap().filter_stacks[0];
        assert_eq!(after.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![before.nodes[1].id, before.nodes[2].id, before.nodes[0].id]);
        let new = Context::new(&moved, Default::default(), &ctl()).unwrap();
        new.validate_operative(&ctl()).unwrap();
        for (at, x) in [0u32, 2, 4, 7].into_iter().enumerate() {
            let p = input(0, x, 0, sample);
            // Canonical [Semi,Hue,Vib] -> [Hue,Vib,Semi]. Fixed named calls
            // deliberately do not traverse either implementation container.
            let pre = step(0, bridge(step(1, bridge(step(2, p, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear);
            let post = step(1, bridge(step(2, bridge(step(0, p, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear);
            assert_eq!(old_pixels[at].map(f64::to_bits), pre.map(f64::to_bits));
            assert_eq!(root_value(&new, 0, x as i32, 0).map(f64::to_bits), post.map(f64::to_bits));
            if x == 2 {
                assert_ne!(pre.map(f64::to_bits), post.map(f64::to_bits));
            }
            let expected =
                working.normal(working.normal([0.; 4], post, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union).unwrap();
            assert_eq!(new.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
        }
        drop(new);
        let same = moved.try_move_readonly_filtered_flat_point_filter(request(&moved, 2, 2), &ctl()).unwrap();
        assert!(same.shares_model(&moved));
        assert!(moved.try_move_readonly_filtered_flat_point_filter(patch, &ctl()).is_err());
        let back = moved.try_move_readonly_filtered_flat_point_filter(request(&moved, 2, 0), &ctl()).unwrap();
        assert_eq!(back.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes, before.nodes);
        let restored = Context::new(&back, Default::default(), &ctl()).unwrap();
        assert_eq!(root_value(&restored, 0, 2, 0).map(f64::to_bits), old_pixels[1].map(f64::to_bits));
    }
}
#[test]
#[ignore = "UNRUN reordered hidden/zero-opacity/all-disabled full native/mask/profile proof, cumulative late Work/cancel and real same-ledger refund; no no-op bypass"]
fn reordered_inactive_recipe_keeps_full_proof_and_control() {
    let mut f = ordered(SampleType::F64, &[&[0, 1, 2], &[]], false);
    f.document.layers[0].visible = false;
    f.document.layers[0].opacity = 0.;
    let id = f.document.layers[0].id;
    f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
    let (owner, rasters, fails, _) = adopt(f);
    let moved = owner.try_move_readonly_filtered_flat_point_filter(request(&owner, 0, 2), &ctl()).unwrap();
    payload(&owner, &moved);
    let prepared = Context::new(&moved, Default::default(), &ctl()).unwrap();
    prepared.validate_operative(&ctl()).unwrap();
    assert_eq!(root_value(&prepared, 0, 4, 0).map(f64::to_bits), input(0, 4, 0, SampleType::F64).map(f64::to_bits));
    // Warm native cache before exact operation-only refund baselines.
    let q = moved.quota();
    let baseline = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    prepared.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    let total = used.load(Ordering::Relaxed);
    assert!(total > 1);
    let used = AtomicU64::new(0);
    let deny = |n| {
        if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) >= total { Err(BackingError::Work) } else { Ok(()) }
    };
    assert!(matches!(prepared.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let polls = AtomicU64::new(0);
    let count = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let interrupt = Interrupt::cancel_only(&count);
    prepared.validate_operative(&ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).unwrap();
    let total = polls.load(Ordering::Relaxed);
    assert!(total > 1);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= total - 1;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert!(matches!(prepared.validate_operative(&ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    drop(prepared);
    rasters[0].clear_cache().unwrap();
    fails[0].store(true, Ordering::Relaxed);
    assert!(Context::new(&moved, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).is_err());
    fails[0].store(false, Ordering::Relaxed);
    // Mutations remain structural: full recipe consumers reject a hidden,
    // disabled malformed physical mask even after a real typed reorder.
    let mut f = ordered(SampleType::F64, &[&[0, 1, 2], &[]], false);
    f.document.layers[0].visible = false;
    f.document.layers[0].opacity = 0.;
    let id = f.document.layers[0].id;
    f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
    f.document.xcf_state.as_mut().unwrap().filter_masks[1].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes()).unwrap();
    let (owner, _, _, _) = adopt(f);
    let moved = owner.try_move_readonly_filtered_flat_point_filter(request(&owner, 0, 2), &ctl()).unwrap();
    assert!(Context::new(&moved, Default::default(), &ctl()).is_err());
}
