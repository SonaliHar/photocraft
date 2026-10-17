use photocraft_doc::{
    ColorMode, Document, SampleType, Size, XcfState,
    item_identity::ResourceId,
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_engine::Session;
use serde_json::{Value, json};
fn resource(id: u64) -> ResourceReference {
    ResourceReference {
        id: ResourceId(id),
        kind: ResourceKind::Font,
        name: "Own font Ω 日本語".into(),
        collection: Some("inert own collection".into()),
        internal: false,
        locator: Some("https://invalid.example/never-load".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    }
}
fn session(resources: Vec<ResourceReference>) -> Session {
    let mut doc = Document::new("Own resource project", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    doc.xcf_state = Some(XcfState { resources, ..Default::default() });
    let mut session = Session::new();
    session.open_document(doc, None).unwrap();
    session
}
#[test]
fn exact_decimal_ids_and_inert_native_reopen_preserve_status_and_evidence() {
    let mut owned = resource(7);
    owned.kind = ResourceKind::Pattern;
    owned.owned_bytes = Some(vec![1, 2, 3, 4].into());
    owned.content_hash = Some(blake3::hash(owned.owned_bytes.as_ref().unwrap()).to_hex().to_string());
    let mut session = session(vec![resource(u64::MAX), owned]);
    let before = session.active().unwrap().doc.clone();
    let undo = session.active().unwrap().history.can_undo();
    let all = session.execute("xcf.resource.inspect", json!({})).unwrap();
    assert_eq!(all["count"], 2);
    assert_eq!(all["resources"][0]["id"], u64::MAX.to_string());
    assert_eq!(all["resources"][0]["name"], "Own font Ω 日本語");
    assert_eq!(all["resources"][0]["status"], "unresolved");
    assert_eq!(all["resources"][0]["identityResolved"], false);
    assert_eq!(all["resources"][0]["activated"], false);
    assert_eq!(all["resources"][1]["status"], "exact-hash-bound");
    assert_eq!(all["resources"][1]["ownedByteLength"], 4);
    assert!(all["resources"][1].get("ownedBytes").is_none());
    let one = session.execute("xcf.resource.inspect", json!({"resource":u64::MAX.to_string()})).unwrap();
    assert_eq!(one["count"], 1);
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().history.can_undo(), undo);
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.xcf_state, before.xcf_state);
    let mut re_session = Session::new();
    re_session.open_document(reopened, None).unwrap();
    assert_eq!(re_session.execute("xcf.resource.inspect", json!({})).unwrap(), all);
}
#[test]
fn malformed_unknown_duplicate_and_unverified_fail_without_history() {
    let mut s = session(vec![resource(1)]);
    let before = s.active().unwrap().doc.clone();
    // Registered read-only XCF queries normalize null to the empty request.
    assert_eq!(s.execute("xcf.resource.inspect", Value::Null).unwrap(), s.execute("xcf.resource.inspect", json!({})).unwrap());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(!s.active().unwrap().history.can_undo());
    for params in [
        json!("invalid"),
        json!([]),
        json!({"resource":1}),
        json!({"resource":1.0}),
        json!({"resource":true}),
        json!({"resource":null}),
        json!({"resource":"0"}),
        json!({"resource":"01"}),
        json!({"resource":"+1"}),
        json!({"resource":" 1"}),
        json!({"resource":"1.0"}),
        json!({"resource":"18446744073709551616"}),
        json!({"resource":"１"}),
        json!({"resource":"2"}),
        json!({"load":true}),
    ] {
        assert!(s.execute("xcf.resource.inspect", params.clone()).is_err(), "{params}");
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(!s.active().unwrap().history.can_undo());
    }
    s.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().resources.push(resource(1));
    assert!(s.execute("xcf.resource.inspect", json!({"resource":"1"})).is_err());
    let mut invalid = resource(3);
    invalid.kind = ResourceKind::Pattern;
    invalid.owned_bytes = Some(vec![0, 1].into());
    let mut s = session(vec![]);
    s.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().resources.push(invalid);
    assert!(s.execute("xcf.resource.inspect", json!({})).unwrap_err().to_string().contains("resource_hash"));
    assert!(!s.active().unwrap().history.can_undo());
}
#[test]
fn output_is_bounded_without_unicode_truncation_and_empty_state_is_readable() {
    let mut s = session((1..=1001).map(resource).collect());
    assert!(s.execute("xcf.resource.inspect", json!({})).is_err());
    assert_eq!(s.execute("xcf.resource.inspect", json!({"resource":"1001"})).unwrap()["count"], 1);
    let mut long = resource(1);
    long.name = "Ω".repeat(8193);
    let mut s = session(vec![long]);
    assert!(s.execute("xcf.resource.inspect", json!({})).is_err());
    let mut aggregate = (1..=128).map(resource).collect::<Vec<_>>();
    for resource in &mut aggregate {
        resource.name = "Ω".repeat(1024);
    }
    let mut s = session(aggregate);
    assert!(s.execute("xcf.resource.inspect", json!({})).is_err());
    assert_eq!(s.execute("xcf.resource.inspect", json!({"resource":"1"})).unwrap()["resources"][0]["name"], "Ω".repeat(1024));
    let mut s = session(vec![]);
    assert_eq!(s.execute("xcf.resource.inspect", json!({})).unwrap(), json!({"resources":[],"count":0}));
    assert!(Session::new().execute("xcf.resource.inspect", json!({})).is_err());
}
