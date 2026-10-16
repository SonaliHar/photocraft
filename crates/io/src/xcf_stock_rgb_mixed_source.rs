//! Mixed-only cheap2/2/10-field adapters. No kernel validation/preparation or gains.
use photocraft_xcf::{ArgumentValue, Effect};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Temperature,
    Sepia,
    AlienRgb,
    AlienHsl,
}
pub(crate) fn parsing_work(operation: &str) -> u64 {
    match operation {
        "gegl:color-temperature" | "gegl:sepia" => 512,
        "gegl:alien-map" => 4096,
        _ => 0,
    }
}
/// New mixed-only prospective fee, independent of the old mapper allowance.
pub(crate) fn mapping_bytes<P>(e: &Effect<P>) -> Option<usize> {
    if parsing_work(&e.operation) == 0 {
        return Some(0);
    }
    let n = e.arguments.len();
    if n > 10 {
        return None;
    }
    let count = n.checked_mul(2)?;
    let mut bytes = count
        .checked_mul(std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>())?
        .checked_add(count.checked_mul(std::mem::size_of::<f64>())?)?
        .checked_add(std::mem::size_of::<Vec<photocraft_doc::xcf_filters::FilterArgument>>())?
        .checked_add(std::mem::size_of::<Vec<f64>>())?
        .checked_add(64)?;
    for a in &e.arguments {
        bytes = bytes.checked_add(a.name.len().checked_mul(2)?)?;
    }
    Some(bytes)
}
fn three<T>(items: [Option<T>; 3]) -> Option<[T; 3]> {
    let [a, b, c] = items;
    Some([a?, b?, c?])
}
pub(crate) fn kind<P>(e: &Effect<P>) -> Option<Kind> {
    if e.version.as_deref() != Some("0:0") {
        return None;
    }
    let expected = match e.operation.as_str() {
        "gegl:color-temperature" | "gegl:sepia" => 2,
        "gegl:alien-map" => 10,
        _ => return None,
    };
    if e.arguments.len() != expected {
        return None;
    }
    if e.operation == "gegl:color-temperature" {
        let (mut original, mut intended) = (None, None);
        for a in &e.arguments {
            if a.configuration.is_some() || a.graph.is_some() {
                return None;
            }
            let ArgumentValue::Float32Bits(bits) = a.value else {
                return None;
            };
            let slot = match a.name.as_str() {
                "original-temperature" => &mut original,
                "intended-temperature" => &mut intended,
                _ => return None,
            };
            if slot.replace(bits).is_some() {
                return None;
            }
        }
        photocraft_compose::TemperatureParameters::from_words(original?, intended?).ok()?;
        return Some(Kind::Temperature);
    }
    if e.operation == "gegl:sepia" {
        let (mut scale, mut srgb) = (None, None);
        for a in &e.arguments {
            if a.configuration.is_some() || a.graph.is_some() {
                return None;
            }
            match (a.name.as_str(), &a.value) {
                ("scale", ArgumentValue::Float32Bits(bits)) if scale.is_none() => scale = Some(*bits),
                ("srgb", ArgumentValue::Boolean(value)) if srgb.is_none() => srgb = Some(*value),
                _ => return None,
            }
        }
        photocraft_compose::SepiaParameters::from_words(scale?, srgb?).ok()?;
        return Some(Kind::Sepia);
    }
    // Fixed10-field dispatch only. The HSL helper rechecks unique model1 and
    // the complete shape; never run an RGB constructor as a failed HSL probe.
    if e.arguments.iter().any(|a| a.name == "color-model" && matches!(a.value, ArgumentValue::Enum(1))) {
        super::mixed_alien_hsl_source::parameters(e)?;
        return Some(Kind::AlienHsl);
    }
    let (mut model, mut frequency, mut phase, mut keep) = (false, [None; 3], [None; 3], [None; 3]);
    for a in &e.arguments {
        if a.configuration.is_some() || a.graph.is_some() {
            return None;
        }
        match (a.name.as_str(), &a.value) {
            ("color-model", ArgumentValue::Enum(0)) if !model => model = true,
            (name, ArgumentValue::Float32Bits(bits)) => {
                let (target, index) = match name {
                    "cpn-1-frequency" => (&mut frequency, 0),
                    "cpn-2-frequency" => (&mut frequency, 1),
                    "cpn-3-frequency" => (&mut frequency, 2),
                    "cpn-1-phaseshift" => (&mut phase, 0),
                    "cpn-2-phaseshift" => (&mut phase, 1),
                    "cpn-3-phaseshift" => (&mut phase, 2),
                    _ => return None,
                };
                if target.get_mut(index)?.replace(*bits).is_some() {
                    return None;
                }
            }
            (name, ArgumentValue::Boolean(value)) => {
                let index = match name {
                    "cpn-1-keep" => 0,
                    "cpn-2-keep" => 1,
                    "cpn-3-keep" => 2,
                    _ => return None,
                };
                if keep.get_mut(index)?.replace(*value).is_some() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    if !model {
        return None;
    }
    photocraft_compose::AlienRgbParameters::from_words(three(frequency)?, three(phase)?, three(keep)?).ok()?;
    Some(Kind::AlienRgb)
}
