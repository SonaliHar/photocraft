use photocraft_doc::{BackedLayer, BackedLayerError, ColorMode, PixelFormat, Rect, SampleType};
use photocraft_raster::Interrupt;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use photocraft_raster::backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

fn work(_: u64) -> Result<(), BackingError> {
    Ok(())
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &work }
}
fn quota(resident: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident, scratch: 1 << 20, index: 1 << 20 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: AtomicU64,
    fail_after: AtomicU64,
    dropped: Arc<AtomicBool>,
}
impl Drop for Store {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(output.len() as u64)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        if offset >= self.fail_after.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        if offset.checked_add(output.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Integrity);
        }
        let pixel = self.descriptor.default_bytes();
        for (i, b) in output.iter_mut().enumerate() {
            *b = pixel[((offset + i as u64) % pixel.len() as u64) as usize];
        }
        Ok(())
    }
}
fn store(format: PixelFormat, bounds: Rect, default: &[u8]) -> Arc<Store> {
    Arc::new(Store {
        descriptor: NativePlaneDescriptor::new(format, bounds, default).unwrap(),
        reads: AtomicU64::new(0),
        fail_after: AtomicU64::new(u64::MAX),
        dropped: Arc::new(AtomicBool::new(false)),
    })
}
fn layer(store: Arc<Store>, q: BackingQuota) -> BackedLayer {
    let descriptor = store.descriptor;
    BackedLayer::new(BackedRaster::new(store, q, 2, &control()).unwrap(), descriptor, &control()).unwrap()
}
fn read(layer: &BackedLayer, x: i32, y: i32) -> Vec<u8> {
    let mut bytes = vec![0; layer.format().bytes_per_pixel()];
    layer.checked_view().read_logical_pixel_native_into(x, y, &mut bytes, &control()).unwrap();
    bytes
}

#[test]
fn descriptor_format_bounds_and_default_are_exact() {
    let base = store(PixelFormat::RGBA64F, Rect::new(-2, -1, 2, 1), &[0; 32]);
    let q = quota(8 << 20);
    for expected in [
        NativePlaneDescriptor::new(PixelFormat::RGBA32F, base.descriptor.bounds(), &[0; 16]).unwrap(),
        NativePlaneDescriptor::new(PixelFormat::RGBA64F, Rect::new(-1, -1, 2, 1), &[0; 32]).unwrap(),
        NativePlaneDescriptor::new(PixelFormat::RGBA64F, base.descriptor.bounds(), &[0x80; 32]).unwrap(),
    ] {
        let raster = BackedRaster::new(base.clone(), q.clone(), 2, &control()).unwrap();
        assert_eq!(BackedLayer::new(raster, expected, &control()).unwrap_err(), BackedLayerError::DescriptorMismatch);
    }
    assert_eq!(base.reads.load(Ordering::SeqCst), 0);
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn exact_all_depth_defaults_cow_and_history_ownership() {
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let format = PixelFormat::new(mode, sample, true);
            let mut default = vec![0; format.bytes_per_pixel()];
            // Native signed-zero and adjacent low integer bits, not display values.
            if sample == SampleType::F64 {
                default[..8].copy_from_slice(&(-0.0_f64).to_ne_bytes());
            }
            if sample == SampleType::U32 {
                default[..4].copy_from_slice(&16_777_217_u32.to_ne_bytes());
            }
            let owner = store(format, Rect::new(-2, -1, 3, 2), &default);
            let dropped = owner.dropped.clone();
            let q = quota(32 << 20);
            let mut current = layer(owner.clone(), q.clone());
            assert_eq!(current.operative_bounds(), owner.descriptor.bounds());
            assert_eq!(current.default_bytes(), default);
            let past = current.clone();
            assert_eq!(current, past);
            drop(owner);
            let bytes = vec![0x91; format.bytes_per_pixel()];
            current.try_write_native(Rect::new(-1, 0, 0, 1), &bytes, &control()).unwrap();
            assert_ne!(current, past);
            assert_eq!(read(&current, -1, 0), bytes);
            assert_eq!(read(&past, -1, 0), default);
            assert_eq!(read(&current, -2, -1), default);
            let pin = current.checked_view().pin_tile(photocraft_geom::TileCoord::new(-1, 0), &control()).unwrap();
            let future = current.clone();
            current = past.clone(); // undo shares exact prior ownership
            assert_eq!(current, past);
            assert_eq!(read(&future, -1, 0), bytes);
            drop((past, future, current));
            assert!(dropped.load(Ordering::SeqCst));
            assert!(q.usage().unwrap().resident > 0); // pin retains payload lease
            drop(pin);
            assert_eq!(q.usage().unwrap().resident, 0);
        }
    }
}

#[test]
fn equality_never_reads_and_distinct_equal_stores_need_explicit_compare() {
    let a = store(PixelFormat::RGBA8, Rect::new(0, 0, 2, 1), &[1, 2, 3, 4]);
    let b = store(PixelFormat::RGBA8, a.descriptor.bounds(), a.descriptor.default_bytes());
    let x = layer(a.clone(), quota(2 << 20));
    let y = layer(b.clone(), quota(2 << 20));
    assert_eq!(x, x.clone());
    assert_ne!(x, y);
    assert_eq!(a.reads.load(Ordering::SeqCst) + b.reads.load(Ordering::SeqCst), 0);
    assert!(x.try_content_equal(&y, &control()).unwrap());
    let bad = store(PixelFormat::RGBA8, a.descriptor.bounds(), a.descriptor.default_bytes());
    bad.fail_after.store(0, Ordering::SeqCst);
    let z = layer(bad, quota(2 << 20));
    assert_ne!(x, z);
    assert_eq!(x.try_content_equal(&z, &control()), Err(BackedLayerError::Backing(BackingError::Io)));
}

#[test]
fn second_tile_io_failure_does_not_publish_partial_edit() {
    let source = store(PixelFormat::RGBA8, Rect::new(0, 0, 257, 1), &[1, 2, 3, 4]);
    source.fail_after.store(1024, Ordering::SeqCst);
    let q = quota(4 << 20);
    let mut current = layer(source.clone(), q.clone());
    let past = current.clone();
    assert_eq!(current.try_write_native(current.operative_bounds(), &vec![9; 257 * 4], &control()), Err(BackedLayerError::Backing(BackingError::Io)));
    assert_eq!(current, past);
    assert_eq!(read(&current, 0, 0), [1, 2, 3, 4]);
    assert!(source.reads.load(Ordering::SeqCst) > 1);
    drop((current, past, source));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn quota_length_and_outside_edit_errors_are_atomic() {
    let q = quota(1 << 20); // one F64 RGBA tile itself exceeds this cap
    let mut current = layer(store(PixelFormat::RGBA64F, Rect::new(0, 0, 2, 1), &[0; 32]), q.clone());
    let past = current.clone();
    assert_eq!(current.try_write_native(Rect::new(0, 0, 1, 1), &[0; 32], &control()), Err(BackedLayerError::Backing(BackingError::Quota)));
    assert_eq!(current.try_write_native(Rect::new(0, 0, 1, 1), &[0; 31], &control()), Err(BackedLayerError::Backing(BackingError::Length)));
    assert_eq!(current.try_write_native(Rect::new(-1, 0, 1, 1), &[0; 32], &control()), Err(BackedLayerError::Backing(BackingError::Bounds)));
    assert_eq!(current, past);
    drop((current, past));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn final_wrapper_work_and_cancel_poll_precedes_publication() {
    let q = quota(4 << 20);
    let mut current = layer(store(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4]), q.clone());
    let bounds = current.operative_bounds();
    // Warm derived cache, then count the exact complete staging path.
    drop(current.try_edited_native(bounds, &[1; 4], &control()).unwrap());
    let count = AtomicU64::new(0);
    let meter = |_: u64| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    drop(current.try_edited_native(bounds, &[1; 4], &ReadControl { interrupt: &Interrupt::NONE, work: &meter }).unwrap());
    let last = count.load(Ordering::SeqCst);
    assert!(last > 3);
    let past = current.clone();
    let before = q.usage().unwrap();
    count.store(0, Ordering::SeqCst);
    let denied = |_: u64| {
        if count.fetch_add(1, Ordering::SeqCst) + 1 == last { Err(BackingError::Work) } else { Ok(()) }
    };
    assert_eq!(
        current.try_write_native(bounds, &[1; 4], &ReadControl { interrupt: &Interrupt::NONE, work: &denied }),
        Err(BackedLayerError::Backing(BackingError::Work))
    );
    assert_eq!(current, past);
    assert_eq!(q.usage().unwrap(), before);
    count.store(0, Ordering::SeqCst);
    let cancelled = AtomicBool::new(false);
    let cancelling = |_: u64| {
        if count.fetch_add(1, Ordering::SeqCst) + 1 == last {
            cancelled.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let cancel = || cancelled.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&cancel);
    assert_eq!(
        current.try_write_native(bounds, &[1; 4], &ReadControl { interrupt: &interrupt, work: &cancelling }),
        Err(BackedLayerError::Backing(BackingError::Cancelled))
    );
    assert_eq!(current, past);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(read(&current, 0, 0), [0; 4]);
    drop((current, past));
    assert_eq!(q.usage().unwrap().resident, 0);
}
