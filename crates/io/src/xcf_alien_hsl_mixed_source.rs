//! Mixed-only fixed model1 transport. No HSL math, prepare, gains, or RGB alias.
use photocraft_xcf::{ArgumentValue, Effect};
fn three<T>(v: [Option<T>; 3]) -> Option<[T; 3]> {
    let [a, b, c] = v;
    Some([a?, b?, c?])
}
pub(crate) fn parameters<P>(e: &Effect<P>) -> Option<photocraft_compose::AlienHslParameters> {
    if e.operation != "gegl:alien-map" || e.version.as_deref() != Some("0:0") || e.arguments.len() != 10 {
        return None;
    }
    let (mut model, mut frequency, mut phase, mut keep) = (false, [None; 3], [None; 3], [None; 3]);
    for a in &e.arguments {
        if a.configuration.is_some() || a.graph.is_some() {
            return None;
        }
        match (a.name.as_str(), &a.value) {
            ("color-model", ArgumentValue::Enum(1)) if !model => model = true,
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
    photocraft_compose::AlienHslParameters::from_words(three(frequency)?, three(phase)?, three(keep)?).ok()
}
