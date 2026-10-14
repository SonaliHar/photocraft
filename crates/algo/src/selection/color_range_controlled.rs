//! Existing Lab box coverage with bounded storage/work and explicit cancellation.
use super::{LabRange, coverage};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
#[derive(Clone, Copy, Debug)]
pub struct ColorRangeLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for ColorRangeLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 }
    }
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
/// The borrowed Lab buffer and output are charged together. Positions/radius are
/// caller-established image-space values, not source-schema or profile admission.
pub fn color_range_lab_with_interrupt(
    lab: &[[f32; 4]],
    width: usize,
    range: &LabRange,
    fuzziness: f32,
    grid8: bool,
    localized: Option<(f32, &[(f32, f32)])>,
    limits: ColorRangeLimits,
    control: &Interrupt<'_>,
) -> Result<Vec<f32>, RasterInputError> {
    poll(control)?;
    let defaults = ColorRangeLimits::default();
    if limits.resident_bytes > defaults.resident_bytes
        || limits.work_units > defaults.work_units
        || width == 0
        || !lab.len().is_multiple_of(width)
        || !fuzziness.is_finite()
        || fuzziness < 0.
    {
        return Err(RasterInputError::InvalidBounds);
    }
    let localized = localized.filter(|(_, at)| !at.is_empty());
    let points = localized.map_or(0, |(_, at)| at.len());
    if let Some((radius, at)) = localized {
        if !radius.is_finite() || radius < 0. || at.len() > 4096 {
            return Err(RasterInputError::InvalidBounds);
        }
        for &(x, y) in at {
            poll(control)?;
            if !x.is_finite() || !y.is_finite() {
                return Err(RasterInputError::InvalidBounds);
            }
        }
    }
    let bytes = (lab.len() as u64).checked_mul(20).and_then(|n| n.checked_add((points as u64).checked_mul(8)?)).ok_or(RasterInputError::LimitExceeded)?;
    let work = (lab.len() as u64)
        .checked_mul((points as u64).checked_mul(48).and_then(|n| n.checked_add(128)).ok_or(RasterInputError::LimitExceeded)?)
        .ok_or(RasterInputError::LimitExceeded)?;
    if bytes > limits.resident_bytes || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut output = Vec::new();
    output.try_reserve_exact(lab.len()).map_err(|_| RasterInputError::AllocationFailed)?;
    if (output.capacity() as u64)
        .checked_mul(4)
        .and_then(|n| n.checked_add((lab.len() as u64).checked_mul(16)?))
        .and_then(|n| n.checked_add((points as u64).checked_mul(8)?))
        .is_none_or(|n| n > limits.resident_bytes)
    {
        return Err(RasterInputError::LimitExceeded);
    }
    for (index, &[l, a, b, alpha]) in lab.iter().enumerate() {
        if index % 256 == 0 {
            poll(control)?;
        }
        let mut k = if alpha.is_nan() || alpha <= 0. { 0. } else { coverage([l, a, b], range, fuzziness, grid8) };
        if let Some((radius, at)) = localized
            && k > 0.
        {
            let (px, py) = ((index % width) as f32, (index / width) as f32);
            let mut near = 0.0f32;
            for (j, &(sx, sy)) in at.iter().enumerate() {
                if j % 64 == 0 {
                    poll(control)?;
                }
                near = near.max((1. - ((px - sx).powi(2) + (py - sy).powi(2)).sqrt() / radius.max(1.)).clamp(0., 1.));
            }
            k *= near;
        }
        output.push(k);
    }
    poll(control)?;
    Ok(output)
}

/// Closed typed presets, using the upstream f32 hue/Rec.601 equations.
#[derive(Clone, Copy, Debug)]
pub enum NamedColorRange {
    Hue { center: f32 },
    SkinTones { fuzziness: f32 },
    Tone { low: f32, high: f32, falloff: f32 },
}
impl NamedColorRange {
    pub fn validate(self) -> Result<(), RasterInputError> {
        let valid = match self {
            Self::Hue { center } => center.is_finite() && [0., 60., 120., 180., 240., 300.].contains(&center),
            Self::SkinTones { fuzziness } => fuzziness.is_finite() && (0. ..=200.).contains(&fuzziness),
            Self::Tone { low, high, falloff } => {
                low.is_finite() && high.is_finite() && falloff.is_finite() && 0. <= low && low <= high && high <= 255. && (0. ..=255.).contains(&falloff)
            }
        };
        if valid { Ok(()) } else { Err(RasterInputError::InvalidBounds) }
    }
}
pub fn named_color_range_with_interrupt(
    px: &[[f32; 4]],
    width: usize,
    preset: NamedColorRange,
    limits: ColorRangeLimits,
    control: &Interrupt<'_>,
) -> Result<Vec<f32>, RasterInputError> {
    poll(control)?;
    preset.validate()?;
    let defaults = ColorRangeLimits::default();
    if limits.resident_bytes > defaults.resident_bytes || limits.work_units > defaults.work_units || width == 0 || !px.len().is_multiple_of(width) {
        return Err(RasterInputError::InvalidBounds);
    }
    let held = (px.len() as u64).checked_mul(16).ok_or(RasterInputError::LimitExceeded)?;
    let work = (px.len() as u64).checked_mul(256).ok_or(RasterInputError::LimitExceeded)?;
    if held.checked_add((px.len() as u64).checked_mul(4).ok_or(RasterInputError::LimitExceeded)?).is_none_or(|n| n > limits.resident_bytes)
        || work > limits.work_units
    {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut output = Vec::new();
    output.try_reserve_exact(px.len()).map_err(|_| RasterInputError::AllocationFailed)?;
    if held.checked_add((output.capacity() as u64).checked_mul(4).ok_or(RasterInputError::LimitExceeded)?).is_none_or(|n| n > limits.resident_bytes) {
        return Err(RasterInputError::LimitExceeded);
    }
    // Computed exactly once, as in the original skin_tones function.
    let (sin, cos) = (-40.0f32).to_radians().sin_cos();
    for (index, &[r, g, b, alpha]) in px.iter().enumerate() {
        if index % 256 == 0 {
            poll(control)?;
        }
        let value = if alpha <= 0. {
            0.
        } else {
            match preset {
                NamedColorRange::Hue { center } => {
                    let (h, chroma) = super::hue_chroma([r, g, b].map(|v| v.clamp(0., 1.)));
                    let dh = (h - center).rem_euclid(360.);
                    let dh = dh.min(360. - dh);
                    ((1. - dh / 60.).clamp(0., 1.) * chroma).clamp(0., 1.)
                }
                NamedColorRange::SkinTones { fuzziness } => {
                    let [r, g, b] = [r, g, b].map(|v| v.clamp(0., 1.) * 255.);
                    let y = 0.299 * r + 0.587 * g + 0.114 * b;
                    let cb = 128. - 0.168_736 * r - 0.331_264 * g + 0.5 * b - 104.;
                    let cr = 128. + 0.5 * r - 0.418_688 * g - 0.081_312 * b - 158.;
                    let d = ((cos * cb + sin * cr) / 22.).hypot((cos * cr - sin * cb) / 14.);
                    let ramp = (fuzziness / 100.).max(1e-3);
                    let chroma = (1. - (d - 1.) / ramp).clamp(0., 1.);
                    let dark = ((y - 25.) / 30.).clamp(0., 1.);
                    let bright = ((250. - y) / 20.).clamp(0., 1.);
                    chroma * dark * bright
                }
                NamedColorRange::Tone { low, high, falloff } => {
                    let ramp = falloff.max(1e-3);
                    let l = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0., 1.) * 255.;
                    let out = if l < low {
                        low - l
                    } else if l > high {
                        l - high
                    } else {
                        0.
                    };
                    (1. - out / ramp).clamp(0., 1.)
                }
            }
        };
        output.push(value);
    }
    poll(control)?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_localized_grid_and_alpha_oracle() {
        let pixels = [[50., 20., 10., 1.], [75., 40., -10., 0.], [50., 20., 10., 1.], [45., 18., 12., 0.5]];
        let range = LabRange::point([50., 20., 10.]);
        let positions = [(0., 0.), (1., 1.)];
        for grid in [false, true] {
            for localized in [None, Some((2., positions.as_slice()))] {
                assert_eq!(
                    color_range_lab_with_interrupt(&pixels, 2, &range, 40., grid, localized, ColorRangeLimits::default(), &Interrupt::NONE).unwrap(),
                    super::super::color_range_lab(&pixels, 2, &range, 40., grid, localized)
                );
            }
        }
    }
    #[test]
    fn low_work_and_cancel_are_errors_not_partial_masks() {
        let range = LabRange::point([50., 0., 0.]);
        let pixels = [[50., 0., 0., 1.]; 4];
        assert_eq!(
            color_range_lab_with_interrupt(&pixels, 2, &range, 40., false, None, ColorRangeLimits { resident_bytes: 1024, work_units: 1 }, &Interrupt::NONE),
            Err(RasterInputError::LimitExceeded)
        );
        assert_eq!(
            color_range_lab_with_interrupt(&pixels, 2, &range, 40., false, None, ColorRangeLimits::default(), &Interrupt::cancel_only(&|| true)),
            Err(RasterInputError::Cancelled)
        );
    }
    #[test]
    fn named_presets_match_original_math_at_depth_neutral_float_inputs() {
        let px = [[1., 0., 0., 1.], [0.7, 0.4, 0.3, 1.], [0.95, 0.9, 0.8, 0.5], [-0.2, 1.3, 0.1, 0.], [0.5, 0.5, 0.5, 1.], [1., 1., 1., 1.]];
        for center in [0., 60., 120., 180., 240., 300.] {
            assert_eq!(
                named_color_range_with_interrupt(&px, 3, NamedColorRange::Hue { center }, ColorRangeLimits::default(), &Interrupt::NONE).unwrap(),
                super::super::hue_range(&px, center)
            );
        }
        for fuzziness in [0., 40., 200.] {
            assert_eq!(
                named_color_range_with_interrupt(&px, 3, NamedColorRange::SkinTones { fuzziness }, ColorRangeLimits::default(), &Interrupt::NONE).unwrap(),
                super::super::skin_tones(&px, fuzziness)
            );
        }
        for (low, high, falloff) in [(0., 65., 51.), (190., 255., 0.), (105., 150., 255.)] {
            assert_eq!(
                named_color_range_with_interrupt(&px, 3, NamedColorRange::Tone { low, high, falloff }, ColorRangeLimits::default(), &Interrupt::NONE).unwrap(),
                super::super::tone_range(&px, low, high, falloff)
            );
        }
    }
}
