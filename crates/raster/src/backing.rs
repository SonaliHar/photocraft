//! Owned, file-independent lazy native planes. This module is an additive foundation,
//! not an implicit replacement for resident Surface readers or a filesystem grant.
#![forbid(unsafe_code)]

use crate::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};

#[path = "backing/current_stream.rs"]
pub mod current_stream;
#[path = "backing/quota.rs"]
pub mod quota;
use quota::{BackingQuota, QuotaKind, QuotaLease};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackingError {
    Bounds,
    Length,
    Quota,
    Allocation,
    Cancelled,
    Work,
    Io,
    Integrity,
    Unsupported,
}
impl std::fmt::Display for BackingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native backing: {self:?}")
    }
}
impl std::error::Error for BackingError {}

/// Job-local callbacks never escape into the persistent store or document.
/// Work charges cover initialized pixels, copied/read bytes and bounded bookkeeping;
/// a store may additionally charge its own IO/decode work, without retaining callbacks.
pub struct ReadControl<'a> {
    pub interrupt: &'a Interrupt<'a>,
    pub work: &'a (dyn Fn(u64) -> Result<(), BackingError> + Sync),
}
impl ReadControl<'_> {
    pub fn check(&self, units: u64) -> Result<(), BackingError> {
        self.interrupt.check().map_err(|_| BackingError::Cancelled)?;
        (self.work)(units)?;
        self.interrupt.check().map_err(|_| BackingError::Cancelled)
    }
}
/// Exact native-endian layout; logical bytes are U64, not a resident allocation request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativePlaneDescriptor {
    format: PixelFormat,
    bounds: Rect,
    default: [u8; 40],
    length: u64,
}
impl NativePlaneDescriptor {
    pub fn new(format: PixelFormat, bounds: Rect, default: &[u8]) -> Result<Self, BackingError> {
        let bpp = format.bytes_per_pixel();
        if bounds.is_empty() || default.len() != bpp || bpp == 0 || bpp > 40 {
            return Err(BackingError::Bounds);
        }
        let length = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).and_then(|n| n.checked_mul(bpp as u64)).ok_or(BackingError::Bounds)?;
        let mut pixel = [0; 40];
        pixel.get_mut(..bpp).ok_or(BackingError::Length)?.copy_from_slice(default);
        Ok(Self { format, bounds, default: pixel, length })
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    pub fn logical_bytes(&self) -> u64 {
        self.length
    }
    pub fn default_bytes(&self) -> &[u8] {
        self.default.get(..self.format.bytes_per_pixel()).unwrap_or(&[])
    }
}
/// Borrowed job-local staging callback for a native physical-tile stream.
pub type NativeTileVisitor<'a> = dyn FnMut(u64, &[u8]) -> Result<(), BackingError> + 'a;

/// TRUSTED compiled store contract: immutable, fully verified native contents for
/// descriptor(), owned storage and persistent storage leases. No locator authority.
/// read_exact_at must return Io/Integrity on short/corrupt reads; never substitute
/// default pixels. Implementations must poll bounded IO slices and charge work.
pub trait NativePlaneStore: std::fmt::Debug + Send + Sync {
    fn descriptor(&self) -> NativePlaneDescriptor;
    /// Validate persistent storage accounting before cache/COW reservations.
    /// The default is a TRUSTED externally-accounted store contract: compiled
    /// implementations must own and bound their storage independently. Stores
    /// retaining this ledger must override and require shares_ledger identity;
    /// matching limit values or current usage is never a compatibility witness.
    /// This confers no path/grant authority and performs no native data reads.
    fn validate_quota(&self, _quota: &BackingQuota) -> Result<(), BackingError> {
        Ok(())
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError>;
    /// Optional authenticated whole-tile stream in native byte order. Offsets
    /// must be contiguous from zero, every block nonempty and <=64KiB, and the
    /// complete physical tile must be covered. `Ok(true)` without callbacks is
    /// an authenticated default hole. `Ok(false)` must never call `stage` and
    /// requests the existing logical-row fallback. Every callback error must be
    /// propagated; caller staging remains private until full verification.
    fn visit_native_tile(&self, _coord: TileCoord, _control: &ReadControl<'_>, _stage: &mut NativeTileVisitor<'_>) -> Result<bool, BackingError> {
        Ok(false)
    }
}

/// Maximum bounded block offered by an optional native tile visitor.
pub const MAX_NATIVE_TILE_VISIT_BYTES: usize = 64 * 1024;

const ENTRY_ENVELOPE: u64 = 256;
const MAX_ENTRIES: usize = 4096;
#[cfg(test)]
#[path = "backing/constructor_requirements_tests.rs"]
mod constructor_requirements_tests;
/// A pin retains its physical allocation reservation independently of cache eviction.
#[derive(Debug)]
pub struct TilePin {
    payload: Arc<Payload>,
}
impl Clone for TilePin {
    fn clone(&self) -> Self {
        Self { payload: Arc::clone(&self.payload) }
    }
}
impl TilePin {
    pub fn bytes(&self) -> &[u8] {
        &self.payload.bytes
    }
}
#[derive(Debug)]
struct Payload {
    bytes: Vec<u8>,
    _lease: QuotaLease,
}
#[derive(Debug)]
struct Entries {
    values: Vec<(TileCoord, TilePin)>,
    _lease: QuotaLease,
}
#[derive(Debug)]
struct Cache {
    entries: Entries,
    capacity: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct BackingLimits {
    pub cache_tiles: usize,
    pub override_tiles: usize,
}
/// Immutable base plus exact COW overrides. Cache residency is derived state.
#[derive(Clone, Debug)]
pub struct BackedRaster {
    store: Arc<dyn NativePlaneStore>,
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    overrides: Arc<Entries>,
    override_limit: usize,
    cache: Arc<Mutex<Cache>>,
}
impl BackedRaster {
    /// Existing constructor's prospective cache/COW index reservations only.
    /// Allocation-free and lower-policy neutral: this is not a ledger receipt,
    /// store/native payload census, future COW budget or allocator/RSS claim.
    pub fn metadata_reservation_bytes(limits: BackingLimits) -> Result<u64, BackingError> {
        if limits.cache_tiles == 0 || limits.cache_tiles > MAX_ENTRIES || limits.override_tiles == 0 || limits.override_tiles > MAX_ENTRIES {
            return Err(BackingError::Bounds);
        }
        entry_reservation_bytes(0)?.checked_add(entry_reservation_bytes(limits.cache_tiles)?).ok_or(BackingError::Quota)
    }
    pub fn new(store: Arc<dyn NativePlaneStore>, quota: BackingQuota, cache_tiles: usize, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        Self::new_with_limits(store, quota, BackingLimits { cache_tiles, override_tiles: MAX_ENTRIES }, control)
    }
    pub fn new_with_limits(
        store: Arc<dyn NativePlaneStore>,
        quota: BackingQuota,
        limits: BackingLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        let cache_tiles = limits.cache_tiles;
        control.check(1)?;
        Self::metadata_reservation_bytes(limits)?;
        store.validate_quota(&quota)?;
        control.check(1)?;
        let descriptor = store.descriptor();
        NativePlaneDescriptor::new(descriptor.format(), descriptor.bounds(), descriptor.default_bytes())?;
        let overrides = Arc::new(entries(&quota, 0)?);
        let cache = Arc::new(Mutex::new(Cache { entries: entries(&quota, cache_tiles)?, capacity: cache_tiles }));
        control.check(1)?;
        Ok(Self { store, descriptor, quota, overrides, override_limit: limits.override_tiles, cache })
    }
    pub fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    /// Persistent cache/COW ledger identity, without filesystem authority.
    pub fn quota(&self) -> &BackingQuota {
        &self.quota
    }
    pub fn override_count(&self) -> usize {
        self.overrides.values.len()
    }
    /// Eviction releases only cache references; live pins/history retain payload leases.
    pub fn clear_cache(&self) -> Result<(), BackingError> {
        self.cache.lock().map_err(|_| BackingError::Integrity)?.entries.values.clear();
        Ok(())
    }
    pub fn pin_tile(&self, coord: TileCoord, control: &ReadControl<'_>) -> Result<TilePin, BackingError> {
        control.check(1)?;
        let rect = tile_rect(coord)?;
        if let Some((_, tile)) = self.overrides.values.iter().find(|(c, _)| *c == coord) {
            return Ok(tile.clone());
        }
        {
            let mut cache = self.cache.lock().map_err(|_| BackingError::Integrity)?;
            if let Some((_, tile)) = cache.entries.values.iter().find(|(c, _)| *c == coord) {
                return Ok(tile.clone());
            }
            if cache.entries.values.len() == cache.capacity {
                cache.entries.values.remove(0);
            }
        }
        // No cache lock is held during quota, store reads, allocation or callbacks.
        let length = tile_length(self.descriptor.format)?;
        let mut payload = allocate(&self.quota, length)?;
        let bpp = self.descriptor.format.bytes_per_pixel();
        for (index, pixel) in payload.bytes.chunks_exact_mut(bpp).enumerate() {
            if index % 64 == 0 {
                control.check(64)?;
            }
            pixel.copy_from_slice(self.descriptor.default_bytes());
        }

        let mut next = 0u64;
        let mut visited = false;
        let mut stage_error = None;
        let streamed = {
            let mut stage = |offset: u64, block: &[u8]| {
                visited = true;
                if let Some(error) = stage_error {
                    return Err(error);
                }
                let result = (|| {
                    let end = offset.checked_add(block.len() as u64).ok_or(BackingError::Integrity)?;
                    if block.is_empty() || block.len() > MAX_NATIVE_TILE_VISIT_BYTES || offset != next || end > length as u64 {
                        return Err(BackingError::Integrity);
                    }
                    stage_tile_bytes(&mut payload.bytes, rect, self.descriptor.bounds, bpp, offset, block, control)?;
                    next = end;
                    Ok(())
                })();
                if let Err(error) = result {
                    stage_error = Some(error);
                }
                result
            };
            self.store.visit_native_tile(coord, control, &mut stage)
        };
        if let Some(error) = stage_error {
            return Err(error);
        }
        let streamed = streamed?;
        if (!streamed && visited) || (streamed && visited && next != length as u64) {
            return Err(BackingError::Integrity);
        }
        if !streamed {
            let overlap = rect.intersect(&self.descriptor.bounds);
            for y in overlap.y0..overlap.y1 {
                let row_pixels = u64::from(self.descriptor.bounds.width());
                let row = u64::try_from(i64::from(y) - i64::from(self.descriptor.bounds.y0)).map_err(|_| BackingError::Bounds)?;
                let col = u64::try_from(i64::from(overlap.x0) - i64::from(self.descriptor.bounds.x0)).map_err(|_| BackingError::Bounds)?;
                let offset =
                    row.checked_mul(row_pixels).and_then(|v| v.checked_add(col)).and_then(|v| v.checked_mul(bpp as u64)).ok_or(BackingError::Bounds)?;
                let local = pixel_offset(rect, overlap.x0, y, bpp)?;
                let count = usize::try_from(overlap.width()).map_err(|_| BackingError::Bounds)?.checked_mul(bpp).ok_or(BackingError::Bounds)?;
                let output = payload.bytes.get_mut(local..local.checked_add(count).ok_or(BackingError::Bounds)?).ok_or(BackingError::Length)?;
                read_store(self.store.as_ref(), offset, output, control)?;
            }
        }
        control.check(1)?;
        let pin = TilePin { payload: Arc::new(payload) };
        {
            let mut cache = self.cache.lock().map_err(|_| BackingError::Integrity)?;
            if let Some((_, tile)) = cache.entries.values.iter().find(|(c, _)| *c == coord) {
                return Ok(tile.clone());
            }
            if cache.entries.values.len() == cache.capacity {
                cache.entries.values.remove(0);
            }
            cache.entries.values.push((coord, pin.clone()));
        }
        Ok(pin)
    }
    pub fn read_pixel_into(&self, x: i32, y: i32, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        let bpp = self.descriptor.format.bytes_per_pixel();
        if output.len() != bpp {
            return Err(BackingError::Length);
        }
        let coord = TileCoord::containing(x, y);
        let pin = self.pin_tile(coord, control)?;
        let start = pixel_offset(tile_rect(coord)?, x, y, bpp)?;
        let end = start.checked_add(bpp).ok_or(BackingError::Bounds)?;
        control.check(1)?;
        output.copy_from_slice(pin.bytes().get(start..end).ok_or(BackingError::Length)?);
        Ok(())
    }
    /// Atomic COW region write, currently constrained to the declared native extent.
    /// Every affected tile and metadata entry is staged before a single publication.
    pub fn write_region(&mut self, bounds: Rect, bytes: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if bounds.is_empty() || !self.descriptor.bounds.contains_rect(&bounds) {
            return Err(BackingError::Bounds);
        }
        let bpp = self.descriptor.format.bytes_per_pixel();
        let required = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).and_then(|v| v.checked_mul(bpp as u64)).ok_or(BackingError::Bounds)?;
        if required > MAX_CHECKED_RASTER_BYTES || required != bytes.len() as u64 {
            return Err(BackingError::Length);
        }
        let mut reserve = self.overrides.values.len();
        for (visited, coord) in bounds.tiles().enumerate() {
            control.check(16)?;
            if visited >= MAX_ENTRIES {
                return Err(BackingError::Quota);
            }
            // Overrides are sorted at publication. A replacement needs no new
            // semantic entry, even when the current bounded index is full.
            if self.overrides.values.binary_search_by_key(&(coord.ty, coord.tx), |(c, _)| (c.ty, c.tx)).is_err() {
                reserve = reserve.checked_add(1).ok_or(BackingError::Quota)?;
                if reserve > self.override_limit {
                    return Err(BackingError::Quota);
                }
            }
        }
        let mut staged = entries(&self.quota, reserve)?;
        for (coord, pin) in &self.overrides.values {
            control.check(1)?;
            staged.values.push((*coord, pin.clone()));
        }
        for coord in bounds.tiles() {
            control.check(1)?;
            let rect = tile_rect(coord)?;
            let base = self.pin_tile(coord, control)?;
            let mut payload = allocate(&self.quota, base.bytes().len())?;
            for (target, source) in payload.bytes.chunks_mut(4096).zip(base.bytes().chunks(4096)) {
                control.check(source.len() as u64)?;
                target.copy_from_slice(source);
            }
            let overlap = rect.intersect(&bounds);
            for y in overlap.y0..overlap.y1 {
                control.check(u64::from(overlap.width()))?;
                let from = pixel_offset(bounds, overlap.x0, y, bpp)?;
                let to = pixel_offset(rect, overlap.x0, y, bpp)?;
                let n = (overlap.width() as usize).checked_mul(bpp).ok_or(BackingError::Bounds)?;
                payload
                    .bytes
                    .get_mut(to..to.checked_add(n).ok_or(BackingError::Bounds)?)
                    .ok_or(BackingError::Length)?
                    .copy_from_slice(bytes.get(from..from.checked_add(n).ok_or(BackingError::Bounds)?).ok_or(BackingError::Length)?);
            }
            let pin = TilePin { payload: Arc::new(payload) };
            if let Some((_, old)) = staged.values.iter_mut().find(|(c, _)| *c == coord) {
                *old = pin;
            } else {
                staged.values.push((coord, pin));
            }
        }
        control.check((staged.values.len() as u64).saturating_mul(16))?;
        staged.values.sort_unstable_by_key(|(c, _)| (c.ty, c.tx));
        control.check(1)?;
        self.overrides = Arc::new(staged);
        Ok(())
    }
    /// Shared exact identity without IO or a whole-payload comparison. Separately
    /// staged equal payloads require the explicit fallible content comparison.
    pub fn shares_exact_state(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.store, &other.store)
            && self.descriptor == other.descriptor
            && (Arc::ptr_eq(&self.overrides, &other.overrides)
                || (self.overrides.values.len() == other.overrides.values.len()
                    && self.overrides.values.iter().zip(&other.overrides.values).all(|((a, x), (b, y))| a == b && Arc::ptr_eq(&x.payload, &y.payload))))
    }
    /// Bounded exact comparison of current logical bytes; caches are not semantic data.
    pub fn try_content_equal(&self, other: &Self, control: &ReadControl<'_>) -> Result<bool, BackingError> {
        if self.descriptor != other.descriptor {
            return Ok(false);
        }
        let mut left = [0; 40];
        let mut right = [0; 40];
        let n = self.descriptor.format.bytes_per_pixel();
        for y in self.descriptor.bounds.y0..self.descriptor.bounds.y1 {
            for x in self.descriptor.bounds.x0..self.descriptor.bounds.x1 {
                self.read_pixel_into(x, y, left.get_mut(..n).ok_or(BackingError::Length)?, control)?;
                other.read_pixel_into(x, y, right.get_mut(..n).ok_or(BackingError::Length)?, control)?;
                if left.get(..n) != right.get(..n) {
                    return Ok(false);
                }
            }
        }
        control.check(1)?;
        Ok(true)
    }
}
// Copy only operative native byte spans; physical padding remains the exact
// initialized descriptor default, including partial pixels at block boundaries.
fn stage_tile_bytes(output: &mut [u8], tile: Rect, bounds: Rect, bpp: usize, offset: u64, block: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
    let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
    let end = start.checked_add(block.len()).ok_or(BackingError::Bounds)?;
    let row_bytes = (TILE_SIZE as usize).checked_mul(bpp).filter(|n| *n != 0).ok_or(BackingError::Bounds)?;
    let overlap = tile.intersect(&bounds);
    let mut at = start;
    while at < end {
        control.check(16)?;
        let row = at / row_bytes;
        let row_start = row.checked_mul(row_bytes).ok_or(BackingError::Bounds)?;
        let row_end = row_start.checked_add(row_bytes).ok_or(BackingError::Bounds)?.min(end);
        let y = tile.y0.checked_add(i32::try_from(row).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
        if y >= overlap.y0 && y < overlap.y1 && !overlap.is_empty() {
            let first = usize::try_from(i64::from(overlap.x0) - i64::from(tile.x0)).map_err(|_| BackingError::Bounds)?;
            let last = usize::try_from(i64::from(overlap.x1) - i64::from(tile.x0)).map_err(|_| BackingError::Bounds)?;
            let lo = row_start.checked_add(first.checked_mul(bpp).ok_or(BackingError::Bounds)?).ok_or(BackingError::Bounds)?.max(at);
            let hi = row_start.checked_add(last.checked_mul(bpp).ok_or(BackingError::Bounds)?).ok_or(BackingError::Bounds)?.min(row_end);
            if lo < hi {
                control.check((hi - lo) as u64)?;
                output.get_mut(lo..hi).ok_or(BackingError::Bounds)?.copy_from_slice(block.get(lo - start..hi - start).ok_or(BackingError::Bounds)?);
            }
        }
        at = row_end;
    }
    Ok(())
}
fn tile_rect(c: TileCoord) -> Result<Rect, BackingError> {
    let x = c.tx.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
    let y = c.ty.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
    Ok(Rect::new(x, y, x.checked_add(TILE_SIZE).ok_or(BackingError::Bounds)?, y.checked_add(TILE_SIZE).ok_or(BackingError::Bounds)?))
}
fn tile_length(format: PixelFormat) -> Result<usize, BackingError> {
    let n = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|v| v.checked_mul(format.bytes_per_pixel() as u64)).ok_or(BackingError::Bounds)?;
    if n > MAX_CHECKED_RASTER_BYTES {
        return Err(BackingError::Quota);
    }
    usize::try_from(n).map_err(|_| BackingError::Bounds)
}
fn pixel_offset(rect: Rect, x: i32, y: i32, bpp: usize) -> Result<usize, BackingError> {
    if !rect.contains(x, y) {
        return Err(BackingError::Bounds);
    }
    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackingError::Bounds)?;
    let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackingError::Bounds)?;
    row.checked_mul(rect.width() as usize).and_then(|v| v.checked_add(col)).and_then(|v| v.checked_mul(bpp)).ok_or(BackingError::Bounds)
}
fn allocate(quota: &BackingQuota, length: usize) -> Result<Payload, BackingError> {
    let envelope = (length as u64).checked_mul(2).and_then(|n| n.checked_add(128)).ok_or(BackingError::Quota)?;
    let mut lease = quota.reserve(QuotaKind::Resident, envelope).map_err(|_| BackingError::Quota)?;
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(length).is_err() {
        return Err(BackingError::Allocation);
    }
    let actual = (bytes.capacity() as u64).checked_add(128).ok_or(BackingError::Quota)?;
    if actual > envelope {
        drop(bytes);
        return Err(BackingError::Quota);
    }
    lease.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
    bytes.resize(length, 0);
    Ok(Payload { bytes, _lease: lease })
}
fn entry_reservation_bytes(count: usize) -> Result<u64, BackingError> {
    u64::try_from(count).ok().and_then(|count| count.checked_mul(ENTRY_ENVELOPE)).and_then(|n| n.checked_add(256)).ok_or(BackingError::Quota)
}
fn entries(quota: &BackingQuota, count: usize) -> Result<Entries, BackingError> {
    let envelope = entry_reservation_bytes(count)?;
    let mut lease = quota.reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
    let actual = (values.capacity() as u64)
        .checked_mul(std::mem::size_of::<(TileCoord, TilePin)>() as u64)
        .and_then(|n| n.checked_add(256))
        .ok_or(BackingError::Quota)?;
    if actual > envelope {
        drop(values);
        return Err(BackingError::Quota);
    }
    lease.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
    Ok(Entries { values, _lease: lease })
}
fn read_store(store: &dyn NativePlaneStore, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
    let end = offset.checked_add(output.len() as u64).ok_or(BackingError::Bounds)?;
    if end > store.descriptor().logical_bytes() {
        return Err(BackingError::Length);
    }
    for (i, part) in output.chunks_mut(4096).enumerate() {
        control.check(part.len() as u64)?;
        let at = offset.checked_add((i as u64).checked_mul(4096).ok_or(BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
        store.read_exact_at(at, part, control)?;
        control.check(0)?;
    }
    Ok(())
}
