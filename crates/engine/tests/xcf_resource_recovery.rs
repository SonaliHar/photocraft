use photocraft_doc::{ColorMode, Document, SampleType, Size, XcfState, item_identity::ResourceId, xcf_resources::*};
use photocraft_engine::{Session, xcf_resources::*};
use photocraft_raster::Interrupt;
use serde_json::json;
fn session(expected: Option<&[u8]>) -> Session {
    let mut document = Document::new("Own resource recovery", Size::new(2, 2), ColorMode::Rgb, SampleType::U16);
    document.xcf_state = Some(XcfState {
        resources: vec![ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Own link".into(),
            collection: None,
            internal: false,
            locator: Some("https://invalid.example/never-read".into()),
            content_hash: expected.map(|b| blake3::hash(b).to_hex().to_string()),
            owned_bytes: None,
            resolution: None,
        }],
        ..Default::default()
    });
    let mut s = Session::new();
    s.open_document(document, None).unwrap();
    s
}
fn input(bytes: &[u8]) -> ServiceResourceBytes {
    ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, bytes.to_vec(), true).unwrap()
}
#[test]
fn exact_and_substituted_recovery_preserve_requested_identity_and_undo_native_data() {
    for expected in [None, Some(b"different".as_slice()), Some(b"own payload".as_slice())] {
        let mut s = session(expected);
        let before = s.active().unwrap().doc.clone();
        let fidelity = recover_authorized_bytes(&mut s, ResourceId(1), input(b"own payload"), RecoveryPolicy::AllowSubstitution, &Interrupt::NONE).unwrap();
        assert_eq!(fidelity, if expected == Some(b"own payload".as_slice()) { ResourceFidelity::Exact } else { ResourceFidelity::Substituted });
        let after = s.active().unwrap().doc.clone();
        assert_eq!(after.xcf_state.as_ref().unwrap().resources[0].content_hash, before.xcf_state.as_ref().unwrap().resources[0].content_hash);
        let inspected = s.execute("xcf.resource.inspect", json!({})).unwrap();
        assert_eq!(inspected["resources"][0]["identityResolved"], true);
        assert_eq!(inspected["resources"][0]["activated"], false);
        let native = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened, *after);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.execute("edit.redo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&after));
    }
}
#[test]
fn missing_wrong_hash_and_cancel_never_change_document_or_history() {
    let mut s = session(None);
    let before = s.active().unwrap().doc.clone();
    assert!(recover_authorized_bytes(&mut s, ResourceId(1), input(b"own"), RecoveryPolicy::Exact, &Interrupt::NONE).is_err());
    assert!(recover_authorized_bytes(&mut s, ResourceId(1), input(b"own"), RecoveryPolicy::AllowSubstitution, &Interrupt::cancel_only(&|| true)).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(!s.active().unwrap().history.can_undo());
    let mut s = session(Some(b"original"));
    let before = s.active().unwrap().doc.clone();
    assert!(recover_authorized_bytes(&mut s, ResourceId(1), input(b"different"), RecoveryPolicy::Exact, &Interrupt::NONE).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn forged_exact_resolution_and_changed_owned_payload_fail_validation() {
    let mut s = session(Some(b"original"));
    recover_authorized_bytes(&mut s, ResourceId(1), input(b"different"), RecoveryPolicy::AllowSubstitution, &Interrupt::NONE).unwrap();
    let mut document = (*s.active().unwrap().doc).clone();
    document.xcf_state.as_mut().unwrap().resources[0].resolution.as_mut().unwrap().fidelity = ResourceFidelity::Exact;
    assert!(document.validate_xcf_state().is_err());
    let mut document = (*s.active().unwrap().doc).clone();
    std::sync::Arc::make_mut(document.xcf_state.as_mut().unwrap().resources[0].owned_bytes.as_mut().unwrap())[0] ^= 1;
    assert!(document.validate_xcf_state().is_err());
    let mut document = (*s.active().unwrap().doc).clone();
    document.xcf_state.as_mut().unwrap().resources[0].owned_bytes = None;
    assert!(document.validate_xcf_state().is_err());
}

#[test]
fn explicit_upload_command_is_undoable_and_bad_parameters_are_atomic() {
    let mut s = session(None);
    let before = s.active().unwrap().doc.clone();
    for params in [
        json!(null),
        json!({"resource":1,"payloadHex":"00","policy":"substitute"}),
        json!({"resource":"1","payloadHex":"0g","policy":"substitute"}),
        json!({"resource":"1","payloadHex":"0","policy":"substitute"}),
        json!({"resource":"1","payloadHex":"00","policy":"silent"}),
        json!({"resource":"1","payloadHex":"00","policy":"exact"}),
        json!({"resource":"1","payloadHex":"00","policy":"substitute","path":"ambient"}),
    ] {
        assert!(s.execute("xcf.resource.upload", params).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(!s.active().unwrap().history.can_undo());
    }
    let result = s.execute("xcf.resource.upload", json!({"resource":"1","payloadHex":"4f776e00ff","policy":"substitute"})).unwrap();
    assert_eq!(result["byteIdentity"], "Substituted");
    assert_eq!(result["activated"], false);
    assert_eq!(
        s.active().unwrap().doc.xcf_state.as_ref().unwrap().resources[0].owned_bytes.as_ref().map(|bytes| bytes.as_slice()),
        Some(b"Own\0\xff".as_slice())
    );
    assert!(!s.journal.iter().any(|(id, _)| id == "xcf.resource.upload"));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&before));
}

// Drafted during implementation-first pass; execute with the later combined gates.
#[test]
fn uppercase_exact_hash_retains_original_text_identity_and_native_undo() {
    let seed = session(Some(b"Own uppercase identity payload"));
    let mut document = (*seed.active().unwrap().doc).clone();
    let resource = document.xcf_state.as_mut().unwrap().resources.get_mut(0).unwrap();
    let original_hash = resource.content_hash.as_ref().unwrap().to_ascii_uppercase();
    assert_ne!(original_hash, resource.content_hash.as_ref().unwrap().as_str());
    resource.content_hash = Some(original_hash.clone());
    document.validate_xcf_state().unwrap();
    let mut session = Session::new();
    session.open_document(document, None).unwrap();
    let before = session.active().unwrap().doc.clone();
    let fidelity =
        recover_authorized_bytes(&mut session, ResourceId(1), input(b"Own uppercase identity payload"), RecoveryPolicy::Exact, &Interrupt::NONE).unwrap();
    assert_eq!(fidelity, ResourceFidelity::Exact);
    let after = session.active().unwrap().doc.clone();
    assert_eq!(after.xcf_state.as_ref().unwrap().resources.first().unwrap().content_hash.as_deref(), Some(original_hash.as_str()));
    assert_eq!(
        after.xcf_state.as_ref().unwrap().resources.first().unwrap().owned_bytes.as_ref().map(|bytes| bytes.as_slice()),
        Some(b"Own uppercase identity payload".as_slice())
    );
    after.validate_xcf_state().unwrap();
    let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *after);
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&before));
    session.execute("edit.redo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&after));
}
