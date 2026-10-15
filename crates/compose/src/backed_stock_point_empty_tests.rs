//! UNRUN synthetic CURRENT empty lifecycle. No original empty-source proof.
use super::tests::{adopt, ctl, fixture};
use super::*;
use photocraft_doc::import_compositing::GimpSpace;
fn empty() -> (Document, BackingQuota, std::sync::Arc<dyn std::any::Any + Send + Sync>, &'static Profile, Vec<u8>) {
    let (mut doc, q, guard, _, profile, raw) = fixture(2, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
    doc.layers[0].import_compositing.as_mut().unwrap().composite_encoding = SampleEncoding::NonlinearProfile;
    let layer = doc.layers[0].id;
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_stacks[0].nodes.clear();
    state.filter_stacks[0].revision = 7;
    state.filter_masks.clear();
    state.items.retain(|item| item.item == ItemRef::Layer(layer));
    let policy = state.items[0].composition.as_mut().unwrap();
    policy.stored_composite_space = 2;
    policy.stored_operator = 1;
    policy.resolved.as_mut().unwrap().composite_space = GimpSpace::ProfileRgb;
    (doc, q, guard, profile, raw)
}
#[test]
fn empty_current_band_is_exact_native_identity_and_matches_independent_legacy_empty_stack() {
    let (doc, q, guard, profile, raw) = empty();
    let owner = adopt(doc, q, guard);
    let current = PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    assert_eq!(current.node_id(), None);
    assert_eq!(current.node_ids().count(), 0);
    current.validate_operative(&ctl()).unwrap();
    let doc = owner.document();
    let legacy = OrderedStack::new_with_interrupt(
        &doc.xcf_state.as_ref().unwrap().filter_stacks[0],
        profile,
        SampleEncoding::NonlinearProfile,
        22,
        doc.bounds(),
        [false; 3],
        &Interrupt::NONE,
    )
    .unwrap();
    let output = current.render_wide_band(doc.bounds(), &ctl()).unwrap();
    for (actual, bytes) in output.pixels().iter().zip(raw.chunks_exact(4)) {
        let input = std::array::from_fn(|c| try_read_sample_f64(bytes, SampleType::U8, c).unwrap());
        assert_eq!(actual.map(f64::to_bits), input.map(f64::to_bits));
        assert_eq!(actual.map(f64::to_bits), legacy.point_at(input, (0, 0), &mut |_| Ok(())).unwrap().map(f64::to_bits));
    }
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
}
#[test]
fn empty_zero_stage_owns_no_working_heap_and_checks_input_without_cms_or_integer_rounding() {
    let stack = OrderedStack::empty_for_stock(Rect::new(0, 0, 8, 6), &Interrupt::NONE).unwrap();
    assert_eq!(stack.stock_retained_bytes().unwrap(), 0);
    let input = [1.0 / 6.0, 0.123456789012345, 0.75, 0.5];
    let mut units = 0;
    let mut poll = |event| {
        match event {
            WideWork::SampleUnits(n) => units += n,
            _ => panic!("identity must not reserve CMS storage"),
        };
        Ok(())
    };
    stack.validate_stock_input((0, 0), input, &Interrupt::NONE, &mut poll).unwrap();
    assert_eq!(stack.point_at(input, (0, 0), &mut poll).unwrap().map(f64::to_bits), input.map(f64::to_bits));
    assert_eq!(units, 4);
    for value in [f64::NAN, f64::INFINITY, -0.001, 1.001] {
        assert!(stack.validate_stock_input((0, 0), [value, 0.0, 0.0, 1.0], &Interrupt::NONE, &mut |_| Ok(())).is_err());
    }
}
#[test]
fn empty_current_refuses_wrong_precision_profile_tuple_shape_and_preserves_quota_on_failure() {
    for case in 0..5 {
        let (mut doc, q, _guard, profile, _) = empty();
        match case {
            0 => doc.xcf_state.as_mut().unwrap().current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Half,
            1 => doc.source_color_context.as_mut().unwrap().source_precision = 100,
            2 => doc.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().stored_operator = 0,
            3 => doc.layers[0].opacity = 0.5,
            _ => doc.working_profile_interpretation = WorkingProfileInterpretation::Icc,
        }
        let before = q.usage().unwrap();
        assert!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let (doc, q, _guard, profile, _) = empty();
    let before = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &denied), Err(BackedNormalError::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), before);
    let stopped = Interrupt::cancel_only(&|| true);
    assert!(matches!(
        PreparedStockPointPipeline::prepare(&doc, profile, &q, &ReadControl { interrupt: &stopped, work: &|_| Ok(()) }),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), before);
}
