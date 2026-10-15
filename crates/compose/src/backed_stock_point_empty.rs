//! Explicit CURRENT empty Point recipe. No source identity or operation epoch
//! is synthesized by the absence of nodes.
use super::*;

pub(super) fn prepare<B: PlanBudget>(
    doc: &Document,
    supplied: Option<&Profile>,
    header: u64,
    interrupt: &Interrupt<'_>,
    budget: &mut B,
) -> Result<(StockPointPlan, B::Lease), BackedNormalError> {
    work(budget, interrupt, 4096)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let [layer] = doc.layers.as_slice() else { return Err(scope()) };
    let [filters] = state.filter_stacks.as_slice() else { return Err(scope()) };
    if !filters.nodes.is_empty()
        || filters.owner != ItemRef::Layer(layer.id)
        || !state.filter_masks.is_empty()
        || state.items.len() != 1
        || state.provenance.format_version != 22
        || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
    {
        return Err(scope());
    }
    let stored = doc.sample_encoding.ok_or_else(scope)?;
    let (extent, format, default) = match &layer.content {
        LayerContent::BackedRaster(backed) => (backed.operative_bounds(), backed.format(), backed.default_bytes()),
        LayerContent::Raster(surface) => {
            (state.items.first().and_then(|item| item.raster_bounds).ok_or_else(scope)?, surface.format(), surface.default_bytes())
        }
        _ => return Err(scope()),
    };
    // Shared fixed owner geometry/precision checks, entered explicitly as Empty;
    // no all(nodes) expression is used as proof of a witnessed operation epoch.
    // A removed operation carries no recoverable operation identity. This is
    // a separate narrow generic derived CURRENT empty role, never an original
    // XCF source selector or an inferred Semi-Flatten provenance grant.
    let float_current =
        matches!((doc.depth, doc.source_color_context.as_ref().map(|s| s.source_precision)), (SampleType::F32, Some(650)) | (SampleType::F64, Some(750)));
    if float_current {
        crate::gimp_semi_flatten::positive_origin(extent).map_err(BackedNormalError::Color)?;
        if doc.size != extent.size()
            || extent != doc.bounds()
            || doc.mode != ColorMode::Rgb
            || stored != SampleEncoding::NonlinearProfile
            || format != doc.pixel_format()
            || !format.alpha
            || default.iter().any(|v| *v != 0)
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.mask.is_some()
            || doc.source_color_context.as_ref().is_none_or(|s| s.original_icc.is_some())
            || state.items.first().is_none_or(|item| {
                item.item != ItemRef::Layer(layer.id)
                    || item.visible != layer.visible
                    || item.raster_bounds != Some(extent)
                    || item.composition.as_ref().is_none_or(|p| {
                        p.source_mode != 28 || p.legacy || p.pass_through || (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (0, -1, -1)
                    })
            })
        {
            return Err(scope());
        }
    } else {
        qualify_witnessed_metadata(doc, layer, state, filters, extent, stored, format, default)?;
    }
    let item = state.items.first().ok_or_else(scope)?;
    let resolved = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
    let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
    if item.item != ItemRef::Layer(layer.id)
        || resolved.operator != GimpOperator::Union
        || resolved.blend_space != if float_current { GimpSpace::LinearRgb } else { GimpSpace::ProfileRgb }
        || resolved.composite_space != if float_current { GimpSpace::LinearRgb } else { GimpSpace::ProfileRgb }
        || physical.pass_through
        || physical.blend_encoding != if float_current { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile }
        || physical.composite_encoding != if float_current { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile }
        || layer.blend != photocraft_color::blend::BlendMode::Normal
        || layer.fill_opacity.to_bits() != 1f32.to_bits()
        || layer.clipped
        || layer.excluded_channels != 0
        || layer.blend_if != Default::default()
        || layer.advanced != Default::default()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || !layer.psd_blocks.is_empty()
    {
        return Err(scope());
    }
    // Keep the actual current effective profile owned and charged exactly as
    // other recipes. Empty owns no Stage/WorkingSpace heap or CMS transform.
    let envelope = header.checked_add(65536).ok_or_else(length)?;
    let lease = budget.reserve(envelope)?;
    let (current_profile, _) = bound_profile(doc, supplied, budget, interrupt)?;
    let stack = OrderedStack::empty_for_stock(extent, interrupt).map_err(BackedNormalError::Color)?;
    let retained = stack.stock_retained_bytes().map_err(BackedNormalError::Color)?.checked_add(header).and_then(|n| n.checked_add(65536)).ok_or_else(length)?;
    if retained > envelope {
        return Err(BackedNormalError::Allocation);
    }
    work(budget, interrupt, 0)?;
    Ok((
        StockPointPlan {
            recipe: Recipe::Empty,
            stack,
            _current_profile: current_profile,
            format,
            opaque_input: false,
            half_input: false,
            unit_input: float_current,
        },
        lease,
    ))
}
