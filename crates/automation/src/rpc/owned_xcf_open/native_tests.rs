//! UNRUN real native archive -> retained RPC publication and policy failures.
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
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-rpc-owned-native-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn native(dir: &Directory) {
    let source = dir.0.join("original.xcf");
    std::fs::write(&source, fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current_bounded(&source).unwrap());
    h.save(None, Some(&dir.0.join("current.pcraft")), None, &Default::default()).unwrap();
    // The persisted original source identity must remain inert on native reopen.
    std::fs::remove_file(source).unwrap();
}
fn request(params: Value) -> String {
    serde_json::json!({"id":{"native":[7,"exact"]},"method":"doc.open","params":params}).to_string()
}
#[test]
fn retained_native_rpc_publishes_current_identity_and_holds_paid_envelope_through_write() {
    let dir = Directory::new();
    native(&dir);
    let h = Mutex::new(Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir.0), None).unwrap()));
    struct Writer<'a> {
        h: &'a Mutex<Headless>,
        bytes: Vec<u8>,
        index: u64,
    }
    impl Write for Writer<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.index = self.h.lock().unwrap().session.active().unwrap().doc.quota().unwrap().usage().unwrap().index;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Writer { h: &h, bytes: Vec::new(), index: 0 };
    super::super::serve_lines(&h, Cursor::new(request(serde_json::json!({"path":"current.pcraft","nativeProfilePolicy":"keepCurrent"})) + "\n"), &mut output)
        .unwrap();
    let reply: Value = serde_json::from_slice(&output.bytes).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["id"], serde_json::json!({"native":[7,"exact"]}));
    assert_eq!(reply["result"]["nativeProfilePolicy"], "keepCurrent");
    assert_eq!(reply["result"]["inspectionScope"], "owned-native-summary");
    let guard = h.lock().unwrap();
    let state = guard.session.active().unwrap();
    assert_eq!(guard.session.documents().len(), 1);
    assert!(state.doc.accounted().is_some());
    assert!(!state.source_read_only);
    assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("current.pcraft"));
    assert!(output.index > state.doc.quota().unwrap().usage().unwrap().index);
}
#[test]
fn both_null_unknown_extra_and_wrong_suffix_policies_refuse_before_sync_or_file_io() {
    let h = Mutex::new(Headless::trusted_local());
    for params in [
        serde_json::json!({"path":"missing.pcraft","nativeProfilePolicy":"keepCurrent","xcfProfilePolicy":"keepCurrent"}),
        serde_json::json!({"path":"missing.pcraft","nativeProfilePolicy":null}),
        serde_json::json!({"path":"missing.pcraft","nativeProfilePolicy":"convert"}),
        serde_json::json!({"path":"missing.pcraft","nativeProfilePolicy":"keepCurrent","extra":true}),
        serde_json::json!({"path":"missing.xcf","nativeProfilePolicy":"keepCurrent"}),
        serde_json::json!({"path":"missing.pcraft","xcfProfilePolicy":"keepCurrent"}),
    ] {
        let reply = super::super::respond_retained(&h, &request(params), true);
        assert_eq!(reply.value()["ok"], false);
        assert!(!reply.value()["error"].as_str().unwrap().contains("I/O:"));
        assert!(h.lock().unwrap().session.documents().is_empty());
    }
    let explicit = serde_json::json!({"path":"missing.pcraft","nativeProfilePolicy":"keepCurrent"});
    assert!(h.lock().unwrap().handle("doc.open", explicit.clone()).is_err());
    assert_eq!(super::super::respond(&h, &request(explicit))["ok"], false);
}
#[test]
fn native_explicit_reader_never_retries_trusted_authority_or_uses_serialized_source_locator() {
    let dir = Directory::new();
    native(&dir);
    let h = Mutex::new(Headless::new());
    let reply =
        super::super::respond_retained(&h, &request(serde_json::json!({"path":dir.0.join("current.pcraft"),"nativeProfilePolicy":"keepCurrent"})), true);
    assert_eq!(reply.value()["ok"], false);
    assert!(h.lock().unwrap().session.documents().is_empty());
    let rooted = Mutex::new(Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir.0), None).unwrap()));
    let reply = super::super::respond_retained(&rooted, &request(serde_json::json!({"path":"../outside.pcraft","nativeProfilePolicy":"keepCurrent"})), true);
    assert_eq!(reply.value()["ok"], false);
    assert!(rooted.lock().unwrap().session.documents().is_empty());
}
