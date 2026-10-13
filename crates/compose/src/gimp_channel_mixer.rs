//! Experimental explicit-profile Channel Mixer consumer; saved-source admission is separate.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_channel_mixer::ChannelMixer;
use photocraft_cms::{
    Builtin, Intent, Profile, Transform,
    xcf_color::{Encoding, convert_encoding_f64, effective_profile_from_definition},
};
use photocraft_color::SampleType;
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::{Interrupt, RasterInputError};

/// Preserve original Float32 versus current Double authority. This parser never
/// rewrites stored values, invents a numeric alias, or normalizes a gain tuple.
pub fn typed_arguments(arguments: &[FilterArgument]) -> Result<ChannelMixer> {
    if arguments.len() != 10 {
        return Err(CompositionError::Unsupported("Channel Mixer argument count"));
    }
    let find = |name: &str| {
        let mut matches = arguments.iter().filter(|a| a.name == name);
        let value = matches.next().ok_or(CompositionError::Unsupported("Channel Mixer missing argument"))?;
        if matches.next().is_some() {
            return Err(CompositionError::Unsupported("Channel Mixer duplicate argument"));
        }
        Ok(&value.value)
    };
    let mut gains = [[0.0; 3]; 3];
    for (row, names) in gains.iter_mut().zip([["rr-gain", "rg-gain", "rb-gain"], ["gr-gain", "gg-gain", "gb-gain"], ["br-gain", "bg-gain", "bb-gain"]]) {
        for (gain, name) in row.iter_mut().zip(names) {
            *gain = match find(name)? {
                FilterValue::Number(ExactNumber::Float32Bits(bits)) => f64::from(f32::from_bits(*bits)),
                FilterValue::Number(ExactNumber::Float64Bits(bits)) => f64::from_bits(*bits),
                _ => return Err(CompositionError::Unsupported("Channel Mixer floating gain type")),
            };
        }
    }
    let FilterValue::Bool(preserve_luminosity) = find("preserve-luminosity")? else {
        return Err(CompositionError::Unsupported("Channel Mixer preservation Bool type"));
    };
    let operation = ChannelMixer { gains, preserve_luminosity: *preserve_luminosity };
    operation.try_eval_linear_float(&[0.0; 4], &mut [0.0; 4])?;
    Ok(operation)
}

pub struct ChannelMixerCandidate {
    profile: Profile,
    stored: Encoding,
    channels: usize,
    output: Transform,
    operation: ChannelMixer,
}

impl ChannelMixerCandidate {
    /// Caller supplies the active analytic profile and owns preparation allocation
    /// accounting. Arbitrary assigned profiles and source-derived guesses are closed.
    pub fn new(profile: &Profile, encoding: SampleEncoding, operation: ChannelMixer) -> Result<Self> {
        if profile != Builtin::Srgb.profile() && profile != Builtin::SGray.profile() {
            return Err(CompositionError::Unsupported("Channel Mixer unproved active profile"));
        }
        operation.try_eval_linear_float(&[0.0; 4], &mut [0.0; 4])?;
        let stored = crate::gimp_composite::encoding(encoding);
        let output = Transform::new(Builtin::LinearSrgb.profile(), &effective_profile_from_definition(profile, stored)?, Intent::RelativeColorimetric, false)?;
        Ok(Self { profile: profile.clone(), stored, channels: profile.channels(), output, operation })
    }

    fn validate(&self, pixel: &[f64]) -> Result<()> {
        if pixel.len() != self.channels.saturating_add(1)
            || pixel.iter().any(|v| !v.is_finite())
            || pixel.get(self.channels).is_none_or(|a| !(0.0..=1.0).contains(a))
        {
            return Err(CompositionError::Unsupported("Channel Mixer finite native/unit alpha"));
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
            pixel.get(..self.channels).ok_or(CompositionError::Invalid("Channel Mixer native channels"))?,
            colors.get_mut(..self.channels).ok_or(CompositionError::Invalid("Channel Mixer working channels"))?,
        )?;
        if self.channels == 1 {
            colors = [colors[0]; 3];
        }
        let rgba =
            [colors[0] as f32, colors[1] as f32, colors[2] as f32, *pixel.get(self.channels).ok_or(CompositionError::Invalid("Channel Mixer alpha"))? as f32];
        if rgba.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Unsupported("Channel Mixer nonfinite Float working input"));
        }
        Ok(rgba)
    }

    /// Unquantized native-encoding output. Generated Half/current integer storage
    /// and composition remain explicit caller obligations; disabled bits are exact.
    pub fn apply_with(
        &self,
        pixel: &[f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        self.apply_impl(pixel, enabled, None, interrupt, work)
    }

    /// Independently observed live Gray U32 Linear/Nonlinear buffers clip
    /// unsigned RGB before the Gray conversion; Perceptual and merge-baked Gray
    /// use the ordinary conversion. This takes mutable current storage explicitly,
    /// never the original source precision. Native output quantization is caller-owned.
    pub fn apply_with_current_storage(
        &self,
        pixel: &[f64],
        enabled: bool,
        current_sample: SampleType,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        self.apply_impl(pixel, enabled, Some(current_sample), interrupt, work)
    }

    fn apply_impl(
        &self,
        pixel: &[f64],
        enabled: bool,
        current_sample: Option<SampleType>,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate(pixel)?;
        work(if enabled { 32 } else { 1 })?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if !enabled {
            return if self.channels == 1 {
                let gray = *pixel.first().ok_or(CompositionError::Invalid("Channel Mixer disabled Gray"))?;
                let alpha = *pixel.get(1).ok_or(CompositionError::Invalid("Channel Mixer disabled alpha"))?;
                Ok([gray, gray, gray, alpha])
            } else {
                pixel.try_into().map_err(|_| CompositionError::Invalid("Channel Mixer disabled RGBA"))
            };
        }
        let mut working = [0.0; 4];
        self.operation.try_eval_linear_float(&self.working_input(pixel)?, &mut working)?;
        if self.channels == 1 && current_sample == Some(SampleType::U32) && self.stored != Encoding::Perceptual {
            for value in working.iter_mut().take(3) {
                *value = value.clamp(0.0, 1.0);
            }
        }
        let rgb = working.map(f64::from);
        let mut native = [0.0; 3];
        self.output.try_eval_f64(&rgb[..3], native.get_mut(..self.channels).ok_or(CompositionError::Invalid("Channel Mixer native output"))?)?;
        if native.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Unsupported("Channel Mixer nonfinite native output"));
        }
        if self.channels == 1 {
            native = [native[0]; 3];
        }
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok([native[0], native[1], native[2], rgb[3]])
    }
}

/// Typed operation identity remains distinct from measured Replace compatibility.
pub(crate) fn operation(
    node: &photocraft_doc::xcf_filters::FilterNode,
) -> Result<(photocraft_algo::gimp::registry::PointMember, photocraft_algo::gimp::point::PointOperation)> {
    if node.operation.name != "gegl:channel-mixer"
        || node.operation.schema_epoch != crate::gimp_filters::CHANNEL_MIXER_PRECISION_EPOCH
        || node
            .composition
            .as_ref()
            .is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (-1150320047, -1150320047, -1150320047))
    {
        return Err(CompositionError::Unsupported("Channel Mixer exact schema/raw envelope"));
    }
    Ok(photocraft_algo::gimp::registry::channel_mixer_member(Some(&node.operation.stored_version), typed_arguments(&node.arguments)?)?)
}
