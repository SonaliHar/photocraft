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
    qualify_witnessed_metadata(doc, layer, state, filters, extent, stored, format, default)?;
    let item = state.items.first().ok_or_else(scope)?;
    let resolved = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
    let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
    if item.item != ItemRef::Layer(layer.id)
        || resolved.operator != GimpOperator::Union
        || resolved.blend_space != GimpSpace::ProfileRgb
        || resolved.composite_space != GimpSpace::ProfileRgb
        || physical.pass_through
        || physical.blend_encoding != SampleEncoding::NonlinearProfile
        || physical.composite_encoding != SampleEncoding::NonlinearProfile
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
    Ok((StockPointPlan { recipe: Recipe::Empty, stack, _current_profile: current_profile, format, opaque_input: false, half_input: false }, lease))
}
