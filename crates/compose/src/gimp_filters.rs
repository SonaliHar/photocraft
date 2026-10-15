//! Ordered point evaluation. The original source surface remains authoritative and untouched.
#[cfg(test)]
#[path = "gimp_alien_map_hsl_tests.rs"]
mod alien_map_hsl_tests;
#[cfg(test)]
#[path = "gimp_alien_map_tests.rs"]
mod alien_map_tests;
#[cfg(test)]
#[path = "gimp_color_overlay_tests.rs"]
mod color_overlay_tests;
#[cfg(all(test, feature = "corpus"))]
#[path = "gimp_exposure_geometry_tests.rs"]
mod exposure_geometry_tests;
#[cfg(test)]
#[path = "gimp_point_replace_half_u32_tests.rs"]
mod point_replace_half_u32_tests;
#[cfg(test)]
#[path = "gimp_point_replace_perceptual_tests.rs"]
mod point_replace_perceptual_tests;
#[cfg(test)]
#[path = "gimp_point_replace_tests.rs"]
mod point_replace_tests;
#[cfg(test)]
#[path = "gimp_saturation_yuv_tests.rs"]
mod saturation_yuv_tests;
#[cfg(test)]
#[path = "gimp_sepia_tests.rs"]
mod sepia_tests;
use crate::gimp_composite::{CompositionError, Result, WorkingSpace};
#[path = "gimp_filters_controlled.rs"]
mod controlled;
#[path = "gimp_alpha_clip.rs"]
mod alpha_clip;
#[path = "gimp_alpha_clip_finite.rs"]
mod alpha_clip_finite;
#[path = "gimp_color_exchange.rs"]
mod color_exchange;
#[path = "gimp_color_to_alpha.rs"]
mod color_to_alpha;
#[path = "gimp_component_extract.rs"]
mod component_extract;
#[cfg(test)]
#[path = "gimp_component_geometry_tests.rs"]
mod component_geometry_tests;
use photocraft_algo::gimp::{
    point::{PointOperation, PreparedPointOperation},
    registry::{WorkingEncoding, point_member},
};
use photocraft_cms::{Profile, WideWork};
use photocraft_doc::{
    SampleEncoding,
    import_compositing::GimpSpace,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};

struct PointNode {
    working: WorkingSpace,
    operation: PointOperation,
    prepared: PreparedPointOperation,
    opacity: f64,
    float_yuv: bool,
    gray_color_exchange: Option<photocraft_cms::Transform>,
    color_overlay: Option<crate::gimp_color_overlay::ColorOverlayCandidate>,
    channel_mixer: Option<crate::gimp_channel_mixer::ChannelMixerCandidate>,
    mono_mixer_signed: Option<crate::gimp_mono_mixer::MonoMixerCandidate>,
    color_rotate: Option<crate::gimp_color_rotate::ColorRotateCandidate>,
    red_eye: Option<crate::gimp_red_eye::RedEyeCandidate>,
    absolute: Option<crate::gimp_absolute::AbsoluteCandidate>,
}
pub const ABSOLUTE_LINEAR_PRECISION_EPOCH: &str = "xcf-22-stock-absolute-linear-precision-v1";
pub const CHANNEL_MIXER_PRECISION_EPOCH: &str = "xcf-22-stock-channel-mixer-precision-v1";
pub const RED_EYE_PRECISION_EPOCH: &str = "xcf-22-stock-red-eye-removal-precision-v1";
pub const MONO_MIXER_SIGNED_PRECISION_EPOCH: &str = "xcf-22-stock-mono-mixer-signed-precision-v1";
pub const COLOR_OVERLAY_PRECISION_EPOCH: &str = "xcf-22-stock-color-overlay-six-precision-v1";
pub const COLOR_TO_ALPHA_PRECISION_EPOCH: &str = "xcf-22-stock-color-to-alpha-precision-v1";
pub const ALPHA_CLIP_FINITE_PRECISION_EPOCH: &str = "xcf22-stock-alpha-clip-finite-precision-v1";
pub const ALPHA_CLIP_PRECISION_EPOCH: &str = "xcf22-stock-alpha-clip-precision-v1";
pub const COMPONENT_EXTRACT_PRECISION_EPOCH: &str = "xcf-22-stock-component-extract-four-precision-v1";
pub const COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH: &str = "xcf-22-stock-component-extract-cmyk-precision-v1";
pub const COMPONENT_EXTRACT_GEOMETRY_EPOCH: &str = "xcf-22-stock-component-extract-geometry-v1";
pub const COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH: &str = "xcf-22-stock-component-extract-ycbcr-precision-v1";
pub const COLOR_EXCHANGE_PRECISION_EPOCH: &str = "xcf-22-stock-color-exchange-precision-v1";
pub const ALIEN_MAP_HSL_PRECISION_EPOCH: &str = "xcf-22-stock-alien-map-hsl-precision-v1";
pub const ALIEN_MAP_PRECISION_EPOCH: &str = "xcf-22-stock-alien-map-precision-v1";
pub const SEPIA_PRECISION_EPOCH: &str = "xcf-22-stock-sepia-precision-v1";
pub const THRESHOLD_ALPHA_PRECISION_EPOCH: &str = "xcf-22-stock-threshold-alpha-precision-v1";

/// Shared exact enabled singleton leaf check. This is current operation policy,
/// not source correspondence. The original PointStack still performs its full
/// generic preparation after this check; the alpha-only native consumer needs
/// no unused color-transfer WorkingSpace allocation.
pub(crate) fn validated_enabled_threshold_alpha(
    source: &[FilterNode],
    channels: usize,
    stored: SampleEncoding,
    version: u32,
) -> Result<photocraft_algo::gimp::threshold_alpha::ThresholdAlpha> {
    let (operation, enabled) = validated_current_threshold_alpha(source, channels, stored, version)?;
    if !enabled {
        return Err(CompositionError::Unsupported("unqualified enabled Threshold Alpha current policy"));
    }
    Ok(operation)
}
/// Exact current singleton recipe, including a disabled node. The disabled
/// consumer must bypass the operation with raw native identity; threshold zero
/// is not an identity. No source correspondence is established here.
pub(crate) fn validated_current_threshold_alpha(
    source: &[FilterNode],
    channels: usize,
    stored: SampleEncoding,
    version: u32,
) -> Result<(photocraft_algo::gimp::threshold_alpha::ThresholdAlpha, bool)> {
    let [node] = source else { return Err(CompositionError::Unsupported("singleton Threshold Alpha required")); };
    let policy = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing Threshold Alpha effect policy"))?;
    if version != 22
        || !matches!(channels, 1 | 3)
        || !matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile)
        || node.operation.name != "gimp:threshold-alpha"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != THRESHOLD_ALPHA_PRECISION_EPOCH
        || policy.compatibility_epoch != THRESHOLD_ALPHA_PRECISION_EPOCH
        || policy.source_mode != 62
        || policy.legacy
        || policy.pass_through
        || policy.resolved.is_some()
        || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (2061861457, 2061861457, 2061861457)
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
    {
        return Err(CompositionError::Unsupported("unqualified enabled Threshold Alpha current policy"));
    }
    let [argument] = node.arguments.as_slice() else { return Err(CompositionError::Unsupported("Threshold Alpha single stored argument")); };
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = argument.value else { return Err(CompositionError::Unsupported("Threshold Alpha stored Float32 argument")); };
    if argument.name != "value" {
        return Err(CompositionError::Unsupported("Threshold Alpha stored argument name"));
    }
    let (_, operation) = point_member("gimp:threshold-alpha", Some(&node.operation.stored_version), &[("value", f64::from(f32::from_bits(bits)))])?;
    let PointOperation::ThresholdAlpha { value } = operation else { return Err(CompositionError::Invalid("Threshold Alpha registry member")); };
    let operation = photocraft_algo::gimp::threshold_alpha::ThresholdAlpha { value };
    operation.validate()?;
    Ok((operation, node.enabled))
}
pub const THRESHOLD_PRECISION_EPOCH: &str = "xcf-22-stock-threshold-precision-v1";
pub const SATURATION_GRAY_PRECISION_EPOCH: &str = "xcf-22-stock-gray-saturation-precision-v1";
pub const SATURATION_PRECISION_EPOCH: &str = "xcf-22-stock-saturation-precision-v1";
pub const SATURATION_RGB_YUV_PRECISION_EPOCH: &str = "xcf-22-stock-rgb-yuv-saturation-precision-v1";
pub const DESATURATE_PRECISION_EPOCH: &str = "xcf-22-stock-desaturate-precision-v1";
pub const COLOR_TEMPERATURE_OFFGRID_EPOCH: &str = "xcf-22-stock-color-temperature-offgrid-v1";
pub const COLOR_TEMPERATURE_PRECISION_EPOCH: &str = "xcf-22-stock-color-temperature-precision-v1";
pub const VALUE_INVERT_PRECISION_EPOCH: &str = "xcf-22-stock-value-invert-precision-v1";
pub const EXPOSURE_BLACK_PRECISION_EPOCH: &str = "xcf-22-stock-exposure-black-level-precision-v1";
pub const EXPOSURE_GEOMETRY_EPOCH: &str = "xcf-22-stock-exposure-offset-geometry-v1";
pub const GEGL_LEVELS_PRECISION_EPOCH: &str = "xcf-22-stock-gegl-levels-precision-v1";
pub const BRIGHTNESS_CONTRAST_PRECISION_EPOCH: &str = "xcf-22-stock-brightness-contrast-precision-v1";
pub const RGB_CLIP_HDR_EPOCH: &str = "xcf-22-stock-rgb-clip-hdr-v1";
pub const MONO_MIXER_EDITS_EPOCH: &str = "xcf-22-stock-mono-mixer-edits-v1";
/// Independently measured Replace-only compatibility; raw signed fields remain unresolved.
pub const POINT_REPLACE_COMPATIBILITY_EPOCH: &str = "xcf-22-stock-point-replace-fields-v1";
pub const INVERT_LINEAR_PRECISION_EPOCH: &str = "xcf-22-stock-invert-linear-precision-v1";
pub const INVERT_GAMMA_PRECISION_EPOCH: &str = "xcf-22-stock-invert-gamma-precision-v1";
pub const STACK_EDITS_EPOCH: &str = "xcf-22-stock-point-stack-edits-v1";

pub(crate) struct PointStack {
    unpremultiply: Option<(crate::gimp_unpremultiply::UnpremultiplyCandidate, bool)>,
    video: Option<(crate::gimp_video_degradation::VideoDegradationCandidate, bool)>,
    nodes: Vec<PointNode>,
    channels: usize,
    integer_gray_temperature_clamp: bool,
    current_sample: Option<photocraft_color::SampleType>,
}
/// Heap ownership of scalar point nodes and maximum argument scratch, excluding
/// each WorkingSpace's separately charged analytic profile envelope.
pub(crate) fn scalar_preparation_bytes(nodes: usize, max_args: usize) -> Result<u64> {
    let bytes = nodes
        .checked_mul(std::mem::size_of::<PointNode>())
        .and_then(|n| max_args.checked_mul(std::mem::size_of::<(&str, f64)>()).and_then(|a| n.checked_add(a)))
        .ok_or(CompositionError::Invalid("scalar preparation size overflow"))?;
    u64::try_from(bytes).map_err(|_| CompositionError::Invalid("scalar preparation size overflow"))
}
impl PointStack {
    #[cfg(test)]
    pub(crate) fn from_nodes_at(source: &[FilterNode], profile: &Profile, stored: SampleEncoding, version: u32, owner: photocraft_geom::Rect) -> Result<Self> {
        Self::from_nodes_at_with_interrupt(source, profile, stored, version, owner, &photocraft_raster::Interrupt::NONE)
    }
    pub(crate) fn from_nodes_at_with_interrupt(source: &[FilterNode], profile: &Profile, stored: SampleEncoding, version: u32, owner: photocraft_geom::Rect, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        controlled::preparation(control, |control| Self::prepare_nodes_at(source, profile, stored, version, owner, control))
    }
    fn prepare_nodes_at(source: &[FilterNode], profile: &Profile, stored: SampleEncoding, version: u32, owner: photocraft_geom::Rect, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        if controlled::iter(source, control).any(crate::gimp_red_eye::claims_geometry) {
            let [node] = source else { return Err(CompositionError::Unsupported("Red Eye owner geometry singleton")); };
            crate::gimp_red_eye::canonical_geometry(node, profile, stored, version)?;
            if !matches!(owner, photocraft_geom::Rect { x0: -7, y0: 5, x1: 12, y1: 18 } | photocraft_geom::Rect { x0: -3, y0: -4, x1: 259, y1: 5 }) {
                return Err(CompositionError::Unsupported("Red Eye owner geometry point extent"));
            }
        }
        if controlled::iter(source, control).any(|node| node.operation.name == "gegl:unpremultiply") {
            let [node] = source else { return Err(CompositionError::Unsupported("Unpremultiply singleton")); };
            let candidate = crate::gimp_unpremultiply::canonical_candidate(node, profile, stored, owner, version)?;
            return Ok(Self { unpremultiply: Some((candidate, node.enabled)), video: None, current_sample: None, nodes: Vec::new(), channels: 3, integer_gray_temperature_clamp: false });
        }
        if controlled::iter(source, control).any(crate::gimp_video_degradation::claims_video) {
            let [node] = source else {
                return Err(CompositionError::Unsupported("Video Degradation singleton"));
            };
            let video = crate::gimp_video_degradation::canonical_candidate(node, profile, stored, owner, version)?;
            return Ok(Self {
                unpremultiply: None,
                video: Some((video, node.enabled)),
                current_sample: None,
                nodes: Vec::new(),
                channels: 3,
                integer_gray_temperature_clamp: false,
            });
        }
        Self::from_nodes_with_interrupt(source, profile, stored, version, control)
    }
    /// Actual retained node capacity plus fixed analytic profile envelopes,
    /// including Color Overlay's bounded nested profile/transform preparation.
    pub(crate) fn scalar_retained_bytes(&self) -> Result<u64> {
        let slots = u64::try_from(self.nodes.capacity()).map_err(|_| CompositionError::Invalid("point capacity overflow"))?;
        let overlays = u64::try_from(self.nodes.iter().filter(|node| node.color_overlay.is_some() || node.color_rotate.is_some()).count())
            .map_err(|_| CompositionError::Invalid("point capacity overflow"))?;
        slots
            .checked_mul(std::mem::size_of::<PointNode>() as u64)
            .and_then(|n| n.checked_add((self.nodes.len() as u64).checked_mul(65536)?))
            .and_then(|n| n.checked_add(overlays.checked_mul(4 * 65536)?))
            .ok_or(CompositionError::Invalid("point capacity overflow"))
    }
    #[cfg(test)]
    pub(crate) fn new(stack: &photocraft_doc::xcf_filters::FilterStack, profile: &Profile, stored: SampleEncoding, version: u32) -> Result<Self> {
        Self::from_nodes(&stack.nodes, profile, stored, version)
    }
    #[cfg(test)]
    pub(crate) fn from_nodes(source: &[FilterNode], profile: &Profile, stored: SampleEncoding, version: u32) -> Result<Self> {
        Self::from_nodes_with_interrupt(source, profile, stored, version, &photocraft_raster::Interrupt::NONE)
    }
    pub(crate) fn from_nodes_with_interrupt(source: &[FilterNode], profile: &Profile, stored: SampleEncoding, version: u32, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        controlled::preparation(control, |control| Self::prepare_nodes(source, profile, stored, version, control))
    }
    fn prepare_nodes(source: &[FilterNode], profile: &Profile, stored: SampleEncoding, version: u32, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        if controlled::iter(source, control).any(crate::gimp_red_eye::claims_geometry) {
            let [node] = source else { return Err(CompositionError::Unsupported("Red Eye owner geometry singleton")); };
            crate::gimp_red_eye::canonical_geometry(node, profile, stored, version)?;
        }
        if controlled::iter(source, control).any(|node| node.operation.name == "gegl:unpremultiply") {
            return Err(CompositionError::Unsupported("Unpremultiply requires operative owner extent"));
        }
        if source.len() > 256 {
            return Err(CompositionError::Invalid("point filter stack limit"));
        }
        let replace_fields =
            controlled::iter(source, control).any(|node| node.composition.as_ref().is_some_and(|policy| {
                policy.compatibility_epoch == POINT_REPLACE_COMPATIBILITY_EPOCH || policy.compatibility_epoch == crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH
            }));
        if replace_fields
            && (source.len() != 1
                || controlled::iter(source, control).any(|node| {
                    let known_schema = match node.operation.name.as_str() {
                        "gegl:absolute" => node.operation.schema_epoch == ABSOLUTE_LINEAR_PRECISION_EPOCH && node.arguments.is_empty(),
                        "gegl:invert-linear" => node.operation.schema_epoch == INVERT_LINEAR_PRECISION_EPOCH && node.arguments.is_empty(),
                        "gegl:invert-gamma" => node.operation.schema_epoch == INVERT_GAMMA_PRECISION_EPOCH && node.arguments.is_empty(),
                        "gegl:color-exchange" => node.operation.schema_epoch == COLOR_EXCHANGE_PRECISION_EPOCH,
                        "gegl:color-overlay" => node.operation.schema_epoch == COLOR_OVERLAY_PRECISION_EPOCH,
                        "gegl:channel-mixer" => node.operation.schema_epoch == CHANNEL_MIXER_PRECISION_EPOCH,
                        "gegl:mono-mixer" => node.operation.schema_epoch == MONO_MIXER_SIGNED_PRECISION_EPOCH,
                        "gegl:red-eye-removal" => matches!(node.operation.schema_epoch.as_str(), RED_EYE_PRECISION_EPOCH | crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH),
                        "gegl:color-rotate" => node.operation.schema_epoch == crate::gimp_color_rotate::COLOR_ROTATE_PERCEPTUAL_EPOCH,
                        "gegl:color-to-alpha" => node.operation.schema_epoch == COLOR_TO_ALPHA_PRECISION_EPOCH,
                        "gegl:component-extract" => component_extract::qualified_epoch(&node.operation.schema_epoch),
                        "gegl:alpha-clip" => matches!(node.operation.schema_epoch.as_str(), ALPHA_CLIP_PRECISION_EPOCH | ALPHA_CLIP_FINITE_PRECISION_EPOCH),
                        "gegl:brightness-contrast" => node.operation.schema_epoch == BRIGHTNESS_CONTRAST_PRECISION_EPOCH,
                        "gegl:levels" => node.operation.schema_epoch == GEGL_LEVELS_PRECISION_EPOCH,
                        "gegl:exposure" => node.operation.schema_epoch == EXPOSURE_BLACK_PRECISION_EPOCH,
                        "gegl:color-temperature" => node.operation.schema_epoch == COLOR_TEMPERATURE_PRECISION_EPOCH,
                        "gegl:sepia" => node.operation.schema_epoch == SEPIA_PRECISION_EPOCH,
                        "gegl:alien-map" => matches!(node.operation.schema_epoch.as_str(), ALIEN_MAP_PRECISION_EPOCH | ALIEN_MAP_HSL_PRECISION_EPOCH),
                        _ => false,
                    };
                    !known_schema
                        || node.opacity_f64_bits != 1.0_f64.to_bits()
                        || node.source_clipping != Some(0)
                        || node.source_region != Some(0)
                        || !node.auxiliary.is_empty()
                        || node.composition.as_ref().is_none_or(|policy| {
                            policy.compatibility_epoch != if node.operation.schema_epoch == crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH {
                                crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH
                            } else { POINT_REPLACE_COMPATIBILITY_EPOCH }
                                || policy.source_mode != 62
                                || policy.legacy
                                || policy.pass_through
                                || policy.resolved.is_some()
                        })
                }))
        {
            return Err(CompositionError::Unsupported("unqualified measured point Replace field compatibility"));
        }
        // Perceptual is independently qualified only for the closed
        // singleton Replace families above, never inferred for older epochs.
        let qualified_point_encoding =
            matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile) || (replace_fields && stored == SampleEncoding::Perceptual);
        controlled::checkpoint(control)?;
        let mut nodes = Vec::new();
        nodes.try_reserve_exact(source.len()).map_err(|_| CompositionError::Invalid("point preparation allocation denied"))?;
        let color_rotate = controlled::iter(source, control).any(|node| node.operation.name == "gegl:color-rotate");
        if color_rotate && (!replace_fields || source.len() != 1 || stored != SampleEncoding::Perceptual || !matches!(profile.channels(), 1 | 3)) {
            return Err(CompositionError::Unsupported("Color Rotate Perceptual singleton"));
        }
        let absolute = controlled::iter(source, control).any(|node| node.operation.name == "gegl:absolute");
        if absolute
            && (!replace_fields
                || source.len() != 1
                || stored != SampleEncoding::Linear
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != ABSOLUTE_LINEAR_PRECISION_EPOCH))
        {
            return Err(CompositionError::Unsupported("Absolute Linear singleton typed epoch"));
        }
        let channel_mixer = controlled::iter(source, control).any(|node| node.operation.name == "gegl:channel-mixer");
        if channel_mixer
            && (!replace_fields
                || source.len() != 1
                || !qualified_point_encoding
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != CHANNEL_MIXER_PRECISION_EPOCH))
        {
            return Err(CompositionError::Unsupported("Channel Mixer singleton typed epoch"));
        }
        let mono_signed = controlled::iter(source, control).any(|node| node.operation.schema_epoch == MONO_MIXER_SIGNED_PRECISION_EPOCH);
        if mono_signed
            && (!replace_fields
                || source.len() != 1
                || !qualified_point_encoding
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|node| node.operation.name != "gegl:mono-mixer" || node.operation.schema_epoch != MONO_MIXER_SIGNED_PRECISION_EPOCH))
        {
            return Err(CompositionError::Unsupported("unqualified expanded Mono Mixer domain"));
        }
        let color_overlay = controlled::iter(source, control).any(|node| node.operation.name == "gegl:color-overlay");
        if color_overlay
            && (!replace_fields
                || source.len() != 1
                || !qualified_point_encoding
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|n| n.operation.schema_epoch != COLOR_OVERLAY_PRECISION_EPOCH))
        {
            return Err(CompositionError::Unsupported("unqualified typed Color Overlay domain"));
        }
        let color_to_alpha = controlled::iter(source, control).any(|node| node.operation.name == "gegl:color-to-alpha");
        let component_extract = controlled::iter(source, control).any(|node| node.operation.name == "gegl:component-extract");
        let alpha_clip = controlled::iter(source, control).any(|node| node.operation.name == "gegl:alpha-clip");
        if alpha_clip
            && (!replace_fields
                || source.len() != 1
                || !qualified_point_encoding
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|n| !matches!(n.operation.schema_epoch.as_str(), ALPHA_CLIP_PRECISION_EPOCH | ALPHA_CLIP_FINITE_PRECISION_EPOCH)))
        {
            return Err(CompositionError::Unsupported("unqualified typed Alpha Clip domain"));
        }
        if controlled::iter(source, control).any(|node| node.operation.schema_epoch == COMPONENT_EXTRACT_GEOMETRY_EPOCH) && stored != SampleEncoding::NonlinearProfile {
            return Err(CompositionError::Unsupported("Component Extract geometry requires current nonlinear storage"));
        }
        if component_extract
            && (!replace_fields
                || source.len() != 1
                || !qualified_point_encoding
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|node| !component_extract::qualified_epoch(&node.operation.schema_epoch)))
        {
            return Err(CompositionError::Unsupported("unqualified typed Component Extract domain"));
        }
        if color_to_alpha
            && (!replace_fields
                || source.len() != 1
                || !qualified_point_encoding
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != COLOR_TO_ALPHA_PRECISION_EPOCH))
        {
            return Err(CompositionError::Unsupported("unqualified typed Color to Alpha domain"));
        }
        let color_exchange = controlled::iter(source, control).any(|node| node.operation.name == "gegl:color-exchange");
        if color_exchange
            && (!replace_fields
                || source.len() != 1
                || !qualified_point_encoding
                || !matches!(profile.channels(), 1 | 3)
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != COLOR_EXCHANGE_PRECISION_EPOCH))
        {
            return Err(CompositionError::Unsupported("unqualified typed Color Exchange domain"));
        }
        let alien_hsl = controlled::iter(source, control).any(|node| node.operation.schema_epoch == ALIEN_MAP_HSL_PRECISION_EPOCH);
        let alien_epoch = if alien_hsl { ALIEN_MAP_HSL_PRECISION_EPOCH } else { ALIEN_MAP_PRECISION_EPOCH };
        let alien_map = alien_hsl || controlled::iter(source, control).any(|node| node.operation.schema_epoch == ALIEN_MAP_PRECISION_EPOCH);
        if controlled::iter(source, control).any(|node| node.operation.name == "gegl:alien-map")
            && (!alien_map
                || source.len() != 1
                || profile.channels() != 3
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|n| {
                    n.operation.name != "gegl:alien-map"
                        || n.operation.stored_version != "0:0"
                        || n.operation.schema_epoch != alien_epoch
                        || !typed_alien_map(n, if alien_hsl { 1 } else { 0 })
                }))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed RGB Alien Map typed precision domain"));
        }
        let sepia = controlled::iter(source, control).any(|node| node.operation.schema_epoch == SEPIA_PRECISION_EPOCH);
        if controlled::iter(source, control).any(|node| node.operation.name == "gegl:sepia")
            && (!sepia
                || source.len() != 1
                || profile.channels() != 3
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != SEPIA_PRECISION_EPOCH || node.operation.name != "gegl:sepia"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Sepia precision domain"));
        }
        if sepia && controlled::iter(source, control).any(|node| {
            node.arguments.len()!=2 || controlled::iter(&node.arguments, control).filter(|arg|arg.name=="srgb" && matches!(arg.value,FilterValue::Bool(_))).count()!=1 || controlled::iter(&node.arguments, control).filter(|arg|arg.name=="scale" && matches!(arg.value,FilterValue::Number(ExactNumber::Float32Bits(bits)) if f32::from_bits(bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(bits)))).count()!=1
        }) { return Err(CompositionError::Unsupported("unqualified typed Sepia arguments")); }
        let threshold_alpha = controlled::iter(source, control).any(|node| node.operation.schema_epoch == THRESHOLD_ALPHA_PRECISION_EPOCH);
        if controlled::iter(source, control).any(|node| node.operation.name == "gimp:threshold-alpha")
            && (!threshold_alpha
                || source.len() != 1
                || !matches!(profile.channels(), 1 | 3)
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != THRESHOLD_ALPHA_PRECISION_EPOCH || node.operation.name != "gimp:threshold-alpha"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Alpha Threshold precision domain"));
        }
        if threshold_alpha && source.first().is_some_and(|node| node.enabled) {
            validated_enabled_threshold_alpha(source, profile.channels(), stored, version)?;
        }
        let threshold = controlled::iter(source, control).any(|node| node.operation.schema_epoch == THRESHOLD_PRECISION_EPOCH);
        if controlled::iter(source, control).any(|node| node.operation.name == "gimp:threshold")
            && (!threshold
                || source.len() != 1
                || profile.channels() != 3
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != THRESHOLD_PRECISION_EPOCH || node.operation.name != "gimp:threshold"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed RGB Threshold precision domain"));
        }
        let saturation_gray = controlled::iter(source, control).any(|node| node.operation.schema_epoch == SATURATION_GRAY_PRECISION_EPOCH);
        let saturation_yuv = controlled::iter(source, control).any(|node| node.operation.schema_epoch == SATURATION_RGB_YUV_PRECISION_EPOCH);
        let saturation = saturation_gray || saturation_yuv || controlled::iter(source, control).any(|node| node.operation.schema_epoch == SATURATION_PRECISION_EPOCH);
        if controlled::iter(source, control).any(|node| node.operation.name == "gegl:saturation")
            && (!saturation
                || source.len() != 1
                || profile.channels() != if saturation_gray { 1 } else { 3 }
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| {
                    node.operation.schema_epoch
                        != if saturation_gray {
                            SATURATION_GRAY_PRECISION_EPOCH
                        } else if saturation_yuv {
                            SATURATION_RGB_YUV_PRECISION_EPOCH
                        } else {
                            SATURATION_PRECISION_EPOCH
                        }
                        || node.operation.name != "gegl:saturation"
                }))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Saturation precision domain"));
        }
        let desaturate = controlled::iter(source, control).any(|node| node.operation.schema_epoch == DESATURATE_PRECISION_EPOCH);
        if controlled::iter(source, control).any(|node| node.operation.name == "gimp:desaturate")
            && (!desaturate
                || source.len() != 1
                || profile.channels() != 3
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != DESATURATE_PRECISION_EPOCH || node.operation.name != "gimp:desaturate"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Desaturate precision domain"));
        }
        let witnessed = controlled::iter(source, control).any(|node| node.operation.schema_epoch == STACK_EDITS_EPOCH);
        let mono_edits = controlled::iter(source, control).any(|node| node.operation.schema_epoch == MONO_MIXER_EDITS_EPOCH);
        let value_precision = controlled::iter(source, control).any(|node| node.operation.schema_epoch == VALUE_INVERT_PRECISION_EPOCH);
        let brightness_precision = controlled::iter(source, control).any(|node| node.operation.schema_epoch == BRIGHTNESS_CONTRAST_PRECISION_EPOCH);
        let levels_precision = controlled::iter(source, control).any(|node| node.operation.schema_epoch == GEGL_LEVELS_PRECISION_EPOCH);
        let exposure_geometry = controlled::iter(source, control).any(|node| node.operation.schema_epoch == EXPOSURE_GEOMETRY_EPOCH);
        let exposure_black = exposure_geometry || controlled::iter(source, control).any(|node| node.operation.schema_epoch == EXPOSURE_BLACK_PRECISION_EPOCH);
        let temperature_offgrid = controlled::iter(source, control).any(|node| node.operation.schema_epoch == COLOR_TEMPERATURE_OFFGRID_EPOCH);
        let temperature = temperature_offgrid || controlled::iter(source, control).any(|node| node.operation.schema_epoch == COLOR_TEMPERATURE_PRECISION_EPOCH);
        if !temperature && controlled::iter(source, control).any(|node| node.operation.name == "gegl:color-temperature") {
            return Err(CompositionError::Unsupported("Color Temperature requires independently witnessed precision epoch"));
        }
        if temperature
            && (source.len() != 1
                || !matches!(profile.channels(), 1 | 3)
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| {
                    !matches!(node.operation.schema_epoch.as_str(), COLOR_TEMPERATURE_PRECISION_EPOCH | COLOR_TEMPERATURE_OFFGRID_EPOCH)
                        || node.operation.name != "gegl:color-temperature"
                }))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Color Temperature precision domain"));
        }

        if exposure_black
            && (source.len() != 1
                || !matches!(profile.channels(), 1 | 3)
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| {
                    !matches!(node.operation.schema_epoch.as_str(), EXPOSURE_BLACK_PRECISION_EPOCH | EXPOSURE_GEOMETRY_EPOCH)
                        || node.operation.name != "gegl:exposure"
                }))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Exposure black-level precision domain"));
        }
        if levels_precision
            && (source.len() != 1
                || !matches!(profile.channels(), 1 | 3)
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != GEGL_LEVELS_PRECISION_EPOCH || node.operation.name != "gegl:levels"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed GEGL Levels precision domain"));
        }
        if brightness_precision
            && (source.len() != 1
                || !matches!(profile.channels(), 1 | 3)
                || !qualified_point_encoding
                || source
                    .iter()
                    .any(|node| node.operation.schema_epoch != BRIGHTNESS_CONTRAST_PRECISION_EPOCH || node.operation.name != "gegl:brightness-contrast"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Brightness Contrast precision domain"));
        }
        if value_precision
            && (source.len() != 1
                || profile.channels() != 3
                || !qualified_point_encoding
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != VALUE_INVERT_PRECISION_EPOCH || node.operation.name != "gegl:value-invert"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Value Invert precision domain"));
        }
        let rgb_clip = controlled::iter(source, control).any(|node| node.operation.schema_epoch == RGB_CLIP_HDR_EPOCH || node.operation.name == "gegl:rgb-clip");
        if rgb_clip
            && (source.len() != 1
                || profile.channels() != 3
                || stored != SampleEncoding::Linear
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != RGB_CLIP_HDR_EPOCH || node.operation.name != "gegl:rgb-clip"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed RGB Clip domain"));
        }
        if mono_edits
            && (source.len() != 1
                || profile.channels() != 3
                || stored != SampleEncoding::NonlinearProfile
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != MONO_MIXER_EDITS_EPOCH || node.operation.name != "gegl:mono-mixer"))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed Mono Mixer domain"));
        }
        if witnessed
            && (source.len() > 2
                || profile.channels() != 3
                || stored != SampleEncoding::NonlinearProfile
                || controlled::iter(source, control).any(|node| node.operation.schema_epoch != STACK_EDITS_EPOCH))
        {
            return Err(CompositionError::Unsupported("unqualified witnessed point stack domain"));
        }
        for node in controlled::iter(source, control).filter(|node| node.enabled || replace_fields) {
            controlled::checkpoint(control)?;
            let policy = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing point effect policy"))?;
            // Exact v022 originals have these malformed raw fields. They are preserved,
            // never normalized into a general enum or used to infer another operator.
            if version != 22
                || policy.source_mode != 62
                || policy.legacy
                || policy.pass_through
                || policy.resolved.is_some()
                || (!replace_fields
                    && (policy.stored_blend_space
                        != if alien_hsl {
                            -1091861935
                        } else if alien_map {
                            1311867473
                        } else if sepia {
                            -779910575
                        } else if threshold_alpha {
                            2061861457
                        } else if threshold {
                            -1565818287
                        } else if saturation {
                            -1581350319
                        } else if desaturate {
                            1789100625
                        } else if temperature_offgrid {
                            -1716420015
                        } else if temperature {
                            -113999279
                        } else if exposure_geometry {
                            -901479855
                        } else if exposure_black {
                            706183761
                        } else if levels_precision {
                            392856145
                        } else if brightness_precision {
                            214598225
                        } else if value_precision {
                            -1395293615
                        } else if rgb_clip {
                            1825931857
                        } else if mono_edits {
                            107971153
                        } else if witnessed {
                            -1178762671
                        } else {
                            -261651887
                        }
                        || policy.stored_composite_space
                            != if alien_hsl {
                                -1091861935
                            } else if alien_map {
                                1311867473
                            } else if sepia {
                                -779910575
                            } else if threshold_alpha {
                                2061861457
                            } else if threshold {
                                -1565818287
                            } else if saturation {
                                -1581350319
                            } else if desaturate {
                                1789100625
                            } else if temperature_offgrid {
                                -1716420015
                            } else if temperature {
                                -113999279
                            } else if exposure_geometry {
                                -901479855
                            } else if exposure_black {
                                706183761
                            } else if levels_precision {
                                392856145
                            } else if brightness_precision {
                                214598225
                            } else if value_precision {
                                -1395293615
                            } else if rgb_clip {
                                1825931857
                            } else if mono_edits {
                                107971153
                            } else if witnessed {
                                -1178762671
                            } else {
                                -261651887
                            }
                        || policy.stored_operator
                            != if alien_hsl {
                                -1091861935
                            } else if alien_map {
                                1311867473
                            } else if sepia {
                                -779910575
                            } else if threshold_alpha {
                                2061861457
                            } else if threshold {
                                -1565818287
                            } else if saturation {
                                -1581350319
                            } else if desaturate {
                                1789100625
                            } else if temperature_offgrid {
                                -1716420015
                            } else if temperature {
                                -113999279
                            } else if exposure_geometry {
                                -901479855
                            } else if exposure_black {
                                706183761
                            } else if levels_precision {
                                392856145
                            } else if brightness_precision {
                                214598225
                            } else if value_precision {
                                -1395293615
                            } else if rgb_clip {
                                1825931857
                            } else if mono_edits {
                                107971153
                            } else if witnessed {
                                -1178762671
                            } else {
                                -261651887
                            }))
                || (!replace_fields && alien_map && policy.compatibility_epoch != alien_epoch)
                || (!replace_fields && sepia && policy.compatibility_epoch != SEPIA_PRECISION_EPOCH)
                || (threshold_alpha && policy.compatibility_epoch != THRESHOLD_ALPHA_PRECISION_EPOCH)
                || (threshold && policy.compatibility_epoch != THRESHOLD_PRECISION_EPOCH)
                || (saturation && policy.compatibility_epoch != node.operation.schema_epoch)
                || (desaturate && policy.compatibility_epoch != DESATURATE_PRECISION_EPOCH)
                || (mono_edits && policy.compatibility_epoch != MONO_MIXER_EDITS_EPOCH)
                || (rgb_clip && policy.compatibility_epoch != RGB_CLIP_HDR_EPOCH)
                || (value_precision && policy.compatibility_epoch != VALUE_INVERT_PRECISION_EPOCH)
                || (!replace_fields && brightness_precision && policy.compatibility_epoch != BRIGHTNESS_CONTRAST_PRECISION_EPOCH)
                || (!replace_fields && levels_precision && policy.compatibility_epoch != GEGL_LEVELS_PRECISION_EPOCH)
                || (!replace_fields && exposure_black && policy.compatibility_epoch != node.operation.schema_epoch)
                || (!replace_fields && temperature && policy.compatibility_epoch != node.operation.schema_epoch)
                || (node.opacity_f64_bits != 1.0_f64.to_bits()
                    && !(witnessed && node.operation.name == "gegl:invert-linear" && node.opacity_f64_bits == 0.5_f64.to_bits()))
                || node.source_clipping != Some(0)
                || node.source_region != Some(0)
                || !node.auxiliary.is_empty()
            {
                return Err(CompositionError::Unsupported("unqualified point Replace effect envelope"));
            }
            if node.arguments.len()
                > if color_rotate || channel_mixer || alien_map {
                    10
                } else if color_exchange {
                    5
                } else {
                    4
                }
            {
                return Err(CompositionError::Unsupported("point arguments"));
            }
            controlled::checkpoint(control)?;
            let mut args = Vec::new();
            args.try_reserve_exact(node.arguments.len()).map_err(|_| CompositionError::Invalid("point argument allocation denied"))?;
            for arg in controlled::iter(&node.arguments, control).filter(|_| {
                !color_rotate && !channel_mixer && !mono_signed && !color_overlay && !color_exchange && !color_to_alpha && !component_extract && !alpha_clip
            }) {
                controlled::checkpoint(control)?;
                let value = match &arg.value {
                    FilterValue::Enum { domain, value } if alien_map && arg.name == "color-model" && domain == "gegl:alien-map/color-model" => {
                        f64::from(*value)
                    }
                    FilterValue::Bool(value) if alien_map && matches!(arg.name.as_str(), "cpn-1-keep" | "cpn-2-keep" | "cpn-3-keep") => {
                        if *value {
                            1.
                        } else {
                            0.
                        }
                    }
                    FilterValue::Enum { domain, value }
                        if threshold && arg.name == "channel" && domain == "GimpHistogramChannel" && (0..=5).contains(value) =>
                    {
                        f64::from(*value)
                    }

                    FilterValue::Enum { domain, value }
                        if saturation && arg.name == "colorspace" && domain == "GeglSaturationType" && (0..=2).contains(value) =>
                    {
                        f64::from(*value)
                    }
                    FilterValue::Enum { domain, value } if desaturate && arg.name == "mode" && domain == "GimpDesaturateMode" && (0..=4).contains(value) => {
                        f64::from(*value)
                    }
                    FilterValue::Number(ExactNumber::Float32Bits(bits))
                        if (node.operation.name != "gimp:threshold" || matches!(arg.name.as_str(), "low" | "high"))
                            && node.operation.name != "gimp:posterize"
                            && node.operation.name != "gimp:desaturate"
                            && !(node.operation.name == "gegl:saturation" && arg.name == "colorspace")
                            && !(node.operation.name == "gegl:mono-mixer" && arg.name == "preserve-luminosity")
                            && !(node.operation.name == "gegl:sepia" && arg.name == "srgb")
                            && !(node.operation.name == "gegl:rgb-clip" && matches!(arg.name.as_str(), "clip-low" | "clip-high")) =>
                    {
                        f64::from(f32::from_bits(*bits))
                    }
                    FilterValue::Bool(value) if rgb_clip && matches!(arg.name.as_str(), "clip-low" | "clip-high") => {
                        if *value {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    FilterValue::Bool(value) if sepia && arg.name == "srgb" => {
                        if *value {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    FilterValue::Bool(false) if node.operation.name == "gegl:mono-mixer" && arg.name == "preserve-luminosity" => 0.0,
                    FilterValue::Number(ExactNumber::Signed { width: 32, value })
                        if node.operation.name == "gimp:posterize" && arg.name == "levels" && (2..=256).contains(value) =>
                    {
                        *value as f64
                    }
                    _ => return Err(CompositionError::Unsupported("point argument stored type")),
                };
                args.push((arg.name.as_str(), value));
            }
            if node.operation.name == "gegl:exposure" && !exposure_black && args.iter().any(|(name, value)| *name == "black-level" && *value != 0.0) {
                return Err(CompositionError::Unsupported("nonzero Exposure black-level requires witnessed precision epoch"));
            }
            let red_eye = node.operation.name == "gegl:red-eye-removal";
            let (member, operation) = if absolute {
                if !node.arguments.is_empty()
                    || node
                        .composition
                        .as_ref()
                        .is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (1574142545, 1574142545, 1574142545))
                {
                    return Err(CompositionError::Unsupported("Absolute Linear schema/raw envelope"));
                }
                photocraft_algo::gimp::registry::absolute_linear_member(Some(&node.operation.stored_version))?
            } else if color_rotate {
                crate::gimp_color_rotate::operation(node)?
            } else if red_eye {
                let raw_space = if node.operation.schema_epoch == crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH {
                    crate::gimp_red_eye::OWNER_GEOMETRY_RAW_SPACE
                } else { 127500881 };
                if !matches!(node.operation.schema_epoch.as_str(), RED_EYE_PRECISION_EPOCH | crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH)
                    || node
                        .composition
                        .as_ref()
                        .is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (raw_space, raw_space, raw_space))
                {
                    return Err(CompositionError::Unsupported("Red Eye schema/raw envelope"));
                }
                photocraft_algo::gimp::registry::red_eye_member(Some(&node.operation.stored_version), crate::gimp_red_eye::typed_threshold(&node.arguments)?)?
            } else if channel_mixer {
                crate::gimp_channel_mixer::operation(node)?
            } else if mono_signed {
                crate::gimp_mono_mixer::operation(node)?
            } else if color_overlay {
                crate::gimp_color_overlay::operation(node)?
            } else if alpha_clip && node.operation.schema_epoch == ALPHA_CLIP_FINITE_PRECISION_EPOCH {
                alpha_clip_finite::operation(
                    node,
                    if profile.channels() == 1 || stored != SampleEncoding::NonlinearProfile { WorkingEncoding::Linear } else { WorkingEncoding::Nonlinear },
                )?
            } else if alpha_clip {
                alpha_clip::operation(
                    node,
                    if profile.channels() == 1 || stored != SampleEncoding::NonlinearProfile { WorkingEncoding::Linear } else { WorkingEncoding::Nonlinear },
                )?
            } else if component_extract {
                component_extract::operation(node)?
            } else if color_to_alpha {
                color_to_alpha::operation(node)?
            } else if color_exchange {
                color_exchange::operation(node)?
            } else {
                point_member(&node.operation.name, Some(&node.operation.stored_version), &args)?
            };
            if matches!(operation, PointOperation::ValueInvert | PointOperation::MonoMixer { .. } | PointOperation::RgbClip { .. }) && profile.channels() != 3 {
                return Err(CompositionError::Unsupported("coupled RGB point operation"));
            }
            if witnessed
                && (policy.compatibility_epoch != STACK_EDITS_EPOCH
                    || !matches!(node.operation.name.as_str(), "gegl:exposure" | "gegl:invert-linear")
                    || !matches!(operation, PointOperation::Exposure { stops: 0.5, black_level: 0.0 } | PointOperation::Invert))
            {
                return Err(CompositionError::Unsupported("unqualified witnessed point operation"));
            }
            let overlay = if color_overlay {
                let (color, srgb) = crate::gimp_color_overlay::literal_arguments(&node.arguments)?;
                Some(crate::gimp_color_overlay::ColorOverlayCandidate::new(profile, stored, color, srgb)?)
            } else {
                None
            };
            if !node.enabled {
                continue;
            }
            let working = match member.encoding {
                WorkingEncoding::Perceptual => GimpSpace::PerceptualRgb,
                WorkingEncoding::Linear => GimpSpace::LinearRgb,
                WorkingEncoding::Nonlinear => GimpSpace::ProfileRgb,
            };
            nodes.push(PointNode {
                absolute: if absolute { Some(crate::gimp_absolute::AbsoluteCandidate::new(profile, stored)?) } else { None },
                color_rotate: if color_rotate {
                    Some(crate::gimp_color_rotate::ColorRotateCandidate::new(profile, stored, crate::gimp_color_rotate::typed_arguments(&node.arguments)?)?)
                } else {
                    None
                },
                channel_mixer: if channel_mixer {
                    Some(crate::gimp_channel_mixer::ChannelMixerCandidate::new(profile, stored, crate::gimp_channel_mixer::typed_arguments(&node.arguments)?)?)
                } else {
                    None
                },
                red_eye: if red_eye {
                    Some(crate::gimp_red_eye::RedEyeCandidate::new(profile, stored, crate::gimp_red_eye::typed_threshold(&node.arguments)?)?)
                } else {
                    None
                },
                color_overlay: overlay,
                mono_mixer_signed: if mono_signed {
                    Some(crate::gimp_mono_mixer::MonoMixerCandidate::new(profile, stored, crate::gimp_mono_mixer::typed_arguments(&node.arguments)?)?)
                } else {
                    None
                },
                working: WorkingSpace::new_with_interrupt(profile, stored, working, control)?,
                operation,
                prepared: operation.prepare()?,
                opacity: f64::from_bits(node.opacity_f64_bits),
                float_yuv: saturation && !saturation_gray && matches!(operation, PointOperation::Saturation { colorspace: 2, .. }),
                gray_color_exchange: if (color_exchange || color_to_alpha || component_extract || alpha_clip) && profile.channels() == 1 {
                    use photocraft_cms::{
                        Builtin, Intent, Transform,
                        xcf_color::{Encoding, effective_profile_from_definition},
                    };
                    let encoding = match stored {
                        SampleEncoding::Linear => Encoding::Linear,
                        SampleEncoding::NonlinearProfile => Encoding::NonlinearProfile,
                        SampleEncoding::Perceptual => Encoding::Perceptual,
                    };
                    let source = if alpha_clip {
                        effective_profile_from_definition(Builtin::Srgb.profile(), Encoding::Linear)?
                    } else {
                        Builtin::Srgb.profile().clone()
                    };
                    Some(Transform::new(&source, &effective_profile_from_definition(profile, encoding)?, Intent::RelativeColorimetric, false)?)
                } else {
                    None
                },
            });
        }
        // Public get_filters and saved effect order is top-to-bottom. The scoped
        // independent stack witnesses prove the bottom effect evaluates first.
        controlled::checkpoint(control)?;
        if witnessed {
            nodes.reverse();
        }
        Ok(Self {
            unpremultiply: None,
            video: None,
            current_sample: None,
            nodes,
            channels: profile.channels(),
            integer_gray_temperature_clamp: stored != SampleEncoding::Perceptual,
        })
    }
    #[cfg(test)]
    pub(crate) fn apply(&self, pixel: [f64; 4], poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        self.apply_current(pixel, false, poll)
    }
    pub(crate) fn set_current_sample(&mut self, sample: Option<photocraft_color::SampleType>) {
        self.current_sample = sample;
    }
    pub(crate) fn has_signed_generated_storage(&self) -> bool {
        self.unpremultiply.is_some() || self.nodes.iter().any(|node| node.mono_mixer_signed.is_some() || node.channel_mixer.is_some() || node.absolute.is_some())
    }
    pub(crate) fn changes_alpha(&self) -> bool {
        self.nodes.iter().any(|node| {
            matches!(
                node.operation,
                PointOperation::AlphaClipFinitePrecision { clip_low: true, .. }
                    | PointOperation::AlphaClipFinitePrecision { clip_high: true, .. }
                    | PointOperation::AlphaClipPrecision { clip_low: true, .. }
                    | PointOperation::AlphaClipPrecision { clip_high: true, .. }
                    | PointOperation::ChannelMixer { .. }
                    | PointOperation::MonoMixerSigned { .. }
                    | PointOperation::RedEyeRemoval { .. }
                    | PointOperation::AbsoluteLinear
                    | PointOperation::ColorOverlay { .. }
                    | PointOperation::ColorToAlpha { .. }
                    | PointOperation::ComponentExtract { .. }
                    | PointOperation::ThresholdAlpha { .. }
            )
        })
    }
    pub(crate) fn apply_current_at(
        &self,
        position: (i32, i32),
        pixel: [f64; 4],
        integer_gray_temperature: bool,
        poll: &mut impl FnMut(WideWork) -> Result<()>,
    ) -> Result<[f64; 4]> {
        if let Some((video, enabled)) = &self.video {
            if self.current_sample != Some(photocraft_color::SampleType::F32) {
                return Err(CompositionError::Unsupported("Video Degradation current native Float storage"));
            }
            poll(WideWork::SampleUnits(128))?;
            let mut output = [0.; 4];
            controlled::run(poll, |control, _work| video.try_eval_at(position, &pixel, &mut output, *enabled, control))?;
            poll(WideWork::SampleUnits(0))?;
            return Ok(output);
        }
        self.apply_current(pixel, integer_gray_temperature, poll)
    }
    pub(crate) fn apply_current(&self, mut pixel: [f64; 4], integer_gray_temperature: bool, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        if let Some((candidate, enabled)) = &self.unpremultiply {
            if self.current_sample != Some(photocraft_color::SampleType::F32) { return Err(CompositionError::Unsupported("Unpremultiply current native Float storage")); }
            poll(WideWork::SampleUnits(128))?;
            let output = controlled::run(poll, |control, work| candidate.apply_with(&pixel, *enabled, control, work))?;
            poll(WideWork::SampleUnits(0))?;
            return Ok(output);
        }
        if self.video.is_some() {
            return Err(CompositionError::Unsupported("Video Degradation requires coordinates"));
        }
        for node in &self.nodes {
            poll(WideWork::SampleUnits(128))?;
            if let Some(absolute) = &node.absolute {
                pixel = if self.channels == 1 {
                    controlled::run(poll, |control, work| absolute.apply_with(&[pixel[0], pixel[3]], true, control, work))?
                } else {
                    controlled::run(poll, |control, work| absolute.apply_with(&pixel, true, control, work))?
                };
                continue;
            }
            if let Some(rotate) = &node.color_rotate {
                pixel = if self.channels == 1 {
                    controlled::run(poll, |control, work| rotate.apply_with(&[pixel[0], pixel[3]], true, control, work))?
                } else {
                    controlled::run(poll, |control, work| rotate.apply_with(&pixel, true, control, work))?
                };
                continue;
            }
            if let Some(red_eye) = &node.red_eye {
                pixel = if self.channels == 1 {
                    controlled::run(poll, |control, work| red_eye.apply_with(&[pixel[0], pixel[3]], true, control, work))?
                } else {
                    controlled::run(poll, |control, work| red_eye.apply_with(&pixel, true, control, work))?
                };
                continue;
            }
            if let Some(channel) = &node.channel_mixer {
                let sample = self.current_sample.ok_or(CompositionError::Unsupported("Channel Mixer requires mutable current storage"))?;
                pixel = if self.channels == 1 {
                    controlled::run(poll, |control, work| channel.apply_with_current_storage(&[pixel[0], pixel[3]], true, sample, control, work))?
                } else {
                    controlled::run(poll, |control, work| channel.apply_with_current_storage(&pixel, true, sample, control, work))?
                };
                continue;
            }
            if let Some(mono) = &node.mono_mixer_signed {
                pixel = if self.channels == 1 {
                    controlled::run(poll, |control, work| mono.apply_with(&[pixel[0], pixel[3]], true, control, work))?
                } else {
                    controlled::run(poll, |control, work| mono.apply_with(&pixel, true, control, work))?
                };
                continue;
            }
            if let Some(overlay) = &node.color_overlay {
                pixel = if self.channels == 1 { overlay.apply(&[pixel[0], pixel[3]], true)? } else { overlay.apply(&pixel, true)? };
                continue;
            }
            if matches!(node.operation, PointOperation::ThresholdAlpha { .. }) {
                // This independently qualified Replace operation changes only
                // coverage. Avoid a color transfer round trip entirely.
                let input = if self.channels == 1 { [pixel[0], pixel[3], 0., 0.] } else { pixel };
                let count = self.channels.checked_add(1).ok_or(CompositionError::Invalid("point channels"))?;
                let mut output = [0.; 4];
                node.prepared.try_eval_pixel(
                    input.get(..count).ok_or(CompositionError::Invalid("point channels"))?,
                    self.channels,
                    output.get_mut(..count).ok_or(CompositionError::Invalid("point channels"))?,
                )?;
                pixel[3] = *output.get(self.channels).ok_or(CompositionError::Invalid("point channels"))?;
                continue;
            }
            // A validated exact identity requires no transfer round trip. Retain
            // authoritative low bits and signed zero rather than re-encoding them.
            if matches!(
                node.operation,
                PointOperation::AlphaClipFinitePrecision { clip_low: false, clip_high: false, .. }
                    | PointOperation::AlphaClipPrecision { clip_low: false, clip_high: false, .. }
                    | PointOperation::Exposure { stops: 0.0, black_level: 0.0 }
                    | PointOperation::BrightnessContrast { brightness: 0.0, contrast: 1.0 }
                    | PointOperation::Levels { input_low: 0.0, input_high: 1.0, output_low: 0.0, output_high: 1.0 }
            ) {
                continue;
            }
            let color = node.working.convert([pixel[0], pixel[1], pixel[2]], true)?;
            if matches!(node.operation, PointOperation::AlphaClipPrecision { .. } | PointOperation::AlphaClipFinitePrecision { .. }) {
                let mut output = [0.; 4];
                node.prepared.try_eval_pixel(&[color[0], color[1], color[2], pixel[3]], 3, &mut output)?;
                if matches!(node.operation, PointOperation::AlphaClipFinitePrecision { .. }) && !(0.0..=1.0).contains(&output[3]) {
                    return Err(CompositionError::Unsupported("finite Alpha Clip ordinary Normal output alpha"));
                }
                let stored = if let Some(reverse) = &node.gray_color_exchange {
                    let mut gray = [0.];
                    reverse.try_eval_f64(&output[..3], &mut gray)?;
                    [gray[0]; 3]
                } else {
                    node.working.convert([output[0], output[1], output[2]], false)?
                };
                pixel = [stored[0], stored[1], stored[2], output[3]];
                continue;
            }
            if matches!(node.operation, PointOperation::ColorExchange { .. } | PointOperation::ColorToAlpha { .. } | PointOperation::ComponentExtract { .. }) {
                // Stock GEGL's operation boundary is nonlinear Float32 RGB;
                // native finite source values are preserved, never clamped.
                let working = color.map(|value| f64::from(value as f32));
                if working.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) || !pixel[3].is_finite() || !(0.0..=1.0).contains(&pixel[3]) {
                    return Err(CompositionError::Unsupported("Color Exchange working component outside unit domain"));
                }
                let mut output = [0.; 4];
                node.prepared.try_eval_pixel(&[working[0], working[1], working[2], pixel[3]], 3, &mut output)?;
                let stored = if let Some(reverse) = &node.gray_color_exchange {
                    let mut gray = [0.];
                    reverse.try_eval_f64(&output[..3], &mut gray)?;
                    [gray[0]; 3]
                } else {
                    node.working.convert([output[0], output[1], output[2]], false)?
                };
                // Literal alpha is inert; Color to Alpha generated coverage is operative.
                let alpha =
                    if matches!(node.operation, PointOperation::ColorToAlpha { .. } | PointOperation::ComponentExtract { .. }) { output[3] } else { pixel[3] };
                pixel = [stored[0], stored[1], stored[2], alpha];
                continue;
            }
            let mut input = [color[0], color[1], color[2], pixel[3]];
            if self.channels == 1 {
                input[1] = pixel[3];
            }
            let count = self.channels.checked_add(1).ok_or(CompositionError::Invalid("point channels"))?;
            let mut output = [0.0; 4];
            // Independently saved Perceptual Gray U32 live and baked outputs
            // are exact matches; its boundary is the ordinary post-Gray clamp.
            if integer_gray_temperature
                && self.integer_gray_temperature_clamp
                && self.channels == 1
                && matches!(node.operation, PointOperation::ColorTemperature { .. })
            {
                node.prepared.try_eval_integer_gray_temperature(
                    input.get(..count).ok_or(CompositionError::Invalid("point channels"))?,
                    output.get_mut(..count).ok_or(CompositionError::Invalid("point channels"))?,
                )?;
            } else if node.float_yuv {
                node.operation.try_eval_float_yuv_pixel(&input, &mut output)?;
            } else {
                node.prepared.try_eval_pixel(
                    input.get(..count).ok_or(CompositionError::Invalid("point channels"))?,
                    self.channels,
                    output.get_mut(..count).ok_or(CompositionError::Invalid("point channels"))?,
                )?;
            }
            let color = if self.channels == 1 { [output[0]; 3] } else { [output[0], output[1], output[2]] };
            let color = if node.opacity == 1.0 {
                color
            } else {
                // The independently authored half-opacity Invert witness proves
                // interpolation before returning from the operation's linear space.
                [
                    input[0] * (1.0 - node.opacity) + color[0] * node.opacity,
                    input[1] * (1.0 - node.opacity) + color[1] * node.opacity,
                    input[2] * (1.0 - node.opacity) + color[2] * node.opacity,
                ]
            };
            let stored = node.working.convert(color, false)?;
            pixel = [stored[0], stored[1], stored[2], pixel[3]];
        }
        controlled::finish_pixel(pixel, poll)
    }
}

#[cfg(test)]
mod unpremultiply_stack_tests {
    use super::*;
    fn node() -> FilterNode {
        use photocraft_doc::{import_compositing::GimpComposition, item_identity::{FilterId, ItemRef, MaskId}, xcf_filters::OperationIdentity};
        FilterNode {
            id: FilterId(1), operation: OperationIdentity { name: "gegl:unpremultiply".into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-unpremultiply-rgb-f32-linear-v1".into() },
            arguments: vec![], source_display_name: None, source_icon: None, enabled: true, opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0), source_region: Some(0), mask: Some(ItemRef::Mask(MaskId(2))), auxiliary: vec![],
            composition: Some(GimpComposition { compatibility_epoch: "xcf-22-stock-unpremultiply-rgb-f32-linear-v1".into(), source_mode: 62, legacy: false,
                stored_blend_space: -1877441967, stored_composite_space: -1877441967, stored_operator: -1877441967, resolved: None, pass_through: false }),
        }
    }
    #[test]
    fn unpremultiply_public_stack_expected_positive() {
        let mut stack = PointStack::from_nodes_at(&[node()], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Linear, 22, photocraft_geom::Rect::new(0, 0, 8, 6)).unwrap();
        stack.set_current_sample(Some(photocraft_color::SampleType::F32));
        let actual = stack.apply_current([0.25, -0.5, 0.75, 0.5], false, &mut |_| Ok(())).unwrap();
        assert_eq!(actual, [0.5, -1., 1.5, 0.5]);
    }
    #[test]
    fn unpremultiply_stack_context_envelope_and_output_refusals() {
        let prepare = |nodes: &[FilterNode], profile, stored, version, owner| PointStack::from_nodes_at(nodes, profile, stored, version, owner);
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let owner = photocraft_geom::Rect::new(0, 0, 8, 6);
        for mutation in 0..8 {
            let mut bad = node();
            match mutation {
                0 => bad.operation.schema_epoch.push('x'),
                1 => bad.operation.stored_version = "1:0".into(),
                2 => bad.opacity_f64_bits = 0.5f64.to_bits(),
                3 => bad.source_clipping = Some(1),
                4 => bad.source_region = None,
                5 => bad.mask = None,
                6 => bad.composition.as_mut().unwrap().stored_operator = 1574142545,
                _ => bad.composition.as_mut().unwrap().compatibility_epoch = POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
            }
            assert!(prepare(&[bad], profile, SampleEncoding::Linear, 22, owner).is_err());
        }
        assert!(prepare(&[node(), node()], profile, SampleEncoding::Linear, 22, owner).is_err());
        for sample in [None, Some(photocraft_color::SampleType::U8), Some(photocraft_color::SampleType::F64)] {
            let mut stack = prepare(&[node()], profile, SampleEncoding::Linear, 22, owner).unwrap();
            stack.set_current_sample(sample);
            assert!(stack.apply_current([0.25, 0.5, 0.75, 0.5], false, &mut |_| Ok(())).is_err());
        }
        let mut original = node(); original.enabled = false;
        let mut stack = prepare(&[original], profile, SampleEncoding::Linear, 22, owner).unwrap();
        stack.set_current_sample(Some(photocraft_color::SampleType::F32));
        assert_eq!(stack.apply_current([-1., 6., -0., -0.], false, &mut |_| Ok(())).unwrap().map(f64::to_bits), [-1., 6., -0., -0.].map(f64::to_bits));
        assert!(stack.apply_current([0.; 4], false, &mut |_| Err(CompositionError::Invalid("denied work"))).is_err());
        let mut stack = prepare(&[node()], profile, SampleEncoding::Linear, 22, owner).unwrap();
        stack.set_current_sample(Some(photocraft_color::SampleType::F32));
        assert!(stack.apply_current([1., 0., 0., f64::from(f32::from_bits(1))], false, &mut |_| Ok(())).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        LayerId,
        import_compositing::GimpComposition,
        item_identity::{FilterId, ItemRef},
        xcf_filters::{FilterArgument, FilterNode, FilterStack, OperationIdentity},
    };
    fn node(id: u64, name: &str, args: &[(&str, f32)]) -> FilterNode {
        FilterNode {
            id: FilterId(id),
            operation: OperationIdentity {
                name: name.into(),
                stored_version: if name == "gegl:exposure" { "1:0" } else { "0:0" }.into(),
                schema_epoch: "test".into(),
            },
            arguments: args
                .iter()
                .map(|(name, value)| FilterArgument { name: (*name).into(), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) })
                .collect(),
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: "test".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -261651887,
                stored_composite_space: -261651887,
                stored_operator: -261651887,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    fn alpha_threshold_closed_replace_preserves_stored_color_bits() {
        let mut original = node(1, "gimp:threshold-alpha", &[("value", 0.5)]);
        original.operation.schema_epoch = "xcf-22-stock-threshold-alpha-precision-v1".into();
        let policy = original.composition.as_mut().unwrap();
        policy.compatibility_epoch = original.operation.schema_epoch.clone();
        policy.stored_blend_space = 2061861457;
        policy.stored_composite_space = 2061861457;
        policy.stored_operator = 2061861457;
        for profile in [photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::Builtin::SGray.profile()] {
            for encoding in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
                let prepared = PointStack::from_nodes(std::slice::from_ref(&original), profile, encoding, 22).unwrap();
                for (alpha, expected) in [(0.5, 0.), (0.5 + 2_f64.powi(-24), 1.)] {
                    let input = [0.173, -0.0, 0.719, alpha];
                    let output = prepared.apply(input, &mut |_| Ok(())).unwrap();
                    assert_eq!(output[3], expected);
                    assert_eq!(output[..3].iter().map(|v| v.to_bits()).collect::<Vec<_>>(), input[..3].iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                }
            }
        }
        for kind in 0..5 {
            let mut bad = original.clone();
            match kind {
                0 => bad.opacity_f64_bits = 0.5_f64.to_bits(),
                1 => bad.operation.schema_epoch = "unknown".into(),
                2 => bad.composition.as_mut().unwrap().stored_operator = 1,
                3 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.5_f64.to_bits())),
                _ => bad.source_region = Some(1),
            }
            assert!(PointStack::from_nodes(&[bad], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).is_err());
        }
    }
    #[test]
    #[ignore = "24 MP release point-stack kernel timing; run explicitly with --release --ignored --nocapture"]
    fn bench_point_stack_24mp() {
        let original =
            FilterStack { owner: ItemRef::Layer(LayerId(1)), revision: 0, nodes: vec![node(1, "gegl:exposure", &[("exposure", 0.5), ("black-level", 0.0)])] };
        let stack = PointStack::new(&original, photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
        let start = std::time::Instant::now();
        let mut result = [0.0; 4];
        for i in 0..24_000_000_u32 {
            let value = f64::from(i % 256) / 255.0;
            result = stack.apply(std::hint::black_box([value, 0.4, 0.7, 1.0]), &mut |_| Ok(())).unwrap();
        }
        println!("24 MP checked encoded exposure point-stack kernel: {:?}; checksum {:?}", start.elapsed(), std::hint::black_box(result));
    }
    #[test]
    fn ordered_hdr_gray_alpha_and_disabled_state() {
        let mut original = FilterStack {
            owner: ItemRef::Layer(LayerId(1)),
            revision: 0,
            nodes: vec![node(1, "gegl:exposure", &[("exposure", 1.0), ("black-level", 0.0)]), node(2, "gegl:invert-linear", &[])],
        };
        let profile = photocraft_cms::Builtin::LinearSrgb.profile();
        let prepared = PointStack::new(&original, profile, SampleEncoding::Linear, 22).unwrap();
        let input = [3.0 + 2.0_f64.powi(-40), -2.0, 0.25, -0.0];
        let actual = prepared.apply(input, &mut |_| Ok(())).unwrap();
        assert_eq!(actual, [1.0 - 2.0 * input[0], 5.0, 0.5, -0.0]);
        assert_eq!(actual[3].to_bits(), input[3].to_bits());
        original.nodes[1].enabled = false;
        original.nodes[1].operation.name = "unimplemented:preserved".into();
        original.nodes[1].composition = None;
        let prepared = PointStack::new(&original, profile, SampleEncoding::Linear, 22).unwrap();
        assert_eq!(prepared.apply(input, &mut |_| Ok(())).unwrap()[0], 2.0 * input[0]);
        let gray = photocraft_cms::Builtin::SGray.profile();
        let prepared = PointStack::new(&original, gray, SampleEncoding::Linear, 22).unwrap();
        assert_eq!(prepared.apply([2.5, 2.5, 2.5, 0.4], &mut |_| Ok(())).unwrap(), [5.0, 5.0, 5.0, 0.4]);
        let identity = FilterStack { owner: original.owner, revision: 0, nodes: vec![node(1, "gegl:exposure", &[("exposure", 0.0), ("black-level", 0.0)])] };
        let prepared = PointStack::new(&identity, photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
        assert_eq!(prepared.apply(input, &mut |_| Ok(())).unwrap().map(f64::to_bits), input.map(f64::to_bits));
    }
    #[test]
    fn posterize_rejects_wrong_stored_types_domains_and_hdr_atomically() {
        let mut current = node(1, "gimp:posterize", &[]);
        current.arguments = vec![FilterArgument { name: "levels".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: 3 }) }];
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let valid = PointStack::from_nodes(std::slice::from_ref(&current), profile, SampleEncoding::NonlinearProfile, 22).unwrap();
        let source = [0.125, 1.001, 0.5, 0.75];
        assert!(valid.apply(source, &mut |_| Ok(())).is_err());
        assert_eq!(source, [0.125, 1.001, 0.5, 0.75]);
        for value in [
            FilterValue::Number(ExactNumber::Signed { width: 16, value: 3 }),
            FilterValue::Number(ExactNumber::Signed { width: 32, value: 1 }),
            FilterValue::Number(ExactNumber::Signed { width: 32, value: 257 }),
            FilterValue::Number(ExactNumber::Float32Bits(3.0f32.to_bits())),
        ] {
            let mut bad = current.clone();
            bad.arguments[0].value = value;
            assert!(PointStack::from_nodes(&[bad], profile, SampleEncoding::NonlinearProfile, 22).is_err());
        }
        current.operation.name = "gegl:posterize".into();
        assert!(PointStack::from_nodes(&[current], profile, SampleEncoding::NonlinearProfile, 22).is_err());
    }
    #[test]
    fn mono_mixer_requires_exact_bool_false_weights_and_rgb() {
        let mut current = node(1, "gegl:mono-mixer", &[("red", 0.4), ("green", 0.3), ("blue", 0.2)]);
        current.arguments.push(FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(false) });
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let valid = PointStack::from_nodes(&[current.clone()], profile, SampleEncoding::NonlinearProfile, 22).unwrap();
        assert!(valid.apply([0.1, 0.2, 0.3, 1.0], &mut |_| Ok(())).is_ok());
        for value in
            [FilterValue::Bool(true), FilterValue::Number(ExactNumber::Float32Bits(0)), FilterValue::Number(ExactNumber::Signed { width: 32, value: 0 })]
        {
            let mut bad = current.clone();
            bad.arguments[3].value = value;
            assert!(PointStack::from_nodes(&[bad], profile, SampleEncoding::NonlinearProfile, 22).is_err());
        }
        assert!(PointStack::from_nodes(&[current.clone()], photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, 22).is_err());
        current.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.8f32.to_bits()));
        assert!(PointStack::from_nodes(&[current], profile, SampleEncoding::NonlinearProfile, 22).is_err());
    }
    #[test]
    fn unknown_envelopes_types_versions_and_cancellation_fail_atomically() {
        let stack = FilterStack { owner: ItemRef::Layer(LayerId(1)), revision: 0, nodes: vec![node(1, "gegl:invert-linear", &[])] };
        let profile = photocraft_cms::Builtin::LinearSrgb.profile();
        for change in 0..6 {
            let mut bad = stack.clone();
            match change {
                0 => bad.nodes[0].source_clipping = None,
                1 => bad.nodes[0].composition.as_mut().unwrap().stored_operator = 1,
                2 => bad.nodes[0].opacity_f64_bits = 0.5_f64.to_bits(),
                3 => bad.nodes[0].operation.stored_version = "missing".into(),
                4 => bad.nodes[0].auxiliary.push(("aux".into(), ItemRef::Layer(LayerId(2)))),
                _ => bad.nodes[0].arguments.push(FilterArgument { name: "invented".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0)) }),
            }
            assert!(PointStack::new(&bad, profile, SampleEncoding::Linear, 22).is_err());
        }
        assert!(PointStack::new(&stack, profile, SampleEncoding::Linear, 23).is_err());
        let prepared = PointStack::new(&stack, profile, SampleEncoding::Linear, 22).unwrap();
        let pixel = [0.2, 0.3, 0.4, 1.0];
        assert!(prepared.apply(pixel, &mut |_| Err(CompositionError::Invalid("cancelled"))).is_err());
        assert_eq!(pixel, [0.2, 0.3, 0.4, 1.0]);
        assert!(prepared.apply([f64::NAN, 0.0, 0.0, 1.0], &mut |_| Ok(())).is_err());
    }
    #[test]
    fn witnessed_stack_rejects_unobserved_envelopes_and_is_cancellable() {
        let mut current = node(1, "gegl:invert-linear", &[]);
        current.operation.schema_epoch = STACK_EDITS_EPOCH.into();
        let policy = current.composition.as_mut().unwrap();
        policy.compatibility_epoch = STACK_EDITS_EPOCH.into();
        policy.stored_blend_space = -1178762671;
        policy.stored_composite_space = -1178762671;
        policy.stored_operator = -1178762671;
        let source = FilterStack { owner: ItemRef::Layer(LayerId(1)), revision: 0, nodes: vec![current] };
        let profile = photocraft_cms::Builtin::Srgb.profile();
        for change in 0..7 {
            let mut invalid = source.clone();
            match change {
                0 => invalid.nodes[0].opacity_f64_bits = 0.25f64.to_bits(),
                1 => invalid.nodes[0].composition.as_mut().unwrap().stored_operator = -261651887,
                2 => invalid.nodes[0].composition.as_mut().unwrap().compatibility_epoch = "unknown".into(),
                3 => invalid.nodes[0].operation.name = "gegl:invert-gamma".into(),
                4 => invalid.nodes[0].source_region = Some(1),
                5 => invalid.nodes[0].operation.stored_version = "1:0".into(),
                _ => {
                    invalid.nodes.push(invalid.nodes[0].clone());
                    invalid.nodes.push(invalid.nodes[0].clone());
                }
            }
            assert!(PointStack::new(&invalid, profile, SampleEncoding::NonlinearProfile, 22).is_err());
        }
        assert!(PointStack::new(&source, profile, SampleEncoding::Linear, 22).is_err());
        assert!(PointStack::new(&source, profile, SampleEncoding::NonlinearProfile, 23).is_err());
        let prepared = PointStack::new(&source, profile, SampleEncoding::NonlinearProfile, 22).unwrap();
        let input = [0.25, 0.5, 0.75, 0.5];
        assert!(prepared.apply(input, &mut |_| Err(CompositionError::Invalid("cancelled"))).is_err());
        assert_eq!(input, [0.25, 0.5, 0.75, 0.5]);
    }
    #[cfg(feature = "corpus")]
    #[test]
    #[ignore = "local unpublished independent GIMP stack-edit packet; run explicitly with corpus"]
    fn independent_point_stack_order_activation_remove_and_opacity() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("filter-stack-edits-evidence");
        let raw = std::fs::read(root.join("batch-manifest.json")).unwrap();
        let hex = |raw: &[u8]| Sha256::digest(raw).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(hex(&raw), "6427be73b8d8bca48f372293aa7e9899120dc20ee209ef5c14746072f4e54f70");
        let manifest: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            let raw = std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap();
            assert_eq!(hex(&raw), artifact["sha256"].as_str().unwrap());
        }
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        for record in observations["records"].as_array().unwrap() {
            let layer = &record["reopened"]["layers"][0];
            let mut nodes = Vec::new();
            for (index, effect) in layer["filters"].as_array().unwrap().iter().enumerate() {
                let operation = effect["operation"].as_str().unwrap();
                let mut current =
                    node(index as u64 + 1, operation, if operation == "gegl:exposure" { &[("exposure", 0.5), ("black-level", 0.0)] } else { &[] });
                current.enabled = effect["visible"].as_bool().unwrap();
                current.opacity_f64_bits = effect["opacity"].as_f64().unwrap().to_bits();
                current.operation.schema_epoch = STACK_EDITS_EPOCH.into();
                let policy = current.composition.as_mut().unwrap();
                policy.compatibility_epoch = "xcf-22-stock-point-stack-edits-v1".into();
                policy.stored_blend_space = -1178762671;
                policy.stored_composite_space = -1178762671;
                policy.stored_operator = -1178762671;
                nodes.push(current);
            }
            let stack = FilterStack { owner: ItemRef::Layer(LayerId(1)), revision: 0, nodes };
            let prepared = PointStack::new(&stack, photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
            let ordered = crate::gimp_neighborhood::OrderedStack::new(
                &stack,
                photocraft_cms::Builtin::Srgb.profile(),
                SampleEncoding::NonlinearProfile,
                22,
                photocraft_geom::Rect::from_xywh(0, 0, 8, 6),
                [false; 3],
            )
            .unwrap();
            let input = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let expected = std::fs::read(root.join(record["reopened"]["projection"]["file"].as_str().unwrap())).unwrap();
            let mut worst = 0.0f64;
            for (pixel, reference) in input.as_chunks::<4>().0.iter().zip(expected.as_chunks::<32>().0.iter()) {
                let color = [f64::from(pixel[0]) / 255.0, f64::from(pixel[1]) / 255.0, f64::from(pixel[2]) / 255.0, f64::from(pixel[3]) / 255.0];
                let actual = prepared.apply(color, &mut |_| Ok(())).unwrap();
                assert_eq!(ordered.point(color, &mut |_| Ok(())).unwrap(), actual, "full ordered stage scheduling must match the qualified stack");
                for (actual, bytes) in actual.iter().zip(reference.as_chunks::<8>().0.iter()) {
                    let expected = f64::from_le_bytes(*bytes);
                    // GIMP's visible U8 projection clips/quantizes output; current source remains exact.
                    worst = worst.max((actual.clamp(0.0, 1.0) - expected).abs());
                }
            }
            println!("{}: point stack {:.6}/255", record["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
        }
    }
}

#[cfg(all(test, feature = "corpus"))]
mod posterize_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent GIMP posterize originals; run explicitly with corpus"]
    fn independent_signed_posterize_source_and_edit() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations/gimp-posterize");
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("probe.json")).unwrap()).unwrap();
        let raw = std::fs::read(root.join("operation-gimp-posterize.source.0.native.bin")).unwrap();
        use sha2::Digest;
        assert_eq!(
            sha2::Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            proof["cases"][0]["state_reopened"]["layers"][0]["native"]["sha256"].as_str().unwrap()
        );
        for (levels, suffix, field) in [(3, "ref.png", "reference_sha256"), (4, "edit.ref.png", "reference_edit_render_sha256")] {
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gimp:posterize".into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-point-replace".into() },
                arguments: vec![FilterArgument { name: "levels".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: levels }) }],
                source_display_name: None,
                source_icon: None,
                enabled: true,
                opacity_f64_bits: 1.0f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: "xcf-22-stock-point-replace".into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -261651887,
                    stored_composite_space: -261651887,
                    stored_operator: -261651887,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(&[node], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
            let bytes = std::fs::read(root.join(format!("operation-gimp-posterize.{suffix}"))).unwrap();
            assert_eq!(sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), proof["cases"][0][field].as_str().unwrap());
            let expected = photocraft_codecs::decode(&bytes).unwrap().convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            let mut worst = 0.0f64;
            for (pixel, reference) in raw.as_chunks::<4>().0.iter().zip(expected.data().as_chunks::<4>().0.iter()) {
                let pixel = [f64::from(pixel[0]) / 255.0, f64::from(pixel[1]) / 255.0, f64::from(pixel[2]) / 255.0, f64::from(pixel[3]) / 255.0];
                let actual = stack.apply(pixel, &mut |_| Ok(())).unwrap();
                for (value, expected) in actual.into_iter().zip(reference) {
                    worst = worst.max((value - f64::from(*expected) / 255.0).abs());
                }
            }
            println!("posterize levels{levels} point stack {} /255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
        }
    }
}

#[cfg(all(test, feature = "corpus"))]
mod value_invert_oracle {
    use super::*;
    use photocraft_doc::{import_compositing::GimpComposition, item_identity::FilterId, xcf_filters::OperationIdentity};
    #[test]
    #[ignore = "local unpublished independent GIMP value invert originals; run explicitly with corpus"]
    fn independent_value_invert_source_and_disabled() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations/gegl-value-invert");
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("probe.json")).unwrap()).unwrap();
        let raw = std::fs::read(root.join("operation-gegl-value-invert.source.0.native.bin")).unwrap();
        use sha2::Digest;
        assert_eq!(
            sha2::Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            proof["cases"][0]["state_reopened"]["layers"][0]["native"]["sha256"].as_str().unwrap()
        );
        for (enabled, suffix, field) in [(true, "ref.png", "reference_sha256"), (false, "edit.ref.png", "reference_edit_render_sha256")] {
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity {
                    name: "gegl:value-invert".into(),
                    stored_version: "0:0".into(),
                    schema_epoch: "xcf-22-stock-point-replace".into(),
                },
                arguments: vec![],
                source_display_name: None,
                source_icon: None,
                enabled,
                opacity_f64_bits: 1.0f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: "xcf-22-stock-point-replace".into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -261651887,
                    stored_composite_space: -261651887,
                    stored_operator: -261651887,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(&[node], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
            let bytes = std::fs::read(root.join(format!("operation-gegl-value-invert.{suffix}"))).unwrap();
            assert_eq!(sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), proof["cases"][0][field].as_str().unwrap());
            let expected = photocraft_codecs::decode(&bytes).unwrap().convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            let mut worst = 0.0f64;
            for (pixel, reference) in raw.as_chunks::<4>().0.iter().zip(expected.data().as_chunks::<4>().0.iter()) {
                let pixel = [f64::from(pixel[0]) / 255.0, f64::from(pixel[1]) / 255.0, f64::from(pixel[2]) / 255.0, f64::from(pixel[3]) / 255.0];
                let actual = stack.apply(pixel, &mut |_| Ok(())).unwrap();
                for (value, expected) in actual.into_iter().zip(reference) {
                    worst = worst.max((value - f64::from(*expected) / 255.0).abs());
                }
            }
            println!("value invert enabled={enabled} point stack {} /255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
        }
    }
}

#[cfg(all(test, feature = "corpus"))]
mod mono_mixer_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent GIMP mono mixer originals; run explicitly with corpus"]
    fn independent_false_luminosity_mono_mixer_source() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations/gegl-mono-mixer");
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("probe.json")).unwrap()).unwrap();
        let raw = std::fs::read(root.join("operation-gegl-mono-mixer.source.0.native.bin")).unwrap();
        use sha2::Digest;
        assert_eq!(
            sha2::Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            proof["cases"][0]["state_reopened"]["layers"][0]["native"]["sha256"].as_str().unwrap()
        );
        for (enabled, suffix, field) in [(true, "ref.png", "reference_sha256")] {
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity {
                    name: "gegl:mono-mixer".into(),
                    stored_version: "0:0".into(),
                    schema_epoch: "xcf-22-stock-point-replace".into(),
                },
                arguments: vec![
                    FilterArgument { name: "red".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.333_f32.to_bits())) },
                    FilterArgument { name: "green".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.333_f32.to_bits())) },
                    FilterArgument { name: "blue".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.333_f32.to_bits())) },
                    FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(false) },
                ],
                source_display_name: None,
                source_icon: None,
                enabled,
                opacity_f64_bits: 1.0f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: "xcf-22-stock-point-replace".into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -261651887,
                    stored_composite_space: -261651887,
                    stored_operator: -261651887,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(&[node], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
            let bytes = std::fs::read(root.join(format!("operation-gegl-mono-mixer.{suffix}"))).unwrap();
            assert_eq!(sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), proof["cases"][0][field].as_str().unwrap());
            let expected = photocraft_codecs::decode(&bytes).unwrap().convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            let mut worst = 0.0f64;
            for (pixel, reference) in raw.as_chunks::<4>().0.iter().zip(expected.data().as_chunks::<4>().0.iter()) {
                let pixel = [f64::from(pixel[0]) / 255.0, f64::from(pixel[1]) / 255.0, f64::from(pixel[2]) / 255.0, f64::from(pixel[3]) / 255.0];
                let actual = stack.apply(pixel, &mut |_| Ok(())).unwrap();
                for (value, expected) in actual.into_iter().zip(reference) {
                    worst = worst.max((value - f64::from(*expected) / 255.0).abs());
                }
            }
            println!("mono mixer enabled={enabled} point stack {} /255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
        }
    }
    #[test]
    #[ignore = "local unpublished independent GIMP false-luminosity weight source/edit originals"]
    fn independent_mono_mixer_weight_source_and_edit() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-edits-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "37935d87fac7c0e8aed14f326c4c39eeedd2e450fb6a7074d90c75d34dcb84f2"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        for stage in ["source", "weights"] {
            let prefix = format!("mono.{stage}.fresh");
            let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.state.json"))).unwrap()).unwrap();
            let config = &state["layers"][0]["filters"][0]["config"];
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:mono-mixer".into(), stored_version: "0:0".into(), schema_epoch: MONO_MIXER_EDITS_EPOCH.into() },
                arguments: vec![
                    FilterArgument {
                        name: "red".into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((config["red"].as_f64().unwrap() as f32).to_bits())),
                    },
                    FilterArgument {
                        name: "green".into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((config["green"].as_f64().unwrap() as f32).to_bits())),
                    },
                    FilterArgument {
                        name: "blue".into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((config["blue"].as_f64().unwrap() as f32).to_bits())),
                    },
                    FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(false) },
                ],
                source_display_name: None,
                source_icon: None,
                enabled: true,
                opacity_f64_bits: 1.0f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: MONO_MIXER_EDITS_EPOCH.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: 107971153,
                    stored_composite_space: 107971153,
                    stored_operator: 107971153,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(&[node], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
            let native = std::fs::read(root.join(format!("{prefix}.layer0.bin"))).unwrap();
            let projection = std::fs::read(root.join(format!("{prefix}.projection.bin"))).unwrap();
            assert_eq!(native.len(), 4096);
            assert_eq!(projection.len(), 32768);
            let mut worst = 0.0_f64;
            for (pixel, expected) in native.as_chunks::<4>().0.iter().zip(projection.as_chunks::<32>().0.iter()) {
                let pixel: [f64; 4] = std::array::from_fn(|index| f64::from(pixel[index]) / 255.0);
                let actual = stack.apply(pixel, &mut |_| Ok(())).unwrap();
                for (actual, expected) in actual.into_iter().zip(expected.as_chunks::<8>().0.iter()) {
                    worst = worst.max((actual - f64::from_le_bytes(*expected)).abs());
                }
            }
            println!("MonoMixer {stage} private consumer {} /255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
        }
    }
}
#[cfg(all(test, feature = "corpus"))]
mod rgb_clip_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independently authored finite HDR RGB Clip references"]
    fn independent_hdr_rgb_clip_source_limits_and_boolean_edit() {
        use sha2::Digest;
        const EPOCH: &str = "xcf-22-stock-rgb-clip-hdr-v1";
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("rgb-clip-hdr-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "45f79892e29ebb9bd3ff07a9c6231063bfe4028e86e3a79562a7daab85fa9532"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let encoded = photocraft_cms::Transform::new(
            photocraft_cms::Builtin::LinearSrgb.profile(),
            photocraft_cms::Builtin::Srgb.profile(),
            photocraft_cms::Intent::RelativeColorimetric,
            false,
        )
        .unwrap();
        for stage in ["source", "limits", "low-disabled"] {
            let prefix = format!("clip.{stage}.fresh");
            let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.state.json"))).unwrap()).unwrap();
            let config = &state["layers"][0]["filters"][0]["config"];
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:rgb-clip".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() },
                arguments: ["clip-low", "low-limit", "clip-high", "high-limit"]
                    .into_iter()
                    .map(|name| FilterArgument {
                        name: name.into(),
                        value: if name.starts_with("clip-") {
                            FilterValue::Bool(config[name].as_bool().unwrap())
                        } else {
                            FilterValue::Number(ExactNumber::Float32Bits((config[name].as_f64().unwrap() as f32).to_bits()))
                        },
                    })
                    .collect(),
                source_display_name: None,
                source_icon: None,
                enabled: true,
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: EPOCH.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: 1825931857,
                    stored_composite_space: 1825931857,
                    stored_operator: 1825931857,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Linear, 22).unwrap();
            let native = std::fs::read(root.join(format!("{prefix}.layer0.bin"))).unwrap();
            let projection = std::fs::read(root.join(format!("{prefix}.projection.bin"))).unwrap();
            let mut worst = 0.0_f64;
            for (pixel, reference) in native.as_chunks::<16>().0.iter().zip(projection.as_chunks::<32>().0.iter()) {
                let input: [f64; 4] = std::array::from_fn(|i| f64::from(f32::from_le_bytes(pixel[i * 4..i * 4 + 4].try_into().unwrap())));
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), input[3].to_bits());
                let mut colors = [0.0; 3];
                encoded.try_eval_f64(&actual[..3], &mut colors).unwrap();
                for (actual, reference) in colors.into_iter().chain([actual[3]]).zip(reference.as_chunks::<8>().0.iter()) {
                    worst = worst.max((actual - f64::from_le_bytes(*reference)).abs());
                }
            }
            println!("RGB Clip {stage} private consumer {} /255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            for invalid in [FilterValue::Number(ExactNumber::Float32Bits(0)), FilterValue::Number(ExactNumber::Signed { width: 32, value: 0 })] {
                let mut bad = node.clone();
                bad.arguments[0].value = invalid;
                assert!(PointStack::from_nodes(&[bad], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Linear, 22).is_err());
            }
            let mut bad = node.clone();
            bad.arguments[2].value = FilterValue::Bool(false);
            assert!(PointStack::from_nodes(&[bad], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Linear, 22).is_err());
            assert!(PointStack::from_nodes(&[node], photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).is_err());
        }
    }
}
#[cfg(all(test, feature = "corpus"))]
mod value_precision_oracle {
    use super::*;
    use photocraft_doc::{import_compositing::GimpComposition, item_identity::FilterId, xcf_filters::OperationIdentity};
    #[test]
    #[ignore = "local unpublished independently authored Value Invert native precision and alpha"]
    fn independent_value_invert_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("value-invert-precision-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "8a75ccbb88e61611d3c3523db8145f922b63676601986c124d2d8493a27e73a1"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for record in proof["checks"].as_array().unwrap() {
            let state = &record["fresh"];
            let precision = state["precision"].as_str().unwrap();
            let stored = if precision == "GIMP_PRECISION_FLOAT_LINEAR" { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let decode = |raw: &[u8]| -> f64 {
                match width {
                    2 => f64::from(u16::from_le_bytes(raw.try_into().unwrap())) / 65535.0,
                    4 => f64::from(f32::from_le_bytes(raw.try_into().unwrap())),
                    _ => f64::from_le_bytes(raw.try_into().unwrap()),
                }
            };
            let epoch = "xcf-22-stock-value-invert-precision-v1";
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:value-invert".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
                arguments: vec![],
                source_display_name: None,
                source_icon: None,
                enabled: state["layers"][0]["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -1395293615,
                    stored_composite_space: -1395293615,
                    stored_operator: -1395293615,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), photocraft_cms::Builtin::Srgb.profile(), stored, 22).unwrap();
            let native = std::fs::read(root.join(state["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(record["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(native.len(), 48 * 4 * width);
            assert_eq!(native.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in native.chunks_exact(width * 4).zip(reference.chunks_exact(width * 4)) {
                let input: [f64; 4] = std::array::from_fn(|i| decode(&pixel[i * width..(i + 1) * width]));
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), input[3].to_bits());
                for (actual, expected) in actual.iter().zip(expected.chunks_exact(width)) {
                    worst = worst.max((actual - decode(expected)).abs());
                }
            }
            println!("{} ValueInvert private bakednative {}/255", record["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut bad = node.clone();
            bad.operation.schema_epoch = "unqualified".into();
            bad.enabled = true;
            assert!(PointStack::from_nodes(&[bad], photocraft_cms::Builtin::Srgb.profile(), stored, 22).is_err());
        }
    }
}

#[cfg(all(test, feature = "corpus"))]
mod brightness_precision_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent Brightness Contrast precision originals"]
    fn independent_brightness_contrast_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("brightness-contrast-precision-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "42f310ede6a2d620f792425e4c8796f905e95cdaf85614b58bafd24d4a08162c"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let state = &row["fresh"];
            let layer = &state["layers"][0];
            let precision = state["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
            let channels = if gray { 1 } else { 3 };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let epoch = "xcf-22-stock-brightness-contrast-precision-v1";
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:brightness-contrast".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
                arguments: ["brightness", "contrast"]
                    .into_iter()
                    .map(|name| FilterArgument {
                        name: name.into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((layer["filters"][0]["config"][name].as_f64().unwrap() as f32).to_bits())),
                    })
                    .collect(),
                source_display_name: None,
                source_icon: None,
                enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: 214598225,
                    stored_composite_space: 214598225,
                    stored_operator: 214598225,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
            let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let decode = |bytes: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                _ => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            assert_eq!(source.len(), 48 * (channels + 1) * width);
            assert_eq!(source.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)) {
                let color = decode(&pixel[..width]);
                let alpha = decode(&pixel[channels * width..]);
                let input =
                    if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), alpha.to_bits());
                assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
                for (channel, expected) in expected.chunks_exact(width).enumerate() {
                    let value = if channel == channels { actual[3] } else { actual[channel] };
                    let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                    worst = worst.max((value - decode(expected)).abs());
                }
            }
            println!("{} Brightness Contrast private native {}/255", row["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut invalid = node.clone();
            invalid.enabled = true;
            invalid.operation.schema_epoch = "unqualified".into();
            assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        }
    }
}

#[cfg(test)]
mod gegl_levels_precision_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent GEGL Levels precision originals"]
    fn independent_gegl_levels_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("gegl-levels-precision-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "3e2270ebeb0f566fc99c1f3062c429279fcd1d0c8bc37e490835376f8abab7b3"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let state = &row["fresh"];
            let layer = &state["layers"][0];
            let precision = state["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
            let channels = if gray { 1 } else { 3 };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let epoch = "xcf-22-stock-gegl-levels-precision-v1";
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:levels".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
                arguments: ["in-low", "in-high", "out-low", "out-high"]
                    .into_iter()
                    .map(|name| FilterArgument {
                        name: name.into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((layer["filters"][0]["config"][name].as_f64().unwrap() as f32).to_bits())),
                    })
                    .collect(),
                source_display_name: None,
                source_icon: None,
                enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: 392856145,
                    stored_composite_space: 392856145,
                    stored_operator: 392856145,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
            let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let decode = |bytes: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                _ => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            assert_eq!(source.len(), 48 * (channels + 1) * width);
            assert_eq!(source.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)) {
                let color = decode(&pixel[..width]);
                let alpha = decode(&pixel[channels * width..]);
                let input =
                    if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), alpha.to_bits());
                assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
                for (channel, expected) in expected.chunks_exact(width).enumerate() {
                    let value = if channel == channels { actual[3] } else { actual[channel] };
                    let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                    worst = worst.max((value - decode(expected)).abs());
                }
            }
            println!("{} GEGL Levels private native {}/255", row["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut invalid = node.clone();
            invalid.enabled = true;
            invalid.operation.schema_epoch = "unqualified".into();
            assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        }
    }
}

#[cfg(test)]
mod exposure_black_precision_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent Exposure black-level precision originals"]
    fn independent_exposure_black_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("exposure-black-level-precision-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "40bb80b27ea1a596309e723c9e4392fbf1f7d07aa56fe291e67c9eab077515eb"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let state = &row["fresh"];
            let layer = &state["layers"][0];
            let precision = state["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
            let channels = if gray { 1 } else { 3 };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let epoch = "xcf-22-stock-exposure-black-level-precision-v1";
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: epoch.into() },
                arguments: ["black-level", "exposure"]
                    .into_iter()
                    .map(|name| FilterArgument {
                        name: name.into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((layer["filters"][0]["config"][name].as_f64().unwrap() as f32).to_bits())),
                    })
                    .collect(),
                source_display_name: None,
                source_icon: None,
                enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: 706183761,
                    stored_composite_space: 706183761,
                    stored_operator: 706183761,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
            let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let decode = |bytes: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                _ => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            assert_eq!(source.len(), 48 * (channels + 1) * width);
            assert_eq!(source.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)) {
                let color = decode(&pixel[..width]);
                let alpha = decode(&pixel[channels * width..]);
                let input =
                    if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), alpha.to_bits());
                assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
                for (channel, expected) in expected.chunks_exact(width).enumerate() {
                    let value = if channel == channels { actual[3] } else { actual[channel] };
                    let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                    worst = worst.max((value - decode(expected)).abs());
                }
            }
            println!("{} Exposure black-level private native {}/255", row["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut invalid = node.clone();
            invalid.enabled = true;
            invalid.operation.schema_epoch = "unqualified".into();
            assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        }
    }
}

#[cfg(test)]
mod color_temperature_precision_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent Color Temperature precision originals"]
    fn independent_color_temperature_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-temperature-precision-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "f727e979471d42eff4535cda98f5adcef9f43cf0f47578dae99a43f2fb104b3c"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let state = &row["fresh"];
            let layer = &state["layers"][0];
            let precision = state["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
            let channels = if gray { 1 } else { 3 };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let epoch = "xcf-22-stock-color-temperature-precision-v1";
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:color-temperature".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
                arguments: ["original-temperature", "intended-temperature"]
                    .into_iter()
                    .map(|name| FilterArgument {
                        name: name.into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((layer["filters"][0]["config"][name].as_f64().unwrap() as f32).to_bits())),
                    })
                    .collect(),
                source_display_name: None,
                source_icon: None,
                enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -113999279,
                    stored_composite_space: -113999279,
                    stored_operator: -113999279,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
            let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let decode = |bytes: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                _ => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            assert_eq!(source.len(), 48 * (channels + 1) * width);
            assert_eq!(source.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)) {
                let color = decode(&pixel[..width]);
                let alpha = decode(&pixel[channels * width..]);
                let input =
                    if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), alpha.to_bits());
                assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
                for (channel, expected) in expected.chunks_exact(width).enumerate() {
                    let value = if channel == channels { actual[3] } else { actual[channel] };
                    let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                    worst = worst.max((value - decode(expected)).abs());
                }
            }
            println!("{} Color Temperature private native {}/255", row["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut invalid = node.clone();
            invalid.enabled = true;
            invalid.operation.schema_epoch = "unqualified".into();
            assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        }
    }
}

#[cfg(test)]
mod color_temperature_offgrid_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent Color Temperature precision originals"]
    fn independent_color_temperature_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-temperature-offgrid-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "8105afb381c5178a375138b3ee6de73f477bf06c3a16d589a0373fc6b76e57de"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let packet: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        let proof = serde_json::json!({"checks":packet["checks"].as_array().unwrap().iter().map(|row| serde_json::json!({"id":format!("{}.edit",row["id"].as_str().unwrap()),"fresh":row["checks"]["edit"]["fresh"],"baked":row["checks"]["edit"]["baked"]})).collect::<Vec<_>>()});
        for row in proof["checks"].as_array().unwrap() {
            let state = &row["fresh"];
            let layer = &state["layers"][0];
            let precision = state["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
            let channels = if gray { 1 } else { 3 };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let epoch = "xcf-22-stock-color-temperature-offgrid-v1";
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:color-temperature".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
                arguments: ["original-temperature", "intended-temperature"]
                    .into_iter()
                    .map(|name| FilterArgument {
                        name: name.into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((layer["filters"][0]["config"][name].as_f64().unwrap() as f32).to_bits())),
                    })
                    .collect(),
                source_display_name: None,
                source_icon: None,
                enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -1716420015,
                    stored_composite_space: -1716420015,
                    stored_operator: -1716420015,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
            let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let decode = |bytes: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                _ => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            assert_eq!(source.len(), 48 * (channels + 1) * width);
            assert_eq!(source.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)) {
                let color = decode(&pixel[..width]);
                let alpha = decode(&pixel[channels * width..]);
                let input =
                    if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), alpha.to_bits());
                assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
                for (channel, expected) in expected.chunks_exact(width).enumerate() {
                    let value = if channel == channels { actual[3] } else { actual[channel] };
                    let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                    worst = worst.max((value - decode(expected)).abs());
                }
            }
            println!("{} Color Temperature private native {}/255", row["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut invalid = node.clone();
            invalid.enabled = true;
            invalid.operation.schema_epoch = "unqualified".into();
            assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        }
    }
}

#[cfg(test)]
mod desaturate_precision_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent Desaturate precision originals"]
    fn independent_desaturate_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("desaturate-precision-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "ff2d1ce5981edb0b3d4b5c22e01e8af015945160062d9b057d997e720f231603"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let state = &row["fresh"];
            let layer = &state["layers"][0];
            let precision = state["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
            let channels = if gray { 1 } else { 3 };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let epoch = "xcf-22-stock-desaturate-precision-v1";
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gimp:desaturate".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
                arguments: vec![FilterArgument {
                    name: "mode".into(),
                    value: FilterValue::Enum { domain: "GimpDesaturateMode".into(), value: layer["filters"][0]["config"]["mode"].as_i64().unwrap() as i32 },
                }],
                source_display_name: None,
                source_icon: None,
                enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: 1789100625,
                    stored_composite_space: 1789100625,
                    stored_operator: 1789100625,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
            let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let decode = |bytes: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                _ => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            assert_eq!(source.len(), 48 * (channels + 1) * width);
            assert_eq!(source.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)) {
                let color = decode(&pixel[..width]);
                let alpha = decode(&pixel[channels * width..]);
                let input =
                    if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), alpha.to_bits());
                assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
                for (channel, expected) in expected.chunks_exact(width).enumerate() {
                    let value = if channel == channels { actual[3] } else { actual[channel] };
                    let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                    worst = worst.max((value - decode(expected)).abs());
                }
            }
            println!("{} Desaturate private native {}/255", row["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut invalid = node.clone();
            invalid.enabled = true;
            invalid.operation.schema_epoch = "unqualified".into();
            assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        }
    }
}

#[cfg(test)]
mod saturation_precision_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    #[test]
    #[ignore = "local unpublished independent Saturation precision originals"]
    fn independent_saturation_precision_alpha_baked_native() {
        use sha2::Digest;
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("saturation-precision-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "1c691b62017f8deed94872939d89cb90ad087b813473c01f3eb1664d1ddaab9c"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for artifact in manifest["artifacts"].as_array().unwrap() {
            assert_eq!(
                sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                artifact["sha256"].as_str().unwrap()
            );
        }
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let state = &row["fresh"];
            let layer = &state["layers"][0];
            if !layer["native_format"].as_str().unwrap().starts_with('Y') && layer["filters"][0]["config"]["colorspace"] == "CIE-Yuv" {
                continue;
            }
            let precision = state["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let gray = layer["native_format"].as_str().unwrap().starts_with('Y');
            let channels = if gray { 1 } else { 3 };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let epoch = if gray { SATURATION_GRAY_PRECISION_EPOCH } else { SATURATION_PRECISION_EPOCH };
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "gegl:saturation".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
                arguments: vec![
                    FilterArgument {
                        name: "scale".into(),
                        value: FilterValue::Number(ExactNumber::Float32Bits((layer["filters"][0]["config"]["scale"].as_f64().unwrap() as f32).to_bits())),
                    },
                    FilterArgument {
                        name: "colorspace".into(),
                        value: FilterValue::Enum {
                            domain: "GeglSaturationType".into(),
                            value: match layer["filters"][0]["config"]["colorspace"].as_str().unwrap() {
                                "Native" => 0,
                                "CIE-Lab" => 1,
                                "CIE-Yuv" => 2,
                                _ => panic!("unexpected observed choice"),
                            },
                        },
                    },
                ],
                source_display_name: None,
                source_icon: None,
                enabled: layer["filters"][0]["visible"].as_bool().unwrap(),
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: epoch.into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: -1581350319,
                    stored_composite_space: -1581350319,
                    stored_operator: -1581350319,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, stored, 22).unwrap();
            let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
            let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            let decode = |bytes: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                _ => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            assert_eq!(source.len(), 48 * (channels + 1) * width);
            assert_eq!(source.len(), reference.len());
            let mut worst = 0.0_f64;
            for (pixel, expected) in source.chunks_exact((channels + 1) * width).zip(reference.chunks_exact((channels + 1) * width)) {
                let color = decode(&pixel[..width]);
                let alpha = decode(&pixel[channels * width..]);
                let input =
                    if gray { [color, color, color, alpha] } else { [color, decode(&pixel[width..2 * width]), decode(&pixel[2 * width..3 * width]), alpha] };
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), alpha.to_bits());
                assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
                for (channel, expected) in expected.chunks_exact(width).enumerate() {
                    let value = if channel == channels { actual[3] } else { actual[channel] };
                    let value = if width == 2 { value.clamp(0.0, 1.0) } else { value };
                    worst = worst.max((value - decode(expected)).abs());
                }
            }
            println!("{} Saturation private native {}/255", row["id"], worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
            let mut invalid = node.clone();
            invalid.enabled = true;
            invalid.operation.schema_epoch = "unqualified".into();
            assert!(PointStack::from_nodes(&[invalid], profile, stored, 22).is_err());
        }
    }
}

#[cfg(test)]
mod threshold_precision_oracle {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node(channel: i32, low: f32, high: f32, enabled: bool) -> FilterNode {
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:threshold".into(), stored_version: "0:0".into(), schema_epoch: THRESHOLD_PRECISION_EPOCH.into() },
            arguments: vec![
                FilterArgument { name: "channel".into(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: channel } },
                FilterArgument { name: "low".into(), value: FilterValue::Number(ExactNumber::Float32Bits(low.to_bits())) },
                FilterArgument { name: "high".into(), value: FilterValue::Number(ExactNumber::Float32Bits(high.to_bits())) },
            ],
            enabled,
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: THRESHOLD_PRECISION_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -1565818287,
                stored_composite_space: -1565818287,
                stored_operator: -1565818287,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    fn threshold_closed_types_epoch_and_gray_domain() {
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let valid = node(0, 0.25, 0.75, true);
        let stack = PointStack::from_nodes(std::slice::from_ref(&valid), profile, SampleEncoding::NonlinearProfile, 22).unwrap();
        assert_eq!(stack.apply([0.5, 0.25, 0.125, 0.4], &mut |_| Ok(())).unwrap(), [1., 1., 1., 0.4]);
        for value in [
            FilterValue::Number(ExactNumber::Float32Bits(0)),
            FilterValue::Enum { domain: "Unknown".into(), value: 0 },
            FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 6 },
        ] {
            let mut invalid = valid.clone();
            invalid.arguments[0].value = value;
            assert!(PointStack::from_nodes(&[invalid], profile, SampleEncoding::NonlinearProfile, 22).is_err());
        }
        let mut wrong = valid.clone();
        wrong.operation.schema_epoch = "unknown".into();
        assert!(PointStack::from_nodes(&[wrong], profile, SampleEncoding::NonlinearProfile, 22).is_err());
        assert!(PointStack::from_nodes(&[valid], photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, 22).is_err());
    }
    #[test]
    #[ignore = "local unpublished independent RGB Threshold references"]
    fn independent_threshold_rgb48_native_and_typed_envelopes() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("threshold-precision-evidence");
        let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
        assert_eq!(
            Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142"
        );
        let hashes: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut count = 0;
        for row in proof["records"].as_array().unwrap().iter().filter(|r| r["model"] == "RGB") {
            let precision = row["precision"].as_str().unwrap();
            let width = if precision.contains("U16") {
                2
            } else if precision.contains("FLOAT") {
                4
            } else {
                8
            };
            let stored =
                if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
            let n = node(
                row["requested"]["channel"].as_i64().unwrap() as i32,
                row["requested"]["low"].as_f64().unwrap() as f32,
                row["requested"]["high"].as_f64().unwrap() as f32,
                row["requested"]["enabled"].as_bool().unwrap(),
            );
            let stack = PointStack::from_nodes(&[n], photocraft_cms::Builtin::Srgb.profile(), stored, 22).unwrap();
            let read = |role: &str| {
                let file = row[role]["layers"][0]["native"]["file"].as_str().unwrap();
                let data = std::fs::read(root.join(file)).unwrap();
                assert_eq!(Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect::<String>(), hashes[file]);
                data
            };
            let source = read("reopened");
            let reference = read("baked");
            assert_eq!(source.len(), 48 * 4 * width);
            assert_eq!(source.len(), reference.len());
            let decode = |b: &[u8]| match width {
                2 => f64::from(u16::from_le_bytes(b.try_into().unwrap())) / 65535.,
                4 => f64::from(f32::from_le_bytes(b.try_into().unwrap())),
                _ => f64::from_le_bytes(b.try_into().unwrap()),
            };
            for (pixel, expected) in source.chunks_exact(width * 4).zip(reference.chunks_exact(width * 4)) {
                let input = std::array::from_fn(|i| decode(&pixel[i * width..(i + 1) * width]));
                let actual = stack.apply(input, &mut |_| Ok(())).unwrap();
                assert_eq!(actual[3].to_bits(), input[3].to_bits());
                for (value, expected) in actual.iter().zip(expected.chunks_exact(width)) {
                    assert!((value - decode(expected)).abs() <= 1. / 65535., "{} {value} vs {}", row["id"], decode(expected));
                }
            }
            count += 1;
        }
        assert_eq!(count, 48);
    }
}

fn typed_alien_map(node: &FilterNode, expected_model: i32) -> bool {
    const NAMES: [&str; 10] = [
        "color-model",
        "cpn-1-frequency",
        "cpn-2-frequency",
        "cpn-3-frequency",
        "cpn-1-phaseshift",
        "cpn-2-phaseshift",
        "cpn-3-phaseshift",
        "cpn-1-keep",
        "cpn-2-keep",
        "cpn-3-keep",
    ];
    node.arguments.len() == 10
        && NAMES.iter().all(|name| {
            let mut found = node.arguments.iter().filter(|a| a.name == *name);
            let Some(arg) = found.next() else {
                return false;
            };
            if found.next().is_some() {
                return false;
            }
            match &arg.value {
                FilterValue::Enum { domain, value } if *name == "color-model" => domain == "gegl:alien-map/color-model" && *value == expected_model,
                FilterValue::Bool(_) if name.ends_with("-keep") => true,
                FilterValue::Number(ExactNumber::Float32Bits(bits)) if name.ends_with("-frequency") || name.ends_with("-phaseshift") => {
                    let v = f32::from_bits(*bits);
                    v.is_finite() && (0.0..=if name.ends_with("-frequency") { 20.0 } else { 360.0 }).contains(&v)
                }
                _ => false,
            }
        })
}

#[cfg(test)]
#[path = "gimp_point_invert_linear_tests.rs"]
mod invert_linear_native_tests;

#[cfg(test)]
#[path = "gimp_point_invert_gamma_tests.rs"]
mod invert_gamma_native_tests;
