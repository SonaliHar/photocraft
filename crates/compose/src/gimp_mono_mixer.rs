//! Private experimental explicit-profile Mono Mixer consumer; no import admission.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_mono_mixer::{MonoMixer, PreparedMonoMixer};
use photocraft_cms::{
    Builtin, Profile,
    xcf_color::{Encoding, convert_encoding_f64},
};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::{Interrupt, RasterInputError};

/// Public Double parameters are retained; stored Float32 arguments are not
/// substituted for a caller's independently supplied Double configuration.
pub fn typed_arguments(arguments: &[FilterArgument]) -> Result<MonoMixer> {
    if arguments.len() != 4 {
        return Err(CompositionError::Unsupported("Mono Mixer argument count"));
    }
    let find = |name: &str| {
        let mut found = arguments.iter().filter(|argument| argument.name == name);
        let first = found.next().ok_or(CompositionError::Unsupported("Mono Mixer missing argument"))?;
        if found.next().is_some() {
            return Err(CompositionError::Unsupported("Mono Mixer duplicate argument"));
        }
        Ok(&first.value)
    };
    let number = |name| match find(name)? {
        FilterValue::Number(ExactNumber::Float32Bits(bits)) => Ok(f64::from(f32::from_bits(*bits))),
        FilterValue::Number(ExactNumber::Float64Bits(bits)) => Ok(f64::from_bits(*bits)),
        _ => Err(CompositionError::Unsupported("Mono Mixer floating parameter type")),
    };
    let FilterValue::Bool(preserve_luminosity) = find("preserve-luminosity")? else {
        return Err(CompositionError::Unsupported("Mono Mixer preservation Bool type"));
    };
    let operation = MonoMixer { red: number("red")?, green: number("green")?, blue: number("blue")?, preserve_luminosity: *preserve_luminosity };
    operation.validate()?;
    Ok(operation)
}

pub struct MonoMixerCandidate {
    profile: Profile,
    stored: Encoding,
    channels: usize,
    operation: PreparedMonoMixer,
}
impl MonoMixerCandidate {
    /// Experimental conversion only, not filter/source admission or allocation
    /// accounting. Caller supplies active interpretation, never source provenance.
    pub fn new(profile: &Profile, encoding: SampleEncoding, operation: MonoMixer) -> Result<Self> {
        if profile != Builtin::Srgb.profile() && profile != Builtin::SGray.profile() {
            return Err(CompositionError::Unsupported("Mono Mixer unproved current profile"));
        }
        Ok(Self { profile: profile.clone(), stored: crate::gimp_composite::encoding(encoding), channels: profile.channels(), operation: operation.prepare()? })
    }
    fn validate(&self, pixel: &[f64]) -> Result<()> {
        if pixel.len() != self.channels.saturating_add(1)
            || pixel.iter().any(|value| !value.is_finite())
            || !pixel.get(self.channels).is_some_and(|alpha| (0.0..=1.0).contains(alpha))
        {
            return Err(CompositionError::Unsupported("Mono Mixer finite native input/unit alpha"));
        }
        Ok(())
    }
    pub fn working_input(&self, pixel: &[f64]) -> Result<[f64; 4]> {
        self.validate(pixel)?;
        let mut colors = [0.; 3];
        convert_encoding_f64(
            &self.profile,
            self.stored,
            Encoding::Linear,
            pixel.get(..self.channels).ok_or(CompositionError::Invalid("Mono Mixer input channels"))?,
            colors.get_mut(..self.channels).ok_or(CompositionError::Invalid("Mono Mixer working channels"))?,
        )?;
        if self.channels == 1 {
            colors = [colors[0]; 3];
        }
        if colors.iter().any(|value| !(*value as f32).is_finite()) {
            return Err(CompositionError::Unsupported("Mono Mixer nonfinite Float working input"));
        }
        Ok([
            f64::from(colors[0] as f32),
            f64::from(colors[1] as f32),
            f64::from(colors[2] as f32),
            f64::from(*pixel.get(self.channels).ok_or(CompositionError::Invalid("Mono Mixer alpha"))? as f32),
        ])
    }
    /// Returns unquantized native-encoding RGBA (neutral RGB for Gray). Native
    /// storage quantization and mutable Half interpretation remain caller-owned.
    pub fn apply_with(
        &self,
        pixel: &[f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate(pixel)?;
        if !enabled {
            work(1)?;
            interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
            let alpha = *pixel.get(self.channels).ok_or(CompositionError::Invalid("Mono Mixer alpha"))?;
            return if self.channels == 1 {
                let gray = *pixel.first().ok_or(CompositionError::Invalid("Mono Mixer gray"))?;
                Ok([gray, gray, gray, alpha])
            } else {
                pixel.try_into().map_err(|_| CompositionError::Invalid("Mono Mixer RGBA"))
            };
        }
        work(12)?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let input = self.working_input(pixel)?;
        let mut working = [0.; 4];
        self.operation.try_eval_pixel_with(&input, &mut working, true, interrupt, work)?;
        let mut native = [0.; 3];
        convert_encoding_f64(
            &self.profile,
            Encoding::Linear,
            self.stored,
            working.get(..self.channels).ok_or(CompositionError::Invalid("Mono Mixer working output"))?,
            native.get_mut(..self.channels).ok_or(CompositionError::Invalid("Mono Mixer native output"))?,
        )?;
        if self.channels == 1 {
            native = [native[0]; 3];
        }
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok([native[0], native[1], native[2], working[3]])
    }
}

pub(crate) fn operation(
    node: &photocraft_doc::xcf_filters::FilterNode,
) -> Result<(photocraft_algo::gimp::registry::PointMember, photocraft_algo::gimp::point::PointOperation)> {
    if node.operation.name != "gegl:mono-mixer"
        || node.operation.schema_epoch != crate::gimp_filters::MONO_MIXER_SIGNED_PRECISION_EPOCH
        || node.composition.as_ref().is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (1799848529, 1799848529, 1799848529))
    {
        return Err(CompositionError::Unsupported("expanded Mono Mixer schema/raw envelope"));
    }
    Ok(photocraft_algo::gimp::registry::mono_mixer_signed_member(Some(&node.operation.stored_version), typed_arguments(&node.arguments)?)?)
}
