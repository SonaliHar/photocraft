//! UNRUN CURRENT-only disabled mechanics; no newly admitted original source
//! is asserted. Existing legacy skipped-stage output is an independent oracle.
use super::*;
fn disabled(kind: u8) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, &'static Profile, Surface) {
    let (mut doc, q, guard, profile, source) = model(kind);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    (doc, q, guard, profile, source)
}
#[test]
fn all_eight_disabled_current_recipes_are_native_wide_identity_without_halo_or_quantization() {
    for kind in 0..8 {
        let (doc, q, guard, profile, source) = disabled(kind);
        let owner = adopt(doc, q, guard);
        let bounds = owner.document().bounds();
        let current = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
        current.validate_operative(&ctl()).unwrap();
        assert_eq!(current.bounds(), bounds);
        assert!(!current.prepared.plan.stack.has_neighborhood());
        let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
        let legacy =
            OrderedStack::new_with_interrupt(stack, profile, SampleEncoding::NonlinearProfile, 22, bounds, [true, false, false], &Interrupt::NONE).unwrap();
        assert!(!legacy.has_neighborhood());
        let expected = legacy.evaluate(&source, bounds, &mut |_| Ok(()), &Interrupt::NONE).unwrap();
        let output = current.render_wide_band(bounds, &ctl()).unwrap();
        let mut index = 0;
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let mut decoded = [0.; 5];
                source.try_read_pixel_f64(x, y, &mut decoded[..source.channels()]).unwrap();
                let native = if owner.document().mode == ColorMode::Grayscale {
                    [decoded[0], decoded[0], decoded[0], decoded[1]]
                } else {
                    [decoded[0], decoded[1], decoded[2], decoded[3]]
                };
                let mut old = [0.; 4];
                expected.try_read_pixel_f64(x, y, &mut old).unwrap();
                assert_eq!(output.pixels()[index].map(f64::to_bits), native.map(f64::to_bits));
                assert_eq!(output.pixels()[index].map(f64::to_bits), old.map(f64::to_bits));
                index += 1;
            }
        }
        if kind >= 4 {
            assert!(output.pixels().iter().any(|pixel| pixel[0] < 0.0));
        }
    }
}
#[test]
fn disabled_does_not_erase_typed_parameter_tuple_profile_or_native_requirements() {
    for kind in 0..8 {
        for case in 0..5 {
            let (mut doc, q, _guard, profile, _) = disabled(kind);
            match case {
                0 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].name = "wrong".into(),
                1 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0,
                2 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
                3 => doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                _ => doc.source_color_context.as_mut().unwrap().source_precision = 999,
            }
            let before = q.usage().unwrap();
            assert!(prepare(&doc, Some(profile), 512, &Interrupt::NONE, &mut NativeBudget { q: &q, c: &ctl() }).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
    }
    // Real paid invalid native alpha is still rejected before identity output.
    let (mut doc, q, guard, profile, _) = disabled(4);
    let format = doc.pixel_format();
    let bounds = doc.bounds();
    let mut pixel = [0; 40];
    photocraft_color::try_write_sample_f64(&mut pixel, format.sample, format.mode.color_channels(), 2.0).unwrap();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 40][..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, std::mem::size_of::<Store>() as u64 + 128).unwrap();
    let store = Arc::new(Store { descriptor, pixel, q: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    let owner = adopt(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).is_err());
    // Legacy metadata callers retain enabled-only behavior, including each op.
    for kind in 0..8 {
        let (doc, _, _, _, _) = disabled(kind);
        let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(crate::gimp_neighborhood::legacy_disabled_neighborhood_parameters_refuse(node, doc.bounds()));
    }
}
#[test]
fn disabled_identity_retention_is_paid_and_cancellation_work_refusal_never_mutate_owner() {
    let (doc, q, guard, profile, _) = disabled(2);
    let owner = adopt(doc, q.clone(), guard);
    {
        let warm = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
        warm.validate_operative(&ctl()).unwrap();
    }
    let before = q.usage().unwrap();
    {
        let current = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > before.resident);
        let held = q.usage().unwrap();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(current.render_wide_band(current.bounds(), &denied), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), held);
    }
    assert_eq!(q.usage().unwrap(), before);
    let stop = Interrupt::cancel_only(&|| true);
    let control = ReadControl { interrupt: &stop, work: &|_| Ok(()) };
    assert!(matches!(PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &control), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), before);
    assert!(!owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
}
