use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent};
use photocraft_format::{
    SaveOptions,
    canonical_object::{PreparedCanonicalTile, TileObjectError},
    canonical_tile::CanonicalTileError,
    save_to_bytes,
    zip::{
        ZipReader,
        stream::{Limits, Reader as StreamReader, Writer},
    },
};
use photocraft_geom::{Rect, Size, TileCoord};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use std::{
    io::{Cursor, Read, Seek, SeekFrom, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Debug)]
struct Literal {
    descriptor: NativePlaneDescriptor,
    pixel: Vec<u8>,
    reads: Arc<AtomicU64>,
    fail: Arc<AtomicU64>,
}
impl NativePlaneStore for Literal {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        let n = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail.load(Ordering::SeqCst) == n {
            if let Some(v) = out.first_mut() {
                *v = 0xff;
            }
            return Err(BackingError::Io);
        }
        for (i, v) in out.iter_mut().enumerate() {
            *v = self.pixel[(offset as usize + i) % self.pixel.len()];
        }
        Ok(())
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn pixel(format: PixelFormat, edit: bool) -> Vec<u8> {
    (0..format.channels())
        .flat_map(|channel| {
            let alpha = channel + 1 == format.channels();
            match format.sample {
                SampleType::U8 => vec![if alpha {
                    255
                } else if edit {
                    17
                } else {
                    13
                }],
                SampleType::U16 => if alpha {
                    65535_u16
                } else if edit {
                    32771
                } else {
                    32769
                }
                .to_ne_bytes()
                .to_vec(),
                SampleType::U32 => if alpha {
                    u32::MAX
                } else if edit {
                    16777219
                } else {
                    16777217
                }
                .to_ne_bytes()
                .to_vec(),
                SampleType::F32 => if alpha {
                    1_f32
                } else if channel == 0 && !edit {
                    -0_f32
                } else if edit {
                    0.3
                } else {
                    0.1
                }
                .to_ne_bytes()
                .to_vec(),
                SampleType::F64 => if alpha {
                    1_f64
                } else if channel == 0 && !edit {
                    -0_f64
                } else if edit {
                    f64::from_bits(0x3fd3333333333334)
                } else {
                    f64::from_bits(0x3fb999999999999a)
                }
                .to_ne_bytes()
                .to_vec(),
            }
        })
        .collect()
}
fn setup(format: PixelFormat) -> (BackedLayer, BackingQuota, Arc<AtomicU64>, Arc<AtomicU64>) {
    let q = BackingQuota::new(QuotaLimits { resident: 16 << 20, scratch: 0, index: 1 << 20 });
    let d = NativePlaneDescriptor::new(format, Rect::new(0, 0, 3, 2), &vec![0; format.bytes_per_pixel()]).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let fail = Arc::new(AtomicU64::new(0));
    let native = Arc::new(Literal { descriptor: d, pixel: pixel(format, false), reads: reads.clone(), fail: fail.clone() });
    let raster = BackedRaster::new(native, q.clone(), 2, &control()).unwrap();
    (BackedLayer::new(raster, d, &control()).unwrap(), q, reads, fail)
}
fn decode(frame: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    ruzstd::decoding::StreamingDecoder::new(frame).unwrap().read_to_end(&mut result).unwrap();
    result
}
#[test]
fn independent_resident_save_hash_path_cow_zipcrc_and_zstd_native_bytes_match() {
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let format = PixelFormat::new(mode, sample, true);
            let (layer, q, _, _) = setup(format);
            let history = layer.clone();
            let edited = layer.try_edited_native(Rect::new(1, 0, 2, 1), &pixel(format, true), &control()).unwrap();
            let mut native = pixel(format, false).repeat(6);
            native[format.bytes_per_pixel()..format.bytes_per_pixel() * 2].copy_from_slice(&pixel(format, true));
            let surface = Surface::try_from_interleaved(format, Rect::new(0, 0, 3, 2), &native).unwrap();
            let mut doc = Document::new("Own native canonical", Size::new(3, 2), mode, sample);
            doc.layers.push(Layer::new("Own current", LayerContent::Raster(surface)));
            let resident_zip = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
            let resident = ZipReader::new(&resident_zip).unwrap();
            let tile_path = &resident.entries.iter().find(|e| e.name.starts_with("tiles/")).unwrap().name;
            let legacy_frame = resident.read_by_name(tile_path, 3 << 20).unwrap();
            let expected = decode(&legacy_frame);
            let ctl = control();
            let prepared = PreparedCanonicalTile::prepare(&edited, TileCoord::new(0, 0), &ctl).unwrap();
            let metadata = prepared.metadata();
            assert_eq!(metadata.entry_name().unwrap(), tile_path);
            assert_eq!(metadata.hash(), *blake3::hash(&expected).as_bytes());
            assert_eq!(metadata.format(), format);
            assert_eq!(metadata.length(), expected.len() as u64);
            let old = PreparedCanonicalTile::prepare(&history, TileCoord::new(0, 0), &ctl).unwrap();
            assert_ne!(old.metadata().hash(), metadata.hash());
            drop((old, history, edited, layer, doc));
            let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
            assert_eq!(prepared.write_entry(&mut writer).unwrap(), metadata);
            let bytes = writer.finish(&Interrupt::NONE).unwrap().into_inner();
            let mut archive = StreamReader::new(Cursor::new(&bytes), Limits::default(), &Interrupt::NONE).unwrap();
            let raw = archive.read_by_name(tile_path, 3 << 20, &Interrupt::NONE).unwrap();
            assert_eq!(decode(&raw), expected);
            assert_eq!(q.usage().unwrap(), QuotaUsage::default());
            let mut corrupted = bytes;
            let name_length = u16::from_le_bytes(corrupted[26..28].try_into().unwrap()) as usize;
            let extra_length = u16::from_le_bytes(corrupted[28..30].try_into().unwrap()) as usize;
            corrupted[30 + name_length + extra_length + 20] ^= 1;
            assert!(
                StreamReader::new(Cursor::new(&corrupted), Limits::default(), &Interrupt::NONE)
                    .unwrap()
                    .read_by_name(tile_path, 3 << 20, &Interrupt::NONE)
                    .is_err()
            );
        }
    }
}
#[test]
fn greater_than2mib_f64_default_tile_preparation_survives_all_layer_owners() {
    let (layer, q, _, _) = setup(PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true));
    let ctl = control();
    let object = PreparedCanonicalTile::prepare(&layer, TileCoord::new(2, 2), &ctl).unwrap();
    let metadata = object.metadata();
    assert!(metadata.length() > 2 << 20);
    let expected = vec![0; metadata.length() as usize];
    assert_eq!(metadata.hash(), *blake3::hash(&expected).as_bytes());
    drop(layer);
    assert!(q.usage().unwrap().resident > 2 << 20);
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    object.write_entry(&mut writer).unwrap();
    let bytes = writer.finish(&Interrupt::NONE).unwrap().into_inner();
    assert_eq!(
        decode(
            &StreamReader::new(Cursor::new(&bytes), Limits::default(), &Interrupt::NONE)
                .unwrap()
                .read_by_name(metadata.entry_name().unwrap(), 3 << 20, &Interrupt::NONE)
                .unwrap()
        ),
        expected
    );
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn accounted_object_current_history_native_hash_and_zip_match_legacy() {
    for sample in SampleType::ALL {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let (layer, q, _, _) = setup(format);
        let ctl = control();
        let history = layer.clone();
        let current = layer.try_edited_native(Rect::new(1, 0, 2, 1), &pixel(format, true), &ctl).unwrap();
        let old = PreparedCanonicalTile::prepare(&current, TileCoord::new(0, 0), &ctl).unwrap();
        let new = PreparedCanonicalTile::prepare(&current, TileCoord::new(0, 0), &ctl).unwrap();
        let metadata = new.metadata();
        assert_ne!(PreparedCanonicalTile::prepare(&history, TileCoord::new(0, 0), &ctl).unwrap().metadata().hash(), metadata.hash());
        drop((history, current, layer));
        let mut legacy = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
        old.write_entry(&mut legacy).unwrap();
        let expected = legacy.finish(&Interrupt::NONE).unwrap().into_inner();
        let mut accounted = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl, &q).unwrap();
        assert_eq!(new.write_entry_accounted(&mut accounted).unwrap(), metadata);
        let actual = accounted.finish_accounted(&ctl).unwrap().into_inner();
        assert_eq!(actual, expected);
        let mut reader = StreamReader::new_accounted(Cursor::new(&actual), Limits::default(), &ctl, &q).unwrap();
        let frame = reader.read_by_name(metadata.entry_name().unwrap(), 3 << 20, &Interrupt::NONE).unwrap();
        assert_eq!(*blake3::hash(&decode(&frame)).as_bytes(), metadata.hash());
        drop(reader);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn prep_late_work_cancel_io_and_quota_publish_nothing_and_refund() {
    for kind in 0..4 {
        let (layer, q, reads, fail) = setup(PixelFormat::RGBA8);
        let initial = q.usage().unwrap();
        if kind == 0 {
            fail.store(2, Ordering::SeqCst);
        }
        let calls = AtomicU64::new(0);
        let cancelled = AtomicBool::new(false);
        let work = |n| {
            if kind == 1 && n == 4096 && calls.fetch_add(1, Ordering::SeqCst) > 2 {
                return Err(BackingError::Work);
            }
            if kind == 2 && n == 74 {
                cancelled.store(true, Ordering::SeqCst);
            }
            Ok(())
        };
        let check = || cancelled.load(Ordering::SeqCst);
        let interrupt = Interrupt::cancel_only(&check);
        let ctl = ReadControl { interrupt: &interrupt, work: &work };
        let blocker = if kind == 3 { Some(q.reserve(QuotaKind::Resident, (16 << 20) - initial.resident).unwrap()) } else { None };
        let result = PreparedCanonicalTile::prepare(&layer, TileCoord::new(0, 0), &ctl);
        match kind {
            0 => assert!(matches!(&result, Err(TileObjectError::Canonical(CanonicalTileError::View(_))))),
            1 => assert!(matches!(&result, Err(TileObjectError::Canonical(CanonicalTileError::Backing(BackingError::Work))))),
            2 => assert!(matches!(&result, Err(TileObjectError::Backing(BackingError::Cancelled)))),
            _ => assert!(matches!(&result, Err(TileObjectError::Backing(BackingError::Quota)))),
        }
        if kind == 3 {
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }
        drop((result, blocker, layer));
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
struct DiskFull {
    inner: Cursor<Vec<u8>>,
    left: usize,
    written: Arc<AtomicU64>,
}
#[test]
fn accounted_object_mixed_or_legacy_writer_refuses_before_entry_bytes() {
    for legacy in [false, true] {
        let (layer, q, _, _) = setup(PixelFormat::RGBA8);
        let ctl = control();
        let object = PreparedCanonicalTile::prepare(&layer, TileCoord::new(0, 0), &ctl).unwrap();
        drop(layer);
        let other = BackingQuota::new(QuotaLimits { resident: 16 << 20, scratch: 0, index: 1 << 20 });
        let written = Arc::new(AtomicU64::new(0));
        let output = DiskFull { inner: Cursor::new(Vec::new()), left: usize::MAX, written: written.clone() };
        let mut writer =
            if legacy { Writer::new(output, Limits::default()).unwrap() } else { Writer::new_accounted(output, Limits::default(), &ctl, &other).unwrap() };
        let baseline = other.usage().unwrap();
        assert!(object.write_entry_accounted(&mut writer).is_err());
        assert_eq!(written.load(Ordering::SeqCst), 0);
        assert_eq!(other.usage().unwrap(), baseline);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        drop(writer);
        assert_eq!(other.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn accounted_object_large_default_and_write_failures_refund_same_ledger() {
    let (layer, q, _, _) = setup(PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true));
    let ctl = control();
    let object = PreparedCanonicalTile::prepare(&layer, TileCoord::new(2, 2), &ctl).unwrap();
    let metadata = object.metadata();
    assert!(metadata.length() > 2 << 20);
    drop(layer);
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl, &q).unwrap();
    object.write_entry_accounted(&mut writer).unwrap();
    let bytes = writer.finish_accounted(&ctl).unwrap().into_inner();
    let frame = StreamReader::new(Cursor::new(&bytes), Limits::default(), &Interrupt::NONE)
        .unwrap()
        .read_by_name(metadata.entry_name().unwrap(), 3 << 20, &Interrupt::NONE)
        .unwrap();
    assert_eq!(decode(&frame), vec![0; metadata.length() as usize]);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    for kind in 0..4 {
        let (layer, q, reads, fail) = setup(PixelFormat::RGBA8);
        let armed = AtomicBool::new(false);
        let calls = AtomicU64::new(0);
        let cancelled = AtomicBool::new(false);
        let work = |_| {
            if armed.load(Ordering::SeqCst) && calls.fetch_add(1, Ordering::SeqCst) > 20 {
                if kind == 1 {
                    return Err(BackingError::Work);
                }
                if kind == 2 {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        };
        let check = || cancelled.load(Ordering::SeqCst);
        let interrupt = Interrupt::cancel_only(&check);
        let ctl = ReadControl { interrupt: &interrupt, work: &work };
        let object = PreparedCanonicalTile::prepare(&layer, TileCoord::new(0, 0), &ctl).unwrap();
        let read_count = reads.load(Ordering::SeqCst);
        fail.store(read_count + 1, Ordering::SeqCst);
        drop(layer);
        let written = Arc::new(AtomicU64::new(0));
        let output = DiskFull { inner: Cursor::new(Vec::new()), left: if kind == 0 { 200 } else { usize::MAX }, written: written.clone() };
        let mut writer = Writer::new_accounted(output, Limits::default(), &ctl, &q).unwrap();
        let held = if kind == 3 { Some(q.reserve(QuotaKind::Resident, (16 << 20) - q.usage().unwrap().resident).unwrap()) } else { None };
        armed.store(true, Ordering::SeqCst);
        assert!(object.write_entry_accounted(&mut writer).is_err());
        assert_eq!(reads.load(Ordering::SeqCst), read_count, "prepared pin never rereads source");
        if kind == 3 {
            assert_eq!(written.load(Ordering::SeqCst), 0);
        } else {
            assert!(written.load(Ordering::SeqCst) > 0);
        }
        cancelled.store(false, Ordering::SeqCst);
        assert!(writer.finish_accounted(&control()).is_err());
        drop(held);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
impl Write for DiskFull {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "own synthetic disk full"));
        }
        let n = bytes.len().min(self.left);
        self.left -= n;
        self.written.fetch_add(n as u64, Ordering::SeqCst);
        self.inner.write(&bytes[..n])
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for DiskFull {
    fn seek(&mut self, s: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(s)
    }
}
#[test]
fn writer_io_or_midframe_work_cancel_poison_archive_and_release_pin() {
    for kind in 0..3 {
        let (layer, q, _, _) = setup(PixelFormat::RGBA8);
        let armed = AtomicBool::new(false);
        let calls = AtomicU64::new(0);
        let cancelled = AtomicBool::new(false);
        let work = |_| {
            if armed.load(Ordering::SeqCst) && calls.fetch_add(1, Ordering::SeqCst) > 10 {
                if kind == 1 {
                    return Err(BackingError::Work);
                }
                if kind == 2 {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        };
        let check = || cancelled.load(Ordering::SeqCst);
        let interrupt = Interrupt::cancel_only(&check);
        let ctl = ReadControl { interrupt: &interrupt, work: &work };
        let object = PreparedCanonicalTile::prepare(&layer, TileCoord::new(0, 0), &ctl).unwrap();
        drop(layer);
        let written = Arc::new(AtomicU64::new(0));
        let output = DiskFull { inner: Cursor::new(Vec::new()), left: if kind == 0 { 200 } else { usize::MAX }, written: written.clone() };
        let mut writer = Writer::new(output, Limits::default()).unwrap();
        armed.store(true, Ordering::SeqCst);
        assert!(object.write_entry(&mut writer).is_err());
        assert!(written.load(Ordering::SeqCst) > 0);
        cancelled.store(false, Ordering::SeqCst);
        assert!(writer.finish(&Interrupt::NONE).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[derive(Debug)]
struct Generated(NativePlaneDescriptor);
impl NativePlaneStore for Generated {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        for (index, byte) in out.iter_mut().enumerate() {
            let at = offset + index as u64;
            let channel = at % 4;
            *byte = if channel == 3 { 255 } else { ((at / 4) >> (channel * 8)) as u8 };
        }
        Ok(())
    }
}
#[derive(Default)]
struct CountingOutput {
    position: u64,
    length: u64,
    written: u64,
}
impl Write for CountingOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.position = self.position.checked_add(bytes.len() as u64).ok_or_else(|| std::io::Error::other("counting position overflow"))?;
        self.length = self.length.max(self.position);
        self.written = self.written.checked_add(bytes.len() as u64).ok_or_else(|| std::io::Error::other("counting write overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for CountingOutput {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let at = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        self.position = u64::try_from(at).map_err(|_| std::io::Error::other("counting seek bounds"))?;
        Ok(self.position)
    }
}
#[test]
#[ignore = "release-only mathematical24MP canonical object hash/ZIPwrite measurement"]
fn release_24mp_canonical_tile_objects() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 6000, 4000), &[0; 4]).unwrap();
    let q = BackingQuota::new(QuotaLimits { resident: 8 << 20, scratch: 0, index: 1 << 20 });
    let ctl = control();
    let raster = BackedRaster::new(Arc::new(Generated(descriptor)), q.clone(), 2, &ctl).unwrap();
    let layer = BackedLayer::new(raster, descriptor, &ctl).unwrap();
    let mut writer = Writer::new(CountingOutput::default(), Limits::default()).unwrap();
    let start = std::time::Instant::now();
    let mut tiles = 0;
    let mut canonical_bytes = 0;
    let mut sampled = 0;
    for coord in descriptor.bounds().tiles() {
        let object = PreparedCanonicalTile::prepare(&layer, coord, &ctl).unwrap();
        canonical_bytes += object.metadata().length();
        sampled = sampled.max(q.usage().unwrap().resident);
        object.write_entry(&mut writer).unwrap();
        tiles += 1;
    }
    let output = writer.finish(&Interrupt::NONE).unwrap();
    println!(
        "mathematical24MP tileorder {tiles} objects logical{}B fullcanonical{canonical_bytes}B ZIP{}B writerbytes{} elapsed={:?} resident_sampled_prewrite{sampled}B (not actualdisk/RSS/transientpeak/aggregateZIPcharge)",
        descriptor.logical_bytes(),
        output.length,
        output.written,
        start.elapsed()
    );
    drop(layer);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    Ok(())
}
#[test]
#[ignore = "release-only generated24MP legacy/accounted canonical object comparison"]
fn release_24mp_accounted_canonical_objects() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let mut lengths = Vec::new();
    for accounted in [false, true] {
        let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 6000, 4000), &[0; 4]).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 8 << 20, scratch: 0, index: 1 << 20 });
        let peak = AtomicU64::new(0);
        let work = |_| {
            peak.fetch_max(q.usage().unwrap().resident, Ordering::SeqCst);
            Ok(())
        };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        let raster = BackedRaster::new(Arc::new(Generated(descriptor)), q.clone(), 2, &ctl).unwrap();
        let layer = BackedLayer::new(raster, descriptor, &ctl).unwrap();
        let mut writer = if accounted {
            Writer::new_accounted(CountingOutput::default(), Limits::default(), &ctl, &q).unwrap()
        } else {
            Writer::new(CountingOutput::default(), Limits::default()).unwrap()
        };
        let start = std::time::Instant::now();
        let mut tiles = 0;
        for coord in descriptor.bounds().tiles() {
            let object = PreparedCanonicalTile::prepare(&layer, coord, &ctl).unwrap();
            if accounted {
                object.write_entry_accounted(&mut writer).unwrap();
            } else {
                object.write_entry(&mut writer).unwrap();
            }
            tiles += 1;
        }
        let output = if accounted { writer.finish_accounted(&ctl).unwrap() } else { writer.finish(&Interrupt::NONE).unwrap() };
        println!(
            "generated24MP accounted={accounted} tiles={tiles} zip_bytes={} elapsed_ms={} callback_resident={} counting-output/noRSS/nativeMigration",
            output.length,
            start.elapsed().as_secs_f64() * 1000.,
            peak.load(Ordering::SeqCst)
        );
        lengths.push(output.length);
        drop(layer);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    assert_eq!(lengths[0], lengths[1]);
    Ok(())
}
