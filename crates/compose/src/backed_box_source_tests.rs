//! UNRUN observational Box adapter consumers. Generated models are CURRENT
//! foundation controls, not independent GIMP producer or ingestion evidence.
#[path = "backed_box_radius_tests.rs"]
mod full_radius;
#[path = "backed_box_source_activity_tests.rs"]
mod source_activity;
use super::*;
use crate::backed_box_source::{BOX_GEOMETRY_EPOCH, LeasedStreamedBoxPipeline};
use crate::gimp_composite::CompositionError;
use crate::gimp_render::PreparationBudget;
use photocraft_raster::RasterInputError;
struct Budget {
    q: BackingQuota,
    work: u64,
    deny: bool,
}
impl PreparationBudget for Budget {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        if self.deny {
            return Err(CompositionError::Invalid("harness quota refusal"));
        }
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Invalid("harness quota"))
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.work = self.work.checked_sub(n).ok_or(CompositionError::Invalid("harness work"))?;
        Ok(())
    }
}
fn observed(s: Rect, r: i64) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile) {
    observed_depth(SampleType::F32, s, r)
}
fn observed_depth(sample: SampleType, s: Rect, r: i64) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile) {
    observed_role(sample, false, s, r)
}
fn observed_role(sample: SampleType, assigned: bool, s: Rect, r: i64) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile) {
    let (mut d, q, g, p, _) = fixture(sample, assigned, s, Rect::new(0, 0, 283, 281), r, true);
    let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    n.operation.schema_epoch = BOX_GEOMETRY_EPOCH.into();
    let c = n.composition.as_mut().unwrap();
    c.compatibility_epoch = BOX_GEOMETRY_EPOCH.into();
    c.stored_blend_space = 1654948433;
    c.stored_composite_space = 1654948433;
    c.stored_operator = 1654948433;
    (d, q, g, p)
}
// Payload fields precede every retained source fee. The raw staging Vec and
// physical tile construction overlap is admitted before either is allocated.
struct ResidentSource {
    document: Document,
    _original_guard: Arc<dyn std::any::Any + Send + Sync>,
    _resident_lease: QuotaLease,
}
fn resident_source(d: Document, q: &BackingQuota, guard: Arc<dyn std::any::Any + Send + Sync>, s: Rect) -> ResidentSource {
    let bpp = d.pixel_format().bytes_per_pixel() as u64;
    let physical = (s.tiles().count() as u64).checked_mul(256 * 256 * bpp).unwrap();
    let raw = u64::from(s.width()).checked_mul(u64::from(s.height())).unwrap().checked_mul(bpp).unwrap();
    // Two physical payload envelopes cover destination construction, growing
    // replacement map and tile temporaries. The staging Vec may reserve twice
    // its logical raw length. Map/default/holder overhead is separately added.
    let peak = physical
        .checked_mul(2)
        .unwrap()
        .checked_add(raw.checked_mul(2).unwrap())
        .unwrap()
        .checked_add(2 * 256 * 256 * bpp + (s.tiles().count() as u64) * 1024 + 65536 + std::mem::size_of::<ResidentSource>() as u64)
        .unwrap();
    let fee = q.reserve(QuotaKind::Resident, peak).unwrap();
    let mut out = ResidentSource { document: d, _original_guard: guard, _resident_lease: fee };
    let v = resident(out.document.pixel_format(), s);
    let retained = v.resident_accounted_bytes().unwrap().checked_add(std::mem::size_of::<ResidentSource>() as u64 + 1024).unwrap();
    assert!(retained <= peak);
    out.document.layers[0].content = LayerContent::Raster(v);
    // resident() has now dropped its temporary interleaved raw Vec.
    out._resident_lease.reconcile_capacity(retained).unwrap();
    out
}
#[test]
#[ignore = "UNRUN observational source adapter; all PhotoCraft execution deferred"]
fn opaque_raw_provenance_and_real_preparation_are_preserved() {
    for s in [Rect::new(0, 0, 257, 259), Rect::new(-7, 11, 250, 270), Rect::new(13, 9, 270, 268)] {
        for r in [4, 5] {
            let (d, q, g, _) = observed(s, r);
            let mut input = resident_source(d, &q, g, s);
            let d = &mut input.document;
            let raw = d.clone(); // External oracle copy, not a production receipt.
            let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
            let baseline = q.usage().unwrap();
            let prepared =
                LeasedStreamedBoxPipeline::new_geometry_budgeted(d, &Interrupt::NONE, &mut budget, (&input._original_guard, &input._resident_lease)).unwrap();
            assert_eq!(*d, raw);
            assert!(budget.work < u64::MAX);
            assert!(q.usage().unwrap().resident > baseline.resident);
            drop(prepared);
            assert_eq!(q.usage().unwrap(), baseline);
            // Raw words are opaque, even when independently different.
            let c = d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap();
            c.stored_blend_space = i32::MIN;
            c.stored_composite_space = 17;
            c.stored_operator = i32::MAX;
            LeasedStreamedBoxPipeline::new_geometry_budgeted(d, &Interrupt::NONE, &mut budget, (&input._original_guard, &input._resident_lease)).unwrap();
        }
    }
}
#[test]
#[ignore = "UNRUN observational CURRENT packed/reference parity"]
fn observed_current_uses_original_source_clamp_and_full_tile_before_crop() {
    let s = Rect::new(-7, 11, 250, 270);
    for r in [4, 5] {
        let (d, q, g, p) = observed(s, r);
        let owned = owner(d, q, g, false);
        let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).unwrap();
        let (o, expected) = reference(owned.document().pixel_format(), s, r as u32, &p);
        assert_eq!(context.bounds(), o);
        for roi in [Rect::new(-9, 12, 4, 15), Rect::new(245, 250, 253, 260), Rect::new(0, 255, 19, 257)] {
            let got = context.render_wide_band(roi, &ctl()).unwrap();
            for (index, actual) in got.pixels().iter().enumerate() {
                let x = roi.x0 + (index % roi.width() as usize) as i32;
                let y = roi.y0 + (index / roi.width() as usize) as i32;
                let mut p = [0.; 4];
                expected.try_read_pixel_f64(x, y, &mut p).unwrap();
                assert_eq!(*actual, p);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN full physical source/mask/default validation"]
fn hidden_padding_mask_and_profile_failures_are_not_waived() {
    let s = Rect::new(13, 9, 270, 268);
    for defect in 0..5 {
        let (d, q, g, _) = observed(s, 4);
        let mut input = resident_source(d, &q, g, s);
        let d = &mut input.document;
        match defect {
            0 => {
                d.layers[0].visible = false;
                d.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(d.layers[0].id)).unwrap().visible = false;
            }
            1 => {
                let LayerContent::Raster(v) = &mut d.layers[0].content else { panic!() };
                let mut raw = [0u8; 16];
                raw[..4].copy_from_slice(&f32::NAN.to_ne_bytes());
                raw[12..].copy_from_slice(&1f32.to_ne_bytes());
                v.try_write_interleaved(Rect::new(0, 0, 1, 1), &raw).unwrap();
            }
            2 => {
                d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_region_f64(Rect::new(511, 511, 512, 512), &[1.]).unwrap();
            }
            3 => {
                d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_clipping = Some(1);
            }
            _ => {
                d.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
            }
        }
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        let before = d.clone(); // external expected model; no renderer authority.
        let baseline = q.usage().unwrap();
        let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(d, &Interrupt::NONE, &mut budget, (&input._original_guard, &input._resident_lease));
        if defect == 0 {
            // Coherent inactivity is admitted, without skipping any native,
            // mask or profile validation exercised by the remaining defects.
            drop(prepared.unwrap());
            assert_eq!(*d, before);
            assert_eq!(q.usage().unwrap(), baseline);
        } else {
            assert!(prepared.is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN actual Budget work/allocation/caller cancellation boundaries"]
fn source_preparation_refuses_budget_and_caller_cancellation() {
    let s = Rect::new(0, 0, 257, 259);
    let (d, q, g, _) = observed(s, 4);
    let mut input = resident_source(d, &q, g, s);
    let d = &mut input.document;
    let baseline = q.usage().unwrap();
    let mut measured = Budget { q: q.clone(), work: u64::MAX, deny: false };
    drop(LeasedStreamedBoxPipeline::new_geometry_budgeted(d, &Interrupt::NONE, &mut measured, (&input._original_guard, &input._resident_lease)).unwrap());
    let consumed = u64::MAX - measured.work;
    let mut late = Budget { q: q.clone(), work: consumed - 1, deny: false };
    assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(d, &Interrupt::NONE, &mut late, (&input._original_guard, &input._resident_lease)).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    for (work, deny) in [(0, false), (u64::MAX, true)] {
        let mut budget = Budget { q: q.clone(), work, deny };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(d, &Interrupt::NONE, &mut budget, (&input._original_guard, &input._resident_lease)).is_err());
    }
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let mut budget = Budget { q, work: u64::MAX, deny: false };
    assert!(matches!(
        LeasedStreamedBoxPipeline::new_geometry_budgeted(d, &interrupt, &mut budget, (&input._original_guard, &input._resident_lease)),
        Err(CompositionError::Raster(RasterInputError::Cancelled))
    ));
}

#[test]
#[ignore = "UNRUN implicit F64 source raw preservation; not producer ICC equivalence"]
fn f64_source_retains_low_bits_and_signed_zero_without_f32_narrowing() {
    let s = Rect::new(-7, 11, 250, 270);
    for radius in [4, 5] {
        let (d, q, g, _) = observed_depth(SampleType::F64, s, radius);
        let mut input = resident_source(d, &q, g, s);
        let mut pixel = [0u8; 32];
        for (index, value) in [1. + 2f64.powi(-40), -0., -2f64.powi(-150), 0.5].into_iter().enumerate() {
            pixel[index * 8..index * 8 + 8].copy_from_slice(&value.to_ne_bytes());
        }
        let LayerContent::Raster(surface) = &mut input.document.layers[0].content else { panic!() };
        surface.try_write_interleaved(Rect::new(s.x0, s.y0, s.x0 + 1, s.y0 + 1), &pixel).unwrap();
        input.document.xcf_state.as_mut().unwrap().filter_masks[0]
            .surface
            .try_write_interleaved(Rect::new(511, 511, 512, 512), &(-0f64).to_ne_bytes())
            .unwrap();
        let expected = input.document.clone(); // External raw oracle, not a production receipt.
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
        assert_eq!(input.document, expected);
        assert!(q.usage().unwrap().resident > baseline.resident);
        drop(prepared);
        assert_eq!(q.usage().unwrap(), baseline);
        let consumed = u64::MAX - budget.work;
        let LayerContent::Raster(surface) = &input.document.layers[0].content else { panic!() };
        assert!(consumed >= surface.resident_payload_bytes().unwrap());
        let mut late = Budget { q: q.clone(), work: consumed - 1, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut late, &input).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN F64 observational CURRENT full-tile reference and reordered ROI"]
fn f64_current_preserves_wide_full_tile_schedule() {
    let s = Rect::new(13, 9, 270, 268);
    for radius in [4, 5] {
        let (d, q, g, p) = observed_depth(SampleType::F64, s, radius);
        let owned = owner(d, q, g, false);
        let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).unwrap();
        let (o, expected) = reference(owned.document().pixel_format(), s, radius as u32, &p);
        assert_eq!(context.bounds(), o);
        for requested in [Rect::new(255, 255, 273, 257), Rect::new(8, 10, 17, 13), Rect::new(257, 250, 261, 254)] {
            let roi = requested.intersect(&o);
            if roi != requested {
                assert!(context.render_wide_band(requested, &ctl()).is_err());
            }
            let got = context.render_wide_band(roi, &ctl()).unwrap();
            for (index, actual) in got.pixels().iter().enumerate() {
                let x = roi.x0 + (index % roi.width() as usize) as i32;
                let y = roi.y0 + (index / roi.width() as usize) as i32;
                let mut pixel = [0.; 4];
                expected.try_read_pixel_f64(x, y, &mut pixel).unwrap();
                assert_eq!(*actual, pixel);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN F64 physical/default/role domains and late work cleanup"]
fn f64_source_rejects_mismatched_roles_and_nonfinite_or_nonunit_physical_words() {
    let s = Rect::new(13, 9, 270, 268);
    for defect in 0..5 {
        let (d, q, g, _) = observed_depth(SampleType::F64, s, 4);
        let mut input = resident_source(d, &q, g, s);
        match defect {
            0 => input.document.source_color_context.as_mut().unwrap().source_precision = 650,
            1 => {
                input.document.xcf_state.as_mut().unwrap().filter_masks[0].surface =
                    Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false))
            }
            2 | 3 => {
                let mut raw = [0u8; 32];
                let values = if defect == 2 { [f64::INFINITY, 0., 0., 1.] } else { [0., 0., 0., 1. + 2f64.powi(-40)] };
                for (index, value) in values.into_iter().enumerate() {
                    raw[index * 8..index * 8 + 8].copy_from_slice(&value.to_ne_bytes());
                }
                let LayerContent::Raster(v) = &mut input.document.layers[0].content else { panic!() };
                // Outside S, but inside an owned physical source tile.
                v.try_write_interleaved(Rect::new(511, 511, 512, 512), &raw).unwrap();
            }
            _ => input.document.xcf_state.as_mut().unwrap().filter_masks[0]
                .surface
                .try_write_interleaved(Rect::new(511, 511, 512, 512), &2f64.powi(-150).to_ne_bytes())
                .unwrap(),
        }
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}

#[test]
#[ignore = "UNRUN assigned geometry source exact-role/both-profile custody"]
fn assigned_source_retains_original_current_and_three_distinct_curves() {
    let s = Rect::new(-7, 11, 250, 270);
    for radius in [4, 5] {
        let (d, q, g, profile) = observed_role(SampleType::F32, true, s, radius);
        let input = resident_source(d, &q, g, s);
        let trc = profile.trc.as_ref().unwrap();
        assert_ne!(trc[0], trc[1]);
        assert_ne!(trc[1], trc[2]);
        let expected = input.document.clone(); // External oracle, no producer equality claim.
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
        assert_eq!(input.document, expected);
        assert!(q.usage().unwrap().resident > baseline.resident);
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
    }
}
#[test]
#[ignore = "UNRUN assigned geometry per-channel native/linear/premultiplied/native reference"]
fn assigned_current_uses_actual_profile_and_preserves_specific_api_boundaries() {
    let s = Rect::new(13, 9, 270, 268);
    for radius in [4, 5] {
        let (d, q, g, p) = observed_role(SampleType::F32, true, s, radius);
        let owned = owner(d, q, g, true);
        assert!(crate::backed_neighborhood_context::PreparedBackedNeighborhoodContext::new_assigned_box(&owned, NormalBandLimits::default(), &ctl()).is_err());
        let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).unwrap();
        let (o, expected) = reference(owned.document().pixel_format(), s, radius as u32, &p);
        assert_eq!(context.bounds(), o);
        for requested in [Rect::new(255, 255, 273, 257), Rect::new(8, 10, 17, 13)] {
            let roi = requested.intersect(&o);
            if roi != requested {
                assert!(context.render_wide_band(requested, &ctl()).is_err());
            }
            compare(&context, roi, &expected);
        }
        // Existing shared factory's placeholder supplies no assigned authority.
        let placeholder = photocraft_cms::Builtin::SGray.profile();
        let renderer = PreparedBackedDocument::new_accounted(&owned, placeholder, NormalBandLimits::default(), &ctl()).unwrap();
        let roi = Rect::new(14, 10, 19, 12);
        let got = renderer.render_band(roi, &ctl()).unwrap();
        for (index, pixel) in got.pixels().iter().enumerate() {
            let x = roi.x0 + (index % roi.width() as usize) as i32;
            let y = roi.y0 + (index / roi.width() as usize) as i32;
            let mut expected_pixel = [0.; 4];
            expected.try_read_pixel_f64(x, y, &mut expected_pixel).unwrap();
            assert_eq!(*pixel, expected_pixel);
        }
    }
}
#[test]
#[ignore = "UNRUN assigned geometry exact current/original/mask/role refusals"]
fn assigned_source_profile_relation_and_physical_inputs_remain_authoritative() {
    let s = Rect::new(13, 9, 270, 268);
    for defect in 0..6 {
        let (d, q, g, _) = observed_role(SampleType::F32, true, s, 4);
        let mut input = resident_source(d, &q, g, s);
        match defect {
            0 => input.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap().to_vec())),
            1 => input.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            2 => input.document.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 640])),
            3 => input.document.xcf_state.as_mut().unwrap().filter_masks[0]
                .surface
                .try_write_interleaved(Rect::new(511, 511, 512, 512), &f32::from_bits(1).to_ne_bytes())
                .unwrap(),
            4 => {
                let LayerContent::Raster(v) = &mut input.document.layers[0].content else { panic!() };
                let mut raw = [0u8; 16];
                raw[..4].copy_from_slice(&f32::NAN.to_ne_bytes());
                v.try_write_interleaved(Rect::new(511, 511, 512, 512), &raw).unwrap();
            }
            _ => {
                input.document.depth = SampleType::F64;
                input.document.source_color_context.as_mut().unwrap().source_precision = 750;
            }
        }
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN assigned geometry profile peak late work/cancellation cleanup"]
fn assigned_source_profile_peaks_remain_charged_until_failure_cleanup() {
    let s = Rect::new(-7, 11, 250, 270);
    let (d, q, g, _) = observed_role(SampleType::F32, true, s, 4);
    let input = resident_source(d, &q, g, s);
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
    assert!(total > 2);
    let mut late = Budget { q: q.clone(), work: consumed - 1, deny: false };
    assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut late, &input).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    for stop in [total / 2, total - 1] {
        let observed = AtomicU64::new(0);
        let cancel = || observed.fetch_add(1, Ordering::Relaxed) >= stop;
        let interrupt = Interrupt::cancel_only(&cancel);
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &interrupt, &mut budget, &input).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
