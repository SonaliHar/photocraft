#![cfg(any(windows, target_os = "linux"))]
use photocraft_io::xcf_source::{OwnedXcfSnapshot, ServiceAuthorizedScratchDirectory};
use photocraft_xcf::{Budget, BudgetKind, ErrorKind, ResourceLimits, ResourcePolicy, Source, XcfError};
use std::{
    fs::File,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-sol2-provider-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        std::fs::remove_dir(&self.0).unwrap();
    }
}
struct Generated {
    len: u64,
    seed: u8,
}
impl Source for Generated {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], _: &Budget<'_>) -> Result<(), XcfError> {
        for (i, v) in out.iter_mut().enumerate() {
            *v = offset.wrapping_add(i as u64).wrapping_mul(37).wrapping_add(u64::from(self.seed)) as u8;
        }
        Ok(())
    }
}
fn limits(resident: u64) -> ResourceLimits {
    ResourceLimits::for_policy(ResourcePolicy::NativeStandard)
        .unwrap()
        .lowered(&[("residentBytes", resident), ("encodedBytes", 131073), ("scratchBytes", 131073)])
        .unwrap()
}
fn empty(root: &Root) {
    assert!(std::fs::read_dir(&root.0).unwrap().next().is_none());
}
fn refunded(budget: &Budget<'_>) {
    for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
        assert_eq!(budget.used(kind), 0);
    }
}
#[test]
fn provider_metadata_denial_precedes_any_private_file_creation() {
    let root = Root::new();
    let budget = Budget::new(limits(0), None);
    assert_eq!(ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).err().unwrap().kind, ErrorKind::Limit);
    empty(&root);
    refunded(&budget);
}
#[test]
fn production_provider_keeps_source_independent_and_metadata_live() {
    let root = Root::new();
    let budget = Budget::new(limits(16384), None);
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let directory_resident = budget.used(BudgetKind::Resident);
    assert!(directory_resident > 0);
    empty(&root);
    let mut source = Generated { len: 131073, seed: 0 };
    assert_eq!(OwnedXcfSnapshot::from_source(&mut source, &budget).err().unwrap().kind, ErrorKind::Limit);
    let mut snapshot = directory.capture(&mut source).unwrap();
    let source_resident = budget.used(BudgetKind::Resident);
    assert!(source_resident > directory_resident);
    assert!(source_resident < 16384);
    assert_eq!(budget.used(BudgetKind::Scratch), 131073);
    assert_eq!(budget.used(BudgetKind::Encoded), 131073);
    source.seed = 99;
    source.len = 0;
    assert_eq!(source.len(), 0);
    assert_eq!(source.seed, 99);
    let mut output = [0; 113];
    snapshot.read_exact_at(65531, &mut output, &budget).unwrap();
    for (i, v) in output.iter().enumerate() {
        assert_eq!(*v, (65531u64 + i as u64).wrapping_mul(37) as u8);
    }
    snapshot.read_exact_at(131073, &mut [], &budget).unwrap();
    assert_eq!(snapshot.read_exact_at(u64::MAX, &mut [0], &budget).unwrap_err().kind, ErrorKind::Structure);
    drop(snapshot);
    empty(&root);
    assert_eq!(budget.used(BudgetKind::Resident), directory_resident);
    assert_eq!(budget.used(BudgetKind::Scratch), 0);
    drop(directory);
    refunded(&budget);
}
#[test]
fn production_source_deleted_after_capture_is_never_reopened() {
    let root = Root::new();
    let path = root.0.join("authorized-original");
    std::fs::write(&path, b"original").unwrap();
    let budget = Budget::new(limits(16384), None);
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let mut source = photocraft_xcf::FileSource::new(File::open(&path).unwrap(), &budget).unwrap();
    let mut snapshot = directory.capture(&mut source).unwrap();
    drop(source);
    std::fs::write(&path, b"changed").unwrap();
    std::fs::remove_file(path).unwrap();
    let mut output = [0; 8];
    snapshot.read_exact_at(0, &mut output, &budget).unwrap();
    assert_eq!(&output, b"original");
    drop(snapshot);
    drop(directory);
    empty(&root);
    refunded(&budget);
}
#[test]
fn directory_inputs_are_explicit_bounded_and_validated() {
    let root = Root::new();
    let file = root.0.join("not-a-directory");
    std::fs::write(&file, b"keep").unwrap();
    for path in [PathBuf::from("relative"), root.0.join("missing"), file.clone(), root.0.join(".."), PathBuf::from("x".repeat(4097))] {
        let budget = Budget::new(limits(16384), None);
        assert!(ServiceAuthorizedScratchDirectory::from_platform_authorized(path, &budget).is_err());
        refunded(&budget);
    }
    let mut spare = root.0.clone();
    spare.reserve(16384);
    let budget = Budget::new(limits(16384), None);
    assert_eq!(ServiceAuthorizedScratchDirectory::from_platform_authorized(spare, &budget).err().unwrap().kind, ErrorKind::Limit);
    refunded(&budget);
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    std::fs::remove_file(file).unwrap();
    empty(&root);
}
#[test]
fn canceled_or_denied_capture_keeps_directory_lease_and_no_file() {
    use std::cell::Cell;
    let root = Root::new();
    let stop = Cell::new(false);
    let poll = || stop.get();
    let budget = Budget::new(limits(16384), Some(&poll));
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let retained = budget.used(BudgetKind::Resident);
    stop.set(true);
    assert_eq!(directory.capture(&mut Generated { len: 97, seed: 0 }).err().unwrap().kind, ErrorKind::Cancelled);
    empty(&root);
    assert_eq!(budget.used(BudgetKind::Resident), retained);
    drop(directory);
    refunded(&budget);
    let budget = Budget::new(limits(16384).lowered(&[("sampleWorkUnits", 30)]).unwrap(), None);
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let retained = budget.used(BudgetKind::Resident);
    assert_eq!(directory.capture(&mut Generated { len: 10000, seed: 0 }).err().unwrap().kind, ErrorKind::Limit);
    empty(&root);
    assert_eq!(budget.used(BudgetKind::Resident), retained);
    drop(directory);
    refunded(&budget);
}
#[test]
fn concurrent_snapshots_keep_distinct_leases_then_refund() {
    let root = Root::new();
    let budget = Budget::new(limits(16384), None);
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let retained = budget.used(BudgetKind::Resident);
    let first = directory.capture(&mut Generated { len: 31, seed: 0 }).unwrap();
    let one = budget.used(BudgetKind::Resident);
    let second = directory.capture(&mut Generated { len: 47, seed: 1 }).unwrap();
    assert_eq!(budget.used(BudgetKind::Resident) - one, one - retained);
    assert_eq!(budget.used(BudgetKind::Encoded), 78);
    assert_eq!(budget.used(BudgetKind::Scratch), 78);
    drop(first);
    assert_eq!(budget.used(BudgetKind::Encoded), 47);
    assert_eq!(budget.used(BudgetKind::Resident), one);
    drop(second);
    assert_eq!(budget.used(BudgetKind::Resident), retained);
    drop(directory);
    empty(&root);
    refunded(&budget);
}

#[test]
fn failed_or_canceled_mid_capture_destroys_grown_storage_before_refunds() {
    use std::cell::Cell;
    struct Mid<'a> {
        cancel: Option<&'a Cell<bool>>,
    }
    impl Source for Mid<'_> {
        fn len(&self) -> u64 {
            8192
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8], _: &Budget<'_>) -> Result<(), XcfError> {
            if offset >= 4096 {
                if let Some(cancel) = self.cancel {
                    cancel.set(true);
                } else {
                    return Err(XcfError {
                        kind: ErrorKind::Structure,
                        offset: 0,
                        item: String::new(),
                        message: "source failed after first stored slice".into(),
                    });
                }
            }
            out.fill(42);
            Ok(())
        }
    }
    for cancelled in [false, true] {
        let root = Root::new();
        let stop = Cell::new(false);
        let poll = || stop.get();
        let budget = Budget::new(limits(16384), Some(&poll));
        let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
        let retained = budget.used(BudgetKind::Resident);
        let error = directory.capture(&mut Mid { cancel: cancelled.then_some(&stop) }).err().unwrap();
        assert_eq!(error.kind, if cancelled { ErrorKind::Cancelled } else { ErrorKind::Structure });
        assert!(budget.used(BudgetKind::Work) >= 8192);
        assert_eq!(budget.used(BudgetKind::Resident), retained);
        assert_eq!(budget.used(BudgetKind::Scratch), 0);
        empty(&root);
        drop(directory);
        refunded(&budget);
    }
}

#[cfg(windows)]
#[test]
#[ignore = "requires OS authority to create symbolic links in the isolated scratch test root"]
fn service_authorized_symlink_root_is_rejected_without_following_it() {
    let root = Root::new();
    let target = root.0.join("original-target");
    std::fs::create_dir(&target).unwrap();
    let link = root.0.join("linked-root");
    let creation = std::os::windows::fs::symlink_dir(&target, &link);
    if let Err(error) = creation {
        std::fs::remove_dir(target).unwrap();
        panic!("symlink authority unavailable: {error}");
    }
    let budget = Budget::new(limits(16384), None);
    assert_eq!(ServiceAuthorizedScratchDirectory::from_platform_authorized(link.clone(), &budget).err().unwrap().kind, ErrorKind::Structure);
    refunded(&budget);
    assert!(std::fs::read_dir(&target).unwrap().next().is_none());
    std::fs::remove_dir(link).unwrap();
    std::fs::remove_dir(target).unwrap();
    empty(&root);
}
