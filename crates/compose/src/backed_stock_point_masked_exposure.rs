//! Private paid singleton nonempty scalar-mask Exposure backend.
//! The raw Source entry is strict; actual Point CURRENT uses its own full proof.
use super::*;
pub(super) fn prepare<B: PlanBudget>(
    doc: &Document,
    supplied: Option<&Profile>,
    purpose: PreparationPurpose,
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
    // All cardinalities precede any borrowed registry find or sample traversal.
    if state.items.len() != 2
        || state.filter_masks.len() != 1
        || node.id.0 == 0
        || layer.id.0 == 0
        || stack.owner != ItemRef::Layer(layer.id)
        || doc.mode != ColorMode::Rgb
        || !matches!(doc.depth, SampleType::F32 | SampleType::F64)
        || format != doc.pixel_format()
        || !format.alpha
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || extent != doc.bounds()
        || extent.size() != doc.size
        || state.provenance.format != "xcf"
        || state.provenance.format_version != 22
        || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || state.selection.selected.len() > 1
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
        || state.selection.selected.iter().any(|v| *v != ItemRef::Layer(layer.id))
        || state.selection.active.is_some_and(|v| v != ItemRef::Layer(layer.id) || !state.selection.selected.contains(&v))
        || doc
            .source_color_context
            .as_ref()
            .is_none_or(|s| s.original_icc.is_some() || !matches!((doc.depth, s.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750)))
        || (purpose == PreparationPurpose::Source && (!layer.visible || layer.opacity.to_bits() != 1f32.to_bits() || !node.enabled))
    {
        return Err(scope());
    }
    crate::gimp_semi_flatten::positive_origin(extent).map_err(BackedNormalError::Color)?;
    let item = state.items.iter().find(|r| r.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
    if item.raster_bounds != Some(extent)
        || item.parent.is_some()
        || item.mask.is_some()
        || item.source_flags != 0
        || layer.name.len() > 1 << 20
        || item.name.len() > 1 << 20
        || item.visible != layer.visible
        || layer.blend != photocraft_color::blend::BlendMode::Normal
        || layer.link_group.is_some()
        || layer.vector_mask.is_some()
        || layer.indexed_source.is_some()
        || layer.fill_cache.is_some()
        || layer.video.is_some()
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
    work(budget, interrupt, (layer.name.len() as u64).checked_add(item.name.len() as u64).ok_or_else(length)?)?;
    if layer.name != item.name {
        return Err(scope());
    }
    crate::gimp_render::qualify_current_point_layer_coverage_metadata(doc, layer, state, stack, extent, SampleEncoding::NonlinearProfile, format, default)
        .map_err(BackedNormalError::Color)?;
    let p = item.composition.as_ref().ok_or_else(scope)?;
    let resolved = p.checked_resolved().map_err(|_| scope())?;
    let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
    if p.compatibility_epoch != "xcf-22-normal"
        || p.source_mode != 28
        || p.legacy
        || p.pass_through
        || (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (0, -1, -1)
        || resolved.blend_space != GimpSpace::LinearRgb
        || resolved.composite_space != GimpSpace::LinearRgb
        || resolved.operator != GimpOperator::Union
        || physical.pass_through
        || physical.blend_encoding != SampleEncoding::Linear
        || physical.composite_encoding != SampleEncoding::Linear
    {
        return Err(scope());
    }
    work(budget, interrupt, 512)?;
    let parameters = crate::gimp_masked_exposure::canonical(node, extent).map_err(BackedNormalError::Color)?;

    let [mask] = state.filter_masks.as_slice() else { return Err(scope()) };
    let tile_bytes = 65536u64.checked_mul(doc.depth.bytes() as u64).ok_or_else(length)?;
    let mask_limit = 16u64 << 20;
    let prospective =
        (mask.surface.tile_count() as u64).checked_mul(tile_bytes).and_then(|n| n.checked_add(mask.surface.default_bytes().len() as u64)).ok_or_else(length)?;
    if mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, doc.depth, false) || mask.surface.tile_count() > 4096 || prospective > mask_limit {
        return Err(scope());
    }
    // The O(1) fixed-payload guard bounds this actual census to at most63 tiles
    // before any descriptor iteration, full mask scan or expensive scan debit.
    work(budget, interrupt, (mask.surface.tile_count() as u64).checked_add(1).ok_or_else(length)?)?;
    if mask.surface.resident_payload_bytes().map_err(|_| length())? > mask_limit {
        return Err(scope());
    }
    work(budget, interrupt, 0)?;
    let mask_ref = node.mask.ok_or_else(scope)?;
    let row = state.items.iter().find(|r| r.item == mask_ref).ok_or_else(scope)?;
    if row.parent.is_some() || row.composition.is_some() || row.raster_bounds != Some(extent) || row.source_flags != 0 {
        return Err(scope());
    }
    let mask_ref = node.mask.ok_or_else(scope)?;
    let ItemRef::Mask(mask_id) = mask_ref else { return Err(scope()) };
    let meta = row.mask.as_ref().ok_or_else(scope)?;
    if mask.id != mask_id
        || mask.owner != node.id
        || mask.bounds != extent
        || !row.visible
        || meta.owner != ItemRef::Layer(layer.id)
        || meta.bounds != extent
        || meta.offset != [0, 0]
        || meta.outside_f64_bits != 0
        || !meta.enabled
        || meta.shown
        || meta.editing
        || meta.linked
    {
        return Err(scope());
    }
    let pixels = (mask.surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_add(1)).ok_or_else(length)?;
    work(budget, interrupt, pixels.checked_mul(96).ok_or_else(length)?)?;
    if mask.surface.default_bytes().len() != doc.depth.bytes() || mask.surface.default_bytes().iter().any(|v| *v != 0) {
        return Err(scope());
    }
    let mut nonempty = false;
    for (coordinate, tile) in mask.surface.tiles() {
        crate::gimp_semi_flatten::mask_tile(extent, *coordinate).map_err(BackedNormalError::Color)?;
        if tile.bytes().len() as u64 != tile_bytes {
            return Err(length());
        }
        let tx = coordinate.tx.checked_mul(TILE_SIZE).ok_or_else(length)?;
        let ty = coordinate.ty.checked_mul(TILE_SIZE).ok_or_else(length)?;
        if tx < 0 || ty < 0 {
            return Err(scope());
        }
        for (block_index, block) in tile.bytes().chunks(doc.depth.bytes() * 64).enumerate() {
            work(budget, interrupt, 0)?;
            for (offset, raw) in block.chunks_exact(doc.depth.bytes()).enumerate() {
                let at = block_index.checked_mul(64).and_then(|n| n.checked_add(offset)).ok_or_else(length)?;
                let x = tx.checked_add(i32::try_from(at % 256).map_err(|_| length())?).ok_or_else(length)?;
                let y = ty.checked_add(i32::try_from(at / 256).map_err(|_| length())?).ok_or_else(length)?;
                if x >= extent.x1 || y >= extent.y1 {
                    if raw.iter().any(|v| *v != 0) {
                        return Err(scope());
                    }
                } else {
                    let value = try_read_sample_f64(raw, doc.depth, 0).map_err(|_| length())?;
                    crate::gimp_masked_exposure::coverage(value)?;
                    nonempty |= value != 0.0;
                }
            }
        }
    }
    // Empty masks belong to the literal old sentinel route, never this epoch.
    if !nonempty {
        return Err(scope());
    }
    // Public standalone pipelines can outlive the borrowed Document. Retain a
    // conservative independent full payload admission in addition to cloned
    // sparse-map/default metadata; the actual payload Arcs remain shared.
    let mask_retained = prospective
        .checked_add((mask.surface.tile_count() as u64).checked_mul(512).ok_or_else(length)?)
        .and_then(|n| n.checked_add(2 * doc.depth.bytes() as u64 + std::mem::size_of::<photocraft_raster::Surface>() as u64 + 128))
        .ok_or_else(length)?;

    // One retained fee covers the actual monomorphized outer holder, program
    // header and current-profile peak simultaneously with the full candidate.
    let outer = header
        .checked_add(std::mem::size_of::<StockPointPlan>() as u64)
        .and_then(|n| n.checked_add(65536))
        .and_then(|n| n.checked_add(mask_retained))
        .ok_or_else(length)?;
    let lease;
    let candidate;
    (candidate, lease) = crate::gimp_masked_exposure::PreparedMaskedExposure::prepare(parameters, outer, interrupt, budget)?;
    work(budget, interrupt, mask_retained)?;
    let mut retained_mask =
        photocraft_raster::Surface::try_with_default_bytes(mask.surface.format(), mask.surface.default_bytes()).map_err(|_| BackedNormalError::Allocation)?;
    for (coordinate, tile) in mask.surface.tiles() {
        work(budget, interrupt, 1)?;
        retained_mask.put_tiles(std::iter::once((*coordinate, std::sync::Arc::clone(tile))));
    }
    let (current_profile, _basis) = bound_profile(doc, supplied, budget, interrupt)?;
    // Raw source preparation proves all retained resident samples, including
    // padding. Backed CURRENT operative samples are proved by its full walk.
    if let LayerContent::Raster(surface) = &layer.content {
        let pixels = (surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_add(1)).ok_or_else(length)?;
        work(budget, interrupt, pixels.checked_mul(64).ok_or_else(length)?)?;
        for (coordinate, tile) in surface.tiles() {
            crate::gimp_semi_flatten::mask_tile(extent, *coordinate).map_err(BackedNormalError::Color)?;
            if tile.bytes().len() != 65536usize.checked_mul(format.bytes_per_pixel()).ok_or_else(length)? {
                return Err(length());
            }
            let tx = coordinate.tx.checked_mul(TILE_SIZE).ok_or_else(length)?;
            let ty = coordinate.ty.checked_mul(TILE_SIZE).ok_or_else(length)?;
            for (block_index, block) in tile.bytes().chunks(format.bytes_per_pixel() * 64).enumerate() {
                work(budget, interrupt, 0)?;
                for (offset, pixel) in block.chunks_exact(format.bytes_per_pixel()).enumerate() {
                    let at = block_index.checked_mul(64).and_then(|n| n.checked_add(offset)).ok_or_else(length)?;
                    let x = tx.checked_add(i32::try_from(at % 256).map_err(|_| length())?).ok_or_else(length)?;
                    let y = ty.checked_add(i32::try_from(at / 256).map_err(|_| length())?).ok_or_else(length)?;
                    if purpose == PreparationPurpose::Source
                        && (x < extent.x0 || y < extent.y0 || x >= extent.x1 || y >= extent.y1)
                        && pixel.iter().any(|v| *v != 0)
                    {
                        return Err(scope());
                    }
                    for channel in 0..4 {
                        let v = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| length())?;
                        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                            return Err(scope());
                        }
                    }
                }
            }
        }
    }
    let plan = StockPointPlan {
        recipe: Recipe::Filtered,
        stack: StockPointProgram::MaskedExposure { candidate, mask: retained_mask, extent, enabled: node.enabled },
        _current_profile: current_profile,
        format,
        opaque_input: false,
        half_input: false,
        unit_input: true,
    };
    work(budget, interrupt, 0)?;
    Ok((plan, lease))
}
