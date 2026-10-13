//! Private explicit-profile Absolute consumer; source/current admission is separate.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_absolute::Absolute;
use photocraft_cms::{
    Builtin, Intent, Profile, Transform,
    xcf_color::{Encoding, convert_encoding_f64, effective_profile_from_definition},
};
use photocraft_doc::{SampleEncoding, xcf_filters::FilterArgument};
use photocraft_raster::{Interrupt, RasterInputError};

pub fn typed_arguments(arguments: &[FilterArgument]) -> Result<Absolute> {
    if !arguments.is_empty() {
        return Err(CompositionError::Unsupported("Absolute has no stored arguments"));
    }
    Ok(Absolute)
}

pub struct AbsoluteCandidate {
    profile: Profile,
    stored: Encoding,
    output: Transform,
    channels: usize,
}

impl AbsoluteCandidate {
    /// Active analytic profile only. Preparation allocation accounting is caller-owned.
    pub fn new(profile: &Profile, encoding: SampleEncoding) -> Result<Self> {
        if profile != Builtin::Srgb.profile() && profile != Builtin::SGray.profile() {
            return Err(CompositionError::Unsupported("Absolute unproved active profile"));
        }
        let stored = crate::gimp_composite::encoding(encoding);
        let output = Transform::new(Builtin::LinearSrgb.profile(), &effective_profile_from_definition(profile, stored)?, Intent::RelativeColorimetric, false)?;
        Ok(Self { profile: profile.clone(), stored, output, channels: profile.channels() })
    }

    fn validate(&self, pixel: &[f64]) -> Result<()> {
        if pixel.len() != self.channels.saturating_add(1)
            || pixel.iter().any(|v| !v.is_finite())
            || pixel.get(self.channels).is_none_or(|a| !(0.0..=1.0).contains(a))
        {
            return Err(CompositionError::Unsupported("Absolute finite native colors/unit alpha"));
        }
        Ok(())
    }

    pub fn working_input(&self, pixel: &[f64]) -> Result<[f32; 4]> {
        self.validate(pixel)?;
        let mut colors = [0.0; 3];
        convert_encoding_f64(
            &self.profile,
            self.stored,
            Encoding::Linear,
            pixel.get(..self.channels).ok_or(CompositionError::Invalid("Absolute native channels"))?,
            colors.get_mut(..self.channels).ok_or(CompositionError::Invalid("Absolute working channels"))?,
        )?;
        if self.channels == 1 {
            colors = [colors[0]; 3];
        }
        let rgba = [colors[0] as f32, colors[1] as f32, colors[2] as f32, *pixel.get(self.channels).ok_or(CompositionError::Invalid("Absolute alpha"))? as f32];
        if rgba.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Unsupported("Absolute nonfinite Float working input"));
        }
        Ok(rgba)
    }

    /// Generated current Half/integer quantization and composition remain explicit
    /// caller obligations. Disabled samples are returned without conversion.
    pub fn apply_with(
        &self,
        pixel: &[f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate(pixel)?;
        work(if enabled { 32 } else { 1 })?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if !enabled {
            return if self.channels == 1 {
                let gray = *pixel.first().ok_or(CompositionError::Invalid("Absolute disabled Gray"))?;
                Ok([gray, gray, gray, *pixel.get(1).ok_or(CompositionError::Invalid("Absolute disabled alpha"))?])
            } else {
                pixel.try_into().map_err(|_| CompositionError::Invalid("Absolute disabled RGBA"))
            };
        }
        let mut working = [0.0; 4];
        Absolute.try_eval_linear_float(&self.working_input(pixel)?, &mut working)?;
        let rgba = working.map(f64::from);
        let mut native = [0.0; 3];
        self.output.try_eval_f64(&rgba[..3], native.get_mut(..self.channels).ok_or(CompositionError::Invalid("Absolute native output"))?)?;
        if native.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Unsupported("Absolute nonfinite native output"));
        }
        if self.channels == 1 {
            native = [native[0]; 3];
        }
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok([native[0], native[1], native[2], rgba[3]])
    }
}
