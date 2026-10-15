//! UNRUN implementation-first drafts: genuine host readers and typed publication.
use super::*;
use photocraft_raster::{Interrupt, backing::quota::BackingQuota};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "../../../../xcf/tests/common/mod.rs"]
mod fixture;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-headless-owned-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
fn trusted_and_rooted_open_publish_original_native_owner_and_keep_reply_fee_until_drop() {
    for trusted in [true, false] {
        let dir = Directory::new();
        let full = dir.0.join("plain.xcf");
        std::fs::write(&full, fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
        let mut h = if trusted { Headless::trusted_local() } else { Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&dir.0), None).unwrap()) };
        let requested = if trusted { full.as_path() } else { Path::new("plain.xcf") };
        let reply = h.open_owned_xcf_keep_current(requested, &ctl()).unwrap();
        assert_eq!(reply.value()["index"], 0);
        assert_eq!(reply.value()["inspectionScope"], "owned-import-summary");
        assert_eq!(reply.value()["name"], "plain.xcf");
        assert_eq!(reply.value()["sourceReadOnly"], true);
        let state = h.session.active().unwrap();
        assert!(state.doc.accounted().is_some());
        assert!(state.source_read_only);
        assert_eq!(state.source.as_ref().unwrap().path.as_deref(), requested.to_str());
        let q = state.doc.quota().unwrap().clone();
        let paid = q.usage().unwrap();
        let photocraft_doc::LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!() };
        let mut pixel = [0; 4];
        layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &ctl()).unwrap();
        assert_eq!(pixel, [31, 61, 91, 255]);
        // Pin/cache can legitimately retain fees, so compare the reply fee after
        // warming that cache, rather than treating cache population as leakage.
        let warm = q.usage().unwrap();
        drop(reply);
        let after = q.usage().unwrap();
        assert!(after.index < warm.index);
        assert!(paid.index > 0);
        std::fs::remove_file(&full).unwrap();
        assert!(h.session.accounted_open_quota().unwrap().shares_ledger(&q));
        assert!(h.inspect(None).is_err()); // No resident inspector escape.
    }
}
#[test]
fn denied_root_and_cancelled_request_never_publish_or_change_existing_tab() {
    let mut denied = Headless::new();
    assert!(denied.open_owned_xcf_keep_current(Path::new("missing.xcf"), &ctl()).is_err());
    assert!(denied.session.documents().is_empty());
    let dir = Directory::new();
    std::fs::write(dir.0.join("plain.xcf"), fixture::image(13, 150, 0, 1, 1, &[1, 2, 3, 255])).unwrap();
    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&dir.0), None).unwrap());
    let reply = h.open_owned_xcf_keep_current(Path::new("plain.xcf"), &ctl()).unwrap();
    drop(reply);
    let before = h.session.active().unwrap().doc.clone();
    let revision = h.session.active().unwrap().revision;
    let q: BackingQuota = before.quota().unwrap().clone();
    let usage = q.usage().unwrap();
    let cancel = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(matches!(h.open_owned_xcf_keep_current(Path::new("plain.xcf"), &cancel), Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))));
    assert!(h.open_owned_xcf_keep_current(Path::new("../escape.xcf"), &ctl()).is_err());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.documents().len(), 1);
    assert_eq!(q.usage().unwrap(), usage);
}
#[test]
fn ordinary_open_stays_resident_and_explicit_failed_source_does_not_retry_it() {
    let dir = Directory::new();
    let plain = dir.0.join("plain.xcf");
    std::fs::write(&plain, fixture::image(13, 150, 0, 1, 1, &[1, 2, 3, 255])).unwrap();
    let mut ordinary = Headless::trusted_local();
    ordinary.open(&plain).unwrap();
    assert!(ordinary.session.active().unwrap().doc.accounted().is_none());
    let mut explicit = Headless::trusted_local();
    assert!(explicit.open_owned_xcf_keep_current(Path::new("not-xcf.png"), &ctl()).is_err());
    assert!(explicit.session.documents().is_empty());
}
