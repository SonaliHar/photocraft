//! Generic allocation accounting for the first already-supported flat current class.
use super::{PreparationBudget, Prepared, PreparedRenderer};
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{ColorMode, Document, LayerContent, WorkingProfileInterpretation};
use photocraft_raster::{Interrupt, RasterInputError, Surface};

fn unsupported() -> CompositionError {
    CompositionError::Unsupported("accounted preparation requires flat implicit analytic scalar plans")
}
fn poll(interrupt: &Interrupt<'_>) -> Result<()> {
    interrupt.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn scalar(name: &str) -> bool {
    matches!(
        name,
        "gegl:exposure"
            | "gegl:invert-linear"
            | "gegl:invert-gamma"
            | "gegl:brightness-contrast"
            | "gegl:levels"
            | "gegl:color-overlay"
            | "gegl:color-rotate"
            | "gegl:mono-mixer"
            | "gegl:rgb-clip"
            | "gegl:color-temperature"
            | "gegl:saturation"
            | "gegl:sepia"
            | "gegl:alien-map"
            | "gimp:desaturate"
            | "gimp:threshold"
            | "gimp:threshold-alpha"
            | "gimp:posterize"
    )
}
/// Allocation-class membership only. Invalid ICC bytes, scalar values, counts,
/// masks and retained samples must reach the fallible builder, never fallback.
pub(super) fn supports(doc: &Document) -> bool {
    if doc.layers.is_empty()
        || !matches!(
            (&doc.working_profile_interpretation, doc.mode),
            (WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb) | (WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale)
        )
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.quick_mask.is_some()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.layers.iter().any(|layer| {
            !matches!(layer.content, LayerContent::Raster(_))
                || layer.blend != photocraft_color::blend::BlendMode::Normal
                || layer.vector_mask.is_some()
                || crate::effects::has_effects(layer)
                || layer.video.is_some()
                || layer.fill_cache.is_some()
                || layer.indexed_source.is_some()
        })
    {
        return false;
    }
    let Some(state) = &doc.xcf_state else {
        return true;
    };
    state.source_items.is_empty()
        && state.floating.is_none()
        && state.editable_indexed.is_empty()
        && state.resources.is_empty()
        && state.item_sets.is_empty()
        && !state.items.iter().any(|item| {
            item.composition.as_ref().is_some_and(|p| {
                p.compatibility_epoch == photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH
                    || p.source_mode != 28
                    || p.legacy
                    || p.pass_through
                    || p.resolved.as_ref().is_some_and(|r| {
                        r.composite_space == photocraft_doc::import_compositing::GimpSpace::Lab
                            || r.blend_space == photocraft_doc::import_compositing::GimpSpace::Lab
                    })
            })
        })
        && state.filter_stacks.iter().all(|stack| stack.nodes.iter().all(|node| scalar(&node.operation.name) && node.auxiliary.is_empty()))
}
/// Checked, nonallocating owner census. This describes allocation complexity;
/// authoritative operation/policy validation still runs in the existing builder.
pub(super) fn bytes(doc: &Document, interrupt: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<u64> {
    poll(interrupt)?;
    if !supports(doc) {
        return Err(unsupported());
    }
    if doc.layers.is_empty()
        || doc.layers.len() > 10000
        || !matches!(
            (&doc.working_profile_interpretation, doc.mode),
            (WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb) | (WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale)
        )
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
    {
        return Err(unsupported());
    }
    let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing accounted current ICC"))?;
    if !(132..=65536).contains(&icc.len()) {
        return Err(CompositionError::Invalid("accounted current ICC bound"));
    }
    let mut nodes = 0_usize;
    let mut overlay_spaces = 0_u64;
    let mut max_args = 0_usize;
    let mut scratch = 0_u64;
    // Doc validation's transient BTreeMap/sets hold only small borrowed identity
    // keys in this class. 512 bytes per insertion overbounds an entire node for
    // each live identity/argument; count concurrent collections separately.
    let mut identities = doc.layers.len().checked_mul(4).ok_or_else(unsupported)?;
    for layer in &doc.layers {
        poll(interrupt)?;
        budget.consume_preparation_work(1)?;
        if !matches!(layer.content, LayerContent::Raster(_)) || layer.blend != photocraft_color::blend::BlendMode::Normal {
            return Err(unsupported());
        }
    }
    if let Some(state) = &doc.xcf_state {
        if !state.source_items.is_empty() || state.floating.is_some() || !state.editable_indexed.is_empty() {
            return Err(unsupported());
        }
        identities = identities
            .checked_add(state.items.len().checked_mul(4).ok_or_else(unsupported)?)
            .and_then(|n| n.checked_add(state.filter_masks.len().checked_mul(2)?))
            .and_then(|n| n.checked_add(state.selection.selected.len()))
            .ok_or_else(unsupported)?;
        for item in &state.items {
            poll(interrupt)?;
            budget.consume_preparation_work(1)?;
            if item.composition.as_ref().is_some_and(|p| {
                p.source_mode != 28
                    || p.legacy
                    || p.pass_through
                    || p.resolved.as_ref().is_some_and(|r| {
                        r.composite_space == photocraft_doc::import_compositing::GimpSpace::Lab
                            || r.blend_space == photocraft_doc::import_compositing::GimpSpace::Lab
                    })
            }) {
                return Err(unsupported());
            }
            if let Some(rect) = item.raster_bounds {
                let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or(CompositionError::Invalid("accounted extent overflow"))?;
                let bytes = count.checked_mul(32).ok_or(CompositionError::Invalid("accounted extent overflow"))?;
                scratch = scratch.max(bytes);
            }
        }
        for stack in &state.filter_stacks {
            poll(interrupt)?;
            budget.consume_preparation_work(1)?;
            if stack.nodes.len() > 256 {
                return Err(CompositionError::Invalid("accounted point node limit"));
            }
            for node in &stack.nodes {
                poll(interrupt)?;
                budget.consume_preparation_work(1)?;
                if node.arguments.len() > 10 {
                    return Err(unsupported());
                }
                if node.arguments.iter().any(|a| matches!(&a.value, photocraft_doc::xcf_filters::FilterValue::StoredColor(_))) {
                    return Err(CompositionError::Unsupported("accounted point stored color requires an exact literal binding"));
                }
                let overlay = node.operation.name == "gegl:color-overlay";
                if !scalar(&node.operation.name)
                    || !node.auxiliary.is_empty()
                    || node.arguments.len() > 10
                    || (!overlay
                        && node.arguments.iter().any(|a| {
                            !matches!(
                                a.value,
                                photocraft_doc::xcf_filters::FilterValue::Number(_)
                                    | photocraft_doc::xcf_filters::FilterValue::Bool(_)
                                    | photocraft_doc::xcf_filters::FilterValue::Enum { .. }
                            )
                        }))
                {
                    return Err(unsupported());
                }
                if node.operation.name == "gegl:color-rotate" {
                    crate::gimp_color_rotate::typed_arguments(&node.arguments)?;
                    // The implicit Perceptual candidate retains a builtin
                    // profile and (Gray only) one analytic transform. The
                    // constructor's effective-profile temporaries share the
                    // same conservative four-space envelope as Overlay.
                    overlay_spaces = overlay_spaces.checked_add(4).ok_or_else(unsupported)?;
                }
                if overlay {
                    // Nonallocating exact four-Float32 nonlinear literal and
                    // Bool only. Arbitrary color profiles/configurations never
                    // join this bounded allocation class.
                    crate::gimp_color_overlay::literal_arguments(&node.arguments)?;
                    // Candidate retains a profile and an analytic transform;
                    // effective-source and literal-conversion temporaries are
                    // live during construction. Four extra fixed analytic
                    // envelopes conservatively include all these owners.
                    overlay_spaces = overlay_spaces.checked_add(4).ok_or_else(unsupported)?;
                }
                nodes = nodes.checked_add(1).ok_or(CompositionError::Invalid("accounted node overflow"))?;
                max_args = max_args.max(node.arguments.len());
                identities = identities.checked_add(4).and_then(|n| n.checked_add(node.arguments.len())).ok_or_else(unsupported)?;
            }
        }
    }
    // A maximum four top-level spaces and one per scalar PointNode. Each
    // implicit analytic WorkingSpace has fixed curves/matrix; its <=160-byte
    // builtin description/copyright and <=12 tiny ICC tags require <16KiB
    // simultaneously across effective source/target/retained definitions,
    // UTF16/tag/data/entries/placed buffers and encoded output. 64KiB per space
    // includes capacity slack and all temporary profile clones, not size_of only.
    // One more 64KiB covers implicit binding/effective-profile construction.
    let profiles = u64::try_from(nodes)
        .map_err(|_| unsupported())?
        .checked_add(5)
        .and_then(|n| n.checked_add(overlay_spaces))
        .and_then(|n| n.checked_mul(65536))
        .ok_or(CompositionError::Invalid("accounted profile size overflow"))?;
    let tree = doc.layers.len().checked_mul(std::mem::size_of::<Prepared<'_>>()).ok_or(CompositionError::Invalid("accounted tree size overflow"))?;
    profiles
        .checked_add(u64::try_from(tree).map_err(|_| unsupported())?)
        .and_then(|n| n.checked_add(scratch))
        .and_then(|n| n.checked_add(u64::try_from(identities).ok()?.checked_mul(512)?))
        .and_then(|n| n.checked_add(65536))
        .and_then(|n| n.checked_add(crate::gimp_filters::scalar_preparation_bytes(nodes, max_args).ok()?))
        .and_then(|n| n.checked_add(crate::gimp_neighborhood::scalar_preparation_bytes(nodes).ok()?))
        .ok_or(CompositionError::Invalid("accounted preparation overflow"))
}
fn prepay_empty_mask_scan(mask: &photocraft_doc::xcf_filter_masks::FilterMask, budget: &mut impl PreparationBudget) -> Result<()> {
    // The authoritative empty-mask scan visits the default and every complete
    // physical tile, including padding outside the logical mask bounds. Keep
    // the prior conservative pass multiplier and logical allowance, but never
    // price a full stored tile as its small canvas intersection.
    let physical = u64::try_from(mask.surface.tile_count())
        .ok()
        .and_then(|n| n.checked_mul(photocraft_geom::TILE_SIZE as u64))
        .and_then(|n| n.checked_mul(photocraft_geom::TILE_SIZE as u64))
        .and_then(|n| n.checked_add(1))
        .ok_or_else(unsupported)?;
    let logical = u64::from(mask.bounds.width()).checked_mul(u64::from(mask.bounds.height())).ok_or_else(unsupported)?;
    budget.consume_preparation_work(logical.max(physical).checked_mul(32).ok_or_else(unsupported)?)
}
fn plane(surface: &Surface, mask: bool, interrupt: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<()> {
    for _ in surface.tiles() {
        poll(interrupt)?;
        budget.consume_preparation_work(1)?;
    }
    surface.try_validate_tile_origins_with(interrupt)?;
    let format = surface.format();
    if (mask && (format.mode != ColorMode::Grayscale || format.alpha)) || (!mask && !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)) {
        return Err(unsupported());
    }
    let channels = format.channels();
    let bpp = format.bytes_per_pixel();
    let check = |raw: &[u8], pixels: usize, budget: &mut dyn FnMut(u64) -> Result<()>| -> Result<()> {
        if raw.len() != pixels.checked_mul(bpp).ok_or_else(unsupported)? {
            return Err(CompositionError::Invalid("accounted plane size"));
        }
        for chunk in (0..pixels).step_by(64) {
            poll(interrupt)?;
            let units = (pixels - chunk).min(64).checked_mul(channels).ok_or_else(unsupported)?;
            budget(u64::try_from(units).map_err(|_| unsupported())?)?;
            let start = chunk.checked_mul(channels).ok_or_else(unsupported)?;
            for index in start..start.checked_add(units).ok_or_else(unsupported)? {
                let value =
                    photocraft_color::try_read_sample_f64(raw, format.sample, index).map_err(|_| CompositionError::Invalid("accounted sample access"))?;
                if !value.is_finite() || ((mask || (format.alpha && index % channels == format.mode.color_channels())) && !(0.0..=1.0).contains(&value)) {
                    return Err(CompositionError::Invalid("accounted current plane sample"));
                }
            }
        }
        Ok(())
    };
    check(surface.default_bytes(), 1, &mut |n| budget.consume_preparation_work(n))?;
    for (_, tile) in surface.tiles() {
        check(tile.bytes(), 65536, &mut |n| budget.consume_preparation_work(n))?;
    }
    Ok(())
}
pub(super) fn validate(doc: &Document, interrupt: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<()> {
    for chunk in doc.icc_profile.as_ref().ok_or_else(unsupported)?.chunks(4096) {
        poll(interrupt)?;
        budget.consume_preparation_work((chunk.len() as u64) * 2)?;
    }
    for layer in &doc.layers {
        plane(layer.surface().ok_or_else(unsupported)?, false, interrupt, budget)?;
        if let Some(mask) = &layer.mask {
            plane(&mask.surface, true, interrupt, budget)?;
        }
    }
    if let Some(state) = &doc.xcf_state {
        // Some authoritative per-node qualifiers repeat state validation. Pay
        // a conservative repeated borrowed-string/hash/identity envelope up
        // front. This is not an inner-validator cancellation/deadline claim.
        let mut bytes = 0_u64;
        let mut nodes = 0_u64;
        let mut add = |len: usize| -> Result<()> {
            bytes = bytes.checked_add(len as u64).ok_or_else(unsupported)?;
            Ok(())
        };
        for item in &state.items {
            poll(interrupt)?;
            add(item.name.len())?;
        }
        for opaque in &state.opaque {
            poll(interrupt)?;
            add(opaque.bytes.len())?;
            add(opaque.namespace.len())?;
            add(opaque.identity.len())?;
        }
        for stack in &state.filter_stacks {
            for node in &stack.nodes {
                poll(interrupt)?;
                nodes = nodes.checked_add(1).ok_or_else(unsupported)?;
                add(node.operation.name.len())?;
                add(node.operation.stored_version.len())?;
                add(node.operation.schema_epoch.len())?;
                if let Some(text) = &node.source_display_name {
                    add(text.len())?;
                }
                if let Some(text) = &node.source_icon {
                    add(text.len())?;
                }
                if let Some(policy) = &node.composition {
                    add(policy.compatibility_epoch.len())?;
                }
                for arg in &node.arguments {
                    poll(interrupt)?;
                    add(arg.name.len())?;
                    if let photocraft_doc::xcf_filters::FilterValue::Enum { domain, .. } = &arg.value {
                        add(domain.len())?;
                    }
                }
            }
        }
        for text in [&state.provenance.format, &state.provenance.container, &state.provenance.source_label] {
            add(text.len())?;
        }
        for text in [&state.provenance.producer, &state.provenance.release_id, &state.provenance.original_locator].into_iter().flatten() {
            add(text.len())?;
        }
        let units = bytes
            .checked_add((state.items.len() as u64).checked_mul(64).ok_or_else(unsupported)?)
            .and_then(|n| n.checked_mul(nodes.checked_add(2)?))
            .and_then(|n| n.checked_mul(64))
            .ok_or_else(unsupported)?;
        budget.consume_preparation_work(units)?;
        for opaque in &state.opaque {
            for bytes in opaque.bytes.chunks(4096) {
                poll(interrupt)?;
                budget.consume_preparation_work(bytes.len() as u64)?;
            }
        }
        for item in &state.items {
            poll(interrupt)?;
            budget.consume_preparation_work(item.name.len() as u64 + 1)?;
        }
        for stack in &state.filter_stacks {
            for node in &stack.nodes {
                poll(interrupt)?;
                budget.consume_preparation_work(
                    (node.operation.name.len() as u64).checked_add(node.operation.stored_version.len() as u64).ok_or_else(unsupported)?,
                )?;
                for arg in &node.arguments {
                    poll(interrupt)?;
                    budget.consume_preparation_work(arg.name.len() as u64 + 1)?;
                    if let photocraft_doc::xcf_filters::FilterValue::Enum { domain, .. } = &arg.value {
                        budget.consume_preparation_work(domain.len() as u64)?;
                    }
                }
            }
        }
        for mask in &state.filter_masks {
            plane(&mask.surface, true, interrupt, budget)?;
        }
        for item in &state.items {
            if let Some(rect) = item.raster_bounds {
                budget.consume_preparation_work(
                    u64::from(rect.width()).checked_mul(u64::from(rect.height())).and_then(|n| n.checked_mul(16)).ok_or_else(unsupported)?,
                )?;
            }
        }
        // Existing empty-selection qualification may inspect a mask per node.
        // Prepay each authoritative invocation separately; the default/all-tile
        // current scan above is a different pass and is not charged twice here.
        for stack in &state.filter_stacks {
            for node in &stack.nodes {
                poll(interrupt)?;
                if let Some(mask) = state.filter_masks.iter().find(|mask| node.mask == Some(photocraft_doc::item_identity::ItemRef::Mask(mask.id))) {
                    prepay_empty_mask_scan(mask, budget)?;
                }
            }
        }
    }
    poll(interrupt)
}
/// Prevent publication if actual retained Vec capacity exceeds the prepaid
/// envelope. No post-construction success can conceal capacity growth.
pub(super) fn validate_capacity(renderer: &PreparedRenderer<'_>, prepaid: u64) -> Result<()> {
    let tree = renderer
        .layers
        .capacity()
        .checked_mul(std::mem::size_of::<Prepared<'_>>())
        .and_then(|n| u64::try_from(n).ok())
        .ok_or(CompositionError::Invalid("actual prepared capacity overflow"))?;
    let spaces = renderer
        .spaces
        .capacity()
        .checked_mul(std::mem::size_of::<(photocraft_doc::import_compositing::GimpSpace, crate::gimp_composite::WorkingSpace)>())
        .and_then(|n| u64::try_from(n).ok())
        .ok_or(CompositionError::Invalid("actual space capacity overflow"))?;
    let mut actual = tree
        .checked_add(spaces)
        .and_then(|n| n.checked_add((renderer.spaces.len() as u64).checked_mul(65536)?))
        .ok_or(CompositionError::Invalid("actual preparation capacity overflow"))?;
    for node in &renderer.layers {
        if !node.children.is_empty() || node.group_gaussian.is_some() || node.text_source.is_some() || node.vector_source.is_some() {
            return Err(unsupported());
        }
        if let Some(filters) = &node.filters {
            actual = actual.checked_add(filters.scalar_retained_bytes()?).ok_or(CompositionError::Invalid("actual preparation capacity overflow"))?;
        }
    }
    if actual > prepaid {
        return Err(CompositionError::Invalid("actual retained preparation exceeds reservation"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::super::{PreparationBudget, PreparedRenderer};
    use crate::gimp_composite::{CompositionError, Result};
    use photocraft_doc::{ColorMode, Document, Layer, LayerMask, PixelFormat, SampleType, Size};
    use photocraft_raster::{Interrupt, Surface};
    use std::{cell::Cell, rc::Rc};
    struct Lease(Rc<Cell<u64>>, u64);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.set(self.0.get() - self.1);
        }
    }
    struct Budget {
        live: Rc<Cell<u64>>,
        limit: u64,
        work: u64,
        work_limit: u64,
    }
    impl Budget {
        fn new() -> Self {
            Self { live: Rc::new(Cell::new(0)), limit: 64 << 20, work: 0, work_limit: u64::MAX }
        }
    }
    impl PreparationBudget for Budget {
        type Lease = Lease;
        fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<Lease> {
            let next = self.live.get().checked_add(bytes).ok_or(CompositionError::Invalid("test bytes"))?;
            if next > self.limit {
                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
            }
            self.live.set(next);
            Ok(Lease(self.live.clone(), bytes))
        }
        fn consume_preparation_work(&mut self, units: u64) -> Result<()> {
            self.work = self.work.checked_add(units).ok_or(CompositionError::Invalid("test work"))?;
            if self.work > self.work_limit {
                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
            }
            Ok(())
        }
    }
    fn masked() -> Document {
        let mut doc = super::super::plain_point::tests::doc();
        doc.xcf_state = None;
        doc.size = Size::new(7, 5);
        let mut top = Layer::raster("masked", doc.pixel_format());
        top.surface_mut().unwrap().try_write_pixel_f64(1, 1, &[0.7, 0.4, 0.1, 0.5]).unwrap();
        top.mask = Some(LayerMask {
            surface: std::sync::Arc::new(Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &[128]).unwrap()),
            enabled: true,
            linked: true,
            density: 1.0,
            feather: 0.0,
        });
        doc.layers.push(top);
        doc
    }
    fn points() -> Document {
        use photocraft_doc::{
            SourceColorContext,
            item_identity::FilterId,
            xcf_filters::{ExactNumber, FilterArgument, FilterValue},
        };
        let mut doc = super::super::plain_point::tests::doc();
        doc.size = Size::new(8, 6);
        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 150 });
        let bounds = doc.bounds();
        let state = doc.xcf_state.as_mut().unwrap();
        state.items.truncate(1);
        state.filter_masks.clear();
        state.items[0].raster_bounds = Some(bounds);
        let owner_policy = state.items[0].composition.as_mut().unwrap();
        owner_policy.stored_composite_space = 2;
        owner_policy.stored_operator = 1;
        let node = &mut state.filter_stacks[0].nodes[0];
        node.mask = None;
        node.operation.schema_epoch = crate::gimp_filters::STACK_EDITS_EPOCH.into();
        node.arguments = vec![
            FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5_f32.to_bits())) },
            FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0)) },
        ];
        let policy = node.composition.as_mut().unwrap();
        policy.compatibility_epoch = crate::gimp_filters::STACK_EDITS_EPOCH.into();
        policy.stored_blend_space = -1178762671;
        policy.stored_composite_space = -1178762671;
        policy.stored_operator = -1178762671;
        let mut invert = node.clone();
        invert.id = FilterId(2);
        invert.operation.name = "gegl:invert-linear".into();
        invert.operation.stored_version = "0:0".into();
        invert.arguments.clear();
        state.filter_stacks[0].nodes.push(invert);
        doc
    }
    // UNRUN: a logically tiny empty mask may still retain a complete native
    // tile. Both the current-plane validation and later empty-mask qualification
    // must be paid, even when all retained words are signed zero.
    #[test]
    fn empty_mask_preparation_accounts_for_physical_padding_per_invocation() {
        use photocraft_doc::{item_identity::{ItemRef, MaskId}, xcf_filter_masks::FilterMask};
        let mut sparse = points();
        let bounds = sparse.bounds();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let state = sparse.xcf_state.as_mut().unwrap();
        let node = state.filter_stacks.first_mut().unwrap().nodes.first_mut().unwrap();
        node.mask = Some(ItemRef::Mask(MaskId(41)));
        state.filter_masks.push(FilterMask { id: MaskId(41), owner: node.id, surface: Surface::new(format), bounds });
        let mut dense = sparse.clone();
        let mask = dense.xcf_state.as_mut().unwrap().filter_masks.first_mut().unwrap();
        mask.surface.try_write_pixel_f64(0, 0, &[-0.0]).unwrap();
        assert!(mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
        let physical_words = mask.surface.tiles().map(|(_, tile)| (tile.bytes().len() / format.sample.bytes()) as u64).sum::<u64>();
        assert!(physical_words > u64::from(bounds.width()) * u64::from(bounds.height()));
        let mut baseline = Budget::new();
        super::validate(&sparse, &Interrupt::NONE, &mut baseline).unwrap();
        let mut actual = Budget::new();
        super::validate(&dense, &Interrupt::NONE, &mut actual).unwrap();
        assert!(actual.work - baseline.work >= physical_words * 2);
        let mut denied = Budget::new();
        denied.work_limit = baseline.work + physical_words;
        assert!(super::validate(&dense, &Interrupt::NONE, &mut denied).is_err());
        assert!(dense.xcf_state.as_ref().unwrap().filter_masks.first().unwrap().is_empty_selection_with(&Interrupt::NONE).unwrap());
    }

    #[test]
    fn color_rotate_census_precharges_builtin_gray_transform_and_rejects_bad_width() {
        use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
        let mut doc = points();
        let baseline = super::bytes(&doc, &Interrupt::NONE, &mut Budget::new()).unwrap();
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.name = "gegl:color-rotate".into();
        node.arguments = [("src-from", 15f32), ("src-to", 120.), ("dest-from", 30.), ("dest-to", 210.), ("threshold", 0.5), ("hue", 45.), ("saturation", 0.75)]
            .into_iter()
            .map(|(name, v)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) })
            .collect();
        node.arguments.push(FilterArgument { name: "src-clockwise".into(), value: FilterValue::Bool(false) });
        node.arguments.push(FilterArgument { name: "dest-clockwise".into(), value: FilterValue::Bool(false) });
        node.arguments.push(FilterArgument { name: "gray-mode".into(), value: FilterValue::Enum { domain: "gegl:color-rotate/gray-mode".into(), value: 0 } });
        let charged = super::bytes(&doc, &Interrupt::NONE, &mut Budget::new()).unwrap();
        assert!(charged >= baseline + 4 * 65536);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(15f64.to_bits()));
        assert!(super::bytes(&doc, &Interrupt::NONE, &mut Budget::new()).is_err());
    }
    #[test]
    fn overlay_literal_census_reserves_nested_profiles_and_refuses_unbounded_values() {
        use photocraft_doc::{
            SampleEncoding,
            xcf_filters::{ExactNumber, FilterArgument, FilterValue},
        };
        let mut doc = points();
        let baseline = super::bytes(&doc, &Interrupt::NONE, &mut Budget::new()).unwrap();
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.name = "gegl:color-overlay".into();
        node.arguments = vec![
            FilterArgument {
                name: "value".into(),
                value: FilterValue::Color {
                    components: vec![ExactNumber::Float32Bits(0.5_f32.to_bits()); 4],
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
            },
            FilterArgument { name: "srgb".into(), value: FilterValue::Bool(true) },
        ];
        let overlay = super::bytes(&doc, &Interrupt::NONE, &mut Budget::new()).unwrap();
        assert!(overlay >= baseline + 4 * 65536);
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        let FilterValue::Color { components, .. } = &mut node.arguments[0].value else {
            panic!("literal");
        };
        components.push(ExactNumber::Float32Bits(0));
        assert!(super::supports(&doc));
        assert!(super::bytes(&doc, &Interrupt::NONE, &mut Budget::new()).is_err());
    }
    #[test]
    fn accepted_masked_scene_requires_external_accounted_builder() {
        let doc = masked();
        PreparedRenderer::new_with_interrupt(&doc, &Interrupt::NONE).unwrap();
        let mut budget = Budget::new();
        PreparedRenderer::new_accounted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
    }
    #[test]
    fn accepted_scalar_stack_requires_external_accounted_builder() {
        let doc = points();
        PreparedRenderer::new_with_interrupt(&doc, &Interrupt::NONE).unwrap();
        let mut budget = Budget::new();
        PreparedRenderer::new_accounted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
    }
    #[test]
    fn accounted_outputs_match_existing_math_and_retain_source_preparation() {
        for doc in [masked(), points()] {
            let before = doc.clone();
            let old = PreparedRenderer::new_with_interrupt(&doc, &Interrupt::NONE).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap();
            let mut budget = Budget::new();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            let prepared = PreparedRenderer::new_accounted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
            let held = budget.live.get();
            assert!(held > 123);
            for _ in 0..2 {
                assert_eq!(prepared.renderer().render(doc.bounds(), &mut |_| Ok(())).unwrap(), old);
                assert_eq!(budget.live.get(), held);
            }
            assert_eq!(doc, before);
            drop(prepared);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn accounting_denial_work_and_mid_scan_cancel_refund_before_publication() {
        for mode in 0..4 {
            let doc = masked();
            let before = doc.clone();
            let mut budget = Budget::new();
            let unrelated = budget.reserve_prepared_bytes(1).unwrap();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            if mode == 0 {
                budget.limit = 124;
            }
            if mode == 1 {
                budget.work_limit = 1000;
            }
            let polls = std::sync::atomic::AtomicUsize::new(0);
            let cancel = || {
                let n = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                mode == 2 || (mode == 3 && n >= 12)
            };
            let control = Interrupt::cancel_only(&cancel);
            assert!(PreparedRenderer::new_accounted(&doc, &control, &mut budget, source).is_err());
            assert_eq!(budget.live.get(), 1);
            assert_eq!(doc, before);
            drop(unrelated);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn malformed_current_default_offbounds_and_complexity_stay_closed() {
        for change in 0..7 {
            let mut doc = masked();
            match change {
                0 => {
                    doc.icc_profile = Some(std::sync::Arc::new(vec![0; 132]));
                }
                1 => {
                    doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
                }
                2 | 3 => {
                    doc.depth = SampleType::F64;
                    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
                    let mut surface = Surface::new(format);
                    surface.try_write_pixel_f64(if change == 2 { 0 } else { -300 }, -300, &[f64::NAN, 0.2, 0.3, 1.0]).unwrap();
                    doc.layers[0].content = photocraft_doc::LayerContent::Raster(surface);
                }
                4 => {
                    doc.layers[0] = Layer::group("unsupported", vec![]);
                }
                5 => {
                    doc.layers[1].mask.as_mut().unwrap().surface = std::sync::Arc::new(
                        Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &f64::NAN.to_ne_bytes()).unwrap(),
                    );
                }
                _ => {
                    doc.icc_profile = None;
                }
            }
            let before = doc.clone();
            let mut budget = Budget::new();
            assert!(PreparedRenderer::new_accounted(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
            assert_eq!(doc, before);
            assert_eq!(budget.live.get(), 0);
        }
        let mut doc = points();
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value = photocraft_doc::xcf_filters::FilterValue::Config(vec![]);
        let mut budget = Budget::new();
        assert!(PreparedRenderer::new_accounted(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn structural_selector_keeps_invalid_members_and_excludes_other_owners() {
        let mut doc = masked();
        assert!(PreparedRenderer::supports_accounted_scalar_preparation(&doc));
        doc.icc_profile = None;
        assert!(PreparedRenderer::supports_accounted_scalar_preparation(&doc));
        doc.layers[1].mask.as_mut().unwrap().density = f32::NAN;
        assert!(PreparedRenderer::supports_accounted_scalar_preparation(&doc));
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
        assert!(!PreparedRenderer::supports_accounted_scalar_preparation(&doc));
        let mut doc = points();
        let node = doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.resize(257, node);
        assert!(PreparedRenderer::supports_accounted_scalar_preparation(&doc));
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.name = "gegl:color-exchange".into();
        assert!(!PreparedRenderer::supports_accounted_scalar_preparation(&doc));
        let mut doc = points();
        doc.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().compatibility_epoch =
            photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH.into();
        assert!(!PreparedRenderer::supports_accounted_scalar_preparation(&doc));
    }
}
