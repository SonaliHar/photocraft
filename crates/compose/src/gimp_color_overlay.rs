//! Experimental explicit-profile Color Overlay conversion, not import admission.
//! The caller supplies current source interpretation. Stored/exported ICC bytes
//! are not treated as authority for an unobserved native Babl space.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_color_overlay::ColorOverlay;
use photocraft_cms::{
    Builtin, ColorSpace, Intent, Profile, Transform,
    xcf_color::{Encoding, convert_encoding_f64, effective_profile_from_definition},
};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};

/// Bounded observed argument shape only; operation/version/envelope and source
/// admission are deliberately separate. Unknown configurations/resources fail.
pub fn literal_arguments(arguments: &[FilterArgument]) -> Result<([f32; 4], bool)> {
    if arguments.len() != 2 {
        return Err(CompositionError::Unsupported("Color Overlay argument count"));
    }
    let find = |name: &str| {
        let mut values = arguments.iter().filter(|argument| argument.name == name);
        let first = values.next().ok_or(CompositionError::Unsupported("Color Overlay missing argument"))?;
        if values.next().is_some() {
            return Err(CompositionError::Unsupported("Color Overlay duplicate argument"));
        }
        Ok(&first.value)
    };
    let FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None } = find("value")? else {
        return Err(CompositionError::Unsupported("Color Overlay literal encoding/profile"));
    };
    if components.len() != 4 {
        return Err(CompositionError::Unsupported("Color Overlay color component count"));
    }
    let mut color = [0.; 4];
    for (destination, component) in color.iter_mut().zip(components) {
        let ExactNumber::Float32Bits(bits) = component else {
            return Err(CompositionError::Unsupported("Color Overlay color component type"));
        };
        *destination = f32::from_bits(*bits);
        if !destination.is_finite() || !(0.0..=1.0).contains(destination) {
            return Err(CompositionError::Unsupported("Color Overlay literal unit color"));
        }
    }
    let FilterValue::Bool(srgb) = find("srgb")? else {
        return Err(CompositionError::Unsupported("Color Overlay srgb type"));
    };
    Ok((color, *srgb))
}

pub struct ColorOverlayCandidate {
    profile: Profile,
    stored: Encoding,
    working: Encoding,
    output: Transform,
    operation: ColorOverlay,
    channels: usize,
}

impl ColorOverlayCandidate {
    /// Candidate math only. The caller must account preparation allocations and
    /// validate source identity separately; this does not admit a saved filter.
    pub fn new(profile: &Profile, encoding: SampleEncoding, color: [f32; 4], srgb: bool) -> Result<Self> {
        if !matches!(profile.color_space, ColorSpace::Rgb | ColorSpace::Gray) {
            return Err(CompositionError::Unsupported("Color Overlay source model"));
        }
        let builtin = if profile.channels() == 1 { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        if profile != builtin {
            return Err(CompositionError::Unsupported("Color Overlay unproved current profile definition"));
        }
        if color.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(CompositionError::Unsupported("Color Overlay literal unit color"));
        }
        let source = effective_profile_from_definition(profile, crate::gimp_composite::encoding(encoding))?;
        let working = if srgb { Builtin::Srgb.profile() } else { Builtin::LinearSrgb.profile() };
        let working_color = prepare_working_color(color, srgb)?;
        Ok(Self {
            profile: profile.clone(),
            stored: crate::gimp_composite::encoding(encoding),
            working: if srgb { Encoding::NonlinearProfile } else { Encoding::Linear },
            output: Transform::new(working, &source, Intent::RelativeColorimetric, false)?,
            operation: ColorOverlay { color: working_color },
            channels: profile.channels(),
        })
    }
    pub fn apply(&self, pixel: &[f64], enabled: bool) -> Result<[f64; 4]> {
        self.validate(pixel)?;
        let alpha = *pixel.get(self.channels).ok_or(CompositionError::Invalid("Color Overlay alpha"))?;
        if !enabled {
            return if self.channels == 1 {
                let gray = *pixel.first().ok_or(CompositionError::Invalid("Color Overlay gray"))?;
                Ok([gray, gray, gray, alpha])
            } else {
                pixel.try_into().map_err(|_| CompositionError::Invalid("Color Overlay RGB"))
            };
        }
        let input = self.working_input(pixel)?;
        let mut result = [0.; 4];
        self.operation.try_eval_pixel(&input, 3, &mut result)?;
        let mut native = [0.; 3];
        self.output.try_eval_f64(&result[..3], native.get_mut(..self.channels).ok_or(CompositionError::Invalid("Color Overlay output channels"))?)?;
        if native.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Invalid("Color Overlay nonfinite output"));
        }
        if self.channels == 1 {
            native = [native[0]; 3];
        }
        Ok([native[0], native[1], native[2], result[3]])
    }
    fn validate(&self, pixel: &[f64]) -> Result<()> {
        // Requested unit RGB/Gray can acquire tiny finite matrix excursions in
        // actual native Float/Double storage. Never clamp or fit an epsilon.
        // This standalone candidate evaluates finite colors; source HDR
        // admission remains separate. Coverage must still be in the unit range.
        if pixel.len() != self.channels.saturating_add(1)
            || pixel.iter().any(|v| !v.is_finite() || !(*v as f32).is_finite())
            || !pixel.get(self.channels).is_some_and(|alpha| (0.0..=1.0).contains(alpha))
        {
            return Err(CompositionError::Unsupported("Color Overlay finite native input/unit alpha"));
        }
        Ok(())
    }
    pub fn working_input(&self, pixel: &[f64]) -> Result<[f64; 4]> {
        self.validate(pixel)?;
        let mut rgb = [0.; 3];
        convert_encoding_f64(
            &self.profile,
            self.stored,
            self.working,
            pixel.get(..self.channels).ok_or(CompositionError::Invalid("Color Overlay source channels"))?,
            rgb.get_mut(..self.channels).ok_or(CompositionError::Invalid("Color Overlay source channels"))?,
        )?;
        // Exact sGray definition uses the same transfer as sRGB and represents
        // neutral RGB. Expansion does not require a quantized PCS matrix round
        // trip. Custom/assigned definitions remain rejected above.
        if self.channels == 1 {
            rgb = [rgb[0]; 3];
        }
        let alpha = *pixel.get(self.channels).ok_or(CompositionError::Invalid("Color Overlay alpha"))?;
        if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(&(*v as f32))) {
            return Err(CompositionError::Invalid("Color Overlay working input"));
        }
        Ok([f64::from(rgb[0] as f32), f64::from(rgb[1] as f32), f64::from(rgb[2] as f32), f64::from(alpha as f32)])
    }
}

/// Convert a retained nonlinear literal to the operation's explicit Float space.
pub fn prepare_working_color(color: [f32; 4], srgb: bool) -> Result<[f32; 4]> {
    if color.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(CompositionError::Unsupported("Color Overlay literal unit color"));
    }
    let target = if srgb { Builtin::Srgb.profile() } else { Builtin::LinearSrgb.profile() };
    let transform = Transform::new(Builtin::Srgb.profile(), target, Intent::RelativeColorimetric, false)?;
    let mut rgb = [0.; 3];
    transform.try_eval_f64(&[f64::from(color[0]), f64::from(color[1]), f64::from(color[2])], &mut rgb)?;
    let mut result = color;
    for (out, value) in result.iter_mut().take(3).zip(rgb) {
        if !value.is_finite() || !(0.0..=1.0).contains(&(value as f32)) {
            return Err(CompositionError::Unsupported("Color Overlay working unit color"));
        }
        *out = value as f32;
    }
    Ok(result)
}

pub(crate) fn operation(
    node: &photocraft_doc::xcf_filters::FilterNode,
) -> Result<(photocraft_algo::gimp::registry::PointMember, photocraft_algo::gimp::point::PointOperation)> {
    if node.operation.name != "gegl:color-overlay"
        || node.operation.schema_epoch != crate::gimp_filters::COLOR_OVERLAY_PRECISION_EPOCH
        || node.composition.as_ref().is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (1435337297, 1435337297, 1435337297))
    {
        return Err(CompositionError::Unsupported("Color Overlay schema/raw field compatibility"));
    }
    let (color, srgb) = literal_arguments(&node.arguments)?;
    Ok(photocraft_algo::gimp::registry::color_overlay_member(Some(&node.operation.stored_version), prepare_working_color(color, srgb)?, srgb)?)
}
