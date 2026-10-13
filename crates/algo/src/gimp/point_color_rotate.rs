//! Checked Color Rotate math in explicit unassociated perceptual Float RGB.
//! Native/profile conversion and source admission are separate contracts.
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HueArc {
    pub from_degrees: f32,
    pub to_degrees: f32,
    pub clockwise: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrayMode {
    TreatAs,
    ChangeTo,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorRotate {
    pub source: HueArc,
    pub destination: HueArc,
    pub gray_threshold: f32,
    pub gray_mode: GrayMode,
    pub gray_hue_degrees: f32,
    pub gray_saturation: f32,
}
impl ColorRotate {
    pub fn validate(&self) -> Result<(), RasterInputError> {
        for angle in [self.source.from_degrees, self.source.to_degrees, self.destination.from_degrees, self.destination.to_degrees, self.gray_hue_degrees] {
            if !angle.is_finite() || !(0.0..=360.0).contains(&angle) {
                return Err(RasterInputError::InvalidSample);
            }
        }
        if !self.gray_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.gray_threshold)
            || !self.gray_saturation.is_finite()
            || !(0.0..=1.0).contains(&self.gray_saturation)
        {
            return Err(RasterInputError::InvalidSample);
        }
        // Circular source boundary behavior has not yet been qualified. In
        // particular a zero source span is singular and its public baked
        // responses were unstable. Never guess that it means a full revolution.
        if self.source.clockwise
            || self.source.from_degrees >= self.source.to_degrees
            || radians(self.source.from_degrees) == radians(self.source.to_degrees)
            || self.destination.from_degrees == self.destination.to_degrees
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    /// Allocation-free evaluation of one unit working RGBA pixel. The explicit
    /// source domain is a nonwrapping counterclockwise arc. Float stage rounding
    /// is retained; no boundary epsilon, native conversion or cache is used.
    /// Errors leave output untouched, including wrong buffer lengths.
    pub fn try_eval_perceptual_float(&self, input: &[f32], output: &mut [f32]) -> Result<(), RasterInputError> {
        self.validate()?;
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::InvalidSample);
        }
        let [red, green, blue, alpha]: [f32; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
        let value = red.max(green).max(blue);
        let delta = value - red.min(green).min(blue);
        let mut saturation = if value == 0.0 { 0.0 } else { delta / value };
        let mut hue = if delta == 0.0 {
            0.0
        } else if value == red {
            (green - blue) / delta
        } else if value == green {
            (blue - red) / delta + 2.0
        } else {
            (red - green) / delta + 4.0
        };
        // Wrap the signed sector before normalization. Algebraically
        // equivalent reordered Float expressions can cross a hue boundary.
        if hue < 0.0 {
            hue += 6.0;
        }
        hue /= 6.0;
        let gray = saturation <= self.gray_threshold;
        if gray {
            hue = (f64::from(radians(self.gray_hue_degrees)) / std::f64::consts::TAU) as f32;
            saturation = self.gray_saturation;
        }
        let mut degrees = (f64::from(hue) * std::f64::consts::TAU) as f32;
        if !(gray && self.gray_mode == GrayMode::ChangeTo) && degrees >= radians(self.source.from_degrees) && degrees <= radians(self.source.to_degrees) {
            let distance = degrees - radians(self.source.from_degrees);
            let length = radians(self.source.to_degrees) - radians(self.source.from_degrees);
            let mut destination_length = if self.destination.clockwise {
                radians(self.destination.from_degrees) - radians(self.destination.to_degrees)
            } else {
                radians(self.destination.to_degrees) - radians(self.destination.from_degrees)
            };
            if destination_length < 0.0 {
                destination_length += std::f64::consts::TAU as f32;
            }
            let change = distance / length * destination_length;
            degrees =
                if self.destination.clockwise { radians(self.destination.from_degrees) - change } else { radians(self.destination.from_degrees) + change };
            if degrees < 0.0 {
                degrees += std::f64::consts::TAU as f32;
            }
            if degrees >= std::f64::consts::TAU as f32 {
                degrees -= std::f64::consts::TAU as f32;
            }
        }
        let sector_hue = (f64::from(degrees) / std::f64::consts::TAU) as f32 * 6.0;
        let sector = sector_hue.floor();
        let fraction = sector_hue - sector;
        let p = value * (1.0 - saturation);
        let q = value * (1.0 - saturation * fraction);
        let t = value * (1.0 - saturation * (1.0 - fraction));
        let rgb = match (sector as u32) % 6 {
            0 => [value, t, p],
            1 => [q, value, p],
            2 => [p, value, t],
            3 => [p, q, value],
            4 => [t, p, value],
            _ => [value, p, q],
        };
        if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::InvalidSample);
        }
        for (destination, value) in output.iter_mut().zip(rgb.into_iter().chain([alpha])) {
            *destination = value;
        }
        Ok(())
    }
}

fn radians(degrees: f32) -> f32 {
    (f64::from(degrees) * std::f64::consts::PI / 180.0) as f32
}
