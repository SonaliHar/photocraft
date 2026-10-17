//! Separate real-budget compiler for owned physical Vibrance Parts. Never
//! entered through the fixed scalar candidate preparation envelope.
use super::*;

// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare<B: PlanBudget>(
    doc: &Document,
    supplied: Option<&Profile>,
    header: u64,
    extent: Rect,
    format: PixelFormat,
    default: &[u8],
    interrupt: &Interrupt<'_>,
    budget: &mut B,
) -> Result<(StockPointPlan, B::Lease), BackedNormalError> {
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let [layer] = doc.layers.as_slice() else { return Err(scope()) };
    let [stack] = state.filter_stacks.as_slice() else { return Err(scope()) };
    let [node] = stack.nodes.as_slice() else { return Err(scope()) };
    if state.items.len() != 2 || state.filter_masks.len() != 1 {
        return Err(scope());
    }
    let parameters = crate::gimp_vibrance::qualify_context(doc, extent, node).map_err(BackedNormalError::Color)?;
    // O(1) caller prepayment precedes borrowed CMS planning. Requirements do not
    // grant source/profile authority and allocate no prepared Parts.
    let preparation = crate::gimp_vibrance::Preparation::plan(Builtin::Srgb.profile(), interrupt, budget)?;
    let stage_peak = crate::gimp_neighborhood::scalar_preparation_bytes(1).map_err(BackedNormalError::Color)?.checked_mul(2).ok_or_else(length)?;
    let envelope = preparation
        .peak_bytes()?
        .checked_add(crate::gimp_vibrance::OwnedVibrance::holder_peak_bytes()?)
        .and_then(|n| n.checked_add(stage_peak))
        .and_then(|n| n.checked_add(header))
        // Actual current effective profile remains live while Parts builds its
        // own effectiveSource/LabD50 and four independent pipeline owners.
        .and_then(|n| n.checked_add(65536))
        .and_then(|n| n.checked_add(crate::gimp_vibrance::PIXEL_SCRATCH_BYTES))
        .ok_or_else(length)?;
    let lease = budget.reserve(envelope)?;
    let (current_profile, basis) = bound_profile(doc, supplied, budget, interrupt)?;
    // Independently pay metadata and profile-definition comparisons, separate
    // from consumed planner and future constructor work.
    work(budget, interrupt, envelope)?;
    crate::gimp_render::qualify_current_point_layer_coverage_metadata(
        doc,
        layer,
        state,
        stack,
        extent,
        doc.sample_encoding.ok_or_else(scope)?,
        format,
        default,
    )
    .map_err(BackedNormalError::Color)?;
    let item = state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
    work(budget, interrupt, (layer.name.len() as u64).checked_add(item.name.len() as u64).ok_or_else(length)?)?;
    let resolved = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
    let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
    if item.name != layer.name
        || layer.blend != photocraft_color::blend::BlendMode::Normal
        || layer.fill_opacity.to_bits() != 1f32.to_bits()
        || layer.clipped
        || layer.excluded_channels != 0
        || layer.blend_if != Default::default()
        || layer.advanced != Default::default()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || !layer.psd_blocks.is_empty()
        || resolved.operator != GimpOperator::Union
        || resolved.blend_space != GimpSpace::LinearRgb
        || resolved.composite_space != GimpSpace::LinearRgb
        || physical.pass_through
        || physical.blend_encoding != SampleEncoding::Linear
        || physical.composite_encoding != SampleEncoding::Linear
    {
        return Err(scope());
    }
    let mask = state.filter_masks.first().ok_or_else(scope)?;
    let pixels = (mask.surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_add(1)).ok_or_else(length)?;
    work(budget, interrupt, pixels.checked_mul(32).ok_or_else(length)?)?;
    crate::gimp_vibrance::qualify_mask(state, node, layer.id, doc.depth, interrupt).map_err(BackedNormalError::Color)?;
    // Every fallible construction after reservation destroys later payloads
    // before this fee. Success transfers payload into StockPointPlan, whose
    // enclosing native/source holder also declares its fee after the plan.
    let candidate = preparation.construct(basis, parameters, interrupt, budget)?;
    let point = crate::gimp_filters::PointStack::from_prepared_vibrance(candidate, node.enabled, doc.depth, interrupt).map_err(BackedNormalError::Color)?;
    let ordered = OrderedStack::from_prepared_stock_point(point, extent, interrupt)
        .map_err(BackedNormalError::Color)?
        .with_current_point_storage(Some(doc.depth))
        .with_filter_output_precision(state.current_filter_output_precision);
    // The cached nested Parts census was actually traversed and charged after
    // construction; no refund is inferred from the retained number.
    let retained =
        ordered.stock_retained_bytes().map_err(BackedNormalError::Color)?.checked_add(header).and_then(|n| n.checked_add(65536)).ok_or_else(length)?;
    if retained > envelope {
        return Err(BackedNormalError::Allocation);
    }
    work(budget, interrupt, 0)?;
    Ok((
        StockPointPlan {
            recipe: Recipe::Filtered,
            stack: StockPointProgram::Ordered(ordered),
            _current_profile: current_profile,
            format,
            opaque_input: false,
            half_input: false,
            unit_input: true,
        },
        lease,
    ))
}
