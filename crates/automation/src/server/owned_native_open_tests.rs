//! UNRUN MCP native policy schema, actual held-root archive and authority denial.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
#[path = "../../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
#[test]
fn native_schema_is_optional_exact_and_mutually_exclusive_without_narrowing_legacy_extras() {
    assert!(serde_json::from_value::<OpenParams>(json!({"path":"current.pcraft","legacy":true})).unwrap().native_profile_policy.is_none());
    assert!(
        serde_json::from_value::<OpenParams>(json!({"path":"current.pcraft","nativeProfilePolicy":"keepCurrent"})).unwrap().native_profile_policy.is_some()
    );
    for value in [
        json!({"path":"current.pcraft","nativeProfilePolicy":null}),
        json!({"path":"current.pcraft","nativeProfilePolicy":"convert"}),
        json!({"path":"current.pcraft","nativeProfilePolicy":"keepCurrent","xcfProfilePolicy":"keepCurrent"}),
        json!({"path":"current.pcraft","nativeProfilePolicy":"keepCurrent","extra":true}),
    ] {
        assert!(serde_json::from_value::<OpenParams>(value).is_err());
    }
    let schema = serde_json::to_value(schemars::schema_for!(OpenParams)).unwrap();
    assert!(!schema["properties"]["nativeProfilePolicy"].to_string().contains("null"));
    assert_eq!(schema["allOf"][1]["if"]["required"], json!(["nativeProfilePolicy"]));
    assert_eq!(schema["allOf"][1]["then"]["additionalProperties"], false);
}
#[tokio::test]
async fn mcp_native_reopen_consumes_real_archive_but_never_acquires_absent_authority() {
    static N: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!("photocraft-mcp-native-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("original.xcf");
    std::fs::write(&source, fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
    let mut writer = Headless::trusted_local();
    drop(writer.open_owned_xcf_keep_current_bounded(&source).unwrap());
    writer.save(None, Some(&root.join("current.pcraft")), None, &Default::default()).unwrap();
    std::fs::remove_file(source).unwrap();
    let h = Arc::new(Mutex::new(Headless::with_workspace(AuthorizedWorkspace::new(Some(&root), None).unwrap())));
    let owned_mcp = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
    let reply = owned_mcp
        .doc_open(Parameters(OpenParams {
            path: "current.pcraft".into(),
            xcf_profile_policy: None,
            native_profile_policy: Some(NativeProfilePolicy::KeepCurrent),
        }))
        .await
        .unwrap();
    assert_ne!(reply.is_error, Some(true));
    assert!(serde_json::to_value(reply).unwrap().to_string().contains("owned-native-summary"));
    assert!(h.lock().unwrap().session.active().unwrap().doc.accounted().is_some());
    let denied = Arc::new(Mutex::new(Headless::new()));
    let mcp = PhotocraftMcp::with_backend(Backend::Headless(denied.clone()));
    let reply = mcp
        .doc_open(Parameters(OpenParams {
            path: root.join("current.pcraft").to_str().unwrap().into(),
            xcf_profile_policy: None,
            native_profile_policy: Some(NativeProfilePolicy::KeepCurrent),
        }))
        .await
        .unwrap();
    assert_eq!(reply.is_error, Some(true));
    assert!(denied.lock().unwrap().session.documents().is_empty());
    drop(owned_mcp);
    drop(h);
    drop(mcp);
    drop(denied);
    drop(writer);
    std::fs::remove_file(root.join("current.pcraft")).unwrap();
    std::fs::remove_dir(root).unwrap();
}
