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

#[path = "backed_stock_point_empty.rs"]
mod empty;
#[path = "backed_stock_point_root.rs"]
pub(crate) mod root;
#[path = "backed_stock_point_hue_chroma.rs"]
mod owned_hue;
#[path = "backed_stock_point_vibrance.rs"]
mod owned_vibrance;
#[path = "backed_stock_point_loaded_exposure.rs"]
mod loaded_exposure;
#[path = "backed_stock_point_masked_exposure.rs"]
mod masked_exposure;
#[path = "backed_stock_point_masked_scalar.rs"]
mod masked_scalar;

#[derive(Clone, Copy)]
enum Recipe {
    Empty,
    Filtered,
}

fn scope() -> BackedNormalError {
    BackedNormalError::Unsupported("closed current stock Point recipe")
}
fn length() -> BackedNormalError {
    BackedNormalError::Invalid("stock Point size or coordinate overflow")
}
pub(crate) trait PlanBudget {
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

#[derive(Clone, Copy)]
enum ConfiguredKind {
    Balance,
    Levels,
    Curves,
}
fn configured_kind(node: &FilterNode) -> Option<ConfiguredKind> {
    match node.operation.name.as_str() {
        "gimp:color-balance" if matches!(node.operation.schema_epoch.as_str(), crate::gimp_color_balance::EPOCH | crate::gimp_color_balance::DEPTH_EPOCH) => {
            Some(ConfiguredKind::Balance)
        }
        "gimp:levels" if node.operation.schema_epoch == "xcf-22-stock-levels-replace" => Some(ConfiguredKind::Levels),
        "gimp:curves" if node.operation.schema_epoch == "xcf-22-stock-curves-replace" => Some(ConfiguredKind::Curves),
        _ => None,
    }
}
fn legacy_opaque(node: &FilterNode) -> bool {
    matches!(
        (node.operation.name.as_str(), node.operation.schema_epoch.as_str()),
        ("gimp:posterize", "xcf-22-stock-point-replace")
            | ("gegl:value-invert", "xcf-22-stock-point-replace")
            | ("gegl:mono-mixer", "xcf-22-stock-point-replace")
            | ("gegl:mono-mixer", gimp_filters::MONO_MIXER_EDITS_EPOCH)
    )
}
fn remaining_optional_mask(node: &FilterNode) -> bool {
    (legacy_opaque(node) && node.operation.name != "gimp:posterize")
        || (node.operation.name == "gegl:rgb-clip" && node.operation.schema_epoch == gimp_filters::RGB_CLIP_HDR_EPOCH)
}
fn configured_arguments(
    arguments: &[photocraft_doc::xcf_filters::FilterArgument],
    depth: u8,
    budget: &mut impl PlanBudget,
    interrupt: &Interrupt<'_>,
) -> Result<(), BackedNormalError> {
    if depth > 3 || arguments.len() > 5 {
        return Err(scope());
    }
    for argument in arguments {
        if argument.name.len() > 4096 {
            return Err(scope());
        }
        work(budget, interrupt, argument.name.len() as u64 + 1)?;
        match &argument.value {
            FilterValue::Bool(_) | FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float64Bits(_)) => {}
            FilterValue::Enum { domain, .. } if depth == 0 && domain.len() <= 4096 => work(budget, interrupt, domain.len() as u64)?,
            FilterValue::Config(fields) => configured_arguments(fields, depth + 1, budget, interrupt)?,
            FilterValue::Curve { free: true, points_f64_bits, point_types, samples_f64_bits }
                if points_f64_bits.is_empty() && point_types.is_empty() && samples_f64_bits.len() == 256 =>
            {
                for block in samples_f64_bits.chunks(64) {
                    work(budget, interrupt, block.len() as u64 * 8)?;
                }
            }
            _ => return Err(scope()),
        }
    }
    Ok(())
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
    let epoch_ok = (node.operation.schema_epoch == gimp_filters::STACK_EDITS_EPOCH
        && matches!(node.operation.name.as_str(), "gegl:exposure" | "gegl:invert-linear"))
        || configured_kind(node).is_some()
        || match node.operation.name.as_str() {
            "gimp:desaturate" => node.operation.schema_epoch == gimp_filters::DESATURATE_PRECISION_EPOCH,
            "gegl:saturation" => matches!(
                node.operation.schema_epoch.as_str(),
                gimp_filters::SATURATION_PRECISION_EPOCH | gimp_filters::SATURATION_GRAY_PRECISION_EPOCH | gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH
            ),
            "gegl:value-invert" => matches!(node.operation.schema_epoch.as_str(), gimp_filters::VALUE_INVERT_PRECISION_EPOCH | "xcf-22-stock-point-replace"),
            "gegl:rgb-clip" => node.operation.schema_epoch == gimp_filters::RGB_CLIP_HDR_EPOCH,
            "gegl:unpremultiply" => node.operation.schema_epoch == crate::gimp_unpremultiply::UNPREMULTIPLY_EPOCH,
            "gimp:posterize" => node.operation.schema_epoch == "xcf-22-stock-point-replace",
            "gegl:brightness-contrast" => node.operation.schema_epoch == gimp_filters::BRIGHTNESS_CONTRAST_PRECISION_EPOCH,
            "gegl:levels" => node.operation.schema_epoch == gimp_filters::GEGL_LEVELS_PRECISION_EPOCH,
            "gegl:exposure" => {
                matches!(node.operation.schema_epoch.as_str(), gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH | gimp_filters::EXPOSURE_GEOMETRY_EPOCH | crate::gimp_exposure::LOADED_POINT_EPOCH)
            }
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
            "gimp:semi-flatten" => node.operation.schema_epoch == gimp_filters::SEMI_FLATTEN_EPOCH,
            "gegl:hue-chroma" => node.operation.schema_epoch == crate::gimp_hue_chroma::EPOCH,
            "gegl:vibrance" => node.operation.schema_epoch == crate::gimp_vibrance::EPOCH,
            "gegl:channel-mixer" => node.operation.schema_epoch == gimp_filters::CHANNEL_MIXER_PRECISION_EPOCH,
            "gegl:mono-mixer" => matches!(
                node.operation.schema_epoch.as_str(),
                gimp_filters::MONO_MIXER_SIGNED_PRECISION_EPOCH | gimp_filters::MONO_MIXER_EDITS_EPOCH | "xcf-22-stock-point-replace"
            ),
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
        crate::gimp_exposure::LOADED_POINT_EPOCH
        | gimp_filters::EXPOSURE_GEOMETRY_EPOCH
        | gimp_filters::RGB_CLIP_HDR_EPOCH
        | gimp_filters::MONO_MIXER_EDITS_EPOCH
        | crate::gimp_unpremultiply::UNPREMULTIPLY_EPOCH => node.operation.schema_epoch.as_str(),
        gimp_filters::SEMI_FLATTEN_EPOCH => gimp_filters::SEMI_FLATTEN_EPOCH,
        crate::gimp_hue_chroma::EPOCH => crate::gimp_hue_chroma::EPOCH,
        crate::gimp_vibrance::EPOCH => crate::gimp_vibrance::EPOCH,
        gimp_filters::STACK_EDITS_EPOCH => gimp_filters::STACK_EDITS_EPOCH,
        crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH => crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH,
        crate::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH => crate::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH,
        gimp_filters::DESATURATE_PRECISION_EPOCH
        | gimp_filters::SATURATION_PRECISION_EPOCH
        | gimp_filters::SATURATION_GRAY_PRECISION_EPOCH
        | gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH
        | gimp_filters::VALUE_INVERT_PRECISION_EPOCH
        | "xcf-22-stock-point-replace" => node.operation.schema_epoch.as_str(),
        crate::gimp_color_balance::EPOCH | crate::gimp_color_balance::DEPTH_EPOCH | "xcf-22-stock-levels-replace" | "xcf-22-stock-curves-replace" => {
            node.operation.schema_epoch.as_str()
        }
        _ => gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH,
    };
    if !epoch_ok || policy.compatibility_epoch != compatibility {
        return Err(scope());
    }
    if configured_kind(node).is_some() {
        return configured_arguments(&node.arguments, 0, budget, interrupt);
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
            FilterValue::Color { components, encoding: SampleEncoding::Linear, profile: None }
                if crate::gimp_semi_flatten::claims(node)
                    && components.len() == 4
                    && components.iter().all(|v| matches!(v, photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))) =>
            {
                work(budget, interrupt, 4)?
            }
            _ => return Err(scope()),
        }
    }
    Ok(())
}

fn qualify_witnessed_metadata(
    doc: &Document,
    layer: &photocraft_doc::Layer,
    state: &photocraft_doc::XcfState,
    filters: &photocraft_doc::xcf_filters::FilterStack,
    extent: Rect,
    stored: SampleEncoding,
    format: PixelFormat,
    default: &[u8],
) -> Result<(), BackedNormalError> {
    if doc.size != photocraft_doc::Size::new(8, 6)
        || extent != Rect::new(0, 0, 8, 6)
        || doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::U8
        || stored != SampleEncoding::NonlinearProfile
        || format != PixelFormat::RGBA8
        || default.iter().any(|byte| *byte != 0)
        || !layer.visible
        || layer.opacity.to_bits() != 1f32.to_bits()
        || layer.mask.is_some()
        || doc.source_color_context.as_ref().is_none_or(|source| source.source_precision != 150 || source.original_icc.is_some())
        || state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).is_none_or(|item| {
            item.visible != layer.visible
                || item.raster_bounds != Some(extent)
                || item.composition.as_ref().is_none_or(|p| {
                    p.source_mode != 28 || (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (0, 2, 1) || p.legacy || p.pass_through
                })
        })
    {
        return Err(scope());
    }
    for (index, node) in filters.nodes.iter().enumerate() {
        if node.id.0 == 0 || filters.nodes.iter().take(index).any(|earlier| earlier.id == node.id) {
            return Err(scope());
        }
        match node.mask {
            None => {}
            Some(ItemRef::Mask(id)) => {
                if filters.nodes.iter().take(index).any(|earlier| earlier.mask == node.mask)
                    || state.filter_masks.iter().filter(|mask| mask.id == id && mask.owner == node.id).count() != 1
                {
                    return Err(scope());
                }
            }
            _ => return Err(scope()),
        }
    }
    for mask in &state.filter_masks {
        if mask.bounds != doc.bounds()
            || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)
            || filters.nodes.iter().filter(|node| node.id == mask.owner && node.mask == Some(ItemRef::Mask(mask.id))).count() != 1
            || state.items.iter().filter(|item| item.item == ItemRef::Mask(mask.id)).count() != 1
        {
            return Err(scope());
        }
    }
    Ok(())
}

fn qualify_remaining_metadata(
    doc: &Document,
    layer: &photocraft_doc::Layer,
    state: &photocraft_doc::XcfState,
    node: &FilterNode,
    extent: Rect,
    stored: SampleEncoding,
) -> Result<(), BackedNormalError> {
    let exposure = node.operation.name == "gegl:exposure" && node.operation.schema_epoch == gimp_filters::EXPOSURE_GEOMETRY_EPOCH;
    let hdr = node.operation.name == "gegl:rgb-clip";
    let unpremultiply = node.operation.name == "gegl:unpremultiply";
    let opaque = legacy_opaque(node) && node.operation.name != "gimp:posterize";
    if !(exposure || hdr || unpremultiply || opaque) {
        return Ok(());
    }
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    let valid = if exposure {
        doc.size == photocraft_doc::Size::new(295, 160)
            && extent.width() == 257
            && extent.height() == 129
            && matches!((extent.x0, extent.y0), (-13, 13) | (17, -11))
            && matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
            && matches!(doc.depth, SampleType::U16 | SampleType::F32 | SampleType::F64)
            && matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile)
    } else if opaque {
        doc.size == photocraft_doc::Size::new(32, 32)
            && extent == Rect::new(0, 0, 32, 32)
            && doc.mode == ColorMode::Rgb
            && doc.depth == SampleType::U8
            && stored == SampleEncoding::NonlinearProfile
            && source.source_precision == 150
    } else {
        doc.size == photocraft_doc::Size::new(8, 6)
            && extent == Rect::new(0, 0, 8, 6)
            && doc.mode == ColorMode::Rgb
            && doc.depth == SampleType::F32
            && stored == SampleEncoding::Linear
            && source.source_precision == 600
    };
    if !valid || !layer.visible || layer.opacity.to_bits() != 1f32.to_bits() {
        return Err(scope());
    }
    for mask in &state.filter_masks {
        if node.mask != Some(ItemRef::Mask(mask.id))
            || mask.owner != node.id
            || mask.bounds != doc.bounds()
            || state.items.iter().filter(|item| item.item == ItemRef::Mask(mask.id)).count() != 1
        {
            return Err(scope());
        }
    }
    Ok(())
}

fn qualify_configured_metadata(doc: &Document, node: &FilterNode, extent: Rect, stored: SampleEncoding) -> Result<(), CompositionError> {
    let Some(kind) = configured_kind(node) else { return Ok(()) };
    let valid = match kind {
        ConfiguredKind::Curves => {
            doc.size == photocraft_doc::Size::new(12, 9)
                && extent == Rect::new(0, 0, 12, 9)
                && doc.depth == SampleType::U8
                && stored == SampleEncoding::NonlinearProfile
        }
        ConfiguredKind::Levels => {
            doc.size == photocraft_doc::Size::new(12, 9)
                && extent == Rect::new(0, 0, 12, 9)
                && doc.depth == SampleType::F32
                && matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile)
        }
        ConfiguredKind::Balance => {
            let depth = node.operation.schema_epoch == crate::gimp_color_balance::DEPTH_EPOCH;
            let base = matches!(doc.depth, SampleType::U16 | SampleType::F32 | SampleType::F64)
                && matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile);
            let extra = doc.source_color_context.as_ref().is_some_and(|source| {
                matches!(
                    (source.source_precision, doc.depth, stored),
                    (100, SampleType::U8, SampleEncoding::Linear)
                        | (150, SampleType::U8, SampleEncoding::NonlinearProfile)
                        | (175, SampleType::U8, SampleEncoding::Perceptual)
                        | (300, SampleType::U32, SampleEncoding::Linear)
                        | (350, SampleType::U32, SampleEncoding::NonlinearProfile)
                        | (375, SampleType::U32, SampleEncoding::Perceptual)
                        | (500, SampleType::F32, SampleEncoding::Linear)
                        | (550, SampleType::F32, SampleEncoding::NonlinearProfile)
                        | (575 | 675, SampleType::F32, SampleEncoding::Perceptual)
                        | (275, SampleType::U16, SampleEncoding::Perceptual)
                        | (775, SampleType::F64, SampleEncoding::Perceptual)
                )
            });
            crate::gimp_render::residual_origin_extent(extent) && extent == doc.bounds() && (if depth { extra } else { base })
        }
    };
    if !valid
        || doc.mode != ColorMode::Rgb
        || (matches!(kind, ConfiguredKind::Balance) && doc.layers.first().is_none_or(|layer| !layer.visible || layer.opacity.to_bits() != 1f32.to_bits()))
    {
        return Err(CompositionError::Unsupported("closed configured stock current raster context"));
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
pub(crate) fn bound_profile(
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
enum StockPointProgram {
    Ordered(OrderedStack),
    LoadedExposure { candidate: crate::gimp_exposure::PreparedExposure, enabled: bool },
    MaskedExposure { candidate: crate::gimp_masked_exposure::PreparedMaskedExposure, mask: photocraft_raster::Surface, extent: Rect, enabled: bool },
    MaskedScalar { candidate: crate::gimp_masked_scalar::PreparedMaskedScalar, mask: photocraft_raster::Surface, extent: Rect, enabled: bool },
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PreparationPurpose { Source, Current }
struct StockPointPlan {
    recipe: Recipe,
    stack: StockPointProgram,
    _current_profile: Profile,
    format: PixelFormat,
    opaque_input: bool,
    half_input: bool,
    unit_input: bool,
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
    purpose: PreparationPurpose,
    header: u64,
    interrupt: &Interrupt<'_>,
    budget: &mut B,
) -> Result<(StockPointPlan, B::Lease), BackedNormalError> {
    work(budget, interrupt, 1)?;
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let [layer] = doc.layers.as_slice() else { return Err(scope()) };
    let [filters] = state.filter_stacks.as_slice() else { return Err(scope()) };
    if filters.nodes.is_empty() {
        return empty::prepare(doc, supplied, header, interrupt, budget);
    }
    let witnessed = matches!(filters.nodes.len(), 1..=2) && filters.nodes.iter().all(|node| node.operation.schema_epoch == gimp_filters::STACK_EDITS_EPOCH);
    if !witnessed && filters.nodes.len() != 1 {
        return Err(scope());
    }
    let node = filters.nodes.first().ok_or_else(scope)?;
    if node.operation.schema_epoch == crate::gimp_exposure::LOADED_POINT_EPOCH && (state.items.len()!=2 || state.filter_masks.len()!=1) { return Err(scope()); }
    if state.provenance.format_version != 22
        || state.filter_masks.len().checked_add(1) != Some(state.items.len())
        || (if witnessed || remaining_optional_mask(node) { state.filter_masks.len() > filters.nodes.len() } else { state.filter_masks.len() != 1 })
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
    if crate::gimp_masked_scalar::claims_epoch(node.operation.schema_epoch.as_str()) {
        if state.items.len()!=2 || state.filter_masks.len()!=1 { return Err(scope()); }
        let (extent,format,default) = match &layer.content {
            LayerContent::BackedRaster(backed)=>(backed.operative_bounds(),backed.format(),backed.default_bytes()),
            LayerContent::Raster(surface)=>{
                let item=state.items.iter().find(|i|i.item==ItemRef::Layer(layer.id)).ok_or_else(scope)?;
                (item.raster_bounds.ok_or_else(scope)?,surface.format(),surface.default_bytes())
            }
            _=>return Err(scope()),
        };
        return masked_scalar::prepare(doc,supplied,purpose,header,extent,format,default,interrupt,budget);
    }
    if node.operation.schema_epoch == crate::gimp_masked_exposure::EPOCH {
        let (extent,format,default) = match &layer.content {
            LayerContent::BackedRaster(backed)=>(backed.operative_bounds(),backed.format(),backed.default_bytes()),
            LayerContent::Raster(surface)=>{
                let item=state.items.iter().find(|i|i.item==ItemRef::Layer(layer.id)).ok_or_else(scope)?;
                (item.raster_bounds.ok_or_else(scope)?,surface.format(),surface.default_bytes())
            }
            _=>return Err(scope()),
        };
        return masked_exposure::prepare(doc,supplied,purpose,header,extent,format,default,interrupt,budget);
    }
    for node in &filters.nodes {
        initial_member(node, budget, interrupt)?;
    }
    let (extent, format, default) = match &layer.content {
        LayerContent::BackedRaster(backed) => (backed.operative_bounds(), backed.format(), backed.default_bytes()),
        LayerContent::Raster(surface) => {
            let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or_else(scope)?;
            (item.raster_bounds.ok_or_else(scope)?, surface.format(), surface.default_bytes())
        }
        _ => return Err(scope()),
    };
    crate::gimp_render::qualify_residual_origin_context(doc, layer, state, filters, extent).map_err(BackedNormalError::Color)?;
    crate::gimp_neighborhood::qualify_expanded_residual_mask_storage(state, node, extent).map_err(BackedNormalError::Color)?;
    let stored = doc.sample_encoding.ok_or_else(scope)?;
    if node.operation.schema_epoch == crate::gimp_exposure::LOADED_POINT_EPOCH {
        return loaded_exposure::prepare(doc, supplied, purpose, header, extent, format, default, interrupt, budget);
    }
    if crate::gimp_vibrance::claims(node) {
        return owned_vibrance::prepare(doc, supplied, header, extent, format, default, interrupt, budget);
    }
    if crate::gimp_hue_chroma::claims(node) {
        return owned_hue::prepare(doc, supplied, header, extent, format, default, interrupt, budget);
    }
    // The genuine existing scalar preparation envelope includes actual
    // PointNode/argument storage and two 64KiB implicit analytic profile
    // envelopes (one WorkingSpace and the retained effective-profile preparation).
    // Header slack is explicit; it is not a replacement source/model fee.
    let mut scalar = crate::gimp_neighborhood::scalar_preparation_bytes(1).map_err(BackedNormalError::Color)?;
    for node in &filters.nodes {
        let own = if configured_kind(node).is_some() {
            crate::gimp_neighborhood::stock_configured_preparation_bytes(&node.operation.name)
        } else {
            gimp_filters::scalar_preparation_bytes(1, node.arguments.len())
        }
        .map_err(BackedNormalError::Color)?;
        scalar = scalar.checked_add(own).ok_or_else(length)?;
    }
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
    let envelope = scalar.checked_add((1 + filters.nodes.len() as u64 + extra_spaces) * 65536).and_then(|n| n.checked_add(header)).ok_or_else(length)?;
    let lease = budget.reserve(envelope)?;
    let (current_profile, known) = bound_profile(doc, supplied, budget, interrupt)?;
    // Price bounded repeated string/registry comparisons and the admitted
    // fixed analytic profile preparation before PointStack allocates.
    work(budget, interrupt, envelope.checked_mul(64).ok_or_else(length)?)?;
    if crate::gimp_semi_flatten::claims(node) {
        // Analytic literal conversion is independent of source/profile
        // qualification and is charged before its constructor evaluates it.
        work(budget, interrupt, crate::gimp_semi_flatten::LITERAL_PREPARATION_WORK)?;
        crate::gimp_semi_flatten::qualify_context(doc, extent, node).map_err(BackedNormalError::Color)?;
    }
    if witnessed {
        qualify_witnessed_metadata(doc, layer, state, filters, extent, stored, format, default)?;
    } else if remaining_optional_mask(node) {
        crate::gimp_render::qualify_current_point_optional_mask_metadata(doc, layer, state, filters, extent, stored, format, default)
            .map_err(BackedNormalError::Color)?;
    } else {
        crate::gimp_render::qualify_current_point_layer_coverage_metadata(doc, layer, state, filters, extent, stored, format, default)
            .map_err(BackedNormalError::Color)?;
    }

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
    let mut mask_work = 0u64;
    for mask in &state.filter_masks {
        let physical_pixels = (mask.surface.tile_count() as u64)
            .checked_mul(TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(TILE_SIZE as u64))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(length)?;
        let logical = u64::from(mask.bounds.width()).checked_mul(u64::from(mask.bounds.height())).ok_or_else(length)?;
        mask_work = mask_work.checked_add(physical_pixels.max(logical).checked_mul(32).ok_or_else(length)?).ok_or_else(length)?;
    }
    work(budget, interrupt, mask_work)?;
    for node in &filters.nodes {
        let valid_mask = if crate::gimp_semi_flatten::claims(node) {
            crate::gimp_semi_flatten::qualify_mask(state, node, layer.id, doc.depth, interrupt)
        } else {
            crate::gimp_neighborhood::qualify_selection_mask_with_interrupt(state, node, layer.id, doc.bounds(), interrupt)
        };
        work(budget, interrupt, 0)?;
        valid_mask.map_err(BackedNormalError::Color)?;
    }
    qualify_remaining_metadata(doc, layer, state, node, extent, stored)?;
    qualify_configured_metadata(doc, node, extent, stored).map_err(BackedNormalError::Color)?;
    crate::gimp_render::qualify_residual_point_metadata(doc, layer, state, filters, extent, stored, interrupt).map_err(BackedNormalError::Color)?;
    crate::gimp_render::qualify_point_family_context(doc, layer, state, filters, extent, known, stored, interrupt).map_err(BackedNormalError::Color)?;
    let stack = OrderedStack::new_for_stock(filters, known, stored, state.provenance.format_version, extent, interrupt)
        .map_err(BackedNormalError::Color)?
        .with_current_point_storage(if witnessed || legacy_opaque(node) { None } else { Some(format.sample) })
        .with_filter_output_precision(state.current_filter_output_precision);
    let retained = stack.stock_retained_bytes().map_err(BackedNormalError::Color)?.checked_add(header).and_then(|n| n.checked_add(65536)).ok_or_else(length)?;
    if retained > envelope {
        return Err(BackedNormalError::Allocation);
    }
    // Keep the entire admitted peak for the pipeline lifetime; no phase
    // refund can expose headroom while this owned plan remains live.
    let plan = StockPointPlan {
        recipe: Recipe::Filtered,
        stack: StockPointProgram::Ordered(stack),
        _current_profile: current_profile,
        format,
        opaque_input: legacy_opaque(node) || node.operation.name == "gegl:rgb-clip",
        half_input: matches!(configured_kind(node), Some(ConfiguredKind::Balance))
            && doc.source_color_context.as_ref().is_some_and(|s| matches!(s.source_precision, 500 | 550 | 575)),
        unit_input: crate::gimp_semi_flatten::claims(node),
    };
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
        let (plan, lease) = prepare_plan(doc, Some(supplied), PreparationPurpose::Current, header, control.interrupt, &mut NativeBudget { quota, control })?;
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
        let (plan, lease) = prepare_plan(doc, None, PreparationPurpose::Source, header, interrupt, &mut SharedBudget { budget }).map_err(preparation_error)?;
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
        match &self.plan.stack {
            StockPointProgram::Ordered(stack) => evaluate_bound_point(stack, self.plan.format, self.plan.unit_input, self.plan.half_input, self.plan.opaque_input, &self.quota, raw, format, position, control),
            StockPointProgram::MaskedExposure {candidate,mask,extent,enabled}=>{
                control.check(256).map_err(BackedNormalError::Backing)?;
                let _mix_scratch=candidate.reserve_runtime(&self.quota,control)?;
                if position.0<extent.x0 || position.0>=extent.x1 || position.1<extent.y0 || position.1>=extent.y1 { return Err(scope()); }
                if format!=self.plan.format || raw.len()!=format.bytes_per_pixel() { return Err(scope()); }
                let read=|channel|try_read_sample_f64(raw,format.sample,channel).map_err(|_|length());
                let before=[read(0)?,read(1)?,read(2)?,read(3)?];
                let mut scalar=[0.;1];
                mask.try_read_pixel_f64(position.0,position.1,&mut scalar).map_err(|_|length())?;
                let [coverage]=scalar;
                candidate.evaluate(before,*enabled,coverage,&self.quota,control)
            }
            StockPointProgram::MaskedScalar {candidate,mask,extent,enabled}=>{
                control.check(256).map_err(BackedNormalError::Backing)?;
                let _mix_scratch=candidate.reserve_runtime(&self.quota,control)?;
                if position.0<extent.x0 || position.0>=extent.x1 || position.1<extent.y0 || position.1>=extent.y1 { return Err(scope()); }
                if format!=self.plan.format || raw.len()!=format.bytes_per_pixel() { return Err(scope()); }
                let read=|channel|try_read_sample_f64(raw,format.sample,channel).map_err(|_|length());
                let before=[read(0)?,read(1)?,read(2)?,read(3)?];
                let mut scalar=[0.;1];
                mask.try_read_pixel_f64(position.0,position.1,&mut scalar).map_err(|_|length())?;
                let [coverage]=scalar;
                candidate.evaluate(before,*enabled,coverage,&self.quota,control)
            }
            StockPointProgram::LoadedExposure { candidate, enabled } => {
                control.check(format.bytes_per_pixel() as u64 + 1).map_err(BackedNormalError::Backing)?;
                if format != self.plan.format || raw.len() != format.bytes_per_pixel() { return Err(scope()); }
                let read = |channel| try_read_sample_f64(raw, format.sample, channel).map_err(|_| length());
                candidate.evaluate([read(0)?,read(1)?,read(2)?,read(3)?],*enabled,false,&self.quota,control)
            }
        }
    }
}


/// Same numeric/control body as the singleton native evaluator. Only sealed
/// paid compilers select these flags; no public arbitrary callback is retained.
pub(crate) fn evaluate_bound_point(stack: &OrderedStack, expected: PixelFormat, unit_input: bool, half_input: bool, opaque_input: bool, quota: &BackingQuota, raw: &[u8], format: PixelFormat, position: (i32,i32), control: &ReadControl<'_>) -> Result<[f64;4],BackedNormalError> {
        control.check(format.bytes_per_pixel() as u64 + 1).map_err(BackedNormalError::Backing)?;
        if format != expected || raw.len() != format.bytes_per_pixel() {
            return Err(scope());
        }
        let read = |channel| try_read_sample_f64(raw, format.sample, channel).map_err(|_| length());
        let rgb = if format.mode == ColorMode::Grayscale { [read(0)?; 3] } else { [read(0)?, read(1)?, read(2)?] };
        let input = [rgb[0], rgb[1], rgb[2], read(format.mode.color_channels())?];
        if unit_input && input.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
            return Err(BackedNormalError::Unsupported("unit CURRENT stock Point native samples"));
        }
        if half_input
            && input.iter().any(|value| {
                !photocraft_color::try_round_f32_to_f16_bits(*value as f32)
                    .is_ok_and(|bits| photocraft_color::widen_f16_to_f32_bits(bits) == (*value as f32).to_bits())
            })
        {
            return Err(BackedNormalError::Unsupported("Color Balance exact widened Half current input"));
        }
        if opaque_input && input[3] != 1.0 {
            return Err(BackedNormalError::Unsupported("Posterize opaque current input"));
        }
        let mut failed = None;
        let mut temporary: Option<QuotaLease> = None;
        let mut poll = |event| {
            let result = match event {
                WideWork::SampleUnits(units) => control.check(units).map_err(BackedNormalError::Backing),
                WideWork::ReserveBytes(bytes) if temporary.is_none() => {
                    quota.reserve(QuotaKind::Resident, bytes).map(|lease| temporary = Some(lease)).map_err(BackedNormalError::Quota)
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
        let validated = stack.validate_stock_input(position, input, control.interrupt, &mut poll);
        let evaluated = match validated {
            Ok(()) => stack.point_at(input, position, &mut poll),
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
    node: Option<FilterId>,
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
        let header = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(length)?;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(BackedNormalError::Quota)?;
        let pipeline = PreparedStockPointPipeline::prepare(doc, profile, owner.quota(), control)?;
        let node = match pipeline.plan.recipe {
            Recipe::Empty => None,
            Recipe::Filtered => Some(filters.nodes.first().ok_or_else(scope)?.id),
        };
        let result = Self {
            pipeline,
            owner,
            layer: layer.id,
            node,
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
    pub fn node_id(&self) -> Option<FilterId> {
        self.node
    }
    /// Saved top-to-bottom identities; this iterator grants no edit authority.
    pub fn node_ids(&self) -> impl Iterator<Item = FilterId> + '_ {
        self.owner
            .document()
            .xcf_state
            .as_ref()
            .into_iter()
            .flat_map(|state| state.filter_stacks.first())
            .flat_map(|stack| stack.nodes.iter().map(|node| node.id))
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
#[path = "backed_semi_flatten_tests.rs"]
mod semi_flatten_tests;

#[cfg(test)]
#[path = "backed_stock_point_general_tests.rs"]
mod general_tests;

#[cfg(test)]
#[path = "backed_stock_point_residual_tests.rs"]
mod residual_tests;

#[cfg(test)]
#[path = "backed_stock_point_configured_tests.rs"]
mod configured_tests;

#[cfg(test)]
#[path = "backed_stock_point_stack_tests.rs"]
mod stack_tests;

#[cfg(test)]
#[path = "backed_stock_point_scalar_tests.rs"]
mod scalar_tests;

#[cfg(test)]
#[path = "backed_stock_point_empty_tests.rs"]
mod empty_tests;

#[cfg(test)]
#[path = "backed_hue_chroma_tests.rs"]
mod hue_chroma_tests;

#[cfg(test)]
#[path = "backed_vibrance_tests.rs"]
mod vibrance_tests;

#[cfg(test)]
#[path = "backed_point_origin_geometry_tests.rs"]
mod origin_geometry_tests;

#[cfg(test)]
#[path = "backed_point_replace_origin_geometry_tests.rs"]
mod point_replace_origin_geometry_tests;

#[cfg(test)]
#[path = "residual_origin_geometry_tests.rs"]
mod residual_origin_geometry_tests;


#[cfg(test)]
#[path="backed_stock_point_masked_exposure_tests.rs"]
mod masked_exposure_tests;

#[cfg(test)]
#[path="backed_stock_point_masked_scalar_tests.rs"]
mod masked_scalar_tests;
