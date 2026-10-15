//! Exact opt-in request and retained wire reply; neither grants filesystem access.
use super::{AutomationError, Value, bad};
use crate::headless::OwnedXcfOpenReply;

pub(super) enum RoutedReply {
    Legacy(Value),
    Owned(OwnedXcfOpenReply),
}
impl RoutedReply {
    pub(super) fn value(&self) -> &Value {
        match self {
            Self::Legacy(value) => value,
            Self::Owned(reply) => reply.value(),
        }
    }
}
pub(crate) fn validate_owned_path(path: &str) -> Result<(), AutomationError> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\0')
        || !std::path::Path::new(path).extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("xcf"))
    {
        return Err(bad("explicit xcfProfilePolicy=keepCurrent requires a bounded .xcf path"));
    }
    Ok(())
}
pub(super) fn parameters(params: &Value) -> Result<&str, AutomationError> {
    let object = params.as_object().ok_or_else(|| bad("explicit owned doc.open requires an object"))?;
    if object.len() != 2
        || object.keys().any(|key| !matches!(key.as_str(), "path" | "xcfProfilePolicy"))
        || object.get("xcfProfilePolicy").and_then(Value::as_str) != Some("keepCurrent")
    {
        return Err(bad("explicit owned doc.open accepts exactly path and xcfProfilePolicy=keepCurrent"));
    }
    let path = object.get("path").and_then(Value::as_str).ok_or_else(|| bad("explicit owned doc.open requires path string"))?;
    validate_owned_path(path)?;
    Ok(path)
}
// Transport-shape prepass only; the controlled constructor separately prepays
// the actual ID copy and charges its visits before source or derived allocation.
pub(super) fn validate_id_shape(value: &Value) -> Result<(), AutomationError> {
    fn walk(value: &Value, depth: usize, nodes: &mut usize, text: &mut usize) -> Result<(), AutomationError> {
        *nodes = nodes.checked_add(1).ok_or_else(|| bad("owned RPC ID node overflow"))?;
        if depth > 8 || *nodes > 4096 {
            return Err(bad("owned RPC ID depth/node limit exceeded"));
        }
        let add = |count: usize, text: &mut usize| -> Result<(), AutomationError> {
            *text = text.checked_add(count).ok_or_else(|| bad("owned RPC ID text overflow"))?;
            if *text > crate::security::MAX_REQUEST_BYTES {
                return Err(bad("owned RPC ID text limit exceeded"));
            }
            Ok(())
        };
        match value {
            Value::String(s) => add(s.len(), text)?,
            Value::Array(a) => {
                for v in a {
                    walk(v, depth + 1, nodes, text)?;
                }
            }
            Value::Object(o) => {
                for (k, v) in o {
                    add(k.len(), text)?;
                    walk(v, depth + 1, nodes, text)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, 0, &mut 0, &mut 0)
}

#[cfg(test)]
mod tests;
