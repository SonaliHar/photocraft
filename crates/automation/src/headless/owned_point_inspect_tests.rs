//! UNRUN paid metadata queries over a genuinely qualified Point source.
//! Inspection adds no source, edit or rendering authority.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Cursor;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn point_session() -> (Session, BackingQuota) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-geometry-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    let name = "alien-hsl-rgb-double_non_linear.one-row.author.xcf";
    let row = manifest["artifacts"].as_array().unwrap().iter().find(|v| v["path"] == name).unwrap();
    let bytes = std::fs::read(root.join(name)).unwrap();
    assert_eq!(hash(&bytes), row["sha256"].as_str().unwrap());
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
        name,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::Point(imported) = imported else { panic!("actual qualified Point source") };
    let mut s = Session::new();
    s.add_owned_point_xcf_import(&imported, name, None, &ctl()).unwrap();
    (s, q)
}
fn request(s: &Session) -> Value {
    json!({"filterId":s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}

#[test]
#[ignore = "UNRUN: pinned qualified Point source and paid metadata transport"]
fn actual_point_class_headless_routes_paid_filter_and_project_without_job_or_model_mutation() {
    let (s, q) = point_session();
    let mut h = Headless::new();
    h.session = s;
    let p = request(&h.session);
    let before = h.session.active().unwrap().doc.clone();
    let journal = h.session.journal.clone();
    let baseline = q.usage().unwrap();
    assert!(h.has_owned_filter_inspection());
    assert!(h.has_owned_inspection_command("xcf.inspect"));
    assert!(h.has_owned_inspection_command("xcf.filter.inspect"));
    for command in ["xcf.inspect", "xcf.filter.inspect"] {
        let params = if command == "xcf.inspect" { Value::Null } else { p.clone() };
        let reply = h.inspect_owned_command_with_read_control(command, &params, false, &ctl()).unwrap();
        assert_eq!(reply.command(), command);
        assert!(reply.quota().shares_ledger(&q));
        drop(reply);
        assert_eq!(q.usage().unwrap(), baseline);
    }

    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(h.session.journal, journal);
    let wire = h.inspect_owned_filter_cli_bounded(&p, true, crate::budgets::MAX_RESPONSE_BYTES).unwrap();
    let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
    assert_eq!(value["command"], "xcf.filter.inspect");
    assert_eq!(value["result"]["argumentCount"], before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.len());
    drop(wire);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
#[ignore = "UNRUN: pinned qualified Point source and paid metadata transport"]
fn point_headless_keeps_authorization_control_and_no_fallback_boundaries() {
    let (s, q) = point_session();
    let mut h = Headless::new();
    h.session = s;
    let p = request(&h.session);
    let before = h.session.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny point metadata".into())));
    for command in ["xcf.inspect", "xcf.filter.inspect"] {
        let params = if command == "xcf.inspect" { Value::Null } else { p.clone() };
        assert!(h.inspect_owned_command_with_read_control(command, &params, true, &ctl()).is_err());
    }
    h.session.authorize = None;
    let deny = |_| Err(BackingError::Work);
    assert!(h.inspect_owned_command_with_read_control("xcf.filter.inspect", &p, false, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert!(h.inspect_owned_command_with_read_control("xcf.filter.set", &p, false, &ctl()).is_err());
    // The edit adapter is now available to Point owners. An absent stored name
    // is a semantic refusal, not a blanket class-level edit prohibition.
    assert!(h.command_run("xcf.filter.set", json!({"filterId":p["filterId"],"arguments":[{"name":"not-a-stored-argument","value":{"Bool":true}}]})).is_err());
    assert!(h.session.execute("xcf.filter.inspect", p.clone()).is_err());
    assert!(h.inspect_owned_filter_bounded(&serde_json::json!({"filterId":"999"}), false).is_err());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(q.usage().unwrap(), baseline);
}
