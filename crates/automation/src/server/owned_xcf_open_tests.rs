//! UNRUN MCP schema and genuine held-root owned-open workflows.
use super::*;
#[path = "../../../xcf/tests/common/mod.rs"]
mod fixture;
#[test]
fn exact_optional_policy_schema_does_not_accept_null_unknown_or_extra_fields() {
    assert!(serde_json::from_value::<OpenParams>(json!({"path":"plain.xcf"})).unwrap().xcf_profile_policy.is_none());
    let legacy = serde_json::from_value::<OpenParams>(json!({"path":"plain.xcf","legacyExtension":{"nested":[1,2,3]}})).unwrap();
    assert_eq!(legacy.path, "plain.xcf");
    assert!(legacy.xcf_profile_policy.is_none());
    // Both incoming key orders enforce the explicit-only restriction.
    assert!(serde_json::from_str::<OpenParams>(r#"{"extra":true,"path":"plain.xcf","xcfProfilePolicy":"keepCurrent"}"#).is_err());
    assert!(serde_json::from_str::<OpenParams>(r#"{"xcfProfilePolicy":"keepCurrent","path":"plain.xcf","extra":true}"#).is_err());
    assert!(serde_json::from_value::<OpenParams>(json!({"path":"plain.xcf","xcfProfilePolicy":"keepCurrent"})).unwrap().xcf_profile_policy.is_some());
    for value in [
        json!({"path":"plain.xcf","xcfProfilePolicy":null}),
        json!({"path":"plain.xcf","xcfProfilePolicy":"convert"}),
        json!({"path":"plain.xcf","xcfProfilePolicy":"keepCurrent","extra":true}),
    ] {
        assert!(serde_json::from_value::<OpenParams>(value).is_err());
    }
    let schema = serde_json::to_value(schemars::schema_for!(OpenParams)).unwrap();
    let field = &schema["properties"]["xcfProfilePolicy"];
    assert!(!field.to_string().contains("null"));
    assert_ne!(schema.get("additionalProperties"), Some(&Value::Bool(false)));
    assert_eq!(schema["allOf"][0]["if"]["required"], json!(["xcfProfilePolicy"]));
    assert_eq!(schema["allOf"][0]["then"]["additionalProperties"], false);
}
#[tokio::test]
async fn actual_mcp_held_root_returns_bounded_summary_and_denied_backend_has_no_tab() {
    let root = std::env::temp_dir().join(format!("photocraft-mcp-owned-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("plain.xcf"), fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
    let h = Arc::new(Mutex::new(Headless::with_workspace(AuthorizedWorkspace::new(Some(&root), None).unwrap())));
    let mcp = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
    let result = mcp.doc_open(Parameters(OpenParams { path: "plain.xcf".into(), xcf_profile_policy: Some(XcfProfilePolicy::KeepCurrent) })).await.unwrap();
    assert_ne!(result.is_error, Some(true));
    let wire = serde_json::to_value(result).unwrap();
    assert!(wire.to_string().contains("owned-import-summary"));
    let headless = h.lock().unwrap();
    assert!(headless.session.active().unwrap().doc.accounted().is_some());
    assert_eq!(headless.session.documents().len(), 1);
    drop(headless);
    let denied = Arc::new(Mutex::new(Headless::new()));
    let mcp = PhotocraftMcp::with_backend(Backend::Headless(denied.clone()));
    let result = mcp
        .doc_open(Parameters(OpenParams { path: root.join("plain.xcf").to_str().unwrap().into(), xcf_profile_policy: Some(XcfProfilePolicy::KeepCurrent) }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(denied.lock().unwrap().session.documents().is_empty());
    std::fs::remove_file(root.join("plain.xcf")).unwrap();
    std::fs::remove_dir(root).unwrap();
}
