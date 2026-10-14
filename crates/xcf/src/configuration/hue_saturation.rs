//! Flat stateful-selector storage observed in independently saved neutral XCF:
//! source2333B SHA2567bdefad77a3b3511957b72744b34c888c5ceaa91b454dcabf862d9ad1804f086.
//! This parser preserves literal nonzero/signed-zero words in the same grammar;
//! it does not establish nonzero authored persistence or any evaluator/profile.
//! Original ConfigNodes, decimal spellings and raw spans remain unchanged.
use super::{fail, field, number};
use crate::{ConfigNode, ConfigValue, HueSaturationConfiguration, TypedConfiguration, XcfError, reader::Reader};

const SELECTORS: [&str; 7] = ["all", "red", "yellow", "green", "cyan", "blue", "magenta"];
enum ShapeError {
    Invalid,
    Interrupted(XcfError),
}
type ShapeResult<T> = Result<T, ShapeError>;
fn scalar(arguments: &[ConfigNode]) -> ShapeResult<u64> {
    if arguments.len() != 1 {
        return Err(ShapeError::Invalid);
    }
    let bits = number(arguments.first().ok_or(ShapeError::Invalid)?).ok_or(ShapeError::Invalid)?;
    if !f64::from_bits(bits).is_finite() {
        return Err(ShapeError::Invalid);
    }
    Ok(bits)
}
fn decode(nodes: &[ConfigNode], mut visit: impl FnMut() -> Result<(), XcfError>) -> ShapeResult<HueSaturationConfiguration> {
    // Exact fixed prospective structure; no unbounded nesting or allocation.
    if nodes.len() != 30 {
        return Err(ShapeError::Invalid);
    }
    visit().map_err(ShapeError::Interrupted)?;
    let (name, args) = field(nodes.first().ok_or(ShapeError::Invalid)?).ok_or(ShapeError::Invalid)?;
    if name != "time" {
        return Err(ShapeError::Invalid);
    }
    let time_bits = scalar(args)?;
    let mut ranges = [[0u64; 3]; 7];
    for (ordinal, selector) in SELECTORS.iter().enumerate() {
        visit().map_err(ShapeError::Interrupted)?;
        let start = ordinal.checked_mul(4).and_then(|i| i.checked_add(1)).ok_or(ShapeError::Invalid)?;
        let (name, args) = field(nodes.get(start).ok_or(ShapeError::Invalid)?).ok_or(ShapeError::Invalid)?;
        if name != "range" || args.len() != 1 {
            return Err(ShapeError::Invalid);
        }
        let ConfigValue::Atom(actual) = &args.first().ok_or(ShapeError::Invalid)?.value else {
            return Err(ShapeError::Invalid);
        };
        if actual.as_str() != *selector {
            return Err(ShapeError::Invalid);
        }
        let mut values = [None; 3];
        for offset in 1usize..=3 {
            visit().map_err(ShapeError::Interrupted)?;
            let index = start.checked_add(offset).ok_or(ShapeError::Invalid)?;
            let (name, args) = field(nodes.get(index).ok_or(ShapeError::Invalid)?).ok_or(ShapeError::Invalid)?;
            let component = match name {
                "hue" => 0,
                "saturation" => 1,
                "lightness" => 2,
                _ => return Err(ShapeError::Invalid),
            };
            let bits = scalar(args)?;
            let value = f64::from_bits(bits);
            if !(-1.0..=1.0).contains(&value) {
                return Err(ShapeError::Invalid);
            }
            let slot = values.get_mut(component).ok_or(ShapeError::Invalid)?;
            if slot.replace(bits).is_some() {
                return Err(ShapeError::Invalid);
            }
        }
        let [Some(hue), Some(saturation), Some(lightness)] = values else {
            return Err(ShapeError::Invalid);
        };
        *ranges.get_mut(ordinal).ok_or(ShapeError::Invalid)? = [hue, saturation, lightness];
    }
    visit().map_err(ShapeError::Interrupted)?;
    let (name, args) = field(nodes.get(29).ok_or(ShapeError::Invalid)?).ok_or(ShapeError::Invalid)?;
    if name != "overlap" {
        return Err(ShapeError::Invalid);
    }
    let overlap_bits = scalar(args)?;
    if !(0.0..=1.0).contains(&f64::from_bits(overlap_bits)) {
        return Err(ShapeError::Invalid);
    }
    Ok(HueSaturationConfiguration { time_bits: Some(time_bits), ranges, overlap_bits })
}
pub(super) fn typed(r: &mut Reader<'_, '_>, nodes: &[ConfigNode]) -> Result<TypedConfiguration, XcfError> {
    if nodes.len() != 30 {
        return Err(fail(r));
    }
    r.charge(std::mem::size_of::<HueSaturationConfiguration>())?;
    let result = decode(nodes, || r.poll());
    let config = match result {
        Ok(config) => config,
        Err(ShapeError::Invalid) => return Err(fail(r)),
        Err(ShapeError::Interrupted(error)) => return Err(error),
    };
    r.poll()?;
    Ok(TypedConfiguration::HueSaturation(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(value: ConfigValue) -> ConfigNode {
        ConfigNode { start: 11, end: 19, value }
    }
    fn numeric(name: &str, value: f64) -> ConfigNode {
        node(ConfigValue::List(vec![node(ConfigValue::Atom(name.into())), node(ConfigValue::Number { literal: value.to_string(), bits: value.to_bits() })]))
    }
    fn selector(name: &str) -> ConfigNode {
        node(ConfigValue::List(vec![node(ConfigValue::Atom("range".into())), node(ConfigValue::Atom(name.into()))]))
    }
    fn neutral() -> Vec<ConfigNode> {
        let mut nodes = vec![numeric("time", 0.)];
        for range in SELECTORS {
            nodes.push(selector(range));
            for field in ["hue", "saturation", "lightness"] {
                nodes.push(numeric(field, 0.));
            }
        }
        nodes.push(numeric("overlap", 0.));
        nodes
    }
    #[test]
    fn literal_same_grammar_nonzero_and_signed_zero_words_are_retained_without_defaults() {
        let mut nodes = neutral();
        nodes[0] = numeric("time", -0.);
        nodes[2] = numeric("hue", -0.);
        nodes[6] = numeric("hue", 0.125);
        nodes[7] = numeric("saturation", -0.25);
        nodes[8] = numeric("lightness", 0.5);
        nodes[29] = numeric("overlap", 0.75);
        let original = nodes.clone();
        let mut visits = 0;
        let config = decode(&nodes, || {
            visits += 1;
            Ok(())
        })
        .ok()
        .unwrap();
        assert_eq!(visits, 30);
        assert_eq!(config.time_bits, Some((-0f64).to_bits()));
        assert_eq!(config.ranges[0][0], (-0f64).to_bits());
        assert_eq!(config.ranges[1], [0.125f64.to_bits(), (-0.25f64).to_bits(), 0.5f64.to_bits()]);
        assert_eq!(config.overlap_bits, 0.75f64.to_bits());
        assert_eq!(nodes, original);
    }
    #[test]
    fn incomplete_duplicate_reordered_unknown_and_nested_blocks_refuse() {
        let base = neutral();
        let mut cases = Vec::new();
        let mut missing = base.clone();
        missing.remove(2);
        cases.push(missing);
        let mut duplicate = base.clone();
        duplicate[7] = numeric("hue", 0.);
        cases.push(duplicate);
        let mut ranges = base.clone();
        ranges[5] = selector("all");
        cases.push(ranges);
        let mut reordered = base.clone();
        reordered.swap(1, 5);
        cases.push(reordered);
        let mut unknown = base.clone();
        unknown[2] = numeric("unknown", 0.);
        cases.push(unknown);
        let mut nested = base.clone();
        nested[1] = node(ConfigValue::List(vec![selector("all")]));
        cases.push(nested);
        for nodes in cases {
            assert!(decode(&nodes, || Ok(())).is_err());
        }
    }
    #[test]
    fn hostile_native_scalar_domains_and_wrong_types_are_not_clamped_or_coerced() {
        for (index, value) in [(2, 1.0000001), (3, -1.0000001), (4, f64::INFINITY), (29, -0.01), (29, 1.01), (0, f64::NAN)] {
            let mut nodes = neutral();
            let field = match index {
                0 => "time",
                2 => "hue",
                3 => "saturation",
                4 => "lightness",
                _ => "overlap",
            };
            nodes[index] = numeric(field, value);
            assert!(decode(&nodes, || Ok(())).is_err());
        }
        let mut nodes = neutral();
        nodes[2] = node(ConfigValue::List(vec![node(ConfigValue::Atom("hue".into())), node(ConfigValue::Atom("0".into()))]));
        assert!(decode(&nodes, || Ok(())).is_err());
    }
    #[test]
    fn bounded_parser_cancellation_is_not_changed_into_an_unsupported_default() {
        let nodes = neutral();
        let mut visits = 0;
        let result = decode(&nodes, || {
            visits += 1;
            if visits == 8 { Err(XcfError::at(crate::ErrorKind::Cancelled, 17, "cancelled")) } else { Ok(()) }
        });
        assert!(matches!(result,Err(ShapeError::Interrupted(error)) if error.kind==crate::ErrorKind::Cancelled));
        assert_eq!(visits, 8);
    }
}
