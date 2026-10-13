//! Exclusive generated archive output; external sequential capture is unchanged.
//! No destination locator or write alias escapes. Authentication proves storage
//! consistency only: canonical CRC/hash/metadata validation is still required.
use super::{
    NATIVE_STORE_BLOCK_BYTES, NATIVE_STORE_MAX_BLOCKS, NATIVE_STORE_MAX_READ_BYTES, NativeStoreLimits, OwnedCanonicalArchive, OwnedHashes, OwnedStorage,
    PersistentNativeStorage, VerifiedOwnedBytes, buffer,
};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    sync::Mutex,
};

/// File drops before its leases. Control is borrowed only during production.
pub struct GeneratedArchiveDestination<'p, 'c> {
    file: File,
    quota: BackingQuota,
    resident: QuotaLease,
    scratch: QuotaLease,
    control: &'p ReadControl<'c>,
    max_bytes: u64,
    length: u64,
    position: u64,
    poisoned: bool,
}
impl<'p, 'c> GeneratedArchiveDestination<'p, 'c> {
    pub(crate) fn handle_envelope() -> u64 {
        std::mem::size_of::<Self>() as u64 + std::mem::size_of::<GeneratedStorage>() as u64 + 1024
    }
    pub(crate) fn from_authorized_handle(
        file: File,
        quota: BackingQuota,
        resident: QuotaLease,
        scratch: QuotaLease,
        max_bytes: u64,
        control: &'p ReadControl<'c>,
    ) -> Self {
        Self { file, quota, resident, scratch, control, max_bytes, length: 0, position: 0, poisoned: false }
    }
    fn check(&self, work: u64) -> Result<(), BackingError> {
        if self.poisoned {
            return Err(BackingError::Integrity);
        }
        self.control.check(work)
    }
    fn write_checked(&mut self, input: &[u8]) -> Result<usize, BackingError> {
        self.check(1)?;
        let count = input.len().min(NATIVE_STORE_MAX_READ_BYTES);
        let end = self.position.checked_add(count as u64).ok_or(BackingError::Bounds)?;
        if end > self.max_bytes || self.position > self.length {
            return Err(BackingError::Bounds);
        }
        let new_length = self.length.max(end);
        self.scratch.try_resize(new_length).map_err(|_| BackingError::Quota)?;
        self.file.seek(SeekFrom::Start(self.position)).map_err(|_| BackingError::Io)?;
        let mut done = 0usize;
        while done < count {
            let stop = done.checked_add(NATIVE_STORE_BLOCK_BYTES).ok_or(BackingError::Bounds)?.min(count);
            let mut attempts = 0;
            while done < stop {
                if attempts >= 32 {
                    return Err(BackingError::Io);
                }
                attempts += 1;
                self.check((stop - done) as u64)?;
                let bytes = input.get(done..stop).ok_or(BackingError::Bounds)?;
                match self.file.write(bytes) {
                    Ok(0) => return Err(BackingError::Io),
                    Ok(n) if n <= bytes.len() => done = done.checked_add(n).ok_or(BackingError::Bounds)?,
                    Ok(_) => return Err(BackingError::Io),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => return Err(BackingError::Io),
                }
            }
        }
        self.check(1)?;
        self.length = new_length;
        self.position = end;
        Ok(count)
    }
    fn seek_checked(&mut self, from: SeekFrom) -> Result<u64, BackingError> {
        self.check(1)?;
        let position = match from {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => self.position.checked_add_signed(n).ok_or(BackingError::Bounds)?,
            SeekFrom::End(n) => self.length.checked_add_signed(n).ok_or(BackingError::Bounds)?,
        };
        if position > self.length {
            return Err(BackingError::Bounds);
        }
        self.file.seek(SeekFrom::Start(position)).map_err(|_| BackingError::Io)?;
        self.check(1)?;
        self.position = position;
        Ok(position)
    }
    /// Consumes the only write owner. Flush is logical, not a durability fsync.
    /// No callback/job/directory reference is retained by the resulting archive.
    pub fn seal_verified(mut self, limits: NativeStoreLimits, control: &ReadControl<'_>) -> Result<OwnedCanonicalArchive<GeneratedStorage>, BackingError> {
        self.check(1)?;
        control.check(1)?;
        if self.length == 0 || self.length != self.scratch.bytes() || self.file.metadata().map_err(|_| BackingError::Io)?.len() != self.length {
            return Err(BackingError::Integrity);
        }
        self.flush().map_err(|e| e.get_ref().and_then(|e| e.downcast_ref::<BackingError>()).copied().unwrap_or(BackingError::Io))?;
        let length = self.length;
        let quota = self.quota.clone();
        let storage = GeneratedStorage { file: self.file, quota: self.quota, _resident: self.resident, _scratch: self.scratch, length };
        authenticate_sealed(storage, length, quota, limits, control)
    }
}
impl Write for GeneratedArchiveDestination<'_, '_> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let result = self.write_checked(input);
        if result.is_err() {
            self.poisoned = true;
        }
        result.map_err(io::Error::other)
    }
    fn flush(&mut self) -> io::Result<()> {
        let result = (|| {
            self.check(1)?;
            self.file.flush().map_err(|_| BackingError::Io)?;
            self.check(1)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result.map_err(io::Error::other)
    }
}
impl Seek for GeneratedArchiveDestination<'_, '_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let result = self.seek_checked(from);
        if result.is_err() {
            self.poisoned = true;
        }
        result.map_err(io::Error::other)
    }
}
/// Already sealed generated storage, with no public constructor or extraction.
pub struct GeneratedStorage {
    file: File,
    quota: BackingQuota,
    _resident: QuotaLease,
    _scratch: QuotaLease,
    length: u64,
}
impl std::fmt::Debug for GeneratedStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedStorage").field("length", &self.length).finish_non_exhaustive()
    }
}
impl PersistentNativeStorage for GeneratedStorage {
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
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
        if out.len() > NATIVE_STORE_BLOCK_BYTES || offset.checked_add(out.len() as u64).ok_or(BackingError::Bounds)? > self.length {
            return Err(BackingError::Bounds);
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| BackingError::Io)?;
        let mut done = 0;
        for _ in 0..32 {
            if done == out.len() {
                return Ok(());
            }
            let bytes = out.get_mut(done..).ok_or(BackingError::Bounds)?;
            match self.file.read(bytes) {
                Ok(0) => return Err(BackingError::Io),
                Ok(n) if n <= bytes.len() => done = done.checked_add(n).ok_or(BackingError::Bounds)?,
                Ok(_) => return Err(BackingError::Io),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(BackingError::Io),
            }
        }
        if done == out.len() { Ok(()) } else { Err(BackingError::Io) }
    }
    fn write_all_at(&mut self, _: u64, _: &[u8]) -> Result<(), BackingError> {
        Err(BackingError::Integrity)
    }
    fn seal(&mut self) -> Result<(), BackingError> {
        Ok(())
    }
}
// Separate generated authentication: never route nonempty storage through external capture.
fn authenticate_sealed<H: PersistentNativeStorage>(
    handle: H,
    length: u64,
    quota: BackingQuota,
    limits: NativeStoreLimits,
    control: &ReadControl<'_>,
) -> Result<OwnedCanonicalArchive<H>, BackingError> {
    control.check(1)?;
    handle.validate_quota(&quota)?;
    let blocks = length.checked_add((NATIVE_STORE_BLOCK_BYTES - 1) as u64).ok_or(BackingError::Bounds)? / NATIVE_STORE_BLOCK_BYTES as u64;
    if length == 0
        || limits.max_blocks == 0
        || limits.max_blocks > NATIVE_STORE_MAX_BLOCKS
        || blocks > u64::from(limits.max_blocks)
        || handle.retained_scratch_bytes() != length
    {
        return Err(BackingError::Bounds);
    }
    let owner = handle
        .resident_envelope()
        .checked_add(std::mem::size_of::<VerifiedOwnedBytes<H>>() as u64)
        .and_then(|n| n.checked_add(512))
        .ok_or(BackingError::Bounds)?;
    let mut storage = OwnedStorage { handle, _scratch: None, _resident: None };
    storage._resident = Some(quota.reserve(QuotaKind::Resident, owner).map_err(|_| BackingError::Quota)?);
    if storage.handle.len()? != length {
        return Err(BackingError::Integrity);
    }
    let count = usize::try_from(blocks).map_err(|_| BackingError::Bounds)?;
    let envelope = count.checked_mul(64).and_then(|n| n.checked_add(std::mem::size_of::<OwnedHashes>())).ok_or(BackingError::Bounds)?;
    let lease = quota.reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
    control.check(envelope as u64)?;
    let mut hashes = OwnedHashes { values: Vec::new(), _lease: lease };
    hashes.values.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
    let actual = hashes.values.capacity().checked_mul(32).and_then(|n| n.checked_add(std::mem::size_of::<OwnedHashes>())).ok_or(BackingError::Bounds)?;
    hashes._lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Quota)?;
    let hasher_lease = quota.reserve(QuotaKind::Resident, std::mem::size_of::<blake3::Hasher>() as u64).map_err(|_| BackingError::Quota)?;
    let mut block = buffer(&quota, NATIVE_STORE_BLOCK_BYTES)?;
    for pass in 0..2 {
        for i in 0..count {
            let offset = (i as u64).checked_mul(NATIVE_STORE_BLOCK_BYTES as u64).ok_or(BackingError::Bounds)?;
            let n = usize::try_from(length.checked_sub(offset).ok_or(BackingError::Bounds)?.min(NATIVE_STORE_BLOCK_BYTES as u64))
                .map_err(|_| BackingError::Bounds)?;
            let bytes = block.bytes.get_mut(..n).ok_or(BackingError::Length)?;
            control.check(n as u64)?;
            storage.handle.read_exact_at(offset, bytes)?;
            control.check(n as u64)?;
            let digest = *blake3::hash(bytes).as_bytes();
            if pass == 0 {
                hashes.values.push(digest);
            } else if hashes.values.get(i) != Some(&digest) {
                return Err(BackingError::Integrity);
            }
        }
    }
    if storage.handle.len()? != length {
        return Err(BackingError::Integrity);
    }
    let actual_owner = storage
        .handle
        .resident_bytes()
        .checked_add(std::mem::size_of::<VerifiedOwnedBytes<H>>() as u64)
        .and_then(|n| n.checked_add(512))
        .ok_or(BackingError::Bounds)?;
    storage._resident.as_mut().ok_or(BackingError::Quota)?.reconcile_capacity(actual_owner).map_err(|_| BackingError::Quota)?;
    control.check(1)?;
    drop(block);
    drop(hasher_lease);
    Ok(OwnedCanonicalArchive { bytes: VerifiedOwnedBytes { storage: Mutex::new(storage), hashes, length, quota } })
}
#[derive(Debug)]
pub enum GeneratedArchiveError {
    Backing(BackingError),
    Format(photocraft_format::finite_raster_archive::FiniteArchiveError),
}
impl std::fmt::Display for GeneratedArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "generated archive: {self:?}")
    }
}
impl std::error::Error for GeneratedArchiveError {}
/// Direct output without an archive Vec. Canonical load verification is separate.
pub fn save_generated_finite_raster_archive(
    service: &crate::xcf_source::ServiceAuthorizedScratchDirectory<'_, '_>,
    layer: &photocraft_doc::BackedLayer,
    limits: photocraft_format::finite_raster_archive::FiniteArchiveLimits,
    native_limits: NativeStoreLimits,
    max_bytes: u64,
    control: &ReadControl<'_>,
) -> Result<OwnedCanonicalArchive<GeneratedStorage>, GeneratedArchiveError> {
    if native_limits.max_blocks == 0
        || native_limits.max_blocks > NATIVE_STORE_MAX_BLOCKS
        || max_bytes == 0
        || max_bytes > u64::from(NATIVE_STORE_MAX_BLOCKS) * NATIVE_STORE_BLOCK_BYTES as u64
    {
        return Err(GeneratedArchiveError::Backing(BackingError::Bounds));
    }
    let cap = max_bytes.min(u64::from(native_limits.max_blocks) * NATIVE_STORE_BLOCK_BYTES as u64);
    let output = service.create_generated_archive_destination(cap, layer.quota(), control).map_err(GeneratedArchiveError::Backing)?;
    let output = photocraft_format::finite_raster_archive::save_finite_raster_archive(layer, output, limits, control).map_err(GeneratedArchiveError::Format)?;
    output.seal_verified(native_limits, control).map_err(GeneratedArchiveError::Backing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};
    #[derive(Debug)]
    struct FaultStorage {
        bytes: Vec<u8>,
        scratch: QuotaLease,
        reads: usize,
        fault: u8,
    }
    impl PersistentNativeStorage for FaultStorage {
        fn retained_scratch_bytes(&self) -> u64 {
            self.scratch.bytes()
        }
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
            self.reads += 1;
            if self.fault == 1 && self.reads == 2 {
                return Err(BackingError::Io);
            }
            let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
            let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
            out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
            if self.fault == 2
                && self.reads == 2
                && let Some(byte) = out.first_mut()
            {
                *byte ^= 1;
            }
            Ok(())
        }
        fn write_all_at(&mut self, _: u64, _: &[u8]) -> Result<(), BackingError> {
            Err(BackingError::Integrity)
        }
        fn seal(&mut self) -> Result<(), BackingError> {
            Ok(())
        }
    }
    #[test]
    fn generated_authentication_readback_io_and_corruption_discard_all_owners() {
        for fault in [1, 2] {
            let q = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 1 << 20, index: 1 << 20 });
            let scratch = q.reserve(QuotaKind::Scratch, 100).unwrap();
            let handle = FaultStorage { bytes: vec![42; 100], scratch, reads: 0, fault };
            let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
            let result = authenticate_sealed(handle, 100, q.clone(), NativeStoreLimits::default(), &ctl);
            assert_eq!(result.unwrap_err(), if fault == 1 { BackingError::Io } else { BackingError::Integrity });
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().scratch, 0);
            assert_eq!(q.usage().unwrap().index, 0);
        }
    }
}
