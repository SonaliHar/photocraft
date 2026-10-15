//! Closed owned flat separable document family. Original source selection is
//! independent; this binds CURRENT metadata, exact implicit profile and native
//! operative storage before any Session publication.
use crate::{
    backed_normal::{BackedNormalError as E, NormalBandLimits},
    backed_normal_stack::OwnedNormalStackBand,
    backed_separable::{self as lower, PreparedBackedSeparableStack, SeparableStackLayer, Spaces},
    gimp_composite::CompositionError,
};
use photocraft_cms::Builtin;
use photocraft_doc::{
    ColorMode, Document, LayerContent, SampleEncoding, SampleType, WorkingProfileInterpretation,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
};
fn scope() -> E {
    E::Unsupported("closed owned flat separable current recipe")
}

#[cfg(test)]
#[path = "backed_separable_document_tests.rs"]
mod tests;

#[path = "backed_separable_component_source.rs"]
mod component_source;
#[path = "backed_separable_general_current.rs"]
mod general_current;
#[path = "backed_separable_general_source.rs"]
mod general_source;
#[path = "backed_separable_legacy_source.rs"]
mod legacy_source;
#[derive(Clone, Copy)]
struct Recipe {
    mode: i32,
    blend: GimpSpace,
    composite: GimpSpace,
    operator: GimpOperator,
}
fn transfer(s: GimpSpace) -> Result<SampleEncoding, E> {
    match s {
        GimpSpace::LinearRgb => Ok(SampleEncoding::Linear),
        GimpSpace::ProfileRgb => Ok(SampleEncoding::NonlinearProfile),
        GimpSpace::PerceptualRgb => Ok(SampleEncoding::Perceptual),
        GimpSpace::Lab => Err(scope()),
    }
}
fn equal(a: &[u8], b: &[u8], check: &mut impl FnMut(u64) -> Result<(), E>) -> Result<bool, E> {
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.chunks(4096).zip(b.chunks(4096)) {
        check((a.len() * 2) as u64)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn metadata(doc: &Document, source: bool, interrupt: &Interrupt<'_>, check: &mut impl FnMut(u64) -> Result<(), E>) -> Result<[Recipe; 2], E> {
    check(1)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let original = doc.source_color_context.as_ref().ok_or_else(scope)?;
    if doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::F32
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. })
        || original.source_precision != 650
        || original.original_icc.is_some()
        || doc.bounds() != Rect::new(0, 0, 16, 12)
        || doc.layers.len() != 2
        || state.items.len() != 2
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !matches!(state.provenance.format_version, 12 | 23)
    {
        return Err(scope());
    }
    check(1)?;
    let profile = Builtin::Srgb.profile();
    check(1)?;
    if !equal(doc.icc_profile.as_deref().map(Vec::as_slice).ok_or_else(scope)?, profile.encoded_bytes().ok_or_else(scope)?, check)? {
        return Err(scope());
    }
    let current = doc.icc_profile.as_deref().map(Vec::as_slice).ok_or_else(scope)?;
    check(current.len() as u64 + 64)?;
    doc.working_profile_interpretation
        .validate_with_interrupt(Some(current), doc.mode, interrupt)
        .map_err(|_| if interrupt.cancelled() { E::Backing(BackingError::Cancelled) } else { scope() })?;
    check(0)?;
    for opaque in &state.opaque {
        check(1)?;
        if opaque.rendering_required {
            return Err(scope());
        }
    }
    let mut out = [Recipe { mode: 28, blend: GimpSpace::LinearRgb, composite: GimpSpace::LinearRgb, operator: GimpOperator::Union }; 2];
    for (index, layer) in doc.layers.iter().enumerate() {
        check(512)?;
        let item = state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
        let extent = item.raster_bounds.ok_or_else(scope)?;
        let format = match &layer.content {
            LayerContent::BackedRaster(backed) if !source && backed.operative_bounds() == extent => backed.format(),
            // Source preparation precedes the move-only native promotion. Its
            // original native plane has already passed the source qualifier.
            LayerContent::Raster(surface) if source => surface.format(),
            _ => return Err(scope()),
        };
        let policy = item.composition.as_ref().ok_or_else(scope)?;
        if policy.compatibility_epoch.len() > 128 {
            return Err(scope());
        }
        check(policy.compatibility_epoch.len() as u64)?;
        let resolved = policy.checked_resolved().map_err(|_| scope())?;
        let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
        if item.parent.is_some()
            || item.visible != layer.visible
            || (source && !layer.visible)
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.blend_if != photocraft_doc::BlendIf::default()
            || policy.legacy
            || policy.pass_through
            || physical.pass_through
            || physical.blend_encoding != transfer(resolved.blend_space)?
            || physical.composite_encoding != transfer(resolved.composite_space)?
            || format != doc.pixel_format()
            || !format.alpha
        {
            return Err(scope());
        }
        if index == 0 {
            let epoch = if state.provenance.format_version == 12 { "xcf-12-normal" } else { "xcf-23-normal" };
            if policy.source_mode != 28
                || policy.compatibility_epoch != epoch
                || extent != doc.bounds()
                || (source && layer.opacity.to_bits() != 1f32.to_bits())
                || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1)
                || resolved.blend_space != GimpSpace::LinearRgb
                || resolved.composite_space != GimpSpace::LinearRgb
                || resolved.operator != GimpOperator::Union
            {
                return Err(scope());
            }
        } else {
            if !lower::is_supported_mode(policy.source_mode) || policy.stored_operator != -2 || resolved.operator != GimpOperator::ClipToBackdrop {
                return Err(scope());
            }
            let bounds = extent;
            if !(bounds == Rect::new(2, -1, 12, 8) || bounds == Rect::new(-1, 2, 9, 11))
                || source && layer.opacity.to_bits() != if bounds.x0 == 2 { 0.7125f32.to_bits() } else { 0.4375f32.to_bits() }
            {
                return Err(scope());
            }
            let (blend, composite) = match (state.provenance.format_version, policy.source_mode, policy.stored_blend_space, policy.stored_composite_space) {
                (12, 30, -1, -1) => (GimpSpace::LinearRgb, GimpSpace::LinearRgb),
                (12, 30, 2, -1) => (GimpSpace::ProfileRgb, GimpSpace::LinearRgb),
                (23, 30, 4, -1) => (GimpSpace::PerceptualRgb, GimpSpace::LinearRgb),
                (12, 30, -1, 2) => (GimpSpace::LinearRgb, GimpSpace::ProfileRgb),
                (23, 30, -1, 4) => (GimpSpace::LinearRgb, GimpSpace::PerceptualRgb),
                (23, 23 | 31 | 32 | 42..=53, -4, -1) => (GimpSpace::PerceptualRgb, GimpSpace::LinearRgb),
                (12, 33 | 34 | 41, -1, -1) => (GimpSpace::LinearRgb, GimpSpace::LinearRgb),
                (12, 35 | 36, 0, -1) => (GimpSpace::ProfileRgb, GimpSpace::LinearRgb),
                _ => return Err(scope()),
            };
            let epoch = match (state.provenance.format_version, policy.source_mode) {
                (12, 35 | 36) => "xcf-12-authored-equivalent-rgb-space",
                (12, _) => "xcf-12-authored-blend-auto",
                (23, _) => "xcf-23-authored-blend-auto",
                _ => return Err(scope()),
            };
            if policy.compatibility_epoch != epoch || resolved.blend_space != blend || resolved.composite_space != composite {
                return Err(scope());
            }
        }
        *out.get_mut(index).ok_or_else(scope)? =
            Recipe { mode: policy.source_mode, blend: resolved.blend_space, composite: resolved.composite_space, operator: resolved.operator };
    }
    if doc.layers.first().map(|l| l.id) == doc.layers.get(1).map(|l| l.id) {
        return Err(scope());
    }
    Ok(out)
}
/// Stage/profile preparation only for an independently admitted original source.
/// No decoded source pixels are evaluated and no source authority is synthesized.
pub struct LeasedSeparableBlendPipeline<S, L> {
    _recipes: Option<general_source::Recipes<L>>,
    _spaces: Spaces<L>,
    _source: S,
}
impl<S, L> LeasedSeparableBlendPipeline<S, L> {
    pub fn new_budgeted<B: crate::gimp_render::PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let result = (|| -> Result<Self, E> {
            let mut check = |n| {
                if interrupt.cancelled() {
                    return Err(E::Backing(BackingError::Cancelled));
                }
                budget.consume_preparation_work(n).map_err(E::Color)
            };
            let recipes = metadata(doc, true, interrupt, &mut check)?;
            let mut needed = [false; 3];
            for r in recipes {
                *needed.get_mut(lower::slot(r.blend)?).ok_or_else(scope)? = true;
                *needed.get_mut(lower::slot(r.composite)?).ok_or_else(scope)? = true;
            }
            let profile = Builtin::Srgb.profile();
            let (bytes, work) = lower::space_requirements(profile, SampleEncoding::NonlinearProfile, needed, std::mem::size_of::<Self>())?;
            let fee = budget.reserve_prepared_bytes(bytes).map_err(E::Color)?;
            budget.consume_preparation_work(work).map_err(E::Color)?;
            let spaces = Spaces::prepare(profile, SampleEncoding::NonlinearProfile, needed, fee, interrupt)?;
            Ok(Self { _recipes: None, _spaces: spaces, _source: source_lease })
        })();
        result.map_err(|e| match e {
            E::Color(e) => e,
            E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
            _ => CompositionError::Unsupported("flat separable source preparation"),
        })
    }
    /// Preparation for separately admitted exact legacy two-leaf originals.
    pub fn new_legacy_budgeted<B: crate::gimp_render::PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let result = (|| -> Result<Self, E> {
            let mut check = |n| {
                if interrupt.cancelled() {
                    return Err(E::Backing(BackingError::Cancelled));
                }
                budget.consume_preparation_work(n).map_err(E::Color)
            };
            let recipes = legacy_source::metadata(doc, interrupt, &mut check)?;
            let mut needed = [false; 3];
            for r in recipes {
                *needed.get_mut(lower::slot(r.blend)?).ok_or_else(scope)? = true;
                *needed.get_mut(lower::slot(r.composite)?).ok_or_else(scope)? = true;
            }
            let profile = Builtin::Srgb.profile();
            let (bytes, work) = lower::space_requirements(profile, SampleEncoding::NonlinearProfile, needed, std::mem::size_of::<Self>())?;
            let fee = budget.reserve_prepared_bytes(bytes).map_err(E::Color)?;
            budget.consume_preparation_work(work).map_err(E::Color)?;
            let spaces = Spaces::prepare(profile, SampleEncoding::NonlinearProfile, needed, fee, interrupt)?;
            Ok(Self { _recipes: None, _spaces: spaces, _source: source_lease })
        })();
        result.map_err(|e| match e {
            E::Color(e) => e,
            E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
            _ => CompositionError::Unsupported("legacy flat separable source preparation"),
        })
    }
    /// Preparation for separately admitted exact component/luma originals.
    pub fn new_component_budgeted<B: crate::gimp_render::PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let result = (|| -> Result<Self, E> {
            let mut check = |n| {
                if interrupt.cancelled() {
                    return Err(E::Backing(BackingError::Cancelled));
                }
                budget.consume_preparation_work(n).map_err(E::Color)
            };
            let recipes = component_source::metadata(doc, interrupt, &mut check)?;
            let mut needed = [false; 3];
            for r in recipes {
                *needed.get_mut(lower::slot(r.blend)?).ok_or_else(scope)? = true;
                *needed.get_mut(lower::slot(r.composite)?).ok_or_else(scope)? = true;
            }
            let profile = Builtin::Srgb.profile();
            let (bytes, work) = lower::space_requirements(profile, SampleEncoding::NonlinearProfile, needed, std::mem::size_of::<Self>())?;
            let fee = budget.reserve_prepared_bytes(bytes).map_err(E::Color)?;
            budget.consume_preparation_work(work).map_err(E::Color)?;
            let spaces = Spaces::prepare(profile, SampleEncoding::NonlinearProfile, needed, fee, interrupt)?;
            Ok(Self { _recipes: None, _spaces: spaces, _source: source_lease })
        })();
        result.map_err(|e| match e {
            E::Color(e) => e,
            E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
            _ => CompositionError::Unsupported("component/luma flat source preparation"),
        })
    }
    /// Preparation only for the separately qualified generic original epoch.
    /// This does not evaluate or promote source-native pixels.
    pub fn new_general_budgeted<B: crate::gimp_render::PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let result = (|| -> Result<Self, E> {
            let (recipes, spaces) = general_source::prepare(doc, interrupt, budget, std::mem::size_of::<Self>())?;
            Ok(Self { _recipes: Some(recipes), _spaces: spaces, _source: source_lease })
        })();
        result.map_err(|e| match e {
            E::Color(e) => e,
            E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
            _ => CompositionError::Unsupported("general flat separable source preparation"),
        })
    }
}
pub struct PreparedBackedSeparableDocument<'a> {
    stack: PreparedBackedSeparableStack<'a>,
    _metadata: general_current::Metadata<'a>,
    owner: &'a AccountedDocument,
    limits: NormalBandLimits,
    _index: QuotaLease,
}
impl<'a> PreparedBackedSeparableDocument<'a> {
    pub fn new(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        owner.validate_readonly_flat_composite_storage(c).map_err(E::Backing)?;
        let index = owner.quota().reserve(QuotaKind::Index, std::mem::size_of::<Self>() as u64).map_err(E::Quota)?;
        let metadata = general_current::Metadata::new(owner, c)?;
        let stack = PreparedBackedSeparableStack::new(&metadata.inputs, owner.quota(), Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, limits, c)?;
        let out = Self { stack, _metadata: metadata, owner, limits, _index: index };
        out.validate_operative(c)?;
        Ok(out)
    }
    pub fn bounds(&self) -> Rect {
        self.owner.document().bounds()
    }
    fn scan_native(&self, c: &ReadControl<'_>) -> Result<(), E> {
        let _scratch = self
            .owner
            .quota()
            .reserve(QuotaKind::Resident, (std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() + 128) as u64)
            .map_err(E::Quota)?;
        for (layer, recipe) in self.owner.document().layers.iter().zip(&self._metadata.recipes) {
            let mode = recipe.mode;
            let unit = matches!(mode, 41..=49 | 51 | 53) || lower::is_supported_legacy_mode(mode) || lower::is_supported_component_mode(mode);
            let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
            let view = backed.checked_view();
            let f = view.format();
            let check = |raw: &[u8], endpoint: bool| -> Result<(), E> {
                c.check(f.channels() as u64).map_err(E::Backing)?;
                let p = lower::decode(raw, f)?;
                for channel in 0..3 {
                    let v = *p.get(channel).ok_or_else(scope)?;
                    if unit && !(0.0..=1.0).contains(&v) {
                        return Err(scope());
                    }
                    if endpoint
                        && match mode {
                            42 => v >= 1.,
                            48 => v <= 0. || v >= 1.,
                            41 | 43 => v <= 0.,
                            _ => false,
                        }
                    {
                        return Err(scope());
                    }
                }
                Ok(())
            };
            check(view.default_bytes(), false)?;
            for coord in view.logical_bounds().tiles() {
                let pin = view.pin_tile(coord, c).map_err(E::View)?;
                let r = coord.rect().intersect(&view.logical_bounds());
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        let mut bytes = [0; 40];
                        let raw = bytes.get_mut(..f.bytes_per_pixel()).ok_or_else(scope)?;
                        pin.read_pixel_native_into(x, y, raw, c).map_err(E::View)?;
                        check(raw, true)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn validate_operative(&self, c: &ReadControl<'_>) -> Result<(), E> {
        self.scan_native(c)?;
        general_current::validate_bands(&self.stack, self.bounds(), self.limits, c)
    }
    pub fn render_band(&self, rect: Rect, c: &ReadControl<'_>) -> Result<OwnedNormalStackBand, E> {
        if !self.bounds().contains_rect(&rect) {
            return Err(scope());
        }
        self.stack.render_band(rect, [0.; 4], c)
    }
}
