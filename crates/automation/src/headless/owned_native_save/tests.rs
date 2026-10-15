//! UNRUN actual owned-source -> protected native Save/Save As workflows.
use super::*;
use crate::AuthorizedWorkspace;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "../../../../xcf/tests/common/mod.rs"]
mod fixture;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-owned-headless-save-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn opened(dir: &Directory, trusted: bool, write: bool) -> Headless {
    std::fs::write(dir.0.join("source.xcf"), fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
    let mut h = if trusted {
        Headless::trusted_local()
    } else {
        Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir.0), write.then_some(dir.0.as_path())).unwrap())
    };
    let path = if trusted { dir.0.join("source.xcf") } else { PathBuf::from("source.xcf") };
    drop(h.open_owned_xcf_keep_current(&path, &control()).unwrap());
    h
}
#[test]
fn actual_trusted_and_rooted_native_save_preserve_source_pixels_quota_and_history_identity() {
    for trusted in [true, false] {
        let dir = Directory::new();
        let mut h = opened(&dir, trusted, true);
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        let revision = h.session.active().unwrap().revision;
        let q = before.quota().unwrap().clone();
        let history = h.session.active().unwrap().history.past_len();
        let path = if trusted { dir.0.join("copy.pcraft") } else { PathBuf::from("copy.pcraft") };
        let reply = h.save_owned_native_active(Some(&path), None, &control()).unwrap();
        assert_eq!(reply["path"], path.to_str().unwrap());
        let state = h.session.active().unwrap();
        assert_eq!(state.doc.name, "copy.pcraft");
        assert_eq!(state.path.as_deref(), path.to_str());
        assert_eq!(state.saved_revision, revision);
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.past_len(), history);
        assert_eq!(state.source, source);
        assert!(state.source_read_only);
        assert!(state.doc.quota().unwrap().shares_ledger(&q));
        assert_eq!(state.doc.xcf_state, before.xcf_state);
        assert_eq!(state.doc.source_color_context, before.source_color_context);
        let archive =
            crate::files::open_accounted_native_selected_path(dir.0.join("copy.pcraft").to_str().unwrap(), Default::default(), q.clone(), &control()).unwrap();
        let loaded = archive.into_accounted_document(&control()).unwrap();
        assert_eq!(loaded.document().name, "copy.pcraft");
        assert!(loaded.quota().shares_ledger(&q));
        let photocraft_doc::LayerContent::BackedRaster(layer) = &loaded.document().layers[0].content else { panic!() };
        let mut pixel = [0; 4];
        layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &control()).unwrap();
        assert_eq!(pixel, [31, 61, 91, 255]);
        // Subsequent Save uses only the actual installed native destination.
        assert!(h.save_owned_native_active(None, None, &control()).is_ok());
        assert_eq!(std::fs::read(dir.0.join("source.xcf")).unwrap(), fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]));
    }
}
#[test]
fn read_only_root_cancel_and_unsupported_output_leave_model_path_history_and_source_unchanged() {
    let dir = Directory::new();
    let mut h = opened(&dir, false, false);
    let before = h.session.active().unwrap().doc.clone();
    let source = h.session.active().unwrap().source.clone();
    let revision = h.session.active().unwrap().revision;
    let cancel = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(matches!(
        h.save_owned_native_active(Some(Path::new("copy.pcraft")), None, &cancel),
        Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
    ));
    // Fail after entry/shape checks, including paid protection/helper phases.
    // Actual parent callback errors must survive their shared String boundary.
    for fail_at in [3u64, 8, 16, 32] {
        for error in [BackingError::Cancelled, BackingError::Work] {
            let calls = AtomicU64::new(0);
            let work = |_| if calls.fetch_add(1, Ordering::Relaxed) >= fail_at { Err(error) } else { Ok(()) };
            let delayed = ReadControl { interrupt: &Interrupt::NONE, work: &work };
            let result = h.save_owned_native_active(Some(Path::new("copy.pcraft")), None, &delayed);
            if calls.load(Ordering::Relaxed) > fail_at {
                let Err(AutomationError::Engine(actual)) = result else { panic!("parent failure type was lost") };
                match error {
                    BackingError::Cancelled => assert!(matches!(actual, photocraft_engine::EngineError::Cancelled)),
                    BackingError::Work => assert_eq!(actual.to_string(), photocraft_engine::EngineError::from(BackingError::Work).to_string()),
                    _ => panic!(),
                }
            } else {
                assert!(result.is_err());
            } // This fixture has no write root.
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
    }
    for (path, format) in [("copy.png", None), ("copy.pcraft", Some("png")), ("copy.pcraft", None)] {
        assert!(h.save_owned_native_active(Some(Path::new(path)), format, &control()).is_err());
    }
    let state = h.session.active().unwrap();
    assert!(state.doc.shares_state(&before));
    assert_eq!(state.source, source);
    assert_eq!(state.revision, revision);
    assert!(state.path.is_none());
    assert_eq!(state.history.past_len(), 0);
    assert!(!dir.0.join("copy.pcraft").exists());
}
#[test]
fn source_content_alias_and_nonactive_accounted_index_refuse_without_publication() {
    let dir = Directory::new();
    let mut h = opened(&dir, true, true);
    let alias = dir.0.join("alias.pcraft");
    std::fs::hard_link(dir.0.join("source.xcf"), &alias).unwrap();
    let before = h.session.active().unwrap().doc.clone();
    assert!(h.save_owned_native_active(Some(&alias), None, &control()).is_err());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert!(h.session.active().unwrap().path.is_none());
    std::fs::write(dir.0.join("second.xcf"), fixture::image(13, 150, 0, 1, 1, &[1, 2, 3, 255])).unwrap();
    drop(h.open_owned_xcf_keep_current(&dir.0.join("second.xcf"), &control()).unwrap());
    let current = h.session.active_index();
    assert!(h.save(Some(0), Some(&dir.0.join("never.pcraft")), None, &Default::default()).is_err());
    assert_eq!(h.session.active_index(), current);
    assert!(!dir.0.join("never.pcraft").exists());
}
