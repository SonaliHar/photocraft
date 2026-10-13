//! Owned kernel-lifetime storage created only by the explicitly authorized service.
use super::{NATIVE_STORE_BLOCK_BYTES, PersistentNativeStorage};
use photocraft_raster::backing::{
    BackingError,
    quota::{BackingQuota, QuotaLease},
};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
};
/// No path/File constructor, cloning, extraction, borrowed service or job control.
/// The File is destroyed before its persistent handle and exact native-data Scratch
/// leases. Verification/index buffers are owned by VerifiedNativePlaneStore.
pub struct OwnedNativeStorage {
    file: File,
    quota: BackingQuota,
    _lease: QuotaLease,
    _scratch: QuotaLease,
    sealed: bool,
}
impl std::fmt::Debug for OwnedNativeStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedNativeStorage").field("sealed", &self.sealed).finish_non_exhaustive()
    }
}
impl OwnedNativeStorage {
    pub(crate) fn from_authorized_handle(file: File, quota: BackingQuota, lease: QuotaLease, scratch: QuotaLease) -> Self {
        Self { file, quota, _lease: lease, _scratch: scratch, sealed: false }
    }
    pub(crate) fn handle_envelope() -> u64 {
        std::mem::size_of::<Self>() as u64 + 1024
    }
}
impl PersistentNativeStorage for OwnedNativeStorage {
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    // Already charged by our retained construction lease, not charged twice.
    fn retained_scratch_bytes(&self) -> u64 {
        self._scratch.bytes()
    }
    fn resident_envelope(&self) -> u64 {
        0
    }
    fn resident_bytes(&self) -> u64 {
        0
    }
    fn len(&mut self) -> Result<u64, BackingError> {
        self.file.metadata().map(|m| m.len()).map_err(|_| BackingError::Io)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BackingError> {
        if !self.sealed || out.len() > NATIVE_STORE_BLOCK_BYTES {
            return Err(BackingError::Bounds);
        }
        if offset.checked_add(out.len() as u64).ok_or(BackingError::Bounds)? > self._scratch.bytes() {
            return Err(BackingError::Bounds);
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| BackingError::Io)?;
        let mut done = 0;
        for _ in 0..32 {
            if done == out.len() {
                return Ok(());
            }
            let target = out.get_mut(done..).ok_or(BackingError::Bounds)?;
            match self.file.read(target) {
                Ok(0) => return Err(BackingError::Io),
                Ok(n) if n <= target.len() => done = done.checked_add(n).ok_or(BackingError::Bounds)?,
                Ok(_) => return Err(BackingError::Io),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err(BackingError::Io),
            }
        }
        if done == out.len() { Ok(()) } else { Err(BackingError::Io) }
    }
    fn write_all_at(&mut self, offset: u64, input: &[u8]) -> Result<(), BackingError> {
        if self.sealed || input.len() > NATIVE_STORE_BLOCK_BYTES {
            return Err(BackingError::Integrity);
        }
        if offset.checked_add(input.len() as u64).ok_or(BackingError::Bounds)? > self._scratch.bytes() {
            return Err(BackingError::Bounds);
        }
        if self.len()? != offset {
            return Err(BackingError::Integrity);
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| BackingError::Io)?;
        let mut done = 0;
        for _ in 0..32 {
            if done == input.len() {
                return Ok(());
            }
            let source = input.get(done..).ok_or(BackingError::Bounds)?;
            match self.file.write(source) {
                Ok(0) => return Err(BackingError::Io),
                Ok(n) if n <= source.len() => done = done.checked_add(n).ok_or(BackingError::Bounds)?,
                Ok(_) => return Err(BackingError::Io),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err(BackingError::Io),
            }
        }
        if done == input.len() { Ok(()) } else { Err(BackingError::Io) }
    }
    fn seal(&mut self) -> Result<(), BackingError> {
        if self.len()? != self._scratch.bytes() {
            return Err(BackingError::Integrity);
        }
        self.file.flush().map_err(|_| BackingError::Io)?;
        self.sealed = true;
        Ok(())
    }
}
