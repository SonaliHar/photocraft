//! Shared borrowed-plane preparation for closed qualified singleton filter domains.
use super::PreparationBudget;
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{
    ColorMode, Document, LayerContent, SampleEncoding, SampleType, WorkingProfileInterpretation,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
fn unsupported() -> CompositionError {
    CompositionError::Unsupported("budgeted singleton filter current state")
}
/// Only independently qualified singleton identities are selectable.
#[derive(Clone, Copy)]
pub(super) enum Singleton {
    Exposure,
    Box,
    Motion,
    Median,
    Gaussian,
    BrightnessContrast,
}
impl Singleton {
    fn bounds(self) -> Rect {
        match self {
            Self::Gaussian => Rect::new(0, 0, 96, 80),
            _ => Rect::new(0, 0, 32, 32),
        }
    }
    fn effect_tuple(self) -> i32 {
        match self {
            Self::Gaussian => -856260015,
            _ => -261651887,
        }
    }
    fn identity(self) -> (&'static str, &'static str, &'static str, &'static str, usize) {
        match self {
            Self::Exposure => {
                ("gegl:exposure", crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH, "1:0", crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH, 2)
            }
            Self::Box => ("gegl:box-blur", crate::gimp_neighborhood::BOX_BLUR_EPOCH, "0:0", crate::gimp_neighborhood::BOX_BLUR_EPOCH, 1),
            Self::Motion => ("gegl:motion-blur-linear", crate::gimp_neighborhood::MOTION_BLUR_EPOCH, "0:0", crate::gimp_neighborhood::MOTION_BLUR_EPOCH, 2),
            Self::Median => ("gegl:median-blur", crate::gimp_neighborhood::MEDIAN_BLUR_EPOCH, "0:0", crate::gimp_neighborhood::MEDIAN_BLUR_EPOCH, 6),
            Self::Gaussian => ("gegl:gaussian-blur", "xcf-22-stock-gaussian-replace", "0:0", "xcf-22-stock-gaussian-replace", 5),
            Self::BrightnessContrast => (
                "gegl:brightness-contrast",
                crate::gimp_filters::BRIGHTNESS_CONTRAST_PRECISION_EPOCH,
                "0:0",
                crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH,
                2,
            ),
        }
    }
}
/// Metadata membership is deliberately separate from sample/profile validation.
/// Invalid values and bindings in this family must not fall back to another route.
pub(crate) fn supports(doc: &Document) -> bool {
    supports_singleton(doc, Singleton::Exposure)
}
pub(super) fn supports_singleton(doc: &Document, kind: Singleton) -> bool {
    if doc.layers.len() != 1
        || doc.bounds() != kind.bounds()
        || doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::U8
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. })
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.quick_mask.is_some()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
    {
        return false;
    }
    let Some(layer) = doc.layers.first() else { return false };
    if !layer.visible || (layer.opacity.is_finite() && layer.opacity != 1.) || (layer.fill_opacity.is_finite() && layer.fill_opacity != 1.) {
        return false;
    }
    if !matches!(layer.content, LayerContent::Raster(_))
        || layer.blend != photocraft_color::blend::BlendMode::Normal
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || crate::effects::has_effects(layer)
        || layer.clipped
        || layer.video.is_some()
        || layer.fill_cache.is_some()
        || layer.indexed_source.is_some()
        || layer.excluded_channels != 0
        || layer.advanced != photocraft_doc::AdvancedBlending::default()
        || layer.blend_if != photocraft_doc::BlendIf::default()
    {
        return false;
    }
    let Some(state) = &doc.xcf_state else { return false };
    if state.items.len() != 2
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() != 1
        || !state.resources.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !state.item_sets.is_empty()
    {
        return false;
    }
    let Some(stack) = state.filter_stacks.first() else { return false };
    let Some(node) = stack.nodes.first() else { return false };
    let member = stack.nodes.len() == 1 && node.enabled && node.operation.name == kind.identity().0 && node.operation.schema_epoch == kind.identity().1;
    if member && matches!(kind, Singleton::BrightnessContrast) {
        return brightness_member(node);
    }
    member
}
/// Only known-valid, well-typed out-of-slice tuples retain the old route.
/// Malformed scoped tuples remain selected and fail authoritative validation.
fn brightness_member(node: &photocraft_doc::xcf_filters::FilterNode) -> bool {
    if node.arguments.len() != 2 {
        return true;
    }
    let mut brightness = None;
    let mut contrast = None;
    for arg in &node.arguments {
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = &arg.value else {
            return true;
        };
        let value = f32::from_bits(*bits);
        if !value.is_finite() {
            return true;
        }
        match arg.name.as_str() {
            "brightness" if brightness.is_none() && (-3.0..=3.0).contains(&value) => brightness = Some(*bits),
            "contrast" if contrast.is_none() && (-5.0..=5.0).contains(&value) => contrast = Some(*bits),
            _ => return true,
        }
    }
    match (brightness, contrast) {
        (Some(b), Some(c)) => b == 0 && matches!(c, 1065353216 | 1066401792),
        _ => true,
    }
}
/// The source and all inert metadata are borrowed, never cloned into a prepared tree.
/// Analytical implicit profiles use the same bounded profile allowance as plain
/// Normal, plus compiled fixed-size storage for one ordered point node.
pub(crate) fn preparation_bytes(icc_bytes: usize) -> Result<u64> {
    let fixed = std::mem::size_of::<super::Prepared<'_>>()
        .checked_add(std::mem::size_of::<crate::gimp_neighborhood::OrderedStack>())
        .and_then(|n| n.checked_add(std::mem::size_of::<crate::gimp_filters::PointStack>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<crate::gimp_composite::WorkingSpace>() * 4))
        .and_then(|n| n.checked_add(std::mem::size_of::<photocraft_algo::gimp::point::PointOperation>() * 4))
        .and_then(|n| n.checked_add(std::mem::size_of::<photocraft_algo::gimp::point::PreparedPointOperation>() * 4))
        .ok_or_else(unsupported)?;
    u64::try_from(icc_bytes)
        .map_err(|_| unsupported())?
        .checked_mul(128)
        .and_then(|n| n.checked_add(65536))
        .and_then(|n| n.checked_add(fixed as u64))
        .ok_or_else(unsupported)
}
pub(crate) fn validate(doc: &Document, ctl: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<()> {
    validate_singleton(doc, ctl, budget, Singleton::Exposure)
}
pub(super) fn validate_singleton(doc: &Document, ctl: &Interrupt<'_>, budget: &mut impl PreparationBudget, kind: Singleton) -> Result<()> {
    ctl.check().map_err(|_| RasterInputError::Cancelled)?;
    if !supports_singleton(doc, kind) {
        return Err(unsupported());
    }
    let layer = doc.layers.first().ok_or_else(unsupported)?;
    if layer.opacity != 1. || layer.fill_opacity != 1. {
        return Err(unsupported());
    }
    let state = doc.xcf_state.as_ref().ok_or_else(unsupported)?;
    let owner = ItemRef::Layer(layer.id);
    let bounds = kind.bounds();
    let stack = state.filter_stacks.first().ok_or_else(unsupported)?;
    let node = stack.nodes.first().ok_or_else(unsupported)?;
    let item = state.items.iter().find(|i| i.item == owner).ok_or_else(unsupported)?;
    let policy = item.composition.as_ref().ok_or_else(unsupported)?;
    if state.provenance.format_version != 22
        || stack.owner != owner
        || !item.visible
        || item.parent.is_some()
        || item.mask.is_some()
        || item.raster_bounds != Some(bounds)
        || policy.compatibility_epoch != "xcf-22-normal"
        || policy.source_mode != 28
        || policy.legacy
        || policy.pass_through
        || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1)
        || !policy
            .resolved
            .as_ref()
            .is_some_and(|r| r.blend_space == GimpSpace::LinearRgb && r.composite_space == GimpSpace::LinearRgb && r.operator == GimpOperator::Union)
        || node.id.0 == 0
        || node.operation.stored_version != kind.identity().2
        || node.opacity_f64_bits != 1_f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
    {
        return Err(unsupported());
    }
    let effect = node.composition.as_ref().ok_or_else(unsupported)?;
    if effect.compatibility_epoch != kind.identity().3
        || effect.source_mode != 62
        || effect.legacy
        || effect.pass_through
        || effect.resolved.is_some()
        || (effect.stored_blend_space, effect.stored_composite_space, effect.stored_operator) != (kind.effect_tuple(), kind.effect_tuple(), kind.effect_tuple())
        || node.arguments.len() != kind.identity().4
    {
        return Err(unsupported());
    }
    match kind {
        Singleton::Exposure | Singleton::BrightnessContrast => {
            let mut args = [("", 0.); 2];
            for (arg, out) in node.arguments.iter().zip(args.iter_mut()) {
                ctl.check().map_err(|_| RasterInputError::Cancelled)?;
                budget.consume_preparation_work(arg.name.len() as u64 + 1)?;
                let FilterValue::Number(ExactNumber::Float32Bits(bits)) = arg.value else { return Err(unsupported()) };
                if matches!(kind, Singleton::BrightnessContrast)
                    && !matches!((arg.name.as_str(), bits), ("brightness", 0) | ("contrast", 1065353216 | 1066401792))
                {
                    return Err(CompositionError::Unsupported("budgeted Brightness Contrast measured argument domain"));
                }
                *out = (arg.name.as_str(), f64::from(f32::from_bits(bits)));
            }
            // Existing registry math/type/domain validation; no new point eligibility.
            photocraft_algo::gimp::registry::point_member(&node.operation.name, Some(&node.operation.stored_version), &args)?;
        }
        Singleton::Box => {
            let arg = node.arguments.first().ok_or_else(unsupported)?;
            ctl.check().map_err(|_| RasterInputError::Cancelled)?;
            budget.consume_preparation_work(arg.name.len() as u64 + 1)?;
            if arg.name != "radius" || !matches!(arg.value, FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 | 5 })) {
                return Err(CompositionError::Unsupported("budgeted Box signed radius domain"));
            }
        }
        Singleton::Motion => {
            let mut length = false;
            let mut angle = false;
            for arg in &node.arguments {
                ctl.check().map_err(|_| RasterInputError::Cancelled)?;
                budget.consume_preparation_work(arg.name.len() as u64 + 1)?;
                match (arg.name.as_str(), &arg.value) {
                    ("length", FilterValue::Number(ExactNumber::Float32Bits(bits))) if !length && matches!(*bits, 0x41200000 | 0x41220000) => length = true,
                    ("angle", FilterValue::Number(ExactNumber::Float32Bits(0))) if !angle => angle = true,
                    _ => return Err(CompositionError::Unsupported("budgeted Motion measured argument domain")),
                }
            }
            if !length || !angle {
                return Err(CompositionError::Unsupported("budgeted Motion arguments absent"));
            }
        }
        Singleton::Median => {
            let mut seen = 0_u8;
            for arg in &node.arguments {
                ctl.check().map_err(|_| RasterInputError::Cancelled)?;
                let domain_bytes = match &arg.value {
                    FilterValue::Enum { domain, .. } => domain.len(),
                    _ => 0,
                };
                let work = arg.name.len().checked_add(domain_bytes).and_then(|n| n.checked_add(1)).ok_or_else(unsupported)?;
                budget.consume_preparation_work(u64::try_from(work).map_err(|_| unsupported())?)?;
                let bit = match (arg.name.as_str(), &arg.value) {
                    ("radius", FilterValue::Number(ExactNumber::Signed { width: 32, value: 3 | 4 })) => 1,
                    ("neighborhood", FilterValue::Enum { domain, value: 1 }) if domain == "gegl:median-blur::neighborhood" => 2,
                    ("percentile", FilterValue::Number(ExactNumber::Float32Bits(1112014848))) => 4,
                    ("alpha-percentile", FilterValue::Number(ExactNumber::Float32Bits(1112014848))) => 8,
                    ("abyss-policy", FilterValue::Enum { domain, value: 1 }) if domain == "gegl:median-blur::abyss-policy" => 16,
                    ("high-precision", FilterValue::Bool(false)) => 32,
                    _ => return Err(CompositionError::Unsupported("budgeted Median measured argument domain")),
                };
                if seen & bit != 0 {
                    return Err(CompositionError::Unsupported("budgeted Median duplicate argument"));
                }
                seen |= bit;
            }
            if seen != 63 {
                return Err(CompositionError::Unsupported("budgeted Median arguments absent"));
            }
        }
        Singleton::Gaussian => {
            let mut seen = 0_u8;
            for arg in &node.arguments {
                ctl.check().map_err(|_| RasterInputError::Cancelled)?;
                let domain_bytes = match &arg.value {
                    FilterValue::Enum { domain, .. } => domain.len(),
                    _ => 0,
                };
                let work = arg.name.len().checked_add(domain_bytes).and_then(|n| n.checked_add(1)).ok_or_else(unsupported)?;
                budget.consume_preparation_work(u64::try_from(work).map_err(|_| unsupported())?)?;
                let bit = match (arg.name.as_str(), &arg.value) {
                    ("std-dev-x", FilterValue::Number(ExactNumber::Float32Bits(1074790400 | 1081081856))) => 1,
                    ("std-dev-y", FilterValue::Number(ExactNumber::Float32Bits(1069547520))) => 2,
                    ("filter", FilterValue::Enum { domain, value: 0 }) if domain == "gegl:gaussian-blur::filter" => 4,
                    ("abyss-policy", FilterValue::Enum { domain, value: 1 }) if domain == "gegl:gaussian-blur::abyss-policy" => 8,
                    ("clip-extent", FilterValue::Bool(true)) => 16,
                    _ => return Err(CompositionError::Unsupported("budgeted Gaussian measured argument domain")),
                };
                if seen & bit != 0 {
                    return Err(CompositionError::Unsupported("budgeted Gaussian duplicate argument"));
                }
                seen |= bit;
            }
            if seen != 31 {
                return Err(CompositionError::Unsupported("budgeted Gaussian arguments absent"));
            }
        }
    }
    let Some(ItemRef::Mask(mask_id)) = node.mask else { return Err(unsupported()) };
    let mask = state.filter_masks.first().ok_or_else(unsupported)?;
    let mask_item = state.items.iter().find(|i| i.item == ItemRef::Mask(mask_id)).ok_or_else(unsupported)?;
    let metadata = mask_item.mask.as_ref().ok_or_else(unsupported)?;
    if mask_id.0 == 0
        || mask.id != mask_id
        || mask.owner != node.id
        || mask.bounds != bounds
        || !mask_item.visible
        || mask_item.parent.is_some()
        || mask_item.composition.is_some()
        || mask_item.raster_bounds != Some(bounds)
        || metadata.owner != owner
        || metadata.bounds != bounds
        || metadata.offset != [0, 0]
        || metadata.outside_f64_bits != 0
        || !metadata.enabled
        || metadata.shown
        || metadata.editing
        || metadata.linked
    {
        return Err(unsupported());
    }
    let surface = layer.surface().ok_or_else(unsupported)?;
    if surface.format() != photocraft_color::PixelFormat::RGBA8
        || mask.surface.format() != photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)
    {
        return Err(unsupported());
    }
    let opaque = matches!(kind, Singleton::Gaussian);
    if opaque && surface.tile(photocraft_geom::TileCoord { tx: 0, ty: 0 }).is_none() {
        return Err(CompositionError::Unsupported("budgeted Gaussian opaque source absent"));
    }
    scan(surface, bounds, false, opaque, ctl, budget)?;
    scan(&mask.surface, bounds, true, false, ctl, budget)?;
    for chunk in state.opaque.chunks(64) {
        ctl.check().map_err(|_| RasterInputError::Cancelled)?;
        budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or_else(unsupported)?)?;
        if chunk.iter().any(|record| record.rendering_required) {
            return Err(unsupported());
        }
    }
    ctl.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(())
}
fn scan(surface: &Surface, bounds: Rect, empty: bool, opaque: bool, ctl: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<()> {
    let channels = surface.format().channels();
    budget.consume_preparation_work((channels as u64).checked_mul(3).ok_or_else(unsupported)?)?;
    if surface.default_bytes().len() != channels || surface.default_bytes().iter().any(|b| *b != 0) {
        return Err(unsupported());
    }
    for (coordinate, tile) in surface.tiles() {
        ctl.check().map_err(|_| RasterInputError::Cancelled)?;
        let x = coordinate.tx.checked_mul(256).ok_or(RasterInputError::InvalidBounds)?;
        let y = coordinate.ty.checked_mul(256).ok_or(RasterInputError::InvalidBounds)?;
        x.checked_add(256).ok_or(RasterInputError::InvalidBounds)?;
        y.checked_add(256).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() != 65536usize.checked_mul(channels).ok_or_else(unsupported)? {
            return Err(unsupported());
        }
        for (part, chunk) in tile.bytes().chunks(channels * 64).enumerate() {
            ctl.check().map_err(|_| RasterInputError::Cancelled)?;
            // Own validation plus the existing current-context and selection-mask
            // scans are prepaid in bounded chunks before those checked readers run.
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(3).ok_or_else(unsupported)?)?;
            for (local, pixel) in chunk.chunks_exact(channels).enumerate() {
                let index = part.checked_mul(64).and_then(|n| n.checked_add(local)).ok_or_else(unsupported)?;
                let px = x.checked_add(i32::try_from(index % 256).map_err(|_| unsupported())?).ok_or(RasterInputError::InvalidBounds)?;
                let py = y.checked_add(i32::try_from(index / 256).map_err(|_| unsupported())?).ok_or(RasterInputError::InvalidBounds)?;
                if opaque && bounds.contains(px, py) && pixel.last() != Some(&255) {
                    return Err(CompositionError::Unsupported("budgeted Gaussian opaque source domain"));
                }
                if (empty || !bounds.contains(px, py)) && pixel.iter().any(|b| *b != 0) {
                    return Err(unsupported());
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
pub(super) mod tests {
    use super::super::{PreparationBudget, PreparedRenderer};
    use crate::gimp_composite::{CompositionError, Result};
    use photocraft_doc::{
        ColorMode, Document, FilterMask, Layer, SampleEncoding, SampleType, Size, WorkingProfileInterpretation, XcfState,
        import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
        item_identity::{FilterId, ItemRef, MaskId},
        xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
        xcf_state::{ItemLocks, ItemState, MaskState},
    };
    use photocraft_geom::Rect;
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
        fn new(limit: u64) -> Self {
            Self { live: Rc::new(Cell::new(0)), limit, work: 0, work_limit: u64::MAX }
        }
    }
    impl PreparationBudget for Budget {
        type Lease = Lease;
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<Lease> {
            let next = self.live.get().checked_add(n).ok_or(CompositionError::Invalid("test overflow"))?;
            if next > self.limit {
                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
            }
            self.live.set(next);
            Ok(Lease(self.live.clone(), n))
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
            let next = self.work.checked_add(n).ok_or(CompositionError::Invalid("test work overflow"))?;
            if next > self.work_limit {
                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
            }
            self.work = next;
            Ok(())
        }
    }
    pub(crate) fn doc() -> Document {
        let mut doc = Document::new("Captured Exposure domain", Size::new(32, 32), ColorMode::Rgb, SampleType::U8);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let bytes = photocraft_cms::Builtin::Srgb.profile().to_bytes();
        doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&bytes);
        doc.icc_profile = Some(bytes);
        let mut layer = Layer::raster("Original base", doc.pixel_format());
        layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.125, 0.25, 0.5, 1.0]).unwrap();
        let owner = ItemRef::Layer(layer.id);
        let mask_id = MaskId(2);
        let filter_id = FilterId(1);
        let bounds = Rect::new(0, 0, 32, 32);
        let mut state = XcfState::default();
        state.provenance.format_version = 22;
        state.items.push(ItemState {
            item: owner,
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(2),
            source_flags: 0,
            raster_bounds: Some(bounds),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: "xcf-22-normal".into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: -1,
                stored_operator: -1,
                pass_through: false,
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                }),
            }),
        });
        state.items.push(ItemState {
            item: ItemRef::Mask(mask_id),
            name: "Own operation gegl:exposure mask".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(3),
            source_flags: 0,
            raster_bounds: Some(bounds),
            mask: Some(MaskState { owner, bounds, offset: [0, 0], outside_f64_bits: 0, enabled: true, shown: false, editing: false, linked: false }),
            composition: None,
        });
        state.filter_masks.push(FilterMask {
            id: mask_id,
            owner: filter_id,
            surface: Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)),
            bounds,
        });
        state.filter_stacks.push(FilterStack {
            owner,
            revision: 0,
            nodes: vec![FilterNode {
                id: filter_id,
                operation: OperationIdentity {
                    name: "gegl:exposure".into(),
                    stored_version: "1:0".into(),
                    schema_epoch: crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into(),
                },
                arguments: vec![
                    FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0)) },
                    FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.125_f32.to_bits())) },
                ],
                source_display_name: Some("Own operation gegl:exposure".into()),
                source_icon: Some("gimp-image".into()),
                enabled: true,
                opacity_f64_bits: 1_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: Some(ItemRef::Mask(mask_id)),
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -261651887,
                    stored_composite_space: -261651887,
                    stored_operator: -261651887,
                    pass_through: false,
                    resolved: None,
                }),
            }],
        });
        doc.layers.push(layer);
        doc.xcf_state = Some(state);
        doc
    }
    #[test]
    fn singleton_exposure_preparation_retains_existing_ledger() {
        let doc = doc();
        let before = doc.clone();
        let mut budget = Budget::new(16 << 20);
        let source = budget.reserve_prepared_bytes(123).unwrap();
        let prepared = PreparedRenderer::new_plain_point_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        let mut live_output = 0;
        let mut peak = 0;
        let mut work = 0;
        let out = prepared
            .renderer()
            .render_reduced_with_interrupt(
                32,
                32,
                None,
                &mut |event| {
                    match event {
                        photocraft_cms::WideWork::ReserveBytes(n) => {
                            live_output += n;
                            peak = peak.max(live_output)
                        }
                        photocraft_cms::WideWork::ReleaseBytes(n) => live_output -= n,
                        photocraft_cms::WideWork::SampleUnits(n) => work += n,
                    };
                    Ok(())
                },
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(out.px.len(), 1024);
        assert_eq!(peak, 1024 * 16);
        assert_eq!(work, 256 * 1024);
        assert_eq!(doc, before);
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn malformed_member_rejects_without_fallback_and_refunds_owned_leases() {
        for kind in 0..14 {
            let mut doc = doc();
            match kind {
                0 => *doc.layers[0].surface_mut().unwrap() = Surface::try_with_default_bytes(doc.pixel_format(), &[1, 0, 0, 0]).unwrap(),
                1 => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &[0.1, 0.2, 0.3, 1.0]).unwrap(),
                2 => doc.icc_profile = None,
                3 => doc.icc_profile = Some(std::sync::Arc::new(vec![0; 200])),
                4 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[1.0]).unwrap(),
                5 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()))
                }
                6 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 1,
                7 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "unknown".into(),
                8 => doc.xcf_state.as_mut().unwrap().items[1].mask.as_mut().unwrap().owner = ItemRef::Layer(photocraft_doc::LayerId(999)),
                9 => doc.layers[0].opacity = f32::NAN,
                10 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(4096, -4096, &[1.0]).unwrap(),
                11 => {
                    doc.xcf_state.as_mut().unwrap().filter_masks[0].surface =
                        Surface::try_with_default_bytes(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &[1]).unwrap()
                }
                12 => {
                    let tile = doc.layers[0].surface().unwrap().tiles().next().unwrap().1.clone();
                    doc.layers[0].surface_mut().unwrap().put_tiles([(photocraft_geom::TileCoord { tx: i32::MAX, ty: 0 }, tile)]);
                }
                _ => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1].name = "black-level".into(),
            }
            assert!(PreparedRenderer::supports_plain_point_budgeted(&doc), "kind {kind}");
            let before = doc.clone();
            let mut budget = Budget::new(16 << 20);
            let unrelated = budget.reserve_prepared_bytes(17).unwrap();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            assert!(PreparedRenderer::new_plain_point_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err(), "kind {kind}");
            assert_eq!(budget.live.get(), 17);
            // NaN opacity has intentionally non-reflexive equality; exact source
            // raster and canonical state still prove no mutation for that case.
            assert_eq!(doc.layers[0].surface(), before.layers[0].surface());
            assert_eq!(doc.xcf_state, before.xcf_state);
            drop(unrelated);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn preparation_denial_work_denial_and_mid_scan_cancel_are_atomic() {
        let doc = doc();
        let before = doc.clone();
        let mut budget = Budget::new(124);
        let source = budget.reserve_prepared_bytes(123).unwrap();
        assert!(PreparedRenderer::new_plain_point_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err());
        assert_eq!(budget.work, 0);
        assert_eq!(budget.live.get(), 0);
        let mut budget = Budget::new(16 << 20);
        budget.work_limit = 100;
        let source = budget.reserve_prepared_bytes(123).unwrap();
        assert!(PreparedRenderer::new_plain_point_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err());
        assert!(budget.work <= 100);
        assert_eq!(budget.live.get(), 0);
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 10;
        let mut budget = Budget::new(16 << 20);
        let unrelated = budget.reserve_prepared_bytes(17).unwrap();
        let source = budget.reserve_prepared_bytes(123).unwrap();
        assert!(matches!(
            PreparedRenderer::new_plain_point_budgeted(&doc, &Interrupt::cancel_only(&cancel), &mut budget, source),
            Err(CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))
        ));
        assert!(budget.work > 20 && budget.work < 65536 * 4 * 3);
        assert_eq!(budget.live.get(), 17);
        drop(unrelated);
        assert_eq!(budget.live.get(), 0);
        assert_eq!(doc, before);
    }
    #[test]
    fn failed_point_work_releases_output_but_retains_source_and_preparation() {
        let doc = doc();
        let before = doc.clone();
        let mut budget = Budget::new(16 << 20);
        let source = budget.reserve_prepared_bytes(123).unwrap();
        let prepared = PreparedRenderer::new_plain_point_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        let retained = budget.live.get();
        let mut output = 0_u64;
        let mut samples = 0_u64;
        let result = prepared.renderer().render_reduced_with_interrupt(
            32,
            32,
            None,
            &mut |event| {
                match event {
                    photocraft_cms::WideWork::ReserveBytes(n) => output += n,
                    photocraft_cms::WideWork::ReleaseBytes(n) => output -= n,
                    photocraft_cms::WideWork::SampleUnits(n) => {
                        if samples + n > 128 {
                            return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
                        }
                        samples += n;
                    }
                }
                Ok(())
            },
            &Interrupt::NONE,
        );
        assert!(result.is_err());
        assert_eq!(samples, 128);
        assert_eq!(output, 0);
        assert_eq!(budget.live.get(), retained);
        assert_eq!(doc, before);
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
    }
}
