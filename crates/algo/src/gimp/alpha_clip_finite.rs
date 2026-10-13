//! Standalone finite Alpha Clip operation foundation, without registry admission.
//!
//! Input is unassociated RGBA in a separately qualified working space. The caller
//! owns Gray/profile transfer and native result encoding. Active operations require
//! finite Float32-representable components and narrow all four components. Disabled
//! and neither-clip operations preserve finite Float64 bits exactly. No document
//! coverage policy, import eligibility or threshold normalization is implied.
use photocraft_raster::{Interrupt, RasterInputError};
/// The independently observed operation version; this module grants no importer admission.
pub const VERSION: &str = "0:0";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaClipFinite {
    pub clip_low: bool,
    pub low_limit: f32,
    pub clip_high: bool,
    pub high_limit: f32,
}

impl AlphaClipFinite {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if !self.low_limit.is_finite() || !self.high_limit.is_finite() {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    pub fn try_eval_pixel(self, input: &[f64], output: &mut [f64]) -> Result<(), RasterInputError> {
        self.try_eval_pixel_with(input, output, true, &Interrupt::NONE, &|_| Ok(()))
    }
    /// Charges eight scalar sample units before staging. On failure the output is
    /// unchanged. Transfer, native quantization and caller allocations are excluded.
    pub fn try_eval_pixel_with(
        self,
        input: &[f64],
        output: &mut [f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), RasterInputError>,
    ) -> Result<(), RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate()?;
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        let active = enabled && (self.clip_low || self.clip_high);
        if input.iter().any(|v| !v.is_finite() || (active && !(*v as f32).is_finite())) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        work(8)?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let mut staged = [0.; 4];
        staged.copy_from_slice(input);
        if active {
            for value in &mut staged {
                *value = f64::from(*value as f32);
            }
            let alpha = staged.get_mut(3).ok_or(RasterInputError::LengthMismatch)?;
            // Public crossed-threshold native controls distinguish this precedence
            // from sequential max/min. Strict comparisons preserve equal and -0 bits.
            if self.clip_high && *alpha > f64::from(self.high_limit) {
                *alpha = f64::from(self.high_limit);
            } else if self.clip_low && *alpha < f64::from(self.low_limit) {
                *alpha = f64::from(self.low_limit);
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn crossed() -> AlphaClipFinite {
        AlphaClipFinite { clip_low: true, low_limit: 0.875, clip_high: true, high_limit: 0.125 }
    }
    #[test]
    fn high_precedence_strict_equal_and_signed_zero_boundaries() {
        for (a, expected) in [(-0.125, 0.875), (-0., 0.875), (0.125, 0.875), (0.25, 0.125), (1.125, 0.125)] {
            let mut out = [0.; 4];
            crossed().try_eval_pixel(&[-0.125, 0.5, 1.5, a], &mut out).unwrap();
            assert_eq!(out, [-0.125, 0.5, 1.5, expected]);
        }
        for (low, high, a) in [(0., 1., -0.), (-0., 1., 0.), (-1., -0., 0.), (-1., 0., -0.), (0.5, 0.5, 0.5)] {
            let op = AlphaClipFinite { low_limit: low, high_limit: high, ..crossed() };
            let mut out = [0.; 4];
            op.try_eval_pixel(&[0., -0., 1., a], &mut out).unwrap();
            assert_eq!(out[3].to_bits(), a.to_bits());
            assert_eq!(out[1].to_bits(), (-0_f64).to_bits());
        }
    }
    #[test]
    fn flags_finite_extremes_underflow_and_noop_bits() {
        let input = [f64::MAX, -f64::MAX, -0., 1.125];
        for enabled in [false, true] {
            let op = AlphaClipFinite { clip_low: false, clip_high: false, ..crossed() };
            let mut out = [0.; 4];
            op.try_eval_pixel_with(&input, &mut out, enabled, &Interrupt::NONE, &|_| Ok(())).unwrap();
            assert_eq!(out.map(f64::to_bits), input.map(f64::to_bits));
        }
        let mut out = [0.; 4];
        crossed().try_eval_pixel_with(&input, &mut out, false, &Interrupt::NONE, &|_| Ok(())).unwrap();
        assert_eq!(out.map(f64::to_bits), input.map(f64::to_bits));
        let op = AlphaClipFinite { low_limit: -f32::MAX, high_limit: f32::MAX, ..crossed() };
        op.try_eval_pixel(&[f64::from(f32::MAX), -f64::from(f32::MAX), -f64::MIN_POSITIVE, 0.], &mut out).unwrap();
        assert_eq!(out[2].to_bits(), (-0_f64).to_bits());
        for (low, high, a, expected) in [(true, false, 0.25, 0.875), (false, true, 0.25, 0.125), (false, false, 0.25, 0.25)] {
            let op = AlphaClipFinite { clip_low: low, clip_high: high, ..crossed() };
            op.try_eval_pixel(&[-0.125, 0.5, 1.5, a], &mut out).unwrap();
            assert_eq!(out[3], expected);
        }
    }
    #[test]
    fn malformed_nonfinite_overflow_and_shapes_are_atomic() {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for op in [AlphaClipFinite { low_limit: v, ..crossed() }, AlphaClipFinite { high_limit: v, ..crossed() }] {
                for enabled in [false, true] {
                    let mut out = [17.; 4];
                    assert!(op.try_eval_pixel_with(&[0.; 4], &mut out, enabled, &Interrupt::NONE, &|_| Ok(())).is_err());
                    assert_eq!(out, [17.; 4]);
                }
            }
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, -f64::MAX] {
            for channel in 0..4 {
                let mut input = [0.; 4];
                input[channel] = value;
                let before = input.map(f64::to_bits);
                let mut out = [17.; 4];
                assert!(crossed().try_eval_pixel(&input, &mut out).is_err());
                assert_eq!(out, [17.; 4]);
                assert_eq!(input.map(f64::to_bits), before);
                if !value.is_finite() {
                    assert!(crossed().try_eval_pixel_with(&input, &mut out, false, &Interrupt::NONE, &|_| Ok(())).is_err());
                    assert_eq!(out, [17.; 4]);
                }
            }
        }
        for input in [vec![0.; 3], vec![0.; 5]] {
            let mut out = [17.; 4];
            assert!(crossed().try_eval_pixel(&input, &mut out).is_err());
            assert_eq!(out, [17.; 4]);
        }
        let mut short = [17.; 3];
        assert!(crossed().try_eval_pixel(&[0.; 4], &mut short).is_err());
        assert_eq!(short, [17.; 3]);
    }
    #[test]
    fn work_and_pre_mid_final_cancellation_are_atomic() {
        let input = [-0.125, 0.5, 1.5, -0.];
        let mut output = [17.; 4];
        let units = Cell::new(0);
        assert!(
            crossed()
                .try_eval_pixel_with(&input, &mut output, true, &Interrupt::NONE, &|n| {
                    units.set(n);
                    Err(RasterInputError::UnsupportedConversion)
                })
                .is_err()
        );
        assert_eq!(units.get(), 8);
        assert_eq!(output, [17.; 4]);
        for cancel_at in 1..=3 {
            let polls = AtomicUsize::new(0);
            let work = Cell::new(0);
            let cancelled = || polls.fetch_add(1, Ordering::Relaxed) + 1 >= cancel_at;
            assert_eq!(
                crossed().try_eval_pixel_with(&input, &mut output, true, &Interrupt::cancel_only(&cancelled), &|n| {
                    work.set(work.get() + n);
                    Ok(())
                }),
                Err(RasterInputError::Cancelled)
            );
            assert_eq!(output, [17.; 4]);
            assert_eq!(work.get(), if cancel_at == 1 { 0 } else { 8 });
        }
    }
}
