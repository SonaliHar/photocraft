//! Explicit held-root route for a host-only Load request in CLI `run`.
//! Ordinary CLI files and commands retain their trusted-local route.

use std::path::Path;

use photocraft_automation::{AuthorizedWorkspace, AutomationError};
use photocraft_engine::{Session, jobs::JobCtx};
use serde_json::Value;

fn special(id: &str, params: &Value) -> bool {
    id == "select.loadSelection" && params.get("originalPath").is_some()
}

/// Check every explicit original request before opening the CLI input document.
/// Only the explicit flag can supply a held capability; stored locators cannot.
pub(super) fn prepare(commands: &[(String, Value)], root: Option<&str>) -> Result<Option<AuthorizedWorkspace>, AutomationError> {
    let mut needed = false;
    for (id, params) in commands {
        if special(id, params) {
            photocraft_automation::validate_stored_selection_request(params, true)?;
            needed = true;
        }
    }
    let Some(root) = root else {
        return if needed {
            Err(AutomationError::BadRequest("select.loadSelection.originalPath: run requires --automation-read-root".into()))
        } else {
            Ok(None)
        };
    };
    if root.is_empty() || root.len() > 4096 || root.contains('\0') {
        return Err(AutomationError::BadRequest("run --automation-read-root requires a bounded explicit directory".into()));
    }
    AuthorizedWorkspace::new(Some(Path::new(root)), None).map(Some)
}

/// Return `None` for ordinary commands. The existing facade owns the same
/// JobCtx throughout preparation and the sole registered final execution.
pub(super) fn execute(session: &mut Session, workspace: Option<&AuthorizedWorkspace>, id: &str, params: &Value) -> Option<Result<Value, AutomationError>> {
    if !special(id, params) {
        return None;
    }
    let Some(workspace) = workspace else {
        return Some(Err(AutomationError::BadRequest("select.loadSelection.originalPath: run requires --automation-read-root".into())));
    };
    let context = JobCtx::new();
    Some(photocraft_automation::load_stored_selection_from_root_with_context(session, workspace, params, true, &context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Value {
        json!({"documentId":"1","expectedRevision":"0","channelId":"2","rawStorage":true,"storedSource":true,
            "originalPath":"original.xcf","originalOrdinal":"0","profilePolicy":"originalExact"})
    }

    #[test]
    fn missing_root_and_malformed_request_refuse_before_root_open() {
        let commands = vec![("select.loadSelection".into(), request())];
        assert!(prepare(&commands, None).err().unwrap().to_string().contains("--automation-read-root"));
        let mut malformed = request();
        malformed["documentId"] = json!("01");
        let commands = vec![("select.loadSelection".into(), malformed)];
        assert!(prepare(&commands, Some("missing-root-not-opened")).err().unwrap().to_string().contains("canonical decimal"));
    }

    #[test]
    fn ordinary_load_remains_on_existing_route_and_missing_offer_cannot_mutate() {
        let mut session = Session::new();
        assert!(execute(&mut session, None, "select.loadSelection", &json!({"channel":0})).is_none());
        assert!(execute(&mut session, None, "select.loadSelection", &request()).unwrap().is_err());
        assert!(session.documents().is_empty());
        assert!(prepare(&[("select.loadSelection".into(), json!({"channel":0}))], None).unwrap().is_none());
    }

    #[test]
    fn cli_special_shape_and_root_errors_precede_input_open() {
        let run = |params: Value, root: Option<&str>| {
            let mut args = vec![
                "run".to_owned(),
                "missing-input-not-opened.pcraft".into(),
                "--cmd".into(),
                "select.loadSelection".into(),
                "--params".into(),
                params.to_string(),
            ];
            if let Some(root) = root {
                args.extend(["--automation-read-root".into(), root.into()]);
            }
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = crate::run(&args, &mut out, &mut err);
            assert_ne!(code, 0);
            assert!(out.is_empty());
            String::from_utf8(err).unwrap()
        };
        assert!(run(request(), None).contains("run requires --automation-read-root"));
        let mut malformed = request();
        malformed["originalOrdinal"] = json!(0);
        assert!(run(malformed, Some("missing-root-not-opened")).contains("identities/revision/ordinal must be strings"));
        let mut malformed = request();
        malformed["extra"] = json!(true);
        assert!(run(malformed, Some("missing-root-not-opened")).contains("exact eight-field object"));
    }
}
