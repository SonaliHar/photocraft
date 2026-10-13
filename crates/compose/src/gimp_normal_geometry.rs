//! Bounded current-state contract for independently observed Normal mask geometry.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{
    ColorMode, Document, LayerContent, PixelFormat, SampleType, WorkingProfileInterpretation,
    import_compositing::{
        GimpOperator, GimpSpace, XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH, analytic_normal_encoding,
    },
    item_identity::ItemRef,
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
fn scope() -> CompositionError {
    CompositionError::Unsupported("bounded analytical Normal mask geometry")
}
fn pixels(rect: Rect) -> Result<u64> {
    let width = u64::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| scope())?;
    let height = u64::try_from(i64::from(rect.y1) - i64::from(rect.y0)).map_err(|_| scope())?;
    let count = width.checked_mul(height).ok_or_else(scope)?;
    if width == 0 || height == 0 || count > 4096 {
        return Err(scope());
    }
    Ok(count)
}
fn unit_plane(surface: &Surface, rect: Rect, format: PixelFormat, interrupt: &Interrupt) -> Result<()> {
    if surface.format() != format {
        return Err(scope());
    }
    let mut visits = 0_u64;
    let mut pixel = [0.0; 4];
    let output = pixel.get_mut(..format.channels()).ok_or_else(scope)?;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            if visits.is_multiple_of(64) {
                interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
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
/// Called only for the persistent geometry epoch; other imported domains are unchanged.
/// Validation scans at most 8192 declared pixels and performs no allocation or mutation.
pub(crate) fn validate(doc: &Document, interrupt: &Interrupt) -> Result<()> {
    let Some(state) = &doc.xcf_state else { return Ok(()) };
    let Some(epoch) = state.items.iter().find_map(|item| {
        let epoch = item.composition.as_ref()?.compatibility_epoch.as_str();
        matches!(
            epoch,
            XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
                | XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH
                | XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH
                | XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
        )
        .then_some(epoch)
    }) else {
        return Ok(());
    };
    if doc.sample_encoding != analytic_normal_encoding(epoch)
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
        || canvas_pixels > 4096
        || doc.layers.len() != 2
        || state.items.len() != 3
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !if epoch == XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH {
            matches!(doc.depth, SampleType::U16 | SampleType::U32)
        } else {
            matches!(doc.depth, SampleType::F32 | SampleType::F64)
        }
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.source_items.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || state.floating.is_some()
    {
        return Err(scope());
    }
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
            if aggregate > 8192 {
                return Err(scope());
            }
            unit_plane(&mask.surface, metadata.bounds, PixelFormat::new(ColorMode::Grayscale, doc.depth, false), interrupt)?;
        }
        if aggregate > 8192 {
            return Err(scope());
        }
        unit_plane(surface, extent, doc.pixel_format(), interrupt)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn bounded_plane_polls_every_sixty_four_pixels_without_mutation() {
        let surface = Surface::new(PixelFormat::RGBA64F);
        let before = surface.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 2;
        let result = unit_plane(&surface, Rect::new(0, 0, 17, 9), surface.format(), &Interrupt::cancel_only(&cancel));
        assert!(matches!(result, Err(CompositionError::Raster(RasterInputError::Cancelled))));
        assert_eq!(polls.load(Ordering::Relaxed), 3);
        assert_eq!(surface, before);
        unit_plane(&surface, Rect::new(0, 0, 17, 9), surface.format(), &Interrupt::NONE).unwrap();
    }
    #[test]
    fn geometry_plane_rejects_nonunit_and_nonfinite_samples() {
        for value in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            let mut surface = Surface::new(PixelFormat::RGBA64F);
            surface.try_write_pixel_f64(1, 1, &[value, 0.5, 0.5, 1.0]).unwrap();
            let original = surface.try_read_interleaved(Rect::new(0, 0, 3, 3)).unwrap();
            assert!(unit_plane(&surface, Rect::new(0, 0, 3, 3), surface.format(), &Interrupt::NONE).is_err());
            assert_eq!(surface.try_read_interleaved(Rect::new(0, 0, 3, 3)).unwrap(), original);
        }
    }
}
