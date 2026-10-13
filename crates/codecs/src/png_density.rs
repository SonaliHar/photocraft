//! Checked PNG physical density; no pixel representation or workspace dependencies.
use crate::CodecError;

/// PNG pHYs pixels per metre, independently checked for both axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngDensity {
    x_ppm: u32,
    y_ppm: u32,
}
impl PngDensity {
    /// Convert positive finite f64 DPI to integer pixels/metre. The binary f64 value is
    /// converted through the exact 5000/127 ratio; half-ppm ties round upward. Positive tiny DPI may quantize to zero ppm, matching existing PNG behavior.
    /// PNG integers cap at 2^31-1; overflow cannot silently saturate. No source metadata is modified.
    pub fn from_dpi(x_dpi: f64, y_dpi: f64) -> Result<Self, CodecError> {
        fn axis(dpi: f64, name: &str) -> Result<u32, CodecError> {
            if !dpi.is_finite() || dpi <= 0.0 {
                return Err(CodecError::InvalidImage(format!("PNG density {name} DPI must be positive and finite")));
            }
            // An inch is exactly 127/5000 metres. Interpret the source f64
            // bits exactly, then round this rational density once, half upward.
            // This avoids f32 narrowing and floating division near a half ppm.
            let bits = dpi.to_bits();
            let biased = ((bits >> 52) & 0x7ff) as i32; // Mask proves 0..=2047.
            let fraction = bits & ((1_u64 << 52) - 1);
            let (significand, exponent) = if biased == 0 { (fraction, -1074) } else { ((1_u64 << 52) | fraction, biased - 1023 - 52) };
            let range_error = || CodecError::InvalidImage(format!("PNG density {name} pixels/metre must round into 0..=2^31-1"));
            // exponent>=0 implies DPI>=2^52, beyond the PNG integer cap. exponent<-100
            // implies DPI<2^-47, quantizes to zero ppm. The remaining
            // denominator (127<<100) and doubled remainder fit in u128.
            if exponent >= 0 {
                return Err(range_error());
            }
            if exponent < -100 {
                return Ok(0);
            }
            let numerator = u128::from(significand).checked_mul(5000).ok_or_else(range_error)?;
            let denominator = 127_u128.checked_shl(exponent.unsigned_abs()).ok_or_else(range_error)?;
            let remainder = numerator % denominator;
            let upward = remainder.checked_mul(2).ok_or_else(range_error)? >= denominator;
            let rounded = (numerator / denominator).checked_add(u128::from(upward)).ok_or_else(range_error)?;
            if rounded > (1_u128 << 31) - 1 {
                return Err(range_error());
            }
            u32::try_from(rounded).map_err(|_| range_error())
        }
        Ok(Self { x_ppm: axis(x_dpi, "x")?, y_ppm: axis(y_dpi, "y")? })
    }
    pub fn pixels_per_meter(self) -> (u32, u32) {
        (self.x_ppm, self.y_ppm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn density_axes_and_rounding_are_explicit() {
        assert_eq!(PngDensity::from_dpi(144.25, 287.75).unwrap().pixels_per_meter(), (5679, 11329));
        assert_eq!(PngDensity::from_dpi(7.9375, 23.8125).unwrap().pixels_per_meter(), (313, 938));
        assert_eq!(PngDensity::from_dpi(0.0254 * f64::from(i32::MAX), 72.0).unwrap().pixels_per_meter().0, i32::MAX as u32);
    }
    #[test]
    fn positive_tiny_density_quantizes_to_zero() {
        assert_eq!(PngDensity::from_dpi(0.001, f64::MIN_POSITIVE).unwrap().pixels_per_meter(), (0, 0));
    }
    #[test]
    fn invalid_or_unrepresentable_density_is_named() {
        for invalid in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, 0.0254 * (f64::from(i32::MAX) + 1.0)] {
            let x = PngDensity::from_dpi(invalid, 72.0).unwrap_err().to_string();
            let y = PngDensity::from_dpi(72.0, invalid).unwrap_err().to_string();
            assert!(x.contains("PNG density x"), "{x}");
            assert!(y.contains("PNG density y"), "{y}");
        }
    }
}
