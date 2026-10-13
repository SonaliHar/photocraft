use photocraft_color::PixelFormat;
use photocraft_format::{
    canonical_archive::{CanonicalArchivePlaneStore, CanonicalTileReference, ImmutableCanonicalArchive},
    zip::stream::{Limits, Writer},
};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    quota: BackingQuota,
    fail: AtomicBool,
    corrupt: AtomicBool,
    reads: AtomicU64,
    fail_at: AtomicU64,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Memory {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        let read = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail.load(Ordering::SeqCst) || self.fail_at.load(Ordering::SeqCst) == read {
            if let Some(v) = out.first_mut() {
                *v = 0xff;
            }
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
        if self.corrupt.load(Ordering::SeqCst)
            && offset > 100
            && let Some(v) = out.first_mut()
        {
            *v ^= 1;
        }
        Ok(())
    }
}

fn memory(bytes: Vec<u8>, q: &BackingQuota) -> Arc<Memory> {
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    Arc::new(Memory {
        bytes,
        quota: q.clone(),
        fail: AtomicBool::new(false),
        corrupt: AtomicBool::new(false),
        reads: AtomicU64::new(0),
        fail_at: AtomicU64::new(0),
        _lease: lease,
    })
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 4 << 20, scratch: 0 })
}
fn raw_archive(raw: &[u8]) -> (Vec<u8>, [u8; 32]) {
    let hash = *blake3::hash(raw).as_bytes();
    let mut path = String::from("tiles/");
    for byte in hash {
        path.push_str(&format!("{byte:02x}"));
    }
    path.push_str(".zst");
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0xc0, 0x38];
    frame.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    let total = raw.len().div_ceil(128 << 10);
    for (i, chunk) in raw.chunks(128 << 10).enumerate() {
        let head = ((chunk.len() as u32) << 3) | u32::from(i + 1 == total);
        frame.extend_from_slice(&head.to_le_bytes()[..3]);
        frame.extend_from_slice(chunk);
    }
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add(&path, frame.len() as u64, &mut Cursor::new(frame), &Interrupt::NONE).unwrap();
    (writer.finish(&Interrupt::NONE).unwrap().into_inner(), hash)
}
#[test]
fn cold_pin_authenticates_once_instead_of_once_per_native_row() {
    let q = quota();
    let native = [1u8, 2, 3, 4].repeat(256 * 256);
    let (bytes, hash) = raw_archive(&native);
    let source = memory(bytes, &q);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 256, 256), &[0; 4]).unwrap();
    let store = Arc::new(
        CanonicalArchivePlaneStore::new_default_padded(
            source.clone(),
            descriptor,
            &[CanonicalTileReference { coord: TileCoord::new(0, 0), hash }],
            q.clone(),
            &ctl(),
        )
        .unwrap(),
    );
    source.reads.store(0, Ordering::SeqCst);
    let mut pixel = [0; 4];
    store.read_exact_at(0, &mut pixel, &ctl()).unwrap();
    assert_eq!(pixel, [1, 2, 3, 4]);
    let one_authentication = source.reads.swap(0, Ordering::SeqCst);
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
    let pin = raster.pin_tile(TileCoord::new(0, 0), &ctl()).unwrap();
    assert_eq!(pin.bytes(), native);
    let cold_reads = source.reads.load(Ordering::SeqCst);
    assert!(cold_reads <= one_authentication + 2, "one tile authentication reads={one_authentication}, cold pin archive reads={cold_reads}");
    drop(pin);
    drop(raster);
    drop(store);
    drop(source);
    assert_eq!(q.usage().unwrap(), Default::default());
}

#[test]
fn late_archive_failure_cancel_and_quota_discard_private_pin_and_keep_old_bytes() {
    let q = quota();
    let native = [1u8, 2, 3, 4].repeat(256 * 256);
    let (bytes, hash) = raw_archive(&native);
    let source = memory(bytes, &q);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 256, 256), &[0; 4]).unwrap();
    let store = Arc::new(
        CanonicalArchivePlaneStore::new_default_padded(
            source.clone(),
            descriptor,
            &[CanonicalTileReference { coord: TileCoord::new(0, 0), hash }],
            q.clone(),
            &ctl(),
        )
        .unwrap(),
    );
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
    source.reads.store(0, Ordering::SeqCst);
    let old = raster.pin_tile(TileCoord::new(0, 0), &ctl()).unwrap();
    let reads = source.reads.load(Ordering::SeqCst);
    raster.clear_cache().unwrap();
    let baseline = q.usage().unwrap();
    source.reads.store(0, Ordering::SeqCst);
    source.fail_at.store(reads, Ordering::SeqCst);
    assert!(raster.pin_tile(TileCoord::new(0, 0), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(old.bytes(), native);
    source.fail_at.store(0, Ordering::SeqCst);
    // Corruption after constructor authentication must still fail the new
    // per-pin verification rather than publishing an earlier successful tile.
    source.corrupt.store(true, Ordering::SeqCst);
    assert!(raster.pin_tile(TileCoord::new(0, 0), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    source.corrupt.store(false, Ordering::SeqCst);
    let polls = AtomicU64::new(0);
    let work = |_| {
        polls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    drop(raster.pin_tile(TileCoord::new(0, 0), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap());
    let count = polls.load(Ordering::Relaxed);
    raster.clear_cache().unwrap();
    for target in [1, count / 2, count] {
        polls.store(0, Ordering::Relaxed);
        let work = |_| if polls.fetch_add(1, Ordering::Relaxed) + 1 == target { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(raster.pin_tile(TileCoord::new(0, 0), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let pressure = q.reserve(QuotaKind::Resident, (32 << 20) - baseline.resident - 16).unwrap();
    let pressured = q.usage().unwrap();
    assert!(raster.pin_tile(TileCoord::new(0, 0), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), pressured);
    drop(pressure);
    drop(old);
    drop(raster);
    drop(store);
    drop(source);
    assert_eq!(q.usage().unwrap(), Default::default());
}
