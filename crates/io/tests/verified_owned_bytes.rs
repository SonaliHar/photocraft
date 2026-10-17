use photocraft_io::native_plane_store::{NativeStoreLimits, PersistentNativeStorage, VerifiedOwnedBytes};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    sealed: bool,
    fault: Arc<AtomicBool>,
    corrupt: Arc<AtomicBool>,
}
impl PersistentNativeStorage for Memory {
    fn resident_envelope(&self) -> u64 {
        self.bytes.capacity() as u64
    }
    fn resident_bytes(&self) -> u64 {
        self.bytes.capacity() as u64
    }
    fn len(&mut self) -> Result<u64, BackingError> {
        Ok(self.bytes.len() as u64)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BackingError> {
        if self.fault.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
        if self.corrupt.load(Ordering::Relaxed)
            && let Some(first) = out.first_mut()
        {
            *first ^= 1;
        }
        Ok(())
    }
    fn write_all_at(&mut self, offset: u64, input: &[u8]) -> Result<(), BackingError> {
        if self.sealed || offset != self.bytes.len() as u64 || self.bytes.len().checked_add(input.len()).is_none_or(|n| n > self.bytes.capacity()) {
            return Err(BackingError::Bounds);
        }
        self.bytes.extend_from_slice(input);
        Ok(())
    }
    fn seal(&mut self) -> Result<(), BackingError> {
        self.sealed = true;
        Ok(())
    }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 1 << 20, index: 1 << 20 })
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn storage(length: usize, fault: Arc<AtomicBool>, corrupt: Arc<AtomicBool>) -> Memory {
    Memory { bytes: Vec::with_capacity(length), sealed: false, fault, corrupt }
}
#[test]
fn arbitrary_nonpixel_byte_length_exact_read_reentrancy_and_last_owner_refund() {
    let q = quota();
    let source = (0..12347usize).map(|n| ((n * 19 + 7) % 251) as u8).collect::<Vec<_>>();
    let fault = Arc::new(AtomicBool::new(false));
    let corrupt = Arc::new(AtomicBool::new(false));
    let store = Arc::new(
        VerifiedOwnedBytes::capture(
            storage(source.len(), fault, corrupt),
            source.len() as u64,
            q.clone(),
            NativeStoreLimits::default(),
            &control(),
            |offset, out, _| {
                let start = offset as usize;
                out.copy_from_slice(&source[start..start + out.len()]);
                Ok(())
            },
        )
        .unwrap(),
    );
    assert_eq!(store.len(), source.len() as u64);
    assert!(!store.is_empty());
    assert_eq!(q.usage().unwrap().scratch, source.len() as u64);
    let shared = store.clone();
    drop(store);
    let once = AtomicBool::new(false);
    let work = |_| {
        if !once.swap(true, Ordering::Relaxed) {
            let mut nested = [0; 3];
            shared.read_exact_at(4095, &mut nested, &control())?;
            assert_eq!(nested, source[4095..4098]);
        }
        Ok(())
    };
    let mut bytes = [0; 8197];
    shared.read_exact_at(17, &mut bytes, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    assert_eq!(bytes, source[17..8214]);
    let other = quota();
    assert_eq!(shared.validate_quota(&other), Err(BackingError::Quota));
    drop(shared);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn late_io_hash_cancel_and_work_failures_leave_caller_output_and_quota_unchanged() {
    let q = quota();
    let fault = Arc::new(AtomicBool::new(false));
    let corrupt = Arc::new(AtomicBool::new(false));
    let store =
        VerifiedOwnedBytes::capture(storage(9001, fault.clone(), corrupt.clone()), 9001, q.clone(), NativeStoreLimits::default(), &control(), |_, out, _| {
            out.fill(0x65);
            Ok(())
        })
        .unwrap();
    let retained = q.usage().unwrap();
    let mut out = [0xa7; 8193];
    fault.store(true, Ordering::Relaxed);
    assert_eq!(store.read_exact_at(1, &mut out, &control()), Err(BackingError::Io));
    fault.store(false, Ordering::Relaxed);
    corrupt.store(true, Ordering::Relaxed);
    assert_eq!(store.read_exact_at(1, &mut out, &control()), Err(BackingError::Integrity));
    corrupt.store(false, Ordering::Relaxed);
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert_eq!(store.read_exact_at(1, &mut out, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(BackingError::Cancelled));
    let work = |_| Err(BackingError::Quota);
    assert_eq!(store.read_exact_at(1, &mut out, &ReadControl { interrupt: &Interrupt::NONE, work: &work }), Err(BackingError::Quota));
    assert_eq!(out, [0xa7; 8193]);
    assert_eq!(q.usage().unwrap(), retained);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn capture_length_index_limit_and_late_source_error_refund_all_storage() {
    for length in [
        0,
        u64::MAX,
        u64::from(photocraft_io::native_plane_store::NATIVE_STORE_MAX_BLOCKS) * photocraft_io::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64 + 1,
    ] {
        let q = quota();
        assert_eq!(
            VerifiedOwnedBytes::capture(
                storage(0, Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false))),
                length,
                q.clone(),
                NativeStoreLimits::default(),
                &control(),
                |_, _, _| Ok(())
            )
            .unwrap_err(),
            BackingError::Bounds
        );
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let q = quota();
    assert_eq!(
        VerifiedOwnedBytes::capture(
            storage(9001, Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false))),
            9001,
            q.clone(),
            NativeStoreLimits::default(),
            &control(),
            |offset, out, _| {
                if offset >= 4096 {
                    return Err(BackingError::Io);
                }
                out.fill(1);
                Ok(())
            }
        )
        .unwrap_err(),
        BackingError::Io
    );
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let denied = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 9001, index: 0 });
    assert_eq!(
        VerifiedOwnedBytes::capture(
            storage(9001, Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false))),
            9001,
            denied.clone(),
            NativeStoreLimits::default(),
            &control(),
            |_, _, _| Ok(())
        )
        .unwrap_err(),
        BackingError::Quota
    );
    assert_eq!(denied.usage().unwrap(), QuotaUsage::default());
}
