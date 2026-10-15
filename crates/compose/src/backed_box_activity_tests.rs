//! UNRUN CURRENT layer activity. The source admission policy remains separate.
use super::*;
fn activity(
    sample: SampleType,
    assigned: bool,
    visible: bool,
    opacity: f32,
    enabled: bool,
    radius: i64,
) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile, Arc<Counts>) {
    let s = Rect::new(13, 9, 18, 12);
    let (mut d, q, g, p, counts) = fixture(sample, assigned, s, Rect::new(0, 0, 283, 281), radius, enabled);
    d.layers[0].visible = visible;
    d.layers[0].opacity = opacity;
    let id = d.layers[0].id;
    let state = d.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap().visible = visible;
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation.schema_epoch = BOX_GEOMETRY_EPOCH.into();
    node.composition.as_mut().unwrap().compatibility_epoch = BOX_GEOMETRY_EPOCH.into();
    (d, q, g, p, counts)
}
#[test]
#[ignore = "UNRUN observed CURRENT activity existing Normal display reference"]
fn current_activity_uses_actual_visibility_and_opacity_after_filtering() {
    let s = Rect::new(13, 9, 18, 12);
    let roi = Rect::new(13, 9, 17, 11);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for (visible, opacity) in [(true, 0.375), (false, 1.), (false, 0.), (true, 0.), (true, -0f32)] {
            for (enabled, radius) in [(true, 0), (true, 4), (true, 127), (false, 127)] {
                let (d, q, g, p, _) = activity(sample, assigned, visible, opacity, enabled, radius);
                let owned = owner(d, q, g, assigned);
                let renderer = PreparedBackedBoxDocument::new(&owned, NormalBandLimits::default(), &ctl()).unwrap();
                renderer.validate_operative(&ctl()).unwrap();
                let expected = if enabled {
                    canonical_reference(sample, s, radius as u32, photocraft_geom::TileCoord::containing(roi.x0, roi.y0), &p).1
                } else {
                    resident(owned.document().pixel_format(), s)
                };
                let output = renderer.render_band(roi, &ctl()).unwrap();
                for (index, actual) in output.pixels().iter().enumerate() {
                    let x = roi.x0 + (index % roi.width() as usize) as i32;
                    let y = roi.y0 + (index / roi.width() as usize) as i32;
                    let mut p = [0.; 4];
                    expected.try_read_pixel_f64(x, y, &mut p).unwrap();
                    let expected = crate::gimp_composite::normal_over_zero_backdrop(p, if visible { f64::from(opacity) } else { 0. }).unwrap();
                    assert_eq!(*actual, expected);
                }
                assert_eq!(owned.document().layers[0].opacity.to_bits(), opacity.to_bits());
            }
        }
    }
}
#[test]
#[ignore = "UNRUN hidden/zero opacity complete native/mask/profile and coherence guards"]
fn hidden_and_zero_coverage_never_waive_current_input_proof() {
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for (visible, opacity) in [(false, 1.), (true, 0.)] {
            for defect in 0..4 {
                let (mut d, q, g, _, counts) = activity(sample, assigned, visible, opacity, false, 1000);
                match defect {
                    0 => counts.bad.store(true, Ordering::Relaxed),
                    1 => {
                        let value = if sample == SampleType::F32 { f32::from_bits(1).to_ne_bytes().to_vec() } else { f64::from_bits(1).to_ne_bytes().to_vec() };
                        d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_interleaved(Rect::new(511, 511, 512, 512), &value).unwrap();
                    }
                    2 => {
                        if assigned {
                            d.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 640]));
                        } else {
                            d.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
                        }
                    }
                    _ => {
                        let id = d.layers[0].id;
                        d.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap().visible = !visible;
                    }
                }
                let owned = owner(d, q, g, assigned);
                assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
            }
        }
    }
    for opacity in [f32::NAN, f32::INFINITY, -0.125, 1.125] {
        let (d, q, g, _, _) = activity(SampleType::F32, false, false, opacity, true, 4);
        // Structural custody may reject invalid activity before the renderer.
        match AccountedDocument::new_readonly_point_storage(d, g, q, &ctl()) {
            Err(_) => {}
            Ok(owned) => assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err()),
        }
    }
}
#[test]
#[ignore = "UNRUN hidden full expanded-output work and observed source full preparation"]
fn hidden_enabled_still_validates_expanded_output_and_source_retains_full_preparation() {
    let s = Rect::new(13, 9, 18, 12);
    for (visible, opacity) in [(false, 1.), (true, 0.), (true, 0.375)] {
        let (d, q, g, _, _) = activity(SampleType::F32, false, visible, opacity, true, 127);
        let owned = owner(d, q.clone(), g, false);
        let context = PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).unwrap();
        let outside = photocraft_geom::TileCoord::containing(-1, -1);
        let plan = n::BoxBlurRowPlan::new(s, outside, 127, &Interrupt::NONE).unwrap();
        assert!(plan.tile_output().intersect(&owned.document().bounds()).is_empty());
        let req = plan.requirements(&Interrupt::NONE).unwrap();
        let armed = AtomicBool::new(false);
        let work = |units| {
            if units == req.work_units {
                armed.store(true, Ordering::Relaxed);
                Err(BackingError::Work)
            } else {
                Ok(())
            }
        };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(matches!(context.validate_operative(&control), Err(E::Backing(BackingError::Work))));
        assert!(armed.load(Ordering::Relaxed));
        let (d, q, g, _, _) = activity(SampleType::F32, false, visible, opacity, true, 127);
        let input = resident_source(d, &q, g, s);
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        // Observed source activity is now admitted, while all profile/Space
        // preparation is retained and charged even for hidden or zero coverage.
        let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
        assert!(budget.work < u64::MAX);
        assert!(q.usage().unwrap().resident > baseline.resident);
        drop(prepared);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
