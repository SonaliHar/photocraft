//! UNRUN bounded native wrapper versus explicit caller-owned lower control.
use super::*;
#[path = "../../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use photocraft_doc::SampleType;
use photocraft_raster::backing::BackingError;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-box-work-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN: bounded owned native wrapper selects standard before input; explicit lower ReadControl is never replaced, no publication on work denial; no24/36MP completion or producer geometry claim"]
fn box_bounded_native_does_not_upgrade_external_lower_control() {
    for assigned in [false, true] {
        let dir = Directory::new();
        let (owner, literal) = fixture::fixture(SampleType::F32, assigned, 0);
        let path = dir.0.join("CURRENT.pcraft");
        // External harness capture; the loaded model/archive/provider is paid
        // by the actual Headless native reader, without model cloning.
        let raw = fixture::encoded(&owner);
        std::fs::write(&path, &raw).unwrap();
        let mut h = Headless::trusted_local();
        let denied_units = AtomicU64::new(0);
        let reject = |n| {
            denied_units.fetch_add(n, Ordering::Relaxed);
            Err(BackingError::Work)
        };
        let lower = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &reject };
        assert!(h.open_owned_native_keep_current(&path, &lower).is_err());
        assert!(denied_units.load(Ordering::Relaxed) > 0);
        assert!(h.session.documents().is_empty());
        let reply = h.open_owned_native_keep_current_bounded(&path).unwrap();
        assert!(reply.value().is_object());
        drop(reply);
        let current = &h.session.active().unwrap().doc;
        assert_eq!(fixture::native(current), literal.raw);
        assert_eq!(current.accounted().unwrap().is_readonly_assigned_point_storage(), assigned);
        assert!(!h.session.active().unwrap().source_read_only);
        assert_eq!(std::fs::read(path).unwrap(), raw);
    }
}
