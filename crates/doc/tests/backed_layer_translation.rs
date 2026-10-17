//! UNRUN native coordinate-view ownership; no color/sample-domain qualification.
use photocraft_doc::{BackedLayer, BackedLayerError, ColorMode, PixelFormat, Rect, SampleType};
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn work(_: u64) -> Result<(), BackingError> {
    Ok(())
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &work }
}
#[derive(Debug)]
struct Native {
    descriptor: NativePlaneDescriptor,
    a: [u8; 40],
    b: [u8; 40],
    reads: AtomicU64,
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Native {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        if quota.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        let bpp = self.descriptor.format().bytes_per_pixel() as u64;
        for (i, byte) in output.iter_mut().enumerate() {
            let at = offset + i as u64;
            let pixel = if (at / bpp).is_multiple_of(2) { &self.a } else { &self.b };
            *byte = pixel[(at % bpp) as usize];
        }
        Ok(())
    }
}
fn words(sample: SampleType) -> ([u8; 40], [u8; 40]) {
    let mut a = [0; 40];
    let mut b = [0; 40];
    let channels: Vec<Vec<u8>> = match sample {
        SampleType::U8 => vec![vec![0x81], vec![0x03], vec![0xe7], vec![0xff]],
        SampleType::U16 => [0x8001u16, 0x1235, 0xabcd, 0xffff].map(|v| v.to_ne_bytes().to_vec()).to_vec(),
        SampleType::U32 => [16_777_217u32, 0x80000001, 0xfedcba98, u32::MAX].map(|v| v.to_ne_bytes().to_vec()).to_vec(),
        SampleType::F32 => [0x80000000u32, 0x7fc12345, 0x7f800000, 0x3f800000].map(|v| v.to_ne_bytes().to_vec()).to_vec(),
        SampleType::F64 => [0x8000000000000000u64, 0x7ff8123456789abc, 0x7ff0000000000000, 0x3ff0000000000000].map(|v| v.to_ne_bytes().to_vec()).to_vec(),
    };
    let n = sample.bytes();
    for (i, channel) in channels.iter().enumerate() {
        a[i * n..(i + 1) * n].copy_from_slice(channel);
        b[i * n..(i + 1) * n].copy_from_slice(channel);
    }
    // A second distinct raw word, without a decode/re-encode roundtrip.
    b[0] ^= 1;
    (a, b)
}
fn fixture(sample: SampleType) -> (BackedLayer, Arc<Native>, BackingQuota) {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 4 << 20, index: 4 << 20 });
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let (a, b) = words(sample);
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(-1, -1, 257, 2), &b[..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, (std::mem::size_of::<Native>() + 128) as u64).unwrap();
    let source = Arc::new(Native { descriptor, a, b, reads: AtomicU64::new(0), quota: q.clone(), _fee: fee });
    let raster = BackedRaster::new(source.clone(), q.clone(), 1, &ctl()).unwrap();
    (BackedLayer::new(raster, descriptor, &ctl()).unwrap(), source, q)
}
fn read(layer: &BackedLayer, x: i32, y: i32) -> Vec<u8> {
    let mut pixel = vec![0; layer.format().bytes_per_pixel()];
    layer.checked_view().read_logical_pixel_native_into(x, y, &mut pixel, &ctl()).unwrap();
    pixel
}

#[test]
fn translation_preserves_every_native_word_default_and_target_pin_grid() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let (base, source, q) = fixture(sample);
        for (dx, dy) in [(TILE_SIZE, -TILE_SIZE), (3, -5), (-259, 257)] {
            let translated = base.try_translated(dx, dy, &ctl()).unwrap();
            assert_eq!(source.reads.load(Ordering::Relaxed), 0, "translation performs no source IO");
            assert!(translated.quota().shares_ledger(&q));
            assert_eq!(translated.format(), base.format());
            assert_eq!(translated.default_bytes(), base.default_bytes());
            let r = base.operative_bounds();
            assert_eq!(translated.operative_bounds(), Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy));
            let target = translated.operative_bounds();
            for coord in target.tiles() {
                let pin = translated.checked_view().pin_tile(coord, &ctl()).unwrap();
                assert_eq!(pin.coord(), coord);
                let tile = Rect::new(coord.tx * TILE_SIZE, coord.ty * TILE_SIZE, (coord.tx + 1) * TILE_SIZE, (coord.ty + 1) * TILE_SIZE).intersect(&target);
                for ty in tile.y0..tile.y1 {
                    for tx in tile.x0..tile.x1 {
                        let (x, y) = (tx - dx, ty - dy);
                        let mut expected = vec![0; base.format().bytes_per_pixel()];
                        let index = ((y - r.y0) as u64 * r.width() as u64 + (x - r.x0) as u64) * expected.len() as u64;
                        source.read_exact_at(index, &mut expected, &ctl()).unwrap();
                        let mut actual = vec![0; expected.len()];
                        pin.read_pixel_native_into(tx, ty, &mut actual, &ctl()).unwrap();
                        assert_eq!(actual, expected);
                    }
                }
            }
            let coord = TileCoord::containing(translated.operative_bounds().x0, translated.operative_bounds().y0);
            let pin = translated.checked_view().pin_tile(coord, &ctl()).unwrap();
            assert_eq!(pin.coord(), coord);
            assert_eq!(pin.format(), base.format());
            let outside = (translated.operative_bounds().x0 - 1, translated.operative_bounds().y0);
            let outside_pin = translated.checked_view().pin_tile(TileCoord::containing(outside.0, outside.1), &ctl()).unwrap();
            let mut raw = vec![0; base.format().bytes_per_pixel()];
            outside_pin.read_pixel_native_into(outside.0, outside.1, &mut raw, &ctl()).unwrap();
            assert_eq!(raw, base.default_bytes());
            assert!(translated.checked_view().read_logical_pixel_native_into(outside.0, outside.1, &mut raw, &ctl()).is_err());
            drop((pin, outside_pin, translated));
            source.reads.store(0, Ordering::Relaxed);
        }
        drop((base, source));
        assert_eq!(q.usage().unwrap().resident, 0);
    }
}

#[test]
fn translated_cow_is_private_and_uses_inverse_root_coordinates() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let (base, source, q) = fixture(sample);
        let translated = base.try_translated(3, -5, &ctl()).unwrap();
        let history = translated.clone();
        let original = read(&base, 0, 0);
        let mut replacement = original.clone();
        replacement[0] ^= 0x5a;
        let edited = translated.try_edited_native(Rect::new(3, -5, 4, -4), &replacement, &ctl()).unwrap();
        assert_ne!(edited, translated);
        assert_eq!(read(&edited, 3, -5), replacement);
        assert_eq!(read(&history, 3, -5), original);
        assert_eq!(read(&base, 0, 0), original);
        let back = edited.try_translated(-3, 5, &ctl()).unwrap();
        assert_eq!(read(&back, 0, 0), replacement);
        assert_eq!(read(&back, -1, 0), read(&base, -1, 0));
        drop((back, edited, history, translated, base, source));
        assert_eq!(q.usage().unwrap().resident, 0);
    }
}

#[test]
fn noop_inverse_errors_and_cancel_keep_original_owner_and_ledger() {
    let (base, source, q) = fixture(SampleType::F64);
    let before = q.usage().unwrap();
    assert_eq!(base.try_translated(0, 0, &ctl()).unwrap(), base);
    let shifted = base.try_translated(7, -9, &ctl()).unwrap();
    assert_eq!(shifted.try_translated(-7, 9, &ctl()).unwrap(), base);
    assert_ne!(shifted, base);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), before);
    assert!(base.try_translated(i32::MAX, 0, &ctl()).is_err());
    assert!(base.try_translated(i32::MIN, 0, &ctl()).is_err());
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(base.try_translated(1, 1, &denied), Err(BackedLayerError::Backing(BackingError::Work))));
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(base.try_translated(1, 1, &ReadControl { interrupt: &interrupt, work: &work }), Err(BackedLayerError::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    drop((shifted, base, source));
    assert_eq!(q.usage().unwrap().resident, 0);
}
