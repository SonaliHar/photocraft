//! Borrowed request admission for the registered XCF metadata queries only.
//! This is not an edit whitelist or document/render qualification. Dispatch must
//! call it before copying params or modifying an implicit floating preview.
use crate::{EngineError, Result};
use serde_json::{Map, Value};

fn bad(command: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: command.into(), msg: message.into() }
}

fn object<'a>(command: &str, params: &'a Value, keys: &[&str]) -> Result<Option<&'a Map<String, Value>>> {
    // Dispatch treats null as {}. Borrow that empty envelope without copying a
    // possibly hostile value or allocating an empty map.
    if params.is_null() {
        return Ok(None);
    }
    let object = params.as_object().ok_or_else(|| bad(command, "query parameters must be an object or null"))?;
    if object.len() > keys.len() {
        return Err(bad(command, "too many query fields"));
    }
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(bad(command, "unknown query field"));
    }
    Ok(Some(object))
}

fn identity(command: &str, value: Option<&Value>) -> Result<()> {
    let text = value.and_then(Value::as_str).ok_or_else(|| bad(command, "query identity must be a canonical nonzero decimal U64 string"))?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad(command, "query identity must be a canonical nonzero decimal U64 string"));
    }
    // No string formatting or duplicate ID allocation is needed to prove the
    // canonical spelling: ASCII digits, no leading zero, and a checked parse.
    text.parse::<u64>().map_err(|_| bad(command, "query identity exceeds U64"))?;
    Ok(())
}

fn integer(command: &str, value: Option<&Value>, maximum: u64, positive: bool) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let number = value.as_u64().ok_or_else(|| bad(command, "query pagination must be an unsigned integer"))?;
    if number > maximum || (positive && number == 0) {
        return Err(bad(command, "query pagination is outside its supported range"));
    }
    Ok(())
}

/// `true` means a known query's borrowed envelope was admitted; `false` means
/// this module does not handle the command. Unknown commands/edits are untouched.
/// This admits no nested containers, payload strings, locators or grants.
pub(crate) fn dispatch_preflight(command: &str, params: &Value) -> Result<bool> {
    if command == "xcf.source.inspect" {
        crate::inspect_xcf_typed::selector(params)?;
        return Ok(true);
    }
    let keys: &[&str] = match command {
        "xcf.assess" => &[],
        "xcf.inspect" => &["offset", "limit"],
        "xcf.composition.inspect" => &["layerId"],
        "xcf.filter.inspect" => &["filterId"],
        "xcf.resource.inspect" => &["resource"],
        _ => return Ok(false),
    };
    let object = object(command, params, keys)?;
    let field = |key: &str| object.and_then(|object| object.get(key));
    match command {
        "xcf.inspect" => {
            integer(command, field("offset"), 100_000, false)?;
            integer(command, field("limit"), 128, true)?;
        }
        "xcf.composition.inspect" => identity(command, field("layerId"))?,
        "xcf.filter.inspect" => identity(command, field("filterId"))?,
        "xcf.resource.inspect" => {
            if let Some(resource) = field("resource") {
                identity(command, Some(resource))?;
            }
        }
        _ => {}
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_query_empty_and_existing_legal_envelopes_are_preserved() {
        for command in ["xcf.assess", "xcf.inspect", "xcf.resource.inspect"] {
            assert!(dispatch_preflight(command, &Value::Null).unwrap());
            assert!(dispatch_preflight(command, &json!({})).unwrap());
        }
        for (command, params) in [
            ("xcf.inspect", json!({"offset":100000,"limit":128})),
            ("xcf.composition.inspect", json!({"layerId":"18446744073709551615"})),
            ("xcf.filter.inspect", json!({"filterId":"9007199254740993"})),
            ("xcf.resource.inspect", json!({"resource":"1"})),
        ] {
            assert!(dispatch_preflight(command, &params).unwrap());
        }
    }

    #[test]
    fn hostile_known_fields_refuse_before_any_document_or_nested_walk() {
        for (command, params) in [
            ("xcf.assess", json!({"ignored":{"payload":"x".repeat(10000)}})),
            ("xcf.inspect", json!({"offset":-1})),
            ("xcf.inspect", json!({"offset":1.0})),
            ("xcf.inspect", json!({"limit":0})),
            ("xcf.inspect", json!({"limit":129})),
            ("xcf.inspect", json!({"offset":100001})),
            ("xcf.filter.inspect", Value::Null),
            ("xcf.filter.inspect", json!({"filterId":1})),
            ("xcf.filter.inspect", json!({"filterId":"01"})),
            ("xcf.composition.inspect", json!({"layerId":"18446744073709551616"})),
            ("xcf.resource.inspect", json!({"resource":[{"nested":"1"}]})),
            ("xcf.resource.inspect", json!({"resource":"1".repeat(10000)})),
        ] {
            assert!(matches!(dispatch_preflight(command, &params), Err(EngineError::BadParams { .. })));
        }
    }

    #[test]
    fn unrelated_queries_and_edits_are_not_given_an_xcf_global_whitelist() {
        let hostile = json!({"unbounded":{"fields":[1,2,3]}});
        for command in ["xcf.future.inspect", "xcf.link.recover", "xcf.itemSet.setMembers", "document.pixel.inspect"] {
            assert!(!dispatch_preflight(command, &hostile).unwrap());
        }
    }
}
