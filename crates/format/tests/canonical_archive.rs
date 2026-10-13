use photocraft_color::{ColorMode, PixelFormat, SampleType};
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
#[test]
fn default_holes_owned_archive_atomic_ranges() {
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let q = BackingQuota::new(QuotaLimits { resident: 4 << 20, index: 1 << 20, scratch: 0 });
    let bytes = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap().finish(&Interrupt::NONE).unwrap().into_inner();
    let source = memory(bytes, &q);
    let desc = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::from_xywh(-2, 3, 4, 2), &[1, 2, 3, 4]).unwrap();
    let store = CanonicalArchivePlaneStore::new(source, desc, &[], q.clone(), &ctl).expect("valid immutable archive default holes");
    let mut bytes = [99; 13];
    store.read_exact_at(3, &mut bytes, &ctl).unwrap();
    assert_eq!(bytes, [4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4]);
    let before = bytes;
    assert!(store.read_exact_at(u64::MAX, &mut bytes, &ctl).is_err());
    assert_eq!(bytes, before);
    drop(store);
    assert_eq!(q.usage().unwrap().resident, 0);
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
fn pixel(sample: SampleType, channels: usize) -> Vec<u8> {
    (0..channels)
        .flat_map(|c| match sample {
            SampleType::U8 => vec![17 + c as u8],
            SampleType::U16 => (32769 + c as u16).to_ne_bytes().to_vec(),
            SampleType::U32 => (16777217 + c as u32).to_ne_bytes().to_vec(),
            SampleType::F32 => {
                if c == 0 {
                    (-0.0_f32).to_ne_bytes().to_vec()
                } else {
                    (0x7fc00001_u32 + c as u32).to_ne_bytes().to_vec()
                }
            }
            SampleType::F64 => {
                if c == 0 {
                    (-0.0_f64).to_ne_bytes().to_vec()
                } else {
                    (0x7ff8000000000001_u64 + c as u64).to_ne_bytes().to_vec()
                }
            }
        })
        .collect()
}
#[test]
fn exact_all_native_depths_partial_rows_default_padding_and_history() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let format = PixelFormat::new(mode, sample, true);
            let value = pixel(sample, format.channels());
            let mut canonical = value.clone();
            if cfg!(target_endian = "big") {
                for sample in canonical.chunks_mut(sample.bytes()) {
                    sample.reverse();
                }
            }
            let raw = canonical.repeat(256 * 256);
            let (bytes, hash) = raw_archive(&raw);
            let q = quota();
            let source = memory(bytes, &q);
            let desc = NativePlaneDescriptor::new(format, Rect::from_xywh(-2, 3, 4, 2), &vec![0; value.len()]).unwrap();
            let store = Arc::new(
                CanonicalArchivePlaneStore::new(source.clone(), desc, &[CanonicalTileReference { coord: TileCoord::new(-1, 0), hash }], q.clone(), &ctl())
                    .unwrap(),
            );
            drop(source);
            let expected = [value.repeat(2), vec![0; value.len() * 2], value.repeat(2), vec![0; value.len() * 2]].concat();
            let mut output = vec![0; expected.len() - 2];
            store.read_exact_at(1, &mut output, &ctl()).unwrap();
            assert_eq!(output, &expected[1..expected.len() - 1]);
            let mut raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
            let history = raster.clone();
            let edit = vec![7; value.len()];
            raster.write_region(Rect::from_xywh(-1, 3, 1, 1), &edit, &ctl()).unwrap();
            let mut original = vec![0; expected.len()];
            history.read_current_exact_at(0, &mut original, &ctl()).unwrap();
            assert_eq!(original, expected);
            let mut current = original.clone();
            raster.read_current_exact_at(0, &mut current, &ctl()).unwrap();
            assert_eq!(&current[value.len()..value.len() * 2], edit);
            assert_eq!(&original[value.len()..value.len() * 2], value);
            drop(history);
            drop(raster);
            drop(store);
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().index, 0);
        }
    }
}
#[test]
fn faults_cancel_work_quota_and_membership_fail_closed() {
    let value = [1, 2, 3, 4];
    let (bytes, hash) = raw_archive(&value.repeat(256 * 256));
    let q = quota();
    let source = memory(bytes, &q);
    let baseline = q.usage().unwrap();
    let desc = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::from_xywh(0, 0, 3, 2), &[0; 4]).unwrap();
    let refs = [CanonicalTileReference { coord: TileCoord::new(0, 0), hash }];
    let other = quota();
    assert!(matches!(CanonicalArchivePlaneStore::new(source.clone(), desc, &refs, other.clone(), &ctl()), Err(BackingError::Quota)));
    assert_eq!(other.usage().unwrap().resident, 0);
    assert!(CanonicalArchivePlaneStore::new(source.clone(), desc, &[refs[0], refs[0]], q.clone(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let store = CanonicalArchivePlaneStore::new(source.clone(), desc, &refs, q.clone(), &ctl()).unwrap();
    let retained = q.usage().unwrap();
    let mut output = [99; 17];
    let before = output;
    source.fail.store(true, Ordering::SeqCst);
    assert!(matches!(store.read_exact_at(1, &mut output, &ctl()), Err(BackingError::Io)));
    assert_eq!(output, before);
    assert_eq!(q.usage().unwrap(), retained);
    source.fail.store(false, Ordering::SeqCst);
    source.corrupt.store(true, Ordering::SeqCst);
    assert!(store.read_exact_at(1, &mut output, &ctl()).is_err());
    assert_eq!(output, before);
    source.corrupt.store(false, Ordering::SeqCst);
    let work = |_| Err(BackingError::Work);
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert_eq!(store.read_exact_at(1, &mut output, &control), Err(BackingError::Work));
    assert_eq!(output, before);
    let held = q.reserve(QuotaKind::Resident, (32 << 20) - q.usage().unwrap().resident).unwrap();
    assert_eq!(store.read_exact_at(1, &mut output, &ctl()), Err(BackingError::Quota));
    assert_eq!(output, before);
    drop(held);
    let reentrant = |_| {
        let temporary = q.reserve(QuotaKind::Resident, 1).map_err(|_| BackingError::Quota)?;
        drop(temporary);
        Ok(())
    };
    store.read_exact_at(1, &mut output, &ReadControl { interrupt: &Interrupt::NONE, work: &reentrant }).unwrap();
    assert_eq!(output, [2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2]);
    drop(store);
    assert_eq!(q.usage().unwrap(), baseline);
    drop(source);
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn late_frame_cancel_read_failure_missing_object_and_empty_end() {
    let (bytes, hash) = raw_archive(&[3, 4, 5, 6].repeat(256 * 256));
    let q = quota();
    let source = memory(bytes, &q);
    let desc = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::from_xywh(0, 0, 4, 4), &[0; 4]).unwrap();
    let reference = CanonicalTileReference { coord: TileCoord::new(0, 0), hash };
    let before = q.usage().unwrap();
    let missing = CanonicalTileReference { hash: [9; 32], ..reference };
    assert!(matches!(CanonicalArchivePlaneStore::new(source.clone(), desc, &[missing], q.clone(), &ctl()), Err(BackingError::Integrity)));
    assert_eq!(q.usage().unwrap(), before);
    let store = CanonicalArchivePlaneStore::new(source.clone(), desc, &[reference], q.clone(), &ctl()).unwrap();
    let live = q.usage().unwrap();
    let mut out = [88; 64];
    let reads = source.reads.load(Ordering::SeqCst);
    source.fail_at.store(reads + 7, Ordering::SeqCst);
    assert_eq!(store.read_exact_at(0, &mut out, &ctl()), Err(BackingError::Io));
    assert!(source.reads.load(Ordering::SeqCst) >= reads + 7);
    assert_eq!(out, [88; 64]);
    assert_eq!(q.usage().unwrap(), live);
    source.fail_at.store(0, Ordering::SeqCst);
    let cancelled = AtomicBool::new(false);
    let units = AtomicU64::new(0);
    let work = |n| {
        if units.fetch_add(n, Ordering::SeqCst) + n > 150000 {
            cancelled.store(true, Ordering::SeqCst)
        }
        Ok(())
    };
    let check = || cancelled.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&check);
    let control = ReadControl { interrupt: &interrupt, work: &work };
    assert_eq!(store.read_exact_at(0, &mut out, &control), Err(BackingError::Cancelled));
    assert_eq!(out, [88; 64]);
    assert_eq!(q.usage().unwrap(), live);
    store.read_exact_at(desc.logical_bytes(), &mut [], &ctl()).unwrap();
    assert_eq!(store.read_exact_at(desc.logical_bytes() + 1, &mut [], &ctl()), Err(BackingError::Bounds));
    assert_eq!(store.read_exact_at(0, &mut vec![0; 65537], &ctl()), Err(BackingError::Bounds));
    drop(store);
    assert_eq!(q.usage().unwrap(), before);
    drop(source);
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn constructor_precharge_and_final_read_publication_denial() {
    let (bytes, hash) = raw_archive(&[1, 2, 3, 4].repeat(256 * 256));
    let q = quota();
    let source = memory(bytes, &q);
    let desc = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::from_xywh(0, 0, 2, 2), &[0; 4]).unwrap();
    let refs = [CanonicalTileReference { coord: TileCoord::new(0, 0), hash }];
    let baseline = q.usage().unwrap();
    let held = q.reserve(QuotaKind::Resident, (32 << 20) - baseline.resident).unwrap();
    let reads = source.reads.load(Ordering::SeqCst);
    assert!(matches!(CanonicalArchivePlaneStore::new(source.clone(), desc, &refs, q.clone(), &ctl()), Err(BackingError::Quota)));
    assert_eq!(source.reads.load(Ordering::SeqCst), reads);
    drop(held);
    assert_eq!(q.usage().unwrap(), baseline);
    let store = CanonicalArchivePlaneStore::new(source.clone(), desc, &refs, q.clone(), &ctl()).unwrap();
    let live = q.usage().unwrap();
    let units = AtomicU64::new(0);
    let count = |n| {
        units.fetch_add(n, Ordering::SeqCst);
        Ok(())
    };
    let mut output = [0; 16];
    store.read_exact_at(0, &mut output, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    let total = units.load(Ordering::SeqCst);
    assert_eq!(output, [1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4]);
    units.store(0, Ordering::SeqCst);
    let deny = |n| {
        if units.fetch_add(n, Ordering::SeqCst) + n >= total { Err(BackingError::Work) } else { Ok(()) }
    };
    output.fill(77);
    assert_eq!(store.read_exact_at(0, &mut output, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(BackingError::Work));
    assert_eq!(output, [77; 16]);
    assert_eq!(q.usage().unwrap(), live);
    drop(store);
    assert_eq!(q.usage().unwrap(), baseline);
    drop(source);
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn large_f64_tile_full_crc_hash_authentication_and_padding() {
    let format = PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true);
    let value = pixel(SampleType::F64, format.channels());
    let mut canonical = value.clone();
    if cfg!(target_endian = "big") {
        for chunk in canonical.chunks_mut(8) {
            chunk.reverse();
        }
    }
    let raw = canonical.repeat(256 * 256);
    assert!(raw.len() > 2 << 20);
    let (bytes, hash) = raw_archive(&raw);
    let q = quota();
    let desc = NativePlaneDescriptor::new(format, Rect::from_xywh(255, 255, 1, 1), &vec![9; value.len()]).unwrap();
    let refs = [CanonicalTileReference { coord: TileCoord::new(0, 0), hash }];
    let source = memory(bytes.clone(), &q);
    let store = CanonicalArchivePlaneStore::new(source.clone(), desc, &refs, q.clone(), &ctl()).unwrap();
    drop(source);
    let mut output = vec![0; value.len() - 2];
    store.read_exact_at(1, &mut output, &ctl()).unwrap();
    assert_eq!(output, &value[1..value.len() - 1]);
    drop(store);
    assert_eq!(q.usage().unwrap().resident, 0);
    // Change an authenticated out-of-descriptor padding byte, leaving ZIP metadata
    // and object name intact. Constructor must verify whole object, not selected data.
    let mut corrupt = bytes;
    let name_length = u16::from_le_bytes(corrupt[26..28].try_into().unwrap()) as usize;
    let extra_length = u16::from_le_bytes(corrupt[28..30].try_into().unwrap()) as usize;
    let payload = 30 + name_length + extra_length;
    corrupt[payload + 17] ^= 1;
    let source = memory(corrupt, &q);
    let baseline = q.usage().unwrap();
    assert!(matches!(CanonicalArchivePlaneStore::new(source.clone(), desc, &refs, q.clone(), &ctl()), Err(BackingError::Integrity)));
    assert_eq!(q.usage().unwrap(), baseline);
    drop(source);
    assert_eq!(q.usage().unwrap().resident, 0);
}
