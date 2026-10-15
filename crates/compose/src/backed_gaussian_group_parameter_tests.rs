//! UNRUN CURRENT-only parameter mechanics. External reference buffers are test
//! harness allocations. These tests do not enroll the new GIMP opaque tuple or
//! claim numerical acceptance of the continuous domain against GIMP.
use super::*;
use photocraft_algo::gimp::neighborhood::{self as n, GaussianAbyss, GaussianFilter, GaussianLimits, GaussianParams};

fn set_axes(doc: &mut Document, x: u32, y: u32) {
    for argument in &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments {
        let bits = match argument.name.as_str() {
            "std-dev-x" => x,
            "std-dev-y" => y,
            _ => continue,
        };
        argument.value = FilterValue::Number(ExactNumber::Float32Bits(bits));
    }
}
fn parameter_owner(x: u32, y: u32, enabled: bool) -> AccountedDocument {
    let q = quota();
    let (mut doc, guard) = fixture(&q, false, enabled);
    set_axes(&mut doc, x, y);
    AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q, &ctl()).unwrap()
}
fn stage(doc: &Document, current: bool) -> Result<gaussian::GroupGaussian, CompositionError> {
    let layer = &doc.layers[1];
    let state = doc.xcf_state.as_ref().unwrap();
    let context = (bounds(doc, layer).unwrap(), Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile);
    if current {
        gaussian::GroupGaussian::new_current_owned(doc, layer, state, &state.filter_stacks[0], context, &Interrupt::NONE)
    } else {
        gaussian::GroupGaussian::new_owned(doc, layer, state, &state.filter_stacks[0], context, &Interrupt::NONE)
    }
}
fn cases() -> Vec<(f32, f32)> {
    vec![
        (0.8, 1.3),
        (0., 1.3),
        (0.8, 0.),
        (0., 0.),
        (-0., 0.),
        (f32::from_bits(0x3f7fffff), 1.3),
        (1., 1.3),
        (f32::from_bits(0x3f800001), 1.3),
        (0.8, f32::from_bits(0x3f7fffff)),
        (0.8, 1.),
        (0.8, f32::from_bits(0x3f800001)),
        (1.3, 0.8),
        (15., 1.3),
        (0.8, 15.),
        (15., 15.),
        (2.375, 7.625),
    ]
}

#[test]
#[ignore = "UNRUN: exact retained CURRENT axes drive the existing general finite Auto kernel and independent inverse transfer; broader GIMP numerical acceptance pending"]
fn retained_axes_match_existing_lower_kernel_reference() {
    for (x, y) in cases() {
        let owner = parameter_owner(x.to_bits(), y.to_bits(), true);
        let prepared = stage(owner.document(), true).unwrap();
        let params = prepared.parameters();
        assert_eq!(params.std_dev_x.to_bits(), f64::from(x).to_bits());
        assert_eq!(params.std_dev_y.to_bits(), f64::from(y).to_bits());
        let rect = prepared.extent;
        let working = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let mut input = Surface::new(PixelFormat::RGBA64F);
        for py in rect.y0..rect.y1 {
            for px in rect.x0..rect.x1 {
                let v = ((px - rect.x0 + 3 * (py - rect.y0)) % 11) as f64 / 10.;
                let rgb = working.convert([v, 0.25, 0.75], true).unwrap();
                input.try_write_pixel_f64(px, py, &[rgb[0], rgb[1], rgb[2], 0.5 + v * 0.25]).unwrap();
            }
        }
        // Literal request values, not prepared.parameters(), define the oracle.
        let expected_params = GaussianParams { std_dev_x: f64::from(x), std_dev_y: f64::from(y), abyss: GaussianAbyss::Clamp, clip_extent: true };
        let req = n::gaussian_finite_requirements(&input, rect, rect, expected_params, GaussianFilter::Auto, &Interrupt::NONE).unwrap();
        let limits = GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units };
        let filtered = n::try_gaussian_finite_clamp(&input, rect, rect, expected_params, GaussianFilter::Auto, limits, &Interrupt::NONE).unwrap();
        let actual = prepared.evaluate_owned(&input, &mut |_| Ok(()), &Interrupt::NONE, limits).unwrap();
        for py in rect.y0..rect.y1 {
            for px in rect.x0..rect.x1 {
                let mut p = [0.; 4];
                let mut a = [0.; 4];
                filtered.try_read_pixel_f64(px, py, &mut p).unwrap();
                actual.try_read_pixel_f64(px, py, &mut a).unwrap();
                let rgb = working.convert([p[0], p[1], p[2]], false).unwrap();
                assert_eq!(a.map(f64::to_bits), [rgb[0], rgb[1], rgb[2], p[3]].map(f64::to_bits));
            }
        }
    }
}

#[test]
#[ignore = "UNRUN: CURRENT continuous axes do not widen legacy/source constructors; hostile axes/types/duplicates remain refused"]
fn current_policy_is_separate_and_bounded() {
    for (x, y) in cases() {
        let owner = parameter_owner(x.to_bits(), y.to_bits(), true);
        let doc = owner.document();
        assert!(stage(doc, true).is_ok());
        let original = x.to_bits() == 0.8f32.to_bits() && y.to_bits() == 1.3f32.to_bits();
        assert_eq!(stage(doc, false).is_ok(), original);
        let layer = &doc.layers[1];
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(
            gaussian::GroupGaussian::new(
                doc,
                layer,
                state,
                &state.filter_stacks[0],
                (bounds(doc, layer).unwrap(), Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile),
                &Interrupt::NONE
            )
            .is_ok(),
            original
        );
    }
    for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.001f32).to_bits(), 15.000001f32.to_bits()] {
        let q = quota();
        let (mut doc, _guard) = fixture(&q, false, false);
        set_axes(&mut doc, bits, 0.8f32.to_bits());
        assert!(stage(&doc, true).is_err());
    }
    for duplicate in [false, true] {
        let q = quota();
        let (mut doc, _guard) = fixture(&q, false, false);
        let args = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments;
        if duplicate {
            args[0].name = "std-dev-x".into();
        } else {
            args.iter_mut().find(|a| a.name == "std-dev-x").unwrap().value = FilterValue::Bool(false);
        }
        assert!(stage(&doc, true).is_err());
    }
}

#[test]
#[ignore = "UNRUN: active zero retains paid transfer/kernel-clone/inverse and exact raw axes; disabled bypass is distinct"]
fn active_zero_and_disabled_are_distinct_recipes() {
    let active = parameter_owner((-0.0f32).to_bits(), 0.0f32.to_bits(), true);
    let inactive = parameter_owner((-0.0f32).to_bits(), 0.0f32.to_bits(), false);
    let a = PreparedBackedGaussianGroupContext::new(&active, Default::default(), &ctl()).unwrap();
    let b = PreparedBackedGaussianGroupContext::new(&inactive, Default::default(), &ctl()).unwrap();
    assert!(stage(active.document(), true).unwrap().enabled);
    assert!(!stage(inactive.document(), true).unwrap().enabled);
    let raw = active.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|a| a.name == "std-dev-x").unwrap();
    assert_eq!(raw.value, FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())));
    a.validate_operative(&ctl()).unwrap();
    b.validate_operative(&ctl()).unwrap();
    assert_eq!(a.bounds(), b.bounds());
    // No equality assertion: active conversion rounding is deliberately retained.
    assert_eq!(a.render_band(a.bounds(), &ctl()).unwrap().pixels().len(), 192);
    assert_eq!(b.render_band(b.bounds(), &ctl()).unwrap().pixels().len(), 192);
}

#[test]
#[ignore = "UNRUN: max/near-one/zero CURRENT preparation retains actual same-q output and rolls back exact Work/Cancelled on warm native caches"]
fn parameter_resource_and_control_rollback() {
    for (x, y) in [(15f32, 15f32), (0., 0.), (f32::from_bits(0x3f800001), 0.8)] {
        let owner = parameter_owner(x.to_bits(), y.to_bits(), true);
        drop(PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &ctl()).unwrap());
        let q = owner.quota();
        let before = q.usage().unwrap();
        {
            let context = PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &ctl()).unwrap();
            assert!(q.usage().unwrap().resident > before.resident);
            context.validate_operative(&ctl()).unwrap();
        }
        assert_eq!(q.usage().unwrap(), before);
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n > 1024 { Err(BackingError::Work) } else { Ok(()) } };
        assert!(matches!(PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &denied), Err(E::Backing(BackingError::Work))));
        let cancelled = || true;
        let interrupt = Interrupt::cancel_only(&cancelled);
        let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &control), Err(E::Backing(BackingError::Cancelled))));
        assert_eq!(q.usage().unwrap(), before);
    }
}
