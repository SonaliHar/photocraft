//! Exact observed stock identity/version dispatch. A member does not qualify every profile/configuration.
use super::point::PointOperation;
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkingEncoding {
    Perceptual,
    Linear,
    Nonlinear,
}

/// Code availability is distinct from source/profile/render qualification.
/// These kernels never expand the qualified point dispatch table below.
#[derive(Clone, Copy, Debug)]
pub struct CandidateKernel {
    pub operation: &'static str,
    pub version: &'static str,
    pub parameters: &'static [&'static str],
    pub domain: &'static str,
}

pub const CANDIDATE_KERNELS: &[CandidateKernel] = &[
    CandidateKernel {
        operation: "gegl:stretch-contrast",
        version: "0:0",
        parameters: &["keep-colors", "perceptual"],
        domain: "standalone opaque unit-range linear RGB F32, false/false; requires full declared input extent and caller CMS; import qualification pending",
    },
    CandidateKernel {
        operation: "gegl:checkerboard",
        version: "0:0",
        parameters: &["x", "y", "x-offset", "y-offset", "color1", "color2"],
        domain: "standalone bounded nonnegative ROI, x=16/17 y=16 and zero offsets; explicit caller-encoded opaque RGB colors; import qualification pending",
    },
    CandidateKernel {
        operation: "gegl:edge-sobel",
        version: "0:0",
        parameters: &["horizontal", "vertical", "keep-sign"],
        domain: "standalone standard unnormalized single-direction Sobel, alpha-less RGB/Gray F32/F64 and explicit transfer/abyss; GEGL profile/normalization/native equivalence pending",
    },
];

pub fn candidate_kernel(operation: &str, version: Option<&str>) -> Option<&'static CandidateKernel> {
    CANDIDATE_KERNELS.iter().find(|member| member.operation == operation && Some(member.version) == version)
}

#[derive(Clone, Copy, Debug)]
pub struct PointMember {
    pub operation: &'static str,
    pub version: &'static str,
    pub encoding: WorkingEncoding,
    pub parameters: &'static [&'static str],
    /// Evaluator domain (narrower than public operation schema where unqualified).
    pub domains: &'static [NumericDomain],
}

#[derive(Clone, Copy, Debug)]
pub struct NumericDomain {
    pub name: &'static str,
    pub minimum: f64,
    pub maximum: f64,
    /// Exposure additionally requires a finite exp2 multiplier; touched output must be finite.
    pub finite: bool,
}

pub const POINT_MEMBERS: &[PointMember] = &[
    PointMember {
        operation: "gegl:alien-map",
        version: "0:0",
        encoding: WorkingEncoding::Nonlinear,
        parameters: &[
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
        ],
        domains: &[
            NumericDomain { name: "color-model", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "cpn-1-frequency", minimum: 0.0, maximum: 20.0, finite: true },
            NumericDomain { name: "cpn-2-frequency", minimum: 0.0, maximum: 20.0, finite: true },
            NumericDomain { name: "cpn-3-frequency", minimum: 0.0, maximum: 20.0, finite: true },
            NumericDomain { name: "cpn-1-phaseshift", minimum: 0.0, maximum: 360.0, finite: true },
            NumericDomain { name: "cpn-2-phaseshift", minimum: 0.0, maximum: 360.0, finite: true },
            NumericDomain { name: "cpn-3-phaseshift", minimum: 0.0, maximum: 360.0, finite: true },
            NumericDomain { name: "cpn-1-keep", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "cpn-2-keep", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "cpn-3-keep", minimum: 0.0, maximum: 1.0, finite: true },
        ],
    },
    PointMember {
        operation: "gegl:sepia",
        version: "0:0",
        encoding: WorkingEncoding::Nonlinear,
        parameters: &["scale", "srgb"],
        domains: &[
            NumericDomain { name: "scale", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "srgb", minimum: 0.0, maximum: 1.0, finite: true },
        ],
    },
    PointMember {
        operation: "gimp:threshold-alpha",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["value"],
        domains: &[NumericDomain { name: "value", minimum: 0.0, maximum: 1.0, finite: true }],
    },
    PointMember {
        operation: "gimp:threshold",
        version: "0:0",
        encoding: WorkingEncoding::Nonlinear,
        parameters: &["channel", "low", "high"],
        domains: &[
            NumericDomain { name: "channel", minimum: 0.0, maximum: 5.0, finite: true },
            NumericDomain { name: "low", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "high", minimum: 0.0, maximum: 1.0, finite: true },
        ],
    },
    PointMember {
        operation: "gegl:saturation",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["scale", "colorspace"],
        domains: &[
            NumericDomain { name: "scale", minimum: 0.0, maximum: 10.0, finite: true },
            NumericDomain { name: "colorspace", minimum: 0.0, maximum: 2.0, finite: true },
        ],
    },
    PointMember {
        operation: "gimp:desaturate",
        version: "0:0",
        encoding: WorkingEncoding::Nonlinear,
        parameters: &["mode"],
        domains: &[NumericDomain { name: "mode", minimum: 0.0, maximum: 4.0, finite: true }],
    },
    PointMember {
        operation: "gegl:color-temperature",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["original-temperature", "intended-temperature"],
        domains: &[
            NumericDomain { name: "original-temperature", minimum: 1000.0, maximum: 12000.0, finite: true },
            NumericDomain { name: "intended-temperature", minimum: 1000.0, maximum: 12000.0, finite: true },
        ],
    },
    PointMember {
        operation: "gegl:rgb-clip",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["clip-low", "low-limit", "clip-high", "high-limit"],
        domains: &[
            NumericDomain { name: "clip-low", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "low-limit", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "clip-high", minimum: 1.0, maximum: 1.0, finite: true },
            NumericDomain { name: "high-limit", minimum: 0.0, maximum: 1.0, finite: true },
        ],
    },
    PointMember {
        operation: "gegl:mono-mixer",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["red", "green", "blue", "preserve-luminosity"],
        domains: &[
            NumericDomain { name: "red", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "green", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "blue", minimum: 0.0, maximum: 1.0, finite: true },
            NumericDomain { name: "preserve-luminosity", minimum: 0.0, maximum: 0.0, finite: true },
        ],
    },
    PointMember { operation: "gegl:value-invert", version: "0:0", encoding: WorkingEncoding::Nonlinear, parameters: &[], domains: &[] },
    PointMember {
        operation: "gimp:posterize",
        version: "0:0",
        encoding: WorkingEncoding::Nonlinear,
        parameters: &["levels"],
        domains: &[NumericDomain { name: "levels", minimum: 2.0, maximum: 256.0, finite: true }],
    },
    PointMember {
        operation: "gegl:exposure",
        version: "1:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["exposure", "black-level"],
        domains: &[
            NumericDomain { name: "exposure", minimum: f64::NEG_INFINITY, maximum: f64::INFINITY, finite: true },
            NumericDomain { name: "black-level", minimum: -0.1, maximum: 0.1, finite: true },
        ],
    },
    PointMember {
        operation: "gegl:brightness-contrast",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["brightness", "contrast"],
        domains: &[
            NumericDomain { name: "brightness", minimum: -3.0, maximum: 3.0, finite: true },
            NumericDomain { name: "contrast", minimum: -5.0, maximum: 5.0, finite: true },
        ],
    },
    PointMember { operation: "gegl:invert-linear", version: "0:0", encoding: WorkingEncoding::Linear, parameters: &[], domains: &[] },
    PointMember { operation: "gegl:invert-gamma", version: "0:0", encoding: WorkingEncoding::Nonlinear, parameters: &[], domains: &[] },
    PointMember {
        operation: "gegl:levels",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["in-low", "in-high", "out-low", "out-high"],
        domains: &[
            NumericDomain { name: "in-low", minimum: f64::NEG_INFINITY, maximum: f64::INFINITY, finite: true },
            NumericDomain { name: "in-high", minimum: f64::NEG_INFINITY, maximum: f64::INFINITY, finite: true },
            NumericDomain { name: "out-low", minimum: f64::NEG_INFINITY, maximum: f64::INFINITY, finite: true },
            NumericDomain { name: "out-high", minimum: f64::NEG_INFINITY, maximum: f64::INFINITY, finite: true },
        ],
    },
];

pub const COLOR_TO_ALPHA_MEMBER: PointMember = PointMember {
    operation: "gegl:color-to-alpha",
    version: "0:0",
    encoding: WorkingEncoding::Nonlinear,
    parameters: &["color", "transparency-threshold", "opacity-threshold"],
    domains: &[
        NumericDomain { name: "transparency-threshold", minimum: 0., maximum: 1., finite: true },
        NumericDomain { name: "opacity-threshold", minimum: 0., maximum: 1., finite: true },
    ],
};

/// Directed hue parameters operate on explicitly selected Perceptual Float RGB.
/// Relative arc constraints are validated by the typed operation, not aliases.
pub fn color_rotate_member(
    version: Option<&str>,
    operation: super::point_color_rotate::ColorRotate,
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    operation.validate()?;
    Ok((
        PointMember {
            operation: "gegl:color-rotate",
            version: "0:0",
            encoding: WorkingEncoding::Perceptual,
            parameters: &["src-clockwise", "src-from", "src-to", "dest-clockwise", "dest-from", "dest-to", "threshold", "gray-mode", "hue", "saturation"],
            domains: &[
                NumericDomain { name: "src-from", minimum: 0., maximum: 360., finite: true },
                NumericDomain { name: "src-to", minimum: 0., maximum: 360., finite: true },
                NumericDomain { name: "dest-from", minimum: 0., maximum: 360., finite: true },
                NumericDomain { name: "dest-to", minimum: 0., maximum: 360., finite: true },
                NumericDomain { name: "threshold", minimum: 0., maximum: 1., finite: true },
                NumericDomain { name: "hue", minimum: 0., maximum: 360., finite: true },
                NumericDomain { name: "saturation", minimum: 0., maximum: 1., finite: true },
            ],
        },
        PointOperation::ColorRotate { operation },
    ))
}
/// Explicit coordinate-only nonlinear Float RGB monitor operation. This member
/// does not qualify source geometry, profiles, saved envelopes or sample storage.
pub fn video_degradation_member(
    version: Option<&str>,
    operation: super::point_video_degradation::VideoDegradation,
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let point = PointOperation::VideoDegradation { operation };
    point.validate()?;
    Ok((
        PointMember {
            operation: "gegl:video-degradation",
            version: "0:0",
            encoding: WorkingEncoding::Nonlinear,
            parameters: &["pattern", "additive", "rotated"],
            domains: &[NumericDomain { name: "pattern", minimum: 0., maximum: 8., finite: true }],
        },
        point,
    ))
}
/// Zero-argument, independently observed Linear Float RGB Unpremultiply slice.
pub fn unpremultiply_linear_member(version: Option<&str>) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") { return Err(RasterInputError::UnsupportedConversion); }
    Ok((PointMember { operation: "gegl:unpremultiply", version: "0:0", encoding: WorkingEncoding::Linear, parameters: &[], domains: &[] }, PointOperation::UnpremultiplyLinear))
}
/// Zero-argument, independently observed Linear Float Absolute slice.
pub fn absolute_linear_member(version: Option<&str>) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    Ok((
        PointMember { operation: "gegl:absolute", version: "0:0", encoding: WorkingEncoding::Linear, parameters: &[], domains: &[] },
        PointOperation::AbsoluteLinear,
    ))
}
pub fn red_eye_member(version: Option<&str>, threshold: f64) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::RedEyeRemoval { threshold };
    operation.validate()?;
    Ok((
        PointMember {
            operation: "gegl:red-eye-removal",
            version: "0:0",
            encoding: WorkingEncoding::Nonlinear,
            parameters: &["threshold"],
            domains: &[NumericDomain { name: "threshold", minimum: 0., maximum: 0.8, finite: true }],
        },
        operation,
    ))
}
pub fn channel_mixer_member(
    version: Option<&str>,
    operation: super::point_channel_mixer::ChannelMixer,
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    operation.try_eval_linear_float(&[0.; 4], &mut [0.; 4])?;
    Ok((
        PointMember {
            operation: "gegl:channel-mixer",
            version: "0:0",
            encoding: WorkingEncoding::Linear,
            parameters: &["rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain", "preserve-luminosity"],
            domains: &[
                NumericDomain { name: "rr-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "rg-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "rb-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "gr-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "gg-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "gb-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "br-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "bg-gain", minimum: -2., maximum: 2., finite: true },
                NumericDomain { name: "bb-gain", minimum: -2., maximum: 2., finite: true },
            ],
        },
        PointOperation::ChannelMixer { gains: operation.gains, preserve_luminosity: operation.preserve_luminosity },
    ))
}

pub fn mono_mixer_signed_member(
    version: Option<&str>,
    operation: super::point_mono_mixer::MonoMixer,
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    operation.validate()?;
    Ok((
        PointMember {
            operation: "gegl:mono-mixer",
            version: "0:0",
            encoding: WorkingEncoding::Linear,
            parameters: &["red", "green", "blue", "preserve-luminosity"],
            domains: &[
                NumericDomain { name: "red", minimum: -5., maximum: 5., finite: true },
                NumericDomain { name: "green", minimum: -5., maximum: 5., finite: true },
                NumericDomain { name: "blue", minimum: -5., maximum: 5., finite: true },
            ],
        },
        PointOperation::MonoMixerSigned {
            red: operation.red,
            green: operation.green,
            blue: operation.blue,
            preserve_luminosity: operation.preserve_luminosity,
        },
    ))
}

/// Code availability only; the caller separately qualifies source, literal and CMS.
pub fn semi_flatten_member(version: Option<&str>, working_color: [f32; 4]) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::SemiFlatten { color: working_color };
    operation.validate()?;
    Ok((PointMember {
        operation: "gimp:semi-flatten",
        version: "0:0",
        encoding: WorkingEncoding::Linear,
        parameters: &["color"],
        domains: &[],
    }, operation))
}

pub fn color_overlay_member(version: Option<&str>, working_color: [f32; 4], srgb: bool) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::ColorOverlay { color: working_color };
    operation.validate()?;
    Ok((
        PointMember {
            operation: "gegl:color-overlay",
            version: "0:0",
            encoding: if srgb { WorkingEncoding::Nonlinear } else { WorkingEncoding::Linear },
            parameters: &["value", "srgb"],
            domains: &[],
        },
        operation,
    ))
}
/// Literal color authority remains distinct from scalar threshold arguments.
pub fn color_to_alpha_member(
    version: Option<&str>,
    color: [f32; 4],
    transparency: f32,
    opacity: f32,
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::ColorToAlpha { color, transparency, opacity };
    operation.validate()?;
    Ok((COLOR_TO_ALPHA_MEMBER, operation))
}

/// Exact booleans and Float32 limits; caller qualifies observed working encoding.
pub fn alpha_clip_member(
    version: Option<&str>,
    clip_low: bool,
    low_limit: f32,
    clip_high: bool,
    high_limit: f32,
    encoding: WorkingEncoding,
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::AlphaClipPrecision { clip_low, low_limit, clip_high, high_limit };
    operation.validate()?;
    Ok((
        PointMember {
            operation: "gegl:alpha-clip",
            version: "0:0",
            encoding,
            parameters: &["clip-low", "low-limit", "clip-high", "high-limit"],
            domains: &[
                NumericDomain { name: "low-limit", minimum: 0., maximum: 1., finite: true },
                NumericDomain { name: "high-limit", minimum: 0., maximum: 1., finite: true },
            ],
        },
        operation,
    ))
}

/// Exact booleans and Float32 limits; caller qualifies observed working encoding.
pub fn alpha_clip_finite_member(
    version: Option<&str>,
    clip_low: bool,
    low_limit: f32,
    clip_high: bool,
    high_limit: f32,
    encoding: WorkingEncoding,
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::AlphaClipFinitePrecision { clip_low, low_limit, clip_high, high_limit };
    operation.validate()?;
    Ok((
        PointMember {
            operation: "gegl:alpha-clip",
            version: "0:0",
            encoding,
            parameters: &["clip-low", "low-limit", "clip-high", "high-limit"],
            domains: &[
                NumericDomain { name: "low-limit", minimum: -(f32::MAX as f64), maximum: f32::MAX as f64, finite: true },
                NumericDomain { name: "high-limit", minimum: -(f32::MAX as f64), maximum: f32::MAX as f64, finite: true },
            ],
        },
        operation,
    ))
}

/// Typed ParamChoice and booleans; no numeric boolean aliases.
pub fn component_extract_member(version: Option<&str>, component: u8, invert: bool, linear: bool) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::ComponentExtract { component, invert, linear };
    operation.validate()?;
    Ok((
        PointMember {
            operation: "gegl:component-extract",
            version: "0:0",
            encoding: WorkingEncoding::Nonlinear,
            parameters: &["component", "invert", "linear"],
            domains: &[],
        },
        operation,
    ))
}

pub const COLOR_EXCHANGE_MEMBER: PointMember = PointMember {
    operation: "gegl:color-exchange",
    version: "0:0",
    encoding: WorkingEncoding::Nonlinear,
    parameters: &["from-color", "to-color", "red-threshold", "green-threshold", "blue-threshold"],
    domains: &[
        NumericDomain { name: "red-threshold", minimum: 0., maximum: 1., finite: true },
        NumericDomain { name: "green-threshold", minimum: 0., maximum: 1., finite: true },
        NumericDomain { name: "blue-threshold", minimum: 0., maximum: 1., finite: true },
    ],
};

/// Typed literal color dispatch. Color operands are never scalar parameter aliases.
pub fn color_exchange_member(
    version: Option<&str>,
    from: [f32; 4],
    to: [f32; 4],
    thresholds: [f32; 3],
) -> Result<(PointMember, PointOperation), RasterInputError> {
    if version != Some("0:0") {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let operation = PointOperation::ColorExchange { from, to, thresholds };
    operation.validate()?;
    Ok((COLOR_EXCHANGE_MEMBER, operation))
}

/// Numeric values must be decoded from exact stored bits; no color flattening.
pub fn point_member(operation: &str, version: Option<&str>, args: &[(&str, f64)]) -> Result<(PointMember, PointOperation), RasterInputError> {
    let mut member =
        POINT_MEMBERS.iter().find(|m| m.operation == operation && Some(m.version) == version).copied().ok_or(RasterInputError::UnsupportedConversion)?;
    if args.len() != member.parameters.len() || args.iter().any(|(name, value)| !member.parameters.contains(name) || !value.is_finite()) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let get = |name: &str| -> Result<f64, RasterInputError> {
        let mut found = args.iter().filter(|(n, _)| *n == name);
        let value = found.next().map(|(_, v)| *v).ok_or(RasterInputError::UnsupportedConversion)?;
        if found.next().is_some() {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(value)
    };
    let evaluator = match operation {
        "gegl:alien-map" => {
            let model = get("color-model")?;
            if !matches!(model, 0.0 | 1.0) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let keep = [get("cpn-1-keep")?, get("cpn-2-keep")?, get("cpn-3-keep")?];
            if keep.iter().any(|v| !matches!(*v, 0.0 | 1.0)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let frequency = [get("cpn-1-frequency")?, get("cpn-2-frequency")?, get("cpn-3-frequency")?];
            let phase = [get("cpn-1-phaseshift")?, get("cpn-2-phaseshift")?, get("cpn-3-phaseshift")?];
            let keep = keep.map(|v| v == 1.0);
            if model == 0.0 { PointOperation::AlienMap { frequency, phase, keep } } else { PointOperation::AlienMapHsl { frequency, phase, keep } }
        }
        "gegl:sepia" => {
            let srgb = get("srgb")?;
            if !matches!(srgb, 0.0 | 1.0) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            if srgb == 0.0 {
                member.encoding = WorkingEncoding::Linear;
            }
            PointOperation::Sepia { scale: get("scale")?, srgb: srgb == 1.0 }
        }
        "gimp:threshold-alpha" => PointOperation::ThresholdAlpha { value: get("value")? },
        "gimp:threshold" => {
            let channel = get("channel")?;
            if !(0.0..=5.0).contains(&channel) || channel.fract() != 0.0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            PointOperation::Threshold { channel: channel as u8, low: get("low")?, high: get("high")? }
        }
        "gegl:saturation" => {
            if !(0.0..=2.0).contains(&get("colorspace")?) || get("colorspace")?.fract() != 0.0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            PointOperation::Saturation { scale: get("scale")?, colorspace: get("colorspace")? as u8 }
        }
        "gimp:desaturate" => {
            let mode = get("mode")?;
            if !(0.0..=4.0).contains(&mode) || mode.fract() != 0.0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            if mode == 3.0 {
                member.encoding = WorkingEncoding::Linear;
            }
            PointOperation::Desaturate { mode: mode as u8 }
        }
        "gegl:color-temperature" => PointOperation::ColorTemperature { original: get("original-temperature")?, intended: get("intended-temperature")? },
        "gegl:rgb-clip" => {
            let low = get("clip-low")?;
            if !matches!(low, 0.0 | 1.0) || get("clip-high")? != 1.0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            PointOperation::RgbClip { clip_low: low == 1.0, low_limit: get("low-limit")?, clip_high: true, high_limit: get("high-limit")? }
        }
        "gegl:mono-mixer" => {
            if get("preserve-luminosity")? != 0.0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            PointOperation::MonoMixer { red: get("red")?, green: get("green")?, blue: get("blue")? }
        }
        "gegl:value-invert" => PointOperation::ValueInvert,
        "gimp:posterize" => {
            let levels = get("levels")?;
            if !(2.0..=256.0).contains(&levels) || levels.fract() != 0.0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            PointOperation::Posterize { levels: levels as u16 }
        }
        "gegl:exposure" => PointOperation::Exposure { stops: get("exposure")?, black_level: get("black-level")? },
        "gegl:brightness-contrast" => PointOperation::BrightnessContrast { brightness: get("brightness")?, contrast: get("contrast")? },
        "gegl:levels" => {
            PointOperation::Levels { input_low: get("in-low")?, input_high: get("in-high")?, output_low: get("out-low")?, output_high: get("out-high")? }
        }
        "gegl:invert-linear" | "gegl:invert-gamma" => PointOperation::Invert,
        _ => return Err(RasterInputError::UnsupportedConversion),
    };
    evaluator.validate()?;
    Ok((member, evaluator))
}
