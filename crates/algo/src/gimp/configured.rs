//! Typed configured Curve/Levels candidates. No serialized epoch, ICC alias or
//! source rendering is admitted here. Callers retain exact original config and
//! supply already converted working samples; this module performs no CMS.
use super::{
    curves::{FreeCurve, OwnedPreparedCurveTables},
    levels::{LevelsChannel, LevelsConfiguration},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Linear,
    NonlinearProfile,
    Perceptual,
}
impl Transfer {
    pub fn from_stored(value: i32) -> Result<Self, RasterInputError> {
        match value {
            0 => Ok(Self::Linear),
            1 => Ok(Self::NonlinearProfile),
            2 => Ok(Self::Perceptual),
            _ => Err(RasterInputError::UnsupportedConversion),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub transfer: Transfer,
    /// Exact caller-established working ICC identity; no default profile assumed.
    pub profile_identity: [u8; 32],
    pub color_channels: usize,
    pub alpha: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn admit(bytes: u64, work: u64, limits: Limits) -> Result<(), RasterInputError> {
    if limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || bytes > limits.resident_bytes || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}
fn context(actual: WorkingContext, expected: WorkingContext, transfer: Transfer) -> Result<(), RasterInputError> {
    if actual != expected || actual.transfer != transfer || !matches!(actual.color_channels, 1 | 3) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveMode {
    Smooth,
    Free,
}
/// Explicit sampling choice. This is not evidence that a requested form was
/// preserved by serialization or independently render-qualified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CurveSamplePolicy {
    #[default]
    RequireExact256,
    /// Only FREE requests256/257/1024 with an actual retained256 sample table.
    /// Caller binds the external canonicalization witness; no table regeneration.
    UseRetainedCanonical256,
}
impl CurveSamplePolicy {
    pub fn permits(self, mode: CurveMode, requested: usize, stored: usize) -> bool {
        stored == 256 && (requested == 256 || (self == Self::UseRetainedCanonical256 && mode == CurveMode::Free && matches!(requested, 257 | 1024)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurvePointKind {
    Smooth,
    Corner,
}
#[derive(Clone, Copy, Debug)]
pub struct CurveChannel<'a> {
    pub mode: CurveMode,
    /// Default retained-node binding uses serialized n-samples, which the
    /// parser guarantees equals stored table length. An external authoring
    /// request257/1024 is separate; using canonical256 needs explicit opt-in.
    pub requested_samples: usize,
    pub points_f64_bits: &'a [[u64; 2]],
    pub point_types: &'a [CurvePointKind],
    pub samples_f64_bits: &'a [u64],
}
#[derive(Clone, Copy, Debug)]
pub struct CurvesConfiguration<'a> {
    pub transfer: Transfer,
    /// Fixed explicit order: value, red, green, blue, alpha.
    pub channels: [CurveChannel<'a>; 5],
}
#[derive(Clone, Copy, Debug)]
pub struct LevelsChannelBits {
    pub low_input: u64,
    pub high_input: u64,
    pub gamma: u64,
    pub low_output: u64,
    pub high_output: u64,
}
impl LevelsChannelBits {
    fn decode(self) -> LevelsChannel {
        LevelsChannel {
            low_input: f64::from_bits(self.low_input),
            high_input: f64::from_bits(self.high_input),
            gamma: f64::from_bits(self.gamma),
            low_output: f64::from_bits(self.low_output),
            high_output: f64::from_bits(self.high_output),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct LevelsConfigurationBits {
    pub transfer: Transfer,
    pub channels: [LevelsChannelBits; 5],
    pub clamp_input: bool,
    pub clamp_output: bool,
}
// Keep the bounded inline representation; boxing adds a new allocation and changes its controlled storage schedule.
#[allow(clippy::large_enum_variant)]
enum Kernel {
    Curves(OwnedPreparedCurveTables),
    Levels(LevelsConfiguration),
}
pub struct PreparedConfiguration<'a> {
    working: WorkingContext,
    original: OriginalConfiguration<'a>,
    sampling: CurveSamplePolicy,
    kernel: Kernel,
    retained_bytes: u64,
}
#[derive(Clone, Copy)]
pub enum OriginalConfiguration<'a> {
    Curves(&'a CurvesConfiguration<'a>),
    Levels(&'a LevelsConfigurationBits),
}
impl<'a> PreparedConfiguration<'a> {
    pub fn original(&self) -> OriginalConfiguration<'a> {
        self.original
    }
    pub fn sampling_policy(&self) -> CurveSamplePolicy {
        self.sampling
    }
    pub fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }
    pub fn working_context(&self) -> WorkingContext {
        self.working
    }
    /// Allocation-free, atomic pixel evaluation. Caller is responsible for native
    /// final quantization and compositor coverage rules. Unclamped Levels alpha
    /// is returned as calculated, never silently changed to unit coverage.
    pub fn try_eval_pixel(&self, input: &[f64], output: &mut [f64], actual: WorkingContext, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
        poll(control)?;
        context(actual, self.working, self.working.transfer)?;
        let channels = self.working.color_channels + usize::from(self.working.alpha);
        if input.len() != channels || output.len() != channels {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|sample| !sample.is_finite()) {
            return Err(RasterInputError::InvalidSample);
        }
        if self.working.alpha && input.get(self.working.color_channels).is_none_or(|alpha| !(0.0..=1.0).contains(alpha)) {
            return Err(RasterInputError::InvalidSample);
        }
        let mut staging = [0.0; 4];
        let dst = staging.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?;
        match &self.kernel {
            Kernel::Curves(tables) => tables.try_eval_pixel(input, self.working.color_channels, dst)?,
            Kernel::Levels(config) => config.try_eval_pixel(input, self.working.color_channels, dst)?,
        }
        poll(control)?;
        output.copy_from_slice(dst);
        Ok(())
    }
    /// Private working-sample transaction, not a native raster/profile converter.
    /// Input remains borrowed, output capacity is admitted before allocation.
    pub fn try_apply_pixels(&self, input: &[f64], actual: WorkingContext, limits: Limits, control: &Interrupt<'_>) -> Result<Vec<f64>, RasterInputError> {
        poll(control)?;
        context(actual, self.working, self.working.transfer)?;
        let channels = self.working.color_channels + usize::from(self.working.alpha);
        if !input.len().is_multiple_of(channels) {
            return Err(RasterInputError::LengthMismatch);
        }
        let bytes = u64::try_from(input.len()).ok().and_then(|n| n.checked_mul(8)).ok_or(RasterInputError::LimitExceeded)?;
        let work = u64::try_from(input.len()).ok().and_then(|n| n.checked_mul(2048)).ok_or(RasterInputError::LimitExceeded)?;
        let peak = bytes.checked_mul(2).and_then(|n| n.checked_add(self.retained_bytes)).ok_or(RasterInputError::LimitExceeded)?;
        admit(peak, work, limits)?;
        let mut output = Vec::new();
        output.try_reserve_exact(input.len()).map_err(|_| RasterInputError::LimitExceeded)?;
        let actual_output = u64::try_from(output.capacity()).ok().and_then(|n| n.checked_mul(8)).ok_or(RasterInputError::LimitExceeded)?;
        admit(actual_output.checked_add(bytes).and_then(|n| n.checked_add(self.retained_bytes)).ok_or(RasterInputError::LimitExceeded)?, work, limits)?;
        for pixel in input.chunks_exact(channels) {
            let mut staging = [0.0; 4];
            let dst = staging.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?;
            self.try_eval_pixel(pixel, dst, actual, control)?;
            output.extend_from_slice(dst);
        }
        poll(control)?;
        Ok(output)
    }
}
/// Compatibility entry: non256 requested forms remain refused by default.
pub fn prepare_curves<'a>(
    config: &'a CurvesConfiguration<'a>,
    actual: WorkingContext,
    expected: WorkingContext,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<PreparedConfiguration<'a>, RasterInputError> {
    prepare_curves_with_sample_policy(config, CurveSamplePolicy::RequireExact256, actual, expected, limits, control)
}
pub fn prepare_curves_with_sample_policy<'a>(
    config: &'a CurvesConfiguration<'a>,
    sampling: CurveSamplePolicy,
    actual: WorkingContext,
    expected: WorkingContext,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<PreparedConfiguration<'a>, RasterInputError> {
    poll(control)?;
    context(actual, expected, config.transfer)?;
    let mut retained = u64::try_from(std::mem::size_of::<PreparedConfiguration<'a>>()).map_err(|_| RasterInputError::LimitExceeded)?;
    for channel in &config.channels {
        poll(control)?;
        if !sampling.permits(channel.mode, channel.requested_samples, channel.samples_f64_bits.len())
            || channel.points_f64_bits.len() > 64
            || channel.point_types.len() != channel.points_f64_bits.len()
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let bytes = channel
            .points_f64_bits
            .len()
            .checked_mul(16)
            .and_then(|n| n.checked_add(channel.point_types.len().checked_mul(std::mem::size_of::<CurvePointKind>())?))
            .and_then(|n| n.checked_add(channel.samples_f64_bits.len().checked_mul(8)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        retained = retained.checked_add(u64::try_from(bytes).map_err(|_| RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    }
    admit(retained.checked_add(5 * 256 * 8).ok_or(RasterInputError::LimitExceeded)?, 8192, limits)?;
    let mut tables = [[0.0; 256]; 5];
    for (channel, table) in config.channels.iter().zip(tables.iter_mut()) {
        // Geometry is retained/validated but not used to invent regenerated
        // SMOOTH samples. Stored tables remain the explicit evaluated input.
        let mut previous = None;
        for point in channel.points_f64_bits {
            poll(control)?;
            let [x, y] = point.map(f64::from_bits);
            if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) || previous.is_some_and(|previous| x <= previous) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            previous = Some(x);
        }
        for (bits, sample) in channel.samples_f64_bits.iter().zip(table.iter_mut()) {
            poll(control)?;
            *sample = f64::from_bits(*bits);
        }
        FreeCurve { samples: table }.validate()?;
    }
    poll(control)?;
    Ok(PreparedConfiguration {
        working: expected,
        original: OriginalConfiguration::Curves(config),
        sampling,
        kernel: Kernel::Curves(OwnedPreparedCurveTables::try_from_tables(tables)?),
        retained_bytes: retained,
    })
}
pub fn prepare_levels<'a>(
    config: &'a LevelsConfigurationBits,
    actual: WorkingContext,
    expected: WorkingContext,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<PreparedConfiguration<'a>, RasterInputError> {
    poll(control)?;
    context(actual, expected, config.transfer)?;
    let retained = u64::try_from(std::mem::size_of::<PreparedConfiguration<'a>>() + std::mem::size_of::<LevelsConfigurationBits>())
        .map_err(|_| RasterInputError::LimitExceeded)?;
    admit(retained, 128, limits)?;
    let mut decoded = [LevelsChannel::default(); 5];
    for (channel, output) in config.channels.iter().zip(decoded.iter_mut()) {
        poll(control)?;
        *output = channel.decode();
    }
    let kernel = LevelsConfiguration { channels: decoded, clamp_input: config.clamp_input, clamp_output: config.clamp_output };
    kernel.validate()?;
    poll(control)?;
    Ok(PreparedConfiguration {
        working: expected,
        original: OriginalConfiguration::Levels(config),
        sampling: CurveSamplePolicy::RequireExact256,
        kernel: Kernel::Levels(kernel),
        retained_bytes: retained,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn working() -> WorkingContext {
        WorkingContext { transfer: Transfer::Linear, profile_identity: [7; 32], color_channels: 3, alpha: true }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: 1024 * 1024, work_units: 1024 * 1024 }
    }
    fn levels() -> LevelsConfigurationBits {
        let channel = LevelsChannelBits {
            low_input: 0f64.to_bits(),
            high_input: 1f64.to_bits(),
            gamma: 1f64.to_bits(),
            low_output: 0f64.to_bits(),
            high_output: 1f64.to_bits(),
        };
        LevelsConfigurationBits { transfer: Transfer::Linear, channels: [channel; 5], clamp_input: false, clamp_output: false }
    }
    #[test]
    fn levels_preserves_original_bits_and_explicit_hdr_without_clamping() {
        let mut config = levels();
        config.channels[1].low_input = (-0.0f64).to_bits();
        let prepared = prepare_levels(&config, working(), working(), limits(), &Interrupt::NONE).unwrap();
        let OriginalConfiguration::Levels(retained) = prepared.original() else {
            panic!("wrong original");
        };
        assert_eq!(retained.channels[1].low_input, (-0.0f64).to_bits());
        let input = [2.0, -0.25, 0.5, 0.75];
        let mut output = [9.0; 4];
        prepared.try_eval_pixel(&input, &mut output, working(), &Interrupt::NONE).unwrap();
        assert_eq!(output, input);
    }
    #[test]
    fn invalid_pixel_context_budget_and_cancel_never_publish_partial_output() {
        let config = levels();
        let prepared = prepare_levels(&config, working(), working(), limits(), &Interrupt::NONE).unwrap();
        let mut output = [9.0; 4];
        let before = output;
        assert!(prepared.try_eval_pixel(&[0.0, f64::NAN, 0.0, 1.0], &mut output, working(), &Interrupt::NONE).is_err());
        assert_eq!(output, before);
        let mut mismatch = working();
        mismatch.profile_identity[0] = 8;
        assert!(prepared.try_eval_pixel(&[0.0; 4], &mut output, mismatch, &Interrupt::NONE).is_err());
        assert_eq!(output, before);
        assert!(prepared.try_apply_pixels(&[0.0; 4], working(), Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE).is_err());
        let cancel = || true;
        let control = Interrupt::new(&cancel, &|_| {});
        assert_eq!(prepared.try_eval_pixel(&[0.0; 4], &mut output, working(), &control), Err(RasterInputError::Cancelled));
        assert_eq!(output, before);
        assert!(prepared.try_apply_pixels(&[0.0; 4], working(), limits(), &control).is_err());
    }
    #[test]
    fn canonicalized_requested_free257_cannot_pass_as_stored256() {
        let bits: [u64; 256] = std::array::from_fn(|index| (index as f64 / 255.0).to_bits());
        let channel = CurveChannel { mode: CurveMode::Free, requested_samples: 257, points_f64_bits: &[], point_types: &[], samples_f64_bits: &bits };
        let mut config = CurvesConfiguration { transfer: Transfer::Linear, channels: [channel; 5] };
        assert!(prepare_curves(&config, working(), working(), limits(), &Interrupt::NONE).is_err());
        for channel in &mut config.channels {
            channel.requested_samples = 256;
        }
        let prepared = prepare_curves(&config, working(), working(), limits(), &Interrupt::NONE).unwrap();
        let mut output = [9.0; 4];
        prepared.try_eval_pixel(&[0.0, 1.0, 0.5, 0.25], &mut output, working(), &Interrupt::NONE).unwrap();
        assert_eq!(output[0], 0.0);
        assert_eq!(output[1], 1.0);
    }
    #[test]
    fn explicit_retained_table_opt_in_keeps_requested_metadata_without_resampling() {
        let bits: [u64; 256] = std::array::from_fn(|index| (index as f64 / 255.0).to_bits());
        for requested in [257, 1024] {
            let channel = CurveChannel { mode: CurveMode::Free, requested_samples: requested, points_f64_bits: &[], point_types: &[], samples_f64_bits: &bits };
            let config = CurvesConfiguration { transfer: Transfer::Linear, channels: [channel; 5] };
            assert!(prepare_curves(&config, working(), working(), limits(), &Interrupt::NONE).is_err());
            let prepared =
                prepare_curves_with_sample_policy(&config, CurveSamplePolicy::UseRetainedCanonical256, working(), working(), limits(), &Interrupt::NONE)
                    .unwrap();
            assert_eq!(prepared.sampling_policy(), CurveSamplePolicy::UseRetainedCanonical256);
            let OriginalConfiguration::Curves(original) = prepared.original() else {
                panic!("wrong config");
            };
            assert_eq!(original.channels[0].requested_samples, requested);
            assert_eq!(original.channels[0].samples_f64_bits.as_ptr(), bits.as_ptr());
            let mut output = [9.0; 4];
            prepared.try_eval_pixel(&[0.0, 1.0, 0.5, 0.25], &mut output, working(), &Interrupt::NONE).unwrap();
            assert_eq!((output[0], output[1]), (0.0, 1.0));
            let smooth = CurveChannel { mode: CurveMode::Smooth, ..channel };
            let smooth_config = CurvesConfiguration { transfer: Transfer::Linear, channels: [smooth; 5] };
            assert!(
                prepare_curves_with_sample_policy(&smooth_config, CurveSamplePolicy::UseRetainedCanonical256, working(), working(), limits(), &Interrupt::NONE)
                    .is_err()
            );
        }
    }
}
