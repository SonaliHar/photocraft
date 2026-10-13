use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, NativeTileVisitor, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 1 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Stream {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    behavior: AtomicUsize,
    reads: AtomicUsize,
}
impl NativePlaneStore for Stream {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Err(BackingError::Unsupported)
    }
    fn visit_native_tile(&self, _: TileCoord, c: &ReadControl<'_>, stage: &mut NativeTileVisitor<'_>) -> Result<bool, BackingError> {
        match self.behavior.load(Ordering::Relaxed) {
            1 => {
                stage(0, &self.bytes[..1])?;
                return Ok(false);
            }
            2 => {
                stage(1, &self.bytes[..1])?;
                return Ok(true);
            }
            3 => {
                stage(0, &self.bytes[..1])?;
                return Ok(true);
            }
            4 => {
                stage(0, &self.bytes[..65537])?;
                return Ok(true);
            }
            5 => return Ok(true),
            6 => {
                let _ = stage(1, &self.bytes[..1]);
                return Ok(true);
            }
            _ => {}
        }
        for (i, block) in self.bytes.chunks(65536).enumerate() {
            c.check(block.len() as u64)?;
            stage((i * 65536) as u64, block)?;
        }
        if self.behavior.load(Ordering::Relaxed) == 7 { Err(BackingError::Io) } else { Ok(true) }
    }
}
fn stream(format: PixelFormat, bounds: Rect, default: &[u8], behavior: usize) -> Arc<Stream> {
    Arc::new(Stream {
        descriptor: NativePlaneDescriptor::new(format, bounds, default).unwrap(),
        bytes: vec![0x59; format.bytes_per_pixel() * 256 * 256],
        behavior: AtomicUsize::new(behavior),
        reads: AtomicUsize::new(0),
    })
}
#[test]
fn stream_preserves_all_native_depth_defaults_and_clips_negative_extent_padding() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let format = PixelFormat::new(mode, sample, true);
            // Distinct exact bytes include signed-zero/NaN payload patterns at
            // wide depths without imposing any render/sample interpretation.
            let component = match sample {
                SampleType::U8 => vec![0x81],
                SampleType::U16 => 0x8001u16.to_ne_bytes().to_vec(),
                SampleType::U32 => 0x80000001u32.to_ne_bytes().to_vec(),
                SampleType::F32 => (-0.0f32).to_ne_bytes().to_vec(),
                SampleType::F64 => 0x7ff8000000000042u64.to_ne_bytes().to_vec(),
            };
            let default = component.repeat(format.channels());
            let q = quota();
            let store = stream(format, Rect::new(-1, 3, 0, 5), &default, 0);
            let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
            let pin = raster.pin_tile(TileCoord::new(-1, 0), &ctl()).unwrap();
            for (i, pixel) in pin.bytes().chunks_exact(format.bytes_per_pixel()).enumerate() {
                let expected = if i % 256 == 255 && (3..5).contains(&(i / 256)) { vec![0x59; default.len()] } else { default.clone() };
                assert_eq!(pixel, expected);
            }
            assert_eq!(store.reads.load(Ordering::Relaxed), 0);
            drop(pin);
            drop(raster);
            drop(store);
            assert_eq!(q.usage().unwrap(), Default::default());
        }
    }
}
#[test]
fn malformed_false_partial_gap_large_swallowed_callback_and_late_error_never_publish() {
    for behavior in [1, 2, 3, 4, 6, 7] {
        let q = quota();
        let store = stream(PixelFormat::RGBA8, Rect::new(0, 0, 256, 256), &[1, 2, 3, 4], behavior);
        let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
        let baseline = q.usage().unwrap();
        assert!(raster.pin_tile(TileCoord::new(0, 0), &ctl()).is_err(), "behavior{behavior}");
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(store.reads.load(Ordering::Relaxed), 0);
        drop(raster);
        drop(store);
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}
#[test]
fn hole_defaults_and_each_cancel_fence_and_output_quota_refund() {
    let q = quota();
    let store = stream(PixelFormat::RGBA8, Rect::new(0, 0, 256, 256), &[1, 2, 3, 4], 5);
    let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
    let pin = raster.pin_tile(TileCoord::new(0, 0), &ctl()).unwrap();
    assert!(pin.bytes().as_chunks::<4>().0.iter().all(|p| p == &[1, 2, 3, 4]));
    drop(pin);
    drop(raster);
    assert_eq!(q.usage().unwrap(), Default::default());
    let store = stream(PixelFormat::RGBA8, Rect::new(0, 0, 256, 256), &[0; 4], 0);
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
    let polls = AtomicUsize::new(0);
    let work = |_| {
        polls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let counted = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    drop(raster.pin_tile(TileCoord::new(0, 0), &counted).unwrap());
    let count = polls.load(Ordering::Relaxed);
    raster.clear_cache().unwrap();
    let baseline = q.usage().unwrap();
    // First, midstream, and final publication fences exercise both allocations
    // and already copied private bytes without quadratic default-fill work.
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
    drop(raster);
    drop(store);
    assert_eq!(q.usage().unwrap(), Default::default());
}
#[derive(Debug)]
struct Rows {
    descriptor: NativePlaneDescriptor,
    reads: AtomicUsize,
}
impl NativePlaneStore for Rows {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        out.fill(0x61);
        Ok(())
    }
}
#[test]
fn default_optional_visitor_keeps_existing_exact_row_fallback() {
    let q = quota();
    let store =
        Arc::new(Rows { descriptor: NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 256, 256), &[0; 4]).unwrap(), reads: AtomicUsize::new(0) });
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
    let pin = raster.pin_tile(TileCoord::new(0, 0), &ctl()).unwrap();
    assert!(pin.bytes().iter().all(|b| *b == 0x61));
    assert_eq!(store.reads.load(Ordering::Relaxed), 256);
    drop(pin);
    drop(raster);
    drop(store);
    assert_eq!(q.usage().unwrap(), Default::default());
}

#[test]
fn late_failed_stream_retains_old_pin_and_never_populates_cache() {
    let q = quota();
    let store = stream(PixelFormat::RGBA8, Rect::new(0, 0, 256, 256), &[0; 4], 0);
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
    let old = raster.pin_tile(TileCoord::new(0, 0), &ctl()).unwrap();
    raster.clear_cache().unwrap();
    let baseline = q.usage().unwrap();
    store.behavior.store(7, Ordering::Relaxed);
    assert!(raster.pin_tile(TileCoord::new(0, 0), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(old.bytes().iter().all(|b| *b == 0x59));
    // A previously failed complete stream cannot become a cache hit.
    assert!(raster.pin_tile(TileCoord::new(0, 0), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    drop(old);
    drop(raster);
    drop(store);
    assert_eq!(q.usage().unwrap(), Default::default());
}
