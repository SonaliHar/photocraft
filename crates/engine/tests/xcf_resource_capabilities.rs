#[path = "common/accounted_resource.rs"]
mod accounted_fixture;
use photocraft_doc::{
    ColorMode, Document, SampleType, Size, XcfState,
    item_identity::ResourceId,
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_engine::document_snapshot::DocumentSnapshot;
use photocraft_engine::xcf_resources::{RecoveryError, ResourceCapabilities, ResourceHandle, ServiceResourceBytes};
use photocraft_raster::Interrupt;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn doc(bytes: &[u8]) -> DocumentSnapshot {
    let mut d = Document::new("Own grant project", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    d.xcf_state = Some(XcfState {
        resources: vec![ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::Pattern,
            name: "Own pattern Ω".into(),
            collection: None,
            internal: false,
            locator: Some("file:///never-authorized".into()),
            content_hash: Some(blake3::hash(bytes).to_hex().to_string()),
            owned_bytes: None,
            resolution: None,
        }],
        ..Default::default()
    });
    DocumentSnapshot::from_resident(Arc::new(d)).unwrap()
}
fn input(bytes: &[u8]) -> ServiceResourceBytes {
    ServiceResourceBytes::from_authorized_service(ResourceKind::Pattern, bytes.to_vec(), true).unwrap()
}
fn handle(n: u8) -> ResourceHandle {
    ResourceHandle::from_service_nonce([n; 32]).unwrap()
}
#[test]
fn exact_bytes_are_consumed_once_without_document_or_native_authority_changes() {
    let bytes = b"Own exact bytes, pending typed pattern decoder";
    let d = doc(bytes);
    let before = d.clone();
    let mut caps = ResourceCapabilities::default();
    caps.issue(handle(1), &d, ResourceId(1), input(bytes), &Interrupt::NONE).unwrap();
    assert_eq!(caps.len(), 1);
    let payload = caps.consume(handle(1), &d, &Interrupt::NONE).unwrap();
    assert_eq!(payload.bytes(), bytes);
    assert_eq!(payload.resource(), ResourceId(1));
    assert_eq!(payload.expected_hash(), blake3::hash(bytes).to_hex().to_string());
    assert!(caps.is_empty());
    assert!(d.shares_state(&before));
    assert!(matches!(caps.consume(handle(1), &d, &Interrupt::NONE), Err(RecoveryError::UnknownHandle)));
    assert_eq!(caps.issue(handle(1), &d, ResourceId(1), input(bytes), &Interrupt::NONE), Err(RecoveryError::DuplicateHandle));
    let native = photocraft_format::save_to_bytes(&d, &Default::default()).unwrap();
    let reopen = DocumentSnapshot::from_resident(Arc::new(photocraft_format::load_from_bytes(&native).unwrap())).unwrap();
    assert_eq!(reopen.xcf_state, d.xcf_state);
    caps.issue(handle(2), &d, ResourceId(1), input(bytes), &Interrupt::NONE).unwrap();
    assert!(matches!(caps.consume(handle(2), &reopen, &Interrupt::NONE), Err(RecoveryError::StaleDocument)));
    assert_eq!(caps.len(), 1);
    caps.clear();
    assert!(caps.is_empty());
    assert!(matches!(caps.consume(handle(2), &d, &Interrupt::NONE), Err(RecoveryError::UnknownHandle)));
    assert_eq!(caps.issue(handle(2), &d, ResourceId(1), input(bytes), &Interrupt::NONE), Err(RecoveryError::DuplicateHandle));
}
#[test]
fn denied_kind_hash_unknown_and_limits_never_issue_authority() {
    let b = b"own";
    let d = doc(b);
    let mut caps = ResourceCapabilities::default();
    assert!(matches!(ServiceResourceBytes::from_authorized_service(ResourceKind::Pattern, b.to_vec(), false), Err(RecoveryError::Denied)));
    assert!(matches!(ServiceResourceBytes::from_authorized_service(ResourceKind::Font, b.to_vec(), true), Err(RecoveryError::UnsupportedFont)));
    assert!(matches!(ResourceHandle::from_service_nonce([0; 32]), Err(RecoveryError::Denied)));
    assert_eq!(caps.issue(handle(1), &d, ResourceId(2), input(b), &Interrupt::NONE), Err(RecoveryError::UnknownResource));
    let wrong_kind = ServiceResourceBytes::from_authorized_service(ResourceKind::Gradient, b.to_vec(), true).unwrap();
    assert_eq!(caps.issue(handle(1), &d, ResourceId(1), wrong_kind, &Interrupt::NONE), Err(RecoveryError::KindMismatch));
    assert_eq!(caps.issue(handle(1), &d, ResourceId(1), input(b"different"), &Interrupt::NONE), Err(RecoveryError::HashMismatch));
    assert!(caps.is_empty());
    let mut missing = doc(b);
    missing.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().resources[0].content_hash = None;
    assert_eq!(caps.issue(handle(1), &missing, ResourceId(1), input(b), &Interrupt::NONE), Err(RecoveryError::MissingHash));
    let mut caps = ResourceCapabilities::with_limits(2, 2, 1).unwrap();
    assert_eq!(caps.issue(handle(1), &d, ResourceId(1), input(b), &Interrupt::NONE), Err(RecoveryError::Limit));
    let mut caps = ResourceCapabilities::with_limits(4, 4, 2).unwrap();
    caps.issue(handle(1), &d, ResourceId(1), input(b), &Interrupt::NONE).unwrap();
    assert_eq!(caps.issue(handle(2), &d, ResourceId(1), input(b), &Interrupt::NONE), Err(RecoveryError::Limit));
    assert_eq!(caps.len(), 1);
    assert!(ResourceCapabilities::with_limits(usize::MAX, usize::MAX, usize::MAX).is_err());
}
#[test]
fn cow_edits_revoke_grants_and_cancellation_is_atomic_during_hashing() {
    let bytes = vec![42; 256 << 10];
    let mut d = doc(&bytes);
    let mut caps = ResourceCapabilities::default();
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 5;
    let ctl = Interrupt::cancel_only(&cancel);
    assert_eq!(caps.issue(handle(1), &d, ResourceId(1), input(&bytes), &ctl), Err(RecoveryError::Cancelled));
    assert!(caps.is_empty());
    assert!(polls.load(Ordering::Relaxed) >= 6);
    caps.issue(handle(1), &d, ResourceId(1), input(&bytes), &Interrupt::NONE).unwrap();
    assert!(matches!(caps.consume(handle(1), &d, &Interrupt::cancel_only(&|| true)), Err(RecoveryError::Cancelled)));
    assert_eq!(caps.len(), 1);
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 5;
    assert!(matches!(caps.consume(handle(1), &d, &Interrupt::cancel_only(&cancel)), Err(RecoveryError::Cancelled)));
    assert_eq!(caps.len(), 1);
    d.try_resident_mut().unwrap().name = "Own renamed grant project".into();
    assert!(matches!(caps.consume(handle(1), &d, &Interrupt::NONE), Err(RecoveryError::StaleDocument)));
    assert_eq!(caps.len(), 1);
}

#[test]
fn typed_accounted_grant_weak_owner_never_detaches_or_retains_model_payload() {
    use photocraft_doc::accounted_document::AccountedDocument;
    use photocraft_raster::backing::{ReadControl, quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage}};
    let bytes = b"own authorized account-bound bytes";
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 0, index: 1 << 20 });
    let retention = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let mut raw = accounted_fixture::document(&quota, &control);
    let mut state = doc(bytes).xcf_state.clone().unwrap();
    state.reconcile_layers(&raw).unwrap();
    raw.xcf_state = Some(state);
    let accounted = AccountedDocument::from_owned(raw, Arc::new(retention), quota.clone(), &control).unwrap();
    let snapshot = DocumentSnapshot::from_accounted(accounted).unwrap();
    assert!(snapshot.resident_arc().is_none());
    let mut caps = ResourceCapabilities::default();
    caps.issue(handle(7), &snapshot, ResourceId(1), input(bytes), &Interrupt::NONE).unwrap();
    let clone = snapshot.clone();
    drop(snapshot);
    // A hostile public-state substitution must not bypass the resident-only
    // compatibility fence through a direct recovery API.
    let mut session = photocraft_engine::Session::new();
    session.open_document((*doc(bytes)).clone(), None).unwrap();
    session.active_mut().unwrap().doc = clone.clone();
    let before = session.active().unwrap().doc.clone();
    assert!(photocraft_engine::xcf_resources::recover_authorized_bytes(
        &mut session, ResourceId(1), input(bytes),
        photocraft_engine::xcf_resources::RecoveryPolicy::Exact, &Interrupt::NONE,
    ).is_err());
    assert!(photocraft_engine::xcf_link_cmds::refresh_with_interrupt(&mut session, &serde_json::json!({"layerId":"1"}), &Interrupt::NONE).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert!(!session.active().unwrap().history.can_undo());
    drop(before);
    drop(session);
    assert!(quota.usage().unwrap().resident >= 65536);
    assert_eq!(caps.consume(handle(7), &clone, &Interrupt::NONE).unwrap().bytes(), bytes);
    caps.issue(handle(8), &clone, ResourceId(1), input(bytes), &Interrupt::NONE).unwrap();
    drop(clone);
    // Index is a resident subquota: the weak Arc control-block lease remains,
    // while all native/model payload ownership has ended.
    let remaining = quota.usage().unwrap();
    assert_eq!(remaining.resident, remaining.index);
    assert!(remaining.index > 0);
    assert!(matches!(caps.consume(handle(8), &doc(bytes), &Interrupt::NONE), Err(RecoveryError::StaleDocument)));
    caps.clear();
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
