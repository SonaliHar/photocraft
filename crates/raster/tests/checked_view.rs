use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::backing::{BackingError, ReadControl};
use photocraft_raster::checked_view::CheckedRasterView;
use photocraft_raster::{Interrupt, Surface};
fn unlimited(_: u64) -> Result<(), BackingError> {
    Ok(())
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &unlimited }
}
#[test]
fn checked_view_u32_never_narrows_adjacent_native_codes() {
    let mut surface = Surface::new(PixelFormat::RGBA32);
    let bytes = [16777217u32.to_ne_bytes(), 16777218u32.to_ne_bytes(), u32::MAX.to_ne_bytes(), 0u32.to_ne_bytes()].concat();
    surface.try_write_interleaved(Rect::new(0, 0, 1, 1), &bytes).unwrap();
    let view = CheckedRasterView::resident(&surface, Rect::new(0, 0, 1, 1)).unwrap();
    let mut output = [0.; 4];
    view.read_pixel_f64_into(0, 0, &mut output, &control()).unwrap();
    assert_eq!(output[0], f64::from(16777217u32) / f64::from(u32::MAX));
    assert_eq!(output[1], f64::from(16777218u32) / f64::from(u32::MAX));
}
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64};
use photocraft_geom::TileCoord;
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits};
use photocraft_raster::backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore};
use photocraft_raster::checked_view::{NativeTileBytes, RasterViewError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 1 << 20, index: 1 << 20 })
}
#[derive(Debug)]
struct Memory {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    _lease: QuotaLease,
    fail: Arc<AtomicBool>,
}
impl NativePlaneStore for Memory {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(output.len() as u64)?;
        if self.fail.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(output.len()).ok_or(BackingError::Bounds)?;
        output.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Integrity)?);
        Ok(())
    }
}
fn backed(format: PixelFormat, bounds: Rect, default: &[u8], bytes: Vec<u8>, q: &BackingQuota, fail: Arc<AtomicBool>) -> BackedRaster {
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    let store = Memory { descriptor: NativePlaneDescriptor::new(format, bounds, default).unwrap(), bytes, _lease: lease, fail };
    BackedRaster::new(Arc::new(store), q.clone(), 2, &control()).unwrap()
}
fn pixel(format: PixelFormat, seed: u8) -> Vec<u8> {
    (0..format.channels())
        .flat_map(|i| {
            let last_alpha = format.alpha && i + 1 == format.channels();
            match format.sample {
                SampleType::U8 => vec![if last_alpha { 0 } else { seed.saturating_add(i as u8) }],
                SampleType::U16 => (if last_alpha { 0 } else { 257u16 + u16::from(seed) + i as u16 }).to_ne_bytes().to_vec(),
                SampleType::U32 => (if last_alpha { 0 } else { 16777217u32 + u32::from(seed) + i as u32 }).to_ne_bytes().to_vec(),
                SampleType::F32 => (if i == 0 {
                    -0.0f32
                } else if last_alpha {
                    0.0
                } else {
                    0.100001f32 + f32::from(seed)
                })
                .to_ne_bytes()
                .to_vec(),
                SampleType::F64 => (if i == 0 {
                    -0.0f64
                } else if last_alpha {
                    0.0
                } else {
                    0.10000000000000002f64 + f64::from(seed)
                })
                .to_ne_bytes()
                .to_vec(),
            }
        })
        .collect()
}
#[test]
fn exact_native_f64_pin_and_defaults_all80_formats() {
    let modes = [
        ColorMode::Bitmap,
        ColorMode::Grayscale,
        ColorMode::Indexed,
        ColorMode::Rgb,
        ColorMode::Cmyk,
        ColorMode::Lab,
        ColorMode::Multichannel,
        ColorMode::Duotone,
    ];
    for mode in modes {
        for sample in SampleType::ALL {
            for alpha in [false, true] {
                let format = PixelFormat::new(mode, sample, alpha);
                let expected = pixel(format, 3);
                let default = pixel(format, 7);
                let bounds = Rect::new(-1, -1, 2, 1);
                let mut surface = Surface::try_with_default_bytes(format, &default).unwrap();
                surface.try_write_interleaved(Rect::new(-1, -1, 0, 0), &expected).unwrap();
                let mut raw = default.repeat(6);
                raw[..expected.len()].copy_from_slice(&expected);
                let q = quota();
                let raster = backed(format, bounds, &default, raw, &q, Arc::new(AtomicBool::new(false)));
                let resident = CheckedRasterView::resident_with_control(&surface, bounds, &control()).unwrap();
                let owned = CheckedRasterView::backed(&raster);
                for view in [resident, owned] {
                    assert_eq!(view.format(), format);
                    assert_eq!(view.default_bytes(), default);
                    assert_eq!(view.logical_bounds(), bounds);
                    for (x, y, want) in [(-1, -1, &expected), (-2, -1, &default), (1, 0, &default)] {
                        let mut actual = vec![55; expected.len()];
                        view.read_pixel_native_into(x, y, &mut actual, &control()).unwrap();
                        assert_eq!(&actual, want);
                        let mut f64s = vec![f64::NAN; format.channels()];
                        view.read_pixel_f64_into(x, y, &mut f64s, &control()).unwrap();
                        for (i, value) in f64s.iter().enumerate() {
                            assert_eq!(value.to_bits(), try_read_sample_f64(want, format.sample, i).unwrap().to_bits());
                        }
                        let pin = view.pin_tile(TileCoord::containing(x, y), &control()).unwrap();
                        assert_eq!(pin.format(), format);
                        assert_eq!(pin.coord(), TileCoord::containing(x, y));
                        pin.read_pixel_native_into(x, y, &mut actual, &control()).unwrap();
                        assert_eq!(&actual, want);
                    }
                }
                drop(raster);
                assert_eq!(q.usage().unwrap().resident, 0);
            }
        }
    }
}
#[test]
fn owned_pin_survives_raster_view_and_source_drop() {
    let q = quota();
    let pin = {
        let raster = backed(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4], vec![9, 8, 7, 0], &q, Arc::new(AtomicBool::new(false)));
        let view = CheckedRasterView::backed(&raster);
        let pin = view.pin_tile(TileCoord::new(0, 0), &control()).unwrap();
        raster.clear_cache().unwrap();
        pin
    };
    assert_eq!(q.usage().unwrap().resident, 256 * 256 * 4 + 128);
    let mut actual = [0; 4];
    pin.read_pixel_native_into(0, 0, &mut actual, &control()).unwrap();
    assert_eq!(actual, [9, 8, 7, 0]);
    assert!(matches!(pin.encoded(), NativeTileBytes::FullTile(_)));
    drop(pin);
    assert_eq!(q.usage().unwrap().resident, 0);
    let resident_pin = {
        let mut surface = Surface::new(PixelFormat::RGBA8);
        surface.try_write_interleaved(Rect::new(0, 0, 1, 1), &[1, 2, 3, 4]).unwrap();
        CheckedRasterView::resident(&surface, Rect::new(0, 0, 1, 1)).unwrap().pin_tile(TileCoord::new(0, 0), &control()).unwrap()
    };
    resident_pin.read_pixel_native_into(0, 0, &mut actual, &control()).unwrap();
    assert_eq!(actual, [1, 2, 3, 4]);
}
#[test]
fn resident_default_pin_has_only_inline_pixel_and_no_surface_tiles() {
    let q = quota();
    let pin = {
        let surface = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &[0x80; 32]).unwrap();
        let view = CheckedRasterView::resident(&surface, Rect::new(-1, -1, 1, 1)).unwrap();
        let pin = view.pin_tile(TileCoord::new(-1, -1), &control()).unwrap();
        assert_eq!(surface.tile_count(), 0);
        pin
    };
    assert_eq!(pin.encoded(), NativeTileBytes::DefaultPixel(&[0x80; 32]));
    assert_eq!(q.usage().unwrap().resident, 0);
    let mut actual = [0; 32];
    pin.read_pixel_native_into(-1, -1, &mut actual, &control()).unwrap();
    assert_eq!(actual, [0x80; 32]);
}
#[test]
fn late_io_and_integrity_preserve_both_pixel_outputs() {
    let q = quota();
    let fail = Arc::new(AtomicBool::new(true));
    let raster = backed(PixelFormat::RGBA64F, Rect::new(0, 0, 1, 1), &[0; 32], vec![0; 32], &q, fail.clone());
    let base = q.usage().unwrap().resident;
    let view = CheckedRasterView::backed(&raster);
    let mut bytes = [77; 32];
    let mut values = [-7.0; 4];
    assert_eq!(view.read_pixel_native_into(0, 0, &mut bytes, &control()), Err(RasterViewError::Backed(BackingError::Io)));
    assert_eq!(view.read_pixel_f64_into(0, 0, &mut values, &control()), Err(RasterViewError::Backed(BackingError::Io)));
    assert_eq!(bytes, [77; 32]);
    assert_eq!(values, [-7.0; 4]);
    assert_eq!(q.usage().unwrap().resident, base);
    fail.store(false, Ordering::SeqCst);
    let corrupt = backed(PixelFormat::RGBA8, Rect::new(0, 0, 2, 1), &[0; 4], vec![0; 4], &q, fail);
    let v = CheckedRasterView::backed(&corrupt);
    assert_eq!(v.read_pixel_f64_into(1, 0, &mut values, &control()), Err(RasterViewError::Backed(BackingError::Integrity)));
    assert_eq!(values, [-7.0; 4]);
}
#[test]
fn cancel_after_staging_and_work_denial_never_publish_pixel() {
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.try_write_interleaved(Rect::new(0, 0, 1, 1), &[8; 4]).unwrap();
    let view = CheckedRasterView::resident(&surface, Rect::new(0, 0, 1, 1)).unwrap();
    let mut bytes = [77; 4];
    let mut values = [-7.0; 4];
    let calls = AtomicU64::new(0);
    let cancel = || calls.load(Ordering::SeqCst) > 7;
    let work = |_: u64| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    let interrupt = Interrupt::cancel_only(&cancel);
    let ctl = ReadControl { interrupt: &interrupt, work: &work };
    assert_eq!(view.read_pixel_f64_into(0, 0, &mut values, &ctl), Err(RasterViewError::Control(BackingError::Cancelled)));
    assert_eq!(values, [-7.0; 4]);
    let deny = |_: u64| Err(BackingError::Work);
    assert_eq!(
        view.read_pixel_native_into(0, 0, &mut bytes, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }),
        Err(RasterViewError::Control(BackingError::Work))
    );
    assert_eq!(bytes, [77; 4]);
    assert_eq!(
        CheckedRasterView::resident_with_control(&surface, Rect::new(0, 0, 1, 1), &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).unwrap_err(),
        RasterViewError::Control(BackingError::Work)
    );
}
#[test]
fn hostile_resident_layout_origins_and_pin_coordinates_are_errors() {
    let mut other = Surface::new(PixelFormat::RGBA8);
    other.try_write_interleaved(Rect::new(0, 0, 1, 1), &[3; 4]).unwrap();
    let tile = other.tile(TileCoord::new(0, 0)).unwrap().clone();
    let mut forged = Surface::new(PixelFormat::RGBA64F);
    forged.put_tiles([(TileCoord::new(0, 0), tile.clone())]);
    let view = CheckedRasterView::resident(&forged, Rect::new(0, 0, 1, 1)).unwrap();
    let mut bytes = [19; 32];
    assert_eq!(view.read_pixel_native_into(0, 0, &mut bytes, &control()), Err(RasterViewError::Resident(photocraft_raster::RasterInputError::LengthMismatch)));
    assert_eq!(bytes, [19; 32]);
    let pin = CheckedRasterView::resident(&other, Rect::new(0, 0, 1, 1)).unwrap().pin_tile(TileCoord::new(0, 0), &control()).unwrap();
    let mut rgba = [55; 4];
    assert_eq!(pin.read_pixel_native_into(-1, 0, &mut rgba, &control()), Err(RasterViewError::Bounds));
    assert_eq!(rgba, [55; 4]);
    assert_eq!(pin.read_pixel_native_into(0, 0, &mut [0; 3], &control()), Err(RasterViewError::Length));
    assert_eq!(view.pin_tile(TileCoord::new(i32::MAX, 0), &control()).unwrap_err(), RasterViewError::Bounds);
    forged.put_tiles([(TileCoord::new(i32::MAX, 0), tile)]);
    assert!(matches!(CheckedRasterView::resident(&forged, Rect::new(0, 0, 1, 1)), Err(RasterViewError::Resident(_))));
    assert!(matches!(CheckedRasterView::resident(&other, Rect::EMPTY), Err(RasterViewError::Descriptor(BackingError::Bounds))));
}
#[test]
fn wrong_native_and_f64_lengths_are_atomic_for_both_branches() {
    let surface = Surface::new(PixelFormat::RGBA8);
    let q = quota();
    let raster = backed(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4], vec![0; 4], &q, Arc::new(AtomicBool::new(false)));
    for view in [CheckedRasterView::resident(&surface, Rect::new(0, 0, 1, 1)).unwrap(), CheckedRasterView::backed(&raster)] {
        let mut bytes = [9; 3];
        let mut f64s = [-9.; 3];
        assert_eq!(view.read_pixel_native_into(0, 0, &mut bytes, &control()), Err(RasterViewError::Length));
        assert_eq!(view.read_pixel_f64_into(0, 0, &mut f64s, &control()), Err(RasterViewError::Length));
        assert_eq!(bytes, [9; 3]);
        assert_eq!(f64s, [-9.; 3]);
    }
}
#[test]
fn raw_retained_bytes_and_logical_coverage_are_distinct() {
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.try_write_interleaved(Rect::new(0, 0, 2, 1), &[1, 2, 3, 4, 99, 98, 97, 96]).unwrap();
    let bounds = Rect::new(0, 0, 1, 1);
    let q = quota();
    let raster = backed(PixelFormat::RGBA8, bounds, &[0; 4], vec![1, 2, 3, 4], &q, Arc::new(AtomicBool::new(false)));
    let resident = CheckedRasterView::resident(&surface, bounds).unwrap();
    let disk = CheckedRasterView::backed(&raster);
    let mut actual = [0; 4];
    resident.read_pixel_native_into(1, 0, &mut actual, &control()).unwrap();
    assert_eq!(actual, [99, 98, 97, 96]);
    disk.read_pixel_native_into(1, 0, &mut actual, &control()).unwrap();
    assert_eq!(actual, [0; 4]);
    for view in [resident, disk] {
        actual.fill(77);
        let mut floats = [-9.; 4];
        assert_eq!(view.read_logical_pixel_native_into(1, 0, &mut actual, &control()), Err(RasterViewError::Bounds));
        assert_eq!(actual, [77; 4]);
        assert_eq!(view.read_logical_pixel_f64_into(1, 0, &mut floats, &control()), Err(RasterViewError::Bounds));
        assert_eq!(floats, [-9.; 4]);
        view.read_logical_pixel_native_into(0, 0, &mut actual, &control()).unwrap();
        assert_eq!(actual, [1, 2, 3, 4]);
        view.read_logical_pixel_f64_into(0, 0, &mut floats, &control()).unwrap();
        assert_eq!(floats, [1. / 255., 2. / 255., 3. / 255., 4. / 255.]);
    }
    // Raw pins still expose the actual retained physical payload, no clearing.
    resident.pin_tile(TileCoord::new(0, 0), &control()).unwrap().read_pixel_native_into(1, 0, &mut actual, &control()).unwrap();
    assert_eq!(actual, [99, 98, 97, 96]);
    let mut native = [0; 4];
    surface.try_read_pixel_bytes(1, 0, &mut native).unwrap();
    assert_eq!(native, [99, 98, 97, 96]);
}
