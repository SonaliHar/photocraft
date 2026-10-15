//! CURRENT singleton stock Point recipe over a genuine readonly storage owner.
//! Original admission, native source storage and derived display are separate:
//! this retains the existing PointStack transfer and current-buffer schedule,
//! without certifying source correspondence, padding or a default import route.
use crate::gimp_render::PreparationBudget;
use crate::{
    backed_normal::{BackedNormalError, NormalBandLimits},
    gimp_composite::CompositionError,
    gimp_filters::{self},
    gimp_neighborhood::OrderedStack,
};
use photocraft_cms::{Builtin, Profile, WideWork};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
use photocraft_doc::{
    Document, LayerContent, LayerId, SampleEncoding, WorkingProfileInterpretation,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::{FilterId, ItemRef},
    xcf_filters::{FilterNode, FilterValue},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Interrupt;
use photocraft_raster::backing::{
    BackingError, NativePlaneDescriptor, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};

fn scope() -> BackedNormalError {
    BackedNormalError::Unsupported("closed current stock Point recipe")
}
fn length() -> BackedNormalError {
    BackedNormalError::Invalid("stock Point size or coordinate overflow")
}
trait PlanBudget {
    type Lease;
    fn reserve(&mut self, bytes: u64) -> Result<Self::Lease, BackedNormalError>;
    fn check(&mut self, units: u64) -> Result<(), BackedNormalError>;
}
struct NativeBudget<'a, 'c> {
    quota: &'a BackingQuota,
    control: &'a ReadControl<'c>,
}
impl PlanBudget for NativeBudget<'_, '_> {
    type Lease = QuotaLease;
    fn reserve(&mut self, bytes: u64) -> Result<QuotaLease, BackedNormalError> {
        self.quota.reserve(QuotaKind::Resident, bytes).map_err(BackedNormalError::Quota)
    }
    fn check(&mut self, units: u64) -> Result<(), BackedNormalError> {
        self.control.check(units).map_err(BackedNormalError::Backing)
    }
}
struct SharedBudget<'a, B> {
    budget: &'a mut B,
}
impl<B: PreparationBudget> PlanBudget for SharedBudget<'_, B> {
    type Lease = B::Lease;
    fn reserve(&mut self, bytes: u64) -> Result<B::Lease, BackedNormalError> {
        self.budget.reserve_prepared_bytes(bytes).map_err(BackedNormalError::Color)
    }
    fn check(&mut self, units: u64) -> Result<(), BackedNormalError> {
        self.budget.consume_preparation_work(units).map_err(BackedNormalError::Color)
    }
}
fn work(budget: &mut impl PlanBudget, interrupt: &Interrupt<'_>, mut units: u64) -> Result<(), BackedNormalError> {
    interrupt.check().map_err(|_| BackedNormalError::Backing(BackingError::Cancelled))?;
    budget.check(0)?;
    while units != 0 {
        let part = units.min(4096);
        budget.check(part)?;
        interrupt.check().map_err(|_| BackedNormalError::Backing(BackingError::Cancelled))?;
        units -= part;
    }
    Ok(())
}
fn bytes_equal(a: &[u8], b: &[u8], budget: &mut impl PlanBudget, interrupt: &Interrupt<'_>) -> Result<bool, BackedNormalError> {
    work(budget, interrupt, 1)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.chunks(4096).zip(b.chunks(4096)) {
        work(budget, interrupt, a.len() as u64)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Existing source-qualified Point family identity boundary. This does not replace the
/// authoritative PointStack version/argument/domain/Replace checks below.
fn initial_member(node: &FilterNode, budget: &mut impl PlanBudget, interrupt: &Interrupt<'_>) -> Result<(), BackedNormalError> {
    for text in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
        if text.len() > 4096 {
            return Err(scope());
        }
        work(budget, interrupt, text.len() as u64)?;
    }
    let epoch_ok = match node.operation.name.as_str() {
        "gimp:desaturate" => node.operation.schema_epoch == gimp_filters::DESATURATE_PRECISION_EPOCH,
        "gegl:saturation" => matches!(
            node.operation.schema_epoch.as_str(),
            gimp_filters::SATURATION_PRECISION_EPOCH | gimp_filters::SATURATION_GRAY_PRECISION_EPOCH | gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH
        ),
        "gegl:value-invert" => node.operation.schema_epoch == gimp_filters::VALUE_INVERT_PRECISION_EPOCH,
        "gimp:posterize" => node.operation.schema_epoch == "xcf-22-stock-point-replace",
        "gegl:brightness-contrast" => node.operation.schema_epoch == gimp_filters::BRIGHTNESS_CONTRAST_PRECISION_EPOCH,
        "gegl:levels" => node.operation.schema_epoch == gimp_filters::GEGL_LEVELS_PRECISION_EPOCH,
        "gegl:exposure" => node.operation.schema_epoch == gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH,
        "gegl:color-temperature" => node.operation.schema_epoch == gimp_filters::COLOR_TEMPERATURE_PRECISION_EPOCH,
        "gegl:sepia" => node.operation.schema_epoch == gimp_filters::SEPIA_PRECISION_EPOCH,
        "gegl:alien-map" => {
            matches!(node.operation.schema_epoch.as_str(), gimp_filters::ALIEN_MAP_PRECISION_EPOCH | gimp_filters::ALIEN_MAP_HSL_PRECISION_EPOCH)
        }
        "gegl:invert-linear" => node.operation.schema_epoch == gimp_filters::INVERT_LINEAR_PRECISION_EPOCH,
        "gegl:invert-gamma" => node.operation.schema_epoch == gimp_filters::INVERT_GAMMA_PRECISION_EPOCH,
        "gegl:absolute" => node.operation.schema_epoch == gimp_filters::ABSOLUTE_LINEAR_PRECISION_EPOCH,
        "gegl:color-exchange" => node.operation.schema_epoch == gimp_filters::COLOR_EXCHANGE_PRECISION_EPOCH,
        "gegl:color-overlay" => node.operation.schema_epoch == gimp_filters::COLOR_OVERLAY_PRECISION_EPOCH,
        "gegl:channel-mixer" => node.operation.schema_epoch == gimp_filters::CHANNEL_MIXER_PRECISION_EPOCH,
        "gegl:mono-mixer" => node.operation.schema_epoch == gimp_filters::MONO_MIXER_SIGNED_PRECISION_EPOCH,
        "gegl:red-eye-removal" => {
            matches!(node.operation.schema_epoch.as_str(), gimp_filters::RED_EYE_PRECISION_EPOCH | crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH)
        }
        "gegl:color-rotate" => node.operation.schema_epoch == crate::gimp_color_rotate::COLOR_ROTATE_PERCEPTUAL_EPOCH,
        "gegl:video-degradation" => matches!(
            node.operation.schema_epoch.as_str(),
            crate::gimp_video_degradation::VIDEO_DEGRADATION_EPOCH | crate::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH
        ),
        "gegl:color-to-alpha" => node.operation.schema_epoch == gimp_filters::COLOR_TO_ALPHA_PRECISION_EPOCH,
        "gegl:component-extract" => matches!(
            node.operation.schema_epoch.as_str(),
            gimp_filters::COMPONENT_EXTRACT_PRECISION_EPOCH
                | gimp_filters::COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH
                | gimp_filters::COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH
                | gimp_filters::COMPONENT_EXTRACT_GEOMETRY_EPOCH
        ),
        "gegl:alpha-clip" => {
            matches!(node.operation.schema_epoch.as_str(), gimp_filters::ALPHA_CLIP_PRECISION_EPOCH | gimp_filters::ALPHA_CLIP_FINITE_PRECISION_EPOCH)
        }
        _ => false,
    };
    let policy = node.composition.as_ref().ok_or_else(scope)?;
    if policy.compatibility_epoch.len() > 4096 || node.arguments.len() > 10 {
        return Err(scope());
    }
    work(budget, interrupt, policy.compatibility_epoch.len() as u64)?;
    let compatibility = match node.operation.schema_epoch.as_str() {
        crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH => crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH,
        crate::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH => crate::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH,
        gimp_filters::DESATURATE_PRECISION_EPOCH
        | gimp_filters::SATURATION_PRECISION_EPOCH
        | gimp_filters::SATURATION_GRAY_PRECISION_EPOCH
        | gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH
        | gimp_filters::VALUE_INVERT_PRECISION_EPOCH
        | "xcf-22-stock-point-replace" => node.operation.schema_epoch.as_str(),
        _ => gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH,
    };
    if !epoch_ok || policy.compatibility_epoch != compatibility {
        return Err(scope());
    }
    for argument in &node.arguments {
        if argument.name.len() > 4096 {
            return Err(scope());
        }
        work(budget, interrupt, argument.name.len() as u64 + 1)?;
        match &argument.value {
            FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_)) | FilterValue::Bool(_) => {}
            FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed { width: 32, .. }) if node.operation.name == "gimp:posterize" => {}
            FilterValue::Enum { domain, .. } if domain.len() <= 4096 => work(budget, interrupt, domain.len() as u64)?,
            FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None }
                if components.len() == 4 && components.iter().all(|v| matches!(v, photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))) =>
            {
                work(budget, interrupt, 4)?
            }
            _ => return Err(scope()),
        }
    }
    Ok(())
}

fn cms_error(error: photocraft_cms::control::Error) -> BackedNormalError {
    use photocraft_cms::control::Error;
    match error {
        Error::Cancelled => BackedNormalError::Backing(BackingError::Cancelled),
        Error::Allocation => BackedNormalError::Allocation,
        Error::Limit(_) => BackedNormalError::Backing(BackingError::Work),
        Error::Cms(error) => BackedNormalError::Color(CompositionError::Color(error)),
    }
}
fn bound_profile(
    doc: &Document,
    supplied: Option<&Profile>,
    budget: &mut impl PlanBudget,
    interrupt: &Interrupt<'_>,
) -> Result<(Profile, &'static Profile), BackedNormalError> {
    let current = doc.icc_profile.as_deref().ok_or_else(scope)?;
    if !(132..=65536).contains(&current.len()) {
        return Err(scope());
    }
    let (kind, space) = match (&doc.working_profile_interpretation, doc.mode) {
        (WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb) => (Builtin::Srgb, photocraft_cms::ColorSpace::Rgb),
        (WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale) => (Builtin::SGray, photocraft_cms::ColorSpace::Gray),
        _ => return Err(scope()),
    };
    let stored = doc.sample_encoding.ok_or_else(scope)?;
    work(budget, interrupt, current.len() as u64)?;
    let marker = doc.working_profile_interpretation.validate_with_interrupt(Some(current), doc.mode, interrupt);
    work(budget, interrupt, 0)?;
    marker.map_err(|e| if e.code == "cancelled" { BackedNormalError::Backing(BackingError::Cancelled) } else { scope() })?;
    // This is the existing generic Point nonlinear basis, distinct from the
    // actual current effective Linear/Perceptual encoded profile below.
    let known = kind.profile();
    work(budget, interrupt, 1)?;
    if let Some(supplied) = supplied {
        let encoded = supplied.encoded_bytes().ok_or(BackedNormalError::Unsupported("stock Point requires retained implicit basis encoding"))?;
        if supplied.channels() != doc.mode.color_channels() || !bytes_equal(known.encoded_bytes().ok_or_else(scope)?, encoded, budget, interrupt)? {
            return Err(scope());
        }
    }
    // Reuse the original accounted analytic-space envelope. Planning itself is
    // bounded by that real already-held slice and a prepaid lower work limit;
    // no default CMS allowance is reset between prospective/constructor phases.
    let planning_work = 65536u64.checked_mul(64).ok_or_else(length)?;
    work(budget, interrupt, planning_work)?;
    let limits = photocraft_cms::control::Limits { resident_bytes: 65536, work_units: planning_work };
    let cancelled = || interrupt.cancelled();
    let required = photocraft_cms::xcf_color::effective_implicit_profile_requirements_with_control(
        current,
        space,
        crate::gimp_composite::encoding(stored),
        limits,
        &cancelled,
    )
    .map_err(cms_error)?;
    // The controlled constructor repeats its own prospective and validation
    // passes. Keep all four bounded schedules prepaid through the real caller.
    let construction = required.work_units.checked_mul(4).ok_or_else(length)?;
    if required.peak_resident_bytes > limits.resident_bytes || construction > planning_work {
        return Err(scope());
    }
    work(budget, interrupt, construction)?;
    let effective = photocraft_cms::xcf_color::effective_implicit_profile_with_control(
        current,
        space,
        crate::gimp_composite::encoding(stored),
        photocraft_cms::control::Limits { resident_bytes: limits.resident_bytes, work_units: construction },
        &cancelled,
    )
    .map_err(cms_error)?;
    work(budget, interrupt, 0)?;
    Ok((effective, known))
}

/// Paid semantic preparation usable by the exact admitted IO family before its
/// storage owner exists. It provides a CURRENT recipe only: callers retain the
/// complete model/source/native backing and their independent original proof.
/// All heaps are destroyed before their real same-ledger preparation charge.
struct StockPointPlan {
    stack: OrderedStack,
    _current_profile: Profile,
    format: PixelFormat,
    opaque_input: bool,
}
pub struct PreparedStockPointPipeline {
    plan: StockPointPlan,
    quota: BackingQuota,
    _lease: QuotaLease,
}
/// Exact admitted IO caller retains both real source and preparation leases.
/// Generic inline headers are admitted at their actual monomorphized size.
/// There is no escaping bare plan or source-grant conversion.
pub struct LeasedStockPointPipeline<S, L> {
    _plan: StockPointPlan,
    _source: S,
    _lease: L,
}
fn prepare_plan<B: PlanBudget>(
    doc: &Document,
    supplied: Option<&Profile>,
    header: u64,
    interrupt: &Interrupt<'_>,
    budget: &mut B,
) -> Result<(StockPointPlan, B::Lease), BackedNormalError> {
    work(budget, interrupt, 1)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let [layer] = doc.layers.as_slice() else { return Err(scope()) };
    let [filters] = state.filter_stacks.as_slice() else { return Err(scope()) };
    let [node] = filters.nodes.as_slice() else { return Err(scope()) };
    if state.provenance.format_version != 22
        || state.items.len() != 2
        || state.filter_masks.len() != 1
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(doc.depth, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        || filters.owner != ItemRef::Layer(layer.id)
        || doc.source_color_context.as_ref().is_none_or(|source| source.original_icc.is_some())
    {
        return Err(scope());
    }
    initial_member(node, budget, interrupt)?;
    let (extent, format, default) = match &layer.content {
        LayerContent::BackedRaster(backed) => (backed.operative_bounds(), backed.format(), backed.default_bytes()),
        LayerContent::Raster(surface) => {
            let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
            (item.raster_bounds.ok_or_else(scope)?, surface.format(), surface.default_bytes())
        }
        _ => return Err(scope()),
    };
    let stored = doc.sample_encoding.ok_or_else(scope)?;
    // The genuine existing scalar preparation envelope includes actual
    // PointNode/argument storage and two 64KiB implicit analytic profile
    // envelopes (one WorkingSpace and the retained effective-profile preparation).
    // Header slack is explicit; it is not a replacement source/model fee.
    let scalar = gimp_filters::scalar_preparation_bytes(1, node.arguments.len())
        .and_then(|n| n.checked_add(crate::gimp_neighborhood::scalar_preparation_bytes(1)?).ok_or(CompositionError::Invalid("stock stage preparation")))
        .map_err(BackedNormalError::Color)?;
    // Four additional existing analytic-space envelopes cover the scoped
    // candidate's cloned profile/effective-profile/paired transform preparation.
    // This is the same bounded implicit candidate class used by accounted
    // Overlay/Rotate preparation; arbitrary profiles/LUTs remain excluded.
    let extra_spaces = if matches!(
        node.operation.name.as_str(),
        "gegl:absolute"
            | "gegl:channel-mixer"
            | "gegl:mono-mixer"
            | "gegl:red-eye-removal"
            | "gegl:color-exchange"
            | "gegl:color-to-alpha"
            | "gegl:component-extract"
            | "gegl:alpha-clip"
            | "gegl:color-overlay"
            | "gegl:color-rotate"
    ) {
        4
    } else {
        0
    };
    let envelope = scalar.checked_add((2 + extra_spaces) * 65536).and_then(|n| n.checked_add(header)).ok_or_else(length)?;
    let lease = budget.reserve(envelope)?;
    let (current_profile, known) = bound_profile(doc, supplied, budget, interrupt)?;
    // Price bounded repeated string/registry comparisons and the admitted
    // fixed analytic profile preparation before PointStack allocates.
    work(budget, interrupt, envelope.checked_mul(64).ok_or_else(length)?)?;
    crate::gimp_render::qualify_current_point_layer_coverage_metadata(doc, layer, state, filters, extent, stored, format, default)
        .map_err(BackedNormalError::Color)?;
    let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
    let resolved = item.composition.as_ref().ok_or_else(scope)?.checked_resolved().map_err(|_| scope())?;
    let physical = layer.import_compositing.as_ref().ok_or_else(scope)?;
    if layer.blend != photocraft_color::blend::BlendMode::Normal
        || layer.fill_opacity.to_bits() != 1f32.to_bits()
        || layer.clipped
        || layer.excluded_channels != 0
        || layer.blend_if != Default::default()
        || layer.advanced != Default::default()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || !layer.psd_blocks.is_empty()
        || resolved.operator != GimpOperator::Union
        || physical.pass_through
        || physical.blend_encoding != encoding(resolved.blend_space)?
        || physical.composite_encoding != encoding(resolved.composite_space)?
    {
        return Err(scope());
    }
    let mask = state.filter_masks.first().ok_or_else(scope)?;
    let physical_pixels = (mask.surface.tile_count() as u64)
        .checked_mul(TILE_SIZE as u64)
        .and_then(|n| n.checked_mul(TILE_SIZE as u64))
        .and_then(|n| n.checked_add(1))
        .ok_or_else(length)?;
    let logical = u64::from(mask.bounds.width()).checked_mul(u64::from(mask.bounds.height())).ok_or_else(length)?;
    work(budget, interrupt, physical_pixels.max(logical).checked_mul(32).ok_or_else(length)?)?;
    // Reuse the exact old canvas-relative mask/default/full-tile/metadata
    // qualifier, including enabled/disabled mask-state semantics. Native
    // tile padding and signed zero are never normalized into an empty mask.
    let valid_mask = crate::gimp_neighborhood::qualify_selection_mask_with_interrupt(state, node, layer.id, doc.bounds(), interrupt);
    work(budget, interrupt, 0)?;
    valid_mask.map_err(BackedNormalError::Color)?;
    crate::gimp_render::qualify_residual_point_metadata(doc, layer, state, filters, extent, stored, interrupt).map_err(BackedNormalError::Color)?;
    crate::gimp_render::qualify_point_family_context(doc, layer, state, filters, extent, known, stored, interrupt).map_err(BackedNormalError::Color)?;
    let stack = OrderedStack::new_for_stock(filters, known, stored, state.provenance.format_version, extent, interrupt)
        .map_err(BackedNormalError::Color)?
        .with_current_point_storage(Some(format.sample))
        .with_filter_output_precision(state.current_filter_output_precision);
    let retained = stack.stock_retained_bytes().map_err(BackedNormalError::Color)?.checked_add(header).and_then(|n| n.checked_add(65536)).ok_or_else(length)?;
    if retained > envelope {
        return Err(BackedNormalError::Allocation);
    }
    // Keep the entire admitted peak for the pipeline lifetime; no phase
    // refund can expose headroom while this owned plan remains live.
    let plan = StockPointPlan { stack, _current_profile: current_profile, format, opaque_input: node.operation.name == "gimp:posterize" };
    work(budget, interrupt, 0)?;
    Ok((plan, lease))
}
fn preparation_error(error: BackedNormalError) -> CompositionError {
    match error {
        BackedNormalError::Color(error) => error,
        BackedNormalError::Backing(BackingError::Cancelled) => photocraft_raster::RasterInputError::Cancelled.into(),
        BackedNormalError::Allocation | BackedNormalError::Backing(BackingError::Allocation) => photocraft_raster::RasterInputError::AllocationFailed.into(),
        BackedNormalError::Backing(_) | BackedNormalError::Quota(_) => photocraft_raster::RasterInputError::LimitExceeded.into(),
        _ => CompositionError::Unsupported("closed stock Point preparation"),
    }
}
impl PreparedStockPointPipeline {
    /// `supplied` is the exact analytical NONLINEAR implicit basis. The current
    /// encoded ICC is independently reconstructed for doc.sample_encoding and
    /// retained in this paid plan; a Linear effective profile is not the basis.
    pub fn prepare(doc: &Document, supplied: &Profile, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        let header = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(length)?;
        if let Some(layer) = doc.layers.first() {
            if let LayerContent::BackedRaster(backed) = &layer.content {
                if !quota.shares_ledger(backed.quota()) {
                    return Err(BackedNormalError::Backing(BackingError::Quota));
                }
            }
        }
        let (plan, lease) = prepare_plan(doc, Some(supplied), header, control.interrupt, &mut NativeBudget { quota, control })?;
        Ok(Self { plan, quota: quota.clone(), _lease: lease })
    }
    /// No external ICC parse is required: this existing generic Point policy
    /// uses the exact implicit basis and controlled effective-profile binding.
    pub fn new_budgeted<B: PreparationBudget, S>(
        doc: &Document,
        interrupt: &Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedStockPointPipeline<S, B::Lease>, CompositionError> {
        let header = std::mem::size_of::<LeasedStockPointPipeline<S, B::Lease>>()
            .checked_add(128)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(CompositionError::Invalid("stock Point generic header"))?;
        let (plan, lease) = prepare_plan(doc, None, header, interrupt, &mut SharedBudget { budget }).map_err(preparation_error)?;
        Ok(LeasedStockPointPipeline { _plan: plan, _source: source_lease, _lease: lease })
    }
    pub(crate) fn format(&self) -> PixelFormat {
        self.plan.format
    }
    pub(crate) fn shares_quota(&self, quota: &BackingQuota) -> bool {
        self.quota.shares_ledger(quota)
    }
    /// Exact input decode + existing PointStack wide current-buffer schedule.
    /// The shared sealed native-input executor invokes this with its actual
    /// request control; no callback or fresh work allowance is retained here.
    #[cfg(test)]
    pub(crate) fn evaluate_native(&self, raw: &[u8], format: PixelFormat, control: &ReadControl<'_>) -> Result<[f64; 4], BackedNormalError> {
        self.evaluate_native_at(raw, format, (0, 0), control)
    }
    pub(crate) fn evaluate_native_at(
        &self,
        raw: &[u8],
        format: PixelFormat,
        position: (i32, i32),
        control: &ReadControl<'_>,
    ) -> Result<[f64; 4], BackedNormalError> {
        control.check(format.bytes_per_pixel() as u64 + 1).map_err(BackedNormalError::Backing)?;
        if format != self.plan.format || raw.len() != format.bytes_per_pixel() {
            return Err(scope());
        }
        let read = |channel| try_read_sample_f64(raw, format.sample, channel).map_err(|_| length());
        let rgb = if format.mode == ColorMode::Grayscale { [read(0)?; 3] } else { [read(0)?, read(1)?, read(2)?] };
        let input = [rgb[0], rgb[1], rgb[2], read(format.mode.color_channels())?];
        if self.plan.opaque_input && input[3] != 1.0 {
            return Err(BackedNormalError::Unsupported("Posterize opaque current input"));
        }
        let mut failed = None;
        let mut temporary: Option<QuotaLease> = None;
        let mut poll = |event| {
            let result = match event {
                WideWork::SampleUnits(units) => control.check(units).map_err(BackedNormalError::Backing),
                WideWork::ReserveBytes(bytes) if temporary.is_none() => {
                    self.quota.reserve(QuotaKind::Resident, bytes).map(|lease| temporary = Some(lease)).map_err(BackedNormalError::Quota)
                }
                WideWork::ReleaseBytes(bytes) if temporary.as_ref().is_some_and(|lease| lease.bytes() == bytes) => {
                    temporary = None;
                    Ok(())
                }
                _ => Err(BackedNormalError::Invalid("stock Point temporary ownership protocol")),
            };
            if let Err(error) = result {
                failed = Some(error);
                return Err(CompositionError::Invalid("stock Point caller control refused"));
            }
            Ok(())
        };
        let validated = self.plan.stack.validate_stock_input(position, input, control.interrupt, &mut poll);
        let evaluated = match validated {
            Ok(()) => self.plan.stack.point_at(input, position, &mut poll),
            Err(error) => Err(error),
        };
        if let Some(error) = failed {
            return Err(error);
        }
        let output = evaluated.map_err(BackedNormalError::Color)?;
        if temporary.is_some() {
            return Err(BackedNormalError::Invalid("stock Point temporary owner retained"));
        }
        if output.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&output[3]) {
            return Err(BackedNormalError::Invalid("stock Point generated finite color/unit alpha"));
        }
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(output)
    }
}

/// Tightly packed straight RGBA64 operative result in the current stored
/// encoding. This is derived display arithmetic, not native source storage.
pub struct OwnedStockPointWideBand {
    rect: Rect,
    pixels: Vec<[f64; 4]>,
    _lease: QuotaLease,
}
impl OwnedStockPointWideBand {
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn pixels(&self) -> &[[f64; 4]] {
        &self.pixels
    }
    pub fn resident_bytes(&self) -> u64 {
        self._lease.bytes()
    }
    pub(crate) fn from_accounted_pixels(rect: Rect, pixels: Vec<[f64; 4]>, lease: QuotaLease) -> Self {
        Self { rect, pixels, _lease: lease }
    }
}

pub struct PreparedBackedStockPointContext<'a> {
    pipeline: PreparedStockPointPipeline,
    owner: &'a AccountedDocument,
    layer: LayerId,
    node: FilterId,
    descriptor: NativePlaneDescriptor,
    limits: NormalBandLimits,
    visible: bool,
    opacity: f64,
    _lease: QuotaLease,
}
fn encoding(space: GimpSpace) -> Result<SampleEncoding, BackedNormalError> {
    match space {
        GimpSpace::LinearRgb => Ok(SampleEncoding::Linear),
        GimpSpace::ProfileRgb => Ok(SampleEncoding::NonlinearProfile),
        GimpSpace::PerceptualRgb => Ok(SampleEncoding::Perceptual),
        GimpSpace::Lab => Err(scope()),
    }
}
impl<'a> PreparedBackedStockPointContext<'a> {
    /// The enclosing current interpreter supplies the nonlinear implicit basis
    /// (Srgb or SGray); actual current ICC/marker/encoding are separately bound
    /// under the real preparation charge. This establishes no source identity.
    pub fn new(owner: &'a AccountedDocument, profile: &Profile, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let maximum = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > maximum.max_rows || limits.max_pixels == 0 || limits.max_pixels > maximum.max_pixels {
            return Err(scope());
        }
        owner.validate_readonly_point_storage(control).map_err(BackedNormalError::Backing)?;
        let doc = owner.document();
        let layer = doc.layers.first().ok_or_else(scope)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
        let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
        let filters = state.filter_stacks.first().ok_or_else(scope)?;
        let node = filters.nodes.first().ok_or_else(scope)?;
        let header = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(length)?;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(BackedNormalError::Quota)?;
        let pipeline = PreparedStockPointPipeline::prepare(doc, profile, owner.quota(), control)?;
        let result = Self {
            pipeline,
            owner,
            layer: layer.id,
            node: node.id,
            descriptor: backed.descriptor(),
            limits,
            visible: layer.visible,
            opacity: f64::from(layer.opacity),
            _lease: lease,
        };
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(result)
    }
    pub fn layer_id(&self) -> LayerId {
        self.layer
    }
    pub fn node_id(&self) -> FilterId {
        self.node
    }
    pub fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    pub fn layer_visible(&self) -> bool {
        self.visible
    }
    pub fn layer_opacity(&self) -> f64 {
        self.opacity
    }
    pub fn render_wide_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedStockPointWideBand, BackedNormalError> {
        let layer = self.owner.document().layers.first().ok_or_else(scope)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return Err(scope()) };
        crate::backed_point::render_stock_wide_band(backed, self.descriptor, self.limits, &self.pipeline, rect, control)
    }
    /// Complete operative validation retains at most one lowered band. Raw
    /// off-plane tile padding is retained but is not certified by this recipe.
    pub fn validate_operative(&self, control: &ReadControl<'_>) -> Result<(), BackedNormalError> {
        let bounds = self.descriptor.bounds();
        let columns = u64::from(bounds.width()).min(self.limits.max_pixels).min(TILE_SIZE as u64);
        let rows = self.limits.max_pixels.checked_div(columns).ok_or_else(length)?.min(u64::from(self.limits.max_rows));
        let mut y = bounds.y0;
        while y < bounds.y1 {
            let end_y = i64::from(y).checked_add(rows as i64).ok_or_else(length)?.min(i64::from(bounds.y1));
            let end_y = i32::try_from(end_y).map_err(|_| length())?;
            let mut x = bounds.x0;
            while x < bounds.x1 {
                control.check(1).map_err(BackedNormalError::Backing)?;
                let end_x = i64::from(x).checked_add(columns as i64).ok_or_else(length)?.min(i64::from(bounds.x1));
                let end_x = i32::try_from(end_x).map_err(|_| length())?;
                if end_x <= x || end_y <= y {
                    return Err(length());
                }
                drop(self.render_wide_band(Rect::new(x, y, end_x, end_y), control)?);
                x = end_x;
            }
            y = end_y;
        }
        control.check(0).map_err(BackedNormalError::Backing)
    }
}

#[cfg(test)]
#[path = "backed_stock_point_context_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "backed_stock_point_general_tests.rs"]
mod general_tests;

#[cfg(test)]
#[path = "backed_stock_point_residual_tests.rs"]
mod residual_tests;
