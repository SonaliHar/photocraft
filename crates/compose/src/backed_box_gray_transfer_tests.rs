//! UNRUN exact implicit Gray transfer roles. Synthetic fixtures/reference
//! Surfaces do not qualify producer numeric or captured ICC equivalence.
use super::*;
const ROLES: [(SampleType, SampleEncoding, u32); 4] = [
    (SampleType::F32, SampleEncoding::Linear, 600),
    (SampleType::F64, SampleEncoding::Linear, 700),
    (SampleType::F32, SampleEncoding::Perceptual, 675),
    (SampleType::F64, SampleEncoding::Perceptual, 775),
];
fn transfer_fixture(
    sample: SampleType,
    stored: SampleEncoding,
    precision: u32,
    s: Rect,
    radius: i64,
    enabled: bool,
) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile, Arc<Counts>) {
    let (mut doc, q, guard, basis, counts) = gray_fixture(sample, s, radius, enabled);
    doc.sample_encoding = Some(stored);
    doc.source_color_context.as_mut().unwrap().source_precision = precision;
    // External fixture generation; production separately prepays its controlled
    // requirements/constructor and binds the actual encoded current marker.
    let effective = photocraft_cms::xcf_color::effective_profile_from_definition(&basis, crate::gimp_composite::encoding(stored)).unwrap();
    let encoded = effective.encoded_bytes().unwrap().to_vec();
    doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(&encoded);
    doc.icc_profile = Some(Arc::new(encoded));
    (doc, q, guard, basis, counts)
}
fn transfer_reference(sample: SampleType, stored: SampleEncoding, s: Rect, radius: u32, coordinate: photocraft_geom::TileCoord) -> (Rect, Surface) {
    let source = gray_surface(photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, true), s);
    let t = coordinate.rect().intersect(&n::box_blur_extent(s, radius).unwrap());
    let curve = photocraft_cms::curve::srgb_trc();
    let native = source.try_read_region_f64(s).unwrap();
    let mut rgba = Vec::new();
    for pixel in native.chunks_exact(2) {
        let gray = if stored == SampleEncoding::Linear { pixel[0] } else { curve.eval64(pixel[0]) };
        rgba.extend_from_slice(&[gray, gray, gray, pixel[1]]);
    }
    let mut linear = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    linear.try_write_region_f64(s, &rgba).unwrap();
    let req = n::box_blur_requirements(&linear, t, s, radius, &Interrupt::NONE).unwrap();
    let filtered =
        n::try_box_blur(&linear, t, s, radius, n::GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units }, &Interrupt::NONE)
            .unwrap();
    let mut pixels = filtered.try_read_region_f64(t).unwrap();
    for pixel in pixels.chunks_exact_mut(4) {
        let gray = if stored == SampleEncoding::Linear { pixel[0] } else { curve.eval_inverse64(pixel[0]) };
        pixel[..3].copy_from_slice(&[gray; 3]);
    }
    let mut out = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    out.try_write_region_f64(t, &pixels).unwrap();
    (t, out)
}

#[test]
#[ignore = "UNRUN four exact Gray source roles retain actual encoded profile and full inactive preparation"]
fn exact_source_roles_separate_current_profile_from_known_basis() {
    let s = Rect::new(13, 9, 18, 12);
    for (sample, stored, precision) in ROLES {
        for radius in [0, 4, 5, 1000] {
            let (doc, q, guard, basis, _) = transfer_fixture(sample, stored, precision, s, radius, true);
            assert_eq!(doc.pixel_format().channels(), 2);
            assert_ne!(doc.icc_profile.as_deref().unwrap().as_slice(), basis.encoded_bytes().unwrap());
            let mut input = gray_source(doc, &q, guard, s);
            let original = input.document.clone(); // External expected model/native owner.
            let baseline = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
            let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
            assert!(q.usage().unwrap().resident > baseline.resident);
            assert_eq!(input.document, original);
            drop(prepared);
            assert_eq!(q.usage().unwrap(), baseline);
            gray_activity(&mut input.document, false);
            let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
            assert!(q.usage().unwrap().resident > baseline.resident);
            drop(prepared);
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
}

#[test]
#[ignore = "UNRUN four Gray CURRENT roles preserve actual transfer and canonical packed/row/zero schedule"]
fn current_uses_role_transfer_and_keeps_native_identity_separate() {
    let s = Rect::new(-3, 5, 2, 8);
    for (sample, stored, precision) in ROLES {
        for radius in [0, 1, 4, 5, 127, 1000] {
            let (doc, q, guard, _, _) = transfer_fixture(sample, stored, precision, s, radius, true);
            assert!(is_candidate(&doc, &ctl()).unwrap());
            let owned = owner(doc, q, guard, false);
            let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits { max_rows: 2, max_pixels: 17 }, &ctl()).unwrap();
            let (tile, expected) = transfer_reference(sample, stored, s, radius as u32, photocraft_geom::TileCoord::containing(-1, 6));
            let roi = Rect::new(tile.x1 - 1, tile.y0, tile.x1, tile.y0 + 1);
            compare(&context, roi, &expected);
            let actual = context.render_wide_band(roi, &ctl()).unwrap();
            for pixel in actual.pixels() {
                assert_eq!(pixel[0].to_bits(), pixel[1].to_bits());
                assert_eq!(pixel[0].to_bits(), pixel[2].to_bits());
            }
            drop(PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::SGray.profile(), Default::default(), &ctl()).unwrap());
            let effective =
                photocraft_cms::xcf_color::effective_profile_from_definition(photocraft_cms::Builtin::SGray.profile(), crate::gimp_composite::encoding(stored))
                    .unwrap();
            assert!(PreparedBackedDocument::new_accounted(&owned, &effective, Default::default(), &ctl()).is_err());
            assert!(PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()).is_err());
        }
        let (mut doc, q, guard, _, _) = transfer_fixture(sample, stored, precision, s, 1000, false);
        gray_activity(&mut doc, false);
        let owned = owner(doc, q, guard, false);
        let context = PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).unwrap();
        assert_eq!(context.bounds(), s);
        let band = context.render_wide_band(s, &ctl()).unwrap();
        let native = gray_surface(owned.document().pixel_format(), s).try_read_region_f64(s).unwrap();
        for (actual, expected) in band.pixels().iter().zip(native.chunks_exact(2)) {
            assert_eq!(actual.map(f64::to_bits), [expected[0], expected[0], expected[0], expected[1]].map(f64::to_bits));
        }
    }
}

#[test]
#[ignore = "UNRUN inactive Gray transfer current/source profiles masks native alpha and role mismatches refuse"]
fn inactive_transfer_does_not_turn_a_hash_or_model_hint_into_proof() {
    let s = Rect::new(13, 9, 18, 12);
    for (sample, stored, precision) in ROLES {
        for defect in 0..6 {
            let (mut doc, q, guard, basis, _) = transfer_fixture(sample, stored, precision, s, 0, false);
            gray_activity(&mut doc, false);
            let mut input = gray_source(doc, &q, guard, s);
            match defect {
                0 => {
                    let bytes = basis.encoded_bytes().unwrap().to_vec();
                    input.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(&bytes);
                    input.document.icc_profile = Some(Arc::new(bytes)); // Coherent hash, wrong effective role.
                }
                1 => input.document.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 640])),
                2 => input.document.source_color_context.as_mut().unwrap().source_precision = if sample == SampleType::F32 { 650 } else { 750 },
                3 | 4 => {
                    let LayerContent::Raster(surface) = &mut input.document.layers[0].content else { panic!() };
                    surface.try_write_region_f64(Rect::new(17, 11, 18, 12), if defect == 3 { &[f64::INFINITY, 0.5] } else { &[0.5, 2.] }).unwrap();
                }
                _ => {
                    let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false);
                    let mut bytes = [0; 8];
                    photocraft_color::try_write_sample_f64(&mut bytes, sample, 0, 0.5).unwrap();
                    input.document.xcf_state.as_mut().unwrap().filter_masks[0].surface =
                        Surface::try_with_default_bytes(format, &bytes[..sample.bytes()]).unwrap();
                }
            }
            let baseline = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
            assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
        }
        let (mut doc, q, guard, _, counts) = transfer_fixture(sample, stored, precision, s, 0, false);
        gray_activity(&mut doc, false);
        counts.bad.store(true, Ordering::Relaxed);
        let owned = owner(doc, q, guard, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
        let (mut doc, q, guard, basis, _) = transfer_fixture(sample, stored, precision, s, 0, false);
        gray_activity(&mut doc, false);
        let bytes = basis.encoded_bytes().unwrap().to_vec();
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(&bytes);
        doc.icc_profile = Some(Arc::new(bytes));
        let owned = owner(doc, q, guard, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
    }
}

#[test]
#[ignore = "UNRUN four Gray transfer role preparation cumulative work cancellation and quota cleanup"]
fn transfer_controls_hold_full_profile_and_source_custody_until_cleanup() {
    let s = Rect::new(-3, 5, 2, 8);
    for (sample, stored, precision) in ROLES {
        let (doc, q, guard, _, _) = transfer_fixture(sample, stored, precision, s, 127, true);
        let input = gray_source(doc, &q, guard, s);
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
        let cancelled = || seen.fetch_add(1, Ordering::Relaxed) >= total - 1;
        let interrupt = Interrupt::cancel_only(&cancelled);
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
