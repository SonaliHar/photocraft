//! Exact mixed-only scalar identities; no singleton candidate substitution.
use photocraft_xcf::{ArgumentValue, Effect};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Parameters {
    BrightnessContrast([u32; 2]),
    Levels([u32; 4]),
}
pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<Parameters> {
    if effect.version.as_deref() != Some("0:0") {
        return None;
    }
    let names: &[&str] = match effect.operation.as_str() {
        "gegl:brightness-contrast" => &["brightness", "contrast"],
        "gegl:levels" => &["in-low", "in-high", "out-low", "out-high"],
        _ => return None,
    };
    if effect.arguments.len() != names.len() {
        return None;
    }
    let mut words = [None; 4];
    for a in &effect.arguments {
        if a.configuration.is_some() || a.graph.is_some() {
            return None;
        }
        let ArgumentValue::Float32Bits(bits) = a.value else {
            return None;
        };
        let index = names.iter().position(|n| *n == a.name)?;
        let slot = words.get_mut(index)?;
        if slot.replace(bits).is_some() {
            return None;
        }
    }
    use photocraft_algo::gimp::point::PointOperation;
    if names.len() == 2 {
        let raw = [words[0]?, words[1]?];
        PointOperation::BrightnessContrast { brightness: f64::from(f32::from_bits(raw[0])), contrast: f64::from(f32::from_bits(raw[1])) }.validate().ok()?;
        Some(Parameters::BrightnessContrast(raw))
    } else {
        let raw = [words[0]?, words[1]?, words[2]?, words[3]?];
        PointOperation::Levels {
            input_low: f64::from(f32::from_bits(raw[0])),
            input_high: f64::from(f32::from_bits(raw[1])),
            output_low: f64::from(f32::from_bits(raw[2])),
            output_high: f64::from(f32::from_bits(raw[3])),
        }
        .validate()
        .ok()?;
        Some(Parameters::Levels(raw))
    }
}
