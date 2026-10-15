//! UNRUN Gray-native consumers. Generated stores and reference Surfaces are
//! independent harness controls, not GIMP numerical/profile equivalence proof.
#[path = "backed_box_gray_factory_tests.rs"]
mod factory;
use super::*;

#[derive(Debug)]
struct GrayPattern {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    counts: Arc<Counts>,
    _fee: QuotaLease,
}
fn gray_values(x: i32, y: i32) -> [f64; 2] {
    let gray = match (x, y) {
        (14, 10) => -0.0,
        (15, 10) => 1. + 2f64.powi(-40),
        (16, 10) => -2f64.powi(-150),
        _ => ((x + 2 * y).rem_euclid(29) as f64) / 16. - 0.125,
    };
    [gray, if (x + y).rem_euclid(7) == 0 { 0. } else { 0.5 }]
}
fn gray_encoded(format: photocraft_color::PixelFormat, x: i32, y: i32) -> [u8; 16] {
    let mut bytes = [0; 16];
    let mut values = gray_values(x, y);
    if format.sample == SampleType::F32 {
        if (x, y) == (15, 10) {
            values[0] = 1. + 2f64.powi(-20);
        }
        if (x, y) == (16, 10) {
            values[0] = -2f64.powi(-100);
        }
    }
    for (channel, value) in values.into_iter().enumerate() {
        photocraft_color::try_write_sample_f64(&mut bytes, format.sample, channel, value).unwrap();
    }
    bytes
}
impl NativePlaneStore for GrayPattern {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.counts.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        if at.checked_add(out.len() as u64).ok_or(BackingError::Length)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        self.counts.reads.fetch_add(1, Ordering::Relaxed);
        self.counts.bytes.fetch_add(out.len() as u64, Ordering::Relaxed);
        let bounds = self.descriptor.bounds();
        let format = self.descriptor.format();
        let bpp = format.bytes_per_pixel() as u64;
        let mut previous = None;
        let mut pixel = [0; 16];
        for (i, byte) in out.iter_mut().enumerate() {
            if i.is_multiple_of(4096) {
                control.check(4096)?;
            }
            let offset = at.checked_add(i as u64).ok_or(BackingError::Length)?;
            let index = offset / bpp;
            if previous != Some(index) {
                let x = i32::try_from(i64::from(bounds.x0) + (index % u64::from(bounds.width())) as i64).map_err(|_| BackingError::Bounds)?;
                let y = i32::try_from(i64::from(bounds.y0) + (index / u64::from(bounds.width())) as i64).map_err(|_| BackingError::Bounds)?;
                pixel = gray_encoded(format, x, y);
                if self.counts.bad.load(Ordering::Relaxed) && index + 1 == u64::from(bounds.width()) * u64::from(bounds.height()) {
                    photocraft_color::try_write_sample_f64(&mut pixel, format.sample, 0, f64::NAN).map_err(|_| BackingError::Integrity)?;
                }
                previous = Some(index);
            }
            *byte = *pixel.get((offset % bpp) as usize).ok_or(BackingError::Bounds)?;
        }
        Ok(())
    }
}
// Root's nested factory child borrows this private fixture API. The native
// descriptor and provider really are Gray+Alpha, not a relabelled RGB store.
fn gray_fixture(
    sample: SampleType,
    s: Rect,
    radius: i64,
    enabled: bool,
) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile, Arc<Counts>) {
    let (mut doc, q, guard, _) = observed_depth(sample, s, radius);
    doc.mode = ColorMode::Grayscale;
    let basis = photocraft_cms::Builtin::SGray.profile().clone();
    let encoded = basis.encoded_bytes().unwrap().to_vec();
    doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(&encoded);
    doc.icc_profile = Some(Arc::new(encoded));
    let format = doc.pixel_format();
    assert_eq!(format.channels(), 2);
    let descriptor = NativePlaneDescriptor::new(format, s, &[0; 16][..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, std::mem::size_of::<GrayPattern>() as u64 + 256).unwrap();
    let counts = Arc::new(Counts::default());
    let store = Arc::new(GrayPattern { descriptor, quota: q.clone(), counts: counts.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = enabled;
    (doc, q, guard, basis, counts)
}
fn gray_surface(format: photocraft_color::PixelFormat, s: Rect) -> Surface {
    let mut bytes = Vec::new();
    for y in s.y0..s.y1 {
        for x in s.x0..s.x1 {
            bytes.extend_from_slice(&gray_encoded(format, x, y)[..format.bytes_per_pixel()]);
        }
    }
    Surface::try_from_interleaved(format, s, &bytes).unwrap()
}
fn gray_source(doc: Document, q: &BackingQuota, guard: Arc<dyn std::any::Any + Send + Sync>, s: Rect) -> ResidentSource {
    let bpp = doc.pixel_format().bytes_per_pixel() as u64;
    let count = s.tiles().count() as u64;
    let physical = count.checked_mul(256 * 256 * bpp).unwrap();
    let raw = u64::from(s.width()).checked_mul(u64::from(s.height())).unwrap().checked_mul(bpp).unwrap();
    let peak = physical
        .checked_mul(2)
        .unwrap()
        .checked_add(raw.checked_mul(2).unwrap())
        .unwrap()
        .checked_add(2 * 256 * 256 * bpp + count * 1024 + 65536 + std::mem::size_of::<ResidentSource>() as u64)
        .unwrap();
    let fee = q.reserve(QuotaKind::Resident, peak).unwrap();
    let mut input = ResidentSource { document: doc, _original_guard: guard, _resident_lease: fee };
    let surface = gray_surface(input.document.pixel_format(), s);
    let retained = surface.resident_accounted_bytes().unwrap() + std::mem::size_of::<ResidentSource>() as u64 + 1024;
    assert!(retained <= peak);
    input.document.layers[0].content = LayerContent::Raster(surface);
    input._resident_lease.reconcile_capacity(retained).unwrap();
    input
}
fn gray_activity(doc: &mut Document, enabled: bool) {
    let id = doc.layers[0].id;
    doc.layers[0].visible = false;
    doc.layers[0].opacity = -0.0;
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap().visible = false;
    state.filter_stacks[0].nodes[0].enabled = enabled;
}
fn gray_reference(sample: SampleType, s: Rect, radius: u32, coordinate: photocraft_geom::TileCoord, profile: &Profile) -> (Rect, Surface) {
    let source = gray_surface(photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, true), s);
    let t = coordinate.rect().intersect(&n::box_blur_extent(s, radius).unwrap());
    let curve = profile.gray_trc.as_ref().unwrap();
    let native = source.try_read_region_f64(s).unwrap();
    let mut rgba = Vec::new();
    for pixel in native.chunks_exact(2) {
        let gray = curve.eval64(pixel[0]);
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
        let gray = curve.eval_inverse64(pixel[0]);
        pixel[..3].copy_from_slice(&[gray; 3]);
    }
    let mut out = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    out.try_write_region_f64(t, &pixels).unwrap();
    (t, out)
}

#[test]
#[ignore = "UNRUN source real YA8/16 words SGray basis/profile custody and inactive full preparation"]
fn source_keeps_two_native_channels_and_pays_sgray_preparation() {
    let s = Rect::new(13, 9, 18, 12);
    for sample in [SampleType::F32, SampleType::F64] {
        for radius in [0, 4, 5, 1000] {
            let (doc, q, guard, basis, _) = gray_fixture(sample, s, radius, true);
            let mut input = gray_source(doc, &q, guard, s);
            assert!(basis.gray_trc.is_some() && basis.trc.is_none());
            let format = input.document.pixel_format();
            assert_eq!(format.channels(), 2);
            assert_eq!(format.bytes_per_pixel(), sample.bytes() * 2);
            let LayerContent::Raster(surface) = &input.document.layers[0].content else { panic!() };
            for (x, y) in [(14, 10), (15, 10), (16, 10)] {
                let mut pixel = [0.; 2];
                surface.try_read_pixel_f64(x, y, &mut pixel).unwrap();
                let raw = gray_encoded(format, x, y);
                for channel in 0..2 {
                    assert_eq!(pixel[channel].to_bits(), photocraft_color::try_read_sample_f64(&raw, sample, channel).unwrap().to_bits());
                }
            }
            let original = input.document.clone(); // External expected metadata/native owner.
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
#[ignore = "UNRUN Gray forward/premultiplied RGBA core/inverse repeated-gray canonical output"]
fn current_repeats_gray_only_inside_working_and_display_lanes() {
    let s = Rect::new(-3, 5, 2, 8);
    for sample in [SampleType::F32, SampleType::F64] {
        for radius in [0, 1, 4, 5, 127, 1000] {
            let (doc, q, guard, basis, _) = gray_fixture(sample, s, radius, true);
            assert!(is_candidate(&doc, &ctl()).unwrap());
            let owned = owner(doc, q, guard, false);
            let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits { max_rows: 2, max_pixels: 17 }, &ctl()).unwrap();
            let (tile, reference) = gray_reference(sample, s, radius as u32, photocraft_geom::TileCoord::containing(-1, 6), &basis);
            let roi = Rect::new(tile.x1 - 1, tile.y0, tile.x1, tile.y0 + 1);
            compare(&context, roi, &reference);
            for pixel in context.render_wide_band(roi, &ctl()).unwrap().pixels() {
                assert_eq!(pixel[0].to_bits(), pixel[1].to_bits());
                assert_eq!(pixel[0].to_bits(), pixel[2].to_bits());
            }
            assert_eq!(owned.document().pixel_format().channels(), 2);
        }
        let (mut doc, q, guard, _, _) = gray_fixture(sample, s, 1000, false);
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
#[ignore = "UNRUN Gray inactive native/default/alpha1/mask/profile proofs and unsupported role boundaries"]
fn malformed_gray_inputs_refuse_even_when_inactive() {
    let s = Rect::new(13, 9, 18, 12);
    for sample in [SampleType::F32, SampleType::F64] {
        for defect in 0..7 {
            let (mut doc, q, guard, _, _) = gray_fixture(sample, s, 4, false);
            gray_activity(&mut doc, false);
            let mut input = gray_source(doc, &q, guard, s);
            match defect {
                0 => input.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap().to_vec())),
                1 => {
                    input.document.working_profile_interpretation =
                        photocraft_doc::WorkingProfileInterpretation::implicit_srgb(input.document.icc_profile.as_ref().unwrap())
                }
                2 => input.document.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 640])),
                3 | 4 => {
                    let LayerContent::Raster(surface) = &mut input.document.layers[0].content else { panic!() };
                    let pixel = if defect == 3 { [f64::NAN, 0.5] } else { [0.5, 2.] };
                    surface.try_write_region_f64(Rect::new(17, 11, 18, 12), &pixel).unwrap();
                }
                5 => {
                    let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false);
                    let mut raw = [0; 8];
                    photocraft_color::try_write_sample_f64(&mut raw, sample, 0, 0.5).unwrap();
                    input.document.xcf_state.as_mut().unwrap().filter_masks[0].surface =
                        Surface::try_with_default_bytes(format, &raw[..sample.bytes()]).unwrap();
                }
                _ => {
                    let format = input.document.pixel_format();
                    let mut raw = [0; 16];
                    photocraft_color::try_write_sample_f64(&mut raw, sample, 0, f64::INFINITY).unwrap();
                    input.document.layers[0].content = LayerContent::Raster(Surface::try_with_default_bytes(format, &raw[..format.bytes_per_pixel()]).unwrap());
                }
            }
            let baseline = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
            assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
        }
        let (mut doc, q, guard, _, counts) = gray_fixture(sample, s, 4, false);
        gray_activity(&mut doc, false);
        counts.bad.store(true, Ordering::Relaxed);
        let owned = owner(doc, q, guard, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
        let (mut doc, q, guard, _, _) = gray_fixture(sample, s, 4, false);
        gray_activity(&mut doc, false);
        let mask_format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false);
        let mut raw = [0; 8];
        photocraft_color::try_write_sample_f64(&mut raw, sample, 0, 0.5).unwrap();
        doc.xcf_state.as_mut().unwrap().filter_masks[0].surface = Surface::try_with_default_bytes(mask_format, &raw[..sample.bytes()]).unwrap();
        let owned = owner(doc, q, guard, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
        // This intentionally malformed model retains a genuine RGB provider;
        // changing only document metadata cannot grant Gray native custody.
        let (mut doc, q, guard, _, _) = fixture(sample, false, s, Rect::new(0, 0, 283, 281), 4, true);
        doc.mode = ColorMode::Grayscale;
        let gray = photocraft_cms::Builtin::SGray.profile().encoded_bytes().unwrap().to_vec();
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(&gray);
        doc.icc_profile = Some(Arc::new(gray));
        assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).is_err());
        for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
            let (mut doc, q, guard, _, _) = gray_fixture(sample, s, 4, true);
            doc.sample_encoding = Some(encoding);
            doc.source_color_context.as_mut().unwrap().source_precision = match (sample, encoding) {
                (SampleType::F32, SampleEncoding::Linear) => 600,
                (SampleType::F64, SampleEncoding::Linear) => 700,
                (SampleType::F32, _) => 675,
                _ => 775,
            };
            assert!(!is_candidate(&doc, &ctl()).unwrap());
            let owned = owner(doc, q, guard, false);
            assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
        }
        let (mut doc, q, guard, _, _) = gray_fixture(sample, Rect::new(0, 0, 16, 12), 4, true);
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.schema_epoch = crate::gimp_box::EPOCH.into();
        node.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_box::EPOCH.into();
        assert!(!is_candidate(&doc, &ctl()).unwrap());
        let owned = owner(doc, q, guard, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
    }
}

#[test]
#[ignore = "UNRUN Gray source profile/control refunds and row callback actual provider error"]
fn gray_controls_preserve_source_custody_and_typed_errors() {
    for sample in [SampleType::F32, SampleType::F64] {
        let s = Rect::new(-3, 5, 2, 8);
        let (doc, q, guard, _, _) = gray_fixture(sample, s, 127, true);
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
    let s = Rect::new(-7, 11, 250, 270);
    let (doc, q, guard, _, counts) = gray_fixture(SampleType::F64, s, 127, true);
    let owned = owner(doc, q, guard, false);
    let context = PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).unwrap();
    let roi = Rect::new(-2, 12, 2, 14);
    drop(context.render_wide_band(roi, &ctl()).unwrap());
    let baseline = owned.quota().usage().unwrap();
    counts.fail.store(true, Ordering::Relaxed);
    assert!(matches!(context.render_wide_band(roi, &ctl()), Err(E::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Io)))));
    assert_eq!(owned.quota().usage().unwrap(), baseline);
    counts.fail.store(false, Ordering::Relaxed);
}
