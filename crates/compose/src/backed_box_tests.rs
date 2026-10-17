//! UNRUN streamed CURRENT Box consumers. Generated lazy providers here are
//! foundation fixtures, not a claim of lazy XCF ingestion or new source evidence.
//! Reference Surfaces/Profile clones are external test harness allocations.
#[path = "backed_box_source_tests.rs"]
mod observed_source;
use super::*;
use crate::{
    backed_box_context::{PreparedBackedBoxContext, is_candidate},
    backed_box_document::PreparedBackedBoxDocument,
    backed_document::PreparedBackedDocument,
    gimp_composite::WorkingSpace,
};
use photocraft_algo::gimp::neighborhood as n;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[derive(Debug, Default)]
struct Counts {
    bytes: AtomicU64,
    reads: AtomicU64,
    fail: AtomicBool,
    bad: AtomicBool,
}
#[derive(Debug)]
struct Pattern {
    descriptor: NativePlaneDescriptor,
    q: BackingQuota,
    counts: Arc<Counts>,
    _fee: QuotaLease,
}
fn values(x: i32, y: i32) -> [f64; 4] {
    [
        (x.rem_euclid(29) as f64) / 16. - 0.125,
        (y.rem_euclid(31) as f64) / 32.,
        ((x + y).rem_euclid(17) as f64) / 12.,
        if (x + y).rem_euclid(7) == 0 { 0. } else { 0.5 },
    ]
}
fn encoded(format: photocraft_color::PixelFormat, x: i32, y: i32) -> [u8; 40] {
    let mut bytes = [0; 40];
    for (c, v) in values(x, y).into_iter().enumerate() {
        photocraft_color::try_write_sample_f64(&mut bytes, format.sample, c, v).unwrap();
    }
    bytes
}
impl NativePlaneStore for Pattern {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.counts.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        if at.checked_add(out.len() as u64).ok_or(BackingError::Length)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        self.counts.reads.fetch_add(1, Ordering::Relaxed);
        self.counts.bytes.fetch_add(out.len() as u64, Ordering::Relaxed);
        let format = self.descriptor.format();
        let bpp = format.bytes_per_pixel() as u64;
        let s = self.descriptor.bounds();
        let mut last = None;
        let mut pixel = [0; 40];
        for (i, byte) in out.iter_mut().enumerate() {
            if i.is_multiple_of(4096) {
                c.check(4096)?;
            }
            let offset = at.checked_add(i as u64).ok_or(BackingError::Length)?;
            let index = offset / bpp;
            if last != Some(index) {
                let x = i64::from(s.x0) + (index % u64::from(s.width())) as i64;
                let y = i64::from(s.y0) + (index / u64::from(s.width())) as i64;
                pixel = encoded(format, i32::try_from(x).map_err(|_| BackingError::Bounds)?, i32::try_from(y).map_err(|_| BackingError::Bounds)?);
                if self.counts.bad.load(Ordering::Relaxed) && index + 1 == u64::from(s.width()) * u64::from(s.height()) {
                    photocraft_color::try_write_sample_f64(&mut pixel, format.sample, 0, f64::NAN).map_err(|_| BackingError::Integrity)?;
                }
                last = Some(index);
            }
            *byte = *pixel.get((offset % bpp) as usize).ok_or(BackingError::Bounds)?;
        }
        Ok(())
    }
}
fn fixture(
    sample: SampleType,
    assigned_profile: bool,
    s: Rect,
    canvas: Rect,
    radius: i64,
    enabled: bool,
) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile, Arc<Counts>) {
    let (mut doc, q, guard, profile) = if assigned_profile {
        let (d, q, g, p, _) = assigned(4, false, 0);
        (d, q, g, p)
    } else {
        let (d, q, g, p, _) = model(4);
        (d, q, g, (*p).clone())
    };
    let extra = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    doc.depth = sample;
    doc.size = Size::new(canvas.width(), canvas.height());
    doc.source_color_context.as_mut().unwrap().source_precision = if sample == SampleType::F32 { 650 } else { 750 };
    let format = doc.pixel_format();
    let descriptor = NativePlaneDescriptor::new(format, s, &[0; 40][..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, std::mem::size_of::<Pattern>() as u64 + 256).unwrap();
    let counts = Arc::new(Counts::default());
    let store = Arc::new(Pattern { descriptor, q: q.clone(), counts: counts.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    let layer = doc.layers[0].id;
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_masks[0].bounds = canvas;
    state.filter_masks[0].surface = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false));
    for item in &mut state.items {
        if item.item == ItemRef::Layer(layer) {
            item.raster_bounds = Some(s);
        } else {
            item.raster_bounds = Some(canvas);
            if let Some(mask) = &mut item.mask {
                mask.bounds = canvas;
                mask.offset = [0, 0];
            }
        }
    }
    state.filter_stacks[0].nodes[0].arguments = vec![i("radius", radius)];
    state.filter_stacks[0].nodes[0].enabled = enabled;
    (doc, q, Arc::new((guard, extra)), profile, counts)
}
fn owner(doc: Document, q: BackingQuota, g: Arc<dyn std::any::Any + Send + Sync>, assigned: bool) -> AccountedDocument {
    if assigned { adopt_assigned(doc, q, g) } else { adopt(doc, q, g) }
}
fn resident(format: photocraft_color::PixelFormat, s: Rect) -> Surface {
    let mut raw = Vec::new();
    for y in s.y0..s.y1 {
        for x in s.x0..s.x1 {
            raw.extend_from_slice(&encoded(format, x, y)[..format.bytes_per_pixel()]);
        }
    }
    Surface::try_from_interleaved(format, s, &raw).unwrap()
}
fn reference(format: photocraft_color::PixelFormat, s: Rect, radius: u32, profile: &Profile) -> (Rect, Surface) {
    let source = resident(format, s);
    let o = n::box_blur_extent(s, radius).unwrap();
    let space = WorkingSpace::new_with_interrupt(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE).unwrap();
    let mut samples = source.try_read_region_f64(s).unwrap();
    for p in samples.as_chunks_mut::<4>().0 {
        let rgb = space.convert([p[0], p[1], p[2]], true).unwrap();
        p[..3].copy_from_slice(&rgb);
    }
    let mut linear = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    linear.try_write_region_f64(s, &samples).unwrap();
    let req = n::box_blur_requirements(&linear, o, s, radius, &Interrupt::NONE).unwrap();
    let filtered =
        n::try_box_blur(&linear, o, s, radius, n::GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units }, &Interrupt::NONE)
            .unwrap();
    let mut samples = filtered.try_read_region_f64(o).unwrap();
    for p in samples.as_chunks_mut::<4>().0 {
        let rgb = space.convert([p[0], p[1], p[2]], false).unwrap();
        p[..3].copy_from_slice(&rgb);
    }
    let mut out = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    out.try_write_region_f64(o, &samples).unwrap();
    (o, out)
}
fn compare(context: &PreparedBackedBoxContext<'_>, rect: Rect, expected: &Surface) {
    let out = context.render_wide_band(rect, &ctl()).unwrap();
    let mut index = 0;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let mut p = [0.; 4];
            expected.try_read_pixel_f64(x, y, &mut p).unwrap();
            assert_eq!(out.pixels()[index].map(f64::to_bits), p.map(f64::to_bits), "{x},{y}");
            index += 1;
        }
    }
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn float_depths_profiles_offsets_and_radii_match_complete_old_tile_schedule() {
    for (sample, a) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for radius in [4, 5] {
            let s = Rect::new(-7, 11, 10, 24);
            let c = Rect::new(0, 0, 41, 37);
            let (doc, q, g, p, _) = fixture(sample, a, s, c, radius, true);
            let format = doc.pixel_format();
            let owned = owner(doc, q, g, a);
            let context = PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).unwrap();
            context.validate_operative(&ctl()).unwrap();
            let (o, expected) = reference(format, s, radius as u32, &p);
            assert_eq!(context.bounds(), o);
            compare(&context, o, &expected);
        }
    }
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn thin_reordered_roi_and_canvas_crop_preserve_full_canonical_tile_resets_and_clamp_s() {
    let s = Rect::new(13, 9, 270, 268);
    let c = Rect::new(0, 0, 301, 291);
    let (doc, q, g, p, _) = fixture(SampleType::F64, false, s, c, 5, true);
    let format = doc.pixel_format();
    let owned = owner(doc, q, g, false);
    let limits = NormalBandLimits { max_rows: 3, max_pixels: 37 };
    let context = PreparedBackedBoxContext::new(&owned, limits, &ctl()).unwrap();
    let (o, expected) = reference(format, s, 5, &p);
    for roi in [Rect::new(253, 254, 265, 257), Rect::new(8, 4, 15, 7), Rect::new(267, 268, 275, 271), Rect::new(253, 254, 265, 257)] {
        assert!(o.contains_rect(&roi));
        compare(&context, roi, &expected);
    }
    assert!(context.render_wide_band(Rect::new(o.x0, o.y0, o.x0 + 38, o.y0 + 1), &ctl()).is_err());
    let (doc, q, g, _, _) = fixture(SampleType::F32, false, Rect::new(13, 9, 14, 10), c, 4, true);
    let owned = owner(doc, q, g, false);
    let bridge = PreparedBackedBoxDocument::new(&owned, Default::default(), &ctl()).unwrap();
    let band = bridge.render_band(Rect::new(8, 4, 20, 16), &ctl()).unwrap();
    let mut index = 0;
    for y in 4..16 {
        for x in 8..20 {
            let p = band.pixels()[index];
            if (9..18).contains(&x) && (5..14).contains(&y) {
                assert_eq!(p[3], 0.5);
            } else {
                assert_eq!(p.map(f64::to_bits), [0u64; 4]);
            }
            index += 1;
        }
    }
    let s = Rect::new(-3, 2, 15, 17);
    let (doc, q, g, _, _) = fixture(SampleType::F32, true, s, c, 4, false);
    let owned = owner(doc, q, g, true);
    let context = PreparedBackedBoxContext::new(&owned, limits, &ctl()).unwrap();
    assert_eq!(context.bounds(), s);
    let source = resident(owned.document().pixel_format(), s);
    compare(&context, Rect::new(-3, 2, 4, 5), &source);
}
#[test]
#[ignore = "UNRUN: potentially expensive 24/36MP foundation provider; no XCF lazy-ingestion claim"]
fn large_lazy_f64_provider_uses_bounded_extra_memory_with_honest_full_input_validation() {
    for (w, h) in [(6000, 4000), (6000, 6000)] {
        let s = Rect::new(0, 0, w, h);
        let (doc, q, g, _, counts) = fixture(SampleType::F64, false, s, s, 5, true);
        let owned = owner(doc, q.clone(), g, false);
        let baseline = q.usage().unwrap();
        let peak = AtomicU64::new(baseline.resident);
        let work = AtomicU64::new(0);
        let charge = |n| {
            work.fetch_add(n, Ordering::Relaxed);
            peak.fetch_max(q.usage().map_err(|_| BackingError::Quota)?.resident, Ordering::Relaxed);
            Ok(())
        };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
        let context = PreparedBackedBoxContext::new(&owned, Default::default(), &control).unwrap();
        assert!(counts.bytes.load(Ordering::Relaxed) >= u64::from(s.width()) * u64::from(s.height()) * 32);
        // Exceeds the wasm512MiB resident cap; native's2GiB cap is larger.
        assert!(u64::from(s.width()) * u64::from(s.height()) * 32 > (512u64 << 20));
        let before = work.load(Ordering::Relaxed);
        let roi = Rect::new(w - 11, h - 11, w, h);
        drop(context.render_wide_band(roi, &control).unwrap());
        let after = work.load(Ordering::Relaxed);
        assert!(after > before);
        drop(context.render_wide_band(roi, &control).unwrap());
        assert!(work.load(Ordering::Relaxed) > after);
        // The provider's <=two simultaneous physical cache tiles are separately paid
        // persistent input custody.32MiB here is renderer/request memory, not total.
        assert!(peak.load(Ordering::Relaxed) - baseline.resident <= (32 << 20) + 2 * 256 * 256 * 32);
    }
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn provider_quota_work_kernel_interrupt_and_inverse_failures_publish_no_partial_band() {
    let s = Rect::new(13, 9, 270, 268);
    let (doc, q, g, _, counts) = fixture(SampleType::F32, false, s, Rect::new(0, 0, 301, 291), 5, true);
    let owned = owner(doc, q.clone(), g, false);
    let context = PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).unwrap();
    let roi = Rect::new(8, 4, 20, 16);
    drop(context.render_wide_band(roi, &ctl()).unwrap());
    let baseline = q.usage().unwrap();
    let blocked = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(context.render_wide_band(roi, &ctl()).is_err());
    drop(blocked);
    assert_eq!(q.usage().unwrap(), baseline);
    let tile = n::BoxBlurTilePlan::new(s, photocraft_geom::TileCoord::containing(roi.x0, roi.y0), 5, &Interrupt::NONE).unwrap();
    let req = tile.requirements(&Interrupt::NONE).unwrap();
    let armed = AtomicBool::new(false);
    let inverse_fail = |units| {
        if units == req.work_units {
            armed.store(true, Ordering::Relaxed);
        }
        if armed.load(Ordering::Relaxed) && units == 4 + 3 * 128 { Err(BackingError::Work) } else { Ok(()) }
    };
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &inverse_fail };
    assert!(matches!(context.render_wide_band(roi, &denied), Err(E::Backing(BackingError::Work))));
    assert!(armed.load(Ordering::Relaxed));
    assert_eq!(q.usage().unwrap(), baseline);
    let armed = AtomicBool::new(false);
    let polls = AtomicU64::new(0);
    let cancel = || armed.load(Ordering::Relaxed) && polls.fetch_add(1, Ordering::Relaxed) >= 12;
    let interrupt = Interrupt::cancel_only(&cancel);
    let charge = |units| {
        if units == req.work_units {
            armed.store(true, Ordering::Relaxed);
        }
        Ok(())
    };
    let control = ReadControl { interrupt: &interrupt, work: &charge };
    assert!(matches!(context.render_wide_band(roi, &control), Err(E::Backing(BackingError::Cancelled))));
    assert!(polls.load(Ordering::Relaxed) >= 12);
    assert_eq!(q.usage().unwrap(), baseline);
    counts.fail.store(true, Ordering::Relaxed);
    let other = Rect::new(257, 257, 265, 265);
    assert!(matches!(context.render_wide_band(other, &ctl()), Err(E::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Io)))));
    counts.fail.store(false, Ordering::Relaxed);
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn current_selector_has_no_source_permission_and_preserves_exact_profile_recipe_and_all_input_proof() {
    let s = Rect::new(-7, 11, 10, 24);
    let c = Rect::new(0, 0, 41, 37);
    let (doc, q, g, _, _) = fixture(SampleType::F32, false, s, c, 4, true);
    assert!(is_candidate(&doc, &ctl()).unwrap());
    let owned = owner(doc, q, g, false);
    assert!(PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::SGray.profile(), Default::default(), &ctl()).is_err());
    assert!(PreparedBackedNeighborhoodContext::new(&owned, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()).is_err());
    let current_control = ctl();
    let mut budget = Budget { q: owned.quota(), c: &current_control };
    assert!(LeasedStockNeighborhoodPipeline::new_budgeted(owned.document(), &Interrupt::NONE, &mut budget, ()).is_err());
    let (doc, q, g, _, _) = fixture(SampleType::F32, true, s, c, 4, true);
    let owned = owner(doc, q, g, true);
    let renderer = PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::SGray.profile(), Default::default(), &ctl()).unwrap();
    drop(renderer.render_band(c, &ctl()).unwrap());
    for case in 0..7 {
        let (mut doc, q, g, _, counts) = fixture(SampleType::F32, true, s, c, 4, false);
        doc.layers[0].visible = false;
        let state = doc.xcf_state.as_mut().unwrap();
        state.items.iter_mut().find(|i| i.item == ItemRef::Layer(doc.layers[0].id)).unwrap().visible = false;
        match case {
            0 => doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()),
            1 => state.filter_masks[0].surface.try_write_pixel_f64(40, 36, &[1.]).unwrap(),
            2 => state.filter_stacks[0].nodes[0].arguments = vec![i("radius", 6)],
            3 => {
                let p = state.filter_stacks[0].nodes[0].composition.as_mut().unwrap();
                p.stored_blend_space = 1654948433;
                p.stored_composite_space = 1654948433;
                p.stored_operator = 1654948433;
            }
            4 => doc.layers[0].excluded_channels = 1,
            5 => {
                counts.bad.store(true, Ordering::Relaxed);
            }
            _ => {
                let mut default = [0; 16];
                default[..4].copy_from_slice(&f32::NAN.to_le_bytes());
                let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), s, &default).unwrap();
                let fee = q.reserve(QuotaKind::Resident, std::mem::size_of::<Pattern>() as u64 + 256).unwrap();
                let store = Arc::new(Pattern { descriptor, q: q.clone(), counts, _fee: fee });
                let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
                doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
            }
        }
        let owned = owner(doc, q, g, true);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
    }
}
