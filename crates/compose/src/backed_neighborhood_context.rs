//! Closed CURRENT neighborhood recipes over genuine owned native storage.
//! Source classification and raw persistence remain separate authorities.
use crate::{
    backed_normal::{BackedNormalError as E, NormalBandLimits},
    backed_stock_point_context::{OwnedStockPointWideBand, PlanBudget, bound_profile},
    gimp_composite::CompositionError,
    gimp_neighborhood::{OrderedStack, OwnedNeighborhoodSurface},
    gimp_render::PreparationBudget,
};
use photocraft_cms::Profile;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    Document, LayerContent, LayerId, SampleEncoding,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
    checked_view::NativeTileBytes,
};
#[path = "backed_neighborhood_assigned_profile.rs"]
mod assigned_profile;
use assigned_profile::BoundAssignedNeighborhoodProfile;

fn scope() -> E {
    E::Unsupported("closed owned neighborhood current recipe")
}
fn size() -> E {
    E::Invalid("neighborhood resource size")
}
/// Routing hint only; exact current preparation remains mandatory.
pub fn is_candidate(doc: &Document, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    c.check(1)?;
    let Some(state) = doc.xcf_state.as_ref() else { return Ok(false) };
    if state.filter_stacks.len() > 1 {
        return Ok(false);
    }
    let Some(stack) = state.filter_stacks.first() else { return Ok(false) };
    if stack.nodes.len() != 1 {
        return Ok(false);
    }
    let node = stack.nodes.first().ok_or(BackingError::Integrity)?;
    if node.operation.name.len() > 128 {
        return Ok(false);
    }
    c.check(node.operation.name.len() as u64)?;
    Ok(matches!(node.operation.name.as_str(), "gegl:box-blur" | "gegl:median-blur" | "gegl:motion-blur-linear" | "gegl:pixelize" | "gegl:gaussian-blur"))
}
struct NativeBudget<'a, 'c> {
    q: &'a BackingQuota,
    c: &'a ReadControl<'c>,
}
impl PlanBudget for NativeBudget<'_, '_> {
    type Lease = QuotaLease;
    fn reserve(&mut self, n: u64) -> Result<QuotaLease, E> {
        self.q.reserve(QuotaKind::Resident, n).map_err(E::Quota)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.c.check(n).map_err(E::Backing)
    }
}
struct Shared<'a, B> {
    budget: &'a mut B,
}
impl<B: PreparationBudget> PlanBudget for Shared<'_, B> {
    type Lease = B::Lease;
    fn reserve(&mut self, n: u64) -> Result<Self::Lease, E> {
        self.budget.reserve_prepared_bytes(n).map_err(E::Color)
    }
    fn check(&mut self, n: u64) -> Result<(), E> {
        self.budget.consume_preparation_work(n).map_err(E::Color)
    }
}
struct RetainedPlan {
    plan: Plan<QuotaLease>,
    _lease: QuotaLease,
}
enum ProfileBinding<L> {
    Implicit(Profile),
    Assigned(BoundAssignedNeighborhoodProfile<L>),
}
impl<L> ProfileBinding<L> {
    fn profile(&self) -> &Profile {
        match self {
            Self::Implicit(p) => p,
            Self::Assigned(p) => p.profile(),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum AssignedRecipe {
    Box,
    Median,
}
impl AssignedRecipe {
    fn operation(self) -> &'static str {
        match self {
            Self::Box => "gegl:box-blur",
            Self::Median => "gegl:median-blur",
        }
    }
}
struct Plan<L> {
    stack: OrderedStack,
    _profile: ProfileBinding<L>,
    opaque: bool,
}
pub struct LeasedStockNeighborhoodPipeline<S, L> {
    _plan: Plan<L>,
    _source: S,
    _lease: L,
}
fn prepare<B: PlanBudget>(
    doc: &Document,
    supplied: Option<&Profile>,
    header: u64,
    interrupt: &Interrupt<'_>,
    b: &mut B,
) -> Result<(Plan<B::Lease>, B::Lease), E> {
    prepare_mode(doc, supplied, header, interrupt, b, false, false)
}
fn prepare_mode<B: PlanBudget>(
    doc: &Document,
    supplied: Option<&Profile>,
    header: u64,
    interrupt: &Interrupt<'_>,
    b: &mut B,
    assigned: bool,
    current: bool,
) -> Result<(Plan<B::Lease>, B::Lease), E> {
    prepare_recipe(doc, supplied, header, interrupt, b, if assigned { Some(AssignedRecipe::Box) } else { None }, current)
}
fn prepare_recipe<B: PlanBudget>(
    doc: &Document,
    supplied: Option<&Profile>,
    header: u64,
    interrupt: &Interrupt<'_>,
    b: &mut B,
    recipe: Option<AssignedRecipe>,
    current: bool,
) -> Result<(Plan<B::Lease>, B::Lease), E> {
    let assigned = recipe.is_some();
    b.check(32768)?;
    interrupt.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    let [layer] = doc.layers.as_slice() else { return Err(scope()) };
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let [stack] = state.filter_stacks.as_slice() else { return Err(scope()) };
    let [node] = stack.nodes.as_slice() else { return Err(scope()) };
    let (extent, format, default) = match &layer.content {
        LayerContent::BackedRaster(v) => (v.operative_bounds(), v.format(), v.default_bytes()),
        LayerContent::Raster(v) => {
            (state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).and_then(|i| i.raster_bounds).ok_or_else(scope)?, v.format(), v.default_bytes())
        }
        _ => return Err(scope()),
    };
    if extent != doc.bounds()
        || !matches!((extent.width(), extent.height()), (32, 32) | (16, 12) | (96, 80))
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || state.provenance.format_version != 22
        || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
        || state.filter_masks.len() > 1
        || state.items.len() != state.filter_masks.len() + 1
        || stack.owner != ItemRef::Layer(layer.id)
        || doc.source_color_context.as_ref().is_none_or(|s| if assigned { s.original_icc.is_none() } else { s.original_icc.is_some() })
        || !state.editable_indexed.is_empty()
        || (!(assigned && current) && !layer.visible)
        || (!(assigned && current) && layer.opacity.to_bits() != 1f32.to_bits())
        || (assigned && !current && !node.enabled)
    {
        return Err(scope());
    }
    let finite = extent == Rect::new(0, 0, 16, 12);
    let original_gaussian = extent == Rect::new(0, 0, 96, 80);
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    if assigned
        && (!finite
            || doc.mode != ColorMode::Rgb
            || doc.depth != SampleType::F32
            || source.source_precision != 650
            || recipe.is_none_or(|recipe| node.operation.name != recipe.operation())
            || node.mask.is_none()
            || state.filter_masks.len() != 1)
    {
        return Err(scope());
    }
    if recipe == Some(AssignedRecipe::Median) && node.composition.as_ref().is_none_or(|policy| policy.compatibility_epoch != "xcf22-rgb-alpha-median-v1") {
        return Err(scope());
    }
    if finite {
        if !matches!((doc.depth, source.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750))
            || !(doc.mode == ColorMode::Rgb || doc.mode == ColorMode::Grayscale && node.operation.name == "gegl:median-blur")
        {
            return Err(scope());
        }
        let epoch = match node.operation.name.as_str() {
            "gegl:box-blur" => "xcf22-rgb-box-replace-v1",
            "gegl:median-blur" if doc.mode == ColorMode::Grayscale => "xcf22-gray-alpha-median-v1",
            "gegl:median-blur" => "xcf22-rgb-alpha-median-v1",
            "gegl:gaussian-blur" => "xcf22-rgb-alpha-finite-gaussian-v1",
            _ => return Err(scope()),
        };
        if node.operation.schema_epoch != epoch {
            return Err(scope());
        }
    } else if original_gaussian {
        if doc.mode != ColorMode::Rgb
            || doc.depth != SampleType::U8
            || source.source_precision != 150
            || node.operation.name != "gegl:gaussian-blur"
            || node.operation.schema_epoch != "xcf-22-stock-gaussian-replace"
            || node.composition.as_ref().is_none_or(|policy| policy.compatibility_epoch != "xcf-22-stock-gaussian-replace")
            || node.mask.is_none()
            || state.filter_masks.len() != 1
        {
            return Err(scope());
        }
    } else if extent != Rect::new(0, 0, 32, 32)
        || doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::U8
        || source.source_precision != 150
        || !matches!(
            (node.operation.name.as_str(), node.operation.schema_epoch.as_str()),
            ("gegl:box-blur", "xcf22-rgb-box-replace-v1")
                | ("gegl:median-blur", "xcf22-rgb-u8-quantized-median-v1")
                | ("gegl:motion-blur-linear", "xcf22-rgb-u8-linear-motion-v1")
                | ("gegl:pixelize", "xcf22-rgb-u8-square-pixelize-v1")
        )
    {
        return Err(scope());
    }
    crate::gimp_render::qualify_current_point_optional_mask_metadata(doc, layer, state, stack, extent, SampleEncoding::NonlinearProfile, format, default)
        .map_err(E::Color)?;
    let item = state.items.iter().find(|v| v.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
    let resolved = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
    let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
    if resolved.operator != GimpOperator::Union
        || physical.pass_through
        || physical.blend_encoding != space(resolved.blend_space)?
        || physical.composite_encoding != space(resolved.composite_space)?
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
    let mut masks = 0u64;
    for mask in &state.filter_masks {
        masks = masks.checked_add((mask.surface.tile_count() as u64 + 1).checked_mul(65536 * 32).ok_or_else(size)?).ok_or_else(size)?;
    }
    b.check(masks)?;
    crate::gimp_neighborhood::qualify_selection_mask_with_interrupt(state, node, layer.id, doc.bounds(), interrupt).map_err(E::Color)?;
    let scalar = crate::gimp_neighborhood::scalar_preparation_bytes(2).map_err(E::Color)?;
    let envelope = scalar.checked_add(2 * 65536).and_then(|v| v.checked_add(header)).ok_or_else(size)?;
    let lease = b.reserve(envelope)?;
    let (profile, implicit_basis) = if assigned {
        let binding = BoundAssignedNeighborhoodProfile::prepare(doc, interrupt, b)?;
        // Both profile peaks are real retained fees, separate from the ordered
        // stack's prospective curve/header envelope.
        let _ = binding.held_peak()?;
        (ProfileBinding::Assigned(binding), None)
    } else {
        let (profile, basis) = bound_profile(doc, supplied, b, interrupt)?;
        (ProfileBinding::Implicit(profile), Some(basis))
    };
    b.check(envelope.checked_mul(64).ok_or_else(size)?)?;
    let basis: &Profile = match implicit_basis {
        Some(basis) => basis,
        None => profile.profile(),
    };
    if recipe == Some(AssignedRecipe::Box) {
        let semantic = photocraft_cms::xcf_color::assigned_effective_planning_work_ceiling(basis, photocraft_cms::xcf_color::Encoding::NonlinearProfile)
            .map_err(|e| match e {
                photocraft_cms::control::Error::Cancelled => E::Backing(BackingError::Cancelled),
                photocraft_cms::control::Error::Allocation => E::Allocation,
                photocraft_cms::control::Error::Limit(_) => E::Backing(BackingError::Quota),
                photocraft_cms::control::Error::Cms(e) => E::Color(CompositionError::Color(e)),
            })?;
        b.check(semantic.checked_mul(3).ok_or_else(size)?)?;
    }
    let ordered = OrderedStack::new_owned_neighborhood(stack, basis, SampleEncoding::NonlinearProfile, 22, extent, interrupt).map_err(E::Color)?;
    let retained = ordered.neighborhood_retained_bytes().map_err(E::Color)?.checked_add(65536).and_then(|v| v.checked_add(header)).ok_or_else(size)?;
    if retained > envelope {
        return Err(E::Allocation);
    }
    b.check(0)?;
    Ok((Plan { stack: ordered, _profile: profile, opaque: !finite }, lease))
}
fn space(v: GimpSpace) -> Result<SampleEncoding, E> {
    match v {
        GimpSpace::LinearRgb => Ok(SampleEncoding::Linear),
        GimpSpace::ProfileRgb => Ok(SampleEncoding::NonlinearProfile),
        GimpSpace::PerceptualRgb => Ok(SampleEncoding::Perceptual),
        _ => Err(scope()),
    }
}
impl<S, L> LeasedStockNeighborhoodPipeline<S, L> {
    pub fn new_budgeted<B: PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let header = std::mem::size_of::<Self>() as u64 + 128;
        let (plan, lease) = prepare(doc, None, header, interrupt, &mut Shared { budget }).map_err(|e| match e {
            E::Color(e) => e,
            E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
            _ => CompositionError::Unsupported("owned neighborhood preparation"),
        })?;
        Ok(Self { _plan: plan, _source: source_lease, _lease: lease })
    }
    /// Dedicated assigned-Box source preparation. Actual source/native admission
    /// remains the mapper's authority; this proves and retains the ICC relation.
    pub fn new_assigned_box_budgeted<B: PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let (plan, lease) =
            prepare_mode(doc, None, std::mem::size_of::<Self>() as u64 + 128, interrupt, &mut Shared { budget }, true, false).map_err(|e| match e {
                E::Color(e) => e,
                E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
                _ => CompositionError::Unsupported("assigned Box preparation"),
            })?;
        Ok(Self { _plan: plan, _source: source_lease, _lease: lease })
    }
    /// Exact assigned Median source preparation, without evaluating native pixels.
    /// Original admission remains sealed in the source mapper.
    pub fn new_assigned_median_budgeted<B: PreparationBudget<Lease = L>>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<Self, CompositionError> {
        let (plan, lease) =
            prepare_recipe(doc, None, std::mem::size_of::<Self>() as u64 + 128, interrupt, &mut Shared { budget }, Some(AssignedRecipe::Median), false)
                .map_err(|e| match e {
                    E::Color(e) => e,
                    E::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
                    _ => CompositionError::Unsupported("assigned Median preparation"),
                })?;
        Ok(Self { _plan: plan, _source: source_lease, _lease: lease })
    }
}
pub struct PreparedBackedNeighborhoodContext<'a> {
    derived: OwnedNeighborhoodSurface,
    prepared: RetainedPlan,
    owner: &'a AccountedDocument,
    layer: LayerId,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}
struct Materialized {
    surface: Surface,
    _lease: QuotaLease,
}
impl<'a> PreparedBackedNeighborhoodContext<'a> {
    pub fn new(owner: &'a AccountedDocument, profile: &Profile, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        Self::new_mode(owner, Some(profile), limits, c, false)
    }
    /// Assigned original/current profile relation is constructed from the actual
    /// immutable owner, never from a supplied standalone Profile or name/hash.
    pub fn new_assigned_box(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        Self::new_mode(owner, None, limits, c, true)
    }
    /// Exact assigned Median CURRENT context. It shares profile custody with
    /// Box but keeps its six-field encoded-pixel recipe independent.
    pub fn new_assigned_median(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        Self::new_recipe(owner, None, limits, c, Some(AssignedRecipe::Median))
    }
    /// Actual assigned-class dispatcher. A selected recipe failure is final;
    /// unknown operations never retry the implicit or another assigned lane.
    pub fn new_assigned(owner: &'a AccountedDocument, limits: NormalBandLimits, c: &ReadControl<'_>) -> Result<Self, E> {
        c.check(1).map_err(E::Backing)?;
        owner.validate_readonly_assigned_point_storage(c).map_err(E::Backing)?;
        let state = owner.document().xcf_state.as_ref().ok_or_else(scope)?;
        let [stack] = state.filter_stacks.as_slice() else { return Err(scope()) };
        let [node] = stack.nodes.as_slice() else { return Err(scope()) };
        if node.operation.name.len() > 128 {
            return Err(scope());
        }
        c.check(128).map_err(E::Backing)?;
        let recipe = match node.operation.name.as_str() {
            "gegl:box-blur" => AssignedRecipe::Box,
            "gegl:median-blur" => AssignedRecipe::Median,
            _ => return Err(scope()),
        };
        Self::new_recipe(owner, None, limits, c, Some(recipe))
    }
    fn new_mode(owner: &'a AccountedDocument, profile: Option<&Profile>, limits: NormalBandLimits, c: &ReadControl<'_>, assigned: bool) -> Result<Self, E> {
        Self::new_recipe(owner, profile, limits, c, if assigned { Some(AssignedRecipe::Box) } else { None })
    }
    fn new_recipe(
        owner: &'a AccountedDocument,
        profile: Option<&Profile>,
        limits: NormalBandLimits,
        c: &ReadControl<'_>,
        recipe: Option<AssignedRecipe>,
    ) -> Result<Self, E> {
        let assigned = recipe.is_some();
        c.check(1).map_err(E::Backing)?;
        let max = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > max.max_rows || limits.max_pixels == 0 || limits.max_pixels > max.max_pixels {
            return Err(scope());
        }
        if assigned { owner.validate_readonly_assigned_point_storage(c) } else { owner.validate_readonly_point_storage(c) }.map_err(E::Backing)?;
        let header = std::mem::size_of::<Self>() as u64 + 128;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(E::Quota)?;
        let (plan, plan_lease) = prepare_recipe(
            owner.document(),
            profile,
            std::mem::size_of::<Plan<QuotaLease>>() as u64 + 128,
            c.interrupt,
            &mut NativeBudget { q: owner.quota(), c },
            recipe,
            true,
        )?;
        let prepared = RetainedPlan { plan, _lease: plan_lease };
        let layer = owner.document().layers.first().ok_or_else(scope)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
        let format = backed.format();
        let bounds = backed.operative_bounds();
        let tile_bytes = 65536u64.checked_mul(format.bytes_per_pixel() as u64).ok_or_else(size)?;
        let envelope = (bounds.tiles().count() as u64)
            .checked_mul(tile_bytes.checked_mul(3).and_then(|v| v.checked_add(512)).ok_or_else(size)?)
            .and_then(|v| v.checked_add(512))
            .ok_or_else(size)?;
        let material_fee = owner.quota().reserve(QuotaKind::Resident, envelope).map_err(E::Quota)?;
        c.check(envelope).map_err(E::Backing)?;
        let mut input =
            Materialized { surface: Surface::try_with_default_bytes(format, backed.default_bytes()).map_err(|e| E::Color(e.into()))?, _lease: material_fee };
        let view = backed.checked_view();
        for coord in bounds.tiles() {
            let pin = view.pin_tile(coord, c).map_err(E::View)?;
            match pin.encoded() {
                NativeTileBytes::FullTile(bytes) => {
                    input.surface.try_write_interleaved_with(coord.rect(), bytes, c.interrupt).map_err(|e| E::Color(e.into()))?
                }
                NativeTileBytes::DefaultPixel(_) => {}
            }
        }
        if input.surface.resident_payload_bytes().map_err(|e| E::Color(e.into()))? > envelope {
            return Err(E::Allocation);
        }
        if assigned {
            for channel in 0..format.channels() {
                c.check(1).map_err(E::Backing)?;
                let value = photocraft_color::try_read_sample_f64(backed.default_bytes(), format.sample, channel)
                    .map_err(|_| E::Color(photocraft_raster::RasterInputError::InvalidSample.into()))?;
                if !value.is_finite() || (channel == format.channels() - 1 && !(0.0..=1.0).contains(&value)) {
                    return Err(scope());
                }
            }
        }
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                c.check(format.channels() as u64).map_err(E::Backing)?;
                let mut pixel = [0.0; 5];
                input.surface.try_read_pixel_f64(x, y, pixel.get_mut(..format.channels()).ok_or_else(size)?).map_err(|e| E::Color(e.into()))?;
                if pixel.iter().take(format.channels()).any(|v| !v.is_finite())
                    || !(0.0..=1.0).contains(pixel.get(format.channels() - 1).ok_or_else(size)?)
                    || prepared.plan.opaque && pixel[format.channels() - 1] != 1.0
                {
                    return Err(scope());
                }
            }
        }
        let derived = prepared.plan.stack.evaluate_owned(&input.surface, owner.quota(), c)?;
        c.check(0).map_err(E::Backing)?;
        Ok(Self { derived, prepared, owner, layer: layer.id, limits, _lease: lease })
    }
    pub fn bounds(&self) -> Rect {
        self.prepared.plan.stack.output_bounds()
    }
    pub fn layer_id(&self) -> LayerId {
        self.layer
    }
    pub fn layer_visible(&self) -> bool {
        self.owner.document().layers.first().is_some_and(|v| v.visible)
    }
    pub fn layer_opacity(&self) -> f64 {
        self.owner.document().layers.first().map_or(0., |v| f64::from(v.opacity))
    }
    pub fn validate_operative(&self, c: &ReadControl<'_>) -> Result<(), E> {
        for y in self.bounds().y0..self.bounds().y1 {
            for x in self.bounds().x0..self.bounds().x1 {
                c.check(4).map_err(E::Backing)?;
                let mut p = [0.; 4];
                self.derived.surface.try_read_pixel_f64(x, y, &mut p).map_err(|e| E::Color(e.into()))?;
                if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) {
                    return Err(scope());
                }
            }
        }
        Ok(())
    }
    pub fn render_wide_band(&self, rect: Rect, c: &ReadControl<'_>) -> Result<OwnedStockPointWideBand, E> {
        let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(size)?;
        if rect.is_empty() || !self.bounds().contains_rect(&rect) || rect.height() > self.limits.max_rows || count > self.limits.max_pixels {
            return Err(scope());
        }
        let cap = count.checked_mul(2).ok_or_else(size)?;
        let lease =
            self.owner.quota().reserve(QuotaKind::Resident, cap.checked_mul(32).and_then(|v| v.checked_add(512)).ok_or_else(size)?).map_err(E::Quota)?;
        c.check(count.checked_mul(4).ok_or_else(size)?).map_err(E::Backing)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(usize::try_from(count).map_err(|_| size())?).map_err(|_| E::Allocation)?;
        if pixels.capacity() as u64 > cap {
            return Err(E::Allocation);
        }
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                c.check(4).map_err(E::Backing)?;
                let mut p = [0.; 4];
                self.derived.surface.try_read_pixel_f64(x, y, &mut p).map_err(|e| E::Color(e.into()))?;
                pixels.push(p);
            }
        }
        c.check(0).map_err(E::Backing)?;
        Ok(OwnedStockPointWideBand::from_accounted_pixels(rect, pixels, lease))
    }
}

#[cfg(test)]
#[path = "backed_neighborhood_tests.rs"]
mod tests;
