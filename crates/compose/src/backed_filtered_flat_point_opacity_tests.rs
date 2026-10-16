//! Ignored UNRUN CURRENT-only opacity regression contracts; no producer/display oracle.
use super::*;
fn set_kind(n: &mut photocraft_doc::xcf_filters::FilterNode, kind: u8) {
    if kind < 3 {
        configure(n, kind);
        return;
    }
    if kind == 3 || kind == 6 {
        let epoch = if kind == 6 { crate::EXPOSURE_LOADED_MIXED_EPOCH } else { crate::EXPOSURE_MIXED_EPOCH };
        n.operation = OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: epoch.into() };
        n.arguments = vec![
            number("exposure", 0.5),
            FilterArgument {
                name: "black-level".into(),
                value: FilterValue::Number(ExactNumber::Float32Bits(if kind == 6 { 0x3dcc_cccd } else { 0.0625f32.to_bits() })),
            },
        ];
        n.composition.as_mut().unwrap().compatibility_epoch = epoch.into();
    } else {
        let k = if kind == 4 { crate::InvertKind::Linear } else { crate::InvertKind::Gamma };
        n.operation = OperationIdentity { name: k.operation_name().into(), stored_version: k.version().into(), schema_epoch: k.epoch().into() };
        n.arguments.clear();
        n.composition.as_mut().unwrap().compatibility_epoch = k.epoch().into();
    }
}
fn fixture_opacity(sample: SampleType, kind: u8, amount: f64, enabled: bool) -> Fixture {
    let mut f = ordered(sample, &[&[0], &[]], enabled);
    let n = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    set_kind(n, kind);
    n.enabled = enabled;
    n.opacity_f64_bits = amount.to_bits();
    f
}
fn current(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>, Arc<AtomicU64>) {
    let o = AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()).unwrap();
    (o, f.rasters, f.fails, f.reads)
}
fn old_leaf(kind: u8, p: [f64; 4], derived: bool, nl: &photocraft_cms::owned_lab::Parts, linear: &photocraft_cms::owned_lab::Parts) -> [f64; 4] {
    if kind < 3 {
        return step(kind, p, derived, nl, linear);
    }
    let c = photocraft_cms::srgb_trc();
    let mut out = p;
    for i in 0..3 {
        out[i] = match kind {
            3 | 6 => {
                let black = if kind == 6 { 0. } else { 0.0625 };
                c.eval_inverse64((c.eval64(p[i]) - black) / ((-0.5f64).exp2() - black))
            }
            4 => c.eval_inverse64(1. - c.eval64(p[i])),
            _ => 1. - p[i],
        };
    }
    out
}
fn mix_reference(b: [f64; 4], a: [f64; 4], t: f64) -> [f64; 4] {
    if t == 1. {
        return a;
    }
    if t == 0. {
        return b.map(|v| f64::from(v as f32));
    }
    let c = photocraft_cms::srgb_trc();
    let alpha = b[3] + t * (a[3] - b[3]);
    let mut out = [0.; 4];
    for i in 0..3 {
        let x = c.eval64(b[i]);
        let y = c.eval64(a[i]);
        let z = if alpha == 0. {
            x + t * (y - x)
        } else {
            let bp = x * b[3];
            let ap = y * a[3];
            (bp + t * (ap - bp)) / alpha
        };
        out[i] = f64::from(c.eval_inverse64(z) as f32);
    }
    out[3] = f64::from(alpha as f32);
    out
}
#[test]
#[ignore = "UNRUN six kernels plus loadedV2 actualCURRENT solepaidstage +/-0/partial/arbitraryF64/1/disabled exact independent oldleaf+mix references, Source+1closure and raw/native/profile/mask immutability"]
fn all_current_families_and_source_boundary() {
    let (nl, linear) = parts(); // External independent public reference allocations, no production fee authority.
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in 0..7 {
            for amount in [1., 0., -0., 0.375, f64::from_bits(0.375f64.to_bits() + 1)] {
                for enabled in [true, false] {
                    let f = fixture_opacity(sample, kind, amount, enabled);
                    let nodes = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.clone();
                    let profile = f.document.icc_profile.clone();
                    let mask = f.document.xcf_state.as_ref().unwrap().filter_masks[0].surface.clone(); // external expected values under retained fixture envelope
                    let (o, _, _, _) = current(f);
                    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                    c.validate_operative(&ctl()).unwrap();
                    for x in 0u32..5 {
                        let p = input(0, x, 0, sample);
                        let expected = if enabled { mix_reference(p, old_leaf(kind, p, false, &nl, &linear), amount) } else { p };
                        assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                    }
                    assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes, nodes);
                    assert_eq!(o.document().icc_profile, profile);
                    assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_masks[0].surface.default_bytes(), mask.default_bytes());
                    let q = o.quota().clone();
                    let before = q.usage().unwrap();
                    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                    let source = LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone());
                    assert_eq!(source.is_ok(), amount.to_bits() == 1f64.to_bits());
                    drop(source);
                    assert_eq!(q.usage().unwrap(), before);
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN named ordered enabledzero versusdisabled bridge state, fullExposure eval despite zero and finiteHDR nextstage, exactunit oldbody/finalwideSemi no extrapostcast"]
fn enabled_zero_is_not_disabled_and_bridges_are_unchanged() {
    let (nl, linear) = parts();
    for sample in [SampleType::F32, SampleType::F64] {
        for middle_enabled in [true, false] {
            let mut f = ordered(sample, &[&[0, 0, 0], &[]], true);
            let nodes = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
            set_kind(&mut nodes[0], 5);
            nodes[0].opacity_f64_bits = 0.375f64.to_bits();
            set_kind(&mut nodes[1], 3);
            nodes[1].opacity_f64_bits = (-0f64).to_bits();
            nodes[1].enabled = middle_enabled;
            set_kind(&mut nodes[2], 4);
            let (o, _, _, _) = current(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in 0u32..5 {
                let p = input(0, x, 0, sample);
                let bottom = old_leaf(4, p, false, &nl, &linear);
                let before_top = if middle_enabled {
                    let incoming = bridge(bottom);
                    let _full_after = old_leaf(3, incoming, true, &nl, &linear);
                    bridge(incoming.map(|v| f64::from(v as f32)))
                } else {
                    bridge(bottom)
                };
                let expected = mix_reference(before_top, old_leaf(5, before_top, true, &nl, &linear), 0.375);
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
    // Zero opacity must not bypass a failing enabled kernel, even though output would be before.
    let mut f = fixture_opacity(SampleType::F64, 3, 0., true);
    let n = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    n.arguments = vec![number("exposure", f32::MAX), number("black-level", 0.)];
    let (o, _, _, _) = current(f);
    assert!(Context::new(&o, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN nonunit allinactive still profile/native/default/mask proof, originalsameq work/cancel cleanup, coldprovider failure and source-refusal; no enginefence/publication bypass"]
fn full_inactive_proof_and_real_control_cleanup() {
    for fault in 0..4 {
        let mut f = fixture_opacity(SampleType::F64, 6, -0., false);
        if fault == 3 {
            f = fixture(SampleType::F64, Size::new(8, 6), &[Some(0), None], Some(0));
            let n = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            set_kind(n, 6);
            n.opacity_f64_bits = (-0f64).to_bits();
            n.enabled = false;
        }
        f.document.layers[0].visible = false;
        f.document.layers[0].opacity = 0.;
        let id = f.document.layers[0].id;
        f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
        if fault == 0 {
            f.document.xcf_state.as_mut().unwrap().filter_masks[0]
                .surface
                .try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes())
                .unwrap();
        } else if fault == 1 {
            f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
        } else if fault == 2 {
            f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "9:9".into();
        }
        let (o, _, _, _) = current(f);
        assert!(Context::new(&o, Default::default(), &ctl()).is_err());
    }
    let (o, rasters, fails, _) = current(fixture_opacity(SampleType::F64, 2, 0.375, true));
    let q = o.quota().clone();
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    let r = Rect::new(0, 0, 2, 1);
    drop(c.render_band(r, &ctl()).unwrap());
    let baseline = q.usage().unwrap();
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &count };
    drop(c.render_band(r, &control).unwrap());
    let limit = total.load(Ordering::Relaxed) - 1;
    let used = AtomicU64::new(0);
    let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > limit { Err(BackingError::Work) } else { Ok(()) };
    assert!(matches!(c.render_band(r, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let polls = AtomicU64::new(0);
    let observe = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let i = Interrupt::cancel_only(&observe);
    drop(c.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).unwrap());
    let accepted = polls.load(Ordering::Relaxed);
    assert!(accepted > 1);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= accepted - 1;
    let i = Interrupt::cancel_only(&cancel);
    assert!(matches!(c.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    rasters[1].clear_cache().unwrap();
    let baseline = q.usage().unwrap();
    fails[1].store(true, Ordering::Relaxed);
    assert!(matches!(c.render_band(r, &ctl()), Err(E::Backing(BackingError::Io))));
    assert_eq!(q.usage().unwrap(), baseline);
    fails[1].store(false, Ordering::Relaxed);
    drop(c.render_band(r, &ctl()).unwrap());
}
