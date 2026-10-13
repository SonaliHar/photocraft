//! Qualified ordinary imported scenes use the native preview ledger directly.
//! The caller retains and has already charged the borrowed source document.
use crate::IoError;
use photocraft_cms::{Builtin, ColorSpace, Intent, Transform, WideWork};
use photocraft_compose::{
    gimp_composite::CompositionError,
    gimp_render::{PreparationBudget, PreparedRenderer},
    preview_budget::{PreparationLease, PreviewPreparationBudget},
};
use photocraft_doc::{ColorMode, Document};
use photocraft_raster::{Interrupt, RasterInputError, Rgba8Image};
use std::cell::Cell;
#[cfg(test)]
#[path = "native_cancellation_measurement.rs"]
mod cancellation_measurement;
fn composition(error: CompositionError) -> IoError {
    match error {
        CompositionError::Raster(RasterInputError::Cancelled) => IoError::Cancelled,
        CompositionError::Raster(RasterInputError::LimitExceeded) => {
            photocraft_format::FormatError::LimitExceeded("native ordinary preview resident bytes".into()).into()
        }
        other => IoError::Unsupported(other.to_string()),
    }
}
fn raster(error: RasterInputError) -> CompositionError {
    CompositionError::Raster(error)
}
fn work(total: &Cell<u64>, units: u64, control: &Interrupt<'_>) -> Result<(), CompositionError> {
    control.check().map_err(|_| raster(RasterInputError::Cancelled))?;
    let next = total.get().checked_add(units).ok_or(CompositionError::Invalid("native ordinary preview work overflow"))?;
    if next > 1 << 36 {
        return Err(CompositionError::Invalid("native ordinary preview work limit"));
    }
    total.set(next);
    Ok(())
}
struct Adapter<'a, 'b, 'i> {
    budget: &'a PreviewPreparationBudget<'b>,
    control: &'a Interrupt<'i>,
    work: &'a Cell<u64>,
}
impl<'a, 'b> PreparationBudget for Adapter<'a, 'b, '_> {
    type Lease = PreparationLease<'a, 'b>;
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<Self::Lease, CompositionError> {
        let bytes = usize::try_from(bytes).map_err(|_| CompositionError::Invalid("native preparation size"))?;
        self.budget.reserve_preparation_bytes(bytes, self.control).map_err(raster)
    }
    fn consume_preparation_work(&mut self, units: u64) -> Result<(), CompositionError> {
        work(self.work, units, self.control)
    }
}
// Qualified plain/point/ordinary Normal members have no additional staging.
// The separately selected Box/Motion/Median/Gaussian members have one fixed transient neighborhood cache.
// The first nonzero render reservation always owns the returned float buffer.
struct Output<'a, 'b, 'i> {
    adapter: Adapter<'a, 'b, 'i>,
    lease: Option<PreparationLease<'a, 'b>>,
    requested: usize,
    cache: Option<PreparationLease<'a, 'b>>,
    cache_seen: bool,
    allow_cache: bool,
}
impl Output<'_, '_, '_> {
    fn event(&mut self, event: WideWork) -> Result<(), CompositionError> {
        match event {
            WideWork::SampleUnits(units) => work(self.adapter.work, units, self.adapter.control),
            WideWork::ReserveBytes(0) | WideWork::ReleaseBytes(0) => Ok(()),
            WideWork::ReserveBytes(bytes) => {
                if self.lease.is_some() {
                    // The qualified neighborhood job reserves its fixed cache after the
                    // returned float buffer. Role/order is authoritative: the
                    // float buffer itself may also happen to be 32 MiB.
                    if self.allow_cache && !self.cache_seen && bytes == 32 << 20 {
                        self.cache = Some(self.adapter.budget.reserve_preparation_bytes(32 << 20, self.adapter.control).map_err(raster)?);
                        self.cache_seen = true;
                        return Ok(());
                    }
                    return Err(CompositionError::Unsupported("ordinary native preview additional render staging"));
                }
                let bytes = usize::try_from(bytes).map_err(|_| CompositionError::Invalid("native output size"))?;
                let cap = bytes.checked_mul(2).ok_or(CompositionError::Invalid("native output capacity headroom"))?;
                self.lease = Some(self.adapter.budget.reserve_preparation_bytes(cap, self.adapter.control).map_err(raster)?);
                self.requested = bytes;
                Ok(())
            }
            WideWork::ReleaseBytes(bytes) => {
                if self.cache.is_some() {
                    if bytes != 32 << 20 {
                        return Err(CompositionError::Invalid("native cache release mismatch"));
                    }
                    // RenderJob destroys cached surfaces before this callback.
                    // Only this transient lease is refunded; the returned float
                    // reservation remains live through RGBA conversion.
                    self.cache = None;
                    return Ok(());
                }
                if usize::try_from(bytes).ok() != Some(self.requested) || self.lease.is_none() {
                    return Err(CompositionError::Invalid("native render release mismatch"));
                }
                self.lease = None;
                self.requested = 0;
                Ok(())
            }
        }
    }
}
struct Image<'a, 'b> {
    image: Rgba8Image,
    _lease: PreparationLease<'a, 'b>,
}
/// `resident` is the native job's remaining budget after its retained source and
/// model census. Both previews and preparation coexist on this single ledger.
pub(super) fn previews(doc: &Document, control: &Interrupt<'_>, resident: usize) -> Result<photocraft_format::SaveOptions, IoError> {
    let observer = |_| {};
    let budget = PreviewPreparationBudget::new(resident, &observer);
    previews_with_budget(doc, control, &budget)
}
fn previews_with_budget(doc: &Document, control: &Interrupt<'_>, budget: &PreviewPreparationBudget<'_>) -> Result<photocraft_format::SaveOptions, IoError> {
    let used_work = Cell::new(0);
    let mut adapter = Adapter { budget, control, work: &used_work };
    // An immutable borrow is an explicit source lifetime token: NativeSave's
    // retained input charge is still held by the caller for this entire phase.
    let prepared = if PreparedRenderer::supports_plain_normal_budgeted(doc) {
        PreparedRenderer::new_plain_normal_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::supports_plain_point_budgeted(doc) {
        PreparedRenderer::new_plain_point_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::supports_plain_box_budgeted(doc) {
        PreparedRenderer::new_plain_box_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::supports_plain_motion_budgeted(doc) {
        PreparedRenderer::new_plain_motion_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::supports_plain_median_budgeted(doc) {
        PreparedRenderer::new_plain_median_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::supports_plain_gaussian_budgeted(doc) {
        PreparedRenderer::new_plain_gaussian_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::supports_plain_brightness_contrast_budgeted(doc) {
        PreparedRenderer::new_plain_brightness_contrast_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::requires_preparation_budget(doc) {
        PreparedRenderer::new_budgeted(doc, control, &mut adapter, doc)
    } else if PreparedRenderer::supports_accounted_scalar_preparation(doc) {
        PreparedRenderer::new_accounted(doc, control, &mut adapter, doc)
    } else {
        PreparedRenderer::new_budgeted(doc, control, &mut adapter, doc)
    }
    .map_err(composition)?;
    let icc = doc.icc_profile.as_ref().ok_or_else(|| IoError::Unsupported("ordinary native preview current ICC absent".into()))?;
    let transform_bytes =
        icc.len().checked_mul(128).and_then(|n| n.checked_add(65536)).ok_or_else(|| IoError::Unsupported("native display profile size".into()))?;
    let transform_lease = budget.reserve_preparation_bytes(transform_bytes, control).map_err(|e| composition(raster(e)))?;
    let source = match crate::xcf_export::active_implicit_profile(doc).map_err(|e| IoError::Unsupported(e.to_string()))? {
        Some(profile) => profile,
        None => photocraft_cms::xcf_color::parse_bounded_profile(icc, if doc.mode == ColorMode::Grayscale { ColorSpace::Gray } else { ColorSpace::Rgb })
            .map_err(|e| IoError::Unsupported(e.to_string()))?,
    };
    let display = Transform::new(&source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).map_err(|e| IoError::Unsupported(e.to_string()))?;
    let preview = |side: u32| -> Result<Image<'_, '_>, IoError> {
        let longest = doc.size.width.max(doc.size.height).max(1);
        let width = (u64::from(doc.size.width) * u64::from(side.min(longest)) / u64::from(longest)).max(1) as u32;
        let height = (u64::from(doc.size.height) * u64::from(side.min(longest)) / u64::from(longest)).max(1) as u32;
        let mut output = Output {
            adapter: Adapter { budget, control, work: &used_work },
            lease: None,
            requested: 0,
            cache: None,
            cache_seen: false,
            allow_cache: PreparedRenderer::supports_plain_box_budgeted(doc)
                || PreparedRenderer::supports_plain_motion_budgeted(doc)
                || PreparedRenderer::supports_plain_median_budgeted(doc)
                || PreparedRenderer::supports_plain_gaussian_budgeted(doc),
        };
        let buffer = prepared.renderer().render_reduced_with_interrupt(width, height, None, &mut |event| output.event(event), control).map_err(composition)?;
        if output.cache.is_some() {
            return Err(IoError::Unsupported("native cache retained after render completion".into()));
        }
        let float_bytes = buffer.px.capacity().checked_mul(16).ok_or_else(|| IoError::Unsupported("native float preview capacity".into()))?;
        output
            .lease
            .as_mut()
            .ok_or_else(|| IoError::Unsupported("native output lease absent".into()))?
            .retain_bytes(float_bytes)
            .map_err(|e| composition(raster(e)))?;
        let len = buffer.px.len().checked_mul(4).ok_or_else(|| IoError::Unsupported("native RGBA preview size".into()))?;
        let reservation = len.checked_mul(2).ok_or_else(|| IoError::Unsupported("native RGBA capacity headroom".into()))?;
        let mut image_lease = budget.reserve_preparation_bytes(reservation, control).map_err(|e| composition(raster(e)))?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(len).map_err(|_| IoError::Unsupported("native RGBA preview allocation denied".into()))?;
        if pixels.capacity() > reservation {
            return Err(IoError::Unsupported("native RGBA allocator capacity exceeds reservation".into()));
        }
        while pixels.len() < len {
            control.check().map_err(|_| IoError::Cancelled)?;
            pixels.resize(pixels.len().saturating_add(4096).min(len), 0);
        }
        for (index, (pixel, bytes)) in buffer.px.iter().zip(pixels.as_chunks_mut::<4>().0).enumerate() {
            if index.is_multiple_of(64) {
                work(&used_work, 64.min(buffer.px.len() - index) as u64, control).map_err(composition)?;
            }
            let input = [f64::from(pixel[0]), f64::from(pixel[1]), f64::from(pixel[2])];
            let mut rgb = [0.; 3];
            display
                .try_eval_f64(if doc.mode == ColorMode::Grayscale { &input[..1] } else { &input }, &mut rgb)
                .map_err(|e| IoError::Unsupported(e.to_string()))?;
            for (target, value) in bytes.iter_mut().take(3).zip(rgb) {
                let display = value.clamp(0., 1.) as f32;
                *target = (display * 255. + 0.5) as u8;
            }
            bytes[3] = (pixel[3].clamp(0., 1.) * 255. + 0.5) as u8;
        }
        image_lease.retain_bytes(pixels.capacity()).map_err(|e| composition(raster(e)))?;
        drop(buffer);
        drop(output);
        control.check().map_err(|_| IoError::Cancelled)?;
        Ok(Image { image: Rgba8Image { width, height, pixels }, _lease: image_lease })
    };
    let thumbnail = preview(256)?;
    let composite = preview(1024)?;
    drop(display);
    drop(source);
    drop(transform_lease);
    drop(prepared);
    control.check().map_err(|_| IoError::Cancelled)?;
    // Caller immediately measures both transferred buffers before preparation.
    Ok(photocraft_format::SaveOptions { thumbnail: Some(thumbnail.image), composite: Some(composite.image) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_and_box_cache_equal_sizes_do_not_confuse_lease_roles() {
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(96 << 20, &observer);
        let units = Cell::new(0);
        let mut output = Output {
            adapter: Adapter { budget: &budget, control: &Interrupt::NONE, work: &units },
            lease: None,
            requested: 0,
            cache: None,
            cache_seen: false,
            allow_cache: true,
        };
        output.event(WideWork::ReserveBytes(32 << 20)).unwrap();
        assert_eq!(budget.usage().live_bytes, 64 << 20);
        output.event(WideWork::ReserveBytes(32 << 20)).unwrap();
        assert_eq!(budget.usage().live_bytes, 96 << 20);
        assert!(output.event(WideWork::ReserveBytes(32 << 20)).is_err());
        assert!(output.event(WideWork::ReleaseBytes(4096)).is_err());
        assert_eq!(budget.usage().live_bytes, 96 << 20);
        output.event(WideWork::ReleaseBytes(32 << 20)).unwrap();
        assert!(output.lease.is_some());
        assert!(output.cache.is_none());
        assert_eq!(budget.usage().live_bytes, 64 << 20);
        assert!(output.event(WideWork::ReserveBytes(32 << 20)).is_err());
        output.event(WideWork::ReleaseBytes(32 << 20)).unwrap();
        assert_eq!(budget.usage().live_bytes, 0);
    }
    #[test]
    #[ignore = "requires reviewed unpublished Box native BEFORE authority"]
    fn actual_box_cache_has_a_distinct_live_lease_and_refunds() {
        let packet = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_BOX_PREVIEW_BASELINE").unwrap());
        let doc = photocraft_format::load_from_bytes(&std::fs::read(packet.join("box-blur-source.before.pcraft")).unwrap()).unwrap();
        let before = doc.clone();
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(48 << 20, &observer);
        let units = Cell::new(0);
        {
            let renderer = PreparedRenderer::new(&doc).unwrap();
            let mut output = Output {
                adapter: Adapter { budget: &budget, control: &Interrupt::NONE, work: &units },
                lease: None,
                requested: 0,
                cache: None,
                cache_seen: false,
                allow_cache: true,
            };
            let result = renderer.render_reduced(32, 32, None, &mut |event| output.event(event));
            assert!(result.is_ok(), "actual Box cache must coexist with returned float: {result:?}");
            assert_eq!(budget.usage().live_bytes, 32768);
            assert!(budget.usage().peak_bytes >= (32 << 20) + 32768);
            drop(result);
        }
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(doc, before);
    }
    #[test]
    fn render_output_owns_only_its_reservation_and_failures_refund() {
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(128, &observer);
        let work = Cell::new(0);
        let retained = budget.reserve_preparation_bytes(32, &Interrupt::NONE).unwrap();
        let mut output = Output {
            adapter: Adapter { budget: &budget, control: &Interrupt::NONE, work: &work },
            lease: None,
            requested: 0,
            cache: None,
            cache_seen: false,
            allow_cache: false,
        };
        output.event(WideWork::ReserveBytes(40)).unwrap();
        assert_eq!(budget.usage().live_bytes, 112);
        drop(retained);
        assert_eq!(budget.usage().live_bytes, 80);
        assert!(output.event(WideWork::ReserveBytes(1)).is_err());
        assert!(output.event(WideWork::ReleaseBytes(39)).is_err());
        assert_eq!(budget.usage().live_bytes, 80);
        output.event(WideWork::ReleaseBytes(40)).unwrap();
        assert_eq!(budget.usage().live_bytes, 0);
        assert!(output.event(WideWork::ReserveBytes(65)).is_err());
        assert_eq!(budget.usage().live_bytes, 0);
        drop(output);
        let cancel = || true;
        let cancelled = Interrupt::cancel_only(&cancel);
        let mut output = Output {
            adapter: Adapter { budget: &budget, control: &cancelled, work: &work },
            lease: None,
            requested: 0,
            cache: None,
            cache_seen: false,
            allow_cache: false,
        };
        assert!(matches!(output.event(WideWork::ReserveBytes(1)), Err(CompositionError::Raster(RasterInputError::Cancelled))));
        assert_eq!(budget.usage().live_bytes, 0);
    }
    fn plain_document() -> Document {
        use photocraft_doc::{Layer, SampleEncoding, SampleType, Size, WorkingProfileInterpretation};
        let mut doc = Document::new("Own preparation stage", Size::new(513, 385), ColorMode::Rgb, SampleType::F64);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let icc = Builtin::Srgb.profile().to_bytes();
        doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&icc);
        doc.icc_profile = Some(icc);
        let mut layer = Layer::raster("Own plain", doc.pixel_format());
        let bytes: Vec<_> = [0.2_f64, 0.5, 0.8, 0.75].into_iter().flat_map(f64::to_ne_bytes).collect();
        *layer.surface_mut().unwrap() = photocraft_raster::Surface::try_with_default_bytes(doc.pixel_format(), &bytes).unwrap();
        doc.layers.push(layer);
        doc
    }
    #[test]
    fn plain_two_outputs_retain_first_image_and_refund_cancelled_second_reservation() {
        use photocraft_compose::preview_budget::PreparationCharge;
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let doc = plain_document();
        let before = doc.clone();
        let second_float = 513 * 385 * 32;
        let first_retained = 256 * 192 * 4;
        for cancel_second in [false, true] {
            let live = AtomicUsize::new(0);
            let cancelled = AtomicBool::new(false);
            let second_before = AtomicUsize::new(0);
            let events = Mutex::new(Vec::new());
            let observer = |event| {
                match event {
                    PreparationCharge::Reserve(bytes) => {
                        let prior = live.fetch_add(bytes, Ordering::Relaxed);
                        if bytes == second_float {
                            second_before.store(prior, Ordering::Relaxed);
                            cancelled.store(cancel_second, Ordering::Relaxed);
                        }
                    }
                    PreparationCharge::Release(bytes) => {
                        live.fetch_sub(bytes, Ordering::Relaxed);
                    }
                }
                events.lock().unwrap().push(event);
            };
            let budget = PreviewPreparationBudget::new(32 << 20, &observer);
            let stop = || cancelled.load(Ordering::Relaxed);
            let result = previews_with_budget(&doc, &Interrupt::cancel_only(&stop), &budget);
            if cancel_second {
                assert!(matches!(result, Err(IoError::Cancelled)));
            } else {
                let outputs = result.unwrap();
                assert_eq!(outputs.thumbnail.as_ref().unwrap().pixels.len(), first_retained);
                assert_eq!(outputs.composite.as_ref().unwrap().pixels.len(), 513 * 385 * 4);
            }
            let prepared_and_profile = doc.icc_profile.as_ref().unwrap().len() * 128 + 65536;
            assert_eq!(second_before.load(Ordering::Relaxed), prepared_and_profile * 2 + first_retained);
            assert_eq!(live.load(Ordering::Relaxed), 0);
            assert_eq!(budget.usage().live_bytes, 0);
            assert!(!events.lock().unwrap().is_empty());
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn plain_second_output_budget_denial_and_preparation_cancel_are_atomic() {
        use photocraft_compose::preview_budget::PreparationCharge;
        use std::sync::atomic::{AtomicBool, Ordering};
        let doc = plain_document();
        let before = doc.clone();
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(4 << 20, &observer);
        assert!(matches!(previews_with_budget(&doc, &Interrupt::NONE, &budget), Err(IoError::Pcraft(photocraft_format::FormatError::LimitExceeded(_)))));
        assert_eq!(budget.usage().live_bytes, 0);
        assert!(budget.usage().peak_bytes < 4 << 20);
        let cancelled = AtomicBool::new(false);
        let observer = |event| {
            if matches!(event, PreparationCharge::Reserve(_)) {
                cancelled.store(true, Ordering::Relaxed);
            }
        };
        let budget = PreviewPreparationBudget::new(32 << 20, &observer);
        let stop = || cancelled.load(Ordering::Relaxed);
        assert!(matches!(previews_with_budget(&doc, &Interrupt::cancel_only(&stop), &budget), Err(IoError::Cancelled)));
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(doc, before);
    }
    fn two_preview_reservations(packet_env: &str, filename: &str, reservation_bytes: usize, maximum: usize) {
        use photocraft_compose::preview_budget::PreparationCharge;
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let packet = std::path::PathBuf::from(std::env::var(packet_env).unwrap());
        let doc = photocraft_format::load_from_bytes(&std::fs::read(packet.join(filename)).unwrap()).unwrap();
        let before = doc.clone();
        assert!(doc.size.width <= 256 && doc.size.height <= 256, "frozen small member preview geometry");
        let image_bytes = doc.size.width as usize * doc.size.height as usize * 4;
        let first_base = AtomicUsize::new(0);
        for mode in 0..3 {
            let live = AtomicUsize::new(0);
            let reservations = AtomicUsize::new(0);
            let stop = AtomicBool::new(false);
            let first = AtomicUsize::new(0);
            let second = AtomicUsize::new(0);
            let events = Mutex::new(Vec::new());
            let observer = |event| {
                match event {
                    PreparationCharge::Reserve(bytes) => {
                        let prior = live.fetch_add(bytes, Ordering::Relaxed);
                        if bytes == reservation_bytes {
                            let n = reservations.fetch_add(1, Ordering::Relaxed);
                            if n == 0 {
                                first.store(prior, Ordering::Relaxed);
                            } else if n == 1 {
                                second.store(prior, Ordering::Relaxed);
                                if mode == 1 {
                                    stop.store(true, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    PreparationCharge::Release(bytes) => {
                        live.fetch_sub(bytes, Ordering::Relaxed);
                    }
                };
                events.lock().unwrap().push(event);
            };
            let cap = if mode == 2 { first_base.load(Ordering::Relaxed) + reservation_bytes + image_bytes - 1 } else { maximum };
            let budget = PreviewPreparationBudget::new(cap, &observer);
            let cancelled = || stop.load(Ordering::Relaxed);
            let result = previews_with_budget(&doc, &Interrupt::cancel_only(&cancelled), &budget);
            if mode == 0 {
                let output = result.unwrap();
                assert_eq!(output.thumbnail.unwrap().pixels.len(), image_bytes);
                assert_eq!(output.composite.unwrap().pixels.len(), image_bytes);
                assert_eq!(reservations.load(Ordering::Relaxed), 2);
                assert_eq!(second.load(Ordering::Relaxed), first.load(Ordering::Relaxed) + image_bytes);
                first_base.store(first.load(Ordering::Relaxed), Ordering::Relaxed);
            } else if mode == 1 {
                assert!(matches!(result, Err(IoError::Cancelled)));
                assert_eq!(second.load(Ordering::Relaxed), first.load(Ordering::Relaxed) + image_bytes);
            } else {
                assert!(matches!(result, Err(IoError::Pcraft(photocraft_format::FormatError::LimitExceeded(_)))));
                assert_eq!(reservations.load(Ordering::Relaxed), 1);
                assert!(events.lock().unwrap().iter().any(|e| *e == PreparationCharge::Reserve(image_bytes * 2)));
            }
            assert_eq!(budget.usage().live_bytes, 0);
            assert_eq!(live.load(Ordering::Relaxed), 0);
            assert!(budget.usage().peak_bytes <= cap);
            assert_eq!(doc, before);
        }
    }
    #[test]
    #[ignore = "requires reviewed unpublished frozen Exposure native BEFORE authority"]
    fn point_preparation_retains_first_image_until_second_output_and_refunds_on_failure() {
        two_preview_reservations("PHOTOCRAFT_XCF_POINT_PREVIEW_BASELINE", "exposure-source.before.pcraft", 32768, 16 << 20);
    }
    #[test]
    #[ignore = "requires reviewed unpublished frozen Box native BEFORE authority"]
    fn box_preparation_retains_first_image_through_second_cache_and_refunds() {
        two_preview_reservations("PHOTOCRAFT_XCF_BOX_PREVIEW_BASELINE", "box-blur-source.before.pcraft", 32 << 20, 48 << 20);
    }
    #[test]
    #[ignore = "requires reviewed unpublished frozen Motion native BEFORE authority"]
    fn motion_preparation_retains_first_image_through_second_cache_and_refunds() {
        two_preview_reservations("PHOTOCRAFT_XCF_MOTION_PREVIEW_BASELINE", "motion-blur-linear-source.before.pcraft", 32 << 20, 48 << 20);
    }
    #[test]
    #[ignore = "requires reviewed unpublished frozen Median native BEFORE authority"]
    fn median_preparation_retains_first_image_through_second_cache_and_refunds() {
        two_preview_reservations("PHOTOCRAFT_XCF_MEDIAN_PREVIEW_BASELINE", "median-blur-source.before.pcraft", 32 << 20, 48 << 20);
    }
    #[test]
    #[ignore = "requires reviewed unpublished frozen Gaussian native BEFORE authority"]
    fn gaussian_preparation_retains_first_image_through_second_cache_and_refunds() {
        two_preview_reservations("PHOTOCRAFT_XCF_GAUSSIAN_PREVIEW_BASELINE", "gaussian-blur-source.before.pcraft", 32 << 20, 48 << 20);
    }
    #[test]
    #[ignore = "requires reviewed unpublished frozen Brightness Contrast native BEFORE authority"]
    fn brightness_preparation_retains_first_image_through_second_float_and_refunds() {
        two_preview_reservations("PHOTOCRAFT_XCF_BRIGHTNESS_PREVIEW_BASELINE", "brightness-contrast-source.before.pcraft", 32768, 16 << 20);
    }
    #[test]
    #[ignore = "requires frozen accounted native BEFORE states"]
    fn accounted_scalar_and_mask_second_output_refunds() {
        two_preview_reservations("PHOTOCRAFT_ACCOUNTED_BASELINE", "stack.source.before.pcraft", 8 * 6 * 32, 16 << 20);
        two_preview_reservations("PHOTOCRAFT_ACCOUNTED_BASELINE", "masked.current.source.before.pcraft", 32 * 32 * 32, 16 << 20);
    }
}
