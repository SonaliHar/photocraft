//! Cooperative ordinary display rendering. Qualified zero-spread shadows use
//! checked preparation, as do ordinary unfeathered vector masks. Feathered masks,
//! source-shape regeneration and unported styles retain internal polling gaps;
//! no full active-polling contract is implied.
use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_raster::{Interrupt, RasterInputError};
/// Known ordinary render-region envelope; does not qualify retained source
/// rendering or make legacy style preparation internally cancellable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrdinaryRenderRequirements {
    pub output_bytes: usize,
    pub scratch_bytes: usize,
    pub preparation_reserve_bytes: usize,
    pub work_units: u64,
    pub peak_bytes: usize,
    columns: usize,
    rows: usize,
}
/// Borrowed metadata-only calculation shared by the renderer and aggregate
/// callers. Existing worker count, microblock sizing and ceilings are retained.
/// This calculation is not permission to render a canonical imported source;
/// each existing renderer entry keeps its original admission policy.
pub fn ordinary_render_requirements(doc: &Document, rect: Rect, control: &Interrupt<'_>) -> Result<OrdinaryRenderRequirements, RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    let bytes = photocraft_raster::checked_region_byte_len(rect, photocraft_color::PixelFormat::RGBA32F)?;
    if rect.is_empty() {
        return Ok(OrdinaryRenderRequirements {
            output_bytes: 0,
            scratch_bytes: 0,
            preparation_reserve_bytes: 0,
            work_units: 0,
            peak_bytes: 0,
            columns: 1,
            rows: 1,
        });
    }
    let (layers, depth) = tree_weight(&doc.layers, 0, control)?;
    let pixels = bytes / 16;
    let work = (pixels as u64).checked_mul(layers.max(1) as u64).ok_or(E::LimitExceeded)?;
    if work > (1_u64 << 32) {
        return Err(E::LimitExceeded);
    }
    let area = (65536 / layers.max(1)).max(1);
    let columns = (rect.width() as usize).min(256).min(area).max(1);
    let rows = (area / columns).clamp(1, 8);
    #[cfg(not(target_arch = "wasm32"))]
    let workers = rayon::current_num_threads();
    #[cfg(target_arch = "wasm32")]
    let workers = 1usize;
    let staged = columns
        .checked_mul(rows)
        .and_then(|n| n.checked_mul(16))
        .and_then(|n| n.checked_mul(depth + 8))
        .and_then(|n| n.checked_mul(workers))
        .ok_or(E::LimitExceeded)?;
    let preparation = pattern::PREPARED_PATTERN_BYTES
        .checked_add(64 << 20)
        .and_then(|n| n.checked_add(vector_interrupt::VECTOR_BYTES))
        .and_then(|n| n.checked_add(feather_cache::FEATHER_BYTES))
        .ok_or(E::LimitExceeded)?;
    let peak = bytes.checked_add(staged).and_then(|n| n.checked_add(preparation)).ok_or(E::LimitExceeded)?;
    if peak as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    control.check().map_err(|_| E::Cancelled)?;
    Ok(OrdinaryRenderRequirements {
        output_bytes: bytes,
        scratch_bytes: staged,
        preparation_reserve_bytes: preparation,
        work_units: work,
        peak_bytes: peak,
        columns,
        rows,
    })
}
/// Same ordinary compositor values, with an additive cancellation-aware sink.
/// A cancelled band is never delivered. Canonical imported rendering uses its
/// own PreparedRenderer rather than this legacy display-buffer path.
pub fn render_bands_with_interrupt(
    doc: &Document,
    rect: Rect,
    band_rows: i32,
    control: &Interrupt<'_>,
    mut sink: impl FnMut(Buffer) -> Result<(), RasterInputError>,
) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    proxy::validate_surfaces_with_interrupt(doc, control)?;
    if doc.xcf_state.is_some() {
        return Err(RasterInputError::UnsupportedConversion);
    }
    photocraft_raster::checked_region_byte_len(rect, photocraft_color::PixelFormat::GRAY8)?;
    if rect.is_empty() {
        return Ok(());
    }
    let cmyk = cmyk_space(doc);
    let patterns = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        pattern::PreparedPatterns::try_new_in_mode_with_interrupt(&doc.patterns, pattern::PREPARED_PATTERN_BYTES, doc.pixel_format().mode, control)
    })?;
    let mut cx = Ctx::for_doc(doc, &patterns);
    cx.checked_effects = true;
    cx.vector_control = Some(control);
    cx.checked_feathers.register_sources(doc, control)?;
    let rows = band_rows_for(rect.width(), band_rows);
    let mut y = rect.y0;
    while y < rect.y1 {
        let end = y.saturating_add(rows).min(rect.y1);
        sink(render_region(doc, Rect::new(rect.x0, y, rect.x1, end), &cx, control)?)?;
        y = end;
    }
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(())
}
/// Whole ordinary display region with checked allocation and cooperative microblocks.
/// Rendering preparations retain the backend limitations documented above.
pub fn render_with_interrupt(doc: &Document, rect: Rect, control: &Interrupt<'_>) -> Result<Buffer, RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    proxy::validate_surfaces_with_interrupt(doc, control)?;
    if doc.xcf_state.is_some() {
        return Err(RasterInputError::UnsupportedConversion);
    }
    photocraft_raster::checked_region_byte_len(rect, photocraft_color::PixelFormat::RGBA32F)?;
    let cmyk = cmyk_space(doc);
    let patterns = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        pattern::PreparedPatterns::try_new_in_mode_with_interrupt(&doc.patterns, pattern::PREPARED_PATTERN_BYTES, doc.pixel_format().mode, control)
    })?;
    let mut cx = Ctx::for_doc(doc, &patterns);
    cx.checked_effects = true;
    cx.vector_control = Some(control);
    cx.checked_feathers.register_sources(doc, control)?;
    render_region(doc, rect, &cx, control)
}

pub(super) fn render_region(doc: &Document, rect: Rect, cx: &Ctx, control: &Interrupt<'_>) -> Result<Buffer, RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    let requirements = ordinary_render_requirements(doc, rect, control)?;
    let bytes = requirements.output_bytes;
    if rect.is_empty() {
        return Ok(Buffer { rect, px: Vec::new() });
    }
    let pixels = bytes / 16;
    let columns = requirements.columns;
    let rows = requirements.rows;
    let peak = requirements.peak_bytes;
    // Preserve legacy effect-map/cache scope before microblocks. These existing
    // backend preparations can still delay cancellation inside a single call.
    let cmyk = cmyk_space(doc);
    let lab = doc.mode == photocraft_color::ColorMode::Lab;
    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        psblend::LAB_MIX.with(|state| state.set(lab));
        let result = prepare_checked_effects(doc, cx, control);
        if result.is_ok() {
            // Nonported styles retain baseline behavior. This legacy call is
            // intentionally still an unpolled preparation gap, not a fully
            // checked fallback claimed as qualified.
            prepare_effects(&doc.layers, rect, cx, |f| {
                photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
                    psblend::LAB_MIX.with(|state| state.set(lab));
                    f();
                    psblend::LAB_MIX.with(|state| state.set(false));
                });
            });
        }
        psblend::LAB_MIX.with(|state| state.set(false));
        result
    })?;
    cx.checked_vectors.error()?;
    control.check().map_err(|_| E::Cancelled)?;
    let mut output = Vec::new();
    output.try_reserve_exact(pixels).map_err(|_| E::AllocationFailed)?;
    let capacity = output.capacity().checked_mul(16).ok_or(E::LimitExceeded)?;
    if capacity.checked_add(peak - bytes).is_none_or(|n| n as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(E::LimitExceeded);
    }
    while output.len() < pixels {
        control.check().map_err(|_| E::Cancelled)?;
        output.resize(output.len().saturating_add(4096).min(pixels), [0.0; 4]);
    }
    let width = rect.width() as usize;
    let band_len = width.checked_mul(rows).ok_or(E::LimitExceeded)?;
    let paint = |index: usize, band: &mut [[f32; 4]]| -> Result<(), E> {
        control.check().map_err(|_| E::Cancelled)?;
        let offset = index.checked_mul(rows).ok_or(E::LimitExceeded)?;
        let y = i32::try_from(i64::from(rect.y0) + offset as i64).map_err(|_| E::InvalidBounds)?;
        let end = y.saturating_add(rows as i32).min(rect.y1);
        let mut x = rect.x0;
        while x < rect.x1 {
            control.check().map_err(|_| E::Cancelled)?;
            let right = x.saturating_add(columns as i32).min(rect.x1);
            let part = render_tiled_with(doc, Rect::new(x, y, right, end), RENDER_TILE, cx);
            cx.checked_vectors.error()?;
            control.check().map_err(|_| E::Cancelled)?;
            let part_width = part.rect.width() as usize;
            for (row, source) in part.px.chunks_exact(part_width).enumerate() {
                control.check().map_err(|_| E::Cancelled)?;
                let start = row.checked_mul(width).and_then(|n| n.checked_add((x - rect.x0) as usize)).ok_or(E::LimitExceeded)?;
                let stop = start.checked_add(part_width).ok_or(E::LimitExceeded)?;
                band.get_mut(start..stop).ok_or(E::LengthMismatch)?.copy_from_slice(source);
            }
            drop(part);
            if let Some(bridge) = cx.preparation.as_ref() {
                bridge.release_transients();
            }
            x = right;
        }
        Ok(())
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        if cx.preparation.is_some() {
            for (index, band) in output.chunks_mut(band_len).enumerate() {
                paint(index, band)?;
            }
        } else {
            output.par_chunks_mut(band_len).enumerate().try_for_each(|(index, band)| paint(index, band))?;
        }
    }
    #[cfg(target_arch = "wasm32")]
    for (index, band) in output.chunks_mut(band_len).enumerate() {
        paint(index, band)?;
    }
    control.check().map_err(|_| E::Cancelled)?;
    Ok(Buffer { rect, px: output })
}
pub(super) fn prepare_checked_effects(doc: &Document, cx: &Ctx, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    use RasterInputError as E;
    fn collect<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>, depth: usize, control: &Interrupt<'_>) -> Result<(), E> {
        if depth >= 100 {
            return Err(E::LimitExceeded);
        }
        for l in layers {
            control.check().map_err(|_| E::Cancelled)?;
            if effects::has_effects(l) {
                // Ported shadow preparation is additive. Other established
                // families still use the legacy preparation below; their
                // internal cancellation/accounting gaps remain explicit.
                if matches!(l.content, LayerContent::Raster(_))
                    && l.vector_mask.is_none()
                    && l.advanced.transparency_shapes
                    && !l.advanced.layer_mask_hides_effects
                    && !l.advanced.vector_mask_hides_effects
                    && match effects::validate_checked_shadows_with_interrupt(l, control) {
                        Ok(_) => true,
                        Err(E::Cancelled) => return Err(E::Cancelled),
                        Err(_) => false,
                    }
                {
                    out.try_reserve(1).map_err(|_| E::AllocationFailed)?;
                    out.push(l);
                    if out.len() > 100000 {
                        return Err(E::LimitExceeded);
                    }
                }
            }
            if let Some(children) = l.children() {
                collect(children, out, depth + 1, control)?;
            }
        }
        Ok(())
    }
    // Flat effect-free views have no collection to allocate. In a charged
    // render this matters: the largest fitting reduction band may occupy the
    // remaining ledger, and reserving an unused pointer vector would falsely
    // deny a valid preview. Groups still use the bounded recursive collector.
    if doc.layers.iter().all(|layer| layer.children().is_none() && !effects::has_effects(layer)) {
        control.check().map_err(|_| E::Cancelled)?;
        return Ok(());
    }
    let collection_lease = cx
        .preparation
        .as_ref()
        .map(|b| b.budget.reserve(doc.layers.len().checked_mul(std::mem::size_of::<&Layer>()).ok_or(E::LimitExceeded)?, control))
        .transpose()?;
    let mut layers = Vec::new();
    if collection_lease.is_some() {
        layers.try_reserve_exact(doc.layers.len()).map_err(|_| E::AllocationFailed)?;
        if layers.capacity() != doc.layers.len() {
            return Err(E::LimitExceeded);
        }
    }
    collect(&doc.layers, &mut layers, 0, control)?;
    let mut retained = 0_usize;
    let mut maximum_scratch = 0_usize;
    for layer in &layers {
        let margin = effects::margin(layer);
        let region = layer_bounds(layer, cx.canvas).inflate(margin).intersect(&cx.canvas.inflate(margin));
        retained = retained.checked_add(effects::checked_effect_map_retained_bytes(layer, region, control)?).ok_or(E::LimitExceeded)?;
        let scratch = effects::checked_effect_map_scratch_bytes(layer, region, control)?;
        maximum_scratch = maximum_scratch.max(scratch);
    }
    if retained.checked_add(maximum_scratch).is_none_or(|n| n > 64 << 20) {
        return Err(E::LimitExceeded);
    }
    for layer in &layers {
        control.check().map_err(|_| E::Cancelled)?;
        let margin = effects::margin(layer);
        let region = layer_bounds(layer, cx.canvas).inflate(margin).intersect(&cx.canvas.inflate(margin));
        let key = effect_cache_key(layer, region, cx);
        if cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&key) {
            continue;
        }
        let len = photocraft_raster::checked_region_byte_len(region, photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false))? / 4;
        let shape_lease = cx.preparation.as_ref().map(|b| b.budget.reserve(len.checked_mul(4).ok_or(E::LimitExceeded)?, control)).transpose()?;
        let mut shape = Vec::new();
        shape.try_reserve_exact(len).map_err(|_| E::AllocationFailed)?;
        if shape.capacity() != len {
            return Err(E::LimitExceeded);
        }
        while shape.len() < len {
            control.check().map_err(|_| E::Cancelled)?;
            shape.resize(shape.len().saturating_add(4096).min(len), 0.0);
        }
        if !region.is_empty() {
            let width = region.width() as usize;
            for y in region.y0..region.y1 {
                let mut x = region.x0;
                while x < region.x1 {
                    control.check().map_err(|_| E::Cancelled)?;
                    let right = x.saturating_add(1024).min(region.x1);
                    let scratch = cx.preparation.as_ref().map(|b| b.budget.reserve(1024 * 16 * 12, control)).transpose()?;
                    let values = effect_shape(layer, Rect::new(x, y, right, y + 1), cx);
                    let start = (y - region.y0) as usize * width + (x - region.x0) as usize;
                    cx.checked_vectors.error()?;
                    shape.get_mut(start..start + values.len()).ok_or(E::LengthMismatch)?.copy_from_slice(&values);
                    drop(values);
                    drop(scratch);
                    if let Some(bridge) = cx.preparation.as_ref() {
                        bridge.release_transients();
                    }
                    x = right;
                }
            }
        }
        let stage = cx.preparation.as_ref().map(|b| b.stage(64 << 20, control)).transpose()?;
        let ceiling = stage.as_ref().map_or(64 << 20, |s| s.bytes());
        let maps = effects::try_build_shadow_maps(layer, shape, region, &cx.light, ceiling, control)?;
        if let (Some(bridge), Some(stage)) = (cx.preparation.as_ref(), stage) {
            bridge.retained(stage, maps.checked_payload_bytes()?.checked_add(1024).ok_or(E::LimitExceeded)?)?;
        }
        drop(shape_lease);
        control.check().map_err(|_| E::Cancelled)?;
        let mut table = cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner());
        table.try_reserve(1).map_err(|_| E::AllocationFailed)?;
        table.insert(key, (std::sync::Arc::new(maps), false));
    }
    Ok(())
}

fn tree_weight(layers: &[Layer], depth: usize, control: &Interrupt<'_>) -> Result<(usize, usize), RasterInputError> {
    if depth >= 100 {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut count = 0_usize;
    let mut deepest = depth;
    for layer in layers {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        count = count.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
        if let Some(children) = layer.children() {
            let (nested, end) = tree_weight(children, depth + 1, control)?;
            count = count.checked_add(nested).ok_or(RasterInputError::LimitExceeded)?;
            deepest = deepest.max(end);
        }
        if count > 100000 {
            return Err(RasterInputError::LimitExceeded);
        }
    }
    Ok((count, deepest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn requirements_keep_existing_envelope_and_render_values() {
        let doc = Document::with_background("requirements draft", Size::new(19, 11), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let rect = Rect::new(0, 0, 19, 11);
        let req = ordinary_render_requirements(&doc, rect, &Interrupt::NONE).unwrap();
        assert_eq!(req.output_bytes, 19 * 11 * 16);
        assert_eq!(req.work_units, 19 * 11);
        assert_eq!(req.preparation_reserve_bytes, pattern::PREPARED_PATTERN_BYTES + (64 << 20) + vector_interrupt::VECTOR_BYTES + feather_cache::FEATHER_BYTES);
        assert_eq!(req.peak_bytes, req.output_bytes + req.scratch_bytes + req.preparation_reserve_bytes);
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads();
        #[cfg(target_arch = "wasm32")]
        let workers = 1;
        assert_eq!(req.scratch_bytes, 19 * 8 * 16 * 8 * workers);
        assert_eq!(render_with_interrupt(&doc, rect, &Interrupt::NONE).unwrap().px, render(&doc, rect).unwrap().px);
        let empty = ordinary_render_requirements(&doc, Rect::new(0, 0, 0, 0), &Interrupt::NONE).unwrap();
        assert_eq!(empty.peak_bytes, 0);
        assert!(matches!(ordinary_render_requirements(&doc, rect, &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled)));
    }
    #[test]
    fn effect_free_preparation_does_not_reserve_an_unused_collection() {
        let doc = Document::with_background("Own full-ledger plain view", Size::new(8, 6), ColorMode::Rgb, SampleType::U16, Color::WHITE);
        let before = doc.clone();
        let observer = |_| {};
        let budget = preview_budget::PreviewPreparationBudget::new(1 << 20, &observer);
        let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES);
        let mut cx = Ctx::for_doc(&doc, &patterns);
        cx.preparation = Some(preparation_bridge::PreparationBridge::new(&budget, &Interrupt::NONE).unwrap());
        let band = budget.reserve(budget.available(), &Interrupt::NONE).unwrap();
        let peak = budget.usage().peak_bytes;
        prepare_checked_effects(&doc, &cx, &Interrupt::NONE).unwrap();
        assert_eq!(budget.usage().peak_bytes, peak);
        let cancelled = || true;
        assert_eq!(prepare_checked_effects(&doc, &cx, &Interrupt::cancel_only(&cancelled)), Err(RasterInputError::Cancelled));
        drop(band);
        drop(cx);
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(doc, before);
    }
    #[test]
    fn checked_feather_preview_matches_baseline_and_never_delivers_cancelled_preparation() {
        use photocraft_doc::{LayerMask, VectorMask};
        let mut doc = Document::with_background("own feather preview", Size::new(256, 192), ColorMode::Rgb, SampleType::U16, Color::WHITE);
        let mut pixel = LayerMask::hide_all();
        std::sync::Arc::make_mut(&mut pixel.surface).fill_rect(Rect::new(15, 20, 178, 151), &[0.73]);
        pixel.feather = 5.25;
        pixel.density = 0.87;
        let mut vector = VectorMask::new(photocraft_vector::shapes::ellipse(-12.7, 5.1, 240.3, 172.7));
        vector.feather = 7.25;
        vector.density = 0.83;
        doc.layers[0].mask = Some(pixel);
        doc.layers[0].vector_mask = Some(vector);
        let before = doc.clone();
        let expected = render(&doc, doc.bounds()).unwrap();
        assert_eq!(render_with_interrupt(&doc, doc.bounds(), &Interrupt::NONE).unwrap(), expected);
        assert_eq!(crate::thumbnail_with_interrupt(&doc, 96, &Interrupt::NONE).unwrap(), thumbnail(&doc, 96).unwrap());
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        let mut delivered = 0;
        assert!(matches!(
            render_bands_with_interrupt(&doc, doc.bounds(), 192, &Interrupt::cancel_only(&cancel), |_| {
                delivered += 1;
                Ok(())
            }),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(delivered, 0);
        assert_eq!(doc, before);
    }
    #[test]
    fn over_limit_vector_components_fail_before_preview_delivery() {
        use photocraft_doc::{Path, Subpath, VectorMask};
        let mut doc = Document::with_background("own bounded mask", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut path = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)])]);
        path.subpaths.resize(100001, Subpath::polygon(&[]));
        doc.layers[0].vector_mask = Some(VectorMask::new(path));
        let mut delivered = 0;
        assert!(matches!(
            render_bands_with_interrupt(&doc, doc.bounds(), 1, &Interrupt::NONE, |_| {
                delivered += 1;
                Ok(())
            }),
            Err(RasterInputError::LimitExceeded)
        ));
        assert_eq!(delivered, 0);
        assert_eq!(doc.layers[0].vector_mask.as_ref().unwrap().path.subpaths.len(), 100001);
    }
    #[test]
    fn distinct_temporary_hiding_masks_keep_their_original_owner_identity() {
        use photocraft_doc::{Path, Subpath, VectorMask};
        let mut doc = Document::with_background("own hiding masks", Size::new(64, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.layers[0].effects.items.push(photocraft_doc::Effect::Stroke(photocraft_doc::StrokeFx {
            common: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Normal, 0.8),
            size: 2.0,
            position: photocraft_doc::StrokePosition::Outside,
            paint: photocraft_doc::FxPaint::Color(Color::BLACK),
        }));
        let mut right = doc.layers[0].clone();
        doc.layers[0].vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (24.0, 0.0), (24.0, 32.0), (0.0, 32.0)])])));
        right.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(32.0, 0.0), (64.0, 0.0), (64.0, 32.0), (32.0, 32.0)])])));
        right.advanced.vector_mask_hides_effects = true;
        doc.layers[0].advanced.vector_mask_hides_effects = true;
        doc.layers.push(right);
        let patterns = pattern::PreparedPatterns::new(&[], pattern::PREPARED_PATTERN_BYTES);
        for checked in [false, true] {
            let mut cx = Ctx::for_doc(&doc, &patterns);
            if checked {
                cx.vector_control = Some(&Interrupt::NONE);
            }
            for _ in 0..3 {
                for layer in &doc.layers {
                    let expected = photocraft_vector::CompiledVectorMask::new(layer.vector_mask.as_ref().unwrap()).render(doc.bounds());
                    assert_eq!(advanced::hiding_mask(layer, doc.bounds(), &cx).unwrap(), expected);
                    assert!(cx.checked_vectors.error().is_ok());
                }
            }
        }
    }
    #[test]
    fn rejected_vector_geometry_is_not_delivered_as_a_placeholder() {
        use photocraft_doc::{Path, Subpath, VectorMask};
        let mut doc = Document::with_background("own invalid mask", Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.layers[0].vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(f64::NAN, 0.0), (20.0, 0.0), (20.0, 20.0)])])));
        doc.layers[0].advanced.vector_mask_hides_effects = false;
        doc.layers[0].effects.items.push(photocraft_doc::Effect::DropShadow(photocraft_doc::Shadow {
            common: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Multiply, 0.57),
            color: Color::BLACK,
            angle: 37.0,
            use_global_light: false,
            distance: 3.25,
            spread: 0.0,
            size: 4.75,
            contour: photocraft_doc::Contour::Linear,
            anti_alias: true,
            noise: 0.0,
            knocks_out: true,
        }));
        let mut delivered = 0;
        assert!(matches!(
            render_bands_with_interrupt(&doc, doc.bounds(), 1, &Interrupt::NONE, |_| {
                delivered += 1;
                Ok(())
            }),
            Err(RasterInputError::InvalidSample)
        ));
        assert_eq!(delivered, 0);
        let patterns = pattern::PreparedPatterns::new(&[], pattern::PREPARED_PATTERN_BYTES);
        let cx = Ctx::for_doc(&doc, &patterns);
        let margin = effects::margin(&doc.layers[0]);
        let region = layer_bounds(&doc.layers[0], cx.canvas).inflate(margin).intersect(&cx.canvas.inflate(margin));
        let key = effect_cache_key(&doc.layers[0], region, &cx);
        assert!(fx_cache().lock().unwrap_or_else(|e| e.into_inner()).map.get(&key).is_none_or(|slot| slot.get().is_none()));
    }
    #[test]
    fn vector_preparation_cancels_before_any_band_is_delivered() {
        use photocraft_doc::{Path, Subpath, VectorMask};
        let mut doc = Document::with_background("own long mask", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let points: Vec<_> = (0..10000).map(|i| (i as f64, ((i * 73) % 101) as f64)).collect();
        doc.layers[0].vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&points)])));
        let before = doc.clone();
        // Negative control: the unchanged legacy Ctx cannot observe polling
        // inside vector compilation, so this same callback stays below 100.
        let legacy_patterns = pattern::PreparedPatterns::new(&[], pattern::PREPARED_PATTERN_BYTES);
        let legacy_cx = Ctx::for_doc(&doc, &legacy_patterns);
        let legacy_polls = AtomicUsize::new(0);
        let legacy_cancel = || legacy_polls.fetch_add(1, Ordering::Relaxed) >= 100;
        assert!(render_region(&doc, doc.bounds(), &legacy_cx, &Interrupt::cancel_only(&legacy_cancel)).is_ok());
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        let mut delivered = 0;
        let result = render_bands_with_interrupt(&doc, doc.bounds(), 1, &Interrupt::cancel_only(&cancel), |_| {
            delivered += 1;
            Ok(())
        });
        assert!(matches!(result, Err(RasterInputError::Cancelled)), "{result:?}");
        assert_eq!(delivered, 0);
        assert_eq!(doc, before);
    }
    #[test]
    fn cancellation_is_observed_inside_a_single_ordinary_band() {
        let doc = Document::with_background("own band", Size::new(512, 256), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let before = doc.clone();
        let patterns = pattern::PreparedPatterns::new(&[], pattern::PREPARED_PATTERN_BYTES);
        let cx = Ctx::for_doc(&doc, &patterns);
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        assert!(matches!(render_region(&doc, doc.bounds(), &cx, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled)));
        assert_eq!(doc, before);
    }
    #[test]
    fn checked_microblocks_match_cached_vector_effect_mask_adjustments() {
        use photocraft_doc::{
            Adjustment, Effect, Fill, FxCommon, FxPaint, LayerMask, Path, ShapeLayer, SmartObject, SmartSource, StrokeFx, StrokePosition, Subpath, TextLayer,
            VectorMask,
        };
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = photocraft_color::PixelFormat { mode: ColorMode::Rgb, sample: depth, alpha: true };
            let mut doc = Document::with_background("own mixed band", Size::new(257, 129), ColorMode::Rgb, depth, Color::WHITE);
            doc.patterns.push(photocraft_doc::Pattern {
                id: "own tile".into(),
                name: "own original tile".into(),
                width: 7,
                height: 5,
                surface: Surface::with_default(format, &[0.7, 0.13, 0.29, 0.61]),
            });
            let mut surface = Surface::new(format);
            surface.fill_rect(Rect::new(-4, 3, 278, 90), &[0.2, 0.6, 0.8, 0.7]);
            let mut layer = Layer::new("own masked", LayerContent::Raster(surface.clone()));
            let mut mask = LayerMask::reveal_all();
            std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(0, 0, 75, 129), &[0.25]);
            layer.mask = Some(mask);
            layer.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(3.5, 0.0), (250.0, 0.0), (200.0, 127.0), (3.5, 127.0)])])));
            layer.effects.items.push(Effect::Stroke(StrokeFx {
                common: FxCommon::new(photocraft_color::BlendMode::Normal, 0.8),
                size: 3.0,
                position: StrokePosition::Outside,
                paint: FxPaint::Pattern { name: "own original tile".into(), id: "own tile".into(), scale: 1.0 },
            }));
            let mut group = Layer::group("own group", vec![layer]);
            group.opacity = 0.8;
            doc.layers.push(group);
            doc.layers.push(Layer::new("own cached text", LayerContent::Text(TextLayer { cache: Some(surface.clone()), ..Default::default() })));
            doc.layers.push(Layer::new(
                "own cached smart",
                LayerContent::Smart(SmartObject::new(
                    SmartSource::Embedded { file_name: "own cache marker".into(), bytes: std::sync::Arc::new(Vec::new()) },
                    photocraft_geom::Affine::IDENTITY,
                    Some(surface),
                )),
            ));
            doc.layers.push(Layer::new(
                "own shape",
                LayerContent::Shape(ShapeLayer {
                    path: Path::new(vec![Subpath::polygon(&[(5.5, 11.2), (128.0, 25.7), (80.5, 123.2)])]),
                    fill: Some(Fill::Pattern {
                        name: "own original tile".into(),
                        id: "own tile".into(),
                        scale: 1.0,
                        angle: 13.0,
                        link: false,
                        phase: (0.2, 0.7),
                    }),
                    ..Default::default()
                }),
            ));
            doc.layers.push(Layer::new("own adjustment", LayerContent::Adjustment(Adjustment::Invert)));
            let before = doc.clone();
            let rect = Rect::new(-8, -8, 265, 137);
            let expected = render(&doc, rect).unwrap();
            assert_eq!(render_with_interrupt(&doc, rect, &Interrupt::NONE).unwrap(), expected, "{depth:?}");
            let mut delivered = Vec::new();
            render_bands_with_interrupt(&doc, rect, 256, &Interrupt::NONE, |band| {
                delivered.extend(band.px);
                Ok(())
            })
            .unwrap();
            assert_eq!(delivered, expected.px, "bands {depth:?}");
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn cancelled_band_is_not_delivered_and_invalid_budget_does_not_allocate() {
        let doc = Document::with_background("own private band", Size::new(512, 256), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let before = doc.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 100;
        let mut delivered = 0;
        assert!(matches!(
            render_bands_with_interrupt(&doc, doc.bounds(), 0, &Interrupt::cancel_only(&cancel), |_| {
                delivered += 1;
                Ok(())
            }),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(delivered, 0);
        assert_eq!(doc, before);
        assert!(matches!(render_with_interrupt(&doc, Rect::new(0, 0, 262144, 4096), &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert!(matches!(render_with_interrupt(&doc, Rect::new(i32::MIN, 0, i32::MAX, 1), &Interrupt::NONE), Err(RasterInputError::InvalidBounds)));
        assert!(matches!(render_bands_with_interrupt(&doc, Rect::new(1, 0, 0, 1), 0, &Interrupt::NONE, |_| Ok(())), Err(RasterInputError::InvalidBounds)));
    }
    #[test]
    fn checked_renderer_rejects_combined_pattern_preparation_before_delivery() {
        let mut doc = Document::new("own bounded patterns", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
        for id in ["own one", "own two"] {
            doc.patterns.push(photocraft_doc::Pattern {
                id: id.into(),
                name: id.into(),
                width: 2048,
                height: 1024,
                surface: Surface::with_default(photocraft_color::PixelFormat::RGBA16, &[0.2, 0.6, 0.8, 0.5]),
            });
        }
        let before = doc.clone();
        let mut delivered = 0;
        assert!(matches!(
            render_bands_with_interrupt(&doc, doc.bounds(), 1, &Interrupt::NONE, |_| {
                delivered += 1;
                Ok(())
            }),
            Err(RasterInputError::LimitExceeded)
        ));
        assert_eq!(delivered, 0);
        assert_eq!(doc, before);
    }
    #[test]
    fn actual_checked_shadow_preview_matches_baseline_and_cancels_atomically() {
        use photocraft_doc::{Contour, Effect, FxCommon, Shadow};
        let mut doc = Document::new("own shadow preparation", Size::new(512, 256), ColorMode::Rgb, SampleType::U16);
        let mut surface = Surface::new(photocraft_color::PixelFormat::RGBA16);
        surface.fill_rect(Rect::new(29, 31, 399, 193), &[0.29, 0.71, 0.43, 0.63]);
        let mut layer = Layer::new("own checked shadow", LayerContent::Raster(surface));
        let shadow = Shadow {
            common: FxCommon::new(photocraft_color::BlendMode::Multiply, 0.61),
            color: Color::BLACK,
            angle: 37.0,
            use_global_light: false,
            distance: 3.4,
            spread: 0.0,
            size: 17.25,
            contour: Contour::Linear,
            anti_alias: true,
            noise: 0.0,
            knocks_out: true,
        };
        layer.effects.items = vec![Effect::DropShadow(shadow.clone()), Effect::InnerShadow(shadow)];
        doc.layers.push(layer);
        let before = doc.clone();
        let expected = render(&doc, doc.bounds()).unwrap();
        assert_eq!(render_with_interrupt(&doc, doc.bounds(), &Interrupt::NONE).unwrap(), expected);
        assert_eq!(doc, before);
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 1000;
        let mut delivered = 0;
        assert!(matches!(
            render_bands_with_interrupt(&doc, doc.bounds(), 256, &Interrupt::cancel_only(&cancel), |_| {
                delivered += 1;
                Ok(())
            }),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(delivered, 0);
        assert_eq!(doc, before);
    }
}
