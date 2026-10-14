//! Observed Float32 alpha stage for Normal/Replace threshold-alpha filters.
//! This is not an RGB conversion, a Surface format or source admission policy.
//! Colors, ICC conversion, masks and final layer composition remain caller stages.
use super::threshold_alpha::ThresholdAlpha;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Replace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedOpacity {
    Half,
    Full,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub borrowed_bytes: usize,
    pub output_bytes: usize,
    pub peak_bytes: usize,
    pub work_units: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<(), E> {
    control.check().map_err(|_| E::Cancelled)
}
pub fn requirements(count: usize, limits: Limits) -> Result<Requirements, E> {
    let ceiling = Limits::default();
    if limits.resident_bytes > ceiling.resident_bytes || limits.work_units > ceiling.work_units {
        return Err(E::LimitExceeded);
    }
    let bytes = count.checked_mul(std::mem::size_of::<f32>()).ok_or(E::LimitExceeded)?;
    let peak = bytes.checked_mul(2).ok_or(E::LimitExceeded)?;
    let work = u64::try_from(count).map_err(|_| E::LimitExceeded)?.checked_mul(20).ok_or(E::LimitExceeded)?;
    if u64::try_from(peak).map_err(|_| E::LimitExceeded)? > limits.resident_bytes || work > limits.work_units {
        return Err(E::LimitExceeded);
    }
    Ok(Requirements { borrowed_bytes: bytes, output_bytes: bytes, peak_bytes: peak, work_units: work })
}
/// Only the observed unit Float32 alpha and Half/Full opacity domains are used.
/// Reuses the existing strict Float32-alpha/Double-threshold point evaluator.
pub fn evaluate(
    source: &[f32],
    threshold: ThresholdAlpha,
    mode: Mode,
    opacity: ObservedOpacity,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Vec<f32>, E> {
    poll(control)?;
    threshold.validate()?;
    let required = requirements(source.len(), limits)?;
    for chunk in source.chunks(128) {
        poll(control)?;
        if chunk.iter().any(|alpha| !alpha.is_finite() || alpha.is_sign_negative() || *alpha > 1.0) {
            return Err(E::UnsupportedConversion);
        }
    }
    poll(control)?;
    let mut output: Vec<f32> = Vec::new();
    output.try_reserve_exact(source.len()).map_err(|_| E::AllocationFailed)?;
    let actual_output = output.capacity().checked_mul(std::mem::size_of::<f32>()).ok_or(E::LimitExceeded)?;
    let actual_peak = required.borrowed_bytes.checked_add(actual_output).ok_or(E::LimitExceeded)?;
    if u64::try_from(actual_peak).map_err(|_| E::LimitExceeded)? > limits.resident_bytes {
        return Err(E::LimitExceeded);
    }
    poll(control)?;
    for chunk in source.chunks(128) {
        poll(control)?;
        for alpha in chunk {
            // Numeric domain was checked before allocating private output.
            let base = f64::from(*alpha);
            let mut point = [0.0; 2];
            threshold.try_eval_pixel(&[0.0, base], 1, &mut point)?;
            let filtered = *point.get(1).ok_or(E::LengthMismatch)?;
            let next = match (mode, opacity) {
                (Mode::Replace, ObservedOpacity::Full) => filtered,
                (Mode::Replace, ObservedOpacity::Half) => base + (filtered - base) * 0.5,
                (Mode::Normal, ObservedOpacity::Full) => {
                    if filtered == 0.0 {
                        base
                    } else {
                        1.0
                    }
                }
                (Mode::Normal, ObservedOpacity::Half) => {
                    if filtered == 0.0 {
                        base
                    } else {
                        base + (1.0 - base) * 0.5
                    }
                }
            };
            output.push(next as f32);
        }
    }
    poll(control)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_transparent_result_preserves_base_while_replace_erases() {
        let input = [0.0, 0.25, 0.49, 0.5, 0.51, 0.75, 1.0, 1e-8];
        let t = ThresholdAlpha { value: 0.5 };
        let normal = evaluate(&input, t, Mode::Normal, ObservedOpacity::Half, Limits::default(), &Interrupt::NONE).unwrap();
        let replace = evaluate(&input, t, Mode::Replace, ObservedOpacity::Half, Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(&normal[..4], &input[..4]);
        assert_eq!(replace[1], 0.125);
        assert_eq!(replace[3], 0.25);
        assert_eq!(normal[5], 0.875);
        assert_eq!(replace[5], normal[5]);
        assert_eq!(normal[7].to_bits(), input[7].to_bits());
    }
    #[test]
    fn hostile_alpha_and_lower_limits_refuse_private_output() {
        let t = ThresholdAlpha { value: 0.5 };
        for alpha in [f32::NAN, f32::INFINITY, -0.0, -0.1, 1.01] {
            assert!(evaluate(&[alpha], t, Mode::Normal, ObservedOpacity::Full, Limits::default(), &Interrupt::NONE).is_err());
        }
        assert!(evaluate(&[0.5], t, Mode::Replace, ObservedOpacity::Full, Limits { resident_bytes: 7, work_units: 20 }, &Interrupt::NONE).is_err());
        assert!(evaluate(&[0.5], t, Mode::Replace, ObservedOpacity::Full, Limits { resident_bytes: 8, work_units: 19 }, &Interrupt::NONE).is_err());
        let cancelled = || true;
        assert!(matches!(evaluate(&[0.5], t, Mode::Normal, ObservedOpacity::Full, Limits::default(), &Interrupt::cancel_only(&cancelled)), Err(E::Cancelled)));
    }
}
