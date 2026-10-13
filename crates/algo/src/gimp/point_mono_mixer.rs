//! Standalone Mono Mixer foundation. Public parameters retain their Double values.
//! Caller owns the qualified linear-RGB working conversion and native quantization.
//! This module does not broaden registry, importer, profile, or document admission.
use photocraft_raster::{Interrupt, RasterInputError};

pub const VERSION: &str = "0:0";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonoMixer {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub preserve_luminosity: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PreparedMonoMixer {
    weights: [f32; 3],
    normalization: f32,
}

impl MonoMixer {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if [self.red, self.green, self.blue].iter().any(|v| !v.is_finite() || !(-5.0..=5.0).contains(v)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }

    /// The public properties remain f64. Independent Double-cancellation controls
    /// distinguish the operation's Float32 coefficient/sum boundary from normalizing
    /// the original Double parameters. A zero Float32 sum leaves the dot unscaled;
    /// a negative sum uses its magnitude, retaining the dot's sign.
    pub fn prepare(self) -> Result<PreparedMonoMixer, RasterInputError> {
        self.validate()?;
        let weights = [self.red as f32, self.green as f32, self.blue as f32];
        let sum = (weights[0] + weights[1]) + weights[2];
        let normalization = if self.preserve_luminosity && sum != 0.0 { 1.0_f32 / sum.abs() } else { 1.0 };
        Ok(PreparedMonoMixer { weights, normalization })
    }

    /// Unassociated linear RGBA. Enabled output uses the independently observed
    /// Float working alpha boundary; disabled pixels preserve original alpha bits.
    /// Output is staged
    /// so cancellation, budget denial, invalid parameters or nonfinite results do
    /// not partially alter the caller's output. Disabled finite pixels remain exact.
    pub fn try_eval_pixel_with(
        self,
        input: &[f64],
        output: &mut [f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), RasterInputError>,
    ) -> Result<(), RasterInputError> {
        self.prepare()?.try_eval_pixel_with(input, output, enabled, interrupt, work)
    }
}

impl PreparedMonoMixer {
    /// Charges twelve scalar sample units before staging. The caller owns transfer,
    /// allocation, alpha admission and native storage. Active nonfinite Float32
    /// results are errors; they are never clamped or replaced by a fallback.
    pub fn try_eval_pixel_with(
        self,
        input: &[f64],
        output: &mut [f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), RasterInputError>,
    ) -> Result<(), RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|v| !v.is_finite()) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        work(12)?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let mut staged = [0.; 4];
        staged.copy_from_slice(input);
        if enabled {
            let rgb = [
                *input.first().ok_or(RasterInputError::LengthMismatch)? as f32,
                *input.get(1).ok_or(RasterInputError::LengthMismatch)? as f32,
                *input.get(2).ok_or(RasterInputError::LengthMismatch)? as f32,
            ];
            if rgb.iter().any(|v| !v.is_finite()) || !self.normalization.is_finite() {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let rg = rgb[0] * self.weights[0] + rgb[1] * self.weights[1];
            let gray = (rg + rgb[2] * self.weights[2]) * self.normalization;
            if !gray.is_finite() {
                return Err(RasterInputError::UnsupportedConversion);
            }
            staged.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?.fill(f64::from(gray));
            let alpha = *input.get(3).ok_or(RasterInputError::LengthMismatch)? as f32;
            if !alpha.is_finite() {
                return Err(RasterInputError::UnsupportedConversion);
            }
            *staged.get_mut(3).ok_or(RasterInputError::LengthMismatch)? = f64::from(alpha);
        }
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        output.copy_from_slice(&staged);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    fn operation(preserve_luminosity: bool) -> MonoMixer {
        MonoMixer { red: -0.5, green: 1.5, blue: 0.25, preserve_luminosity }
    }
    #[test]
    fn signed_weights_preserve_and_zero_sum_are_general() {
        let input = [0.125, 0.375, 0.75, -0.0];
        let mut output = [0.; 4];
        operation(false).try_eval_pixel_with(&input, &mut output, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
        assert_eq!(output[..3], [0.6875; 3]);
        assert_eq!(output[3].to_bits(), input[3].to_bits());
        operation(true).try_eval_pixel_with(&input, &mut output, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
        assert_eq!(output[0], f64::from(0.55_f32));
        let zero_sum = MonoMixer { red: 1.00000001, green: -1., blue: 0., preserve_luminosity: true };
        zero_sum.try_eval_pixel_with(&[1., 0.25, 0.5, 0.625], &mut output, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
        assert_eq!(output, [0.75, 0.75, 0.75, 0.625]);
    }
    #[test]
    fn disabled_keeps_exact_current_bits() {
        let input = [-0., f64::MAX, -f64::MIN_POSITIVE, f64::from_bits(0x3fd0000000000001)];
        let mut output = [0.; 4];
        operation(true).try_eval_pixel_with(&input, &mut output, false, &Interrupt::NONE, &|_| Ok(())).unwrap();
        assert_eq!(input.map(f64::to_bits), output.map(f64::to_bits));
    }
    #[test]
    fn hostile_domains_shapes_nonfinite_and_overflow_are_atomic() {
        for value in [f64::NAN, f64::INFINITY, -5.0001, 5.0001] {
            let mut out = [17.; 4];
            assert!(MonoMixer { red: value, ..operation(false) }.try_eval_pixel_with(&[0.; 4], &mut out, false, &Interrupt::NONE, &|_| Ok(())).is_err());
            assert_eq!(out, [17.; 4]);
        }
        for input in [[f64::NAN, 0., 0., 1.], [f64::MAX, 0., 0., 1.], [0., 0., 0., f64::INFINITY]] {
            let mut out = [17.; 4];
            assert!(operation(false).try_eval_pixel_with(&input, &mut out, true, &Interrupt::NONE, &|_| Ok(())).is_err());
            assert_eq!(out, [17.; 4]);
        }
        let mut out = [17.; 4];
        assert!(operation(false).try_eval_pixel_with(&[0.; 3], &mut out, true, &Interrupt::NONE, &|_| Ok(())).is_err());
        assert_eq!(out, [17.; 4]);
        let overflow = MonoMixer { red: 1., green: -1., blue: f64::from(f32::from_bits(1)), preserve_luminosity: true };
        assert!(overflow.try_eval_pixel_with(&[1., 0., 0., 1.], &mut out, true, &Interrupt::NONE, &|_| Ok(())).is_err());
        assert_eq!(out, [17.; 4]);
    }
    #[test]
    fn cancellation_and_work_denial_preserve_output() {
        let mut out = [17.; 4];
        let flag = AtomicBool::new(true);
        let cancelled = || flag.load(Ordering::Relaxed);
        let ctl = Interrupt::cancel_only(&cancelled);
        assert_eq!(operation(false).try_eval_pixel_with(&[0.; 4], &mut out, true, &ctl, &|_| Ok(())), Err(RasterInputError::Cancelled));
        flag.store(false, Ordering::Relaxed);
        assert_eq!(
            operation(false).try_eval_pixel_with(&[0.; 4], &mut out, true, &ctl, &|_| {
                flag.store(true, Ordering::Relaxed);
                Ok(())
            }),
            Err(RasterInputError::Cancelled)
        );
        assert_eq!(out, [17.; 4]);
        assert!(operation(false).try_eval_pixel_with(&[0.; 4], &mut out, true, &Interrupt::NONE, &|_| Err(RasterInputError::UnsupportedConversion)).is_err());
        assert_eq!(out, [17.; 4]);
    }
    #[test]
    fn checked_surface_writeback_preserves_computed_rgb_and_float_alpha() {
        use photocraft_color::PixelFormat;
        use photocraft_geom::Rect;
        use photocraft_raster::Surface;
        let area = Rect::new(0, 0, 1, 1);
        let mut source = Surface::new(PixelFormat::RGBA64F);
        source.try_write_interleaved(area, &[0.25_f64, 0.5, 0.75, 0.333333333].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
        let op = super::super::point::PointOperation::MonoMixerSigned { red: 0.25, green: 0.5, blue: 0.75, preserve_luminosity: false };
        let output = super::super::point::try_apply(&source, area, op, 64 << 20, 64 << 20, &Interrupt::NONE).unwrap();
        let mut actual = [0.; 4];
        output.try_read_pixel_f64(0, 0, &mut actual).unwrap();
        assert_eq!(actual, [0.875, 0.875, 0.875, f64::from(0.333333333_f64 as f32)]);
        let mut original = [0.; 4];
        source.try_read_pixel_f64(0, 0, &mut original).unwrap();
        assert_eq!(original, [0.25, 0.5, 0.75, 0.333333333]);
    }
    #[test]
    #[ignore = "release-only 24MP standalone Mono Mixer working kernel timing"]
    fn prepared_24mp_working_kernel() {
        let prepared = operation(true).prepare().unwrap();
        let start = std::time::Instant::now();
        let mut output = [0.; 4];
        for i in 0..24_000_000 {
            let input = [f64::from((i % 257) as u16) / 256., 0.375, 0.75, 0.625];
            prepared.try_eval_pixel_with(std::hint::black_box(&input), &mut output, true, &Interrupt::NONE, &|_| Ok(())).unwrap();
            std::hint::black_box(output);
        }
        println!("24MP Mono Mixer prepared linear working kernel {:?}; transfer/native/storage excluded", start.elapsed());
    }
}
