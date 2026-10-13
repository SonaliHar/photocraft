use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::BackedLayer;
use photocraft_format::canonical_tile::{CanonicalTileError, CanonicalTileReader};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Debug)]
struct Pattern {
    d: NativePlaneDescriptor,
    fail: Arc<AtomicBool>,
}
fn native_sample(sample: SampleType, index: u64) -> Vec<u8> {
    match sample {
        SampleType::U8 => vec![index.wrapping_mul(37) as u8],
        SampleType::U16 => (index.wrapping_add(0x8001) as u16).to_ne_bytes().to_vec(),
        SampleType::U32 => (index.wrapping_add(16777217) as u32).to_ne_bytes().to_vec(),
        SampleType::F32 => if index.is_multiple_of(2) { 0x80000000_u32 } else { 0x7fc00001_u32 }.to_ne_bytes().to_vec(),
        SampleType::F64 => if index.is_multiple_of(2) { 0x8000000000000000_u64 } else { 0x7ff8000000000001_u64 }.to_ne_bytes().to_vec(),
    }
}
impl NativePlaneStore for Pattern {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.d
    }
    fn read_exact_at(&self, o: u64, b: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(b.len() as u64)?;
        if self.fail.load(Ordering::SeqCst) {
            if let Some(v) = b.first_mut() {
                *v = 0xff;
            }
            return Err(BackingError::Io);
        }
        let sw = self.d.format().sample.bytes();
        for (i, v) in b.iter_mut().enumerate() {
            let at = o + i as u64;
            *v = native_sample(self.d.format().sample, at / sw as u64)[at as usize % sw];
        }
        Ok(())
    }
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn layer(format: PixelFormat, bounds: Rect) -> (BackedLayer, BackingQuota, Arc<AtomicBool>) {
    let q = BackingQuota::new(QuotaLimits { resident: 16 << 20, scratch: 0, index: 1 << 20 });
    let default = native_sample(format.sample, 0).repeat(format.channels());
    let d = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let store = Arc::new(Pattern { d, fail: fail.clone() });
    let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
    (BackedLayer::new(raster, d, &ctl()).unwrap(), q, fail)
}
fn expected(layer: &BackedLayer, coord: TileCoord) -> Vec<u8> {
    let f = layer.format();
    let mut out = Vec::new();
    for y in coord.ty * 256..coord.ty * 256 + 256 {
        for x in coord.tx * 256..coord.tx * 256 + 256 {
            let native = if layer.operative_bounds().contains(x, y) {
                let b = layer.operative_bounds();
                let pixel = ((i64::from(y) - i64::from(b.y0)) * i64::from(b.width()) + i64::from(x) - i64::from(b.x0)) as u64;
                (0..f.channels()).flat_map(|channel| native_sample(f.sample, pixel * f.channels() as u64 + channel as u64)).collect::<Vec<_>>()
            } else {
                layer.default_bytes().to_vec()
            };
            for bytes in native.chunks_exact(f.sample.bytes()) {
                let mut canonical = bytes.to_vec();
                if cfg!(target_endian = "big") {
                    canonical.reverse();
                }
                out.extend(canonical);
            }
        }
    }
    out
}
#[test]
fn independent_canonical_all_depths_rgb_gray_fragmented_hash_padding_and_seek() {
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let (layer, q, _) = layer(PixelFormat::new(mode, sample, true), Rect::new(-2, -1, 7, 3));
            let control = ctl();
            for coord in [TileCoord::new(-1, -1), TileCoord::new(0, 0), TileCoord::new(2, 2)] {
                let expected = expected(&layer, coord);
                let mut reader = CanonicalTileReader::new(&layer, coord, &control).unwrap();
                assert_eq!(reader.length(), expected.len() as u64);
                let mut all = Vec::new();
                let mut buffer = [0; 4093];
                loop {
                    let n = reader.read(&mut buffer).unwrap();
                    if n == 0 {
                        break;
                    }
                    all.extend_from_slice(&buffer[..n]);
                }
                assert_eq!(all, expected);
                assert_eq!(blake3::hash(&all), blake3::hash(&expected));
                for offset in [1, 3, 4095, reader.length() - 7] {
                    reader.seek(SeekFrom::Start(offset)).unwrap();
                    let mut out = [0; 7];
                    reader.read_exact(&mut out).unwrap();
                    assert_eq!(out, expected[offset as usize..offset as usize + 7]);
                }
                assert!(reader.seek(SeekFrom::Current(i64::MAX)).is_err());
                assert!(reader.seek(SeekFrom::Start(u64::MAX)).is_err());
            }
            drop(layer);
            assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        }
    }
}
#[test]
fn current_cow_history_pin_lifetime_and_greater_than2mib_tile_are_exact() {
    let (layer, q, _) = layer(PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true), Rect::new(0, 0, 1, 1));
    let history = layer.clone();
    let edited = layer.try_edited_native(Rect::new(0, 0, 1, 1), &[0x19; 40], &ctl()).unwrap();
    let control = ctl();
    let mut reader = CanonicalTileReader::new(&edited, TileCoord::new(0, 0), &control).unwrap();
    assert!(reader.length() > 2 << 20);
    let mut first = [0; 40];
    reader.read_exact(&mut first).unwrap();
    assert_eq!(first, [0x19; 40]);
    let mut original = CanonicalTileReader::new(&history, TileCoord::new(0, 0), &control).unwrap();
    original.read_exact(&mut first).unwrap();
    assert_ne!(first, [0x19; 40]);
    drop((layer, history, edited, original));
    assert!(q.usage().unwrap().resident > 2 << 20);
    reader.seek(SeekFrom::End(-5)).unwrap();
    reader.read_exact(&mut first[..5]).unwrap();
    drop(reader);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn late_final_work_cancel_and_seek_errors_leave_output_position_unchanged() {
    let (layer, q, _) = layer(PixelFormat::RGBA64F, Rect::new(0, 0, 1, 1));
    let armed = AtomicBool::new(false);
    let calls = AtomicU64::new(0);
    let work = |_| {
        if armed.load(Ordering::SeqCst) && calls.fetch_add(1, Ordering::SeqCst) >= 2 { Err(BackingError::Work) } else { Ok(()) }
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut reader = CanonicalTileReader::new(&layer, TileCoord::new(0, 0), &control).unwrap();
    let held = q.usage().unwrap();
    armed.store(true, Ordering::SeqCst);
    let mut output = [0xa5; 13];
    assert_eq!(reader.try_read(&mut output), Err(CanonicalTileError::Backing(BackingError::Work)));
    assert_eq!(output, [0xa5; 13]);
    assert_eq!(reader.position(), 0);
    assert_eq!(q.usage().unwrap(), held);
    drop(reader);
    let calls = AtomicU64::new(0);
    let armed = AtomicBool::new(false);
    let cancelled = || armed.load(Ordering::SeqCst) && calls.load(Ordering::SeqCst) >= 3;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let work = |_| {
        if armed.load(Ordering::SeqCst) {
            calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    };
    let control = ReadControl { interrupt: &interrupt, work: &work };
    let mut reader = CanonicalTileReader::new(&layer, TileCoord::new(0, 0), &control).unwrap();
    armed.store(true, Ordering::SeqCst);
    assert_eq!(reader.try_read(&mut output), Err(CanonicalTileError::Backing(BackingError::Cancelled)));
    assert_eq!(output, [0xa5; 13]);
    assert_eq!(reader.position(), 0);
    assert!(reader.try_seek(SeekFrom::Start(1)).is_err());
    assert_eq!(reader.position(), 0);
    drop((reader, layer));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn constructor_io_quota_and_final_cancel_never_publish_or_leak() {
    let (layer, q, fail) = layer(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1));
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (16 << 20) - baseline.resident).unwrap();
    assert!(matches!(CanonicalTileReader::new(&layer, TileCoord::new(0, 0), &ctl()), Err(CanonicalTileError::Backing(BackingError::Quota))));
    drop(blocker);
    fail.store(true, Ordering::SeqCst);
    assert!(matches!(CanonicalTileReader::new(&layer, TileCoord::new(0, 0), &ctl()), Err(CanonicalTileError::View(_))));
    assert_eq!(q.usage().unwrap(), baseline);
    fail.store(false, Ordering::SeqCst);
    let calls = AtomicU64::new(0);
    let work = |n| {
        calls.fetch_add(1, Ordering::SeqCst);
        if n == 4096 { Err(BackingError::Cancelled) } else { Ok(()) }
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(CanonicalTileReader::new(&layer, TileCoord::new(2, 2), &control).is_err());
    assert!(calls.load(Ordering::SeqCst) > 1);
    drop(layer);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
