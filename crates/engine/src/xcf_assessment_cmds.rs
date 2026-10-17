//! Reevaluate current import semantics without persisting authority or history.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_raster::Interrupt;
use serde_json::Value;

const ID: &str = "xcf.assess";
#[cfg(test)]
#[path = "../../io/tests/common/xcf.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    session.active().and_then(|state| state.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no imported XCF state".into())
}
/// Shared interruptible current-state assessment. Failed qualification produces
/// an error, rather than returning a cached successful import classification.
pub fn assess_with_interrupt(session: &Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    validate_params(params)?;
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    assess_document(doc, control)
}
fn validate_params(params: &Value) -> Result<()> {
    let params = params.as_object().ok_or_else(|| bad("expected an empty object"))?;
    if !params.is_empty() {
        return Err(bad("unknown assessment parameter"));
    }
    Ok(())
}
fn assess_document(doc: &photocraft_doc::Document, control: &Interrupt<'_>) -> Result<Value> {
    let assessment = photocraft_io::xcf_assessment::assess_xcf_document(doc, control)
        .map_err(|error| if matches!(error, photocraft_io::IoError::Cancelled) { EngineError::Cancelled } else { EngineError::Other(error.to_string()) })?
        .ok_or_else(|| bad("document has no retained XCF source state"))?;
    serde_json::to_value(assessment).map_err(|error| EngineError::Other(format!("XCF assessment serialization: {error}")))
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    validate_params(params)?;
    let snapshot = session.active().ok_or(EngineError::NoDocument)?.doc.clone();
    crate::jobs::run(
        session,
        "Assess Imported GIMP Project",
        true,
        move |context| context.stage(0., 1., "Qualifying current XCF state", |control| assess_document(&snapshot, control)),
        |_, report| Ok(report),
    )
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Assess Imported GIMP Project",
        menu: &[],
        shortcut: None,
        params: "{} -> current fidelity and bounded dependency diagnostics; current semantic qualification must succeed; grants no resource access",
        enabled,
        run,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> Session {
        let source = fixture::image(12, 150, 0, 1, 1, &[40, 60, 80, 255]);
        let doc = photocraft_io::import("current.xcf", &source).unwrap().document;
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn registered_current_query_does_not_change_document_revision_or_history() {
        let mut session = session();
        let before = session.active().unwrap().try_clone_resident().unwrap();
        let report = session.execute(ID, json!({})).unwrap();
        assert_eq!(report["fidelity"], "full");
        let after = session.active().unwrap();
        assert!(before.doc.shares_state(&after.doc));
        assert_eq!(before.revision, after.revision);
        assert_eq!(before.history.entries(&crate::read_control_none()).unwrap().values(), after.history.entries(&crate::read_control_none()).unwrap().values());
        assert_eq!(before.history.can_undo(), after.history.can_undo());
        assert_eq!(before.history.can_redo(), after.history.can_redo());
    }
    #[test]
    fn malformed_cancelled_or_unqualified_current_state_never_reports_success() {
        let mut session = session();
        for params in [Value::Null, json!([]), json!({"grant":true}), json!({"full":true})] {
            assert!(session.execute(ID, params).is_err());
        }
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(matches!(assess_with_interrupt(&session, &json!({}), &cancelled), Err(EngineError::Cancelled)));
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().provenance.format_version = 27;
        assert!(session.execute(ID, json!({})).is_err());
    }
}
