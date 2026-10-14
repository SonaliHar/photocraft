//! UNRUN native service-routing drafts; no synthetic source/correspondence grant.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn inner() -> Value {
    json!({"documentId":"1","expectedRevision":"0","channelId":"2","rawStorage":true,"storedSource":true,"originalPath":"originals/own.xcf","originalOrdinal":"0","profilePolicy":"originalExact"})
}
fn call(app: &mut PhotocraftApp, params: Value) -> Value {
    let (request, _) = ControlRequest::new("engine.execute", params);
    match handle(app, &egui::Context::default(), &request) {
        Outcome::Done(value) => value,
        _ => panic!("stored original routing must settle synchronously"),
    }
}
#[test]
fn absent_service_and_no_wait_never_run_ordinary_load_or_create_offer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let value = call(&mut app, json!({"command":"select.loadSelection","params":inner()}));
    assert!(value["error"].as_str().unwrap().contains("held read-root service unavailable"));
    let value = call(&mut app, json!({"command":"select.loadSelection","params":inner(),"wait":false}));
    assert!(value["error"].as_str().unwrap().contains("wait=true"));
    assert!(app.session.documents().is_empty());
}
#[test]
fn borrowed_host_service_receives_exact_request_and_ordinary_load_never_uses_it() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let services = crate::Services {
        automation_stored_selection: Some(Box::new(move |session, params, wait, context| {
            count.fetch_add(1, Ordering::Relaxed);
            assert!(wait);
            assert!(!context.cancelled());
            assert_eq!(params, &inner());
            assert!(session.documents().is_empty());
            Err("Own held-root denial".into()) // no fabricated successful edit
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let value = call(&mut app, json!({"command":"select.loadSelection","params":inner()}));
    assert!(value["error"].as_str().unwrap().contains("Own held-root denial"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let value = call(&mut app, json!({"command":"select.loadSelection","params":{"channelId":"2","rawStorage":true}}));
    assert_eq!(value["ok"], false);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let value = call(&mut app, json!({"command":"select.loadSelection","params":inner(),"wait":"true"}));
    assert!(value["error"].as_str().unwrap().contains("boolean"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn box_only_policy_denial_precedes_host_callback_and_keeps_session() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let services = crate::Services {
        automation_authorize: None,
        automation_command: Some(Box::new(|id, request| {
            assert_eq!(id, "select.loadSelection");
            assert_eq!(request, &inner());
            Err("Own Box-only denial".into())
        })),
        automation_stored_selection: Some(Box::new(move |_, _, _, _| {
            count.fetch_add(1, Ordering::Relaxed);
            Err("callback must not be reached".into())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let response = call(&mut app, json!({"command":"select.loadSelection","params":inner()}));
    assert!(response["error"].as_str().unwrap().contains("Own Box-only denial"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(app.session.documents().is_empty());
    assert!(app.session.authorize.is_none());
}
