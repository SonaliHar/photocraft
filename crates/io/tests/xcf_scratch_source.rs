#![cfg(not(target_arch = "wasm32"))]
use photocraft_io::xcf_source::{OwnedScratchXcfSnapshot, OwnedXcfSnapshot, ScratchHandle};
use photocraft_xcf::{Budget, BudgetKind, ErrorKind, ResourceLimits, ResourcePolicy, Source, XcfError};
use std::{
    cell::Cell,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    rc::Rc,
};

struct Generated {
    len: u64,
    seed: u64,
}
impl Source for Generated {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], _: &Budget<'_>) -> Result<(), XcfError> {
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = offset.wrapping_add(i as u64).wrapping_mul(37).wrapping_add(self.seed) as u8;
        }
        Ok(())
    }
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path =
            std::env::temp_dir().join(format!("photocraft-sol2-scratch-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn handle(&self) -> Disk {
        let path = self.0.join("exclusive-scratch");
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x04000000); // FILE_FLAG_DELETE_ON_CLOSE
        }
        let file = options.open(&path).unwrap();
        #[cfg(unix)]
        std::fs::remove_file(path).unwrap();
        Disk { file, sealed: false, short: false, failure: None, dropped: Rc::new(Cell::new(false)), writes: Rc::new(Cell::new(0)), cancel: None }
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        std::fs::remove_dir(&self.0).unwrap();
    }
}
struct Disk {
    file: File,
    sealed: bool,
    short: bool,
    failure: Option<&'static str>,
    dropped: Rc<Cell<bool>>,
    writes: Rc<Cell<usize>>,
    cancel: Option<Rc<Cell<bool>>>,
}
impl Drop for Disk {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}
impl ScratchHandle for Disk {
    fn resident_bytes(&self) -> u64 {
        256
    }
    fn len(&mut self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        if self.failure == Some("read-interrupted") {
            self.failure = None;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if self.failure == Some("read") {
            return Err(io::Error::other("injected read"));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        let n = if self.short { output.len().min(17) } else { output.len() };
        self.file.read(&mut output[..n])
    }
    fn write_at(&mut self, offset: u64, input: &[u8]) -> io::Result<usize> {
        assert!(!self.sealed);
        if self.failure == Some("write-interrupted") {
            self.failure = None;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if self.failure == Some("bad-count") {
            return Ok(input.len() + 1);
        }
        if self.failure == Some("write") {
            return Err(io::Error::other("injected full disk"));
        }
        if self.failure == Some("zero") {
            return Ok(0);
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.writes.set(self.writes.get() + 1);
        if self.failure != Some("cancel-seal")
            && let Some(cancel) = &self.cancel
        {
            cancel.set(true);
        }
        let n = if self.short { input.len().min(19) } else { input.len() };
        self.file.write(&input[..n])
    }
    fn seal(&mut self) -> io::Result<()> {
        if self.failure == Some("seal") {
            return Err(io::Error::other("injected flush/seal"));
        }
        if self.failure == Some("truncate") {
            self.file.set_len(0)?;
        }
        self.file.flush()?;
        self.sealed = true;
        if self.failure == Some("cancel-seal")
            && let Some(cancel) = &self.cancel
        {
            cancel.set(true);
        }
        Ok(())
    }
}
fn limits(encoded: u64, scratch: u64, resident: u64) -> ResourceLimits {
    ResourceLimits::for_policy(ResourcePolicy::NativeStandard)
        .unwrap()
        .lowered(&[("residentBytes", resident), ("encodedBytes", encoded), ("scratchBytes", scratch)])
        .unwrap()
}
fn refunded(budget: &Budget<'_>) {
    for kind in [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Scratch] {
        assert_eq!(budget.used(kind), 0);
    }
}

#[test]
fn generated_source_fits_disk_budget_without_resident_source_copy() {
    let root = Root::new();
    let budget = Budget::new(limits(131073, 131073, 16384), None);
    let mut source = Generated { len: 131073, seed: 0 };
    assert_eq!(OwnedXcfSnapshot::from_source(&mut source, &budget).err().unwrap().kind, ErrorKind::Limit);
    refunded(&budget);
    let mut snapshot = OwnedScratchXcfSnapshot::capture(&mut source, root.handle(), &budget).unwrap();
    source.seed = 99;
    source.len = 0; // Original mutation cannot influence retained reads.
    assert_eq!(source.len(), 0);
    assert_eq!(source.seed, 99);
    let mut output = [0; 113];
    snapshot.read_exact_at(65531, &mut output, &budget).unwrap();
    for (i, byte) in output.iter().enumerate() {
        assert_eq!(*byte, (65531u64 + i as u64).wrapping_mul(37) as u8);
    }
    assert_eq!(budget.used(BudgetKind::Encoded), 131073);
    assert_eq!(budget.used(BudgetKind::Scratch), 131073);
    assert_eq!(budget.used(BudgetKind::Resident), 256);
    assert!(budget.peak(BudgetKind::Resident) <= 8448);
    snapshot.read_exact_at(131073, &mut [], &budget).unwrap();
    assert_eq!(snapshot.read_exact_at(131074, &mut [], &budget).unwrap_err().kind, ErrorKind::Truncated);
    assert_eq!(snapshot.read_exact_at(u64::MAX, &mut [0], &budget).unwrap_err().kind, ErrorKind::Structure);
    drop(snapshot);
    refunded(&budget);
    assert!(std::fs::read_dir(&root.0).unwrap().next().is_none());
}
#[test]
fn quota_failures_precede_disk_growth_and_refund() {
    for (encoded, scratch, resident) in [(0, 100, 10000), (99, 100, 10000), (100, 0, 10000), (100, 99, 10000), (100, 100, 0), (100, 100, 8447)] {
        let root = Root::new();
        let handle = root.handle();
        let writes = handle.writes.clone();
        let dropped = handle.dropped.clone();
        let budget = Budget::new(limits(encoded, scratch, resident), None);
        assert_eq!(OwnedScratchXcfSnapshot::capture(&mut Generated { len: 100, seed: 0 }, handle, &budget).err().unwrap().kind, ErrorKind::Limit);
        assert_eq!(writes.get(), 0);
        assert!(dropped.get());
        refunded(&budget);
    }
    let root = Root::new();
    let budget = Budget::new(limits(100, 100, 8448), None);
    drop(OwnedScratchXcfSnapshot::capture(&mut Generated { len: 100, seed: 0 }, root.handle(), &budget).unwrap());
    refunded(&budget);
}
#[test]
fn short_io_exact_and_failures_cleanup() {
    for failure in [None, Some("write"), Some("zero"), Some("seal"), Some("truncate")] {
        let root = Root::new();
        let mut handle = root.handle();
        handle.short = true;
        handle.failure = failure;
        let dropped = handle.dropped.clone();
        let budget = Budget::new(limits(97, 97, 16384), None);
        let result = OwnedScratchXcfSnapshot::capture(&mut Generated { len: 97, seed: 0 }, handle, &budget);
        if failure.is_none() {
            let mut snapshot = result.unwrap();
            let mut output = [0; 97];
            snapshot.read_exact_at(0, &mut output, &budget).unwrap();
            for (i, byte) in output.iter().enumerate() {
                assert_eq!(*byte, (i as u64 * 37) as u8);
            }
            drop(snapshot);
        } else {
            assert!(result.is_err());
        }
        assert!(dropped.get());
        refunded(&budget);
        assert!(std::fs::read_dir(&root.0).unwrap().next().is_none());
    }
}
#[test]
fn cancel_and_work_denial_cleanup() {
    let root = Root::new();
    let cancel = Rc::new(Cell::new(false));
    let poll = || cancel.get();
    let budget = Budget::new(limits(10000, 10000, 16384), Some(&poll));
    let mut handle = root.handle();
    handle.cancel = Some(cancel.clone());
    let dropped = handle.dropped.clone();
    assert_eq!(OwnedScratchXcfSnapshot::capture(&mut Generated { len: 10000, seed: 0 }, handle, &budget).err().unwrap().kind, ErrorKind::Cancelled);
    assert!(dropped.get());
    refunded(&budget);
    let budget = Budget::new(limits(10000, 10000, 16384).lowered(&[("sampleWorkUnits", 4096)]).unwrap(), None);
    assert_eq!(OwnedScratchXcfSnapshot::capture(&mut Generated { len: 10000, seed: 0 }, root.handle(), &budget).err().unwrap().kind, ErrorKind::Limit);
    refunded(&budget);
}

#[test]
fn read_failures_interrupts_and_post_seal_cancel() {
    for failure in [Some("read"), Some("read-interrupted"), Some("write-interrupted")] {
        let root = Root::new();
        let mut handle = root.handle();
        handle.failure = failure;
        let budget = Budget::new(limits(32, 32, 16384), None);
        let mut snapshot = OwnedScratchXcfSnapshot::capture(&mut Generated { len: 32, seed: 0 }, handle, &budget).unwrap();
        let mut output = [0; 32];
        let result = snapshot.read_exact_at(0, &mut output, &budget);
        if failure == Some("read") {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Structure);
        } else {
            result.unwrap();
        }
        drop(snapshot);
        refunded(&budget);
    }
    for stage in ["cancel-seal", "write"] {
        let root = Root::new();
        let cancel = Rc::new(Cell::new(stage == "write"));
        let poll = || cancel.get();
        let budget = Budget::new(limits(32, 32, 16384), Some(&poll));
        let mut handle = root.handle();
        handle.failure = Some(stage);
        handle.cancel = Some(cancel.clone());
        let writes = handle.writes.clone();
        assert_eq!(OwnedScratchXcfSnapshot::capture(&mut Generated { len: 32, seed: 0 }, handle, &budget).err().unwrap().kind, ErrorKind::Cancelled);
        if stage == "write" {
            assert_eq!(writes.get(), 0);
        }
        refunded(&budget);
    }
}
#[test]
fn deleted_original_file_is_not_a_snapshot_locator() {
    let root = Root::new();
    let path = root.0.join("authorized-original");
    std::fs::write(&path, b"original literal input").unwrap();
    let budget = Budget::new(limits(22, 22, 16384), None);
    let mut source = photocraft_xcf::FileSource::new(File::open(&path).unwrap(), &budget).unwrap();
    let mut snapshot = OwnedScratchXcfSnapshot::capture(&mut source, root.handle(), &budget).unwrap();
    drop(source);
    std::fs::write(&path, b"changed").unwrap();
    std::fs::remove_file(path).unwrap();
    let mut output = [0; 22];
    snapshot.read_exact_at(0, &mut output, &budget).unwrap();
    assert_eq!(&output, b"original literal input");
    drop(snapshot);
    refunded(&budget);
}
#[test]
fn zero_length_and_huge_length_remain_checked_u64() {
    let root = Root::new();
    let budget = Budget::new(limits(0, 0, 8448), None);
    let mut snapshot = OwnedScratchXcfSnapshot::capture(&mut Generated { len: 0, seed: 0 }, root.handle(), &budget).unwrap();
    snapshot.read_exact_at(0, &mut [], &budget).unwrap();
    drop(snapshot);
    refunded(&budget);
    let handle = root.handle();
    let writes = handle.writes.clone();
    assert_eq!(OwnedScratchXcfSnapshot::capture(&mut Generated { len: u64::MAX, seed: 0 }, handle, &budget).err().unwrap().kind, ErrorKind::Limit);
    assert_eq!(writes.get(), 0);
    refunded(&budget);
    assert!(ResourceLimits::for_policy(ResourcePolicy::NativeLarge).is_err());
}

#[test]
fn read_budget_and_cancel_deny_before_backend_with_live_source_leases() {
    let root = Root::new();
    let mut handle = root.handle();
    handle.failure = Some("read");
    let budget = Budget::new(limits(32, 32, 16384), None);
    let mut snapshot = OwnedScratchXcfSnapshot::capture(&mut Generated { len: 32, seed: 0 }, handle, &budget).unwrap();
    let no_work = Budget::new(limits(32, 32, 16384).lowered(&[("sampleWorkUnits", 0)]).unwrap(), None);
    assert_eq!(snapshot.read_exact_at(0, &mut [0], &no_work).unwrap_err().kind, ErrorKind::Limit);
    let no_encoded = Budget::new(limits(31, 32, 16384), None);
    assert_eq!(snapshot.read_exact_at(0, &mut [0], &no_encoded).unwrap_err().kind, ErrorKind::Limit);
    let stop = || true;
    let cancelled = Budget::new(limits(32, 32, 16384), Some(&stop));
    assert_eq!(snapshot.read_exact_at(0, &mut [0], &cancelled).unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(budget.used(BudgetKind::Scratch), 32);
    assert_eq!(budget.used(BudgetKind::Encoded), 32);
    drop(snapshot);
    refunded(&budget);
}
#[test]
fn invalid_service_handle_and_creation_failure_publish_nothing() {
    let root = Root::new();
    let mut handle = root.handle();
    handle.file.write_all(b"not empty").unwrap();
    let budget = Budget::new(limits(32, 32, 16384), None);
    let writes = handle.writes.clone();
    assert_eq!(OwnedScratchXcfSnapshot::capture(&mut Generated { len: 32, seed: 0 }, handle, &budget).err().unwrap().kind, ErrorKind::Structure);
    assert_eq!(writes.get(), 0);
    refunded(&budget);
    let collision = root.0.join("already-owned");
    std::fs::write(&collision, b"keep").unwrap();
    assert!(OpenOptions::new().write(true).create_new(true).open(&collision).is_err());
    assert_eq!(std::fs::read(&collision).unwrap(), b"keep");
    std::fs::remove_file(collision).unwrap();
    refunded(&budget);
}

#[test]
fn owned_vec_spare_capacity_is_resident_charged() {
    let budget = Budget::new(limits(1, 0, 1024), None);
    let mut bytes = Vec::with_capacity(16384);
    bytes.push(42);
    assert_eq!(OwnedXcfSnapshot::from_owned(bytes, &budget).err().unwrap().kind, ErrorKind::Limit);
    refunded(&budget);
}

#[test]
fn owned_and_captured_vec_actual_capacity_leases_are_exact() {
    for resident in [16383, 16384] {
        let budget = Budget::new(limits(1, 0, resident), None);
        let mut bytes = Vec::with_capacity(16384);
        bytes.push(7);
        let result = OwnedXcfSnapshot::from_owned(bytes, &budget);
        if resident == 16383 {
            assert_eq!(result.err().unwrap().kind, ErrorKind::Limit);
        } else {
            let snapshot = result.unwrap();
            assert_eq!(budget.used(BudgetKind::Encoded), 1);
            assert_eq!(budget.used(BudgetKind::Resident), 16384);
            assert_eq!(snapshot.bytes(), &[7]);
            drop(snapshot);
        }
        refunded(&budget);
    }
    let budget = Budget::new(limits(32, 0, 64), None);
    let snapshot = OwnedXcfSnapshot::from_source(&mut Generated { len: 32, seed: 0 }, &budget).unwrap();
    assert_eq!(budget.peak(BudgetKind::Resident), 64);
    assert_eq!(budget.used(BudgetKind::Resident), 32);
    assert_eq!(snapshot.bytes().len(), 32);
    drop(snapshot);
    refunded(&budget);
    let budget = Budget::new(limits(32, 0, 63), None);
    assert_eq!(OwnedXcfSnapshot::from_source(&mut Generated { len: 32, seed: 0 }, &budget).err().unwrap().kind, ErrorKind::Limit);
    assert_eq!(budget.used(BudgetKind::Work), 0);
    refunded(&budget);
}
