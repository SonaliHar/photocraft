//! UNRUN actual-root authority and same-ledger private failure drafts.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaLimits, QuotaUsage},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-owned-xcf-root-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 })
}
#[test]
fn absent_root_escape_nonregular_oversized_and_corrupt_sources_refuse_without_retention() {
    let directory = Directory::new();
    std::fs::write(directory.0.join("corrupt.xcf"), b"gimp xcf v013\0").unwrap();
    std::fs::create_dir(directory.0.join("folder.xcf")).unwrap();
    std::fs::File::create(directory.0.join("large.xcf")).unwrap().set_len((16 << 20) + 1).unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&directory.0), None).unwrap();
    let q = quota();
    for path in ["../outside.xcf", "folder.xcf", "large.xcf", "corrupt.xcf", "bad\0name", ""] {
        assert!(workspace.open_owned_plain_xcf_with_read_control(path, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let absent = AuthorizedWorkspace::new(None, None).unwrap();
    assert!(
        matches!(absent.open_owned_plain_xcf_with_read_control("corrupt.xcf", q.clone(), &ctl()), Err(AutomationError::BadRequest(message)) if message.contains(DENIED))
    );
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[cfg(unix)]
#[test]
fn root_fifo_never_waits_for_writer_and_cancel_does_not_read() {
    let directory = Directory::new();
    let path = directory.0.join("fifo.xcf");
    rustix::fs::mkfifoat(rustix::fs::CWD, &path, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR).unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&directory.0), None).unwrap();
    let q = quota();
    assert!(matches!(workspace.open_owned_plain_xcf_with_read_control("fifo.xcf", q.clone(), &ctl()), Err(AutomationError::BadRequest(_))));
    let deny = |_: u64| Err(BackingError::Cancelled);
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(matches!(
        workspace.open_owned_plain_xcf_with_read_control("fifo.xcf", q.clone(), &cancelled),
        Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn combined_root_route_has_no_ambient_fallback_or_failed_decode_retry() {
    let directory = Directory::new();
    std::fs::write(directory.0.join("original.xcf"), b"one held root original").unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&directory.0), None).unwrap();
    let q = quota();
    for path in ["../outside.xcf", "original.xcf"] {
        assert!(workspace.open_owned_current_xcf_with_read_control(path, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let absent = AuthorizedWorkspace::new(None, None).unwrap();
    assert!(
        matches!(absent.open_owned_current_xcf_with_read_control("original.xcf", q.clone(), &ctl()), Err(AutomationError::BadRequest(message)) if message.contains(DENIED))
    );
    let calls = AtomicU64::new(0);
    let result: Result<(), AutomationError> = workspace.open_owned_xcf_with("original.xcf", q.clone(), &ctl(), |name, _, _, _, _, actual, _| {
        calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(name, "original.xcf");
        assert!(actual.shares_ledger(&q));
        Err(photocraft_io::IoError::Unsupported("selected family refused".into()))
    });
    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
