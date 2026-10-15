//! UNRUN assigned F64 observed-role consumers. Generated CURRENT fixtures and
//! lower references are external harness controls, not new producer acceptance.
use super::*;
fn activity(doc: &mut Document, visible: bool, opacity: f32, enabled: bool) {
    let id = doc.layers[0].id;
    doc.layers[0].visible = visible;
    doc.layers[0].opacity = opacity;
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().visible = visible;
    state.filter_stacks[0].nodes[0].enabled = enabled;
}

#[test]
#[ignore = "UNRUN observed assigned F64 source raw words/profile leases and exact old-source refusal"]
fn source_keeps_double_controls_and_both_profile_owners() {
    let s = Rect::new(13, -9, 270, 250);
    for radius in [4, 5] {
        let (d, q, g, p) = observed_role(SampleType::F64, true, s, radius);
        let mut input = resident_source(d, &q, g, s);
        let LayerContent::Raster(surface) = &mut input.document.layers[0].content else { panic!() };
        // Independent synthetic controls corresponding to the research word
        // classes; these are not a substitution for its full native capture.
        for (index, value) in [-0.125, 1.25, -0.0, 1. + 2f64.powi(-40), -2f64.powi(-150)].into_iter().enumerate() {
            let x = s.x0 + 100 + index as i32;
            let y = s.y0 + 100;
            let mut pixel = [0.; 4];
            surface.try_read_pixel_f64(x, y, &mut pixel).unwrap();
            pixel[index % 3] = value;
            surface.try_write_region_f64(Rect::new(x, y, x + 1, y + 1), &pixel).unwrap();
            let mut raw = [0.; 4];
            surface.try_read_pixel_f64(x, y, &mut raw).unwrap();
            assert_eq!(raw[index % 3].to_bits(), value.to_bits());
        }
        let before = input.document.clone(); // External metadata/native oracle.
        assert_ne!(p.trc.as_ref().unwrap()[0], p.trc.as_ref().unwrap()[1]);
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
        assert!(q.usage().unwrap().resident > baseline.resident);
        assert_eq!(input.document, before);
        drop(prepared);
        assert_eq!(q.usage().unwrap(), baseline);
        assert!(
            crate::backed_neighborhood_context::LeasedStockNeighborhoodPipeline::new_assigned_box_budgeted(
                &input.document,
                &Interrupt::NONE,
                &mut budget,
                &input
            )
            .is_err()
        );
        assert!(
            crate::backed_neighborhood_context::LeasedStockNeighborhoodPipeline::new_assigned_median_budgeted(
                &input.document,
                &Interrupt::NONE,
                &mut budget,
                &input
            )
            .is_err()
        );
        assert!(
            crate::backed_neighborhood_context::LeasedStockNeighborhoodPipeline::new_assigned_gaussian_budgeted(
                &input.document,
                &Interrupt::NONE,
                &mut budget,
                &input
            )
            .is_err()
        );
    }
}

#[test]
#[ignore = "UNRUN assigned F64 observed full-radius canonical tile reference and disabled native identity"]
fn current_uses_actual_profile_for_packed_row_and_zero_paths() {
    let s = Rect::new(-3, 5, 2, 8);
    for radius in [0, 1, 4, 5, 127, 1000] {
        let (d, q, g, p) = observed_role(SampleType::F64, true, s, radius);
        let owned = owner(d, q, g, true);
        let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits { max_rows: 2, max_pixels: 17 }, &ctl()).unwrap();
        let coordinate = photocraft_geom::TileCoord::containing(-1, 6);
        let (t, expected) = canonical_reference(SampleType::F64, s, radius as u32, coordinate, &p);
        let roi = Rect::new(t.x1 - 1, t.y0, t.x1, t.y0 + 1);
        compare(&context, roi, &expected);
        // The shared renderer placeholder is not assigned-profile authority.
        drop(PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::SGray.profile(), NormalBandLimits::default(), &ctl()).unwrap());
        assert!(crate::backed_neighborhood_context::PreparedBackedNeighborhoodContext::new_assigned_box(&owned, NormalBandLimits::default(), &ctl()).is_err());
        assert!(
            crate::backed_neighborhood_context::PreparedBackedNeighborhoodContext::new_assigned_median(&owned, NormalBandLimits::default(), &ctl()).is_err()
        );
        assert!(
            crate::backed_neighborhood_context::PreparedBackedNeighborhoodContext::new_assigned_gaussian(&owned, NormalBandLimits::default(), &ctl()).is_err()
        );
    }
    let (mut d, q, g, _) = observed_role(SampleType::F64, true, s, 1000);
    activity(&mut d, false, -0.0, false);
    let owned = owner(d, q, g, true);
    let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).unwrap();
    assert_eq!(context.bounds(), s);
    let got = context.render_wide_band(s, &ctl()).unwrap();
    let expected = resident(owned.document().pixel_format(), s).try_read_region_f64(s).unwrap();
    for (actual, expected) in got.pixels().iter().zip(expected.chunks_exact(4)) {
        assert_eq!(actual.map(f64::to_bits), [expected[0], expected[1], expected[2], expected[3]].map(f64::to_bits));
    }
}

#[test]
#[ignore = "UNRUN inactive assigned F64 profile/native/default/mask proof and old epoch boundary"]
fn inactive_source_and_current_do_not_skip_role_proofs() {
    let s = Rect::new(13, 9, 18, 12);
    for defect in 0..6 {
        let (mut d, q, g, _) = observed_role(SampleType::F64, true, s, 4);
        activity(&mut d, false, 0., false);
        let mut input = resident_source(d, &q, g, s);
        match defect {
            0 => input.document.source_color_context.as_mut().unwrap().source_precision = 650,
            1 => input.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap().to_vec())),
            2 => input.document.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 640])),
            3 => {
                let LayerContent::Raster(surface) = &mut input.document.layers[0].content else { panic!() };
                surface.try_write_region_f64(Rect::new(17, 11, 18, 12), &[f64::NAN, 0., 0., 1.]).unwrap();
            }
            4 => {
                let LayerContent::Raster(surface) = &mut input.document.layers[0].content else { panic!() };
                surface.try_write_region_f64(Rect::new(17, 11, 18, 12), &[0., 0., 0., 1. + 2f64.powi(-40)]).unwrap();
            }
            _ => {
                input.document.xcf_state.as_mut().unwrap().filter_masks[0].surface = Surface::try_with_default_bytes(
                    photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
                    &2f64.powi(-150).to_ne_bytes(),
                )
                .unwrap();
            }
        }
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    for defect in 0..3 {
        let (mut d, q, g, _) = observed_role(SampleType::F64, true, s, 4);
        activity(&mut d, false, 0., false);
        match defect {
            0 => d.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap().to_vec())),
            1 => d.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 640])),
            _ => {
                d.xcf_state.as_mut().unwrap().filter_masks[0].surface = Surface::try_with_default_bytes(
                    photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
                    &2f64.powi(-150).to_ne_bytes(),
                )
                .unwrap()
            }
        }
        let owned = owner(d, q, g, true);
        assert!(PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).is_err());
    }
    let (mut d, q, g, _, counts) = fixture(SampleType::F64, true, s, Rect::new(0, 0, 283, 281), 4, false);
    let node = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.schema_epoch = BOX_GEOMETRY_EPOCH.into();
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = BOX_GEOMETRY_EPOCH.into();
    policy.stored_blend_space = 1654948433;
    policy.stored_composite_space = 1654948433;
    policy.stored_operator = 1654948433;
    activity(&mut d, false, 0., false);
    counts.bad.store(true, Ordering::Relaxed);
    let owned = owner(d, q, g, true);
    assert!(PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).is_err());
    let (mut d, q, g, _) = observed_role(SampleType::F64, true, Rect::new(0, 0, 16, 12), 4);
    let node = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.schema_epoch = crate::gimp_box::EPOCH.into();
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = crate::gimp_box::EPOCH.into();
    policy.stored_blend_space = -1474592175;
    policy.stored_composite_space = -1474592175;
    policy.stored_operator = -1474592175;
    let owned = owner(d, q, g, true);
    assert!(PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).is_err());
}

#[test]
#[ignore = "UNRUN assigned F64 source profile late-work/cancellation/quota cleanup"]
fn profile_peak_cleanup_preserves_source_and_quota() {
    let s = Rect::new(-3, 5, 2, 8);
    let (d, q, g, _) = observed_role(SampleType::F64, true, s, 4);
    let input = resident_source(d, &q, g, s);
    let original = input.document.clone(); // External expected model.
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
    let observed = AtomicU64::new(0);
    let cancel = || observed.fetch_add(1, Ordering::Relaxed) >= total - 1;
    let interrupt = Interrupt::cancel_only(&cancel);
    let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
    assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &interrupt, &mut budget, &input).is_err());
    let mut denied = Budget { q: q.clone(), work: u64::MAX, deny: true };
    assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut denied, &input).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(input.document, original);
}
