//! Verified owned byte storage and native wrappers, independent of source paths and job lifetimes.
//!
//! This trusted adapter is not a filesystem capability/provider or a default import
//! route. Incoming storage construction/authority belongs to an audited service.
//! No whole-plane allocation fallback, serialized authority, or native_large claim.
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease};
use photocraft_raster::backing::{BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl};
use std::sync::Mutex;
#[cfg(any(windows, target_os = "linux"))]
pub mod provider;
#[cfg(any(windows, target_os = "linux"))]
pub mod generated;

pub const NATIVE_STORE_BLOCK_BYTES: usize = 4096;
/// Encoded standard-profile ceiling: native1GiB, wasm256MiB. Actual storage,
/// digest index, scratch and resident capacity are admitted independently.
#[cfg(not(target_arch = "wasm32"))]
pub const NATIVE_STORE_MAX_BLOCKS: u32 = 262_144;
#[cfg(target_arch = "wasm32")]
pub const NATIVE_STORE_MAX_BLOCKS: u32 = 65_536;
pub const NATIVE_STORE_MAX_READ_BYTES: usize = 65_536;
#[derive(Clone, Copy, Debug)]
pub struct NativeStoreLimits {
    pub max_blocks: u32,
}
impl Default for NativeStoreLimits {
    fn default() -> Self {
        Self { max_blocks: NATIVE_STORE_MAX_BLOCKS }
    }
}
/// TRUSTED implementation contract. Own exclusive empty storage with no write
/// aliases; seal irreversibly prevents writes. Destroy storage on drop. Each call
/// must be bounded and return typed IO/integrity errors, never default bytes.
/// resident_envelope includes all handle/buffer capacities that can be retained
/// after capture, before any write. resident_bytes is the actual sealed capacity.
/// Incoming provider allocations are its responsibility; this adapter precharges
/// them before retention and all further writes. No borrowed controls may escape.
/// Standard Mutex/owner construction has a bounded allocator limitation.
pub trait PersistentNativeStorage: std::fmt::Debug + Send + 'static {
    /// Precharged owned handle resources may be excluded from the envelope only
    /// when their live lease stays inside this handle and this hook enforces its ledger.
    /// The default is a trusted external-accounting contract for existing handles.
    fn validate_quota(&self, _quota: &BackingQuota) -> Result<(), BackingError> {
        Ok(())
    }
    /// Exact planned native Scratch already owned by this same-ledger handle.
    /// Zero means the adapter must reserve the storage itself before writes.
    fn retained_scratch_bytes(&self) -> u64 {
        0
    }
    fn resident_envelope(&self) -> u64;
    fn resident_bytes(&self) -> u64;
    fn len(&mut self) -> Result<u64, BackingError>;
    fn is_empty(&mut self) -> Result<bool, BackingError> {
        Ok(self.len()? == 0)
    }
    fn read_exact_at(&mut self, offset: u64, output: &mut [u8]) -> Result<(), BackingError>;
    fn write_all_at(&mut self, offset: u64, input: &[u8]) -> Result<(), BackingError>;
    fn seal(&mut self) -> Result<(), BackingError>;
}
struct OwnedStorage<H> {
    handle: H,
    _scratch: Option<QuotaLease>,
    _resident: Option<QuotaLease>,
}
struct OwnedHashes {
    values: Vec<[u8; 32]>,
    _lease: QuotaLease,
}
struct Buffer {
    bytes: Vec<u8>,
    _lease: QuotaLease,
}
/// Sealed standard-profile bytes and per-block digests own their persistent leases until
/// the last shared store owner drops. No operation callback or path is retained.
/// Each read <=64KiB is transactional: caller output changes only after every
/// touched block passes integrity checks and the final cancellation/work check.
pub struct VerifiedOwnedBytes<H: PersistentNativeStorage> {
    storage: Mutex<OwnedStorage<H>>,
    hashes: OwnedHashes,
    length: u64,
    quota: BackingQuota,
}
impl<H: PersistentNativeStorage> std::fmt::Debug for VerifiedOwnedBytes<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedOwnedBytes").field("length", &self.length).field("blocks", &self.hashes.values.len()).finish_non_exhaustive()
    }
}
fn buffer(quota: &BackingQuota, length: usize) -> Result<Buffer, BackingError> {
    let envelope = length.checked_mul(2).ok_or(BackingError::Bounds)?;
    let lease = quota.reserve(QuotaKind::Resident, envelope as u64).map_err(|_| BackingError::Quota)?;
    let mut out = Buffer { bytes: Vec::new(), _lease: lease };
    out.bytes.try_reserve_exact(length).map_err(|_| BackingError::Allocation)?;
    out._lease.reconcile_capacity(out.bytes.capacity() as u64).map_err(|_| BackingError::Quota)?;
    out.bytes.resize(length, 0);
    Ok(out)
}
impl<H: PersistentNativeStorage> VerifiedOwnedBytes<H> {
    /// Copies an exact-length immutable byte payload from an independently validated
    /// caller source. Hashes establish storage consistency, not pixel admission
    /// or a GIMP display oracle. Capture failure publishes no store.
    pub fn capture(
        handle: H,
        length: u64,
        quota: BackingQuota,
        limits: NativeStoreLimits,
        control: &ReadControl<'_>,
        mut source_read: impl FnMut(u64, &mut [u8], &ReadControl<'_>) -> Result<(), BackingError>,
    ) -> Result<Self, BackingError> {
        control.check(1)?;
        handle.validate_quota(&quota)?;
        if length == 0 {
            return Err(BackingError::Bounds);
        }
        let blocks = length.checked_add((NATIVE_STORE_BLOCK_BYTES - 1) as u64).ok_or(BackingError::Bounds)? / NATIVE_STORE_BLOCK_BYTES as u64;
        if limits.max_blocks == 0 || limits.max_blocks > NATIVE_STORE_MAX_BLOCKS || blocks > u64::from(limits.max_blocks) {
            return Err(BackingError::Bounds);
        }
        let owner = handle.resident_envelope().checked_add(std::mem::size_of::<Self>() as u64).and_then(|n| n.checked_add(512)).ok_or(BackingError::Bounds)?;
        // Handle is moved before leases so its physical storage drops first on failure.
        let mut storage = OwnedStorage { handle, _scratch: None, _resident: None };
        storage._resident = Some(quota.reserve(QuotaKind::Resident, owner).map_err(|_| BackingError::Quota)?);
        match storage.handle.retained_scratch_bytes() {
            0 => storage._scratch = Some(quota.reserve(QuotaKind::Scratch, length).map_err(|_| BackingError::Quota)?),
            bytes if bytes == length => {}
            _ => return Err(BackingError::Quota),
        }
        control.check(1)?;
        if !storage.handle.is_empty()? {
            return Err(BackingError::Integrity);
        }
        let count = usize::try_from(blocks).map_err(|_| BackingError::Bounds)?;
        let index_envelope =
            count.checked_mul(32).and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(std::mem::size_of::<OwnedHashes>())).ok_or(BackingError::Bounds)?;
        let lease = quota.reserve(QuotaKind::Index, index_envelope as u64).map_err(|_| BackingError::Quota)?;
        control.check(index_envelope as u64)?;
        let mut hashes = OwnedHashes { values: Vec::new(), _lease: lease };
        hashes.values.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
        let actual = hashes.values.capacity().checked_mul(32).and_then(|n| n.checked_add(std::mem::size_of::<OwnedHashes>())).ok_or(BackingError::Bounds)?;
        hashes._lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Quota)?;
        control.check((NATIVE_STORE_BLOCK_BYTES * 2) as u64)?;
        let mut block = buffer(&quota, NATIVE_STORE_BLOCK_BYTES)?;
        for index in 0..count {
            let offset = (index as u64).checked_mul(NATIVE_STORE_BLOCK_BYTES as u64).ok_or(BackingError::Bounds)?;
            let bytes = usize::try_from((length - offset).min(NATIVE_STORE_BLOCK_BYTES as u64)).map_err(|_| BackingError::Bounds)?;
            let target = block.bytes.get_mut(..bytes).ok_or(BackingError::Length)?;
            control.check(bytes as u64)?;
            source_read(offset, target, control)?;
            control.check(bytes as u64)?;
            storage.handle.write_all_at(offset, target)?;
            control.check(bytes as u64)?;
            hashes.values.push(*blake3::hash(target).as_bytes());
        }
        control.check(1)?;
        storage.handle.seal()?;
        control.check(1)?;
        if storage.handle.len()? != length {
            return Err(BackingError::Integrity);
        }
        let actual_owner =
            storage.handle.resident_bytes().checked_add(std::mem::size_of::<Self>() as u64).and_then(|n| n.checked_add(512)).ok_or(BackingError::Bounds)?;
        storage._resident.as_mut().ok_or(BackingError::Quota)?.reconcile_capacity(actual_owner).map_err(|_| BackingError::Quota)?;
        // Independent read-back catches write/flush/seal faults before publication.
        for (index, expected) in hashes.values.iter().enumerate() {
            let offset = (index as u64).checked_mul(NATIVE_STORE_BLOCK_BYTES as u64).ok_or(BackingError::Bounds)?;
            let count = usize::try_from((length - offset).min(NATIVE_STORE_BLOCK_BYTES as u64)).map_err(|_| BackingError::Bounds)?;
            let target = block.bytes.get_mut(..count).ok_or(BackingError::Length)?;
            control.check(count as u64)?;
            storage.handle.read_exact_at(offset, target)?;
            control.check(count as u64)?;
            if blake3::hash(target).as_bytes() != expected {
                return Err(BackingError::Integrity);
            }
        }
        control.check(1)?;
        drop(block);
        Ok(Self { storage: Mutex::new(storage), hashes, length, quota })
    }
}
impl<H: PersistentNativeStorage> VerifiedOwnedBytes<H> {
    /// Exact nonzero sealed byte length; no pixel interpretation.
    pub fn len(&self) -> u64 {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    pub fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        let end = offset.checked_add(output.len() as u64).ok_or(BackingError::Bounds)?;
        let length = self.length;
        if end > length || output.len() > NATIVE_STORE_MAX_READ_BYTES {
            return Err(BackingError::Bounds);
        }
        if output.is_empty() {
            return Ok(());
        }
        control.check((output.len() as u64).checked_mul(2).ok_or(BackingError::Bounds)?)?;
        let mut staged = buffer(&self.quota, output.len())?;
        control.check((NATIVE_STORE_BLOCK_BYTES * 2) as u64)?;
        let mut block = buffer(&self.quota, NATIVE_STORE_BLOCK_BYTES)?;
        let first = offset / NATIVE_STORE_BLOCK_BYTES as u64;
        let last = (end - 1) / NATIVE_STORE_BLOCK_BYTES as u64;
        for index in first..=last {
            let block_offset = index.checked_mul(NATIVE_STORE_BLOCK_BYTES as u64).ok_or(BackingError::Bounds)?;
            let count = usize::try_from((length - block_offset).min(NATIVE_STORE_BLOCK_BYTES as u64)).map_err(|_| BackingError::Bounds)?;
            let target = block.bytes.get_mut(..count).ok_or(BackingError::Length)?;
            control.check(count as u64)?;
            {
                let mut storage = self.storage.lock().map_err(|_| BackingError::Integrity)?;
                storage.handle.read_exact_at(block_offset, target)?;
            }
            control.check(count as u64)?;
            let expected = self.hashes.values.get(usize::try_from(index).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Integrity)?;
            if blake3::hash(target).as_bytes() != expected {
                return Err(BackingError::Integrity);
            }
            let begin = offset.max(block_offset);
            let finish = end.min(block_offset.checked_add(count as u64).ok_or(BackingError::Bounds)?);
            let source_start = usize::try_from(begin - block_offset).map_err(|_| BackingError::Bounds)?;
            let destination_start = usize::try_from(begin - offset).map_err(|_| BackingError::Bounds)?;
            let bytes = usize::try_from(finish - begin).map_err(|_| BackingError::Bounds)?;
            control.check(bytes as u64)?;
            staged
                .bytes
                .get_mut(destination_start..destination_start.checked_add(bytes).ok_or(BackingError::Bounds)?)
                .ok_or(BackingError::Length)?
                .copy_from_slice(target.get(source_start..source_start.checked_add(bytes).ok_or(BackingError::Bounds)?).ok_or(BackingError::Length)?);
        }
        control.check(output.len() as u64)?;
        output.copy_from_slice(&staged.bytes);
        Ok(())
    }
}

/// Native descriptor wrapper over exact verified bytes. The existing public API
/// and admission responsibility are unchanged; ZIP bytes use a different wrapper.
#[derive(Debug)]
pub struct VerifiedNativePlaneStore<H: PersistentNativeStorage> {
    bytes: VerifiedOwnedBytes<H>,
    descriptor: NativePlaneDescriptor,
}
impl<H: PersistentNativeStorage> VerifiedNativePlaneStore<H> {
    pub fn capture(
        handle: H,
        descriptor: NativePlaneDescriptor,
        quota: BackingQuota,
        limits: NativeStoreLimits,
        control: &ReadControl<'_>,
        source_read: impl FnMut(u64, &mut [u8], &ReadControl<'_>) -> Result<(), BackingError>,
    ) -> Result<Self, BackingError> {
        let bytes = VerifiedOwnedBytes::capture(handle, descriptor.logical_bytes(), quota, limits, control, source_read)?;
        Ok(Self { bytes, descriptor })
    }
}
impl<H: PersistentNativeStorage> NativePlaneStore for VerifiedNativePlaneStore<H> {
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        self.bytes.validate_quota(quota)
    }
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        self.bytes.read_exact_at(offset, output, control)
    }
}

/// Immutable canonical archive capability. Owns actual archive bytes, never a
/// fictitious pixel descriptor or locator. ZIP validation belongs to format's
/// CanonicalArchivePlaneStore, and the standard loader is unchanged.
#[derive(Debug)]
pub struct OwnedCanonicalArchive<H: PersistentNativeStorage> {
    bytes: VerifiedOwnedBytes<H>,
}
impl<H: PersistentNativeStorage> OwnedCanonicalArchive<H> {
    pub fn capture(
        handle: H,
        length: u64,
        quota: BackingQuota,
        limits: NativeStoreLimits,
        control: &ReadControl<'_>,
        source_read: impl FnMut(u64, &mut [u8], &ReadControl<'_>) -> Result<(), BackingError>,
    ) -> Result<Self, BackingError> {
        Ok(Self { bytes: VerifiedOwnedBytes::capture(handle, length, quota, limits, control, source_read)? })
    }
}
impl<H: PersistentNativeStorage> photocraft_format::canonical_archive::ImmutableCanonicalArchive for OwnedCanonicalArchive<H> {
    fn len(&self) -> u64 {
        self.bytes.len()
    }
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        self.bytes.validate_quota(quota)
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        self.bytes.read_exact_at(offset, output, control)
    }
}
