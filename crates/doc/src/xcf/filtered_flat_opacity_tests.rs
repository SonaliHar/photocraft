//! TEMP installation candidate: one ignored UNRUN structural data-only group.
//! No Source, L3, engine/editor, filter-compositing or numerical grant.
use super::*;
#[test]
#[ignore = "UNRUN: compound raw args/enabled/F64opacity one revision/copy, signedzero exactness/noop/MAX and failure fences preserve all source/native/profile/mask custody"]
fn compound_opacity_words_and_atomic_custody() {
    for sample in [SampleType::F32, SampleType::F64] {
        let (mut d, g, q) = raw_remove(sample, &[2, 3]);
        let state = d.xcf_state.as_mut().unwrap();
        let layer = d.layers[1].id;
        let fid = state.filter_stacks[1].nodes[1].id;
        let enabled = !state.filter_stacks[1].nodes[1].enabled;
        // Existing prospective fixture preholds cover this synthetic inert source
        // property. It is not a newly captured producer opacity witness.
        let bytes = Arc::new(1f32.to_be_bytes().to_vec());
        state.opaque.push(OpaqueRecord {
            owner: Some(ItemRef::Layer(layer)),
            namespace: "gimp/xcf/source-property".into(),
            identity: "synthetic/filter-opacity/33".into(),
            source_version: 22,
            order: 33,
            flags: 0,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
        assert!(measured(&d, &q, &ctl()).unwrap().index <= 16 << 20);
        let owner = AccountedDocument::new_readonly_filtered_flat_point_current_storage(d, g, q, &ctl()).unwrap();
        let args = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.625f32.to_bits()) }];
        let patch = PointFilterPatch { layer, filter: fid, expected_stack_revision: 17, arguments: &args, enabled: Some(enabled) };
        owner.validate_readonly_filtered_flat_point_filter_with_opacity(patch, Some(0.375f64.to_bits()), &ctl()).unwrap();
        let changed = owner.try_with_readonly_filtered_flat_point_filter_with_opacity(patch, Some(0.375f64.to_bits()), &ctl()).unwrap();
        let mut expected = owner.document().clone(); // external expected-model harness
        let s = expected.xcf_state.as_mut().unwrap();
        s.filter_stacks[1].revision = 18;
        let n = &mut s.filter_stacks[1].nodes[1];
        n.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.625f32.to_bits()));
        n.enabled = enabled;
        n.opacity_f64_bits = 0.375f64.to_bits();
        assert_eq!(changed.document(), &expected);
        assert!(Arc::ptr_eq(&changed.owner.retention, &owner.owner.retention));
        assert!(Arc::ptr_eq(changed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
        assert_eq!(changed.document().xcf_state.as_ref().unwrap().opaque, owner.document().xcf_state.as_ref().unwrap().opaque);
        let before = changed.quota().usage().unwrap();
        let blocker = changed.quota().reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
        let same = PointFilterPatch { expected_stack_revision: 18, ..patch };
        assert!(changed.try_with_readonly_filtered_flat_point_filter_with_opacity(same, Some(0.375f64.to_bits()), &ctl()).unwrap().shares_model(&changed));
        drop(blocker);
        assert_eq!(changed.quota().usage().unwrap(), before);
        let empty = PointFilterPatch { layer, filter: fid, expected_stack_revision: 18, arguments: &[], enabled: None };
        let plus = changed.try_with_readonly_filtered_flat_point_filter_with_opacity(empty, Some(0f64.to_bits()), &ctl()).unwrap();
        let minus = plus
            .try_with_readonly_filtered_flat_point_filter_with_opacity(
                PointFilterPatch { expected_stack_revision: 19, ..empty },
                Some((-0f64).to_bits()),
                &ctl(),
            )
            .unwrap();
        let s = minus.document().xcf_state.as_ref().unwrap();
        assert_eq!(s.filter_stacks[1].revision, 20);
        assert_eq!(s.filter_stacks[1].nodes[1].opacity_f64_bits, (-0f64).to_bits());
        // The old API's omitted opacity retains its exact stored word, and its
        // None delegation has precisely the same charged-work schedule.
        let old_patch = PointFilterPatch { expected_stack_revision: 20, enabled: Some(!enabled), ..empty };
        let old_work = AtomicU64::new(0);
        let count = |n| {
            old_work.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let old = minus.try_with_readonly_filtered_flat_point_filter(old_patch, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
        assert_eq!(old.document().xcf_state.as_ref().unwrap().filter_stacks[1].nodes[1].opacity_f64_bits, (-0f64).to_bits());
        drop(old);
        let new_work = AtomicU64::new(0);
        let count = |n| {
            new_work.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let copy = minus
            .try_with_readonly_filtered_flat_point_filter_with_opacity(old_patch, None, &ReadControl { interrupt: &Interrupt::NONE, work: &count })
            .unwrap();
        drop(copy);
        assert_eq!(old_work.load(Ordering::Relaxed), new_work.load(Ordering::Relaxed));
        // Failures use the original target/model and its one cumulative ledger.
        let before = owner.quota().usage().unwrap();
        for bits in [f64::NAN.to_bits(), f64::INFINITY.to_bits(), (-0.001f64).to_bits(), 1.001f64.to_bits()] {
            assert!(owner.try_with_readonly_filtered_flat_point_filter_with_opacity(patch, Some(bits), &ctl()).is_err());
        }
        let blocker = owner.quota().reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
        assert!(owner.try_with_readonly_filtered_flat_point_filter_with_opacity(patch, Some(0.375f64.to_bits()), &ctl()).is_err());
        drop(blocker);
        assert_eq!(owner.quota().usage().unwrap(), before);
        let total = AtomicU64::new(0);
        let work = |n| {
            total.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let tmp = owner
            .try_with_readonly_filtered_flat_point_filter_with_opacity(
                patch,
                Some(0.375f64.to_bits()),
                &ReadControl { interrupt: &Interrupt::NONE, work: &work },
            )
            .unwrap();
        drop(tmp);
        assert_eq!(owner.quota().usage().unwrap(), before);
        let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
        let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
        assert!(
            owner
                .try_with_readonly_filtered_flat_point_filter_with_opacity(
                    patch,
                    Some(0.375f64.to_bits()),
                    &ReadControl { interrupt: &Interrupt::NONE, work: &work }
                )
                .is_err()
        );
        assert_eq!(owner.quota().usage().unwrap(), before);
        let work = |_| if owner.quota().usage().map_err(|_| BackingError::Quota)?.index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(
            owner
                .try_with_readonly_filtered_flat_point_filter_with_opacity(
                    patch,
                    Some(0.375f64.to_bits()),
                    &ReadControl { interrupt: &Interrupt::NONE, work: &work }
                )
                .is_err()
        );
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
    let (mut d, g, q) = raw_remove(SampleType::F64, &[2, 3]);
    let s = d.xcf_state.as_mut().unwrap();
    s.filter_stacks[1].revision = u64::MAX;
    s.filter_stacks[1].nodes[1].opacity_f64_bits = (-0f64).to_bits();
    let owner = AccountedDocument::new_readonly_filtered_flat_point_current_storage(d, g, q, &ctl()).unwrap();
    let s = owner.document().xcf_state.as_ref().unwrap();
    let p = PointFilterPatch {
        layer: owner.document().layers[1].id,
        filter: s.filter_stacks[1].nodes[1].id,
        expected_stack_revision: u64::MAX,
        arguments: &[],
        enabled: None,
    };
    assert!(owner.try_with_readonly_filtered_flat_point_filter_with_opacity(p, Some((-0f64).to_bits()), &ctl()).unwrap().shares_model(&owner));
    assert!(matches!(owner.try_with_readonly_filtered_flat_point_filter_with_opacity(p, Some(0f64.to_bits()), &ctl()), Err(BackingError::Length)));
    let mut wrong = p;
    wrong.filter = s.filter_stacks[0].nodes[0].id;
    assert!(owner.try_with_readonly_filtered_flat_point_filter_with_opacity(wrong, None, &ctl()).is_err());
    wrong = p;
    wrong.expected_stack_revision -= 1;
    assert!(owner.validate_readonly_filtered_flat_point_filter_with_opacity(wrong, Some((-0f64).to_bits()), &ctl()).is_err());
}
