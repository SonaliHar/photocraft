//! UNRUN retained RPC transport, authority and early refusal workflows.
use super::*;
use crate::{AuthorizedWorkspace, Headless};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
#[path = "../../../../xcf/tests/common/mod.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-rpc-owned-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn request(id: Value, params: Value) -> String {
    serde_json::json!({"id":id,"method":"doc.open","params":params}).to_string()
}
#[test]
fn retained_stdio_publishes_once_and_holds_complete_reply_charge_during_writer() {
    let dir = Directory::new();
    std::fs::write(dir.0.join("plain.xcf"), fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
    let session = Mutex::new(Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir.0), None).unwrap()));
    struct Writer<'a> {
        session: &'a Mutex<Headless>,
        bytes: Vec<u8>,
        paid: Option<u64>,
    }
    impl Write for Writer<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let h = self.session.lock().unwrap();
            let q = h.session.active().unwrap().doc.quota().unwrap();
            self.paid = Some(q.usage().unwrap().index);
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let input = request(serde_json::json!({"request":[7,"exact"]}), serde_json::json!({"path":"plain.xcf","xcfProfilePolicy":"keepCurrent"})) + "\n";
    let mut writer = Writer { session: &session, bytes: Vec::new(), paid: None };
    super::super::serve_lines(&session, Cursor::new(input), &mut writer).unwrap();
    let reply: Value = serde_json::from_slice(&writer.bytes).unwrap();
    assert_eq!(reply["id"], serde_json::json!({"request":[7,"exact"]}));
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"]["inspectionScope"], "owned-import-summary");
    let h = session.lock().unwrap();
    let state = h.session.active().unwrap();
    assert!(state.doc.accounted().is_some());
    assert!(state.source_read_only);
    assert_eq!(h.session.documents().len(), 1);
    assert!(writer.paid.unwrap() > state.doc.quota().unwrap().usage().unwrap().index);
}
#[test]
fn raw_value_unknown_null_extra_and_unbounded_ids_decline_without_opening() {
    let h = Mutex::new(Headless::trusted_local());
    let good = serde_json::json!({"path":"missing.xcf","xcfProfilePolicy":"keepCurrent"});
    assert!(h.lock().unwrap().handle("doc.open", good.clone()).is_err());
    assert_eq!(super::super::respond(&h, &request(Value::Null, good))["ok"], false);
    for params in [
        serde_json::json!({"path":"missing.xcf","xcfProfilePolicy":null}),
        serde_json::json!({"path":"missing.xcf","xcfProfilePolicy":"convert"}),
        serde_json::json!({"path":"missing.xcf","xcfProfilePolicy":"keepCurrent","extra":true}),
    ] {
        let reply = super::super::respond_retained(&h, &request(serde_json::json!(1), params), true);
        assert_eq!(reply.value()["ok"], false);
        assert!(!reply.value()["error"].as_str().unwrap().contains("I/O:"));
    }
    let mut deep = Value::Null;
    for _ in 0..10 {
        deep = serde_json::json!([deep]);
    }
    let reply = super::super::respond_retained(&h, &request(deep, serde_json::json!({"path":"missing.xcf","xcfProfilePolicy":"keepCurrent"})), true);
    assert_eq!(reply.value()["id"], Value::Null);
    assert!(h.lock().unwrap().session.documents().is_empty());
}
#[test]
fn absent_authority_cannot_become_trusted_and_absent_policy_keeps_resident_default() {
    let dir = Directory::new();
    let path = dir.0.join("plain.xcf");
    std::fs::write(&path, fixture::image(13, 150, 0, 1, 1, &[1, 2, 3, 255])).unwrap();
    let denied = Mutex::new(Headless::new());
    let reply =
        super::super::respond_retained(&denied, &request(serde_json::json!(1), serde_json::json!({"path":path,"xcfProfilePolicy":"keepCurrent"})), true);
    assert_eq!(reply.value()["ok"], false);
    assert!(denied.lock().unwrap().session.documents().is_empty());
    let h = Mutex::new(Headless::trusted_local());
    assert_eq!(super::super::respond(&h, &request(serde_json::json!(2), serde_json::json!({"path":path})))["ok"], true);
    assert!(h.lock().unwrap().session.active().unwrap().doc.accounted().is_none());
}
