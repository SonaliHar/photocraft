//! UNRUN selected-path host ownership, refusal, and nonblocking-open drafts.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaLimits, QuotaUsage},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "../../../xcf/tests/common/mod.rs"]
mod fixture;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-owned-xcf-host-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
fn actual_selected_basename_source_survives_unlink_on_same_catalog_ledger() {
    let directory = Directory::new();
    // Suffix deliberately grants no format authority: the reader checks content.
    let path = directory.0.join("actual-selected.data");
    let bytes = fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    std::fs::write(&path, &bytes).unwrap();
    let q = quota();
    let initial = q.usage().unwrap();
    let work = AtomicU64::new(0);
    let charge = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let imported = open_owned_plain_xcf_selected_path(path.to_str().unwrap(), q.clone(), &control).unwrap();
    assert!(imported.document().quota().shares_ledger(&q));
    assert!(imported.source_read_only());
    assert!(!imported.preview_only());
    assert!(work.load(Ordering::Relaxed) > 2 * bytes.len() as u64);
    std::fs::remove_file(path).unwrap();
    let photocraft_doc::LayerContent::BackedRaster(layer) = &imported.document().document().layers[0].content else { panic!() };
    let mut pixel = [0; 4];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &control).unwrap();
    assert_eq!(pixel, [31, 61, 91, 255]);
    drop(imported);
    assert_eq!(q.usage().unwrap(), initial);
}

#[test]
fn memory_subset_and_capture_private_failures_never_leave_retained_fees() {
    let (options, limits) = memory_policy().unwrap();
    let standard = ResourceLimits::for_policy(Default::default()).unwrap();
    assert_eq!(limits.get(BudgetKind::Scratch), 0);
    assert!(limits.get(BudgetKind::Resident) <= 128 << 20);
    assert!(limits.get(BudgetKind::Encoded) <= 16 << 20);
    assert!(limits.get(BudgetKind::Logical) <= 16 << 20);
    assert!(limits.get(BudgetKind::Index) <= 16 << 20);
    for &kind in BudgetKind::ALL {
        assert!(limits.get(kind) <= standard.get(kind));
    }
    assert_eq!(options.limits.input_bytes as u64, limits.get(BudgetKind::Encoded).min(limits.get(BudgetKind::Logical)));
    let directory = Directory::new();
    let path = directory.0.join("invalid.xcf");
    std::fs::write(&path, b"gimp xcf v013\0").unwrap();
    let oversized = directory.0.join("oversized.xcf");
    std::fs::File::create(&oversized).unwrap().set_len((16 << 20) + 1).unwrap();
    let q = quota();
    for path in [&path, &oversized, &directory.0] {
        assert!(open_owned_plain_xcf_selected_path(path.to_str().unwrap(), q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    // Refuses path/handle bookkeeping before opening even a nonexistent path.
    let no = BackingQuota::new(QuotaLimits { resident: 0, index: 0, scratch: 0 });
    assert!(open_owned_plain_xcf_selected_path("nonexistent.xcf", no.clone(), &ctl()).is_err());
    assert_eq!(no.usage().unwrap(), QuotaUsage::default());
    let deny = |_: u64| Err(BackingError::Cancelled);
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(matches!(
        open_owned_plain_xcf_selected_path(path.to_str().unwrap(), q.clone(), &cancelled),
        Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[cfg(unix)]
#[test]
fn selected_fifo_without_writer_refuses_without_blocking_open() {
    let directory = Directory::new();
    let path = directory.0.join("input.xcf");
    rustix::fs::mkfifoat(rustix::fs::CWD, &path, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR).unwrap();
    let q = quota();
    assert!(matches!(open_owned_plain_xcf_selected_path(path.to_str().unwrap(), q.clone(), &ctl()), Err(AutomationError::BadRequest(_))));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
