//! Qualified ordinary styled-preview accounting. Other style families keep
//! their legacy API; this is not a general allocator/RSS or native-large claim.
use super::*;
use photocraft_raster::{Interrupt, RasterInputError};
use preview_budget::{ChargedThumbnail, PreviewPreparationBudget};
pub fn styled_preview_supported(doc: &Document) -> bool {
    doc.size.width > 0
        && doc.size.height > 0
        && doc.size.width <= 262144
        && doc.size.height <= 262144
        && doc.xcf_state.is_none()
        && doc.sample_encoding.is_none()
        && doc.source_color_context.is_none()
        && doc.icc_profile.is_none()
        && matches!(doc.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
        && matches!(doc.depth, photocraft_color::SampleType::U8 | photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32)
        && doc.layers.len() <= 128
        && doc.patterns.len() <= 64
        && (doc.size.area() <= PROXY_THUMBNAIL_PIXELS
            || doc.layers.iter().any(|l| effects::has_effects(l) || l.vector_mask.as_ref().is_some_and(|v| v.enabled)))
        && doc.layers.iter().all(|layer| {
            let content = match &layer.content {
                LayerContent::Raster(s) => s.format().mode == doc.mode && s.format().sample == doc.depth,
                LayerContent::Fill(Fill::Pattern { .. }) => !effects::has_effects(layer),
                _ => false,
            };
            let styles = layer.effects.psd_raw.is_none()
                && layer.effects.items.len() <= 32
                && layer.effects.items.iter().all(|e| match e {
                    photocraft_doc::Effect::DropShadow(s) | photocraft_doc::Effect::InnerShadow(s) => matches!(s.contour, photocraft_doc::Contour::Linear),
                    _ => false,
                });
            content
                && styles
                && layer.blend == BlendMode::Normal
                && !layer.clipped
                && layer.advanced.is_default()
                && layer.import_compositing.is_none()
                && layer.video.is_none()
                && (!effects::has_effects(layer)
                    || (layer.vector_mask.is_none() && effects::validate_checked_shadows(layer).is_ok() && matches!(layer.content, LayerContent::Raster(_))))
        })
}
pub fn thumbnail_with_styled_preparation_budget<'a, 'b>(
    doc: &Document,
    side: u32,
    control: &Interrupt<'_>,
    budget: &'a PreviewPreparationBudget<'b>,
) -> Result<ChargedThumbnail<'a, 'b>, RasterInputError> {
    thumbnail_with_styled_preparation_budget_sized(doc, side, control, budget, None)
}
pub(crate) fn thumbnail_with_styled_preparation_budget_sized<'a, 'b>(
    doc: &Document,
    side: u32,
    control: &Interrupt<'_>,
    budget: &'a PreviewPreparationBudget<'b>,
    output: Option<(usize, usize)>,
) -> Result<ChargedThumbnail<'a, 'b>, RasterInputError> {
    use RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    if !styled_preview_supported(doc) {
        return Err(E::UnsupportedConversion);
    }
    proxy::validate_surfaces_with_interrupt(doc, control)?;
    // Slot metadata plus known prepared pattern pixels, before allocation.
    let pattern_bytes = pattern::checked_reservation_bytes(&doc.patterns, control)?;
    let pattern_lease = budget.reserve(pattern_bytes, control)?;
    let patterns = pattern::PreparedPatterns::try_new_with_interrupt(&doc.patterns, pattern_bytes, control)?;
    // Existing shadow-key Debug formatting is bounded by this admitted literal
    // schema (no raw metadata/resources/vector paths or other effect families).
    let key_bytes = doc.layers.iter().try_fold(1024usize, |n, l| n.checked_add(l.effects.items.len().checked_mul(8192)?)).ok_or(E::LimitExceeded)?;
    let hash_guard = budget.reserve(key_bytes, control)?;
    let mut cx = Ctx::for_doc(doc, &patterns);
    cx.checked_effects = true;
    cx.vector_control = Some(control);
    cx.preparation = Some(preparation_bridge::PreparationBridge::new(budget, control)?);
    cx.checked_feathers.register_sources_budgeted(doc, control, cx.preparation.as_ref())?;
    super::render_interrupt::prepare_checked_effects(doc, &cx, control)?;
    // Prepare immutable mask geometry/maps before reduction allocates its band.
    // Otherwise a valid cache's construction peak can be forced to overlap the
    // largest fitting band, even though only small mask reads overlap rendering.
    let probe = Rect::new(0, 0, 1, 1);
    for layer in &doc.layers {
        control.check().map_err(|_| E::Cancelled)?;
        let values = mask_vals(layer, probe, &cx);
        cx.checked_vectors.error()?;
        drop(values);
        if let Some(bridge) = cx.preparation.as_ref() {
            bridge.release_transients();
        }
    }
    cx.checked_vectors.error()?;
    let result = preview_budget::thumbnail_with_budget_backend_sized(
        doc,
        side,
        control,
        budget,
        preview_budget::ReductionPlan { weight: doc.layers.len().max(1), workers: 1, output },
        |rect| super::render_interrupt::render_region(doc, rect, &cx, control),
    );
    drop(cx);
    drop(hash_guard);
    drop(patterns);
    drop(pattern_lease);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, LayerMask, SampleType, Size, VectorMask};
    use preview_budget::PreparationCharge;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    fn own_styled(depth: SampleType, shadow: bool) -> Document {
        let mut doc = Document::with_background("Own ledger styles", Size::new(48, 36), ColorMode::Rgb, depth, Color::rgba(0.21, 0.63, 0.17, 0.71));
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(3, 4, 13, 10), &[0.73, 0.28, 0.41, 0.0]);
        let mut mask = LayerMask::hide_all();
        std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(5, 7, 39, 31), &[0.73]);
        mask.feather = 4.25;
        mask.density = 0.81;
        doc.layers[0].mask = Some(mask);
        if shadow {
            doc.layers[0].effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        } else {
            let mut vector = VectorMask::new(photocraft_vector::shapes::ellipse(2.25, 3.75, 42.5, 29.25));
            vector.feather = 2.25;
            vector.density = 0.79;
            doc.layers[0].vector_mask = Some(vector);
        }
        doc
    }
    #[test]
    fn styles_keep_byte_parity_and_outputs_share_live_preparation_ledger() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for shadow in [false, true] {
                let doc = own_styled(depth, shadow);
                let before = doc.clone();
                assert!(styled_preview_supported(&doc));
                let observer = |_| {};
                let budget = PreviewPreparationBudget::new(8 << 20, &observer);
                let first = thumbnail_with_styled_preparation_budget(&doc, 17, &Interrupt::NONE, &budget).unwrap();
                assert_eq!(first.image(), &thumbnail(&doc, 17).unwrap());
                assert_eq!(budget.usage().live_bytes, first.image().pixels.capacity());
                let second = thumbnail_with_styled_preparation_budget(&doc, 256, &Interrupt::NONE, &budget).unwrap();
                assert_eq!(second.image(), &thumbnail(&doc, 256).unwrap());
                assert_eq!(budget.usage().live_bytes, first.image().pixels.capacity() + second.image().pixels.capacity());
                assert!(budget.usage().peak_bytes <= 8 << 20);
                drop(first);
                drop(second);
                assert_eq!(budget.usage().live_bytes, 0);
                assert_eq!(doc, before);
            }
        }
    }
    #[test]
    fn cache_and_inner_staging_cancellation_refund_and_never_publish_pixels() {
        let doc = own_styled(SampleType::U16, false);
        let before = doc.clone();
        for stop_at in [3, 5, 8, 12] {
            let cancel = AtomicBool::new(false);
            let count = AtomicUsize::new(0);
            let observe = |event| {
                if matches!(event, PreparationCharge::Reserve(_)) && count.fetch_add(1, Ordering::Relaxed) + 1 == stop_at {
                    cancel.store(true, Ordering::Relaxed);
                }
            };
            let budget = PreviewPreparationBudget::new(8 << 20, &observe);
            let cancelled = || cancel.load(Ordering::Relaxed);
            assert!(
                matches!(thumbnail_with_styled_preparation_budget(&doc, 256, &Interrupt::cancel_only(&cancelled), &budget), Err(RasterInputError::Cancelled)),
                "stage {stop_at}"
            );
            assert_eq!(budget.usage().live_bytes, 0);
            assert_eq!(doc, before);
        }
        let observer = |_| {};
        let small = PreviewPreparationBudget::new(48 << 10, &observer);
        assert!(matches!(thumbnail_with_styled_preparation_budget(&doc, 256, &Interrupt::NONE, &small), Err(RasterInputError::LimitExceeded)));
        assert_eq!(small.usage().live_bytes, 0);
        let budget = PreviewPreparationBudget::new(8 << 20, &observer);
        let result = thumbnail_with_styled_preparation_budget(&doc, 256, &Interrupt::NONE, &budget).unwrap();
        assert_eq!(result.image(), &thumbnail(&doc, 256).unwrap());
        drop(result);
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(doc, before);
    }
    #[test]
    fn prepared_pattern_payload_is_charged_with_masks_and_output() {
        let mut doc = own_styled(SampleType::U16, false);
        let mut tile = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA16);
        tile.fill_rect(Rect::new(0, 0, 3, 2), &[0.61, 0.23, 0.78, 0.47]);
        tile.fill_rect(Rect::new(1, 0, 2, 1), &[0.12, 0.73, 0.31, 0.91]);
        let pattern = photocraft_doc::Pattern::new("Own literal tile", tile, 3, 2);
        doc.layers[0].content =
            LayerContent::Fill(Fill::Pattern { name: pattern.name.clone(), id: pattern.id.clone(), scale: 1.25, angle: 17., link: true, phase: (2., -3.) });
        doc.patterns.push(pattern);
        let before = doc.clone();
        assert!(styled_preview_supported(&doc));
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(8 << 20, &observer);
        let image = thumbnail_with_styled_preparation_budget(&doc, 256, &Interrupt::NONE, &budget).unwrap();
        assert_eq!(image.image(), &thumbnail(&doc, 256).unwrap());
        assert_eq!(budget.usage().live_bytes, image.image().pixels.capacity());
        drop(image);
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(doc, before);
    }
}
