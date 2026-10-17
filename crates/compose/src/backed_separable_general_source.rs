//! Preparation for an independently admitted general original flat source.
//! Native source qualification belongs to the bounded I/O visitor; this stage
//! retains only paid recipes and the requested analytical working spaces.
use super::*;
use photocraft_doc::import_compositing::{XCF_FLAT_MODERN_IMPLICIT_SOURCE_EPOCH, flat_modern_source_rule};

pub(super) struct Recipes<L> {
    values: Vec<Recipe>,
    _fee: L,
}

type PreparedSource<L> = (Recipes<L>, Spaces<L>);

pub(super) fn prepare<B: crate::gimp_render::PreparationBudget>(
    doc: &Document,
    interrupt: &Interrupt<'_>,
    budget: &mut B,
    header: usize,
) -> Result<PreparedSource<B::Lease>, E> {
    let check = |budget: &mut B, n| {
        if interrupt.cancelled() {
            return Err(E::Backing(BackingError::Cancelled));
        }
        budget.consume_preparation_work(n).map_err(E::Color)
    };
    check(budget, 1)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    let n = doc.layers.len();
    if n == 0
        || n > 4096
        || state.items.len() != n
        || doc.bounds().is_empty()
        || doc.size.width > 262144
        || doc.size.height > 262144
        || u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(scope)? > 1 << 30
        || doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::F32
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. })
        || source.source_precision != 650
        || source.original_icc.is_some()
        || !matches!(state.provenance.format_version, 12 | 23)
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
    {
        return Err(scope());
    }
    check(budget, 1)?;
    let profile = Builtin::Srgb.profile();
    let current = doc.icc_profile.as_deref().map(Vec::as_slice).ok_or_else(scope)?;
    if !equal(current, profile.encoded_bytes().ok_or_else(scope)?, &mut |n| check(budget, n))? {
        return Err(scope());
    }
    check(budget, current.len() as u64 + 64)?;
    doc.working_profile_interpretation
        .validate_with_interrupt(Some(current), doc.mode, interrupt)
        .map_err(|_| if interrupt.cancelled() { E::Backing(BackingError::Cancelled) } else { scope() })?;
    for opaque in &state.opaque {
        check(budget, 1)?;
        if opaque.rendering_required {
            return Err(scope());
        }
    }
    let ceiling = n.checked_mul(2).ok_or_else(scope)?;
    let bytes = ceiling.checked_mul(std::mem::size_of::<Recipe>()).and_then(|v| v.checked_add(std::mem::size_of::<Recipes<B::Lease>>())).ok_or_else(scope)?;
    let fee = budget.reserve_prepared_bytes(bytes as u64).map_err(E::Color)?;
    // Install payload-before-fee custody before any fallible allocation.
    let mut recipes = Recipes { values: Vec::new(), _fee: fee };
    check(budget, bytes as u64)?;
    recipes.values.try_reserve_exact(n).map_err(|_| E::Allocation)?;
    if recipes.values.capacity() > ceiling {
        return Err(E::Allocation);
    }
    let mut needed = [false; 3];
    for (index, layer) in doc.layers.iter().enumerate() {
        check(budget, 1)?;
        for previous in doc.layers.iter().take(index) {
            check(budget, 1)?;
            if previous.id == layer.id {
                return Err(scope());
            }
        }
        let mut item = None;
        for row in &state.items {
            check(budget, 1)?;
            if row.item == ItemRef::Layer(layer.id) {
                if item.is_some() {
                    return Err(scope());
                }
                item = Some(row);
            }
        }
        let item = item.ok_or_else(scope)?;
        let LayerContent::Raster(surface) = &layer.content else {
            return Err(scope());
        };
        let extent = item.raster_bounds.ok_or_else(scope)?;
        let width = i64::from(extent.x1) - i64::from(extent.x0);
        let height = i64::from(extent.y1) - i64::from(extent.y0);
        if width <= 0
            || height <= 0
            || width > 262144
            || height > 262144
            || item.parent.is_some()
            || item.mask.is_some()
            || item.visible != layer.visible
            || surface.format() != doc.pixel_format()
            || !surface.format().alpha
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.clipped
            || layer.link_group.is_some()
            || !layer.effects.items.is_empty()
            || layer.blend_if != photocraft_doc::BlendIf::default()
            || layer.excluded_channels != 0
            || layer.advanced != photocraft_doc::AdvancedBlending::default()
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
        {
            return Err(scope());
        }
        if item.name.len() != layer.name.len() || !equal(item.name.as_bytes(), layer.name.as_bytes(), &mut |n| check(budget, n))? {
            return Err(scope());
        }
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        if policy.compatibility_epoch.len() > 128 {
            return Err(scope());
        }
        check(budget, policy.compatibility_epoch.len() as u64 + 1)?;
        if policy.compatibility_epoch != XCF_FLAT_MODERN_IMPLICIT_SOURCE_EPOCH || policy.legacy || policy.pass_through {
            return Err(scope());
        }
        let rule = flat_modern_source_rule(
            state.provenance.format_version,
            policy.source_mode,
            policy.stored_blend_space,
            policy.stored_composite_space,
            policy.stored_operator,
        )
        .ok_or_else(scope)?;
        if policy.checked_resolved().map_err(|_| scope())? != &rule.resolved() {
            return Err(scope());
        }
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if physical.pass_through || physical.blend_encoding != transfer(rule.blend_space)? || physical.composite_encoding != transfer(rule.composite_space)? {
            return Err(scope());
        }
        *needed.get_mut(lower::slot(rule.blend_space)?).ok_or_else(scope)? = true;
        *needed.get_mut(lower::slot(rule.composite_space)?).ok_or_else(scope)? = true;
        recipes.values.push(Recipe { mode: policy.source_mode, blend: rule.blend_space, composite: rule.composite_space, operator: rule.operator });
    }
    let (bytes, work) = lower::space_requirements(profile, SampleEncoding::NonlinearProfile, needed, header)?;
    let fee = budget.reserve_prepared_bytes(bytes).map_err(E::Color)?;
    check(budget, work)?;
    let spaces = Spaces::prepare(profile, SampleEncoding::NonlinearProfile, needed, fee, interrupt)?;
    check(budget, 0)?;
    Ok((recipes, spaces))
}
