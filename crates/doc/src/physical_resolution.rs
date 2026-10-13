//! Exact physical resolution. Custom factors are units per inch, never a runtime grant.
use crate::xcf_state::XcfValidationError;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomUnit {
    pub name: String,
    pub factor: f64,
    pub digits: u32,
    pub symbol: String,
    pub abbreviation: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum PhysicalUnit {
    Pixel,
    #[default]
    Inch,
    Millimeter,
    Point,
    Pica,
    Custom(CustomUnit),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalResolution {
    pub x_dpi: f64,
    pub y_dpi: f64,
    pub unit: PhysicalUnit,
}
impl PhysicalResolution {
    pub fn square(dpi: f64) -> Self {
        Self { x_dpi: dpi, y_dpi: dpi, unit: PhysicalUnit::Inch }
    }
    pub fn validate(&self) -> Result<(), XcfValidationError> {
        if !self.x_dpi.is_finite() || !self.y_dpi.is_finite() || self.x_dpi <= 0.0 || self.y_dpi <= 0.0 {
            return Err(XcfValidationError::new("resolution", "DPI axes must be positive finite values"));
        }
        if let PhysicalUnit::Custom(u) = &self.unit
            && (!u.factor.is_finite()
                || u.factor <= 0.0
                || u.name.is_empty()
                || [&u.name, &u.symbol, &u.abbreviation].iter().any(|s| s.len() > 1 << 20 || s.contains('\0')))
        {
            return Err(XcfValidationError::new("unit", "invalid custom unit definition"));
        }
        Ok(())
    }
    pub fn inches(&self, width: u32, height: u32) -> Result<(f64, f64), XcfValidationError> {
        self.validate()?;
        let dimensions = (f64::from(width) / self.x_dpi, f64::from(height) / self.y_dpi);
        if !dimensions.0.is_finite() || !dimensions.1.is_finite() {
            return Err(XcfValidationError::new("resolution", "physical dimensions exceed finite range"));
        }
        Ok(dimensions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inches_preserves_axes_and_refuses_overflow() {
        let resolution = PhysicalResolution { x_dpi: 120.0, y_dpi: 240.0, unit: PhysicalUnit::Millimeter };
        assert_eq!(resolution.inches(600, 600).unwrap(), (5.0, 2.5));
        let tiny = PhysicalResolution { x_dpi: f64::MIN_POSITIVE / 1024.0, ..resolution };
        assert!(tiny.inches(600, 600).is_err());
        assert_eq!(tiny.inches(0, 0).unwrap(), (0.0, 0.0));
    }
}
