//! Explicit-profile Color Rotate candidate. No source admission.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_color_rotate::{ColorRotate, GrayMode, HueArc};
use photocraft_cms::{
    xcf_color::{convert_encoding_f64, effective_profile_from_definition, Encoding},
    Builtin, Intent, Profile, Transform,
};
use photocraft_doc::SampleEncoding;
use photocraft_raster::{Interrupt, RasterInputError};
pub const COLOR_ROTATE_PERCEPTUAL_EPOCH: &str = "xcf-22-stock-color-rotate-perceptual-v1";
pub fn typed_arguments(arguments: &[photocraft_doc::xcf_filters::FilterArgument]) -> Result<ColorRotate> {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    if arguments.len() != 10 {
        return Err(CompositionError::Unsupported("Color Rotate argument count"));
    }
    let mut result = ColorRotate {
        source: HueArc { from_degrees: 0., to_degrees: 90., clockwise: false },
        destination: HueArc { from_degrees: 0., to_degrees: 90., clockwise: false },
        gray_threshold: 0.,
        gray_mode: GrayMode::TreatAs,
        gray_hue_degrees: 0.,
        gray_saturation: 0.,
    };
    let number = |value: &FilterValue| -> Result<f32> {
        match value {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) => Ok(f32::from_bits(*bits)),
            _ => Err(CompositionError::Unsupported("Color Rotate stored numeric width")),
        }
    };
    let boolean = |value: &FilterValue| -> Result<bool> {
        match value {
            FilterValue::Bool(value) => Ok(*value),
            _ => Err(CompositionError::Unsupported("Color Rotate stored Boolean type")),
        }
    };
    let mut seen = 0u16;
    for argument in arguments {
        let bit = match argument.name.as_str() {
            "src-clockwise" => 1,
            "src-from" => 2,
            "src-to" => 4,
            "dest-clockwise" => 8,
            "dest-from" => 16,
            "dest-to" => 32,
            "threshold" => 64,
            "gray-mode" => 128,
            "hue" => 256,
            "saturation" => 512,
            _ => return Err(CompositionError::Unsupported("Color Rotate argument name")),
        };
        if seen & bit != 0 {
            return Err(CompositionError::Unsupported("Color Rotate duplicate argument"));
        }
        seen |= bit;
        match argument.name.as_str() {
            "src-clockwise" => result.source.clockwise = boolean(&argument.value)?,
            "src-from" => result.source.from_degrees = number(&argument.value)?,
            "src-to" => result.source.to_degrees = number(&argument.value)?,
            "dest-clockwise" => result.destination.clockwise = boolean(&argument.value)?,
            "dest-from" => result.destination.from_degrees = number(&argument.value)?,
            "dest-to" => result.destination.to_degrees = number(&argument.value)?,
            "threshold" => result.gray_threshold = number(&argument.value)?,
            "gray-mode" => {
                result.gray_mode = match &argument.value {
                    FilterValue::Enum { domain, value: 0 } if domain == "gegl:color-rotate/gray-mode" => GrayMode::TreatAs,
                    FilterValue::Enum { domain, value: 1 } if domain == "gegl:color-rotate/gray-mode" => GrayMode::ChangeTo,
                    _ => return Err(CompositionError::Unsupported("Color Rotate gray mode domain")),
                }
            }
            "hue" => result.gray_hue_degrees = number(&argument.value)?,
            "saturation" => result.gray_saturation = number(&argument.value)?,
            _ => return Err(CompositionError::Unsupported("Color Rotate argument name")),
        }
    }
    result.validate()?;
    if !witnessed_source_parameters(result) {
        return Err(CompositionError::Unsupported("Color Rotate unqualified source arc/gray configuration"));
    }
    Ok(result)
}
/// Source/current admission for the independently witnessed native Perceptual
/// controls. This does not restrict the generic directed-hue math kernel.
pub fn witnessed_source_parameters(operation: ColorRotate) -> bool {
    witnessed_source_triplet(operation).is_some()
}
/// Exact source/gray configurations and their independently observed raw
/// envelope identity. This is not a general rule for angle or sentinel values.
pub fn witnessed_source_triplet(operation: ColorRotate) -> Option<i32> {
    for (from, to, threshold, mode, hue, saturation, raw) in [
        (15f32, 120f32, 0.5f32, GrayMode::TreatAs, 45f32, 0.75f32, -1567849903),
        (17.25, 143.75, 0.3125, GrayMode::TreatAs, 61.125, 0.625, -352812463),
        (61.125, 239.5, 0.625, GrayMode::ChangeTo, 217.25, 0.3125, -352812463),
    ] {
        if operation.source == (HueArc { from_degrees: from, to_degrees: to, clockwise: false })
            && operation.gray_threshold.to_bits() == threshold.to_bits()
            && operation.gray_mode == mode
            && operation.gray_hue_degrees.to_bits() == hue.to_bits()
            && operation.gray_saturation.to_bits() == saturation.to_bits()
        {
            return Some(raw);
        }
    }
    None
}
pub(crate) fn operation(
    node: &photocraft_doc::xcf_filters::FilterNode,
) -> Result<(photocraft_algo::gimp::registry::PointMember, photocraft_algo::gimp::point::PointOperation)> {
    let parameters = typed_arguments(&node.arguments)?;
    let raw = witnessed_source_triplet(parameters).ok_or(CompositionError::Unsupported("Color Rotate unqualified source envelope"))?;
    if node.operation.name != "gegl:color-rotate"
        || node.operation.schema_epoch != COLOR_ROTATE_PERCEPTUAL_EPOCH
        || node.composition.as_ref().is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (raw, raw, raw))
    {
        return Err(CompositionError::Unsupported("Color Rotate schema/raw envelope"));
    }
    Ok(photocraft_algo::gimp::registry::color_rotate_member(Some(&node.operation.stored_version), parameters)?)
}
/// Preparation clones bounded built-in profiles and may construct a CMS transform.
/// Its allocations require caller preparation accounting; the per-pixel method
/// allocates nothing and checks a caller work ledger before conversion.
pub struct ColorRotateCandidate {
    profile: Profile,
    stored: Encoding,
    channels: usize,
    gray_output: Option<Transform>,
    operation: ColorRotate,
}
impl ColorRotateCandidate {
    pub fn new(profile: &Profile, encoding: SampleEncoding, operation: ColorRotate) -> Result<Self> {
        // Nonlinear→Perceptual conversion crosses directed hue boundaries in
        // the independent controls. Only the separately observed native
        // Perceptual storage route is exposed; no transfer epsilon is used.
        if encoding != SampleEncoding::Perceptual {
            return Err(CompositionError::Unsupported("Color Rotate unproved native transfer route"));
        }
        if profile != Builtin::Srgb.profile() && profile != Builtin::SGray.profile() {
            return Err(CompositionError::Unsupported("Color Rotate unproved current profile"));
        }
        operation.validate()?;
        let stored = crate::gimp_composite::encoding(encoding);
        let gray_output = if profile.channels() == 1 {
            Some(Transform::new(
                &effective_profile_from_definition(Builtin::Srgb.profile(), Encoding::Perceptual)?,
                &effective_profile_from_definition(profile, stored)?,
                Intent::RelativeColorimetric,
                false,
            )?)
        } else {
            None
        };
        Ok(Self { profile: profile.clone(), stored, channels: profile.channels(), gray_output, operation })
    }
    fn validate(&self, pixel: &[f64]) -> Result<()> {
        if pixel.len() != self.channels.saturating_add(1)
            || pixel.iter().any(|v| !v.is_finite())
            || !pixel.get(self.channels).is_some_and(|v| (0.0..=1.0).contains(v))
        {
            return Err(CompositionError::Unsupported("Color Rotate finite native input/unit alpha"));
        }
        Ok(())
    }
    pub fn working_input(&self, pixel: &[f64]) -> Result<[f64; 4]> {
        self.validate(pixel)?;
        let mut rgb = [0.; 3];
        convert_encoding_f64(
            &self.profile,
            self.stored,
            Encoding::Perceptual,
            pixel.get(..self.channels).ok_or(CompositionError::Invalid("Color Rotate input channels"))?,
            rgb.get_mut(..self.channels).ok_or(CompositionError::Invalid("Color Rotate working channels"))?,
        )?;
        if self.channels == 1 {
            rgb = [rgb[0]; 3];
        }
        if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(&(*v as f32))) {
            return Err(CompositionError::Unsupported("Color Rotate perceptual Float unit working input"));
        }
        Ok([
            f64::from(rgb[0] as f32),
            f64::from(rgb[1] as f32),
            f64::from(rgb[2] as f32),
            f64::from(*pixel.get(self.channels).ok_or(CompositionError::Invalid("Color Rotate alpha"))? as f32),
        ])
    }
    pub fn apply_with(
        &self,
        pixel: &[f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate(pixel)?;
        work(if enabled { 16 } else { 1 })?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if !enabled {
            let alpha = *pixel.get(self.channels).ok_or(CompositionError::Invalid("Color Rotate alpha"))?;
            return if self.channels == 1 {
                let gray = *pixel.first().ok_or(CompositionError::Invalid("Color Rotate gray"))?;
                Ok([gray, gray, gray, alpha])
            } else {
                pixel.try_into().map_err(|_| CompositionError::Invalid("Color Rotate RGBA"))
            };
        }
        let input = self.working_input(pixel)?.map(|v| v as f32);
        let mut output = [0.; 4];
        self.operation.try_eval_perceptual_float(&input, &mut output)?;
        let output = output.map(f64::from);
        let mut native = [0.; 3];
        if let Some(transform) = &self.gray_output {
            transform.try_eval_f64(&output[..3], native.get_mut(..1).ok_or(CompositionError::Invalid("Color Rotate gray output"))?)?;
            native = [native[0]; 3];
        } else {
            convert_encoding_f64(&self.profile, Encoding::Perceptual, self.stored, &output[..3], &mut native)?;
        }
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if native.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Unsupported("Color Rotate nonfinite native output"));
        }
        Ok([native[0], native[1], native[2], output[3]])
    }
}
