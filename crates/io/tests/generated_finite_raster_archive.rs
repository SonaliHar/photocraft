#![cfg(windows)]
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::BackedLayer;
use photocraft_format::{
    finite_raster::FiniteRasterLimits,
    finite_raster_archive::{FiniteArchiveLimits, load_finite_raster_archive},
    zip::stream::Limits,
};
use photocraft_geom::Rect;
use photocraft_io::{
    native_plane_store::{NativeStoreLimits, generated::save_generated_finite_raster_archive},
    xcf_source::ServiceAuthorizedScratchDirectory,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
};
use photocraft_xcf::{Budget, ResourceLimits, ResourcePolicy};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-generated-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().count()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir(&self.0) {
            eprintln!("generated cleanup: {e}");
        }
    }
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 256 << 20, index: 4 << 20 })
}
fn limits() -> FiniteArchiveLimits {
    FiniteArchiveLimits { raster: FiniteRasterLimits { max_logical_bytes: 256 << 20, max_tiles: 4096 }, zip: Limits::default(), max_metadata_bytes: 1 << 20 }
}
#[derive(Debug)]
struct Defaults(NativePlaneDescriptor);
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let p = self.0.default_bytes();
        for (i, b) in out.iter_mut().enumerate() {
            *b = p[((offset + i as u64) % p.len() as u64) as usize];
        }
        Ok(())
    }
}
#[test]
fn direct_generated_os_archive_preserves_wide_current_history_and_owner_lifetime() {
    for sample in [SampleType::U32, SampleType::F64] {
        let root = Root::new();
        let quota = q();
        {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut default = vec![0; format.bytes_per_pixel()];
            for i in 0..4 {
                photocraft_color::try_write_sample_f64(&mut default, sample, i, 0.25).unwrap();
            }
            let descriptor = NativePlaneDescriptor::new(format, Rect::new(-2, -1, 259, 3), &default).unwrap();
            let source = BackedLayer::new(BackedRaster::new(Arc::new(Defaults(descriptor)), quota.clone(), 1, &ctl()).unwrap(), descriptor, &ctl()).unwrap();
            let history = source.clone();
            let mut edit = default.clone();
            if sample == SampleType::U32 {
                edit[..4].copy_from_slice(&16_777_217u32.to_ne_bytes());
            } else {
                edit[..8].copy_from_slice(&(-0.0f64).to_ne_bytes());
            }
            let current = source.try_edited_native(Rect::new(-1, 0, 0, 1), &edit, &ctl()).unwrap();
            let archive = {
                let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
                let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
                save_generated_finite_raster_archive(&service, &current, limits(), NativeStoreLimits::default(), 256 << 20, &ctl()).unwrap()
            };
            assert_eq!(root.entries(), 1);
            let store = Arc::new(load_finite_raster_archive(Arc::new(archive), quota.clone(), limits(), &ctl()).unwrap());
            let loaded = BackedLayer::new(BackedRaster::new(store, quota.clone(), 1, &ctl()).unwrap(), descriptor, &ctl()).unwrap();
            drop((source, current));
            let mut actual = vec![0; edit.len()];
            loaded.checked_view().read_logical_pixel_native_into(-1, 0, &mut actual, &ctl()).unwrap();
            assert_eq!(actual, edit);
            history.checked_view().read_logical_pixel_native_into(-1, 0, &mut actual, &ctl()).unwrap();
            assert_eq!(actual, default);
            for y in descriptor.bounds().y0..descriptor.bounds().y1 {
                for x in descriptor.bounds().x0..descriptor.bounds().x1 {
                    loaded.checked_view().read_logical_pixel_native_into(x, y, &mut actual, &ctl()).unwrap();
                    assert_eq!(actual.as_slice(), if x == -1 && y == 0 { edit.as_slice() } else { default.as_slice() });
                }
            }
            assert_eq!(loaded.default_bytes(), default);
            assert_eq!(loaded.operative_bounds(), descriptor.bounds());
            drop(history);
            assert_eq!(root.entries(), 1);
        }
        assert_eq!(root.entries(), 0);
        assert_eq!(quota.usage().unwrap().resident, 0);
        assert_eq!(quota.usage().unwrap().scratch, 0);
        assert_eq!(quota.usage().unwrap().index, 0);
    }
}

#[test]
fn generated_backpatch_is_exact_sealed_and_same_ledger() {
    use photocraft_format::canonical_archive::ImmutableCanonicalArchive;
    use std::io::{Seek, SeekFrom, Write};
    let root = Root::new();
    let quota = q();
    let control = ctl();
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let mut output = service.create_generated_archive_destination(16, &quota, &control).unwrap();
    output.write_all(b"abcdefgh").unwrap();
    assert_eq!(quota.usage().unwrap().scratch, 8);
    output.seek(SeekFrom::Start(2)).unwrap();
    output.write_all(b"XY").unwrap();
    assert_eq!(quota.usage().unwrap().scratch, 8);
    output.seek(SeekFrom::End(0)).unwrap();
    output.write_all(b"ij").unwrap();
    let archive = output.seal_verified(NativeStoreLimits::default(), &control).unwrap();
    let mut actual = [0; 10];
    archive.read_exact_at(0, &mut actual, &control).unwrap();
    assert_eq!(&actual, b"abXYefghij");
    assert_eq!(archive.validate_quota(&q()), Err(BackingError::Quota));
    // Byte authentication is deliberately not canonical archive admission.
    assert!(load_finite_raster_archive(Arc::new(archive), quota.clone(), limits(), &control).is_err());
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
    assert_eq!(quota.usage().unwrap().index, 0);
}

#[test]
fn rejected_growth_seek_and_control_poison_and_refund_generated_output() {
    use std::io::{Seek, SeekFrom, Write};
    use std::sync::atomic::AtomicBool;
    let root = Root::new();
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let control = ctl();
    // Exact Scratch quota: backpatch fits, extension is denied before file growth.
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 4, index: 1 << 20 });
    let mut output = service.create_generated_archive_destination(8, &quota, &control).unwrap();
    output.write_all(b"abcd").unwrap();
    assert_eq!(output.seek(SeekFrom::Start(0)).unwrap(), 0);
    output.write_all(b"AB").unwrap();
    output.seek(SeekFrom::End(0)).unwrap();
    assert!(output.write_all(b"e").is_err());
    assert_eq!(quota.usage().unwrap().scratch, 4);
    assert!(output.flush().is_err());
    assert!(matches!(output.seal_verified(NativeStoreLimits::default(), &control), Err(BackingError::Integrity)));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
    for from in [SeekFrom::Start(1), SeekFrom::Current(-1), SeekFrom::End(i64::MAX)] {
        let mut output = service.create_generated_archive_destination(8, &quota, &control).unwrap();
        assert!(output.seek(from).is_err());
        assert!(output.write_all(b"x").is_err());
        drop(output);
        assert_eq!(root.entries(), 0);
    }
    let denied = AtomicBool::new(false);
    let work = |_: u64| if denied.load(Ordering::Relaxed) { Err(BackingError::Work) } else { Ok(()) };
    let controlled = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut output = service.create_generated_archive_destination(8, &quota, &controlled).unwrap();
    output.write_all(b"abcd").unwrap();
    denied.store(true, Ordering::Relaxed);
    assert!(output.seek(SeekFrom::Start(0)).is_err());
    denied.store(false, Ordering::Relaxed);
    assert!(output.write_all(b"x").is_err());
    drop(output);
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
}

#[test]
fn creation_denial_and_authentication_failure_never_publish_or_retain_storage() {
    use std::io::Write;
    let root = Root::new();
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let control = ctl();
    let zero = BackingQuota::new(QuotaLimits { resident: 0, scratch: 1 << 20, index: 1 << 20 });
    assert!(matches!(service.create_generated_archive_destination(1 << 20, &zero, &control), Err(BackingError::Quota)));
    assert_eq!(root.entries(), 0);
    for size in [
        0,
        u64::from(photocraft_io::native_plane_store::NATIVE_STORE_MAX_BLOCKS) * photocraft_io::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64 + 1,
        u64::MAX,
    ] {
        assert!(matches!(service.create_generated_archive_destination(size, &q(), &control), Err(BackingError::Bounds)));
    }
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 1 << 20, index: 0 });
    let mut output = service.create_generated_archive_destination(4096, &quota, &control).unwrap();
    output.write_all(b"actual data").unwrap();
    assert!(matches!(output.seal_verified(NativeStoreLimits::default(), &control), Err(BackingError::Quota)));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
    let quota = q();
    let mut output = service.create_generated_archive_destination(4096, &quota, &control).unwrap();
    output.write_all(b"actual data").unwrap();
    let work = |_: u64| Err(BackingError::Work);
    let denial = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(matches!(output.seal_verified(NativeStoreLimits::default(), &denial), Err(BackingError::Work)));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
}

#[test]
fn actual_cancel_poison_and_seal_limits_refund_without_publication() {
    use std::io::Write;
    use std::sync::atomic::AtomicBool;
    let root = Root::new();
    let quota = q();
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let flag = AtomicBool::new(false);
    let cancel = || flag.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancel);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    let mut output = service.create_generated_archive_destination(8192, &quota, &control).unwrap();
    output.write_all(b"first bytes").unwrap();
    flag.store(true, Ordering::Relaxed);
    assert!(output.write_all(b"more").is_err());
    flag.store(false, Ordering::Relaxed);
    assert!(matches!(output.seal_verified(NativeStoreLimits::default(), &control), Err(BackingError::Integrity)));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
    let mut output = service.create_generated_archive_destination(8192, &quota, &control).unwrap();
    output.write_all(&[7; 4097]).unwrap();
    assert!(matches!(output.seal_verified(NativeStoreLimits { max_blocks: 1 }, &control), Err(BackingError::Bounds)));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
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
            *b = ((offset + i as u64).wrapping_mul(37).wrapping_add((offset + i as u64) / 101) % 256) as u8;
        }
        Ok(())
    }
}
#[test]
#[ignore = "generated24MP direct OS archive release measurement"]
fn generated_24mp_direct_stream_measurement() {
    let root = Root::new();
    let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 256 << 20, index: 8 << 20 });
    let control = ctl();
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(-3, -2, 5997, 3998), &[0, 0, 0, 0]).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(Pattern(d)), quota.clone(), 2, &control).unwrap(), d, &control).unwrap();
    let start = std::time::Instant::now();
    let archive = {
        let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
        let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
        save_generated_finite_raster_archive(&service, &layer, limits(), NativeStoreLimits::default(), 256 << 20, &control).unwrap()
    };
    let elapsed = start.elapsed();
    use photocraft_format::canonical_archive::ImmutableCanonicalArchive;
    let archive_bytes = archive.len();
    let usage = quota.usage().unwrap();
    let loaded = load_finite_raster_archive(Arc::new(archive), quota.clone(), limits(), &control).unwrap();
    let backed = BackedRaster::new(Arc::new(loaded), quota.clone(), 2, &control).unwrap();
    for (x, y) in [(-3, -2), (255, 255), (5996, 3997)] {
        let mut expected = [0; 4];
        let mut actual = [0; 4];
        layer.checked_view().read_logical_pixel_native_into(x, y, &mut expected, &control).unwrap();
        photocraft_raster::checked_view::CheckedRasterView::backed(&backed).read_logical_pixel_native_into(x, y, &mut actual, &control).unwrap();
        assert_eq!(actual, expected);
    }
    eprintln!(
        "generated24MP direct save/auth={elapsed:?} archive={archive_bytes} retained_resident={} scratch={} index={}",
        usage.resident, usage.scratch, usage.index
    );
    drop((layer, backed));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
    assert_eq!(quota.usage().unwrap().index, 0);
}

#[derive(Debug)]
struct FailingSource(NativePlaneDescriptor);
impl NativePlaneStore for FailingSource {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        Err(BackingError::Io)
    }
}
#[test]
fn failed_finite_producer_and_reentrant_write_denial_delete_private_destination() {
    let root = Root::new();
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    for source_failure in [true, false] {
        let quota = q();
        let control = ctl();
        let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(-1, -1, 2, 2), &[0; 4]).unwrap();
        let store: Arc<dyn NativePlaneStore> = if source_failure { Arc::new(FailingSource(d)) } else { Arc::new(Defaults(d)) };
        let layer = BackedLayer::new(BackedRaster::new(store, quota.clone(), 1, &control).unwrap(), d, &control).unwrap();
        // Callback inspects the actual ledger without deadlock, and denies only
        // after generated OS output extension has already been prepaid.
        let work = |_: u64| if !source_failure && quota.usage().unwrap().scratch > 0 { Err(BackingError::Work) } else { Ok(()) };
        let controlled = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(save_generated_finite_raster_archive(&service, &layer, limits(), NativeStoreLimits::default(), 256 << 20, &controlled).is_err());
        assert_eq!(root.entries(), 0);
        assert_eq!(quota.usage().unwrap().scratch, 0);
        drop(layer);
        assert_eq!(quota.usage().unwrap().resident, 0);
        assert_eq!(quota.usage().unwrap().index, 0);
    }
}
