//! Lossless, bounded scalar wire DTOs; this does not select an evaluator or grant.
use photocraft_doc::{
    SampleEncoding,
    item_identity::ResourceId,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
};
use serde_json::{Map, Value};

type Result<T> = std::result::Result<T, String>;
const TOTAL_BYTES: usize = 16 << 10;
const MAX_NODES: usize = 1024;
const MAX_DEPTH: usize = 8;
const MAX_ARGS: usize = 64;
const MAX_COMPONENTS: usize = 16;

fn record<'a>(value: &'a Value, keys: &[&str], required: bool) -> Result<&'a Map<String, Value>> {
    let object = value.as_object().ok_or("filter wire record must be an object")?;
    if object.len() > keys.len() || object.keys().any(|key| !keys.contains(&key.as_str())) || (required && object.len() != keys.len()) {
        return Err("unknown or missing filter wire field".into());
    }
    Ok(object)
}
fn field<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    object.get(key).ok_or_else(|| "missing filter wire field".into())
}
fn text(value: &Value, maximum: usize) -> Result<&str> {
    value.as_str().filter(|text| text.len() <= maximum && !text.contains('\0')).ok_or_else(|| "filter wire text limit/type".into())
}
fn unsigned(value: &Value) -> Result<u64> {
    if let Some(text) = value.as_str() {
        if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("canonical unsigned decimal string required".into());
        }
        text.parse::<u64>().map_err(|_| "unsigned integer overflow".into())
    } else {
        value.as_u64().ok_or_else(|| "exact unsigned JSON integer or canonical string required".into())
    }
}
fn signed(value: &Value) -> Result<i64> {
    if let Some(text) = value.as_str() {
        let digits = text.strip_prefix('-').unwrap_or(text);
        if text.is_empty()
            || text.len() > 20
            || digits.is_empty()
            || (digits.len() > 1 && digits.starts_with('0'))
            || text == "-0"
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("canonical signed decimal string required".into());
        }
        text.parse::<i64>().map_err(|_| "signed integer overflow".into())
    } else {
        value.as_i64().ok_or_else(|| "exact signed JSON integer or canonical string required".into())
    }
}
fn number(value: &Value) -> Result<ExactNumber> {
    let object = record(value, &["Signed", "Unsigned", "Float32Bits", "Float64Bits"], false)?;
    if object.len() != 1 {
        return Err("exact number requires one variant".into());
    }
    if let Some(value) = object.get("Float32Bits") {
        return unsigned(value).and_then(|bits| u32::try_from(bits).map(ExactNumber::Float32Bits).map_err(|_| "Float32Bits overflow".into()));
    }
    if let Some(value) = object.get("Float64Bits") {
        return unsigned(value).map(ExactNumber::Float64Bits);
    }
    let (value, is_signed) = match (object.get("Signed"), object.get("Unsigned")) {
        (Some(value), None) => (value, true),
        (None, Some(value)) => (value, false),
        _ => return Err("invalid exact number variant".into()),
    };
    let object = record(value, &["width", "value"], true)?;
    let width = u8::try_from(unsigned(field(object, "width")?)?).map_err(|_| "integer width overflow")?;
    if is_signed {
        let value = signed(field(object, "value")?)?;
        let fits = match width {
            8 => i8::try_from(value).is_ok(),
            16 => i16::try_from(value).is_ok(),
            32 => i32::try_from(value).is_ok(),
            64 => true,
            _ => false,
        };
        if !fits {
            return Err("signed integer does not fit supported declared width".into());
        }
        Ok(ExactNumber::Signed { width, value })
    } else {
        let value = unsigned(field(object, "value")?)?;
        let fits = match width {
            8 => u8::try_from(value).is_ok(),
            16 => u16::try_from(value).is_ok(),
            32 => u32::try_from(value).is_ok(),
            64 => true,
            _ => false,
        };
        if !fits {
            return Err("unsigned integer does not fit supported declared width".into());
        }
        Ok(ExactNumber::Unsigned { width, value })
    }
}
fn encoding(value: &Value) -> Result<SampleEncoding> {
    match value.as_str() {
        Some("linear") => Ok(SampleEncoding::Linear),
        Some("nonlinear_profile") => Ok(SampleEncoding::NonlinearProfile),
        Some("perceptual") => Ok(SampleEncoding::Perceptual),
        _ => Err("unknown exact sample encoding".into()),
    }
}
fn profile(value: &Value) -> Result<Option<ResourceId>> {
    if value.is_null() {
        return Ok(None);
    }
    let id = unsigned(value)?;
    if id == 0 {
        return Err("profile identity must be nonzero".into());
    }
    Ok(Some(ResourceId(id)))
}
fn scalar_preflight(value: &Value) -> Result<()> {
    if value.as_object().is_some_and(|object| object.contains_key("StoredColor")) {
        return Err("StoredColor editing is unsupported without an exact stored-color schema".into());
    }
    let outer = record(value, &["Number", "Bool", "String", "Enum", "Color"], false)?;
    if outer.len() != 1 {
        return Err("filter value requires one supported scalar variant".into());
    }
    if let Some(value) = outer.get("Number") {
        number(value)?;
        return Ok(());
    }
    if let Some(value) = outer.get("Bool") {
        value.as_bool().ok_or("Bool needs a boolean")?;
        return Ok(());
    }
    if let Some(value) = outer.get("String") {
        text(value, 4096)?;
        return Ok(());
    }
    if let Some(value) = outer.get("Enum") {
        let object = record(value, &["domain", "value"], true)?;
        text(field(object, "domain")?, 256)?;
        i32::try_from(signed(field(object, "value")?)?).map_err(|_| "enum integer overflow")?;
        return Ok(());
    }
    let object = record(field(outer, "Color")?, &["components", "encoding", "profile"], true)?;
    let components =
        field(object, "components")?.as_array().filter(|values| !values.is_empty() && values.len() <= MAX_COMPONENTS).ok_or("Color component limit/type")?;
    for value in components {
        number(value)?;
    }
    encoding(field(object, "encoding")?)?;
    profile(field(object, "profile")?)?;
    Ok(())
}
fn operation_preflight(value: &Value) -> Result<()> {
    let object = record(value, &["name", "stored_version", "schema_epoch"], true)?;
    text(field(object, "name")?, 256)?;
    text(field(object, "stored_version")?, 64)?;
    text(field(object, "schema_epoch")?, 256)?;
    Ok(())
}
fn arguments_preflight(value: &Value) -> Result<()> {
    let arguments = value.as_array().filter(|args| args.len() <= MAX_ARGS).ok_or("at most 64 filter arguments required")?;
    for (index, argument) in arguments.iter().enumerate() {
        let object = record(argument, &["name", "value"], true)?;
        let name = text(field(object, "name")?, 128)?;
        if name.is_empty() {
            return Err("argument name cannot be empty".into());
        }
        // At most 64 borrowed names: duplicate refusal requires no allocation.
        for previous in arguments.iter().take(index) {
            if previous.get("name").and_then(Value::as_str) == Some(name) {
                return Err("duplicate filter argument name".into());
            }
        }
        scalar_preflight(field(object, "value")?)?;
    }
    Ok(())
}
struct Budget {
    nodes: usize,
    bytes: usize,
}
impl Budget {
    fn add(&mut self, amount: usize) -> Result<()> {
        self.bytes = self.bytes.checked_add(amount).filter(|n| *n <= TOTAL_BYTES).ok_or("filter wire aggregate byte limit")?;
        Ok(())
    }
    /// A conservative JSON byte bound (escaping included), not physical RSS.
    /// All trees have already passed the closed scalar schema before this walk.
    fn value(&mut self, value: &Value, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err("filter wire depth limit".into());
        }
        self.nodes = self.nodes.checked_add(1).filter(|n| *n <= MAX_NODES).ok_or("filter wire node limit")?;
        match value {
            Value::Null | Value::Bool(_) => self.add(5)?,
            Value::Number(_) => self.add(32)?,
            Value::String(text) => self.add(text.len().checked_mul(6).and_then(|n| n.checked_add(2)).ok_or("filter wire byte overflow")?)?,
            Value::Array(values) => {
                self.add(values.len().checked_add(2).ok_or("filter wire byte overflow")?)?;
                for value in values {
                    self.value(value, depth + 1)?;
                }
            }
            Value::Object(object) => {
                self.add(2)?;
                for (key, value) in object {
                    self.add(key.len().checked_mul(6).and_then(|n| n.checked_add(4)).ok_or("filter wire byte overflow")?)?;
                    self.value(value, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}
pub fn preflight_operation(value: &Value) -> Result<()> {
    operation_preflight(value)?;
    Budget { nodes: 0, bytes: 0 }.value(value, 0)
}
pub fn preflight_arguments(value: &Value) -> Result<()> {
    arguments_preflight(value)?;
    Budget { nodes: 0, bytes: 0 }.value(value, 0)
}
/// Use this joint admission before cloning an insertion request: the operation
/// and arguments share one byte/node budget rather than two independent caps.
pub fn preflight_filter(operation: &Value, arguments: &Value) -> Result<()> {
    operation_preflight(operation)?;
    arguments_preflight(arguments)?;
    let mut budget = Budget { nodes: 0, bytes: 0 };
    budget.value(operation, 0)?;
    budget.value(arguments, 0)
}
fn owned(value: &str) -> Result<String> {
    let mut output = String::new();
    output.try_reserve_exact(value.len()).map_err(|_| "filter wire allocation")?;
    output.push_str(value);
    Ok(output)
}
pub fn decode_operation(value: &Value) -> Result<OperationIdentity> {
    preflight_operation(value)?;
    let object = record(value, &["name", "stored_version", "schema_epoch"], true)?;
    Ok(OperationIdentity {
        name: owned(text(field(object, "name")?, 256)?)?,
        stored_version: owned(text(field(object, "stored_version")?, 64)?)?,
        schema_epoch: owned(text(field(object, "schema_epoch")?, 256)?)?,
    })
}
fn decode_scalar(value: &Value) -> Result<FilterValue> {
    if value.as_object().is_some_and(|object| object.contains_key("StoredColor")) {
        return Err("StoredColor editing is unsupported without an exact stored-color schema".into());
    }
    let outer = record(value, &["Number", "Bool", "String", "Enum", "Color"], false)?;
    if let Some(value) = outer.get("Number") {
        return number(value).map(FilterValue::Number);
    }
    if let Some(value) = outer.get("Bool") {
        return value.as_bool().map(FilterValue::Bool).ok_or_else(|| "Bool needs a boolean".into());
    }
    if let Some(value) = outer.get("String") {
        return owned(text(value, 4096)?).map(FilterValue::String);
    }
    if let Some(value) = outer.get("Enum") {
        let object = record(value, &["domain", "value"], true)?;
        return Ok(FilterValue::Enum {
            domain: owned(text(field(object, "domain")?, 256)?)?,
            value: i32::try_from(signed(field(object, "value")?)?).map_err(|_| "enum integer overflow")?,
        });
    }
    let object = record(field(outer, "Color")?, &["components", "encoding", "profile"], true)?;
    let values = field(object, "components")?.as_array().ok_or("Color components must be an array")?;
    let mut components = Vec::new();
    components.try_reserve_exact(values.len()).map_err(|_| "filter component allocation")?;
    for value in values {
        components.push(number(value)?);
    }
    Ok(FilterValue::Color { components, encoding: encoding(field(object, "encoding")?)?, profile: profile(field(object, "profile")?)? })
}
pub fn decode_arguments(value: &Value) -> Result<Vec<FilterArgument>> {
    preflight_arguments(value)?;
    let arguments = value.as_array().ok_or("arguments must be an array")?;
    let mut output = Vec::new();
    output.try_reserve_exact(arguments.len()).map_err(|_| "filter argument allocation")?;
    for value in arguments {
        let object = record(value, &["name", "value"], true)?;
        output.push(FilterArgument { name: owned(text(field(object, "name")?, 128)?)?, value: decode_scalar(field(object, "value")?)? });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    #[test]
    fn stored_color_wire_cannot_grant_an_editing_schema() {
        let value = serde_json::json!([{"name":"color","value":{"StoredColor":{"layout":"HSL float","samples":[0,0,0,128],"opaque_tail":[]}}}]);
        assert!(preflight_arguments(&value).unwrap_err().contains("StoredColor editing is unsupported"));
        assert!(decode_arguments(&value).unwrap_err().contains("StoredColor editing is unsupported"));
    }
    use super::*;
    use serde_json::json;
    #[test]
    fn decimal_bits_and_integers_decode_without_f64_or_signedness_loss() {
        let values = json!([
            {"name":"negative-zero","value":{"Number":{"Float64Bits":"9223372036854775808"}}},
            {"name":"nan-payload","value":{"Number":{"Float64Bits":"18446744073709551615"}}},
            {"name":"float32","value":{"Number":{"Float32Bits":"4294967295"}}},
            {"name":"signed","value":{"Number":{"Signed":{"width":"64","value":"-9223372036854775808"}}}},
            {"name":"unsigned","value":{"Number":{"Unsigned":{"width":64,"value":18446744073709551615_u64}}}}
        ]);
        let decoded = decode_arguments(&values).unwrap();
        assert_eq!(decoded[0].value, FilterValue::Number(ExactNumber::Float64Bits(0x8000000000000000)));
        assert_eq!(decoded[1].value, FilterValue::Number(ExactNumber::Float64Bits(u64::MAX)));
        assert_eq!(decoded[2].value, FilterValue::Number(ExactNumber::Float32Bits(u32::MAX)));
        assert_eq!(decoded[3].value, FilterValue::Number(ExactNumber::Signed { width: 64, value: i64::MIN }));
        assert_eq!(decoded[4].value, FilterValue::Number(ExactNumber::Unsigned { width: 64, value: u64::MAX }));
        assert_eq!(decode_arguments(&serde_json::to_value(&decoded).unwrap()).unwrap(), decoded);
    }
    #[test]
    fn wrong_types_nested_families_noncanonical_and_size_overflows_refuse() {
        for number in [
            json!({"Float64Bits":1.0}),
            json!({"Float64Bits":"01"}),
            json!({"Float64Bits":"+1"}),
            json!({"Float64Bits":"18446744073709551616"}),
            json!({"Float32Bits":"4294967296"}),
            json!({"Signed":{"width":8,"value":"128"}}),
            json!({"Unsigned":{"width":8,"value":"256"}}),
        ] {
            assert!(preflight_arguments(&json!([{"name":"x","value":{"Number":number}}])).is_err());
        }
        for value in [json!({"Graph":{"nodes":[]}}), json!({"Config":[]}), json!({"Resource":"1"}), json!({"Bytes":[]}), json!({"Curve":{}})] {
            assert!(preflight_arguments(&json!([{"name":"x","value":value}])).is_err());
        }
        assert!(preflight_arguments(&json!(vec![json!({"name":"x","value":{"Bool":true}}); 65])).is_err());
        assert!(preflight_arguments(&json!([{"name":"x","value":{"String":"x".repeat(4097)}}])).is_err());
        assert!(preflight_operation(&json!({"name":"x".repeat(257),"stored_version":"0:0","schema_epoch":"same"})).is_err());
    }
    #[test]
    fn colors_enums_and_same_identity_roundtrip_preserve_explicit_context() {
        let operation = json!({"name":"gegl:own","stored_version":"0:0","schema_epoch":"own-known-epoch"});
        let values = json!([{"name":"enum","value":{"Enum":{"domain":"own","value":"-2147483648"}}},
            {"name":"color","value":{"Color":{"components":[{"Float32Bits":"2147483648"},{"Float32Bits":1065353216}],"encoding":"linear","profile":null}}}]);
        preflight_filter(&operation, &values).unwrap();
        let identity = decode_operation(&operation).unwrap();
        assert_eq!(serde_json::to_value(&identity).unwrap(), operation);
        let decoded = decode_arguments(&values).unwrap();
        assert_eq!(decode_arguments(&serde_json::to_value(&decoded).unwrap()).unwrap(), decoded);
        let duplicate = json!([{"name":"same","value":{"Bool":true}},{"name":"same","value":{"Bool":false}}]);
        assert!(preflight_arguments(&duplicate).is_err());
    }
}
