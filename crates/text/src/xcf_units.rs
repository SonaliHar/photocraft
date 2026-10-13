//! Borrowed source-unit resolution, independent of font authority and render admission.
//! The image custom definition must already be retained; unknown names never fall back.
use crate::xcf::{TextSourceError, poll};
use photocraft_doc::{
    PhysicalResolution, PhysicalUnit,
    physical_size::{self, Axis},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::Interrupt;

type Result<T> = std::result::Result<T, TextSourceError>;

/// Pixel lengths along X and Y. This is a unit basis, not a scalar font-render size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextUnitSize {
    pub x_pixels: f64,
    pub y_pixels: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextUnitLayout {
    pub font_size: TextUnitSize,
    pub box_size: Option<TextUnitSize>,
}
fn invalid() -> TextSourceError {
    TextSourceError::Unsupported("unknown or malformed retained text unit dependency")
}

/// Resolve exact stored spellings, including the shared image/text custom name.
/// No display symbol, abbreviation, font selector or ambient unit database is authority.
/// Pixel lengths require no physical dependency; physical lengths require validated axes.
pub fn resolve_size(size_bits: u64, unit_name: &str, resolution: Option<&PhysicalResolution>, control: &Interrupt<'_>) -> Result<TextUnitSize> {
    poll(control)?;
    if unit_name.is_empty() || unit_name.len() > 4096 || unit_name.contains('\0') {
        return Err(invalid());
    }
    let size = f64::from_bits(size_bits);
    if !size.is_finite() || size < 0.0 {
        return Err(invalid());
    }
    if unit_name == "pixels" {
        return Ok(TextUnitSize { x_pixels: size, y_pixels: size });
    }
    let resolution = resolution.ok_or_else(invalid)?;
    resolution.validate().map_err(|_| invalid())?;
    let builtin = match unit_name {
        "inches" => Some(PhysicalUnit::Inch),
        "millimeters" => Some(PhysicalUnit::Millimeter),
        "points" => Some(PhysicalUnit::Point),
        "picas" => Some(PhysicalUnit::Pica),
        _ => None,
    };
    let unit = if let Some(unit) = builtin.as_ref() {
        unit
    } else if let PhysicalUnit::Custom(custom) = &resolution.unit {
        if custom.name != unit_name {
            return Err(invalid());
        }
        &resolution.unit
    } else {
        return Err(invalid());
    };
    let x_pixels = physical_size::units_to_pixels(size, resolution, unit, Axis::X).map_err(|_| invalid())?;
    let y_pixels = physical_size::units_to_pixels(size, resolution, unit, Axis::Y).map_err(|_| invalid())?;
    poll(control)?;
    Ok(TextUnitSize { x_pixels, y_pixels })
}

fn field<'a>(layout: &'a [FilterArgument], name: &str, control: &Interrupt<'_>) -> Result<Option<&'a FilterValue>> {
    if layout.len() > 64 {
        return Err(TextSourceError::Budget);
    }
    let mut result = None;
    for argument in layout {
        poll(control)?;
        if argument.name.len() > 128 {
            return Err(TextSourceError::Budget);
        }
        if argument.name == name {
            if result.is_some() {
                return Err(invalid());
            }
            result = Some(&argument.value);
        }
    }
    Ok(result)
}
fn bits(value: &FilterValue) -> Result<u64> {
    match value {
        FilterValue::Number(ExactNumber::Float64Bits(bits)) => Ok(*bits),
        _ => Err(invalid()),
    }
}
fn name(value: &FilterValue) -> Result<&str> {
    match value {
        FilterValue::String(name) => Ok(name),
        _ => Err(invalid()),
    }
}

/// Resolve canonical borrowed layout fields without rewriting the original AST or cache.
/// Both box dimensions must be present together; each uses its own DPI axis.
/// Other layout fields are inert here and still require the renderer's own qualification.
pub fn resolve_layout(layout: &[FilterArgument], resolution: Option<&PhysicalResolution>, control: &Interrupt<'_>) -> Result<TextUnitLayout> {
    poll(control)?;
    let size = bits(field(layout, "font-size", control)?.ok_or_else(invalid)?)?;
    let unit = name(field(layout, "font-size-unit", control)?.ok_or_else(invalid)?)?;
    let font_size = resolve_size(size, unit, resolution, control)?;
    let width = field(layout, "box-width", control)?;
    let height = field(layout, "box-height", control)?;
    let box_size = match (width, height) {
        (None, None) => None,
        (Some(width), Some(height)) => {
            let unit = name(field(layout, "box-unit", control)?.ok_or_else(invalid)?)?;
            let width = resolve_size(bits(width)?, unit, resolution, control)?;
            let height = resolve_size(bits(height)?, unit, resolution, control)?;
            Some(TextUnitSize { x_pixels: width.x_pixels, y_pixels: height.y_pixels })
        }
        _ => return Err(invalid()),
    };
    poll(control)?;
    Ok(TextUnitLayout { font_size, box_size })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::CustomUnit;
    #[test]
    fn shared_unit_preserves_both_axes_without_font_authority() {
        let resolution = PhysicalResolution {
            x_dpi: 96.0,
            y_dpi: 120.0,
            unit: PhysicalUnit::Custom(CustomUnit {
                name: "Own shared text unit".into(),
                factor: 2.5,
                digits: 2,
                symbol: "tu".into(),
                abbreviation: "tun".into(),
            }),
        };
        let actual = resolve_size(0.3f64.to_bits(), "Own shared text unit", Some(&resolution), &Interrupt::NONE).unwrap();
        assert_eq!(actual.x_pixels, physical_size::units_to_pixels(0.3, &resolution, &resolution.unit, Axis::X).unwrap());
        assert_eq!(actual.y_pixels, physical_size::units_to_pixels(0.3, &resolution, &resolution.unit, Axis::Y).unwrap());
        assert_ne!(actual.x_pixels, actual.y_pixels);
        assert!(resolve_size(0.3f64.to_bits(), "tun", Some(&resolution), &Interrupt::NONE).is_err());
        assert!(resolve_size(0.3f64.to_bits(), "Own shared text unit", None, &Interrupt::NONE).is_err());
    }
    #[test]
    fn pixels_unknown_and_nonfinite_are_distinct() {
        assert_eq!(resolve_size(32f64.to_bits(), "pixels", None, &Interrupt::NONE).unwrap(), TextUnitSize { x_pixels: 32.0, y_pixels: 32.0 });
        assert!(resolve_size(32f64.to_bits(), "unknown", None, &Interrupt::NONE).is_err());
        assert!(resolve_size(f64::NAN.to_bits(), "pixels", None, &Interrupt::NONE).is_err());
    }
}
