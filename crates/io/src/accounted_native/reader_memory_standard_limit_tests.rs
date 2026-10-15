//! UNRUN encoded admission and sparse storage controls; no large-file fit claim.
use super::*;
use crate::native_plane_store::{NATIVE_STORE_BLOCK_BYTES, NATIVE_STORE_MAX_BLOCKS, NativeStoreLimits, PersistentNativeStorage, VerifiedOwnedBytes};
use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
struct Untouched(usize);
impl Read for Untouched {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        self.0 += 1;
        Err(std::io::Error::other("reader must not be accessed before admission"))
    }
}

#[test]
#[ignore = "UNRUN: platform encoded ceiling and real resident pre-admission precede reader access"]
fn platform_cap_and_actual_quota_remain_separate() {
    let standard = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    assert_eq!(MAX_CAPTURE_BYTES, standard.get(photocraft_xcf::BudgetKind::Encoded));
    let q = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 0, index: 1 << 20 });
    let before = q.usage().unwrap();
    let mut reader = Untouched(0);
    for length in [0, MAX_CAPTURE_BYTES + 1] {
        assert!(matches!(MemoryArchive::capture(&mut reader, length, q.clone(), &control()), Err(AccountedNativeError::Backing(BackingError::Bounds))));
    }
    assert!(matches!(MemoryArchive::capture(&mut reader, MAX_CAPTURE_BYTES, q.clone(), &control()), Err(AccountedNativeError::Backing(BackingError::Quota))));
    assert_eq!(reader.0, 0);
    assert_eq!(q.usage().unwrap(), before);
}

/// Exact sparse zero storage: supports contiguous all-zero writes, immutable
/// reads after seal, and no physical byte vector. Capture still owns its real
/// scratch/digest/resident leases and verifies every generated encoded byte.
#[derive(Debug, Default)]
struct ZeroStorage {
    length: u64,
    sealed: bool,
}
impl PersistentNativeStorage for ZeroStorage {
    fn resident_envelope(&self) -> u64 {
        0
    }
    fn resident_bytes(&self) -> u64 {
        0
    }
    fn len(&mut self) -> Result<u64, BackingError> {
        Ok(self.length)
    }
    fn read_exact_at(&mut self, offset: u64, output: &mut [u8]) -> Result<(), BackingError> {
        if !self.sealed || offset.checked_add(output.len() as u64).is_none_or(|end| end > self.length) {
            return Err(BackingError::Bounds);
        }
        output.fill(0);
        Ok(())
    }
    fn write_all_at(&mut self, offset: u64, input: &[u8]) -> Result<(), BackingError> {
        if self.sealed || offset != self.length || input.iter().any(|byte| *byte != 0) {
            return Err(BackingError::Integrity);
        }
        self.length = self.length.checked_add(input.len() as u64).ok_or(BackingError::Bounds)?;
        Ok(())
    }
    fn seal(&mut self) -> Result<(), BackingError> {
        self.sealed = true;
        Ok(())
    }
}

#[test]
#[ignore = "UNRUN: smaller caller cap and actual digest-index denial precede source bytes and refund fees"]
fn raised_platform_ceiling_does_not_raise_lower_caps_or_index_capacity() {
    let length = u64::from(NATIVE_STORE_MAX_BLOCKS) * NATIVE_STORE_BLOCK_BYTES as u64;
    let q = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: length, index: 1024 });
    let before = q.usage().unwrap();
    let mut calls = 0;
    let mut source = |_: u64, _: &mut [u8], _: &ReadControl<'_>| {
        calls += 1;
        Ok(())
    };
    assert!(matches!(
        VerifiedOwnedBytes::capture(ZeroStorage::default(), 4097, q.clone(), NativeStoreLimits { max_blocks: 1 }, &control(), &mut source),
        Err(BackingError::Bounds)
    ));
    assert!(matches!(
        VerifiedOwnedBytes::capture(ZeroStorage::default(), length, q.clone(), Default::default(), &control(), &mut source),
        Err(BackingError::Quota)
    ));
    drop(source);
    assert_eq!(calls, 0);
    assert_eq!(q.usage().unwrap(), before);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "UNRUN: sparse encoded storage above old256MiB captures and independently verifies under actual bounded memory"]
fn native_extended_encoded_storage_keeps_streaming_and_exact_readback() {
    let length = (256u64 << 20) + NATIVE_STORE_BLOCK_BYTES as u64;
    let q = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: length, index: 16 << 20 });
    let before = q.usage().unwrap();
    let store = VerifiedOwnedBytes::capture(ZeroStorage::default(), length, q.clone(), Default::default(), &control(), |_, output, c| {
        c.check(output.len() as u64)?;
        output.fill(0);
        Ok(())
    })
    .unwrap();
    assert_eq!(store.len(), length);
    let mut tail = [0xff; 4096];
    store.read_exact_at(length - tail.len() as u64, &mut tail, &control()).unwrap();
    assert_eq!(tail, [0; 4096]);
    assert!(q.usage().unwrap().resident < 16 << 20);
    assert_eq!(q.usage().unwrap().scratch, length);
    drop(store);
    assert_eq!(q.usage().unwrap(), before);
}
