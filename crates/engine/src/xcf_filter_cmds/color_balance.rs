//! Closed exact-number wire schema for Color Balance tonal ranges.
use super::{ExactNumber, FilterArgument, FilterValue, Result, SET, Value, bad, bits64, json, object};
const RANGES: [&str; 3] = ["shadows", "midtones", "highlights"];
const AXES: [&str; 3] = ["cyan-red", "magenta-green", "yellow-blue"];

pub(super) fn decode(input: &Value) -> Result<FilterValue> {
    let outer = object(SET, input, &["Config"])?;
    let ranges = outer
        .get("Config")
        .and_then(Value::as_array)
        .filter(|ranges| ranges.len() == 3)
        .ok_or_else(|| bad(SET, "Color Balance requires three ordered ranges"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(3).map_err(|_| bad(SET, "Color Balance allocation denied"))?;
    for (range, name) in ranges.iter().zip(RANGES) {
        let range = object(SET, range, &["name", "value"])?;
        if range.get("name").and_then(Value::as_str) != Some(name) {
            return Err(bad(SET, "Color Balance range order"));
        }
        let value = object(SET, range.get("value").ok_or_else(|| bad(SET, "Color Balance range value missing"))?, &["Config"])?;
        let fields = value
            .get("Config")
            .and_then(Value::as_array)
            .filter(|fields| fields.len() == 3)
            .ok_or_else(|| bad(SET, "Color Balance requires three ordered axes"))?;
        let mut decoded = Vec::new();
        decoded.try_reserve_exact(3).map_err(|_| bad(SET, "Color Balance allocation denied"))?;
        for (field, name) in fields.iter().zip(AXES) {
            let field = object(SET, field, &["name", "value"])?;
            if field.get("name").and_then(Value::as_str) != Some(name) {
                return Err(bad(SET, "Color Balance axis order"));
            }
            let number = object(SET, field.get("value").ok_or_else(|| bad(SET, "Color Balance scalar missing"))?, &["Number"])?;
            let number = object(SET, number.get("Number").ok_or_else(|| bad(SET, "Color Balance exact number required"))?, &["Float64Bits"])?;
            let bits = bits64(number.get("Float64Bits").ok_or_else(|| bad(SET, "Color Balance Float64Bits missing"))?)?;
            if !(-1.0..=1.0).contains(&f64::from_bits(bits)) {
                return Err(bad(SET, "Color Balance axis outside -1..1"));
            }
            decoded.push(FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(bits)) });
        }
        output.push(FilterArgument { name: name.into(), value: FilterValue::Config(decoded) });
    }
    Ok(FilterValue::Config(output))
}
pub(super) fn wire(input: &FilterValue) -> Option<Value> {
    let FilterValue::Config(ranges) = input else { return None };
    if ranges.len() != 3 {
        return None;
    }
    let mut output = Vec::new();
    for (range, name) in ranges.iter().zip(RANGES) {
        let FilterValue::Config(fields) = &range.value else { return None };
        if range.name != name || fields.len() != 3 {
            return None;
        }
        let mut values = Vec::new();
        for (field, name) in fields.iter().zip(AXES) {
            let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field.value else { return None };
            if field.name != name || !f64::from_bits(bits).is_finite() || !(-1.0..=1.0).contains(&f64::from_bits(bits)) {
                return None;
            }
            values.push(json!({"name":name,"value":{"Number":{"Float64Bits":format!("0x{bits:016x}")}}}));
        }
        output.push(json!({"name":name,"value":{"Config":values}}));
    }
    Some(json!({"Config":output}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value {
        json!({"Config": RANGES.map(|name| json!({"name":name,"value":{"Config":AXES.map(|name| json!({"name":name,"value":{"Number":{"Float64Bits": "0x8000000000000000"}}}))}}))})
    }
    #[test]
    fn exact_wire_round_trip_preserves_negative_zero_and_rejects_malformed_shapes() {
        let value = input();
        assert_eq!(wire(&decode(&value).unwrap()).unwrap(), value);
        for variant in 0..8 {
            let mut value = input();
            match variant {
                0 => value["extra"] = json!(true),
                1 => {
                    value["Config"].as_array_mut().unwrap().pop();
                }
                2 => value["Config"][1]["name"] = json!("shadows"),
                3 => value["Config"][0]["value"]["Config"][1]["name"] = json!("cyan-red"),
                4 => value["Config"][0]["value"]["Config"][0]["value"] = json!({"Number":{"Float32Bits":"0x3f000000"}}),
                5 => value["Config"][0]["value"]["Config"][0]["value"] = json!({"Number":{"Float64Bits":"0x7ff8000000000000"}}),
                6 => value["Config"][0]["value"]["Config"][0]["value"] = json!({"Number":{"Float64Bits":"0x4000000000000000"}}),
                _ => value["Config"][0]["value"] = json!({"Config": []}),
            }
            assert!(decode(&value).is_err(), "variant{variant}");
        }
    }
}
