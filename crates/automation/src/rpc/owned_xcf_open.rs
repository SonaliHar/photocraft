//! Exact opt-in request and retained wire reply; neither grants filesystem access.
use super::{AutomationError, Value, bad};
use crate::headless::OwnedXcfOpenReply;

pub(super) enum RoutedReply {
    Legacy(Value),
    Owned(OwnedXcfOpenReply),
    FilterInspect(photocraft_engine::accounted_filter_inspect::OwnedFilterInspectWire),
}
impl RoutedReply {
    // Existing transport drafts inspect their replies without adding a
    // production Value escape to the paid wire-only variant.
    #[cfg(test)]
    pub(super) fn value(&self) -> Value {
        match self {
            Self::Legacy(value) => value.clone(),
            Self::Owned(reply) => reply.value().clone(),
            Self::FilterInspect(reply) => serde_json::from_slice(reply.bytes()).unwrap(),
        }
    }
    pub(super) fn write_line_to(&self, output: &mut impl std::io::Write) -> std::io::Result<()> {
        match self {
            Self::Legacy(value) => crate::budgets::write_reply(output, value),
            Self::Owned(reply) => crate::budgets::write_reply(output, reply.value()),
            Self::FilterInspect(reply) => reply.write_line_to(output),
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
pub(crate) fn validate_owned_native_path(path: &str) -> Result<(), AutomationError> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\0')
        || !std::path::Path::new(path).extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("pcraft"))
    {
        return Err(bad("explicit nativeProfilePolicy=keepCurrent requires a bounded .pcraft path"));
    }
    Ok(())
}
pub(super) enum OpenRequest<'a> {
    Xcf(&'a str),
    Native(&'a str),
}
pub(super) fn parameters(params: &Value) -> Result<OpenRequest<'_>, AutomationError> {
    let object = params.as_object().ok_or_else(|| bad("explicit owned doc.open requires an object"))?;
    let xcf = object.contains_key("xcfProfilePolicy");
    let native = object.contains_key("nativeProfilePolicy");
    if xcf == native || object.len() != 2 || object.keys().any(|key| !matches!(key.as_str(), "path" | "xcfProfilePolicy" | "nativeProfilePolicy")) {
        return Err(bad("explicit owned doc.open requires path and exactly one profile policy"));
    }
    let policy = if native { "nativeProfilePolicy" } else { "xcfProfilePolicy" };
    if object.get(policy).and_then(Value::as_str) != Some("keepCurrent") {
        return Err(bad("explicit owned Open profile policy must be keepCurrent"));
    }
    let path = object.get("path").and_then(Value::as_str).ok_or_else(|| bad("explicit owned doc.open requires path string"))?;
    if native {
        validate_owned_native_path(path)?;
        Ok(OpenRequest::Native(path))
    } else {
        validate_owned_path(path)?;
        Ok(OpenRequest::Xcf(path))
    }
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
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;

#[cfg(test)]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod native_tests;
