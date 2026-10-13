//! Real kernel-owned storage integration, independent of standard Document routing.
#![cfg(windows)]
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_write_sample_f64};
use photocraft_compose::backed_normal::{BackedNormalError, NormalBandLimits, NormalMask, PreparedBackedNormal};
use photocraft_doc::{BackedLayer, SampleEncoding};
use photocraft_format::{
    finite_raster::FiniteRasterLimits,
    finite_raster_archive::{FiniteArchiveError, FiniteArchiveLimits, load_finite_raster_archive, save_finite_raster_archive},
    zip::stream::{Limits, Reader},
};
use photocraft_geom::Rect;
use photocraft_io::{
    native_plane_store::{NativeStoreLimits, OwnedCanonicalArchive, provider::OwnedNativeStorage},
    xcf_source::ServiceAuthorizedScratchDirectory,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use photocraft_xcf::{Budget, ResourceLimits, ResourcePolicy};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

struct ScratchRoot(PathBuf);
impl ScratchRoot {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-root-finite-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().count()
    }
}
impl Drop for ScratchRoot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir(&self.0) {
            eprintln!("finite archive scratch cleanup: {error}");
        }
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 48 << 20, scratch: 32 << 20, index: 4 << 20 })
}
fn limits() -> FiniteArchiveLimits {
    FiniteArchiveLimits { raster: FiniteRasterLimits { max_logical_bytes: 1 << 30, max_tiles: 4096 }, zip: Limits::default(), max_metadata_bytes: 1 << 20 }
}
#[derive(Debug)]
struct Defaults(NativePlaneDescriptor);
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = *self.0.default_bytes().get(((offset + index as u64) % self.0.format().bytes_per_pixel() as u64) as usize).ok_or(BackingError::Bounds)?;
        }
        Ok(())
    }
}
fn pixel(format: PixelFormat, values: [f64; 4]) -> Vec<u8> {
    let mut bytes = vec![0; format.bytes_per_pixel()];
    for (channel, value) in values.iter().enumerate() {
        try_write_sample_f64(&mut bytes, format.sample, channel, *value).unwrap();
    }
    bytes
}
fn layer(sample: SampleType, q: &BackingQuota) -> (BackedLayer, Vec<u8>) {
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let default = pixel(format, [0.125, 0.25, 0.375, 0.5]);
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(-2, -1, 259, 3), &default).unwrap();
    let raster = BackedRaster::new(Arc::new(Defaults(descriptor)), q.clone(), 1, &control()).unwrap();
    (BackedLayer::new(raster, descriptor, &control()).unwrap(), default)
}
fn capture(root: &ScratchRoot, input: &[u8], q: &BackingQuota) -> Arc<OwnedCanonicalArchive<OwnedNativeStorage>> {
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let storage = service.create_owned_byte_storage(input.len() as u64, q, &control()).unwrap();
    Arc::new(
        OwnedCanonicalArchive::capture(storage, input.len() as u64, q.clone(), NativeStoreLimits::default(), &control(), |offset, out, _| {
            let start = offset as usize;
            out.copy_from_slice(&input[start..start + out.len()]);
            Ok(())
        })
        .unwrap(),
    )
}
fn native(layer: &BackedLayer, x: i32, y: i32) -> Vec<u8> {
    let mut bytes = vec![77; layer.format().bytes_per_pixel()];
    layer.checked_view().read_logical_pixel_native_into(x, y, &mut bytes, &control()).unwrap();
    bytes
}

fn reopen_owned(root: &ScratchRoot, layer: &BackedLayer, q: &BackingQuota) -> BackedLayer {
    let bytes = save_finite_raster_archive(layer, Cursor::new(Vec::new()), limits(), &control()).unwrap().into_inner();
    let archive = capture(root, &bytes, q);
    let store = Arc::new(load_finite_raster_archive(archive, q.clone(), limits(), &control()).unwrap());
    let descriptor = store.descriptor();
    BackedLayer::new(BackedRaster::new(store, q.clone(), 1, &control()).unwrap(), descriptor, &control()).unwrap()
}

#[test]
fn separate_owned_source_and_mask_keep_exact_history_and_bounded_coverage_after_import_owners_drop() {
    for sample in [SampleType::U32, SampleType::F64] {
        let root = ScratchRoot::new();
        let q = quota();
        let (initial_source, source_default) = layer(sample, &q);
        let source_edit = pixel(initial_source.format(), [0.25, 0.5, 0.75, 0.5]);
        let changed_source = initial_source.try_edited_native(Rect::new(-1, 0, 0, 1), &source_edit, &control()).unwrap();
        let source = reopen_owned(&root, &changed_source, &q);
        let mask_format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mask_default = 0.25_f64.to_ne_bytes();
        let mask_descriptor = NativePlaneDescriptor::new(mask_format, Rect::new(-1, 0, 257, 2), &mask_default).unwrap();
        let initial_mask =
            BackedLayer::new(BackedRaster::new(Arc::new(Defaults(mask_descriptor)), q.clone(), 1, &control()).unwrap(), mask_descriptor, &control()).unwrap();
        let mask_edit = 0.5000000000000001_f64.to_ne_bytes();
        let changed_mask = initial_mask.try_edited_native(Rect::new(256, 0, 257, 1), &mask_edit, &control()).unwrap();
        let mask = reopen_owned(&root, &changed_mask, &q);
        drop((initial_source, changed_source, initial_mask, changed_mask));
        assert_eq!(root.entries(), 2);
        assert_eq!(native(&source, -1, 0), source_edit);
        assert_eq!(native(&mask, 256, 0), mask_edit);
        let history_mask = mask.clone();
        let current_mask = mask.try_edited_native(Rect::new(-1, 0, 0, 1), &0.0_f64.to_ne_bytes(), &control()).unwrap();
        assert_eq!(native(&history_mask, -1, 0), mask_default);
        assert_eq!(native(&current_mask, -1, 0), 0.0_f64.to_ne_bytes());
        let prepared = PreparedBackedNormal::new(
            &source,
            photocraft_cms::Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            source.descriptor(),
            NormalBandLimits::default(),
            &control(),
        )
        .unwrap();
        let backdrop = [0.2, 0.4, 0.6, 1.0];
        let rect = Rect::new(-2, 0, 259, 1);
        let band = prepared
            .render_band_with_coverage(rect, backdrop, 0.75, Some(NormalMask { layer: &history_mask, enabled: true, outside_coverage: 0.125 }), &control())
            .unwrap();
        // Independent closed-form Normal over an opaque backdrop. The mask
        // extent differs from the source and crosses a tile boundary.
        for (index, x) in (rect.x0..rect.x1).enumerate() {
            let raw = if x == -1 { &source_edit } else { &source_default };
            let mask_value = if x == 256 {
                f64::from_ne_bytes(mask_edit)
            } else if (-1..257).contains(&x) {
                0.25
            } else {
                0.125
            };
            let alpha = photocraft_color::try_read_sample_f64(raw, sample, 3).unwrap() * 0.75 * mask_value;
            for (channel, bg) in backdrop.iter().take(3).enumerate() {
                let foreground = photocraft_color::try_read_sample_f64(raw, sample, channel).unwrap();
                let expected = bg * (1.0 - alpha) + foreground * alpha;
                assert!((band.pixels()[index][channel] - expected).abs() <= 1.0e-14, "{sample:?} x={x} channel={channel}");
            }
            assert_eq!(band.pixels()[index][3], 1.0);
        }
        let edited = prepared
            .render_band_with_coverage(
                Rect::new(-1, 0, 0, 1),
                backdrop,
                0.75,
                Some(NormalMask { layer: &current_mask, enabled: true, outside_coverage: 0.125 }),
                &control(),
            )
            .unwrap();
        assert_eq!(edited.pixels(), &[backdrop]);
        let spent = AtomicU64::new(0);
        let work = |bytes| {
            if spent.fetch_add(bytes, Ordering::Relaxed) > 2000 { Err(BackingError::Work) } else { Ok(()) }
        };
        let exhausted = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(matches!(
            prepared.render_band_with_coverage(
                rect,
                backdrop,
                0.75,
                Some(NormalMask { layer: &current_mask, enabled: true, outside_coverage: 0.125 }),
                &exhausted
            ),
            Err(BackedNormalError::Backing(BackingError::Work))
                | Err(BackedNormalError::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Work)))
                | Err(BackedNormalError::View(photocraft_raster::checked_view::RasterViewError::Control(BackingError::Work)))
        ));
        assert_eq!(native(&source, -1, 0), source_edit);
        assert_eq!(native(&history_mask, -1, 0), mask_default);
        assert_eq!(native(&current_mask, -1, 0), 0.0_f64.to_ne_bytes());
        drop((edited, band, prepared));
        drop((source, mask, history_mask, current_mask));
        assert_eq!(root.entries(), 0);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn finite_current_native_bits_history_and_render_survive_real_file_owner_lifetimes() {
    for sample in [SampleType::U32, SampleType::F64] {
        let root = ScratchRoot::new();
        let q = quota();
        let (original, default) = layer(sample, &q);
        let mut edit = pixel(original.format(), [-0.0, 0.5000000000000001, 0.75, 1.0]);
        if sample == SampleType::U32 {
            edit[..4].copy_from_slice(&16_777_217_u32.to_ne_bytes());
        }
        let current = original.try_edited_native(Rect::new(-1, 0, 0, 1), &edit, &control()).unwrap();
        let bytes = save_finite_raster_archive(&current, Cursor::new(Vec::new()), limits(), &control()).unwrap().into_inner();
        let zip = Reader::new(Cursor::new(&bytes), Limits::default(), &Interrupt::NONE).unwrap();
        assert!(zip.contains("finite-raster.json"));
        assert!(!zip.contains("manifest.json"));
        drop(zip);
        let archive = capture(&root, &bytes, &q);
        let store = Arc::new(load_finite_raster_archive(archive.clone(), q.clone(), limits(), &control()).unwrap());
        let descriptor = store.descriptor();
        assert_eq!(descriptor, original.descriptor());
        let reopened = BackedLayer::new(BackedRaster::new(store.clone(), q.clone(), 1, &control()).unwrap(), descriptor, &control()).unwrap();
        assert_eq!(native(&original, -1, 0), default);
        assert_eq!(native(&current, -1, 0), edit);
        drop((bytes, archive, store, current, original));
        assert_eq!(root.entries(), 1);
        assert_eq!(native(&reopened, -1, 0), edit);
        let mut sentinel = vec![0x79; reopened.format().bytes_per_pixel()];
        assert_eq!(
            reopened.checked_view().read_logical_pixel_native_into(259, 2, &mut sentinel, &control()),
            Err(photocraft_raster::checked_view::RasterViewError::Bounds)
        );
        assert!(sentinel.iter().all(|byte| *byte == 0x79));
        for (x, y) in [(-2, -1), (0, 0), (256, 2), (258, 2)] {
            assert_eq!(native(&reopened, x, y), default);
        }
        let history = reopened.clone();
        let second = pixel(reopened.format(), [0.25, 0.5, 0.75, 1.0]);
        let edited = reopened.try_edited_native(Rect::new(256, 2, 257, 3), &second, &control()).unwrap();
        let output = save_finite_raster_archive(&edited, Cursor::new(Vec::new()), limits(), &control()).unwrap().into_inner();
        let new_archive = capture(&root, &output, &q);
        let new_store = Arc::new(load_finite_raster_archive(new_archive.clone(), q.clone(), limits(), &control()).unwrap());
        let reedited = BackedLayer::new(BackedRaster::new(new_store.clone(), q.clone(), 1, &control()).unwrap(), descriptor, &control()).unwrap();
        drop((output, new_archive, new_store, edited, reopened));
        assert_eq!(root.entries(), 2);
        assert_eq!(native(&history, 256, 2), default);
        assert_eq!(native(&reedited, -1, 0), edit);
        assert_eq!(native(&reedited, 256, 2), second);
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let prepared =
            PreparedBackedNormal::new(&reedited, profile, SampleEncoding::NonlinearProfile, descriptor, NormalBandLimits::default(), &control()).unwrap();
        let band = prepared.render_band(Rect::new(-1, 0, 0, 1), [0.0; 4], &control()).unwrap();
        for (channel, actual) in band.pixels()[0].iter().enumerate() {
            let expected = photocraft_color::try_read_sample_f64(&edit, sample, channel).unwrap();
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
        let outside = prepared.render_band(Rect::new(-3, -2, -2, -1), [0.2, 0.3, 0.4, 1.0], &control()).unwrap();
        assert_eq!(outside.pixels(), &[[0.2, 0.3, 0.4, 1.0]]);
        drop((outside, band, prepared, history));
        assert_eq!(root.entries(), 1);
        drop(reedited);
        assert_eq!(root.entries(), 0);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn actual_archive_nonfinite_transport_and_failed_load_do_not_publish_blank_pixels_or_leak_handles() {
    let root = ScratchRoot::new();
    let q = quota();
    let (old, _) = layer(SampleType::F64, &q);
    let mut edit = pixel(old.format(), [0.25, 0.5, 0.75, 1.0]);
    edit[..8].copy_from_slice(&0x7ff8000000000042_u64.to_ne_bytes());
    let current = old.try_edited_native(Rect::new(-1, 0, 0, 1), &edit, &control()).unwrap();
    let mut encoded = save_finite_raster_archive(&current, Cursor::new(Vec::new()), limits(), &control()).unwrap().into_inner();
    let archive = capture(&root, &encoded, &q);
    let baseline = q.usage().unwrap();
    let exhausted = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(load_finite_raster_archive(archive.clone(), q.clone(), limits(), &exhausted), Err(FiniteArchiveError::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let store = Arc::new(load_finite_raster_archive(archive.clone(), q.clone(), limits(), &control()).unwrap());
    let loaded = BackedLayer::new(BackedRaster::new(store.clone(), q.clone(), 1, &control()).unwrap(), store.descriptor(), &control()).unwrap();
    assert_eq!(native(&loaded, -1, 0), edit);
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let prepared =
        PreparedBackedNormal::new(&loaded, profile, SampleEncoding::NonlinearProfile, loaded.descriptor(), NormalBandLimits::default(), &control()).unwrap();
    assert!(matches!(prepared.render_band(Rect::new(-1, 0, 0, 1), [0.0; 4], &control()), Err(BackedNormalError::Unsupported("finite unit colors/alpha only"))));
    drop(prepared);
    drop((loaded, store, archive, old, current));
    assert_eq!(root.entries(), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    // A valid block-authenticated OS capture is still not a valid canonical tile.
    // Alter raw tile payload; load must verify canonical hash/ZIP CRC before publication.
    encoded[100] ^= 0x40;
    let corrupt = capture(&root, &encoded, &q);
    let baseline = q.usage().unwrap();
    assert!(load_finite_raster_archive(corrupt.clone(), q.clone(), limits(), &control()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    drop(corrupt);
    assert_eq!(root.entries(), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
