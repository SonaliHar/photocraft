//! UNRUN synthetic CURRENT translation consumers. Native input bytes are original
//! harness literals, not XCF source qualification or a profile/operation grant.
use super::*;
use photocraft_cms::Builtin;
use photocraft_color::{PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::backed_layer::BackedLayerError;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
const CAP: u64 = 64 << 20;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    bytes: Vec<u8>,
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    reads: AtomicU64,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        let a = usize::try_from(offset).map_err(|_| BackingError::Length)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Length)?;
        let input = self.bytes.get(a..b).ok_or(BackingError::Bounds)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        out.copy_from_slice(input);
        Ok(())
    }
}
fn pixel(format: PixelFormat, value: f64, alpha: f64) -> Vec<u8> {
    let mut bytes = vec![0; format.bytes_per_pixel()];
    for channel in 0..format.channels() {
        try_write_sample_f64(&mut bytes, format.sample, channel, if channel == format.mode.color_channels() { alpha } else { value }).unwrap();
    }
    bytes
}
fn fixture(format: PixelFormat) -> (BackedLayer, Arc<Store>, Vec<u8>, BackingQuota) {
    let bounds = Rect::new(-259, -2, 4, 2);
    let len = bounds.width() as usize * bounds.height() as usize * format.bytes_per_pixel();
    let quota = BackingQuota::new(QuotaLimits { resident: CAP, index: 16 << 20, scratch: 16 << 20 });
    let header = std::mem::size_of::<Store>() as u64 + 128;
    let mut fee = quota.reserve(QuotaKind::Resident, len as u64 + header).unwrap();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).unwrap();
    fee.reconcile_capacity(bytes.capacity() as u64 + header).unwrap();
    for i in 0..bounds.width() as usize * bounds.height() as usize {
        let mut p = pixel(format, [0.0, 0.25, 0.5, 0.75][i % 4], [0., 0.25, 0.5, 1.][i % 4]);
        if i == 2 && format.sample == SampleType::U32 {
            p[..4].copy_from_slice(&16_777_217u32.to_ne_bytes());
        }
        if i == 3 && format.sample == SampleType::F32 {
            p[..4].copy_from_slice(&(-0.0f32).to_ne_bytes());
        }
        if i == 3 && format.sample == SampleType::F64 {
            p[..8].copy_from_slice(&(-0.0f64).to_ne_bytes());
        }
        if i == 4 && format.sample == SampleType::F64 {
            p[..8].copy_from_slice(&0.5000000000000001f64.to_ne_bytes());
        }
        bytes.extend_from_slice(&p);
    }
    // Independent expected capture is external test harness storage.
    let expected = bytes.clone();
    let default = pixel(format, 0.125, 0.5);
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let store = Arc::new(Store { bytes, descriptor, quota: quota.clone(), reads: AtomicU64::new(0), _fee: fee });
    let raster = BackedRaster::new(store.clone(), quota.clone(), 2, &ctl()).unwrap();
    (BackedLayer::new(raster, descriptor, &ctl()).unwrap(), store, expected, quota)
}
fn exact_pixel(layer: &BackedLayer, x: i32, y: i32) -> Vec<u8> {
    let mut bytes = vec![0; layer.format().bytes_per_pixel()];
    layer.checked_view().read_logical_pixel_native_into(x, y, &mut bytes, &ctl()).unwrap();
    bytes
}
fn expected_pixel(raw: &[u8], format: PixelFormat, bounds: Rect, x: i32, y: i32) -> &[u8] {
    let at = ((y - bounds.y0) as usize * bounds.width() as usize + (x - bounds.x0) as usize) * format.bytes_per_pixel();
    &raw[at..at + format.bytes_per_pixel()]
}
fn rgba(bytes: &[u8], format: PixelFormat) -> [f64; 4] {
    let mut p = [0.; 4];
    for (i, v) in p.iter_mut().enumerate().take(format.channels()) {
        *v = try_read_sample_f64(bytes, format.sample, i).unwrap();
    }
    if format.mode == ColorMode::Grayscale { [p[0], p[0], p[0], p[1]] } else { p }
}
#[test]
#[ignore = "UNRUN:all five depths RGB/Gray; negative/unaligned translated native pins/defaults and contained ROI Normal versus independently shifted raw samples"]
fn translated_native_bands_match_raw_harness_and_existing_normal_kernel() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in SampleType::ALL {
            let format = PixelFormat::new(mode, depth, true);
            let (source, store, raw, quota) = fixture(format);
            let original = source.descriptor();
            let current = photocraft_cms::xcf_color::effective_profile_from_definition(
                if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() },
                encoding(SampleEncoding::NonlinearProfile),
            )
            .unwrap();
            let working = WorkingSpace::new(&current, SampleEncoding::NonlinearProfile, space(SampleEncoding::NonlinearProfile)).unwrap();
            for (dx, dy) in [(258, 255), (-3, -255)] {
                let shifted = source.try_translated(dx, dy, &ctl()).unwrap();
                assert!(shifted.quota().shares_ledger(&quota));
                assert_eq!(shifted.format(), format);
                assert_eq!(shifted.default_bytes(), source.default_bytes());
                let bounds = shifted.operative_bounds();
                assert_eq!(bounds, Rect::new(original.bounds().x0 + dx, original.bounds().y0 + dy, original.bounds().x1 + dx, original.bounds().y1 + dy));
                for coord in bounds.tiles() {
                    let pin = shifted.checked_view().pin_tile(coord, &ctl()).unwrap();
                    let part = coord.rect().intersect(&bounds);
                    let mut actual = vec![0; format.bytes_per_pixel()];
                    for y in part.y0..part.y1 {
                        for x in part.x0..part.x1 {
                            pin.read_pixel_native_into(x, y, &mut actual, &ctl()).unwrap();
                            assert_eq!(actual, expected_pixel(&raw, format, original.bounds(), x - dx, y - dy));
                        }
                    }
                }
                let outside = (bounds.x0 - 1, bounds.y0 - 1);
                let mut p = vec![0; format.bytes_per_pixel()];
                assert!(shifted.checked_view().read_logical_pixel_native_into(outside.0, outside.1, &mut p, &ctl()).is_err());
                let pin = shifted.checked_view().pin_tile(photocraft_geom::TileCoord::containing(outside.0, outside.1), &ctl()).unwrap();
                pin.read_pixel_native_into(outside.0, outside.1, &mut p, &ctl()).unwrap();
                assert_eq!(p, shifted.default_bytes());
                match pin.encoded() {
                    NativeTileBytes::FullTile(bytes) => assert_eq!(bytes.len(), 256 * 256 * format.bytes_per_pixel()),
                    NativeTileBytes::DefaultPixel(bytes) => assert_eq!(bytes, shifted.default_bytes()),
                };
                drop(pin);
                let roi = Rect::new(bounds.x0 + 1, bounds.y0 + 1, bounds.x1 - 1, bounds.y1 - 1);
                let prepared =
                    PreparedBackedNormal::new(&shifted, &current, SampleEncoding::NonlinearProfile, shifted.descriptor(), NormalBandLimits::default(), &ctl())
                        .unwrap();
                let backdrop = if mode == ColorMode::Grayscale { [0.125, 0.125, 0.125, 0.5] } else { [0.125, 0.25, 0.375, 0.5] };
                let band = prepared.render_band_with_coverage(roi, backdrop, 0.625, None, &ctl()).unwrap();
                assert_eq!(band.rect(), roi);
                let expected: Vec<_> = (roi.y0..roi.y1)
                    .flat_map(|y| (roi.x0..roi.x1).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        working
                            .normal(backdrop, rgba(expected_pixel(&raw, format, original.bounds(), x - dx, y - dy), format), 0.625, GimpOperator::Union)
                            .unwrap()
                    })
                    .collect();
                for (a, b) in band.pixels().iter().zip(&expected) {
                    assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
                }
                assert_eq!(store.bytes, raw);
                assert_eq!(source.descriptor(), original);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN:native COW before/after unaligned translation preserves original and sibling words/defaults while existing Normal reads edited destination"]
fn translated_cow_remains_private_and_provider_owner_outlives_original() {
    for depth in SampleType::ALL {
        let format = PixelFormat::new(ColorMode::Rgb, depth, true);
        let (source, store, raw, quota) = fixture(format);
        let a = Rect::new(-257, -1, -256, 0);
        let replacement = pixel(format, 0.875, 0.5);
        let edited = source.try_edited_native(a, &replacement, &ctl()).unwrap();
        let shifted = edited.try_translated(258, 255, &ctl()).unwrap();
        let sibling = shifted.clone();
        assert_eq!(exact_pixel(&shifted, 1, 254), replacement);
        assert_eq!(exact_pixel(&source, -257, -1), expected_pixel(&raw, format, source.operative_bounds(), -257, -1));
        let second = pixel(format, 0.0625, 1.);
        let changed = shifted.try_edited_native(Rect::new(1, 254, 2, 255), &second, &ctl()).unwrap();
        assert_eq!(exact_pixel(&changed, 1, 254), second);
        assert_eq!(exact_pixel(&sibling, 1, 254), replacement);
        assert_eq!(exact_pixel(&edited, -257, -1), replacement);
        assert_eq!(store.bytes, raw);
        drop(source);
        drop(edited);
        drop(shifted);
        drop(store);
        assert_eq!(exact_pixel(&changed, 1, 254), second);
        assert_eq!(exact_pixel(&sibling, 1, 254), replacement);
        let current = Builtin::Srgb.profile();
        let prepared =
            PreparedBackedNormal::new(&changed, current, SampleEncoding::NonlinearProfile, changed.descriptor(), NormalBandLimits::default(), &ctl()).unwrap();
        let band = prepared.render_band(Rect::new(1, 254, 2, 255), [0.; 4], &ctl()).unwrap();
        let expected = WorkingSpace::new(current, SampleEncoding::NonlinearProfile, space(SampleEncoding::NonlinearProfile))
            .unwrap()
            .normal([0.; 4], rgba(&second, format), 1., GimpOperator::Union)
            .unwrap();
        assert_eq!(band.pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
        assert!(quota.usage().unwrap().resident >= band.resident_bytes());
        drop(band);
        drop(prepared);
        drop(changed);
        drop(sibling);
        assert_eq!(quota.usage().unwrap(), Default::default());
    }
}
#[test]
#[ignore = "UNRUN:controlled translation/Normal consumer refusals preserve exact source and held ledgers; complete output fee retained until output drop"]
fn translated_consumer_control_quota_and_failure_boundaries() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let (source, store, raw, quota) = fixture(format);
    let reads = store.reads.load(Ordering::Relaxed);
    assert!(source.try_translated(i32::MIN, 0, &ctl()).is_err());
    assert_eq!(store.reads.load(Ordering::Relaxed), reads);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(source.try_translated(1, 1, &denied).is_err());
    let cancelled = Interrupt::cancel_only(&|| true);
    let control = ReadControl { interrupt: &cancelled, work: &|_| Ok(()) };
    assert!(matches!(source.try_translated(1, 1, &control), Err(BackedLayerError::Backing(BackingError::Cancelled))));
    let shifted = source.try_translated(258, 255, &ctl()).unwrap();
    let prepared = PreparedBackedNormal::new(
        &shifted,
        Builtin::Srgb.profile(),
        SampleEncoding::NonlinearProfile,
        shifted.descriptor(),
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap();
    let roi = Rect::new(0, 254, 3, 256);
    assert!(prepared.render_band(roi, [0.; 4], &denied).is_err());
    assert!(prepared.render_band(roi, [0.; 4], &control).is_err());
    let fill = quota.reserve(QuotaKind::Resident, CAP - quota.usage().unwrap().resident).unwrap();
    assert!(prepared.render_band(roi, [0.; 4], &ctl()).is_err());
    drop(fill);
    // Warm controlled provider caches before measuring only returned band custody.
    drop(prepared.render_band(roi, [0.; 4], &ctl()).unwrap());
    let baseline = quota.usage().unwrap();
    let band = prepared.render_band(roi, [0.; 4], &ctl()).unwrap();
    assert!(quota.usage().unwrap().resident >= baseline.resident + band.resident_bytes());
    drop(band);
    assert_eq!(quota.usage().unwrap(), baseline);
    assert_eq!(store.bytes, raw);
}
