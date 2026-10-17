//! Standalone checked Alpha Clip candidate; no registry/import/profile admission.
//! The caller supplies unassociated unit RGB and alpha in an independently qualified
//! working representation: public conversion controls prove Linear Float32 for
//! implicit RGB Linear/Perceptual and all implicit Gray inputs, Nonlinear Float32
//! only for implicit RGB nonlinear inputs. Gray must first become linear RGB.
//! An operative clip narrows all four working components; neither-clip/disabled is
//! exact identity. Native result encoding/quantization is a separate boundary.
//! Gray source/profile conversion must be independently supplied as RGB by a caller.
use photocraft_raster::{Interrupt, RasterInputError};
pub const VERSION: &str = "0:0";
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaClip {
    pub clip_low: bool,
    pub low_limit: f32,
    pub clip_high: bool,
    pub high_limit: f32,
}
impl AlphaClip {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if [self.low_limit, self.high_limit].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) || self.low_limit > self.high_limit {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    pub fn try_eval_pixel(self, input: &[f64], channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.try_eval_pixel_with(input, channels, output, true, &Interrupt::NONE, &|_| Ok(()))
    }
    /// Eight kernel sample units, charged before staging. Caller-owned transfer,
    /// source storage and result allocations are outside this scalar meter.
    /// Disabled or neither-clip identity is exact after the same input validation.
    /// On every error/cancellation the output slice remains unchanged.
    pub fn try_eval_pixel_with(
        self,
        input: &[f64],
        channels: usize,
        output: &mut [f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), RasterInputError>,
    ) -> Result<(), RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate()?;
        if channels != 3 || input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        work(8)?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let mut staged = [0.; 4];
        staged.copy_from_slice(input);
        if enabled && (self.clip_low || self.clip_high) {
            for value in &mut staged {
                *value = f64::from(*value as f32);
            }
            let alpha = staged.get_mut(3).ok_or(RasterInputError::LengthMismatch)?;
            if self.clip_low && *alpha < f64::from(self.low_limit) {
                *alpha = f64::from(self.low_limit);
            }
            if self.clip_high && *alpha > f64::from(self.high_limit) {
                *alpha = f64::from(self.high_limit);
            }
        }
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        output.copy_from_slice(&staged);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn operation() -> AlphaClip {
        AlphaClip { clip_low: true, low_limit: 0.25, clip_high: true, high_limit: 0.75 }
    }
    #[test]
    fn operative_float32_channels_and_low_then_high_clamp() {
        let op = operation();
        for (alpha, expected) in
            [(0., 0.25), (0.25 - 2_f64.powi(-40), 0.25), (0.25 + 2_f64.powi(-40), 0.25), (0.50000007, f64::from(0.50000007_f32)), (1., 0.75)]
        {
            let input = [0.125 + 2_f64.powi(-40), -0., 0.625 + 2_f64.powi(-41), alpha];
            let mut output = [17.; 4];
            op.try_eval_pixel(&input, 3, &mut output).unwrap();
            assert_eq!(output[3].to_bits(), expected.to_bits());
            for (a, b) in output.iter().take(3).zip(input) {
                assert_eq!(a.to_bits(), f64::from(b as f32).to_bits());
            }
        }
    }
    #[test]
    fn equal_signed_zero_endpoint_preserves_native_alpha_sign() {
        for high_limit in [0., 1.] {
            let op = AlphaClip { clip_low: true, low_limit: 0., clip_high: true, high_limit };
            let mut output = [17.; 4];
            op.try_eval_pixel(&[0.125, 0.375, 0.625, -0.], 3, &mut output).unwrap();
            assert_eq!(output.map(f64::to_bits), [0.125_f64, 0.375, 0.625, -0.].map(f64::to_bits));
        }
    }
    #[test]
    fn flags_and_disabled_preserve_exact_identity_when_no_clip_is_operative() {
        let input = [0.173 + 2_f64.powi(-40), -0., 0.719, 0.50000007];
        for enabled in [false, true] {
            for low in [false, true] {
                for high in [false, true] {
                    let op = AlphaClip { clip_low: low, clip_high: high, ..operation() };
                    let mut output = [0.; 4];
                    op.try_eval_pixel_with(&input, 3, &mut output, enabled, &Interrupt::NONE, &|_| Ok(())).unwrap();
                    if !enabled || (!low && !high) {
                        assert_eq!(input.map(f64::to_bits), output.map(f64::to_bits));
                    } else {
                        assert_eq!(output[3], f64::from(input[3] as f32));
                    }
                }
            }
        }
    }
    #[test]
    fn invalid_config_and_input_errors_are_atomic() {
        for v in [f32::NAN, f32::INFINITY, -0.001, 1.001] {
            for lower in [false, true] {
                let op = if lower { AlphaClip { low_limit: v, ..operation() } } else { AlphaClip { high_limit: v, ..operation() } };
                let mut out = [17.; 4];
                assert!(op.try_eval_pixel(&[0.25, 0.5, 0.75, 0.5], 3, &mut out).is_err());
                assert_eq!(out, [17.; 4]);
            }
        }
        assert!(AlphaClip { low_limit: 0.875, high_limit: 0.125, ..operation() }.validate().is_err());
        for value in [f64::NAN, f64::INFINITY, -0.001, 1.001] {
            for field in 0..4 {
                let mut input = [0.25, 0.5, 0.75, 0.5];
                input[field] = value;
                let mut out = [17.; 4];
                assert!(operation().try_eval_pixel(&input, 3, &mut out).is_err());
                assert_eq!(out, [17.; 4]);
            }
        }
        let mut out = [17.; 4];
        assert!(operation().try_eval_pixel(&[0.5; 4], 1, &mut out).is_err());
        assert_eq!(out, [17.; 4]);
        let mut short = [17.; 3];
        assert!(operation().try_eval_pixel(&[0.5; 4], 3, &mut short).is_err());
        assert_eq!(short, [17.; 3]);
    }
    #[test]
    fn borrowed_work_and_pre_mid_final_cancellation_never_publish_output() {
        let op = operation();
        let input = [0.25, 0.5, 0.75, 0.5];
        let units = Cell::new(0u64);
        let mut out = [17.; 4];
        assert_eq!(
            op.try_eval_pixel_with(&input, 3, &mut out, true, &Interrupt::NONE, &|n| {
                units.set(n);
                Err(RasterInputError::LimitExceeded)
            }),
            Err(RasterInputError::LimitExceeded)
        );
        assert_eq!(units.get(), 8);
        assert_eq!(out, [17.; 4]);
        for stop_at in 1..=3 {
            let polls = std::sync::atomic::AtomicU32::new(0);
            let poll = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 >= stop_at;
            let ctl = Interrupt::cancel_only(&poll);
            let mut out = [17.; 4];
            assert_eq!(op.try_eval_pixel_with(&input, 3, &mut out, true, &ctl, &|_| Ok(())), Err(RasterInputError::Cancelled));
            assert_eq!(out, [17.; 4]);
        }
    }
}
