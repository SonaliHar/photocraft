//! Structural CURRENT routing only. The inherited paid fixture is not a
//! Source/profile/evaluator witness; genuine transport remains the IO six-case test.
use super::*;
fn ordinary(sample: SampleType) -> AccountedDocument {
    let source = fixture(sample);
    let q = source.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = source.document().clone(); // paid external fixture construction
    for item in &mut doc.xcf_state.as_mut().unwrap().items {
        if let Some(policy) = &mut item.composition {
            policy.compatibility_epoch = XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH.into();
        }
    }
    AccountedDocument::from_owned(doc, Arc::new((source, fee)), q, &ctl()).unwrap()
}
#[test]
fn ordinary_metadata_uses_real_legacy_census_and_preserves_native_mask_source() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let source = ordinary(sample);
        let id = source.document().layers[1].id;
        let renamed = source.try_edit(AccountedEdit::LayerName { layer: id, name: "ordinary current name" }, &ctl()).unwrap();
        assert!(renamed.is_legacy_storage());
        assert_eq!(renamed.document().layers[1].name, "ordinary current name");
        assert_eq!(renamed.document().xcf_state.as_ref().unwrap().provenance, source.document().xcf_state.as_ref().unwrap().provenance);
        assert_eq!(renamed.document().xcf_state.as_ref().unwrap().opaque, source.document().xcf_state.as_ref().unwrap().opaque);
        assert_eq!(renamed.document().source_color_context, source.document().source_color_context);
        assert_eq!(renamed.document().icc_profile, source.document().icc_profile);
        assert!(Arc::ptr_eq(&renamed.document().layers[1].mask.as_ref().unwrap().surface, &source.document().layers[1].mask.as_ref().unwrap().surface));
        for (before, after) in source.document().layers.iter().zip(&renamed.document().layers) {
            let (LayerContent::BackedRaster(before), LayerContent::BackedRaster(after)) = (&before.content, &after.content) else { panic!("native fixture") };
            assert_eq!(before, after);
        }
        // Current metadata remains usable after genuine canonical synchronization.
        let hidden = renamed.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &ctl()).unwrap();
        assert!(!hidden.document().layers[1].visible);
        let faded = hidden.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.375 }, &ctl()).unwrap();
        assert_eq!(faded.document().layers[1].opacity.to_bits(), 0.375f32.to_bits());
        let item = faded.document().xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(id)).unwrap();
        assert_eq!(item.name, "ordinary current name");
        assert!(!item.visible);
        assert_eq!(source.document().layers[1].name, "layer1");
    }
}
#[test]
fn assigned_unknown_and_incomplete_ordinary_contexts_keep_final_refusals() {
    let assigned = fixture(SampleType::F32);
    let id = assigned.document().layers[1].id;
    assert!(assigned.try_edit(AccountedEdit::LayerName { layer: id, name: "not the assigned endpoint" }, &ctl()).is_err());
    for case in 0..5 {
        let source = ordinary(SampleType::F32);
        let q = source.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = source.document().clone(); // paid external hostile fixture
        let state = doc.xcf_state.as_mut().unwrap();
        match case {
            0 => state.items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().compatibility_epoch = "unknown".into(),
            1 => state.items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().compatibility_epoch = XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH.into(),
            2 => state.items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap().offset[0] += 1,
            3 => state.items.push(state.items[0].clone()),
            _ => state.items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().resolved.as_mut().unwrap().operator = GimpOperator::Intersection,
        }
        let invalid = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q.clone(), &ctl()).unwrap();
        let id = invalid.document().layers[1].id;
        let before = invalid.document().clone(); // external expected value
        let baseline = q.usage().unwrap();
        assert!(invalid.try_edit(AccountedEdit::LayerName { layer: id, name: "must remain refused" }, &ctl()).is_err());
        assert_eq!(invalid.document(), &before);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
fn ordinary_metadata_raw_noop_and_late_work_failure_retain_owner_and_fees() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let source = ordinary(SampleType::F64);
    let id = source.document().layers[1].id;
    let name = source.document().layers[1].name.as_str();
    let baseline = source.quota().usage().unwrap();
    assert!(source.try_edit(AccountedEdit::LayerName { layer: id, name }, &ctl()).unwrap().shares_model(&source));
    let spent = AtomicU64::new(0);
    let count = |units| {
        spent.fetch_add(units, Ordering::SeqCst);
        Ok(())
    };
    let changed = source.try_edit(AccountedEdit::LayerName { layer: id, name: "changed" }, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(changed);
    let limit = spent.load(Ordering::SeqCst).checked_sub(1).unwrap();
    spent.store(0, Ordering::SeqCst);
    let late = |units| {
        let prior = spent.fetch_add(units, Ordering::SeqCst);
        if prior.checked_add(units).is_none_or(|total| total > limit) { Err(BackingError::Work) } else { Ok(()) }
    };
    assert!(matches!(
        source.try_edit(AccountedEdit::LayerName { layer: id, name: "changed" }, &ReadControl { interrupt: &Interrupt::NONE, work: &late }),
        Err(BackingError::Work)
    ));
    assert_eq!(source.document().layers[1].name, "layer1");
    assert_eq!(source.quota().usage().unwrap(), baseline);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(
        source.try_edit(AccountedEdit::LayerName { layer: id, name }, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackingError::Cancelled)
    ));
}
