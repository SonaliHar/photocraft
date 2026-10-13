//! Explicit-profile HSV/HSL conversion candidate; not source/import admission.
//! Component behavior: https://gegl.org/operations/gegl-component-extract.html
//! HSL coordinates: https://www.w3.org/TR/css-color-4/#hsl-color
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_component_polar::{extract_model, hsv_hsl};
use photocraft_cms::{
    Builtin, ColorSpace, Profile,
    xcf_color::{Encoding, convert_encoding_f64},
};
use photocraft_doc::SampleEncoding;

pub struct HsvHslCandidate {
    profile: Profile,
    stored: Encoding,
    channels: usize,
    component: u8,
    invert: bool,
    linear: bool,
}
impl HsvHslCandidate {
    /// Caller accounts bounded preparation and supplies current interpretation.
    /// Exact built-in RGB/Gray definitions only; no source/profile admission.
    pub fn new(profile: &Profile, stored: SampleEncoding, component: u8, invert: bool, linear: bool) -> Result<Self> {
        if !matches!(component, 3..=7) || !matches!(profile.color_space, ColorSpace::Rgb | ColorSpace::Gray) {
            return Err(CompositionError::Unsupported("HSV/HSL component/profile model"));
        }
        let builtin = if profile.channels() == 1 { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
        if profile != builtin {
            return Err(CompositionError::Unsupported("HSV/HSL unproved current profile definition"));
        }
        Ok(Self { profile: profile.clone(), stored: crate::gimp_composite::encoding(stored), channels: profile.channels(), component, invert, linear })
    }
    /// Returns generated current-encoding samples, before native storage rounding.
    /// Disabled state returns original f64 samples without a working conversion.
    pub fn apply(&self, pixel: &[f64], enabled: bool) -> Result<[f64; 4]> {
        if pixel.len() != self.channels + 1 || pixel.iter().any(|v| !v.is_finite()) || pixel.get(self.channels).is_none_or(|v| !(0.0..=1.0).contains(v)) {
            return Err(CompositionError::Unsupported("HSV/HSL finite native colors/unit coverage"));
        }
        let alpha = *pixel.get(self.channels).ok_or(CompositionError::Invalid("HSV/HSL alpha"))?;
        if !enabled {
            return if self.channels == 1 {
                let gray = *pixel.first().ok_or(CompositionError::Invalid("HSV/HSL Gray source"))?;
                Ok([gray, gray, gray, alpha])
            } else {
                pixel.try_into().map_err(|_| CompositionError::Invalid("HSV/HSL RGB source"))
            };
        }
        let mut nonlinear = [0.; 3];
        convert_encoding_f64(
            &self.profile,
            self.stored,
            Encoding::NonlinearProfile,
            pixel.get(..self.channels).ok_or(CompositionError::Invalid("HSV/HSL source channels"))?,
            nonlinear.get_mut(..self.channels).ok_or(CompositionError::Invalid("HSV/HSL working channels"))?,
        )?;
        if self.channels == 1 {
            nonlinear = [nonlinear[0]; 3];
        }
        // The model conversion's requested Float boundary is explicit. A finite
        // native excursion that rounds to one remains one; no fitted epsilon or
        // near-neutral hue suppression is used. Wider HDR is not admitted here.
        let nonlinear = nonlinear.map(|v| f64::from(v as f32));
        let (hsv, hsl) = hsv_hsl(nonlinear)?;
        let result = extract_model(self.component, if self.component <= 5 { hsv } else { hsl }, self.invert, self.linear)?;
        let mut native = [0.; 3];
        convert_encoding_f64(
            &self.profile,
            Encoding::NonlinearProfile,
            self.stored,
            result.get(..self.channels).ok_or(CompositionError::Invalid("HSV/HSL generated channels"))?,
            native.get_mut(..self.channels).ok_or(CompositionError::Invalid("HSV/HSL output channels"))?,
        )?;
        if native.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Invalid("HSV/HSL generated nonfinite output"));
        }
        if self.channels == 1 {
            native = [native[0]; 3];
        }
        Ok([native[0], native[1], native[2], 1.])
    }
}
