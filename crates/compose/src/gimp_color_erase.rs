//! Color-to-alpha decomposition from the source-over equation.
//!
//! For each unit RGB component, find the least retained coverage permitting
//! `back = source*(1-retained) + residual*retained` with residual in [0,1].
//! The maximum component bound is the shared RGB coverage. Source opacity
//! interpolates removed coverage, not the unassociated residual color.
use super::{CompositionError, Result};

pub(crate) fn erase(back: [f64; 4], source: [f64; 4]) -> Result<[f64; 4]> {
    if back.iter().chain(source.iter()).any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(CompositionError::Unsupported("Color Erase finite unit RGB/alpha domain is not qualified"));
    }
    if source[3] == 0.0 {
        return Ok(back);
    }
    if back[3] == 0.0 {
        return Ok([0.0; 4]);
    }
    let mut retained = 0.0_f64;
    for (&b, &s) in back.iter().take(3).zip(source.iter().take(3)) {
        let bound = if b > s {
            (b - s) / (1.0 - s)
        } else if b < s {
            (s - b) / s
        } else {
            0.0
        };
        retained = retained.max(bound);
    }
    // Retain tiny positive alpha instead of cancelling it in 1-(1-retained).
    let remaining = (1.0 - source[3]) + source[3] * retained;
    if remaining == 0.0 {
        return Ok([0.0; 4]);
    }
    let mut output = [0.0; 4];
    for (i, value) in output.iter_mut().take(3).enumerate() {
        *value = source[i] + (back[i] - source[i]) / remaining;
    }
    output[3] = back[3] * remaining;
    if output.iter().any(|v| !v.is_finite()) {
        return Err(CompositionError::Invalid("Color Erase arithmetic overflow"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimum_rgb_coverage_reconstructs_color_with_alpha_and_near_zero_limits() {
        // The least coverage is .5, fixed by red/blue gamut constraints.
        let b = [0.75, 0.5, 0.25, 0.8];
        let s = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(erase(b, s).unwrap(), [1.0, 0.5, 0.0, 0.4]);
        let partial = erase(b, [0.5, 0.5, 0.5, 0.5]).unwrap();
        let retained = partial[3] / b[3];
        for c in 0..3 {
            assert!((partial[c] * retained + s[c] * (1.0 - retained) - b[c]).abs() < 1e-15);
        }
        assert_eq!(erase([0.5, 0.5, 0.5, 0.8], s).unwrap(), [0.0; 4]);
        assert_eq!(erase(b, [0.5, 0.5, 0.5, 0.0]).unwrap(), b);
        let tiny = erase([1e-300, 1e-300, 1e-300, 1.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
        assert_eq!(tiny, [1.0, 1.0, 1.0, 1e-300]);
        assert!(erase([-1.0, 0.0, 0.0, 0.0], s).is_err());
        assert!(erase([f64::NAN, 0.0, 0.0, 1.0], s).is_err());
        assert!(erase(b, [0.0, 0.0, 0.0, f64::INFINITY]).is_err());
    }
}
