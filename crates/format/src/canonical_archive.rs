//! Optional owned raw-canonical archive plane store. No standard native loader routing.
//! The compiled capability is trusted to own immutable storage and its persistent
//! charges. It confers no path/grant authority. Per-read ZIP reconstruction and
//! whole referenced object verification are deliberately bounded but expensive.
use crate::{
    FormatError,
    canonical_load::{VerificationError, verify_tile},
    zip::stream::{Limits, Reader},
};
use photocraft_geom::TileCoord;
use photocraft_raster::backing::{
    BackingError, NativePlaneDescriptor, NativePlaneStore, NativeTileVisitor, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};
/// TRUSTED owned immutable archive contract. Source consistency, storage lifetime,
/// and physical storage charging are the provider's obligations; no locator escapes.
pub trait ImmutableCanonicalArchive: std::fmt::Debug + Send + Sync + 'static {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError>;
    /// <=64KiB. Failure must never silently substitute missing bytes.
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError>;
}
#[derive(Clone, Copy, Debug)]
pub struct CanonicalTileReference {
    pub coord: TileCoord,
    pub hash: [u8; 32],
}
#[derive(Debug)]
pub struct CanonicalArchivePlaneStore {
    source: Arc<dyn ImmutableCanonicalArchive>,
    descriptor: NativePlaneDescriptor,
    refs: Vec<CanonicalTileReference>,
    quota: BackingQuota,
    _index: QuotaLease,
    _owner: QuotaLease,
}
impl CanonicalArchivePlaneStore {
    /// Input references must already be strictly coordinate-sorted, unique and
    /// intersect the operative descriptor. Holes are exact defaults; missing objects
    /// are integrity errors. Only exact canonical RawFrame STORE entries are supported.
    pub fn new(
        source: Arc<dyn ImmutableCanonicalArchive>,
        descriptor: NativePlaneDescriptor,
        refs: &[CanonicalTileReference],
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_internal(source, descriptor, refs, quota, control, false)
    }
    /// Finite wire representation: nonlogical tile padding must be the exact
    /// descriptor default, so logical reconstruction cannot erase stored bits.
    /// The older `new` contract exposes only logical bytes and is unchanged.
    pub fn new_default_padded(
        source: Arc<dyn ImmutableCanonicalArchive>,
        descriptor: NativePlaneDescriptor,
        refs: &[CanonicalTileReference],
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_internal(source, descriptor, refs, quota, control, true)
    }
    fn new_internal(
        source: Arc<dyn ImmutableCanonicalArchive>,
        descriptor: NativePlaneDescriptor,
        refs: &[CanonicalTileReference],
        quota: BackingQuota,
        control: &ReadControl<'_>,
        default_padding: bool,
    ) -> Result<Self, BackingError> {
        control.check(1)?;
        source.validate_quota(&quota)?;
        NativePlaneDescriptor::new(descriptor.format(), descriptor.bounds(), descriptor.default_bytes())?;
        if refs.len() > 4096 {
            return Err(BackingError::Bounds);
        }
        let owner = quota.reserve(QuotaKind::Resident, (std::mem::size_of::<Self>() + 128) as u64).map_err(|_| BackingError::Quota)?;
        let bytes = refs.len().checked_mul(std::mem::size_of::<CanonicalTileReference>()).ok_or(BackingError::Bounds)?;
        let mut index = quota
            .reserve(QuotaKind::Index, bytes.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or(BackingError::Bounds)? as u64)
            .map_err(|_| BackingError::Quota)?;
        let mut copied = Vec::new();
        copied.try_reserve_exact(refs.len()).map_err(|_| BackingError::Allocation)?;
        index
            .reconcile_capacity(copied.capacity().checked_mul(std::mem::size_of::<CanonicalTileReference>()).ok_or(BackingError::Bounds)? as u64)
            .map_err(|_| BackingError::Quota)?;
        let bounds = descriptor.bounds();
        let first = TileCoord::containing(bounds.x0, bounds.y0);
        let last = TileCoord::containing(bounds.x1.checked_sub(1).ok_or(BackingError::Bounds)?, bounds.y1.checked_sub(1).ok_or(BackingError::Bounds)?);
        let mut previous = None;
        for reference in refs {
            control.check(1)?;
            if previous.is_some_and(|p| p >= reference.coord)
                || reference.coord.tx < first.tx
                || reference.coord.tx > last.tx
                || reference.coord.ty < first.ty
                || reference.coord.ty > last.ty
            {
                return Err(BackingError::Bounds);
            }
            previous = Some(reference.coord);
            copied.push(*reference);
        }
        let mut archive = open(source.as_ref(), &quota, control)?;
        for reference in &copied {
            verify_tile(&mut archive, descriptor.format(), reference.hash, &quota, control, |offset, bytes| {
                if default_padding { validate_padding(descriptor, reference.coord, offset, bytes, control) } else { Ok(()) }
            })
            .map_err(verification)?;
        }
        control.check(1)?;
        Ok(Self { source, descriptor, refs: copied, quota, _index: index, _owner: owner })
    }
}
fn validate_padding(descriptor: NativePlaneDescriptor, coord: TileCoord, offset: u64, bytes: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
    control.check(bytes.len() as u64)?;
    let bpp = descriptor.format().bytes_per_pixel() as u64;
    let sample = descriptor.format().sample.bytes() as u64;
    let bounds = descriptor.bounds();
    let tx = i64::from(coord.tx) * 256;
    let ty = i64::from(coord.ty) * 256;
    for (i, byte) in bytes.iter().enumerate() {
        let at = offset.checked_add(i as u64).ok_or(BackingError::Bounds)?;
        let pixel = at / bpp;
        let x = tx + (pixel % 256) as i64;
        let y = ty + (pixel / 256) as i64;
        if x >= i64::from(bounds.x0) && x < i64::from(bounds.x1) && y >= i64::from(bounds.y0) && y < i64::from(bounds.y1) {
            continue;
        }
        let component = at % bpp;
        let native = if cfg!(target_endian = "little") { component } else { component - component % sample + sample - 1 - component % sample };
        if descriptor.default_bytes().get(native as usize) != Some(byte) {
            return Err(BackingError::Integrity);
        }
    }
    control.check(0)
}
impl NativePlaneStore for CanonicalArchivePlaneStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(quota) { self.source.validate_quota(quota) } else { Err(BackingError::Quota) }
    }
    fn visit_native_tile(&self, coord: TileCoord, control: &ReadControl<'_>, stage: &mut NativeTileVisitor<'_>) -> Result<bool, BackingError> {
        control.check(1)?;
        let Some(reference) = self.refs.iter().find(|reference| reference.coord == coord) else {
            return Ok(true);
        };
        let mut archive = open(self.source.as_ref(), &self.quota, control)?;
        if cfg!(target_endian = "little") {
            verify_tile(&mut archive, self.descriptor.format(), reference.hash, &self.quota, control, stage).map_err(verification)?;
        } else {
            // Fixed sample carry handles verifier/ZIP block boundaries that can
            // split an IEEE or integer sample. Storage is prepaid before creation.
            let size = (std::mem::size_of::<NativeEndianStage>() + 128) as u64;
            let mut lease = self.quota.reserve(QuotaKind::Resident, size).map_err(|_| BackingError::Quota)?;
            let mut native = NativeEndianStage::new();
            lease.reconcile_capacity(size).map_err(|_| BackingError::Quota)?;
            let sample = self.descriptor.format().sample.bytes();
            verify_tile(&mut archive, self.descriptor.format(), reference.hash, &self.quota, control, |offset, bytes| {
                native.emit(offset, bytes, sample, control, stage)
            })
            .map_err(verification)?;
            let length = 256u64.checked_mul(256).and_then(|n| n.checked_mul(self.descriptor.format().bytes_per_pixel() as u64)).ok_or(BackingError::Bounds)?;
            if native.carried != 0 || native.next != length {
                return Err(BackingError::Integrity);
            }
        }
        control.check(1)?;
        Ok(true)
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        let end = offset.checked_add(output.len() as u64).ok_or(BackingError::Bounds)?;
        if end > self.descriptor.logical_bytes() || output.len() > 65536 {
            return Err(BackingError::Bounds);
        }
        if output.is_empty() {
            return control.check(0);
        }
        let mut lease = self
            .quota
            .reserve(QuotaKind::Resident, (output.len() as u64).checked_mul(2).and_then(|n| n.checked_add(64)).ok_or(BackingError::Bounds)?)
            .map_err(|_| BackingError::Quota)?;
        let mut staged = Vec::new();
        staged.try_reserve_exact(output.len()).map_err(|_| BackingError::Allocation)?;
        lease.reconcile_capacity(staged.capacity() as u64).map_err(|_| BackingError::Quota)?;
        staged.resize(output.len(), 0);
        let bpp = self.descriptor.format().bytes_per_pixel() as u64;
        let sample = self.descriptor.format().sample.bytes() as u64;
        let bounds = self.descriptor.bounds();
        let width = u64::from(bounds.width());
        // Initialize private staging with exact defaults, then authenticate each
        // touched tile once. No tile-sized decoded allocation or segment index.
        for (i, byte) in staged.iter_mut().enumerate() {
            *byte = *self.descriptor.default_bytes().get(((offset + i as u64) % bpp) as usize).ok_or(BackingError::Integrity)?;
        }
        control.check(output.len() as u64)?;
        let mut archive = None;
        for reference in &self.refs {
            let tx = i64::from(reference.coord.tx) * 256;
            let ty = i64::from(reference.coord.ty) * 256;
            let x0 = tx.max(i64::from(bounds.x0));
            let x1 = (tx + 256).min(i64::from(bounds.x1));
            let y0 = ty.max(i64::from(bounds.y0));
            let y1 = (ty + 256).min(i64::from(bounds.y1));
            let mut touched = false;
            for y in y0..y1 {
                control.check(1)?;
                let row = u64::try_from(y - i64::from(bounds.y0)).map_err(|_| BackingError::Bounds)?;
                let x = u64::try_from(x0 - i64::from(bounds.x0)).map_err(|_| BackingError::Bounds)?;
                let start = row.checked_mul(width).and_then(|n| n.checked_add(x)).and_then(|n| n.checked_mul(bpp)).ok_or(BackingError::Bounds)?;
                let stop = start
                    .checked_add(u64::try_from(x1 - x0).map_err(|_| BackingError::Bounds)?.checked_mul(bpp).ok_or(BackingError::Bounds)?)
                    .ok_or(BackingError::Bounds)?;
                if start < end && stop > offset {
                    touched = true;
                    break;
                }
            }
            if !touched {
                continue;
            }
            if archive.is_none() {
                archive = Some(open(self.source.as_ref(), &self.quota, control)?);
            }
            let reader = archive.as_mut().ok_or(BackingError::Integrity)?;
            verify_tile(reader, self.descriptor.format(), reference.hash, &self.quota, control, |block_offset, block| {
                control.check(block.len() as u64)?;
                for (i, byte) in block.iter().enumerate() {
                    let canonical = block_offset.checked_add(i as u64).ok_or(BackingError::Bounds)?;
                    let native = if cfg!(target_endian = "little") { canonical } else { canonical - canonical % sample + sample - 1 - canonical % sample };
                    let pixel = native / bpp;
                    let x = tx + (pixel % 256) as i64;
                    let y = ty + (pixel / 256) as i64;
                    if x < i64::from(bounds.x0) || x >= i64::from(bounds.x1) || y < i64::from(bounds.y0) || y >= i64::from(bounds.y1) {
                        continue;
                    }
                    let row = u64::try_from(y - i64::from(bounds.y0)).map_err(|_| BackingError::Bounds)?;
                    let column = u64::try_from(x - i64::from(bounds.x0)).map_err(|_| BackingError::Bounds)?;
                    let logical = row
                        .checked_mul(width)
                        .and_then(|n| n.checked_add(column))
                        .and_then(|n| n.checked_mul(bpp))
                        .and_then(|n| n.checked_add(native % bpp))
                        .ok_or(BackingError::Bounds)?;
                    if logical >= offset && logical < end {
                        *staged.get_mut((logical - offset) as usize).ok_or(BackingError::Bounds)? = *byte;
                    }
                }
                Ok(())
            })
            .map_err(verification)?;
        }
        control.check(output.len() as u64)?;
        output.copy_from_slice(&staged);
        Ok(())
    }
}
struct NativeEndianStage {
    buffer: [u8; 4096],
    carry: [u8; 8],
    carried: usize,
    next: u64,
}
impl NativeEndianStage {
    fn new() -> Self {
        Self { buffer: [0; 4096], carry: [0; 8], carried: 0, next: 0 }
    }
    fn emit(&mut self, offset: u64, mut bytes: &[u8], sample: usize, control: &ReadControl<'_>, stage: &mut NativeTileVisitor<'_>) -> Result<(), BackingError> {
        if !matches!(sample, 1 | 2 | 4 | 8)
            || bytes.len() > self.buffer.len()
            || offset != self.next.checked_add(self.carried as u64).ok_or(BackingError::Bounds)?
        {
            return Err(BackingError::Integrity);
        }
        control.check(bytes.len() as u64)?;
        if self.carried != 0 {
            let needed = sample.checked_sub(self.carried).ok_or(BackingError::Integrity)?;
            let take = needed.min(bytes.len());
            let end = self.carried.checked_add(take).ok_or(BackingError::Bounds)?;
            self.carry.get_mut(self.carried..end).ok_or(BackingError::Bounds)?.copy_from_slice(bytes.get(..take).ok_or(BackingError::Bounds)?);
            self.carried = end;
            bytes = bytes.get(take..).ok_or(BackingError::Bounds)?;
            if self.carried == sample {
                let value = self.carry.get_mut(..sample).ok_or(BackingError::Bounds)?;
                value.reverse();
                stage(self.next, value)?;
                self.next = self.next.checked_add(sample as u64).ok_or(BackingError::Bounds)?;
                self.carried = 0;
            }
        }
        let count = (bytes.len() / sample).checked_mul(sample).ok_or(BackingError::Bounds)?;
        if count != 0 {
            let value = self.buffer.get_mut(..count).ok_or(BackingError::Bounds)?;
            value.copy_from_slice(bytes.get(..count).ok_or(BackingError::Bounds)?);
            for value in value.chunks_exact_mut(sample) {
                value.reverse();
            }
            stage(self.next, value)?;
            self.next = self.next.checked_add(count as u64).ok_or(BackingError::Bounds)?;
            bytes = bytes.get(count..).ok_or(BackingError::Bounds)?;
        }
        if !bytes.is_empty() {
            // If carry remains incomplete, the entire input was already consumed
            // above. Otherwise retain at most sample-1 exact canonical bytes.
            if self.carried != 0 {
                return Err(BackingError::Integrity);
            }
            self.carry.get_mut(..bytes.len()).ok_or(BackingError::Bounds)?.copy_from_slice(bytes);
            self.carried = bytes.len();
        }
        Ok(())
    }
}
fn verification(error: VerificationError) -> BackingError {
    match error {
        VerificationError::Backing(e) => e,
        VerificationError::Container(e) => container(e),
        VerificationError::Integrity => BackingError::Integrity,
        VerificationError::Unsupported => BackingError::Unsupported,
    }
}
fn container(error: FormatError) -> BackingError {
    match error {
        FormatError::Cancelled => BackingError::Cancelled,
        FormatError::LimitExceeded(_) => BackingError::Quota,
        FormatError::Unsupported(_) => BackingError::Unsupported,
        FormatError::Io(e) => e.get_ref().and_then(|e| e.downcast_ref::<BackingError>()).copied().unwrap_or(BackingError::Io),
        _ => BackingError::Integrity,
    }
}
pub(crate) struct ReadAtCursor<'a, 'c> {
    source: &'a dyn ImmutableCanonicalArchive,
    control: &'a ReadControl<'c>,
    position: u64,
}
impl<'a, 'c> ReadAtCursor<'a, 'c> {
    pub(crate) fn new(source: &'a dyn ImmutableCanonicalArchive, control: &'a ReadControl<'c>) -> Self {
        Self { source, control, position: 0 }
    }
}
impl Read for ReadAtCursor<'_, '_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let left = self.source.len().checked_sub(self.position).ok_or_else(|| std::io::Error::other(BackingError::Bounds))?;
        let n = out.len().min(65536).min(usize::try_from(left).unwrap_or(usize::MAX));
        let next = self.position.checked_add(n as u64).ok_or_else(|| std::io::Error::other(BackingError::Bounds))?;
        self.source
            .read_exact_at(self.position, out.get_mut(..n).ok_or_else(|| std::io::Error::other(BackingError::Bounds))?, self.control)
            .map_err(std::io::Error::other)?;
        self.position = next;
        Ok(n)
    }
}
impl Seek for ReadAtCursor<'_, '_> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let next = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => i128::from(self.source.len()) + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
        };
        let n = u64::try_from(next).map_err(|_| std::io::Error::other(BackingError::Bounds))?;
        if n > self.source.len() {
            return Err(std::io::Error::other(BackingError::Bounds));
        }
        self.position = n;
        Ok(n)
    }
}
fn open<'a, 'c>(
    source: &'a dyn ImmutableCanonicalArchive,
    quota: &BackingQuota,
    control: &'a ReadControl<'c>,
) -> Result<Reader<ReadAtCursor<'a, 'c>>, BackingError> {
    use std::sync::atomic::{AtomicU8, Ordering};
    let failure = AtomicU8::new(0);
    let work = |units| match control.check(units) {
        Ok(()) => Ok(()),
        Err(e) => {
            failure.store(
                match e {
                    BackingError::Work => 1,
                    BackingError::Cancelled => 2,
                    _ => 3,
                },
                Ordering::Relaxed,
            );
            Err(e)
        }
    };
    let wrapped = ReadControl { interrupt: control.interrupt, work: &work };
    let result = Reader::new_accounted(ReadAtCursor { source, control, position: 0 }, Limits::default(), &wrapped, quota);
    result.map_err(|e| match failure.load(Ordering::Relaxed) {
        1 => BackingError::Work,
        2 => BackingError::Cancelled,
        _ => container(e),
    })
}

#[cfg(test)]
mod native_endian_stream_tests {
    use super::*;
    #[test]
    fn split_samples_reverse_exact_integer_and_ieee_bytes_independent_of_host() {
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Ok(()) };
        for sample in [1usize, 2, 4, 8] {
            let canonical: Vec<u8> = (0..sample * 5000).map(|i| (i.wrapping_mul(79) % 256) as u8).collect();
            let expected: Vec<_> = canonical.chunks_exact(sample).flat_map(|s| s.iter().rev().copied()).collect();
            let mut state = NativeEndianStage::new();
            let mut result = Vec::new();
            let mut stage = |offset: u64, bytes: &[u8]| {
                assert_eq!(offset, result.len() as u64);
                result.extend_from_slice(bytes);
                Ok(())
            };
            let mut offset = 0usize;
            for size in [1usize, 3, 4095, 7, 2, 4096].into_iter().cycle() {
                if offset == canonical.len() {
                    break;
                }
                let end = (offset + size).min(canonical.len());
                state.emit(offset as u64, &canonical[offset..end], sample, &c, &mut stage).unwrap();
                offset = end;
            }
            assert_eq!(state.carried, 0);
            assert_eq!(state.next, canonical.len() as u64);
            assert_eq!(result, expected);
        }
    }
}
