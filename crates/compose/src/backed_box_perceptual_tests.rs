//! UNRUN implicit Perceptual consumers. Synthetic current models and lower Surface
//! references are harness allocations, not producer/profile equivalence proof.
use super::*;
fn perceptual(sample: SampleType, s: Rect, radius: i64) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>) {
    let (mut doc, q, guard, _) = observed_depth(sample, s, radius);
    doc.sample_encoding = Some(SampleEncoding::Perceptual);
    doc.source_color_context.as_mut().unwrap().source_precision = if sample == SampleType::F32 { 675 } else { 775 };
    // This is an external fixture constructor; production uses its controlled
    // paid implicit requirements+constructor and exact encoded comparison.
    let effective =
        photocraft_cms::xcf_color::effective_profile_from_definition(photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::xcf_color::Encoding::Perceptual)
            .unwrap();
    let bytes = effective.encoded_bytes().unwrap().to_vec();
    doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(&bytes);
    doc.icc_profile = Some(Arc::new(bytes));
    (doc, q, guard)
}
fn activity(doc: &mut Document, enabled: bool) {
    let id = doc.layers[0].id;
    doc.layers[0].visible = false;
    doc.layers[0].opacity = -0.0;
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().visible = false;
    state.filter_stacks[0].nodes[0].enabled = enabled;
}
fn expected(sample: SampleType, s: Rect, radius: u32, coordinate: photocraft_geom::TileCoord) -> (Rect, Surface) {
    let source = resident(photocraft_color::PixelFormat::new(ColorMode::Rgb, sample, true), s);
    let t = coordinate.rect().intersect(&n::box_blur_extent(s, radius).unwrap());
    let curve = photocraft_cms::curve::srgb_trc();
    let mut samples = source.try_read_region_f64(s).unwrap();
    for pixel in samples.chunks_exact_mut(4) {
        for value in &mut pixel[..3] {
            *value = curve.eval64(*value);
        }
    }
    let mut working = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    working.try_write_region_f64(s, &samples).unwrap();
    let req = n::box_blur_requirements(&working, t, s, radius, &Interrupt::NONE).unwrap();
    let filtered =
        n::try_box_blur(&working, t, s, radius, n::GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units }, &Interrupt::NONE)
            .unwrap();
    let mut pixels = filtered.try_read_region_f64(t).unwrap();
    for pixel in pixels.chunks_exact_mut(4) {
        for value in &mut pixel[..3] {
            *value = curve.eval_inverse64(*value);
        }
    }
    let mut native = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    native.try_write_region_f64(t, &pixels).unwrap();
    (t, native)
}

#[test]
#[ignore = "UNRUN implicit Perceptual source keeps actual effective ICC separate from nonlinear basis"]
fn source_pays_actual_perceptual_profile_and_space_even_inactive() {
    for sample in [SampleType::F32, SampleType::F64] {
        let s = Rect::new(-3, 5, 2, 8);
        for radius in [0, 4, 5, 1000] {
            let (d, q, g) = perceptual(sample, s, radius);
            let mut input = resident_source(d, &q, g, s);
            assert_ne!(input.document.icc_profile.as_deref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap());
            let before = input.document.clone();
            let baseline = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
            let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
            assert!(q.usage().unwrap().resident > baseline.resident);
            assert_eq!(input.document, before);
            drop(prepared);
            assert_eq!(q.usage().unwrap(), baseline);
            activity(&mut input.document, false);
            drop(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap());
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
}

#[test]
#[ignore = "UNRUN Perceptual stored encoding retains forward and reverse transfer for packed/row/zero CURRENT output"]
fn current_matches_perceptual_canonical_kernel_and_preserves_factory_basis_contract() {
    let s = Rect::new(-3, 5, 2, 8);
    for sample in [SampleType::F32, SampleType::F64] {
        for radius in [0, 1, 4, 5, 127, 1000] {
            let (d, q, g) = perceptual(sample, s, radius);
            assert!(is_candidate(&d, &ctl()).unwrap());
            let owned = owner(d, q, g, false);
            let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits { max_rows: 2, max_pixels: 17 }, &ctl()).unwrap();
            let coordinate = photocraft_geom::TileCoord::containing(-1, 6);
            let (t, reference) = expected(sample, s, radius as u32, coordinate);
            let roi = Rect::new(t.x1 - 1, t.y0, t.x1, t.y0 + 1);
            compare(&context, roi, &reference);
            let ws =
                WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Perceptual, GimpSpace::LinearRgb, &Interrupt::NONE)
                    .unwrap();
            let rgb = [-0.125, 1.25, -0.0];
            let curve = photocraft_cms::curve::srgb_trc();
            assert_eq!(ws.convert(rgb, true).unwrap().map(f64::to_bits), rgb.map(|v| curve.eval64(v).to_bits()));
            assert_eq!(ws.convert(rgb, false).unwrap().map(f64::to_bits), rgb.map(|v| curve.eval_inverse64(v).to_bits()));
            assert_ne!(ws.convert([0.5; 3], true).unwrap(), [0.5; 3]);
            drop(PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::Srgb.profile(), NormalBandLimits::default(), &ctl()).unwrap());
            assert!(PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::SGray.profile(), NormalBandLimits::default(), &ctl()).is_err());
            let effective = photocraft_cms::xcf_color::effective_profile_from_definition(
                photocraft_cms::Builtin::Srgb.profile(),
                photocraft_cms::xcf_color::Encoding::Perceptual,
            )
            .unwrap();
            assert!(PreparedBackedDocument::new_accounted(&owned, &effective, NormalBandLimits::default(), &ctl()).is_err());
        }
        let (mut d, q, g) = perceptual(sample, s, 1000);
        activity(&mut d, false);
        let owned = owner(d, q, g, false);
        let context = PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).unwrap();
        assert_eq!(context.bounds(), s);
        compare(&context, s, &resident(owned.document().pixel_format(), s));
    }
}

#[test]
#[ignore = "UNRUN implicit Perceptual exact role/epoch/profile/native/mask refusals while hidden and disabled"]
fn identity_and_inactivity_do_not_waive_full_proofs_or_old_boundaries() {
    let s = Rect::new(13, 9, 18, 12);
    for sample in [SampleType::F32, SampleType::F64] {
        for defect in 0..7 {
            let (mut d, q, g) = perceptual(sample, s, 4);
            activity(&mut d, false);
            let mut input = resident_source(d, &q, g, s);
            match defect {
                0 => input.document.source_color_context.as_mut().unwrap().source_precision = if sample == SampleType::F32 { 650 } else { 750 },
                1 => input.document.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 640])),
                2 => input.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap().to_vec())),
                3 => input.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
                4 | 5 => {
                    let LayerContent::Raster(source) = &mut input.document.layers[0].content else { panic!() };
                    let pixel = if defect == 4 { [f64::NAN, 0., 0., 1.] } else { [0., 0., 0., 2.] };
                    source.try_write_region_f64(Rect::new(17, 11, 18, 12), &pixel).unwrap();
                }
                _ => {
                    let mut default = [0u8; 8];
                    photocraft_color::try_write_sample_f64(&mut default, sample, 0, 0.5).unwrap();
                    input.document.xcf_state.as_mut().unwrap().filter_masks[0].surface =
                        Surface::try_with_default_bytes(photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false), &default[..sample.bytes()])
                            .unwrap();
                }
            }
            let baseline = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
            assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
        }
        let (mut d, q, g) = perceptual(sample, Rect::new(0, 0, 16, 12), 4);
        let node = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.schema_epoch = crate::gimp_box::EPOCH.into();
        let policy = node.composition.as_mut().unwrap();
        policy.compatibility_epoch = crate::gimp_box::EPOCH.into();
        policy.stored_blend_space = -1474592175;
        policy.stored_composite_space = -1474592175;
        policy.stored_operator = -1474592175;
        assert!(!is_candidate(&d, &ctl()).unwrap());
        let owned = owner(d, q, g, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
    }
}

#[test]
#[ignore = "UNRUN Perceptual profile preparation quota/late work/cancellation source ownership"]
fn preparation_errors_keep_typed_control_and_release_all_temporary_fees() {
    let s = Rect::new(-3, 5, 2, 8);
    for sample in [SampleType::F32, SampleType::F64] {
        let (d, q, g) = perceptual(sample, s, 127);
        let input = resident_source(d, &q, g, s);
        let original = input.document.clone();
        let baseline = q.usage().unwrap();
        let polls = AtomicU64::new(0);
        let count = || {
            polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        let interrupt = Interrupt::cancel_only(&count);
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        drop(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &interrupt, &mut budget, &input).unwrap());
        let consumed = u64::MAX - budget.work;
        let total = polls.load(Ordering::Relaxed);
        assert!(consumed > 0 && total > 2);
        let mut late = Budget { q: q.clone(), work: consumed - 1, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut late, &input).is_err());
        let seen = AtomicU64::new(0);
        let cancel = || seen.fetch_add(1, Ordering::Relaxed) >= total - 1;
        let interrupt = Interrupt::cancel_only(&cancel);
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        assert!(matches!(
            LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &interrupt, &mut budget, &input),
            Err(CompositionError::Raster(RasterInputError::Cancelled))
        ));
        let mut denied = Budget { q: q.clone(), work: u64::MAX, deny: true };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut denied, &input).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(input.document, original);
    }
}
