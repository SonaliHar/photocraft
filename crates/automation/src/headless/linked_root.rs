//! Explicit, immediate linked-image substitution beneath an authorized root.

use photocraft_doc::item_identity::ResourceId;
use photocraft_doc::LayerId;
use photocraft_engine::xcf_link_substitution_cmds::{capture_recovery_target, capture_target, request_for_recovery_target, request_for_target};
use photocraft_raster::Interrupt;
use serde_json::Value;

use super::{Filesystem, Headless};
use crate::AutomationError;

fn invalid(code: &str, message: &str) -> AutomationError {
    AutomationError::BadRequest(format!("{code}.request: {message}"))
}

fn decimal(value: &Value, key: &str, nonzero: bool, code: &str) -> Result<u64, AutomationError> {
    let text = value.get(key).and_then(Value::as_str).ok_or_else(|| invalid(code, "identities and revision must be strings"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(code, "invalid decimal identity or revision"));
    }
    let number = text.parse::<u64>().map_err(|_| invalid(code, "decimal identity or revision is out of range"))?;
    if (nonzero && number == 0) || number.to_string() != text {
        return Err(invalid(code, "identities and revision must be canonical decimal strings"));
    }
    Ok(number)
}

impl Headless {
    /// Read one explicitly requested replacement from the held read root, then
    /// dispatch the registered atomic command. The bytes never cross RPC.
    /// Capture starts at server entry; this is not a remote picker lease.
    pub fn substitute_xcf_link_from_root(&mut self, params: &Value) -> Result<Value, AutomationError> {
        self.replace_xcf_link_from_root(params, false)
    }

    /// Recover exact original bytes through the same held-root boundary.
    pub fn recover_xcf_link_from_root(&mut self, params: &Value) -> Result<Value, AutomationError> {
        self.replace_xcf_link_from_root(params, true)
    }

    fn replace_xcf_link_from_root(&mut self, params: &Value, exact: bool) -> Result<Value, AutomationError> {
        let code = if exact { "xcf.link.recover" } else { "xcf.link.substitute" };
        let object = params.as_object().ok_or_else(|| invalid(code, "expected an object"))?;
        const KEYS: [&str; 6] = ["documentId", "expectedRevision", "layerId", "resourceId", "path", "policy"];
        if object.len() != KEYS.len() || object.keys().any(|key| !KEYS.contains(&key.as_str())) {
            return Err(invalid(code, "expected exactly documentId, expectedRevision, layerId, resourceId, path and policy"));
        }
        let policy = if exact { "exact" } else { "substitute" };
        if params.get("policy").and_then(Value::as_str) != Some(policy) {
            return Err(invalid(code, "policy does not match the requested recovery method"));
        }
        let document = decimal(params, "documentId", true, code)?;
        let revision = decimal(params, "expectedRevision", false, code)?;
        let layer = LayerId(decimal(params, "layerId", true, code)?);
        let resource = ResourceId(decimal(params, "resourceId", true, code)?);
        let path = params.get("path").and_then(Value::as_str).ok_or_else(|| invalid(code, "path must be an explicit string"))?;
        if path.is_empty() || path.len() > 4096 {
            return Err(invalid(code, "path is empty or too long"));
        }
        let Filesystem::Workspace(workspace) = &self.filesystem else {
            return Err(AutomationError::BadRequest(format!("{code}.authority: an explicit read-root capability is required")));
        };
        let active = self.session.active().ok_or(photocraft_engine::EngineError::NoDocument)?;
        if active.doc.id.0 != document || active.revision != revision {
            return Err(AutomationError::BadRequest(format!("{code}.target: document identity or revision changed")));
        }
        // This also refuses pending/running jobs before any read or polling.
        if exact {
            let target = capture_recovery_target(&self.session, layer, resource)?;
            let bytes = workspace.read_xcf_linked_replacement(path, &Interrupt::NONE)?;
            let request = request_for_recovery_target(&self.session, &target, &bytes)?;
            Ok(self.session.execute("xcf.link.recover", request)?)
        } else {
            let target = capture_target(&self.session, layer, resource)?;
            let bytes = workspace.read_xcf_linked_replacement(path, &Interrupt::NONE)?;
            let request = request_for_target(&self.session, &target, &bytes)?;
            // No command_start/sync_jobs or fallible post-commit processing here.
            Ok(self.session.execute("xcf.link.substitute", request)?)
        }
    }
}
