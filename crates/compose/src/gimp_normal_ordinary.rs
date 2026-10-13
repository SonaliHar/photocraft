//! Job-accounted current-state validation for independently observed ordinary Normal scenes.
use super::PreparationBudget;
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{
    ColorMode, Document, LayerContent, PixelFormat, SampleType, WorkingProfileInterpretation,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::Rect;
pub(crate) const EPOCH: &str = "xcf12-analytic-ordinary-normal-mask-v1";
pub(crate) fn applies(doc: &Document) -> bool {
    doc.xcf_state
        .as_ref()
        .is_some_and(|state| state.items.iter().any(|item| item.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == EPOCH)))
}
use photocraft_raster::{Interrupt, RasterInputError, Surface};
fn scope() -> CompositionError {
    CompositionError::Unsupported("job-accounted analytical Normal mask geometry")
}
fn pixels(rect: Rect) -> Result<u64> {
    let width = u64::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| scope())?;
    let height = u64::try_from(i64::from(rect.y1) - i64::from(rect.y0)).map_err(|_| scope())?;
    let count = width.checked_mul(height).ok_or_else(scope)?;
    if width == 0 || height == 0 || width > 262144 || height > 262144 {
        return Err(scope());
    }
    Ok(count)
}
fn unit_plane(surface: &Surface, rect: Rect, format: PixelFormat, interrupt: &Interrupt, budget: &mut impl PreparationBudget) -> Result<()> {
    if surface.format() != format {
        return Err(scope());
    }
    let count = pixels(rect)?;
    let mut visits = 0_u64;
    let mut pixel = [0.0; 4];
    let output = pixel.get_mut(..format.channels()).ok_or_else(scope)?;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            if visits.is_multiple_of(64) {
                interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
                let pixels = (count - visits).min(64);
                let units = pixels.checked_mul(format.channels() as u64).ok_or_else(scope)?;
                budget.consume_preparation_work(units)?;
            }
            visits += 1;
            surface.try_read_pixel_f64(x, y, output)?;
            if output.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(scope());
            }
        }
    }
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(())
}
/// The caller reserves preparation memory first and retains source accounting.
/// Every <=64-pixel scan chunk consumes job work before reading samples.
pub(crate) fn validate(doc: &Document, interrupt: &Interrupt, budget: &mut impl PreparationBudget) -> Result<()> {
    let Some(state) = &doc.xcf_state else { return Ok(()) };
    if !applies(doc) {
        return Err(scope());
    }
    let epoch = EPOCH;
    if doc.sample_encoding != Some(photocraft_doc::SampleEncoding::NonlinearProfile)
        || doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
        || !doc.icc_profile.as_ref().is_some_and(|bytes| (132..=65536).contains(&bytes.len()))
    {
        return Err(scope());
    }
    // The shared checked constructor validates this bounded CURRENT ICC's model
    // and operative analytical curves; source provenance is never consulted here.
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    let canvas_pixels = u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(scope)?;
    if doc.size.width == 0
        || doc.size.height == 0
        || doc.size.width > 262144
        || doc.size.height > 262144
        || canvas_pixels > (1_u64 << 30)
        || doc.layers.len() != 2
        || state.items.len() != 3
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(doc.depth, SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || state.floating.is_some()
    {
        return Err(scope());
    }
    budget.consume_preparation_work(32)?;
    let canvas = doc.bounds();
    let mut aggregate = 0_u64;
    for (index, layer) in doc.layers.iter().enumerate() {
        if !layer.visible
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity != 1.0
            || layer.clipped
            || layer.vector_mask.is_some()
            || crate::effects::has_effects(layer)
        {
            return Err(scope());
        }
        let LayerContent::Raster(surface) = &layer.content else { return Err(scope()) };
        let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
        let extent = item.raster_bounds.ok_or_else(scope)?;
        aggregate = aggregate.checked_add(pixels(extent)?).ok_or_else(scope)?;
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        let resolved = policy.resolved.as_ref().ok_or_else(scope)?;
        if !item.visible
            || item.parent.is_some()
            || policy.compatibility_epoch != epoch
            || policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || resolved.composite_space != GimpSpace::LinearRgb
            || resolved.blend_space != GimpSpace::LinearRgb
        {
            return Err(scope());
        }
        if index == 0 {
            if extent != canvas || layer.opacity != 1.0 || layer.mask.is_some() || resolved.operator != GimpOperator::Union {
                return Err(scope());
            }
        } else {
            let mask = layer.mask.as_ref().ok_or_else(scope)?;
            let metadata = state.items.iter().find_map(|item| item.mask.as_ref().filter(|m| m.owner == ItemRef::Layer(layer.id))).ok_or_else(scope)?;
            if !mask.enabled
                || mask.density != 1.0
                || mask.feather != 0.0
                || metadata.bounds != extent
                || metadata.offset != [extent.x0, extent.y0]
                || metadata.outside_f64_bits != 0.0_f64.to_bits()
                || !metadata.enabled
                || metadata.shown
                || metadata.editing
            {
                return Err(scope());
            }
            aggregate = aggregate.checked_add(pixels(metadata.bounds)?).ok_or_else(scope)?;
            unit_plane(&mask.surface, metadata.bounds, PixelFormat::new(ColorMode::Grayscale, doc.depth, false), interrupt, budget)?;
        }
        unit_plane(surface, extent, doc.pixel_format(), interrupt, budget)?;
    }
    Ok(())
}
