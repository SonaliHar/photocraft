#![cfg(windows)]
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_io::{
    native_plane_store::{NativeStoreLimits, PersistentNativeStorage, VerifiedNativePlaneStore, provider::OwnedNativeStorage},
    xcf_source::ServiceAuthorizedScratchDirectory,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use photocraft_xcf::{Budget, BudgetKind, ResourceLimits, ResourcePolicy};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Debug, Default)]
struct Counters {
    calls: AtomicU64,
    bytes: AtomicU64,
    fail: AtomicU64,
    corrupt: AtomicBool,
}
impl Counters {
    fn reset(&self) {
        self.calls.store(0, Ordering::SeqCst);
        self.bytes.store(0, Ordering::SeqCst);
    }
}
#[derive(Debug)]
struct Counted {
    inner: OwnedNativeStorage,
    counts: Arc<Counters>,
}
impl PersistentNativeStorage for Counted {
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        self.inner.validate_quota(q)
    }
    fn retained_scratch_bytes(&self) -> u64 {
        self.inner.retained_scratch_bytes()
    }
    fn resident_envelope(&self) -> u64 {
        self.inner.resident_envelope()
    }
    fn resident_bytes(&self) -> u64 {
        self.inner.resident_bytes()
    }
    fn len(&mut self) -> Result<u64, BackingError> {
        self.inner.len()
    }
    fn read_exact_at(&mut self, o: u64, b: &mut [u8]) -> Result<(), BackingError> {
        let n = self.counts.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.counts.fail.load(Ordering::SeqCst) == n {
            if let Some(v) = b.first_mut() {
                *v ^= 1;
            }
            return Err(BackingError::Io);
        }
        self.inner.read_exact_at(o, b)?;
        self.counts.bytes.fetch_add(b.len() as u64, Ordering::SeqCst);
        if self.counts.corrupt.load(Ordering::SeqCst)
            && let Some(v) = b.first_mut()
        {
            *v ^= 1;
        }
        Ok(())
    }
    fn write_all_at(&mut self, o: u64, b: &[u8]) -> Result<(), BackingError> {
        self.inner.write_all_at(o, b)
    }
    fn seal(&mut self) -> Result<(), BackingError> {
        self.inner.seal()
    }
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-sol2-stream-provider-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().count()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir(&self.0) {
            eprintln!("isolated directory cleanup: {e}");
        }
    }
}
fn pattern(o: u64) -> u8 {
    o.wrapping_mul(37).wrapping_add(19) as u8
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn capture(root: &Root, d: NativePlaneDescriptor, q: &BackingQuota, c: Arc<Counters>) -> Arc<VerifiedNativePlaneStore<Counted>> {
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let original = root.0.join("original-native");
    let mut source = File::create(&original).unwrap();
    {
        let _buffer_lease = q.reserve(QuotaKind::Resident, 4096).unwrap();
        let mut block = [0; 4096];
        let mut offset = 0;
        while offset < d.logical_bytes() {
            let n = (d.logical_bytes() - offset).min(4096) as usize;
            for (i, v) in block[..n].iter_mut().enumerate() {
                *v = pattern(offset + i as u64);
            }
            source.write_all(&block[..n]).unwrap();
            offset += n as u64;
        }
    }
    drop(source);
    let mut source = File::open(&original).unwrap();
    let handle = service.create_owned_native_storage(d, q, &control()).unwrap();
    let store =
        VerifiedNativePlaneStore::capture(Counted { inner: handle, counts: c.clone() }, d, q.clone(), NativeStoreLimits::default(), &control(), |o, b, _| {
            source.seek(SeekFrom::Start(o)).map_err(|_| BackingError::Io)?;
            source.read_exact(b).map_err(|_| BackingError::Io)
        })
        .unwrap();
    drop(source);
    std::fs::remove_file(original).unwrap();
    drop(service);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(root.entries(), 1);
    c.reset();
    Arc::new(store)
}
#[test]
fn actual_provider_current_history_and_atomic_failures_outlive_creation_authority() {
    let root = Root::new();
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(-3, -1, 510, 3), &[0; 4]).unwrap();
    let q = BackingQuota::new(QuotaLimits { resident: 8 << 20, scratch: d.logical_bytes(), index: 1 << 20 });
    let c = Arc::new(Counters::default());
    let store = capture(&root, d, &q, c.clone());
    let mut current = BackedRaster::new(store.clone(), q.clone(), 2, &control()).unwrap();
    let history = current.clone();
    current.write_region(Rect::new(-2, 0, -1, 1), &[9, 8, 7, 6], &control()).unwrap();
    let edit_offset = (u64::from(d.bounds().width()) + 1) * 4;
    let mut out = [0; 4];
    current.read_current_exact_at(edit_offset, &mut out, &control()).unwrap();
    assert_eq!(out, [9, 8, 7, 6]);
    history.read_current_exact_at(edit_offset, &mut out, &control()).unwrap();
    assert_eq!(out, std::array::from_fn(|i| pattern(edit_offset + i as u64)));
    drop(store);
    current.clear_cache().unwrap();
    history.clear_cache().unwrap();
    let baseline = q.usage().unwrap();
    c.fail.store(c.calls.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
    let mut out = [0xa5; 32];
    assert_eq!(history.read_current_exact_at(0, &mut out, &control()), Err(BackingError::Io));
    assert_eq!(out, [0xa5; 32]);
    history.clear_cache().unwrap();
    assert_eq!(q.usage().unwrap(), baseline);
    c.fail.store(0, Ordering::SeqCst);
    c.corrupt.store(true, Ordering::SeqCst);
    assert_eq!(history.read_current_exact_at(0, &mut out, &control()), Err(BackingError::Integrity));
    assert_eq!(out, [0xa5; 32]);
    history.clear_cache().unwrap();
    assert_eq!(q.usage().unwrap(), baseline);
    c.corrupt.store(false, Ordering::SeqCst);
    let denied = |_| Err(BackingError::Work);
    assert_eq!(history.read_current_exact_at(0, &mut out, &ReadControl { interrupt: &Interrupt::NONE, work: &denied }), Err(BackingError::Work));
    assert_eq!(out, [0xa5; 32]);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert_eq!(history.read_current_exact_at(0, &mut out, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(BackingError::Cancelled));
    assert_eq!(out, [0xa5; 32]);
    history.clear_cache().unwrap();
    assert_eq!(q.usage().unwrap(), baseline);
    let first_reads = c.calls.load(Ordering::SeqCst);
    let late_work = |_| if c.calls.load(Ordering::SeqCst) > first_reads + 1 { Err(BackingError::Work) } else { Ok(()) };
    assert_eq!(history.read_current_exact_at(0, &mut out, &ReadControl { interrupt: &Interrupt::NONE, work: &late_work }), Err(BackingError::Work));
    assert!(c.calls.load(Ordering::SeqCst) > first_reads + 1);
    assert_eq!(out, [0xa5; 32]);
    history.clear_cache().unwrap();
    assert_eq!(q.usage().unwrap(), baseline);
    let first_reads = c.calls.load(Ordering::SeqCst);
    let late_cancel = || c.calls.load(Ordering::SeqCst) > first_reads + 1;
    let interrupt = Interrupt::cancel_only(&late_cancel);
    assert_eq!(history.read_current_exact_at(0, &mut out, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(BackingError::Cancelled));
    assert!(c.calls.load(Ordering::SeqCst) > first_reads + 1);
    assert_eq!(out, [0xa5; 32]);
    history.clear_cache().unwrap();
    assert_eq!(q.usage().unwrap(), baseline);
    let blocker = q.reserve(QuotaKind::Resident, (8 << 20) - baseline.resident).unwrap();
    let calls = c.calls.load(Ordering::SeqCst);
    assert_eq!(history.read_current_exact_at(0, &mut out, &control()), Err(BackingError::Quota));
    assert_eq!(out, [0xa5; 32]);
    assert_eq!(c.calls.load(Ordering::SeqCst), calls);
    drop(blocker);
    history.clear_cache().unwrap();
    assert_eq!(q.usage().unwrap(), baseline);
    drop(current);
    assert_eq!(root.entries(), 1);
    drop(history);
    assert_eq!(root.entries(), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
#[ignore = "release-only sequential actualWindows24MP provider traversal measurement"]
fn release_actual24mp_provider_traversals() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let root = Root::new();
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 6000, 4000), &[0; 4]).unwrap();
    let q = BackingQuota::new(QuotaLimits { resident: 8 << 20, scratch: d.logical_bytes(), index: 2 << 20 });
    let c = Arc::new(Counters::default());
    let store = capture(&root, d, &q, c.clone());
    for (cache, tile_order) in [(2, false), (24, false), (2, true)] {
        c.reset();
        let raster = BackedRaster::new(store.clone(), q.clone(), cache, &control()).unwrap();
        let output_lease = q.reserve(QuotaKind::Resident, 65536).unwrap();
        let mut output = [0_u8; 65536];
        let start = std::time::Instant::now();
        let mut logical = 0;
        let mut sampled = 0;
        if tile_order {
            for coord in d.bounds().tiles() {
                let x = coord.tx * 256;
                let y = coord.ty * 256;
                let cell = Rect::new(x, y, x + 256, y + 256).intersect(&d.bounds());
                for row in cell.y0..cell.y1 {
                    let offset = (row as u64 * 6000 + cell.x0 as u64) * 4;
                    let n = cell.width() as usize * 4;
                    raster.read_current_exact_at(offset, &mut output[..n], &control()).unwrap();
                    assert!(output[..n].iter().enumerate().all(|(i, v)| *v == pattern(offset + i as u64)));
                    logical += n as u64;
                    sampled = sampled.max(q.usage().unwrap().resident);
                }
            }
        } else {
            while logical < d.logical_bytes() {
                let n = (d.logical_bytes() - logical).min(65536) as usize;
                raster.read_current_exact_at(logical, &mut output[..n], &control()).unwrap();
                assert!(output[..n].iter().enumerate().all(|(i, v)| *v == pattern(logical + i as u64)));
                logical += n as u64;
                sampled = sampled.max(q.usage().unwrap().resident);
            }
        }
        assert_eq!(logical, d.logical_bytes());
        println!(
            "actualWindows24MP cache{cache} tile_order={tile_order} exact{logical}B elapsed={:?} storage_reads={} storage_bytes={} amplification={:.3} retained_sampled={sampled}B (not RSS/transientpeak/syscallcount)",
            start.elapsed(),
            c.calls.load(Ordering::SeqCst),
            c.bytes.load(Ordering::SeqCst),
            c.bytes.load(Ordering::SeqCst) as f64 / logical as f64
        );
        drop(output_lease);
        drop(raster);
    }
    drop(store);
    assert_eq!(root.entries(), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    Ok(())
}
