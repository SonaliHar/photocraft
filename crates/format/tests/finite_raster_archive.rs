use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::BackedLayer;
use photocraft_format::{
    canonical_archive::ImmutableCanonicalArchive,
    finite_raster::FiniteRasterLimits,
    finite_raster_archive::{FiniteArchiveLimits, load_finite_raster_archive, save_finite_raster_archive},
    zip::stream::{Limits, Reader},
};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
    },
};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn limits() -> FiniteArchiveLimits {
    FiniteArchiveLimits { raster: FiniteRasterLimits { max_logical_bytes: 1 << 30, max_tiles: 4096 }, zip: Limits::default(), max_metadata_bytes: 1 << 20 }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 2 << 20, scratch: 0 })
}
#[derive(Debug)]
struct DefaultStore(NativePlaneDescriptor);
impl NativePlaneStore for DefaultStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.0.default_bytes()[((offset + i as u64) % self.0.format().bytes_per_pixel() as u64) as usize];
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Memory {
    data: Vec<u8>,
    quota: BackingQuota,
    reads: AtomicU64,
    fail: AtomicU64,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Memory {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        let n = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail.load(Ordering::SeqCst) == n {
            if let Some(b) = out.first_mut() {
                *b = 99;
            }
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.data.get(start..end).ok_or(BackingError::Io)?);
        Ok(())
    }
}
fn memory(data: Vec<u8>, q: &BackingQuota) -> Arc<Memory> {
    let lease = q.reserve(QuotaKind::Resident, data.capacity() as u64).unwrap();
    Arc::new(Memory { data, quota: q.clone(), reads: AtomicU64::new(0), fail: AtomicU64::new(0), _lease: lease })
}
#[test]
fn current_wide_cow_negative_partial_defaults_and_history_survive_owned_archive() {
    for sample in [SampleType::U32, SampleType::F64] {
        let q = quota();
        let control = ctl();
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let default = if sample == SampleType::U32 { 0x01000001_u32.to_ne_bytes().repeat(4) } else { (-0_f64).to_ne_bytes().repeat(4) };
        let descriptor = NativePlaneDescriptor::new(format, Rect::new(-2, -1, 259, 3), &default).unwrap();
        let raster = BackedRaster::new(Arc::new(DefaultStore(descriptor)), q.clone(), 2, &control).unwrap();
        let old = BackedLayer::new(raster, descriptor, &control).unwrap();
        let edit = if sample == SampleType::U32 { 0x01000003_u32.to_ne_bytes().repeat(4) } else { f64::from_bits(0x3fd3333333333334).to_ne_bytes().repeat(4) };
        let current = old.try_edited_native(Rect::new(-1, 0, 0, 1), &edit, &control).unwrap();
        let data = save_finite_raster_archive(&current, Cursor::new(Vec::new()), limits(), &control).expect("actual current archive save").into_inner();
        let mut zip = Reader::new(Cursor::new(&data), Limits::default(), &Interrupt::NONE).unwrap();
        assert!(zip.contains("finite-raster.json"));
        assert!(!zip.contains("manifest.json"));
        let json = zip.read_by_name("finite-raster.json", 1 << 20, &Interrupt::NONE).unwrap();
        let meta: photocraft_format::finite_raster::FiniteRasterM = serde_json::from_slice(&json).unwrap();
        assert_eq!(meta.tiles.len(), 1);
        assert_eq!(meta.tiles[0].tx, -1);
        assert_eq!(meta.tiles[0].ty, 0);
        drop(zip);
        let input = memory(data, &q);
        let store = load_finite_raster_archive(input.clone(), q.clone(), limits(), &control).expect("verified owned archive load");
        assert_eq!(store.descriptor(), descriptor);
        let at = (descriptor.bounds().width() as u64 + 1) * format.bytes_per_pixel() as u64;
        let mut bytes = vec![77; edit.len()];
        store.read_exact_at(at, &mut bytes, &control).unwrap();
        assert_eq!(bytes, edit);
        let mut expected = default.repeat((descriptor.logical_bytes() as usize) / default.len());
        expected[at as usize..at as usize + edit.len()].copy_from_slice(&edit);
        let mut complete = vec![77; expected.len()];
        store.read_exact_at(0, &mut complete, &control).unwrap();
        assert_eq!(complete, expected);
        let pin = old.checked_view().pin_tile(TileCoord::new(-1, 0), &control).unwrap();
        assert_ne!(pin.encoded(), current.checked_view().pin_tile(TileCoord::new(-1, 0), &control).unwrap().encoded());
        drop(pin);
        drop((old, current, input));
        let mut first = vec![0; default.len()];
        store.read_exact_at(0, &mut first, &control).unwrap();
        assert_eq!(first, default);
        drop(store);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn all_native_depths_gray_rgb_defaults_include_nan_and_signedzero_without_plane_allocation() {
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for alpha in [false, true] {
                let q = quota();
                let control = ctl();
                let format = PixelFormat::new(mode, sample, alpha);
                let value = match sample {
                    SampleType::U8 => vec![17],
                    SampleType::U16 => 0x0101_u16.to_ne_bytes().to_vec(),
                    SampleType::U32 => 0x01000001_u32.to_ne_bytes().to_vec(),
                    SampleType::F32 => f32::from_bits(0x7fc01234).to_ne_bytes().to_vec(),
                    SampleType::F64 => (-0_f64).to_ne_bytes().to_vec(),
                };
                let default = value.repeat(format.channels());
                let descriptor = NativePlaneDescriptor::new(format, Rect::new(-2, 1, 4, 3), &default).unwrap();
                let layer =
                    BackedLayer::new(BackedRaster::new(Arc::new(DefaultStore(descriptor)), q.clone(), 1, &control).unwrap(), descriptor, &control).unwrap();
                let data = save_finite_raster_archive(&layer, Cursor::new(Vec::new()), limits(), &control).unwrap().into_inner();
                drop(layer);
                let old = Reader::new(Cursor::new(&data), Limits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(entry_count(&data), 1);
                assert!(old.contains("finite-raster.json"));
                let source = memory(data, &q);
                let store = load_finite_raster_archive(source, q.clone(), limits(), &control).unwrap();
                let mut output = vec![9; default.len() + 3];
                store.read_exact_at(1, &mut output, &control).unwrap();
                let expected = (0..output.len()).map(|i| default[(i + 1) % default.len()]).collect::<Vec<_>>();
                assert_eq!(output, expected);
                assert_eq!(store.descriptor(), descriptor);
                drop(store);
                assert_eq!(q.usage().unwrap(), QuotaUsage::default());
            }
        }
    }
}
#[derive(Debug)]
struct Pattern(NativePlaneDescriptor);
impl NativePlaneStore for Pattern {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        for (i, b) in out.iter_mut().enumerate() {
            let pixel = (offset + i as u64) / 4;
            *b = if pixel % 768 < 512 { 17 } else { 0 };
        }
        Ok(())
    }
}
#[test]
fn repeated_current_tiles_are_written_once_and_refs_preserve_sorted_holes() {
    let q = quota();
    let control = ctl();
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 768, 256), &[0; 4]).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(Pattern(d)), q.clone(), 2, &control).unwrap(), d, &control).unwrap();
    let data = save_finite_raster_archive(&layer, Cursor::new(Vec::new()), limits(), &control).unwrap().into_inner();
    drop(layer);
    let mut reader = Reader::new(Cursor::new(&data), Limits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(entry_count(&data), 2);
    let json = reader.read_by_name("finite-raster.json", 1 << 20, &Interrupt::NONE).unwrap();
    let m: photocraft_format::finite_raster::FiniteRasterM = serde_json::from_slice(&json).unwrap();
    assert_eq!(m.tiles.len(), 2);
    assert_eq!(m.tiles[0].hash, m.tiles[1].hash);
    assert_eq!((m.tiles[0].tx, m.tiles[1].tx), (0, 1));
    let source = memory(data, &q);
    let store = load_finite_raster_archive(source, q.clone(), limits(), &control).unwrap();
    let mut output = [1; 8];
    store.read_exact_at(510 * 4, &mut output, &control).unwrap();
    assert_eq!(output, [17; 8]);
    store.read_exact_at(512 * 4, &mut output, &control).unwrap();
    assert_eq!(output, [0; 8]);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
struct Output {
    inner: Cursor<Vec<u8>>,
    seeks: Arc<AtomicU64>,
    written: Arc<AtomicU64>,
    left: usize,
}
impl std::io::Seek for Output {
    fn seek(&mut self, at: std::io::SeekFrom) -> std::io::Result<u64> {
        self.seeks.fetch_add(1, Ordering::SeqCst);
        self.inner.seek(at)
    }
}
impl std::io::Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "own generated full disk"));
        }
        let n = bytes.len().min(self.left);
        self.left -= n;
        self.written.fetch_add(n as u64, Ordering::SeqCst);
        std::io::Write::write(&mut self.inner, &bytes[..n])
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn current(q: &BackingQuota) -> BackedLayer {
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 2, 2), &[0; 4]).unwrap();
    BackedLayer::new(BackedRaster::new(Arc::new(DefaultStore(d)), q.clone(), 1, &ctl()).unwrap(), d, &ctl())
        .unwrap()
        .try_edited_native(Rect::new(0, 0, 1, 1), &[1, 2, 3, 4], &ctl())
        .unwrap()
}
#[test]
fn quota_limits_work_cancel_and_writer_failure_never_return_completed_stream() {
    for kind in 0..7 {
        let q = quota();
        let layer = current(&q);
        let seeks = Arc::new(AtomicU64::new(0));
        let written = Arc::new(AtomicU64::new(0));
        let output = Output { inner: Cursor::new(Vec::new()), seeks: seeks.clone(), written: written.clone(), left: if kind == 3 { 200 } else { usize::MAX } };
        let mut lim = limits();
        if kind == 0 {
            lim.raster.max_tiles = 0
        }
        if kind == 1 {
            lim.raster.max_logical_bytes = 15
        }
        if kind == 2 {
            lim.max_metadata_bytes = 1
        }
        let held = if kind == 4 { Some(q.reserve(QuotaKind::Resident, (32 << 20) - q.usage().unwrap().resident).unwrap()) } else { None };
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let calls = AtomicU64::new(0);
        let work = |n| {
            if n == 4096 && calls.fetch_add(1, Ordering::SeqCst) > 3 {
                if kind == 5 {
                    return Err(BackingError::Work);
                }
                if kind == 6 {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        };
        let check = || cancelled.load(Ordering::SeqCst);
        let interrupt = Interrupt::cancel_only(&check);
        let control = ReadControl { interrupt: &interrupt, work: &work };
        assert!(save_finite_raster_archive(&layer, output, lim, &control).is_err());
        if matches!(kind, 0 | 1 | 4) {
            assert_eq!(seeks.load(Ordering::SeqCst), 0);
            assert_eq!(written.load(Ordering::SeqCst), 0)
        }
        if kind == 3 {
            assert!(written.load(Ordering::SeqCst) > 0)
        }
        drop(held);
        drop(layer);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn owned_load_missing_corrupt_late_io_cancel_quota_and_atomic_current_reads() {
    let q = quota();
    let layer = current(&q);
    let data = save_finite_raster_archive(&layer, Cursor::new(Vec::new()), limits(), &ctl()).unwrap().into_inner();
    drop(layer);
    let source = memory(data.clone(), &q);
    let store = load_finite_raster_archive(source.clone(), q.clone(), limits(), &ctl()).unwrap();
    let last = source.reads.load(Ordering::SeqCst);
    assert!(last > 10);
    drop(store);
    let baseline = q.usage().unwrap();
    source.reads.store(0, Ordering::SeqCst);
    source.fail.store(last - 1, Ordering::SeqCst);
    assert!(load_finite_raster_archive(source.clone(), q.clone(), limits(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    source.fail.store(0, Ordering::SeqCst);
    let other = quota();
    source.reads.store(0, Ordering::SeqCst);
    assert!(load_finite_raster_archive(source.clone(), other.clone(), limits(), &ctl()).is_err());
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    assert_eq!(other.usage().unwrap(), QuotaUsage::default());
    let store = load_finite_raster_archive(source.clone(), q.clone(), limits(), &ctl()).unwrap();
    let live = q.usage().unwrap();
    source.fail.store(source.reads.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
    let mut output = [77; 16];
    assert!(store.read_exact_at(0, &mut output, &ctl()).is_err());
    assert_eq!(output, [77; 16]);
    assert_eq!(q.usage().unwrap(), live);
    source.fail.store(0, Ordering::SeqCst);
    drop(store);
    let work = |_| Err(BackingError::Work);
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(load_finite_raster_archive(source.clone(), q.clone(), limits(), &control).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let held = q.reserve(QuotaKind::Resident, (32 << 20) - q.usage().unwrap().resident).unwrap();
    assert!(load_finite_raster_archive(source.clone(), q.clone(), limits(), &ctl()).is_err());
    drop(held);
    assert_eq!(q.usage().unwrap(), baseline);
    let mut legacy = Reader::new(Cursor::new(&data), Limits::default(), &Interrupt::NONE).unwrap();
    let json = legacy.read_by_name("finite-raster.json", 1 << 20, &Interrupt::NONE).unwrap();
    let mut missing = photocraft_format::zip::stream::Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    missing.add("finite-raster.json", json.len() as u64, &mut json.as_slice(), &Interrupt::NONE).unwrap();
    let missing = memory(missing.finish(&Interrupt::NONE).unwrap().into_inner(), &q);
    assert!(load_finite_raster_archive(missing.clone(), q.clone(), limits(), &ctl()).is_err());
    drop(missing);
    let mut damaged = data;
    damaged[140] ^= 1;
    let damaged = memory(damaged, &q);
    let before = q.usage().unwrap();
    assert!(load_finite_raster_archive(damaged.clone(), q.clone(), limits(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(damaged);
    drop(source);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
fn entry_count(bytes: &[u8]) -> u64 {
    let start = bytes.len() - 98;
    assert_eq!(&bytes[start..start + 4], &0x06064b50_u32.to_le_bytes());
    u64::from_le_bytes(bytes[start + 24..start + 32].try_into().unwrap())
}
#[test]
fn late_authentication_cancel_reentrant_ledger_and_metadata_overlap_refund() {
    let q = quota();
    let layer = current(&q);
    let data = save_finite_raster_archive(&layer, Cursor::new(Vec::new()), limits(), &ctl()).unwrap().into_inner();
    drop(layer);
    let source = memory(data, &q);
    let golden = load_finite_raster_archive(source.clone(), q.clone(), limits(), &ctl()).unwrap();
    let full_reads = source.reads.load(Ordering::SeqCst);
    drop(golden);
    let baseline = q.usage().unwrap();
    source.reads.store(0, Ordering::SeqCst);
    let peak_resident = AtomicU64::new(0);
    let peak_index = AtomicU64::new(0);
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let work = |_| {
        let usage = q.usage().unwrap();
        peak_resident.fetch_max(usage.resident, Ordering::SeqCst);
        peak_index.fetch_max(usage.index, Ordering::SeqCst);
        let reentrant = q.reserve(QuotaKind::Resident, 1).map_err(|_| BackingError::Quota)?;
        drop(reentrant);
        if source.reads.load(Ordering::SeqCst) >= full_reads - 2 {
            cancelled.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let check = || cancelled.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&check);
    let control = ReadControl { interrupt: &interrupt, work: &work };
    assert!(load_finite_raster_archive(source.clone(), q.clone(), limits(), &control).is_err());
    assert!(source.reads.load(Ordering::SeqCst) >= full_reads - 2);
    assert!(peak_index.load(Ordering::SeqCst) > 1 << 20);
    assert!(peak_resident.load(Ordering::SeqCst) > baseline.resident + (1 << 20));
    assert_eq!(q.usage().unwrap(), baseline);
    let mut small = limits();
    small.max_metadata_bytes = 1;
    assert!(load_finite_raster_archive(source.clone(), q.clone(), small, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    drop(source);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[derive(Debug)]
struct Generated(NativePlaneDescriptor);
impl NativePlaneStore for Generated {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        for (i, b) in out.iter_mut().enumerate() {
            let at = offset + i as u64;
            let channel = at % 4;
            *b = if channel == 3 { 255 } else { ((at / 4) >> (channel * 8)) as u8 };
        }
        Ok(())
    }
}
#[derive(Default)]
struct CountOutput {
    position: u64,
    length: u64,
}
impl std::io::Write for CountOutput {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.position += b.len() as u64;
        self.length = self.length.max(self.position);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl std::io::Seek for CountOutput {
    fn seek(&mut self, s: std::io::SeekFrom) -> std::io::Result<u64> {
        let next = match s {
            std::io::SeekFrom::Start(n) => i128::from(n),
            std::io::SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            std::io::SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        self.position = u64::try_from(next).map_err(|_| std::io::Error::other("counting seek bounds"))?;
        Ok(self.position)
    }
}
#[test]
#[ignore = "release-only generated24MP standalone save/counting-output measurement"]
fn generated_24mp_finite_save_measurement() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let q = quota();
    let peak = AtomicU64::new(0);
    let work = |_| {
        peak.fetch_max(q.usage().unwrap().resident, Ordering::SeqCst);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 6000, 4000), &[0; 4]).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(Generated(d)), q.clone(), 2, &c).unwrap(), d, &c).unwrap();
    let start = std::time::Instant::now();
    let out = save_finite_raster_archive(&layer, CountOutput::default(), limits(), &c).unwrap();
    println!(
        "generated24MP standalonefinite logical={} zip={} elapsed_ms={} callback_resident={} (counting output/no disk/RSS/native migration)",
        d.logical_bytes(),
        out.length,
        start.elapsed().as_secs_f64() * 1000.,
        peak.load(Ordering::SeqCst)
    );
    drop(layer);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    Ok(())
}

#[test]
fn finite_wire_rejects_authenticated_nondefault_padding_without_erasing_native_bits() {
    for sample in [SampleType::U32, SampleType::F64] {
        let q = quota();
        let control = ctl();
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let native = if sample == SampleType::U32 { 0x01000001_u32.to_ne_bytes().repeat(4) } else { (-0.0_f64).to_ne_bytes().repeat(4) };
        let mut canonical = native.clone();
        if cfg!(target_endian = "big") {
            for bytes in canonical.chunks_exact_mut(sample.bytes()) {
                bytes.reverse();
            }
        }
        let mut raw = canonical.repeat(256 * 256);
        // Logical pixel is byte-exact default; only the authenticated, nonlogical
        // last tile byte differs. Accepting then repinning would erase this bit.
        *raw.last_mut().unwrap() ^= 1;
        let hash = blake3::hash(&raw).to_hex().to_string();
        let default = canonical.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let metadata = photocraft_format::finite_raster::FiniteRasterM {
            format,
            bounds: Rect::from_xywh(0, 0, 1, 1),
            default,
            tiles: vec![photocraft_format::manifest::TileRef { tx: 0, ty: 0, hash: hash.clone() }],
        };
        let json = serde_json::to_vec(&metadata).unwrap();
        let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0xc0, 0x38];
        frame.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        let total = raw.len().div_ceil(128 << 10);
        for (i, chunk) in raw.chunks(128 << 10).enumerate() {
            let header = ((chunk.len() as u32) << 3) | u32::from(i + 1 == total);
            frame.extend_from_slice(&header.to_le_bytes()[..3]);
            frame.extend_from_slice(chunk);
        }
        let mut zip = photocraft_format::zip::stream::Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
        zip.add("finite-raster.json", json.len() as u64, &mut Cursor::new(json), &Interrupt::NONE).unwrap();
        zip.add(&format!("tiles/{hash}.zst"), frame.len() as u64, &mut Cursor::new(frame), &Interrupt::NONE).unwrap();
        let source = memory(zip.finish(&Interrupt::NONE).unwrap().into_inner(), &q);
        let before = q.usage().unwrap();
        assert!(
            matches!(
                load_finite_raster_archive(source.clone(), q.clone(), limits(), &control),
                Err(photocraft_format::finite_raster_archive::FiniteArchiveError::Backing(BackingError::Integrity))
            ),
            "authenticated nonlogical tile bytes must not be silently replaced by defaults"
        );
        assert_eq!(q.usage().unwrap(), before);
        drop(source);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
