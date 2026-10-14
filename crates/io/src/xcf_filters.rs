//! Scoped lossless mapping of independently checked stock point-filter records.
use crate::IoError;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
};
use photocraft_xcf::{ArgumentValue, Effect};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointReplaceFamily {
    Brightness,
    Levels,
    Exposure,
    Temperature,
    Sepia,
    AlienRgb,
    AlienHsl,
    AbsoluteLinear,
    InvertLinear,
    InvertGamma,
    ColorExchange,
    ColorOverlay,
    ChannelMixer,
    MonoMixerSigned,
    RedEye,
    ColorRotate,
    VideoDegradation,
    ColorToAlpha,
    ComponentExtract,
    ComponentExtractCmyk,
    ComponentExtractYcbcr,
    ComponentExtractGeometry,
    AlphaClip,
    AlphaClipFinite,
}
impl PointReplaceFamily {
    fn domain(self) -> NeighborhoodDomain {
        match self {
            Self::Brightness => NeighborhoodDomain::BrightnessContrastPrecision,
            Self::Levels => NeighborhoodDomain::GeglLevelsPrecision,
            Self::Exposure => NeighborhoodDomain::ExposureBlackPrecision,
            Self::Temperature => NeighborhoodDomain::ColorTemperaturePrecision,
            Self::Sepia => NeighborhoodDomain::SepiaPrecision,
            Self::AlienRgb => NeighborhoodDomain::AlienMapPrecision,
            Self::AlienHsl => NeighborhoodDomain::AlienMapHslPrecision,
            Self::AbsoluteLinear => NeighborhoodDomain::AbsoluteLinearPrecision,
            Self::InvertLinear => NeighborhoodDomain::InvertLinearPrecision,
            Self::InvertGamma => NeighborhoodDomain::InvertGammaPrecision,
            Self::ColorExchange => NeighborhoodDomain::ColorExchangePrecision,
            Self::ColorOverlay => NeighborhoodDomain::ColorOverlayPrecision,
            Self::ChannelMixer => NeighborhoodDomain::ChannelMixerPrecision,
            Self::MonoMixerSigned => NeighborhoodDomain::MonoMixerSignedPrecision,
            Self::RedEye => NeighborhoodDomain::RedEyePrecision,
            Self::ColorRotate => NeighborhoodDomain::ColorRotatePerceptual,
            Self::VideoDegradation => NeighborhoodDomain::VideoDegradationFloat,
            Self::ColorToAlpha => NeighborhoodDomain::ColorToAlphaPrecision,
            Self::ComponentExtract => NeighborhoodDomain::ComponentExtractPrecision,
            Self::ComponentExtractCmyk => NeighborhoodDomain::ComponentExtractCmykPrecision,
            Self::ComponentExtractYcbcr => NeighborhoodDomain::ComponentExtractYcbcrPrecision,
            Self::ComponentExtractGeometry => NeighborhoodDomain::ComponentExtractGeometryPrecision,
            Self::AlphaClip => NeighborhoodDomain::AlphaClipPrecision,
            Self::AlphaClipFinite => NeighborhoodDomain::AlphaClipFinitePrecision,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NeighborhoodDomain {
    PointReplace(PointReplaceFamily),
    UnpremultiplyRgbFloatLinear,
    AbsoluteLinearPrecision,
    InvertLinearPrecision,
    InvertGammaPrecision,
    ColorExchangePrecision,
    ColorOverlayPrecision,
    ChannelMixerPrecision,
    MonoMixerSignedPrecision,
    RedEyePrecision,
    ColorRotatePerceptual,
    VideoDegradationFloat,
    ColorToAlphaPrecision,
    ComponentExtractPrecision,
    ComponentExtractCmykPrecision,
    ComponentExtractYcbcrPrecision,
    ComponentExtractGeometryPrecision,
    AlphaClipPrecision,
    AlphaClipFinitePrecision,
    Unqualified,
    PointStackEdits,
    PosterizeOriginal,
    ValueInvertOriginal,
    ValueInvertPrecision,
    BrightnessContrastPrecision,
    GeglLevelsPrecision,
    ExposureBlackPrecision,
    ExposureGeometry,
    ColorBalancePrecision,
    ColorBalanceDepth,
    ColorTemperaturePrecision,
    DesaturatePrecision,
    SaturationPrecision,
    SaturationGrayPrecision,
    SaturationRgbYuvPrecision,
    SepiaPrecision,
    AlienMapPrecision,
    AlienMapHslPrecision,
    ThresholdPrecision,
    ThresholdAlphaPrecision,
    MonoMixerOriginal,
    MonoMixerEditsOriginal,
    RgbClipHdr,
    RasterOriginal,
    RasterFinite,
    IsolatedSmallAuto,
    BoxOriginal,
    BoxFinite,
    MedianOriginal,
    MedianFinite,
    MedianGray,
    MotionOriginal,
    PixelizeOriginal,
}

fn unsupported<P>(effect: &Effect<P>, detail: &str) -> IoError {
    IoError::Unsupported(format!("XCF filter {:?}: {detail}", effect.operation))
}

/// No allocation: exact observed literal wire schema before source admission.
pub(crate) fn color_overlay_parameters<P>(effect: &Effect<P>) -> Option<([f32; 4], bool)> {
    if effect.operation != "gegl:color-overlay" || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 2 {
        return None;
    }
    let (mut color, mut srgb) = (None, None);
    for argument in &effect.arguments {
        if argument.configuration.is_some() || argument.graph.is_some() {
            return None;
        }
        match (argument.name.as_str(), &argument.value) {
            ("value", ArgumentValue::Color { encoding, samples, icc })
                if color.is_none() && encoding == "R'G'B'A float" && samples.len() == 16 && icc.is_empty() =>
            {
                let mut values = [0.; 4];
                for (destination, raw) in values.iter_mut().zip(samples.as_chunks::<4>().0.iter()) {
                    *destination = f32::from_le_bytes(*raw);
                }
                if values.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                    return None;
                }
                color = Some(values);
            }
            ("srgb", ArgumentValue::Boolean(value)) if srgb.is_none() => srgb = Some(*value),
            _ => return None,
        }
    }
    Some((color?, srgb?))
}

fn color_overlay_arguments<P>(effect: &Effect<P>) -> Result<Vec<FilterArgument>, IoError> {
    let (color, srgb) = color_overlay_parameters(effect).ok_or_else(|| unsupported(effect, "Color Overlay literal schema"))?;
    let mut result = Vec::new();
    result.try_reserve_exact(2).map_err(|_| unsupported(effect, "Color Overlay argument allocation"))?;
    for argument in &effect.arguments {
        let value = if argument.name == "value" {
            let mut components = Vec::new();
            components.try_reserve_exact(4).map_err(|_| unsupported(effect, "Color Overlay component allocation"))?;
            components.extend(color.into_iter().map(|v| ExactNumber::Float32Bits(v.to_bits())));
            FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None }
        } else {
            FilterValue::Bool(srgb)
        };
        result.push(FilterArgument { name: argument.name.clone(), value });
    }
    Ok(result)
}

fn color_exchange_arguments<P>(effect: &Effect<P>) -> Result<Vec<FilterArgument>, IoError> {
    if effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 5 {
        return Err(unsupported(effect, "Color Exchange version/count"));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(5).map_err(|_| unsupported(effect, "Color Exchange allocation"))?;
    let mut colors = [[0.; 4]; 2];
    let mut thresholds = [0.; 3];
    for argument in &effect.arguments {
        if result.iter().any(|value: &FilterArgument| value.name == argument.name) {
            return Err(unsupported(effect, "Color Exchange duplicate argument"));
        }
        let value = match (argument.name.as_str(), &argument.value) {
            (name @ ("from-color" | "to-color"), ArgumentValue::Color { encoding, samples, icc })
                if encoding == "R'G'B'A float" && samples.len() == 16 && icc.is_empty() =>
            {
                let mut components = Vec::new();
                components.try_reserve_exact(4).map_err(|_| unsupported(effect, "Color Exchange allocation"))?;
                let target = if name == "from-color" { &mut colors[0] } else { &mut colors[1] };
                for (target, raw) in target.iter_mut().zip(samples.as_chunks::<4>().0.iter()) {
                    let bits = u32::from_le_bytes(*raw);
                    *target = f32::from_bits(bits);
                    components.push(ExactNumber::Float32Bits(bits));
                }
                FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None }
            }
            (name @ ("red-threshold" | "green-threshold" | "blue-threshold"), ArgumentValue::Float32Bits(bits)) => {
                let target = match name {
                    "red-threshold" => &mut thresholds[0],
                    "green-threshold" => &mut thresholds[1],
                    _ => &mut thresholds[2],
                };
                *target = f32::from_bits(*bits);
                FilterValue::Number(ExactNumber::Float32Bits(*bits))
            }
            _ => return Err(unsupported(effect, "Color Exchange literal encoding/profile or threshold type")),
        };
        result.push(FilterArgument { name: argument.name.clone(), value });
    }
    photocraft_algo::gimp::registry::color_exchange_member(effect.version.as_deref(), colors[0], colors[1], thresholds)
        .map_err(|_| unsupported(effect, "Color Exchange finite unit parameters"))?;
    Ok(result)
}

#[path = "xcf_filters/configured_source_candidate.rs"]
mod configured_source_candidate;
pub use configured_source_candidate::{
    CurvesMappingLimits, CurvesMappingRequirements, MappedCurvesSource, map_retained_curves_source,
};

fn color_to_alpha_arguments<P>(effect: &Effect<P>) -> Result<Vec<FilterArgument>, IoError> {
    if effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 3 {
        return Err(unsupported(effect, "Color to Alpha version/count"));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(3).map_err(|_| unsupported(effect, "Color to Alpha allocation"))?;
    let mut color = [0.; 4];
    let mut thresholds = [0.; 2];
    for argument in &effect.arguments {
        if result.iter().any(|value: &FilterArgument| value.name == argument.name) {
            return Err(unsupported(effect, "Color to Alpha duplicate argument"));
        }
        let value = match (argument.name.as_str(), &argument.value) {
            ("color", ArgumentValue::Color { encoding, samples, icc }) if encoding == "R'G'B'A float" && samples.len() == 16 && icc.is_empty() => {
                let mut components = Vec::new();
                components.try_reserve_exact(4).map_err(|_| unsupported(effect, "Color to Alpha allocation"))?;
                let target = &mut color;
                for (target, raw) in target.iter_mut().zip(samples.as_chunks::<4>().0.iter()) {
                    let bits = u32::from_le_bytes(*raw);
                    *target = f32::from_bits(bits);
                    components.push(ExactNumber::Float32Bits(bits));
                }
                FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None }
            }
            (name @ ("transparency-threshold" | "opacity-threshold"), ArgumentValue::Float32Bits(bits)) => {
                let target = match name {
                    "transparency-threshold" => &mut thresholds[0],
                    _ => &mut thresholds[1],
                };
                *target = f32::from_bits(*bits);
                FilterValue::Number(ExactNumber::Float32Bits(*bits))
            }
            _ => return Err(unsupported(effect, "Color to Alpha literal encoding/profile or threshold type")),
        };
        result.push(FilterArgument { name: argument.name.clone(), value });
    }
    photocraft_algo::gimp::registry::color_to_alpha_member(effect.version.as_deref(), color, thresholds[0], thresholds[1])
        .map_err(|_| unsupported(effect, "Color to Alpha finite unit parameters"))?;
    Ok(result)
}

fn alpha_clip_arguments<P>(effect: &Effect<P>) -> Result<Vec<FilterArgument>, IoError> {
    if effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 4 {
        return Err(unsupported(effect, "Alpha Clip version/count"));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(4).map_err(|_| unsupported(effect, "Alpha Clip allocation"))?;
    let mut clip_low = None;
    let mut clip_high = None;
    let mut low = None;
    let mut high = None;
    for argument in &effect.arguments {
        if result.iter().any(|a: &FilterArgument| a.name == argument.name) {
            return Err(unsupported(effect, "Alpha Clip duplicate argument"));
        }
        let value = match (argument.name.as_str(), &argument.value) {
            ("clip-low", ArgumentValue::Boolean(v)) => {
                clip_low = Some(*v);
                FilterValue::Bool(*v)
            }
            ("clip-high", ArgumentValue::Boolean(v)) => {
                clip_high = Some(*v);
                FilterValue::Bool(*v)
            }
            ("low-limit", ArgumentValue::Float32Bits(v)) => {
                low = Some(f32::from_bits(*v));
                FilterValue::Number(ExactNumber::Float32Bits(*v))
            }
            ("high-limit", ArgumentValue::Float32Bits(v)) => {
                high = Some(f32::from_bits(*v));
                FilterValue::Number(ExactNumber::Float32Bits(*v))
            }
            _ => return Err(unsupported(effect, "Alpha Clip exact Bool/Float32 types")),
        };
        result.push(FilterArgument { name: argument.name.clone(), value });
    }
    photocraft_algo::gimp::registry::alpha_clip_member(
        effect.version.as_deref(),
        clip_low.ok_or_else(|| unsupported(effect, "missing clip-low"))?,
        low.ok_or_else(|| unsupported(effect, "missing low-limit"))?,
        clip_high.ok_or_else(|| unsupported(effect, "missing clip-high"))?,
        high.ok_or_else(|| unsupported(effect, "missing high-limit"))?,
        photocraft_algo::gimp::registry::WorkingEncoding::Linear,
    )
    .map_err(|_| unsupported(effect, "Alpha Clip ordered unit limits"))?;
    Ok(result)
}

fn alpha_clip_finite_arguments<P>(effect: &Effect<P>) -> Result<Vec<FilterArgument>, IoError> {
    if effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 4 {
        return Err(unsupported(effect, "Alpha Clip version/count"));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(4).map_err(|_| unsupported(effect, "Alpha Clip allocation"))?;
    let mut clip_low = None;
    let mut clip_high = None;
    let mut low = None;
    let mut high = None;
    for argument in &effect.arguments {
        if result.iter().any(|a: &FilterArgument| a.name == argument.name) {
            return Err(unsupported(effect, "Alpha Clip duplicate argument"));
        }
        let value = match (argument.name.as_str(), &argument.value) {
            ("clip-low", ArgumentValue::Boolean(v)) => {
                clip_low = Some(*v);
                FilterValue::Bool(*v)
            }
            ("clip-high", ArgumentValue::Boolean(v)) => {
                clip_high = Some(*v);
                FilterValue::Bool(*v)
            }
            ("low-limit", ArgumentValue::Float32Bits(v)) => {
                low = Some(f32::from_bits(*v));
                FilterValue::Number(ExactNumber::Float32Bits(*v))
            }
            ("high-limit", ArgumentValue::Float32Bits(v)) => {
                high = Some(f32::from_bits(*v));
                FilterValue::Number(ExactNumber::Float32Bits(*v))
            }
            _ => return Err(unsupported(effect, "Alpha Clip exact Bool/Float32 types")),
        };
        result.push(FilterArgument { name: argument.name.clone(), value });
    }
    photocraft_algo::gimp::registry::alpha_clip_finite_member(
        effect.version.as_deref(),
        clip_low.ok_or_else(|| unsupported(effect, "missing clip-low"))?,
        low.ok_or_else(|| unsupported(effect, "missing low-limit"))?,
        clip_high.ok_or_else(|| unsupported(effect, "missing clip-high"))?,
        high.ok_or_else(|| unsupported(effect, "missing high-limit"))?,
        photocraft_algo::gimp::registry::WorkingEncoding::Linear,
    )
    .map_err(|_| unsupported(effect, "Alpha Clip finite limits"))?;
    Ok(result)
}

fn component_arguments<P>(effect: &Effect<P>, domain: NeighborhoodDomain) -> Result<Vec<FilterArgument>, IoError> {
    if effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 3 {
        return Err(unsupported(effect, "Component Extract version/count"));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(3).map_err(|_| unsupported(effect, "Component Extract allocation"))?;
    let mut component = None;
    let mut invert = None;
    let mut linear = None;
    for argument in &effect.arguments {
        if result.iter().any(|value: &FilterArgument| value.name == argument.name) {
            return Err(unsupported(effect, "Component Extract duplicate argument"));
        }
        let value = match (argument.name.as_str(), &argument.value) {
            ("component", ArgumentValue::Enum(value)) => {
                component = Some(u8::try_from(*value).map_err(|_| unsupported(effect, "Component Extract choice"))?);
                FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: *value }
            }
            ("invert", ArgumentValue::Boolean(value)) => {
                invert = Some(*value);
                FilterValue::Bool(*value)
            }
            ("linear", ArgumentValue::Boolean(value)) => {
                linear = Some(*value);
                FilterValue::Bool(*value)
            }
            _ => return Err(unsupported(effect, "Component Extract exact choice/Bool types")),
        };
        result.push(FilterArgument { name: argument.name.clone(), value });
    }
    let component = component.ok_or_else(|| unsupported(effect, "Component Extract missing choice"))?;
    let valid = match domain {
        NeighborhoodDomain::ComponentExtractPrecision => matches!(component, 0 | 1 | 2 | 20),
        NeighborhoodDomain::ComponentExtractCmykPrecision => (8..=11).contains(&component),
        NeighborhoodDomain::ComponentExtractYcbcrPrecision => (12..=14).contains(&component),
        NeighborhoodDomain::ComponentExtractGeometryPrecision => matches!(component, 0 | 1 | 2 | 8..=14 | 20),
        _ => false,
    };
    if !valid {
        return Err(unsupported(effect, "Component Extract choice/domain mismatch"));
    }
    photocraft_algo::gimp::registry::component_extract_member(
        effect.version.as_deref(),
        component,
        invert.ok_or_else(|| unsupported(effect, "Component Extract missing invert"))?,
        linear.ok_or_else(|| unsupported(effect, "Component Extract missing linear"))?,
    )
    .map_err(|_| unsupported(effect, "Component Extract unqualified choice"))?;
    Ok(result)
}

fn levels_arguments<P>(effect: &Effect<P>) -> Result<Vec<FilterArgument>, IoError> {
    let [trc, source] = effect.arguments.as_slice() else {
        return Err(unsupported(effect, "structured Levels argument count"));
    };
    let ArgumentValue::Enum(value @ (0 | 1)) = trc.value else {
        return Err(unsupported(effect, "structured Levels transfer"));
    };
    let Some(photocraft_xcf::TypedConfiguration::Levels(config)) = source.configuration.as_ref().and_then(|c| c.typed.as_ref()) else {
        return Err(unsupported(effect, "missing typed Levels configuration"));
    };
    if trc.name != "trc"
        || source.name != "config"
        || !matches!(source.value, ArgumentValue::Config(_))
        || config.time_bits != Some(0)
        || config.trc.as_deref() != Some(if value == 0 { "linear" } else { "non-linear" })
        || config.channels.len() != 5
    {
        return Err(unsupported(effect, "unqualified structured Levels schema"));
    }
    let mut arguments = Vec::new();
    arguments.try_reserve_exact(4).map_err(|_| unsupported(effect, "Levels allocation failed"))?;
    arguments.push(FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:levels::trc".into(), value } });
    for (name, flag) in [("clamp-input", config.clamp_input), ("clamp-output", config.clamp_output)] {
        arguments.push(FilterArgument { name: name.into(), value: FilterValue::Bool(flag.ok_or_else(|| unsupported(effect, "missing Levels clamp"))?) });
    }
    let mut channels = Vec::new();
    channels.try_reserve_exact(5).map_err(|_| unsupported(effect, "Levels allocation failed"))?;
    for (channel, name) in config.channels.iter().zip(["value", "red", "green", "blue", "alpha"]) {
        if channel.channel != name {
            return Err(unsupported(effect, "Levels selector order"));
        }
        let mut fields = Vec::new();
        fields.try_reserve_exact(5).map_err(|_| unsupported(effect, "Levels allocation failed"))?;
        for (field, bits) in [
            ("low-input", channel.low_input_bits),
            ("high-input", channel.high_input_bits),
            ("gamma", channel.gamma_bits),
            ("low-output", channel.low_output_bits),
            ("high-output", channel.high_output_bits),
        ] {
            fields.push(FilterArgument { name: field.into(), value: FilterValue::Number(ExactNumber::Float64Bits(bits)) });
        }
        channels.push(FilterArgument { name: name.into(), value: FilterValue::Config(fields) });
    }
    arguments.push(FilterArgument { name: "config".into(), value: FilterValue::Config(channels) });
    Ok(arguments)
}

fn color_balance_arguments<P>(effect: &Effect<P>) -> Result<Vec<FilterArgument>, IoError> {
    let [source] = effect.arguments.as_slice() else {
        return Err(unsupported(effect, "Color Balance argument count"));
    };
    let Some(photocraft_xcf::TypedConfiguration::ColorBalance(config)) = source.configuration.as_ref().and_then(|value| value.typed.as_ref()) else {
        return Err(unsupported(effect, "missing typed Color Balance config"));
    };
    if source.name != "config" || !matches!(source.value, ArgumentValue::Config(_)) || config.time_bits != Some(0) || config.ranges.len() != 3 {
        return Err(unsupported(effect, "Color Balance source schema"));
    }
    let preserve = config.preserve_luminosity.ok_or_else(|| unsupported(effect, "Color Balance luminosity missing"))?;
    let mut ranges = Vec::new();
    ranges.try_reserve_exact(3).map_err(|_| unsupported(effect, "Color Balance allocation"))?;
    let mut numbers = [[0.0; 3]; 3];
    for ((source, name), decoded) in config.ranges.iter().zip(["shadows", "midtones", "highlights"]).zip(numbers.iter_mut()) {
        if source.range != name {
            return Err(unsupported(effect, "Color Balance range order"));
        }
        let mut fields = Vec::new();
        fields.try_reserve_exact(3).map_err(|_| unsupported(effect, "Color Balance allocation"))?;
        for ((bits, name), number) in [source.cyan_red_bits, source.magenta_green_bits, source.yellow_blue_bits]
            .into_iter()
            .zip(["cyan-red", "magenta-green", "yellow-blue"])
            .zip(decoded.iter_mut())
        {
            *number = f64::from_bits(bits);
            fields.push(FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(bits)) });
        }
        ranges.push(FilterArgument { name: name.into(), value: FilterValue::Config(fields) });
    }
    photocraft_algo::gimp::color_balance::ColorBalance { ranges: numbers, preserve_luminosity: preserve }
        .validate()
        .map_err(|_| unsupported(effect, "Color Balance parameter domain"))?;
    let mut arguments = Vec::new();
    arguments.try_reserve_exact(2).map_err(|_| unsupported(effect, "Color Balance allocation"))?;
    arguments.push(FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(preserve) });
    arguments.push(FilterArgument { name: "config".into(), value: FilterValue::Config(ranges) });
    Ok(arguments)
}

/// This epoch has positive source/edit witnesses for the complete Replace envelope.
/// The opaque signed policy fields stay unresolved; they are never turned into known enums.
pub(crate) fn qualified_stack<P>(
    effects: &[Effect<P>],
    masks: &[Option<ItemRef>],
    owner: ItemRef,
    first_id: u64,
    version: u32,
    domain: NeighborhoodDomain,
) -> Result<FilterStack, IoError> {
    let generic_replace = matches!(domain, NeighborhoodDomain::PointReplace(_));
    let domain = if let NeighborhoodDomain::PointReplace(family) = domain { family.domain() } else { domain };
    if generic_replace && effects.len() != 1 {
        return Err(IoError::Unsupported("generic point Replace requires one effect".into()));
    }
    if effects.len() != masks.len() {
        return Err(IoError::Unsupported("XCF filter mask mapping mismatch".into()));
    }
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(effects.len()).map_err(|_| IoError::Unsupported("XCF filter allocation failed".into()))?;
    for (ordinal, effect) in effects.iter().enumerate() {
        let source = &effect.state;
        let point_stack = domain == NeighborhoodDomain::PointStackEdits;
        let invert_linear = domain == NeighborhoodDomain::InvertLinearPrecision;
        let invert_gamma = domain == NeighborhoodDomain::InvertGammaPrecision;
        let color_exchange = domain == NeighborhoodDomain::ColorExchangePrecision;
        let color_overlay = domain == NeighborhoodDomain::ColorOverlayPrecision;
        if effect.operation == "gegl:color-overlay"
            && (!color_overlay
                || !generic_replace
                || effects.len() != 1
                || (source.blend_space, source.composite_space, source.composite_mode) != (Some(1435337297), Some(1435337297), Some(1435337297)))
        {
            return Err(unsupported(effect, "unqualified Color Overlay source envelope"));
        }
        let color_to_alpha = domain == NeighborhoodDomain::ColorToAlphaPrecision;
        let component_extract = matches!(
            domain,
            NeighborhoodDomain::ComponentExtractPrecision
                | NeighborhoodDomain::ComponentExtractCmykPrecision
                | NeighborhoodDomain::ComponentExtractYcbcrPrecision
                | NeighborhoodDomain::ComponentExtractGeometryPrecision
        );
        let component_epoch = match domain {
            NeighborhoodDomain::ComponentExtractGeometryPrecision => "xcf-22-stock-component-extract-geometry-v1",
            NeighborhoodDomain::ComponentExtractCmykPrecision => "xcf-22-stock-component-extract-cmyk-precision-v1",
            NeighborhoodDomain::ComponentExtractYcbcrPrecision => "xcf-22-stock-component-extract-ycbcr-precision-v1",
            _ => "xcf-22-stock-component-extract-four-precision-v1",
        };
        let alpha_clip_finite = domain == NeighborhoodDomain::AlphaClipFinitePrecision;
        let alpha_clip = domain == NeighborhoodDomain::AlphaClipPrecision || alpha_clip_finite;
        let alpha_clip_tuple = if alpha_clip_finite { -305954223 } else { -1710980527 };
        if alpha_clip
            && (source.blend_space, source.composite_space, source.composite_mode) != (Some(alpha_clip_tuple), Some(alpha_clip_tuple), Some(alpha_clip_tuple))
        {
            return Err(unsupported(effect, "Alpha Clip stored field envelope"));
        }
        if effect.operation == "gegl:alpha-clip" && (!alpha_clip || !generic_replace || effects.len() != 1) {
            return Err(unsupported(effect, "unqualified Alpha Clip source"));
        }
        if effect.operation == "gegl:component-extract" && (!component_extract || !generic_replace || effects.len() != 1) {
            return Err(unsupported(effect, "unqualified Component Extract source"));
        }
        if matches!(domain, NeighborhoodDomain::ComponentExtractCmykPrecision | NeighborhoodDomain::ComponentExtractYcbcrPrecision)
            && (source.blend_space, source.composite_space, source.composite_mode) != (Some(-305954223), Some(-305954223), Some(-305954223))
        {
            return Err(unsupported(effect, "CMYK/YCbCr Component Extract stored field envelope"));
        }
        if domain == NeighborhoodDomain::ComponentExtractGeometryPrecision
            && (source.blend_space, source.composite_space, source.composite_mode) != (Some(87327313), Some(87327313), Some(87327313))
        {
            return Err(unsupported(effect, "Component Extract geometry stored field envelope"));
        }
        if effect.operation == "gegl:color-to-alpha" && (!color_to_alpha || !generic_replace || effects.len() != 1) {
            return Err(unsupported(effect, "unqualified Color to Alpha source"));
        }
        if effect.operation == "gegl:color-exchange" && (!color_exchange || !generic_replace || effects.len() != 1) {
            return Err(unsupported(effect, "unqualified Color Exchange source"));
        }
        if effect.operation == "gegl:invert-gamma"
            && (!invert_gamma || !generic_replace || effects.len() != 1 || effect.version.as_deref() != Some("0:0") || !effect.arguments.is_empty())
        {
            return Err(unsupported(effect, "unqualified zero-argument Invert Gamma source"));
        }
        if effect.operation == "gegl:invert-linear"
            && (!matches!(domain, NeighborhoodDomain::InvertLinearPrecision | NeighborhoodDomain::PointStackEdits)
                || effect.version.as_deref() != Some("0:0")
                || !effect.arguments.is_empty()
                || invert_linear && (!generic_replace || effects.len() != 1))
        {
            return Err(unsupported(effect, "unqualified zero-argument Invert Linear source"));
        }
        let posterize = effect.operation == "gimp:posterize";
        let mono = effect.operation == "gegl:mono-mixer";
        let channel_mixer = domain == NeighborhoodDomain::ChannelMixerPrecision;
        if effect.operation == "gegl:channel-mixer"
            && (!channel_mixer || !generic_replace || effects.len() != 1 || !crate::xcf_point_replace_source::channel_mixer_source::observed_envelope(effect))
        {
            return Err(unsupported(effect, "Channel Mixer exact source envelope"));
        }
        let mono_signed = domain == NeighborhoodDomain::MonoMixerSignedPrecision;
        let color_rotate = domain == NeighborhoodDomain::ColorRotatePerceptual;
        let video = domain == NeighborhoodDomain::VideoDegradationFloat;
        let video_epoch = if video {
            Some(crate::xcf_point_replace_source::video_degradation_source::observed_epoch(effect)
                .ok_or_else(|| unsupported(effect, "Video Degradation source epoch"))?)
        } else {
            None
        };
        if effect.operation == "gegl:video-degradation"
            && (!video || !generic_replace || effects.len() != 1 || video_epoch.is_none())
        {
            return Err(unsupported(effect, "Video Degradation exact source envelope"));
        }
        if effect.operation == "gegl:color-rotate"
            && (!color_rotate || !generic_replace || effects.len() != 1 || !crate::xcf_point_replace_source::color_rotate_source::observed_envelope(effect))
        {
            return Err(unsupported(effect, "Color Rotate exact source envelope"));
        }
        let unpremultiply = domain == NeighborhoodDomain::UnpremultiplyRgbFloatLinear;
        if unpremultiply != (effect.operation == "gegl:unpremultiply")
            || unpremultiply && (effects.len() != 1 || !crate::xcf_import::unpremultiply::observed_envelope(effect))
        {
            return Err(unsupported(effect, "Unpremultiply RGB Float Linear source envelope"));
        }
        let absolute = domain == NeighborhoodDomain::AbsoluteLinearPrecision;
        if effect.operation == "gegl:absolute"
            && (!absolute || !generic_replace || effects.len() != 1 || !crate::xcf_point_replace_source::absolute_source::observed_envelope(effect))
        {
            return Err(unsupported(effect, "Absolute Linear source envelope"));
        }
        let red_eye = domain == NeighborhoodDomain::RedEyePrecision;
        let red_eye_epoch = if red_eye {
            Some(crate::xcf_point_replace_source::red_eye_source::observed_epoch(effect).ok_or_else(|| unsupported(effect, "Red Eye source epoch"))?)
        } else { None };
        if effect.operation == "gegl:red-eye-removal"
            && (!red_eye || !generic_replace || effects.len() != 1 || red_eye_epoch.is_none())
        {
            return Err(unsupported(effect, "Red Eye source envelope"));
        }
        if mono_signed && (!generic_replace || effects.len() != 1 || !crate::xcf_point_replace_source::mono_mixer_source::observed_envelope(effect)) {
            return Err(unsupported(effect, "expanded Mono Mixer source envelope"));
        }
        let mono_edits = domain == NeighborhoodDomain::MonoMixerEditsOriginal;
        let clip = effect.operation == "gegl:rgb-clip";
        let value_precision = domain == NeighborhoodDomain::ValueInvertPrecision;
        let brightness_precision = domain == NeighborhoodDomain::BrightnessContrastPrecision;
        let gegl_levels_precision = domain == NeighborhoodDomain::GeglLevelsPrecision;
        let exposure_geometry = domain == NeighborhoodDomain::ExposureGeometry;
        let exposure_black_precision = exposure_geometry || domain == NeighborhoodDomain::ExposureBlackPrecision;
        let exposure_epoch = if exposure_geometry { "xcf-22-stock-exposure-offset-geometry-v1" } else { "xcf-22-stock-exposure-black-level-precision-v1" };
        let color_balance_depth = domain == NeighborhoodDomain::ColorBalanceDepth;
        let color_balance = color_balance_depth || domain == NeighborhoodDomain::ColorBalancePrecision;
        let color_balance_epoch = if color_balance_depth { "xcf-22-stock-color-balance-depth-v1" } else { "xcf-22-stock-color-balance-precision-v1" };
        let temperature = domain == NeighborhoodDomain::ColorTemperaturePrecision;
        let threshold_alpha = domain == NeighborhoodDomain::ThresholdAlphaPrecision;
        if effect.operation == "gimp:threshold-alpha"
            && (!threshold_alpha || effects.len() != 1 || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 1)
        {
            return Err(unsupported(effect, "unqualified Alpha Threshold owner/version/argument count"));
        }
        let threshold = domain == NeighborhoodDomain::ThresholdPrecision;
        if effect.operation == "gimp:threshold" && (!threshold || effects.len() != 1 || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 3)
        {
            return Err(unsupported(effect, "unqualified Threshold owner/version/argument count"));
        }
        let alien_hsl = domain == NeighborhoodDomain::AlienMapHslPrecision;
        let alien_map = alien_hsl || domain == NeighborhoodDomain::AlienMapPrecision;
        let alien_epoch = if alien_hsl { "xcf-22-stock-alien-map-hsl-precision-v1" } else { "xcf-22-stock-alien-map-precision-v1" };
        if effect.operation == "gegl:alien-map"
            && (!alien_map || effects.len() != 1 || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 10)
        {
            return Err(unsupported(effect, "unqualified Alien Map owner/version/argument count"));
        }
        let sepia = domain == NeighborhoodDomain::SepiaPrecision;
        if effect.operation == "gegl:sepia" && (!sepia || effects.len() != 1 || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 2) {
            return Err(unsupported(effect, "unqualified Sepia owner/version/argument count"));
        }
        let saturation_gray = domain == NeighborhoodDomain::SaturationGrayPrecision;
        let saturation_yuv = domain == NeighborhoodDomain::SaturationRgbYuvPrecision;
        let saturation = saturation_gray || saturation_yuv || domain == NeighborhoodDomain::SaturationPrecision;
        let saturation_epoch = if saturation_gray {
            "xcf-22-stock-gray-saturation-precision-v1"
        } else if saturation_yuv {
            "xcf-22-stock-rgb-yuv-saturation-precision-v1"
        } else {
            "xcf-22-stock-saturation-precision-v1"
        };
        if effect.operation == "gegl:saturation"
            && (!saturation || effects.len() != 1 || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 2)
        {
            return Err(unsupported(effect, "unqualified Saturation owner/version/argument count"));
        }
        let desaturate = domain == NeighborhoodDomain::DesaturatePrecision;
        if effect.operation == "gimp:desaturate"
            && (!desaturate || effects.len() != 1 || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 1)
        {
            return Err(unsupported(effect, "unqualified Desaturate owner/version/argument count"));
        }
        let temperature_offgrid = temperature
            && source.composite_mode == Some(-1716420015)
            && source.composite_space == Some(-1716420015)
            && source.blend_space == Some(-1716420015)
            && effect.arguments.len() == 2
            && effect.arguments.iter().all(|argument| match (&*argument.name, &argument.value) {
                ("original-temperature", ArgumentValue::Float32Bits(bits)) => *bits == 7031.125_f32.to_bits(),
                ("intended-temperature", ArgumentValue::Float32Bits(bits)) => *bits == 5299.375_f32.to_bits(),
                _ => false,
            });
        let temperature_epoch = if temperature_offgrid { "xcf-22-stock-color-temperature-offgrid-v1" } else { "xcf-22-stock-color-temperature-precision-v1" };
        if effect.operation == "gegl:color-temperature" && (!temperature || effects.len() != 1 || effect.version.as_deref() != Some("0:0")) {
            return Err(unsupported(effect, "unqualified Color Temperature owner/version"));
        }
        if effect.operation == "gimp:color-balance" && (!color_balance || effects.len() != 1 || effect.version.as_deref() != Some("0:0")) {
            return Err(unsupported(effect, "unqualified Color Balance owner/version"));
        }
        if exposure_black_precision && (effect.operation != "gegl:exposure" || effects.len() != 1 || effect.version.as_deref() != Some("1:0")) {
            return Err(unsupported(effect, "unqualified Exposure black-level owner or version"));
        }
        if effect.operation == "gegl:levels" && (!gegl_levels_precision || effects.len() != 1 || effect.version.as_deref() != Some("0:0")) {
            return Err(unsupported(effect, "unqualified GEGL Levels owner or version"));
        }
        if effect.operation == "gegl:brightness-contrast" && (!brightness_precision || effects.len() != 1 || effect.version.as_deref() != Some("0:0")) {
            return Err(unsupported(effect, "unqualified Brightness/Contrast owner or version"));
        }
        if clip && (domain != NeighborhoodDomain::RgbClipHdr || effects.len() != 1 || effect.version.as_deref() != Some("0:0")) {
            return Err(unsupported(effect, "unqualified RGB Clip owner or version"));
        }
        if mono
            && (!matches!(
                domain,
                NeighborhoodDomain::MonoMixerOriginal | NeighborhoodDomain::MonoMixerEditsOriginal | NeighborhoodDomain::MonoMixerSignedPrecision
            ) || effects.len() != 1
                || effect.version.as_deref() != Some("0:0"))
        {
            return Err(unsupported(effect, "unqualified Mono Mixer owner or version"));
        }
        if effect.operation == "gegl:value-invert"
            && (!matches!(domain, NeighborhoodDomain::ValueInvertOriginal | NeighborhoodDomain::ValueInvertPrecision)
                || effects.len() != 1
                || effect.version.as_deref() != Some("0:0")
                || !effect.arguments.is_empty())
        {
            return Err(unsupported(effect, "unqualified Value Invert owner, version or arguments"));
        }
        if posterize && (domain != NeighborhoodDomain::PosterizeOriginal || effects.len() != 1 || effect.version.as_deref() != Some("0:0")) {
            return Err(unsupported(effect, "unqualified Posterize owner or version"));
        }
        if point_stack
            && (effects.len() > 2
                || !matches!(effect.operation.as_str(), "gegl:exposure" | "gegl:invert-linear")
                || effect.operation == "gegl:exposure"
                    && (effect.version.as_deref() != Some("1:0")
                        || effect.arguments.len() != 2
                        || !effect.arguments.iter().all(|a| match (&*a.name, &a.value) {
                            ("exposure", ArgumentValue::Float32Bits(bits)) => *bits == 0.5_f32.to_bits(),
                            ("black-level", ArgumentValue::Float32Bits(bits)) => *bits == 0.0_f32.to_bits(),
                            _ => false,
                        }))
                || effect.operation == "gegl:invert-linear" && (effect.version.as_deref() != Some("0:0") || !effect.arguments.is_empty()))
        {
            return Err(unsupported(effect, "unqualified point stack identity or arguments"));
        }
        let gaussian = effect.operation == "gegl:gaussian-blur";
        let group_gaussian = gaussian && domain == NeighborhoodDomain::IsolatedSmallAuto;
        let finite_gaussian = gaussian && domain == NeighborhoodDomain::RasterFinite;
        let box_blur = effect.operation == "gegl:box-blur";
        let finite_box = box_blur && domain == NeighborhoodDomain::BoxFinite;
        let median = effect.operation == "gegl:median-blur";
        let finite_median = median && matches!(domain, NeighborhoodDomain::MedianFinite | NeighborhoodDomain::MedianGray);
        let motion = effect.operation == "gegl:motion-blur-linear";
        let pixelize = effect.operation == "gegl:pixelize";
        let curves = effect.operation == "gimp:curves";
        let levels = effect.operation == "gimp:levels";
        let sentinel = Some(if alien_hsl {
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
        } else if color_balance_depth {
            -1215462831
        } else if color_balance || temperature {
            -113999279
        } else if exposure_geometry {
            -901479855
        } else if exposure_black_precision {
            706183761
        } else if gegl_levels_precision {
            392856145
        } else if brightness_precision {
            214598225
        } else if value_precision {
            -1395293615
        } else if clip {
            1825931857
        } else if unpremultiply {
            -1877441967
        } else if absolute {
            1574142545
        } else if channel_mixer {
            -1150320047
        } else if mono_signed {
            1799848529
        } else if mono_edits {
            107971153
        } else if point_stack {
            -1178762671
        } else if group_gaussian {
            if source.enabled == Some(false) && source.composite_mode == Some(-1474592175) { -1474592175 } else { -643996976 }
        } else if finite_gaussian || finite_box || finite_median {
            -1474592175
        } else if gaussian {
            -856260015
        } else if curves || levels {
            -1080065455
        } else {
            -261651887
        });
        if version != 22
            || effects.len() > 256
            || source.mode != Some(62)
            || source.opacity_u8.is_some()
            || if generic_replace {
                source.composite_mode.is_none() || source.composite_space.is_none() || source.blend_space.is_none()
            } else {
                source.composite_mode != sentinel || source.composite_space != sentinel || source.blend_space != sentinel
            }
            || !(source.opacity_bits == Some(1.0_f32.to_bits())
                || point_stack && effect.operation == "gegl:invert-linear" && source.opacity_bits == Some(0.5_f32.to_bits()))
            || source.clipping != Some(0)
            || if group_gaussian { !matches!(source.region, Some(0 | 1)) } else { source.region != Some(0) }
            || effect.properties.iter().any(|p| !matches!(p.tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 45 | 46))
        {
            return Err(unsupported(effect, "unqualified stored filter envelope"));
        }
        let enabled = source.enabled.ok_or_else(|| unsupported(effect, "missing enabled state"))?;
        if (finite_gaussian || box_blur || median || motion || pixelize) && !enabled {
            return Err(unsupported(effect, "unqualified disabled neighborhood envelope"));
        }
        if gaussian
            && (!matches!(domain, NeighborhoodDomain::RasterOriginal | NeighborhoodDomain::RasterFinite | NeighborhoodDomain::IsolatedSmallAuto)
                || effect.version.as_deref() != Some("0:0"))
        {
            return Err(unsupported(effect, "unqualified Gaussian source domain or version"));
        }
        if box_blur
            && (!matches!(domain, NeighborhoodDomain::BoxOriginal | NeighborhoodDomain::BoxFinite)
                || effect.version.as_deref() != Some("0:0")
                || effects.len() != 1)
        {
            return Err(unsupported(effect, "unqualified Box Blur source domain or version"));
        }
        if median
            && (!matches!(domain, NeighborhoodDomain::MedianOriginal | NeighborhoodDomain::MedianFinite | NeighborhoodDomain::MedianGray)
                || effect.version.as_deref() != Some("0:0")
                || effects.len() != 1)
        {
            return Err(unsupported(effect, "unqualified Median Blur source domain or version"));
        }
        if motion && (domain != NeighborhoodDomain::MotionOriginal || effect.version.as_deref() != Some("0:0") || effects.len() != 1) {
            return Err(unsupported(effect, "unqualified Motion Blur source domain or version"));
        }
        if pixelize && (domain != NeighborhoodDomain::PixelizeOriginal || effect.version.as_deref() != Some("0:0") || effects.len() != 1) {
            return Err(unsupported(effect, "unqualified Pixelize source domain or version"));
        }
        if (curves || levels) && effect.version.as_deref() != Some("0:0") {
            return Err(unsupported(effect, "unqualified Curves version"));
        }
        let mut arguments = Vec::new();
        let mut numeric = Vec::new();
        arguments.try_reserve_exact(effect.arguments.len()).map_err(|_| unsupported(effect, "argument allocation failed"))?;
        numeric.try_reserve_exact(effect.arguments.len()).map_err(|_| unsupported(effect, "argument allocation failed"))?;
        if alpha_clip {
            arguments = if alpha_clip_finite { alpha_clip_finite_arguments(effect)? } else { alpha_clip_arguments(effect)? };
        }
        if component_extract {
            arguments = component_arguments(effect, domain)?;
        }
        if color_to_alpha {
            arguments = color_to_alpha_arguments(effect)?;
        }
        if color_exchange {
            arguments = color_exchange_arguments(effect)?;
        } else if color_overlay {
            arguments = color_overlay_arguments(effect)?;
        } else if levels {
            arguments = levels_arguments(effect)?;
        } else if color_balance {
            arguments = color_balance_arguments(effect)?;
        }
        for argument in effect
            .arguments
            .iter()
            .filter(|_| !levels && !color_balance && !color_exchange && !color_overlay && !color_to_alpha && !component_extract && !alpha_clip)
        {
            if alien_map {
                let (value, scalar) = match (&*argument.name, &argument.value) {
                    ("color-model", ArgumentValue::Enum(value)) if *value == if alien_hsl { 1 } else { 0 } => {
                        (FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: *value }, f64::from(*value))
                    }
                    ("cpn-1-keep" | "cpn-2-keep" | "cpn-3-keep", ArgumentValue::Boolean(value)) => (FilterValue::Bool(*value), if *value { 1.0 } else { 0.0 }),
                    (
                        name @ ("cpn-1-frequency" | "cpn-2-frequency" | "cpn-3-frequency" | "cpn-1-phaseshift" | "cpn-2-phaseshift" | "cpn-3-phaseshift"),
                        ArgumentValue::Float32Bits(bits),
                    ) => {
                        let value = f32::from_bits(*bits);
                        let maximum = if name.ends_with("-frequency") { 20.0 } else { 360.0 };
                        if !value.is_finite() || !(0.0..=maximum).contains(&value) {
                            return Err(unsupported(effect, "Alien Map frequency/phase domain"));
                        }
                        (FilterValue::Number(ExactNumber::Float32Bits(*bits)), f64::from(value))
                    }
                    _ => return Err(unsupported(effect, "Alien Map requires stored RGB choice, Bool keeps and Float32 frequencies/phases")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Alien Map argument"));
                }
                numeric.push((argument.name.as_str(), scalar));
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if sepia {
                let (value, scalar) = match (&*argument.name, &argument.value) {
                    ("srgb", ArgumentValue::Boolean(value)) => (FilterValue::Bool(*value), if *value { 1.0 } else { 0.0 }),
                    ("scale", ArgumentValue::Float32Bits(bits)) => {
                        let value = f32::from_bits(*bits);
                        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                            return Err(unsupported(effect, "Sepia scale domain"));
                        }
                        (FilterValue::Number(ExactNumber::Float32Bits(*bits)), f64::from(value))
                    }
                    _ => return Err(unsupported(effect, "Sepia requires stored Bool srgb and Float32 scale")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Sepia argument"));
                }
                numeric.push((argument.name.as_str(), scalar));
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if threshold_alpha && argument.name != "value" {
                return Err(unsupported(effect, "Alpha Threshold argument identity"));
            }
            if threshold && argument.name == "channel" {
                let ArgumentValue::Enum(value @ (0..=5)) = argument.value else {
                    return Err(unsupported(effect, "Threshold requires stored histogram-channel enum"));
                };
                numeric.push((argument.name.as_str(), f64::from(value)));
                arguments.push(FilterArgument { name: argument.name.clone(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value } });
                continue;
            }
            if threshold && !matches!(argument.name.as_str(), "low" | "high") {
                return Err(unsupported(effect, "Threshold argument identity"));
            }
            if saturation && argument.name == "colorspace" {
                let ArgumentValue::Enum(value @ (0..=2)) = argument.value else {
                    return Err(unsupported(effect, "Saturation requires its stored colorspace enum"));
                };
                if arguments.iter().any(|argument: &FilterArgument| argument.name == "colorspace") {
                    return Err(unsupported(effect, "duplicate Saturation colorspace"));
                }
                if (value == 2 && !saturation_gray && !saturation_yuv) || (saturation_yuv && value != 2) {
                    return Err(unsupported(effect, "unqualified RGB Yuv saturation"));
                }
                numeric.push((argument.name.as_str(), f64::from(value)));
                arguments.push(FilterArgument { name: argument.name.clone(), value: FilterValue::Enum { domain: "GeglSaturationType".into(), value } });
                continue;
            }
            if desaturate {
                let ("mode", ArgumentValue::Enum(value @ (0..=4))) = (&*argument.name, &argument.value) else {
                    return Err(unsupported(effect, "Desaturate requires stored GimpDesaturateMode enum"));
                };
                numeric.push((argument.name.as_str(), f64::from(*value)));
                arguments.push(FilterArgument { name: "mode".into(), value: FilterValue::Enum { domain: "GimpDesaturateMode".into(), value: *value } });
                continue;
            }
            if clip {
                let (value, scalar) = match (&*argument.name, &argument.value) {
                    ("clip-low", ArgumentValue::Boolean(value)) => (FilterValue::Bool(*value), if *value { 1.0 } else { 0.0 }),
                    ("clip-high", ArgumentValue::Boolean(true)) => (FilterValue::Bool(true), 1.0),
                    ("low-limit" | "high-limit", ArgumentValue::Float32Bits(bits)) => {
                        (FilterValue::Number(ExactNumber::Float32Bits(*bits)), f64::from(f32::from_bits(*bits)))
                    }
                    _ => return Err(unsupported(effect, "RGB Clip requires stored Boolean flags and Float32 limits")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate RGB Clip argument"));
                }
                numeric.push((argument.name.as_str(), scalar));
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if video {
                let value = match (argument.name.as_str(), &argument.value) {
                    ("pattern", ArgumentValue::Enum(value @ 0..=8)) => FilterValue::Enum { domain: "gegl:video-degradation/pattern".into(), value: *value },
                    ("additive" | "rotated", ArgumentValue::Boolean(value)) => FilterValue::Bool(*value),
                    _ => return Err(unsupported(effect, "Video Degradation typed argument")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Video Degradation argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if color_rotate {
                let value = match (&*argument.name, &argument.value) {
                    ("src-clockwise" | "dest-clockwise", ArgumentValue::Boolean(v)) => FilterValue::Bool(*v),
                    ("src-from" | "src-to" | "dest-from" | "dest-to" | "threshold" | "hue" | "saturation", ArgumentValue::Float32Bits(bits)) => {
                        FilterValue::Number(ExactNumber::Float32Bits(*bits))
                    }
                    ("gray-mode", ArgumentValue::Enum(v @ (0 | 1))) => FilterValue::Enum { domain: "gegl:color-rotate/gray-mode".into(), value: *v },
                    _ => return Err(unsupported(effect, "Color Rotate typed arguments")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Color Rotate argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if channel_mixer {
                let value = match (&*argument.name, &argument.value) {
                    ("preserve-luminosity", ArgumentValue::Boolean(value)) => FilterValue::Bool(*value),
                    (
                        "rr-gain" | "rg-gain" | "rb-gain" | "gr-gain" | "gg-gain" | "gb-gain" | "br-gain" | "bg-gain" | "bb-gain",
                        ArgumentValue::Float32Bits(bits),
                    ) => FilterValue::Number(ExactNumber::Float32Bits(*bits)),
                    _ => return Err(unsupported(effect, "Channel Mixer typed arguments")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Channel Mixer argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if mono_signed {
                let value = match (&*argument.name, &argument.value) {
                    ("preserve-luminosity", ArgumentValue::Boolean(value)) => FilterValue::Bool(*value),
                    ("red" | "green" | "blue", ArgumentValue::Float32Bits(bits)) => FilterValue::Number(ExactNumber::Float32Bits(*bits)),
                    _ => return Err(unsupported(effect, "expanded Mono Mixer typed arguments")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate expanded Mono Mixer argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if mono {
                let (value, scalar) = match (&*argument.name, &argument.value) {
                    ("preserve-luminosity", ArgumentValue::Boolean(false)) => (FilterValue::Bool(false), 0.0),
                    ("red" | "green" | "blue", ArgumentValue::Float32Bits(bits)) => {
                        (FilterValue::Number(ExactNumber::Float32Bits(*bits)), f64::from(f32::from_bits(*bits)))
                    }
                    _ => return Err(unsupported(effect, "Mono Mixer requires false Bool and stored Float32 RGB weights")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Mono Mixer argument"));
                }
                numeric.push((argument.name.as_str(), scalar));
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if posterize {
                let ("levels", ArgumentValue::Integer(value @ (2..=256))) = (&*argument.name, &argument.value) else {
                    return Err(unsupported(effect, "Posterize requires its stored Signed32 levels"));
                };
                if !arguments.is_empty() {
                    return Err(unsupported(effect, "duplicate Posterize levels"));
                }
                numeric.push((argument.name.as_str(), f64::from(*value)));
                arguments.push(FilterArgument {
                    name: argument.name.clone(),
                    value: FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(*value) }),
                });
                continue;
            }
            if pixelize {
                let value = match (&*argument.name, &argument.value) {
                    ("norm", ArgumentValue::Enum(2)) => FilterValue::Enum { domain: "gegl:pixelize::norm".into(), value: 2 },
                    ("size-x", ArgumentValue::Integer(value @ (16 | 17)))
                    | ("size-y", ArgumentValue::Integer(value @ 16))
                    | ("offset-x" | "offset-y", ArgumentValue::Integer(value @ 0)) => {
                        FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(*value) })
                    }
                    ("ratio-x" | "ratio-y", ArgumentValue::Float32Bits(bits)) if *bits == 1.0_f32.to_bits() => {
                        FilterValue::Number(ExactNumber::Float32Bits(*bits))
                    }
                    ("background", ArgumentValue::Color { encoding, samples, icc })
                        if encoding == "R'G'B'A float"
                            && icc.is_empty()
                            && samples.as_slice() == [0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63] =>
                    {
                        FilterValue::Color {
                            components: vec![ExactNumber::Float32Bits(1.0_f32.to_bits()); 4],
                            encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
                            profile: None,
                        }
                    }
                    _ => return Err(unsupported(effect, "unqualified Pixelize argument type or value")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Pixelize argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if motion {
                let bits = match (&*argument.name, &argument.value) {
                    ("length", ArgumentValue::Float32Bits(bits)) if matches!(f32::from_bits(*bits), 10.0 | 10.125) => *bits,
                    ("angle", ArgumentValue::Float32Bits(bits)) if *bits == 0.0_f32.to_bits() => *bits,
                    _ => return Err(unsupported(effect, "unqualified Motion Blur argument type or value")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Motion Blur argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value: FilterValue::Number(ExactNumber::Float32Bits(bits)) });
                continue;
            }
            if median {
                let value = match (&*argument.name, &argument.value) {
                    ("radius", ArgumentValue::Integer(value @ (3 | 4))) => FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(*value) }),
                    ("neighborhood", ArgumentValue::Enum(1)) => FilterValue::Enum { domain: "gegl:median-blur::neighborhood".into(), value: 1 },
                    ("abyss-policy", ArgumentValue::Enum(1)) => FilterValue::Enum { domain: "gegl:median-blur::abyss-policy".into(), value: 1 },
                    ("percentile" | "alpha-percentile", ArgumentValue::Float32Bits(bits)) if *bits == 50.0_f32.to_bits() => {
                        FilterValue::Number(ExactNumber::Float32Bits(*bits))
                    }
                    ("high-precision", ArgumentValue::Boolean(false)) => FilterValue::Bool(false),
                    ("high-precision", ArgumentValue::Boolean(true)) if finite_median => FilterValue::Bool(true),
                    _ => return Err(unsupported(effect, "unqualified Median Blur argument type or value")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Median Blur argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if box_blur {
                let value = match (&*argument.name, &argument.value) {
                    ("radius", ArgumentValue::Integer(value @ (4 | 5))) => FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(*value) }),
                    _ => return Err(unsupported(effect, "unqualified Box Blur argument type or value")),
                };
                if !arguments.is_empty() {
                    return Err(unsupported(effect, "duplicate Box Blur radius"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if curves {
                let value = match (&*argument.name, &argument.value) {
                    ("trc", ArgumentValue::Enum(1)) => FilterValue::Enum { domain: "gimp:curves::trc".into(), value: 1 },
                    ("config", ArgumentValue::Config(_)) => {
                        let Some(photocraft_xcf::TypedConfiguration::Curves(config)) = argument.configuration.as_ref().and_then(|c| c.typed.as_ref()) else {
                            return Err(unsupported(effect, "missing typed Curves configuration"));
                        };
                        if config.trc.as_deref() != Some("non-linear") || config.time_bits != Some(0) || config.channels.len() != 5 {
                            return Err(unsupported(effect, "unqualified Curves configuration envelope"));
                        }
                        let mut channels = Vec::new();
                        channels.try_reserve_exact(5).map_err(|_| unsupported(effect, "Curves allocation failed"))?;
                        for (channel, name) in config.channels.iter().zip(["value", "red", "green", "blue", "alpha"]) {
                            let curve = &channel.curve;
                            if channel.channel != name
                                || curve.curve_type != "free"
                                || !curve.points_bits.is_empty()
                                || !curve.point_types.is_empty()
                                || curve.n_samples != 256
                                || curve.samples_bits.len() != 256
                                || curve.samples_bits.iter().any(|bits| !f64::from_bits(*bits).is_finite())
                            {
                                return Err(unsupported(effect, "unqualified FREE256 curve"));
                            }
                            let mut samples = Vec::new();
                            samples.try_reserve_exact(256).map_err(|_| unsupported(effect, "Curves sample allocation failed"))?;
                            samples.extend_from_slice(&curve.samples_bits);
                            channels.push(FilterArgument {
                                name: name.into(),
                                value: FilterValue::Curve { free: true, points_f64_bits: Vec::new(), point_types: Vec::new(), samples_f64_bits: samples },
                            });
                        }
                        FilterValue::Config(channels)
                    }
                    _ => return Err(unsupported(effect, "unqualified Curves argument")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Curves argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            if gaussian {
                let value = match (&*argument.name, &argument.value) {
                    ("std-dev-x", ArgumentValue::Float32Bits(bits))
                        if if group_gaussian { *bits == 0.8_f32.to_bits() } else { matches!(f32::from_bits(*bits), 2.25 | 3.75) } =>
                    {
                        FilterValue::Number(ExactNumber::Float32Bits(*bits))
                    }
                    ("std-dev-y", ArgumentValue::Float32Bits(bits))
                        if if finite_gaussian {
                            matches!(f32::from_bits(*bits), 1.3 | 0.8)
                        } else {
                            *bits == if group_gaussian { 1.3_f32.to_bits() } else { 1.5_f32.to_bits() }
                        } =>
                    {
                        FilterValue::Number(ExactNumber::Float32Bits(*bits))
                    }
                    ("filter", ArgumentValue::Enum(0)) => FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 0 },
                    ("filter", ArgumentValue::Enum(1)) if finite_gaussian => FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 1 },
                    ("abyss-policy", ArgumentValue::Enum(1)) => FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 },
                    ("clip-extent", ArgumentValue::Boolean(true)) => FilterValue::Bool(true),
                    _ => return Err(unsupported(effect, "unqualified Gaussian argument type or value")),
                };
                if arguments.iter().any(|a: &FilterArgument| a.name == argument.name) {
                    return Err(unsupported(effect, "duplicate Gaussian argument"));
                }
                arguments.push(FilterArgument { name: argument.name.clone(), value });
                continue;
            }
            let (number, value) = match argument.value {
                ArgumentValue::Float32Bits(bits) => (ExactNumber::Float32Bits(bits), f64::from(f32::from_bits(bits))),
                _ => return Err(unsupported(effect, "point parameter requires its stored floating type")),
            };
            numeric.push((argument.name.as_str(), value));
            arguments.push(FilterArgument { name: argument.name.clone(), value: FilterValue::Number(number) });
        }
        if unpremultiply {
            // Exact zero-argument version/envelope was checked before mapping.
        } else if absolute {
            photocraft_algo::gimp::registry::absolute_linear_member(effect.version.as_deref())
                .map_err(|_| unsupported(effect, "Absolute Linear zero-argument schema"))?;
        } else if red_eye {
            photocraft_algo::gimp::registry::red_eye_member(
                effect.version.as_deref(),
                crate::xcf_point_replace_source::red_eye_source::threshold(effect).ok_or_else(|| unsupported(effect, "Red Eye threshold"))?,
            )
            .map_err(|_| unsupported(effect, "Red Eye typed parameters"))?;
        } else if video {
            photocraft_compose::gimp_video_degradation::typed_arguments(&arguments).map_err(|_| unsupported(effect, "Video Degradation typed schema"))?;
        } else if color_rotate || channel_mixer || mono_signed || color_exchange || color_overlay || color_to_alpha || component_extract || alpha_clip {
            // The dedicated typed constructor already validated every name.
        } else if pixelize {
            if arguments.len() != 8 {
                return Err(unsupported(effect, "missing Pixelize argument"));
            }
        } else if median {
            if arguments.len() != 6 {
                return Err(unsupported(effect, "missing Median Blur argument"));
            }
            if finite_median {
                let radius = arguments.iter().find(|a| a.name == "radius").map(|a| &a.value);
                let high = arguments.iter().find(|a| a.name == "high-precision").map(|a| &a.value);
                if !matches!(
                    (radius, high),
                    (Some(FilterValue::Number(ExactNumber::Signed { value: 3, .. })), Some(FilterValue::Bool(false)))
                        | (Some(FilterValue::Number(ExactNumber::Signed { value: 4, .. })), Some(FilterValue::Bool(true)))
                ) {
                    return Err(unsupported(effect, "unqualified finite Median parameter combination"));
                }
            }
        } else if motion {
            if arguments.len() != 2 {
                return Err(unsupported(effect, "missing Motion Blur argument"));
            }
        } else if box_blur {
            if arguments.len() != 1 {
                return Err(unsupported(effect, "missing Box Blur radius"));
            }
        } else if color_balance {
            if arguments.len() != 2 {
                return Err(unsupported(effect, "Color Balance current schema"));
            }
        } else if levels {
            if effects.len() != 1 {
                return Err(unsupported(effect, "unqualified structured Levels stack"));
            }
        } else if curves {
            if arguments.len() != 2 {
                return Err(unsupported(effect, "missing Curves argument"));
            }
        } else if gaussian {
            if arguments.len() != 5 {
                return Err(unsupported(effect, "missing Gaussian argument"));
            }
            if finite_gaussian {
                let x = arguments.iter().find(|a| a.name == "std-dev-x").map(|a| &a.value);
                let y = arguments.iter().find(|a| a.name == "std-dev-y").map(|a| &a.value);
                let filter = arguments.iter().find(|a| a.name == "filter").map(|a| &a.value);
                let qualified = match (x, y, filter) {
                    (
                        Some(FilterValue::Number(ExactNumber::Float32Bits(x))),
                        Some(FilterValue::Number(ExactNumber::Float32Bits(y))),
                        Some(FilterValue::Enum { value, .. }),
                    ) => {
                        (*x == 2.25_f32.to_bits() && *y == 1.3_f32.to_bits() && *value == 0)
                            || (*x == 3.75_f32.to_bits() && *y == 0.8_f32.to_bits() && *value == 1)
                    }
                    _ => false,
                };
                if !qualified {
                    return Err(unsupported(effect, "unqualified finite Gaussian parameter combination"));
                }
            }
        } else {
            photocraft_algo::gimp::registry::point_member(&effect.operation, effect.version.as_deref(), &numeric)
                .map_err(|_| unsupported(effect, "unqualified operation version or arguments"))?;
        }
        let id = u64::try_from(ordinal)
            .ok()
            .and_then(|n| first_id.checked_add(n))
            .filter(|id| *id != 0)
            .ok_or_else(|| unsupported(effect, "filter identity overflow"))?;
        nodes.push(FilterNode {
            id: FilterId(id),
            source_display_name: Some(effect.name.clone()),
            source_icon: Some(effect.icon.clone()),
            operation: OperationIdentity {
                name: effect.operation.clone(),
                stored_version: effect.version.clone().ok_or_else(|| unsupported(effect, "missing operation version"))?,
                schema_epoch: if unpremultiply {
                    crate::xcf_import::unpremultiply::EPOCH
                } else if alpha_clip_finite {
                    "xcf22-stock-alpha-clip-finite-precision-v1"
                } else if alpha_clip {
                    "xcf22-stock-alpha-clip-precision-v1"
                } else if component_extract {
                    component_epoch
                } else if color_to_alpha {
                    "xcf-22-stock-color-to-alpha-precision-v1"
                } else if color_exchange {
                    "xcf-22-stock-color-exchange-precision-v1"
                } else if color_overlay {
                    "xcf-22-stock-color-overlay-six-precision-v1"
                } else if invert_gamma {
                    "xcf-22-stock-invert-gamma-precision-v1"
                } else if invert_linear {
                    "xcf-22-stock-invert-linear-precision-v1"
                } else if alien_map {
                    alien_epoch
                } else if sepia {
                    "xcf-22-stock-sepia-precision-v1"
                } else if threshold_alpha {
                    "xcf-22-stock-threshold-alpha-precision-v1"
                } else if threshold {
                    "xcf-22-stock-threshold-precision-v1"
                } else if saturation {
                    saturation_epoch
                } else if desaturate {
                    "xcf-22-stock-desaturate-precision-v1"
                } else if temperature {
                    temperature_epoch
                } else if color_balance {
                    color_balance_epoch
                } else if exposure_black_precision {
                    exposure_epoch
                } else if gegl_levels_precision {
                    "xcf-22-stock-gegl-levels-precision-v1"
                } else if brightness_precision {
                    "xcf-22-stock-brightness-contrast-precision-v1"
                } else if value_precision {
                    "xcf-22-stock-value-invert-precision-v1"
                } else if clip {
                    "xcf-22-stock-rgb-clip-hdr-v1"
                } else if video {
                    video_epoch.ok_or_else(|| unsupported(effect, "Video Degradation source epoch"))?
                } else if color_rotate {
                    "xcf-22-stock-color-rotate-perceptual-v1"
                } else if absolute {
                    "xcf-22-stock-absolute-linear-precision-v1"
                } else if channel_mixer {
                    "xcf-22-stock-channel-mixer-precision-v1"
                } else if mono_signed {
                    "xcf-22-stock-mono-mixer-signed-precision-v1"
                } else if red_eye {
                    red_eye_epoch.ok_or_else(|| unsupported(effect, "Red Eye source epoch"))?
                } else if mono_edits {
                    "xcf-22-stock-mono-mixer-edits-v1"
                } else if point_stack {
                    "xcf-22-stock-point-stack-edits-v1"
                } else if group_gaussian {
                    "xcf22-isolated-small-auto-v1"
                } else if median {
                    if domain == NeighborhoodDomain::MedianGray {
                        "xcf22-gray-alpha-median-v1"
                    } else if finite_median {
                        "xcf22-rgb-alpha-median-v1"
                    } else {
                        "xcf22-rgb-u8-quantized-median-v1"
                    }
                } else if motion {
                    "xcf22-rgb-u8-linear-motion-v1"
                } else if pixelize {
                    "xcf22-rgb-u8-square-pixelize-v1"
                } else if box_blur {
                    "xcf22-rgb-box-replace-v1"
                } else if finite_gaussian {
                    "xcf22-rgb-alpha-finite-gaussian-v1"
                } else if gaussian {
                    "xcf-22-stock-gaussian-replace"
                } else if levels {
                    "xcf-22-stock-levels-replace"
                } else if curves {
                    "xcf-22-stock-curves-replace"
                } else {
                    "xcf-22-stock-point-replace"
                }
                .into(),
            },
            arguments,
            enabled,
            opacity_f64_bits: f64::from(f32::from_bits(source.opacity_bits.ok_or_else(|| unsupported(effect, "missing opacity"))?)).to_bits(),
            mask: masks.get(ordinal).copied().flatten(),
            auxiliary: Vec::new(),
            source_clipping: source.clipping,
            source_region: source.region,
            composition: Some(GimpComposition {
                compatibility_epoch: if unpremultiply {
                    crate::xcf_import::unpremultiply::EPOCH
                } else if video_epoch == Some(photocraft_compose::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH) {
                    photocraft_compose::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH
                } else if red_eye_epoch == Some(photocraft_compose::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH) {
                    photocraft_compose::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH
                } else if generic_replace {
                    "xcf-22-stock-point-replace-fields-v1"
                } else if alien_map {
                    alien_epoch
                } else if sepia {
                    "xcf-22-stock-sepia-precision-v1"
                } else if threshold_alpha {
                    "xcf-22-stock-threshold-alpha-precision-v1"
                } else if threshold {
                    "xcf-22-stock-threshold-precision-v1"
                } else if saturation {
                    saturation_epoch
                } else if desaturate {
                    "xcf-22-stock-desaturate-precision-v1"
                } else if temperature {
                    temperature_epoch
                } else if color_balance {
                    color_balance_epoch
                } else if exposure_black_precision {
                    exposure_epoch
                } else if gegl_levels_precision {
                    "xcf-22-stock-gegl-levels-precision-v1"
                } else if brightness_precision {
                    "xcf-22-stock-brightness-contrast-precision-v1"
                } else if value_precision {
                    "xcf-22-stock-value-invert-precision-v1"
                } else if clip {
                    "xcf-22-stock-rgb-clip-hdr-v1"
                } else if video {
                    video_epoch.ok_or_else(|| unsupported(effect, "Video Degradation source epoch"))?
                } else if color_rotate {
                    "xcf-22-stock-color-rotate-perceptual-v1"
                } else if absolute {
                    "xcf-22-stock-absolute-linear-precision-v1"
                } else if channel_mixer {
                    "xcf-22-stock-channel-mixer-precision-v1"
                } else if mono_signed {
                    "xcf-22-stock-mono-mixer-signed-precision-v1"
                } else if mono_edits {
                    "xcf-22-stock-mono-mixer-edits-v1"
                } else if point_stack {
                    "xcf-22-stock-point-stack-edits-v1"
                } else if group_gaussian {
                    "xcf22-isolated-small-auto-v1"
                } else if median {
                    if domain == NeighborhoodDomain::MedianGray {
                        "xcf22-gray-alpha-median-v1"
                    } else if finite_median {
                        "xcf22-rgb-alpha-median-v1"
                    } else {
                        "xcf22-rgb-u8-quantized-median-v1"
                    }
                } else if motion {
                    "xcf22-rgb-u8-linear-motion-v1"
                } else if pixelize {
                    "xcf22-rgb-u8-square-pixelize-v1"
                } else if box_blur {
                    "xcf22-rgb-box-replace-v1"
                } else if finite_gaussian {
                    "xcf22-rgb-alpha-finite-gaussian-v1"
                } else if gaussian {
                    "xcf-22-stock-gaussian-replace"
                } else if levels {
                    "xcf-22-stock-levels-replace"
                } else if curves {
                    "xcf-22-stock-curves-replace"
                } else {
                    "xcf-22-stock-point-replace"
                }
                .into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: source.blend_space.ok_or_else(|| unsupported(effect, "missing blend space"))?,
                stored_composite_space: source.composite_space.ok_or_else(|| unsupported(effect, "missing composite space"))?,
                stored_operator: source.composite_mode.ok_or_else(|| unsupported(effect, "missing composite operator"))?,
                resolved: None,
                pass_through: false,
            }),
        });
    }
    Ok(FilterStack { owner, nodes, revision: 0 })
}

/// Explicit opt-in structural graph argument bridge. The normal stock/source
/// admission path does not call this; unknown source graph types remain inert.
#[allow(clippy::too_many_arguments)]
pub fn graph_argument_candidate<'a>(
    argument:&'a photocraft_xcf::Argument,
    schemas:&photocraft_algo::gimp::typed_schema::PreparedSchemaIndex<'_,'_>,
    limits:crate::xcf_graph_candidate::Limits,
    control:&photocraft_raster::Interrupt<'_>,
    fresh:&mut dyn FnMut()->photocraft_doc::item_identity::GraphNodeId,
    resolve:&mut crate::xcf_graphs::GraphSchemaResolver<'_>,
    ports:&mut crate::xcf_graph_candidate::PortValidator<'_>,
    auxiliary:&mut crate::xcf_graph_candidate::AuxiliaryValidator<'_>,
    resources:&mut crate::xcf_graph_candidate::ResourceValidator<'_>,
    charge:&mut dyn FnMut(usize)->Result<(),IoError>,
)->Result<crate::xcf_graph_candidate::GraphCandidate<'a>,IoError>{
    control.check().map_err(|_|IoError::Cancelled)?;
    if argument.configuration.is_some(){return Err(IoError::Unsupported("XCF graph argument also contains unbound configuration".into()));}
    let photocraft_xcf::ArgumentValue::String(original)=&argument.value else{return Err(IoError::Unsupported("XCF graph argument requires original serialized string".into()));};
    let graph=argument.graph.as_ref().ok_or_else(||IoError::Unsupported("XCF graph argument has no retained grammar".into()))?;
    crate::xcf_graph_candidate::map_with_schemas(graph,original,schemas,limits,control,fresh,resolve,ports,auxiliary,resources,charge)
}
