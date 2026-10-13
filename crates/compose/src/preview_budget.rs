//! Adaptive ordinary preview workspace. This scoped route admits one plain
//! raster owner; styles, proxies and source regeneration use separate APIs.
//! Charges describe known payloads and conservative compositor scratch, not RSS.
use super::*;
use photocraft_raster::{Interrupt, RasterInputError};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationCharge {
    Reserve(usize),
    Release(usize),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparationUsage {
    pub live_bytes: usize,
    pub peak_bytes: usize,
}
/// Observer runs synchronously under the ledger lock and must not reenter it.
pub struct PreviewPreparationBudget<'a> {
    maximum: usize,
    state: Mutex<PreparationUsage>,
    observer: &'a (dyn Fn(PreparationCharge) + Sync),
}
impl<'a> PreviewPreparationBudget<'a> {
    pub fn new(maximum: usize, observer: &'a (dyn Fn(PreparationCharge) + Sync)) -> Self {
        Self { maximum, state: Mutex::new(PreparationUsage::default()), observer }
    }
    /// Reserve additional caller-owned preparation on this same live ledger.
    /// The returned lease must outlive the allocation it accounts for.
    pub fn reserve_preparation_bytes<'b>(&'b self, bytes: usize, control: &Interrupt<'_>) -> Result<PreparationLease<'b, 'a>, RasterInputError> {
        Ok(PreparationLease { lease: self.reserve(bytes, control)? })
    }
    pub(crate) fn available(&self) -> usize {
        self.maximum.saturating_sub(self.usage().live_bytes)
    }
    pub fn usage(&self) -> PreparationUsage {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn reserve<'b>(&'b self, bytes: usize, control: &Interrupt<'_>) -> Result<Lease<'b, 'a>, RasterInputError> {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let next = state.live_bytes.checked_add(bytes).ok_or(RasterInputError::LimitExceeded)?;
        if next > self.maximum {
            return Err(RasterInputError::LimitExceeded);
        }
        state.live_bytes = next;
        state.peak_bytes = state.peak_bytes.max(next);
        (self.observer)(PreparationCharge::Reserve(bytes));
        drop(state);
        let lease = Lease { budget: self, bytes };
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(lease)
    }
}
/// A caller-owned reservation; dropping it refunds only its own live bytes.
/// Destroy or transfer the accounted allocation before dropping this lease.
pub struct PreparationLease<'a, 'b> {
    lease: Lease<'a, 'b>,
}
impl PreparationLease<'_, '_> {
    pub fn reserved_bytes(&self) -> usize {
        self.lease.bytes()
    }
    /// Refund construction headroom after the actual retained capacity is known.
    pub fn retain_bytes(&mut self, bytes: usize) -> Result<(), RasterInputError> {
        self.lease.shrink(bytes)
    }
}
pub(crate) struct Lease<'a, 'b> {
    budget: &'a PreviewPreparationBudget<'b>,
    bytes: usize,
}
impl Lease<'_, '_> {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(crate) fn shrink(&mut self, to: usize) -> Result<(), RasterInputError> {
        if to > self.bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let refund = self.bytes - to;
        if refund > 0 {
            let mut state = self.budget.state.lock().unwrap_or_else(|e| e.into_inner());
            state.live_bytes = state.live_bytes.saturating_sub(refund);
            (self.budget.observer)(PreparationCharge::Release(refund));
            self.bytes = to;
        }
        Ok(())
    }
}
impl Drop for Lease<'_, '_> {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().unwrap_or_else(|e| e.into_inner());
        state.live_bytes = state.live_bytes.saturating_sub(self.bytes);
        (self.budget.observer)(PreparationCharge::Release(self.bytes));
    }
}
/// Returned pixels remain charged until this owner is consumed or dropped.
/// `into_image` transfers accounting responsibility to the caller.
pub struct ChargedThumbnail<'a, 'b> {
    image: Rgba8Image,
    _lease: Lease<'a, 'b>,
}
impl ChargedThumbnail<'_, '_> {
    pub fn image(&self) -> &Rgba8Image {
        &self.image
    }
    pub fn into_image(self) -> Rgba8Image {
        self.image
    }
}
pub fn plain_preview_supported(doc: &Document) -> bool {
    doc.size.width > 0
        && doc.size.height > 0
        && doc.size.width <= 262144
        && doc.size.height <= 262144
        && doc.size.area() <= PROXY_THUMBNAIL_PIXELS
        && doc.xcf_state.is_none()
        && doc.patterns.is_empty()
        && doc.layers.len() == 1
        && matches!(doc.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
        && doc.layers.first().is_some_and(|layer| {
            matches!(&layer.content, LayerContent::Raster(surface) if surface.format().mode == doc.mode && surface.format().sample == doc.depth)
                && layer.mask.is_none()
                && layer.vector_mask.is_none()
                && !effects::has_effects(layer)
                && layer.advanced.is_default()
                && layer.video.is_none()
                && layer.blend == BlendMode::Normal
                && !layer.clipped
                && layer.import_compositing.is_none()
        })
}
/// Byte-identical ordinary reduction without a document proxy clone. Known
/// buffers are leased before allocation; scratch is a conservative payload bound.
pub fn thumbnail_with_preparation_budget<'a, 'b>(
    doc: &Document,
    side: u32,
    control: &Interrupt<'_>,
    budget: &'a PreviewPreparationBudget<'b>,
) -> Result<ChargedThumbnail<'a, 'b>, RasterInputError> {
    use RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    if !plain_preview_supported(doc) {
        return Err(E::UnsupportedConversion);
    }
    thumbnail_with_budget_backend(doc, side, control, budget, 1, rayon_workers(), |rect| render_with_interrupt(doc, rect, control))
}
fn rayon_workers() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::current_num_threads().max(1)
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}
pub(crate) fn thumbnail_with_budget_backend<'a, 'b>(
    doc: &Document,
    side: u32,
    control: &Interrupt<'_>,
    budget: &'a PreviewPreparationBudget<'b>,
    weight: usize,
    workers: usize,
    render: impl FnMut(Rect) -> Result<Buffer, RasterInputError>,
) -> Result<ChargedThumbnail<'a, 'b>, RasterInputError> {
    thumbnail_with_budget_backend_sized(doc, side, control, budget, ReductionPlan { weight, workers, output: None }, render)
}
pub(crate) struct ReductionPlan {
    pub weight: usize,
    pub workers: usize,
    pub output: Option<(usize, usize)>,
}
pub(crate) fn thumbnail_with_budget_backend_sized<'a, 'b>(
    doc: &Document,
    side: u32,
    control: &Interrupt<'_>,
    budget: &'a PreviewPreparationBudget<'b>,
    plan: ReductionPlan,
    mut render: impl FnMut(Rect) -> Result<Buffer, RasterInputError>,
) -> Result<ChargedThumbnail<'a, 'b>, RasterInputError> {
    use RasterInputError as E;
    let ReductionPlan { weight, workers, output } = plan;
    proxy::validate_surfaces_with_interrupt(doc, control)?;
    let bounds = doc.bounds();
    photocraft_raster::checked_region_byte_len(bounds, photocraft_color::PixelFormat::RGBA32F)?;
    let (fw, fh) = (bounds.width() as usize, bounds.height() as usize);
    if fw == 0 || fh == 0 || side == 0 || side > 4096 {
        return Err(E::InvalidBounds);
    }
    let scale = (side as f32 / fw.max(fh) as f32).min(1.0);
    let (w, h) = output.unwrap_or_else(|| (((fw as f32 * scale).round() as usize).max(1), ((fh as f32 * scale).round() as usize).max(1)));
    let (w, h) = (w.clamp(1, fw), h.clamp(1, fh));
    let count = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let accumulation = budget.reserve(count.checked_mul(16).ok_or(E::LimitExceeded)?, control)?;
    let mut acc = zeros::<[f32; 4]>(count, control)?;
    let map_lease = budget.reserve(fw.checked_add(fh).and_then(|n| n.checked_mul(4)).ok_or(E::LimitExceeded)?, control)?;
    let cell = |x: usize, f: usize, n: usize| ((x + 1) * n).saturating_sub(1) / f;
    let mut cols = zeros::<u32>(fw, control)?;
    let mut rows = zeros::<u32>(fh, control)?;
    for (map, f, n) in [(&mut cols, fw, w), (&mut rows, fh, h)] {
        for (index, dst) in map.iter_mut().enumerate() {
            if index.is_multiple_of(1024) {
                control.check().map_err(|_| E::Cancelled)?;
            }
            *dst = cell(index, f, n) as u32;
        }
    }
    let band_rows = band_rows_for(bounds.width(), 0) as usize;
    let mut y = 0usize;
    while y < fh {
        control.check().map_err(|_| E::Cancelled)?;
        let requested = band_rows.min(fh - y);
        let charged_bytes = |rows: usize| -> Result<usize, E> {
            let pixels = fw.checked_mul(rows).ok_or(E::LimitExceeded)?;
            let scratch = fw
                .min(256)
                .checked_mul(rows.min(8))
                .and_then(|n| n.checked_mul(16usize.saturating_mul(weight.saturating_add(9))))
                .and_then(|n| n.checked_mul(workers))
                .ok_or(E::LimitExceeded)?;
            pixels.checked_mul(16).and_then(|n| n.checked_add(scratch)).ok_or(E::LimitExceeded)
        };
        // Choose the largest fitting band instead of reserving a fixed full
        // backend band. Reducing band boundaries preserves summation order.
        let available = budget.available();
        let (mut low, mut high) = (0usize, requested);
        while low < high {
            control.check().map_err(|_| E::Cancelled)?;
            let middle = low + (high - low).div_ceil(2);
            if charged_bytes(middle)? <= available {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        if low == 0 {
            return Err(E::LimitExceeded);
        }
        let end = y + low;
        let pixels = fw.checked_mul(low).ok_or(E::LimitExceeded)?;
        // One plain Raster has no cache/geometry/style preparation. The ordinary
        // microblock compositor has bounded temporary buffer nesting; reserve
        // its conservative payload ceiling alongside the actual band output.
        let scratch = fw
            .min(256)
            .checked_mul((end - y).min(8))
            .and_then(|n| n.checked_mul(16usize.saturating_mul(weight.saturating_add(9))))
            .and_then(|n| n.checked_mul(workers))
            .ok_or(E::LimitExceeded)?;
        let band_lease = budget.reserve(pixels.checked_mul(16).and_then(|n| n.checked_add(scratch)).ok_or(E::LimitExceeded)?, control)?;
        let rect = Rect::new(bounds.x0, bounds.y0 + y as i32, bounds.x1, bounds.y0 + end as i32);
        let band = render(rect)?;
        for (local_y, line) in band.px.chunks_exact(fw).enumerate() {
            control.check().map_err(|_| E::Cancelled)?;
            let target_y = *rows.get(y + local_y).ok_or(E::InvalidBounds)? as usize;
            for (x, p) in line.iter().enumerate() {
                if x.is_multiple_of(1024) {
                    control.check().map_err(|_| E::Cancelled)?;
                }
                let target_x = *cols.get(x).ok_or(E::InvalidBounds)? as usize;
                let q = acc.get_mut(target_y * w + target_x).ok_or(E::InvalidBounds)?;
                if (w, h) == (fw, fh) {
                    *q = *p;
                    continue;
                }
                for i in 0..3 {
                    q[i] += p[i] * p[3];
                }
                q[3] += p[3];
            }
        }
        drop(band);
        drop(band_lease);
        y = end;
    }
    drop(cols);
    drop(rows);
    drop(map_lease);
    let span = |t: usize, f: usize, n: usize| (t * f / n, ((t + 1) * f / n).max(t * f / n + 1).min(f));
    for (index, p) in acc.iter_mut().enumerate() {
        if index.is_multiple_of(1024) {
            control.check().map_err(|_| E::Cancelled)?;
        }
        if (w, h) == (fw, fh) {
            continue;
        }
        let ((x0, x1), (y0, y1)) = (span(index % w, fw, w), span(index / w, fh, h));
        let n = ((x1 - x0) * (y1 - y0)).max(1) as f32;
        let a = p[3];
        *p = if a > 0.0 { [p[0] / a, p[1] / a, p[2] / a, a / n] } else { [0.0; 4] };
    }
    let lease = budget.reserve(count.checked_mul(4).ok_or(E::LimitExceeded)?, control)?;
    let mut pixels = zeros::<u8>(count.checked_mul(4).ok_or(E::LimitExceeded)?, control)?;
    for (index, (p, q)) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(&acc).enumerate() {
        if index.is_multiple_of(1024) {
            control.check().map_err(|_| E::Cancelled)?;
        }
        for (dst, v) in p.iter_mut().zip(q) {
            *dst = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    drop(acc);
    drop(accumulation);
    control.check().map_err(|_| E::Cancelled)?;
    Ok(ChargedThumbnail { image: Rgba8Image { width: w as u32, height: h as u32, pixels }, _lease: lease })
}
fn zeros<T: Default + Clone>(count: usize, token: &Interrupt<'_>) -> Result<Vec<T>, RasterInputError> {
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
    if out.capacity() != count {
        return Err(RasterInputError::LimitExceeded);
    }
    while out.len() < count {
        token.check().map_err(|_| RasterInputError::Cancelled)?;
        out.resize(out.len().saturating_add(1024).min(count), T::default());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[test]
    fn public_preparation_lease_is_independent_of_older_output_refunds() {
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(64, &observer);
        let older = budget.reserve_preparation_bytes(10, &Interrupt::NONE).unwrap();
        let mut current = budget.reserve_preparation_bytes(30, &Interrupt::NONE).unwrap();
        drop(older);
        assert_eq!(budget.usage().live_bytes, 30);
        assert_eq!(current.reserved_bytes(), 30);
        assert!(current.retain_bytes(31).is_err());
        assert_eq!(budget.usage().live_bytes, 30);
        current.retain_bytes(25).unwrap();
        assert_eq!(budget.usage().live_bytes, 25);
        drop(current);
        assert_eq!(budget.usage().live_bytes, 0);
        let cancelled = || true;
        assert!(matches!(budget.reserve_preparation_bytes(1, &Interrupt::cancel_only(&cancelled)), Err(RasterInputError::Cancelled)));
        assert_eq!(budget.usage().live_bytes, 0);
    }
    #[test]
    fn real_preview_leases_balance_and_hold_returned_pixels() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::with_background("Own observed preview", Size::new(19, 13), ColorMode::Rgb, depth, Color::rgba(0.21, 0.63, 0.18, 0.47));
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(3, 4, 12, 9), &[0.73, 0.41, 0.28, 0.0]);
            let before = doc.clone();
            let events = Mutex::new(Vec::new());
            let observe = |event| events.lock().unwrap().push(event);
            let budget = PreviewPreparationBudget::new(8 << 20, &observe);
            for side in [7, 256] {
                let result = thumbnail_with_preparation_budget(&doc, side, &Interrupt::NONE, &budget).unwrap();
                assert_eq!(result.image(), &thumbnail(&doc, side).unwrap());
                assert_eq!(budget.usage().live_bytes, result.image().pixels.capacity());
                drop(result);
                assert_eq!(budget.usage().live_bytes, 0);
            }
            let sum: i64 = events
                .lock()
                .unwrap()
                .iter()
                .map(|e| match e {
                    PreparationCharge::Reserve(n) => *n as i64,
                    PreparationCharge::Release(n) => -(*n as i64),
                })
                .sum();
            assert_eq!(sum, 0);
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn observed_minimum_peak_is_real_and_shared_between_returned_images() {
        let doc = Document::with_background("Own peak", Size::new(8, 6), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let observer = |_| {};
        let measuring = PreviewPreparationBudget::new(8 << 20, &observer);
        let first = thumbnail_with_preparation_budget(&doc, 256, &Interrupt::NONE, &measuring).unwrap();
        let second = thumbnail_with_preparation_budget(&doc, 1024, &Interrupt::NONE, &measuring).unwrap();
        let peak = measuring.usage().peak_bytes;
        assert!(peak < 8 << 20);
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads().max(1);
        #[cfg(target_arch = "wasm32")]
        let workers = 1usize;
        // The second image retains the first image's 192-byte payload. With
        // this cap exactly one source row fits, so cap-minus-one cannot be
        // rescued by another reduction in band height.
        let minimum = 192 + 768 + 56 + 128 + 8 * 16 * 10 * workers;
        drop(first);
        drop(second);
        assert_eq!(measuring.usage().live_bytes, 0);
        let exact = PreviewPreparationBudget::new(minimum, &observer);
        let a = thumbnail_with_preparation_budget(&doc, 256, &Interrupt::NONE, &exact).unwrap();
        let b = thumbnail_with_preparation_budget(&doc, 1024, &Interrupt::NONE, &exact).unwrap();
        drop(a);
        drop(b);
        let short = PreviewPreparationBudget::new(minimum - 1, &observer);
        let a = thumbnail_with_preparation_budget(&doc, 256, &Interrupt::NONE, &short).unwrap();
        assert!(matches!(thumbnail_with_preparation_budget(&doc, 1024, &Interrupt::NONE, &short), Err(RasterInputError::LimitExceeded)));
        drop(a);
        assert_eq!(short.usage().live_bytes, 0);
    }
    #[test]
    fn observer_triggered_cancel_refunds_before_return_and_styles_remain_explicit() {
        let mut doc = Document::with_background("Own cancel", Size::new(8, 6), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let before = doc.clone();
        for stop_at in [1, 3, 4] {
            let cancel = AtomicBool::new(false);
            let reservations = AtomicUsize::new(0);
            let observe = |e| {
                if matches!(e, PreparationCharge::Reserve(_)) && reservations.fetch_add(1, Ordering::Relaxed) + 1 >= stop_at {
                    cancel.store(true, Ordering::Relaxed)
                }
            };
            let budget = PreviewPreparationBudget::new(8 << 20, &observe);
            let cancelled = || cancel.load(Ordering::Relaxed);
            assert!(matches!(thumbnail_with_preparation_budget(&doc, 256, &Interrupt::cancel_only(&cancelled), &budget), Err(RasterInputError::Cancelled)));
            assert_eq!(budget.usage().live_bytes, 0);
            assert_eq!(doc, before);
            assert!(reservations.load(Ordering::Relaxed) >= stop_at);
        }
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(8 << 20, &observer);
        doc.layers[0].mask = Some(photocraft_doc::LayerMask::reveal_all());
        assert!(!plain_preview_supported(&doc));
        assert!(matches!(thumbnail_with_preparation_budget(&doc, 256, &Interrupt::NONE, &budget), Err(RasterInputError::UnsupportedConversion)));
    }
}
