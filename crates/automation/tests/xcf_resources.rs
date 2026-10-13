use photocraft_automation::AuthorizedWorkspace;
use photocraft_doc::{ColorMode, Document, SampleType, Size, XcfState, item_identity::ResourceId, xcf_resources::*};
use photocraft_engine::{Session, xcf_resources::RecoveryPolicy};
use photocraft_raster::Interrupt;
use serde_json::json;
#[test]
fn rpc_resource_recovery_requires_a_new_explicit_root_after_native_reopen() {
    use photocraft_automation::Headless;
    let dir = std::env::temp_dir().join(format!("photocraft-own-resource-rpc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("own.resource"), b"own RPC data").unwrap();
    let mut document = Document::new("Own RPC resource", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    document.xcf_state = Some(XcfState {
        resources: vec![ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Own resource".into(),
            collection: None,
            internal: false,
            locator: Some("own.resource".into()),
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        }],
        ..Default::default()
    });
    let request = json!({"resource":"1","path":"own.resource","policy":"substitute"});
    for mut denied in [Headless::new(), Headless::trusted_local()] {
        denied.session.open_document(document.clone(), None).unwrap();
        assert!(denied.handle("xcf.resource.recover", request.clone()).is_err());
        assert_eq!(*denied.session.active().unwrap().doc, document);
    }
    let mut granted = Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir), None).unwrap());
    granted.session.open_document(document.clone(), None).unwrap();
    for bad in [
        json!({}),
        json!({"resource":1,"path":"own.resource","policy":"substitute"}),
        json!({"resource":"01","path":"own.resource","policy":"substitute"}),
        json!({"resource":"1","path":"own.resource","policy":"substitute","extra":true}),
    ] {
        assert!(granted.handle("xcf.resource.recover", bad).is_err());
        assert_eq!(*granted.session.active().unwrap().doc, document);
    }
    assert_eq!(granted.handle("xcf.resource.recover", request.clone()).unwrap()["byteIdentity"], "Substituted");
    let recovered = granted.session.active().unwrap().doc.clone();
    let native = photocraft_format::save_to_bytes(&recovered, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened, *recovered);
    let mut denied = Headless::new();
    denied.session.open_document(reopened, None).unwrap();
    assert!(denied.handle("xcf.resource.recover", request).is_err());
    assert_eq!(*denied.session.active().unwrap().doc, *recovered);
    assert!(!denied.session.active().unwrap().history.can_undo());
    granted.session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*granted.session.active().unwrap().doc, document);
    drop(granted);
    std::fs::remove_file(dir.join("own.resource")).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}
#[test]
fn authorized_root_recovery_is_explicit_bounded_and_undoable() {
    let dir = std::env::temp_dir().join(format!("photocraft-own-resource-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("own.resource"), b"own recovered data").unwrap();
    let mut document = Document::new("Own link project", Size::new(2, 2), ColorMode::Rgb, SampleType::U16);
    document.xcf_state = Some(XcfState {
        resources: vec![ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Own inert resource".into(),
            collection: None,
            internal: false,
            locator: Some("https://invalid.example/never-read".into()),
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        }],
        ..Default::default()
    });
    let mut s = Session::new();
    s.open_document(document, None).unwrap();
    let before = s.active().unwrap().doc.clone();
    let denied = AuthorizedWorkspace::default();
    assert!(denied.recover_xcf_resource(&mut s, ResourceId(1), "own.resource", RecoveryPolicy::AllowSubstitution, &Interrupt::NONE).is_err());
    let granted = AuthorizedWorkspace::new(Some(&dir), None).unwrap();
    for path in ["../own.resource", "https://invalid.example/never-read", "C:/own.resource", "own\\resource", "/own.resource"] {
        assert!(granted.recover_xcf_resource(&mut s, ResourceId(1), path, RecoveryPolicy::AllowSubstitution, &Interrupt::NONE).is_err());
    }
    assert!(granted.recover_xcf_resource(&mut s, ResourceId(1), "own.resource", RecoveryPolicy::AllowSubstitution, &Interrupt::cancel_only(&|| true)).is_err());
    assert!(granted.recover_xcf_resource(&mut s, ResourceId(1), "own.resource", RecoveryPolicy::Exact, &Interrupt::NONE).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(!s.active().unwrap().history.can_undo());
    assert_eq!(
        granted.recover_xcf_resource(&mut s, ResourceId(1), "own.resource", RecoveryPolicy::AllowSubstitution, &Interrupt::NONE).unwrap(),
        ResourceFidelity::Substituted
    );
    assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().resources[0].owned_bytes.as_deref().map(Vec::as_slice), Some(b"own recovered data".as_slice()));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(std::fs::read(dir.join("own.resource")).unwrap(), b"own recovered data");
    drop(granted);
    std::fs::remove_file(dir.join("own.resource")).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}
