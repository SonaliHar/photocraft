//! UNRUN structural copies plus existing declared math; no producer oracle.
use super::*;
use photocraft_doc::accounted_document::{AccountedEdit, PointArgumentPatch, PointArgumentValue, PointFilterPatch};
fn changed(kind: usize, p: [f64; 4], nl: &photocraft_cms::owned_lab::Parts, linear: &photocraft_cms::owned_lab::Parts) -> [f64; 4] {
    let curve = photocraft_cms::curve::srgb_trc();
    match kind {
        0 => {
            let work = [curve.eval64(p[0]) as f32, curve.eval64(p[1]) as f32, curve.eval64(p[2]) as f32, p[3] as f32].map(f64::from);
            let mut out = [0.; 4];
            photocraft_algo::gimp::point_semi_flatten::SemiFlatten { color: [0.125, 0.25, 0.5, 0.375] }.try_eval_pixel(&work, 3, &mut out).unwrap();
            [curve.eval_inverse64(out[0]), curve.eval_inverse64(out[1]), curve.eval_inverse64(out[2]), out[3]]
        }
        1 => physical(nl, p, -17.25, 22.5, -12.5),
        _ => {
            let work = [curve.eval64(p[0]) as f32, curve.eval64(p[1]) as f32, curve.eval64(p[2]) as f32, p[3] as f32];
            let sat = photocraft_algo::gimp::point_vibrance::saturation_linear_float(work, 1.125).unwrap();
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
#[test]
#[ignore = "UNRUN sameq targeted mixed typed copies/metadata composed against independent named declared leaf and bottom-first Normal math"]
fn target_copies_preserve_other_roots_and_composed_order() {
    let nl = photocraft_cms::owned_lab::construct_with_control(
        photocraft_cms::Builtin::Srgb.profile(),
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        Default::default(),
        &|| false,
    )
    .unwrap();
    let linear = photocraft_cms::owned_linear_srgb_lab::construct_with_control(Default::default(), &|| false).unwrap();
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        for target in 0..3 {
            let (owner, _, _, _) = adopt(fixture(sample, Size::new(257, 3), &[Some(0), Some(1), Some(2), None], None));
            let old = owner.document();
            let stack = &old.xcf_state.as_ref().unwrap().filter_stacks[target];
            let layer = old.layers[target].id;
            let replacement = match target {
                0 => PointArgumentPatch {
                    name: "color",
                    value: PointArgumentValue::LiteralColor4F32 {
                        components: [0.125f32, 0.25, 0.5, 0.375].map(f32::to_bits),
                        encoding: SampleEncoding::NonlinearProfile,
                    },
                },
                1 => PointArgumentPatch { name: "hue", value: PointArgumentValue::Float32Bits((-17.25f32).to_bits()) },
                _ => PointArgumentPatch { name: "saturation", value: PointArgumentValue::Float32Bits(1.125f32.to_bits()) },
            };
            let copy = owner
                .try_with_readonly_filtered_flat_point_filter(
                    PointFilterPatch { layer, filter: stack.nodes[0].id, expected_stack_revision: stack.revision, arguments: &[replacement], enabled: None },
                    &ctl(),
                )
                .unwrap();
            assert!(copy.is_readonly_filtered_flat_point_storage());
            assert_eq!(copy.document().icc_profile, old.icc_profile);
            assert_eq!(copy.document().source_color_context, old.source_color_context);
            assert_eq!(copy.document().xcf_state.as_ref().unwrap().filter_masks, old.xcf_state.as_ref().unwrap().filter_masks);
            for index in 0..4 {
                let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&old.layers[index].content, &copy.document().layers[index].content)
                else {
                    panic!("backed")
                };
                assert_eq!(a.descriptor(), b.descriptor());
                assert_eq!(a.default_bytes(), b.default_bytes());
            }
            for (index, s) in old.xcf_state.as_ref().unwrap().filter_stacks.iter().enumerate() {
                let next = &copy.document().xcf_state.as_ref().unwrap().filter_stacks[index];
                if index != target {
                    assert_eq!(s, next);
                } else {
                    assert_eq!(next.revision, s.revision + 1);
                }
            }
            let prepared = PreparedBackedFilteredFlatPointDocument::new(&copy, Default::default(), &ctl()).unwrap();
            prepared.validate_operative(&ctl()).unwrap();
            let mut expected = [0.; 4];
            for index in 0..4 {
                let source = if index == target {
                    changed(index, input(index, 2, 0, sample), &nl, &linear)
                } else {
                    leaf([Some(0), Some(1), Some(2), None][index], input(index, 2, 0, sample), &nl, &linear)
                };
                expected = working.normal(expected, source, f64::from(old.layers[index].opacity), GimpOperator::Union).unwrap();
            }
            assert_eq!(prepared.render_band(Rect::new(2, 0, 3, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
            drop(prepared);
            let metadata =
                copy.try_edit(AccountedEdit::LayerMetadata { layer, name: Some("Targeted root"), visible: Some(false), opacity: Some(0.375) }, &ctl()).unwrap();
            let prepared = PreparedBackedFilteredFlatPointDocument::new(&metadata, Default::default(), &ctl()).unwrap();
            prepared.validate_operative(&ctl()).unwrap();
            let mut hidden = [0.; 4];
            for index in 0..4 {
                if index != target {
                    hidden = working
                        .normal(
                            hidden,
                            leaf([Some(0), Some(1), Some(2), None][index], input(index, 2, 0, sample), &nl, &linear),
                            f64::from(old.layers[index].opacity),
                            GimpOperator::Union,
                        )
                        .unwrap();
                }
            }
            assert_eq!(prepared.render_band(Rect::new(2, 0, 3, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), hidden.map(f64::to_bits));
        }
    }
}
#[test]
#[ignore = "UNRUN targeted activation keeps inactive badnative proof, stale/noop structural guards and sameq failure cleanup after copies"]
fn disabled_copy_never_waives_full_native_proof_or_target_identity() {
    let (owner, _, _, _) = adopt(fixture(SampleType::F64, Size::new(257, 3), &[Some(0), Some(1), Some(2), None], Some(1)));
    let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[1];
    let layer = owner.document().layers[1].id;
    let patch = PointFilterPatch { layer, filter: stack.nodes[0].id, expected_stack_revision: stack.revision, arguments: &[], enabled: Some(false) };
    let copy = owner.try_with_readonly_filtered_flat_point_filter(patch, &ctl()).unwrap();
    assert!(PreparedBackedFilteredFlatPointDocument::new(&copy, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).is_err());
    let before = owner.quota().usage().unwrap();
    let foreign = PointFilterPatch { layer: owner.document().layers[2].id, ..patch };
    assert!(owner.try_with_readonly_filtered_flat_point_filter(foreign, &ctl()).is_err());
    let stale = PointFilterPatch { expected_stack_revision: stack.revision + 1, ..patch };
    assert!(owner.try_with_readonly_filtered_flat_point_filter(stale, &ctl()).is_err());
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(owner.try_with_readonly_filtered_flat_point_filter(patch, &denied), Err(BackingError::Work)));
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(owner.try_with_readonly_filtered_flat_point_filter(patch, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(owner.quota().usage().unwrap(), before);
}
