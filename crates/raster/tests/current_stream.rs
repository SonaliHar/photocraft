use photocraft_color::{ColorMode, PixelFormat, SampleType, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
fn work(_: u64) -> Result<(), BackingError> {
    Ok(())
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &work }
}
fn quota(bytes: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: bytes, index: 1 << 20, scratch: 1 << 20 })
}
#[derive(Debug)]
struct Memory {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    _lease: QuotaLease,
    quota: BackingQuota,
    reads: AtomicU64,
    fail_after: AtomicU64,
    fault: AtomicU8,
    dropped: Arc<AtomicBool>,
}
impl Drop for Memory {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
impl NativePlaneStore for Memory {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        if offset >= self.fail_after.load(Ordering::SeqCst) {
            if let Some(first) = out.first_mut() {
                *first = 0xde;
            }
            return Err(if self.fault.load(Ordering::SeqCst) == 2 { BackingError::Integrity } else { BackingError::Io });
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
        Ok(())
    }
}
fn setup(format: PixelFormat, bounds: Rect, q: BackingQuota) -> (BackedRaster, Arc<Memory>) {
    let bpp = format.bytes_per_pixel();
    let mut default = vec![0; bpp];
    for channel in 0..format.channels() {
        try_write_sample_f64(&mut default, format.sample, channel, 0.125).unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let length = descriptor.logical_bytes() as usize;
    let mut lease = q.reserve(QuotaKind::Resident, length as u64 * 2 + 128).unwrap();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).unwrap();
    lease.reconcile_capacity(bytes.capacity() as u64 + 128).unwrap();
    for i in 0..length / bpp {
        let mut p = default.clone();
        for c in 0..format.channels() {
            try_write_sample_f64(&mut p, format.sample, c, (i % 4) as f64 * 0.25).unwrap();
        }
        if format.sample == SampleType::U32 {
            p[..4].copy_from_slice(&16_777_217_u32.to_ne_bytes());
        }
        if format.sample == SampleType::F64 {
            p[..8].copy_from_slice(&(-0.0_f64).to_ne_bytes());
        }
        bytes.extend(p);
    }
    let store = Arc::new(Memory {
        descriptor,
        bytes,
        _lease: lease,
        quota: q.clone(),
        reads: AtomicU64::new(0),
        fail_after: AtomicU64::new(u64::MAX),
        fault: AtomicU8::new(0),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let raster = BackedRaster::new(store.clone(), q, 2, &ctl()).unwrap();
    (raster, store)
}
#[test]
fn current_cow_native_bytes_must_not_use_base_source() {
    let q = quota(8 << 20);
    let (mut current, store) = setup(PixelFormat::RGBA64F, Rect::new(-1, -1, 3, 2), q.clone());
    let history = current.clone();
    let edit = [0x31; 32];
    current.write_region(Rect::new(-1, -1, 0, 0), &edit, &ctl()).unwrap();
    let mut output = [0xa5; 32];
    current.read_current_exact_at(0, &mut output, &ctl()).unwrap();
    assert_eq!(output, edit, "native persistence must read current COW, never immutable base source");
    drop((current, history, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn five_depth_current_ranges_partial_samples_rows_tiles_and_history_are_exact() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let format = PixelFormat::new(mode, sample, true);
            let bpp = format.bytes_per_pixel();
            let bounds = Rect::new(-3, -1, 265, 2);
            let q = quota(32 << 20);
            let (mut current, store) = setup(format, bounds, q.clone());
            let history = current.clone();
            let original = store.bytes.clone();
            let mut expected = original.clone();
            let patch_bounds = Rect::new(-1, 0, 1, 1);
            let mut patch = vec![0; bpp * 2];
            for p in patch.chunks_exact_mut(bpp) {
                for c in 0..format.channels() {
                    try_write_sample_f64(p, sample, c, if sample == SampleType::F64 { -0.0 } else { 0.5 }).unwrap();
                }
                if sample == SampleType::U32 {
                    p[..4].copy_from_slice(&16_777_219_u32.to_ne_bytes());
                }
            }
            current.write_region(patch_bounds, &patch, &ctl()).unwrap();
            let at = (bounds.width() as usize + 2) * bpp;
            expected[at..at + patch.len()].copy_from_slice(&patch);
            let row = bounds.width() as usize * bpp;
            let mut ranges =
                vec![(0, 0), (0, 1), (1, 17.min(expected.len() - 1)), (row - 1, 2), (255 * bpp + 1, bpp + 3), (expected.len() - 7, 7), (0, expected.len())];
            let mut seed = 7_u64;
            for _ in 0..32 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let start = seed as usize % expected.len();
                let n = (seed.rotate_left(23) as usize % 97).min(expected.len() - start);
                ranges.push((start, n));
            }
            for (offset, n) in ranges {
                let mut output = vec![0xa5; n];
                current.read_current_exact_at(offset as u64, &mut output, &ctl()).unwrap();
                assert_eq!(output, expected[offset..offset + n], "{mode:?}/{sample:?}/{offset}/{n}");
                let mut old = vec![0; n];
                history.read_current_exact_at(offset as u64, &mut old, &ctl()).unwrap();
                assert_eq!(old, original[offset..offset + n]);
            }
            assert_eq!(store.bytes, original);
            assert_eq!(current.descriptor().default_bytes(), store.descriptor.default_bytes());
            let dropped = store.dropped.clone();
            drop(store);
            assert!(!dropped.load(Ordering::SeqCst));
            drop(current);
            assert!(!dropped.load(Ordering::SeqCst));
            let mut old = vec![0; original.len()];
            history.read_current_exact_at(0, &mut old, &ctl()).unwrap();
            assert_eq!(old, original);
            drop(history);
            assert!(dropped.load(Ordering::SeqCst));
            assert_eq!(q.usage().unwrap().resident, 0);
        }
    }
}

#[test]
fn empty_eof_overflow_limit_and_staging_denial_do_not_read_or_change_output() {
    let q = quota(8 << 20);
    let (raster, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 4, 3), q.clone());
    let end = raster.descriptor().logical_bytes();
    let before = q.usage().unwrap();
    raster.read_current_exact_at(end, &mut [], &ctl()).unwrap();
    raster.read_current_exact_at(0, &mut [], &ctl()).unwrap();
    for offset in [end, end + 1, u64::MAX] {
        let mut out = [0xa5; 1];
        assert_eq!(raster.read_current_exact_at(offset, &mut out, &ctl()), Err(BackingError::Bounds));
        assert_eq!(out, [0xa5]);
    }
    assert_eq!(raster.read_current_exact_at(end + 1, &mut [], &ctl()), Err(BackingError::Bounds));
    let mut oversized = vec![0xa5; 65537];
    assert_eq!(raster.read_current_exact_at(0, &mut oversized, &ctl()), Err(BackingError::Length));
    assert!(oversized.iter().all(|b| *b == 0xa5));
    let blocker = q.reserve(QuotaKind::Resident, (8 << 20) - before.resident - 64).unwrap();
    let held = q.usage().unwrap();
    let mut out = [0xa5; 32];
    assert_eq!(raster.read_current_exact_at(0, &mut out, &ctl()), Err(BackingError::Quota));
    assert_eq!(out, [0xa5; 32]);
    assert_eq!(q.usage().unwrap(), held);
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    drop((raster, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn late_short_io_integrity_work_and_final_cancel_keep_caller_buffer_atomic() {
    let q = quota(8 << 20);
    let (raster, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 257, 1), q.clone());
    let before_state = raster.clone();
    let original = store.bytes.clone();
    raster.read_current_exact_at(0, &mut [0; 1], &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    store.fail_after.store(1024, Ordering::SeqCst);
    for (fault, error) in [(1, BackingError::Io), (2, BackingError::Integrity)] {
        store.fault.store(fault, Ordering::SeqCst);
        let mut output = [0xa5; 6];
        assert_eq!(raster.read_current_exact_at(1022, &mut output, &ctl()), Err(error));
        assert_eq!(output, [0xa5; 6]);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    store.fail_after.store(u64::MAX, Ordering::SeqCst);
    raster.read_current_exact_at(1024, &mut [0; 1], &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let count = AtomicU64::new(0);
    let meter = |_: u64| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    let mut success = [0; 6];
    raster.read_current_exact_at(1022, &mut success, &ReadControl { interrupt: &Interrupt::NONE, work: &meter }).unwrap();
    let last = count.load(Ordering::SeqCst);
    for stop in [last - 1, last] {
        count.store(0, Ordering::SeqCst);
        let deny = |_: u64| if count.fetch_add(1, Ordering::SeqCst) + 1 == stop { Err(BackingError::Work) } else { Ok(()) };
        let mut out = [0xa5; 6];
        assert_eq!(raster.read_current_exact_at(1022, &mut out, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(BackingError::Work));
        assert_eq!(out, [0xa5; 6]);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    count.store(0, Ordering::SeqCst);
    let cancel = AtomicBool::new(false);
    let signal = |_: u64| {
        if count.fetch_add(1, Ordering::SeqCst) + 1 == last {
            cancel.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let probe = || cancel.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&probe);
    let mut out = [0xa5; 6];
    assert_eq!(raster.read_current_exact_at(1022, &mut out, &ReadControl { interrupt: &interrupt, work: &signal }), Err(BackingError::Cancelled));
    assert_eq!(out, [0xa5; 6]);
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(raster.shares_exact_state(&before_state));
    assert_eq!(store.bytes, original);
    drop((raster, before_state, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn mixed_store_ledger_is_rejected_before_any_stream_cache_allocation() {
    let q = quota(8 << 20);
    let (raster, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 4, 3), q.clone());
    let unrelated = quota(8 << 20);
    let source_usage = q.usage().unwrap();
    assert!(matches!(BackedRaster::new(store.clone(), unrelated.clone(), 2, &ctl()), Err(BackingError::Quota)));
    assert_eq!(unrelated.usage().unwrap().resident, 0);
    assert_eq!(q.usage().unwrap(), source_usage);
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    let mut output = [0; 1];
    raster.read_current_exact_at(0, &mut output, &ctl()).unwrap();
    assert_eq!(output[0], store.bytes[0]);
    drop((raster, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[derive(Debug)]
struct Generated {
    descriptor: NativePlaneDescriptor,
    reads: AtomicU64,
    last: AtomicU64,
    bytes_read: AtomicU64,
}
fn pattern(offset: u64) -> u8 {
    offset.wrapping_mul(37).wrapping_add(19) as u8
}
impl NativePlaneStore for Generated {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        let end = offset.checked_add(out.len() as u64).ok_or(BackingError::Bounds)?;
        if end > self.descriptor.logical_bytes() {
            return Err(BackingError::Io);
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.last.fetch_max(offset, Ordering::SeqCst);
        self.bytes_read.fetch_add(out.len() as u64, Ordering::SeqCst);
        for (i, b) in out.iter_mut().enumerate() {
            *b = pattern(offset + i as u64);
        }
        Ok(())
    }
}
#[test]
fn u64_offsets_and_extreme_row_wrap_need_no_whole_plane_or_bounding_hole_scan() {
    // Mathematical immutable store only; no actual large-file acceptance claim.
    for bounds in [Rect::new(-17, -17, 65519, 65519), Rect::new(-1, -1, i32::MAX - 257, 1)] {
        let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA64F, bounds, &[0x80; 32]).unwrap();
        assert!(descriptor.logical_bytes() > u64::from(u32::MAX));
        let store = Arc::new(Generated { descriptor, reads: AtomicU64::new(0), last: AtomicU64::new(0), bytes_read: AtomicU64::new(0) });
        let q = quota(16 << 20);
        let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
        let row = u64::from(bounds.width()) * 32;
        for (offset, n) in [(u64::from(u32::MAX) + 13, 63), (row - 3, 7), (descriptor.logical_bytes() - 17, 17)] {
            let mut out = vec![0xa5; n];
            raster.read_current_exact_at(offset, &mut out, &ctl()).unwrap();
            assert_eq!(out, (0..n).map(|i| pattern(offset + i as u64)).collect::<Vec<_>>());
            raster.clear_cache().unwrap();
        }
        assert!(store.last.load(Ordering::SeqCst) > u64::from(u32::MAX));
        if bounds.height() == 2 {
            assert!(store.reads.load(Ordering::SeqCst) <= 12, "must not visit a billions-of-pixels bounding hole");
        }
        drop((raster, store));
        assert_eq!(q.usage().unwrap().resident, 0);
    }
}

#[test]
fn exact64k_requests_and_default_padding_are_not_source_normalized() {
    let bounds = Rect::new(-3, -1, 1021, 31);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, bounds, &[0x80, 0x00, 0x01, 0x7f]).unwrap();
    let q = quota(8 << 20);
    let store = Arc::new(Generated { descriptor, reads: AtomicU64::new(0), last: AtomicU64::new(0), bytes_read: AtomicU64::new(0) });
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
    let mut output = vec![0xa5; 65536];
    raster.read_current_exact_at(1, &mut output, &ctl()).unwrap();
    assert_eq!(output, (1..65537).map(pattern).collect::<Vec<_>>());
    // The first physical tile has descriptor-default pixels outside the logical
    // extent. Byte streaming serializes only descriptor bytes, never that padding.
    let mut outside = [0; 4];
    raster.read_pixel_into(-4, -1, &mut outside, &ctl()).unwrap();
    assert_eq!(outside, descriptor.default_bytes());
    assert_eq!(raster.descriptor(), descriptor);
    drop((raster, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
fn late_second_tile_quota_failure_discards_staged_first_tile_bytes() {
    let q = quota(8 << 20);
    let (raster, store) = setup(PixelFormat::RGBA8, Rect::new(0, 0, 257, 1), q.clone());
    raster.read_current_exact_at(0, &mut [0; 1], &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (8 << 20) - baseline.resident - 300_000).unwrap();
    let held = q.usage().unwrap();
    let reads = store.reads.load(Ordering::SeqCst);
    let mut output = [0xa5; 6];
    // Private byte staging fits; the first cached tile is copied, then the
    // second physical tile's prepaid allocation envelope must be denied.
    assert_eq!(raster.read_current_exact_at(1022, &mut output, &ctl()), Err(BackingError::Quota));
    assert_eq!(output, [0xa5; 6]);
    assert_eq!(q.usage().unwrap(), held);
    assert_eq!(store.reads.load(Ordering::SeqCst), reads);
    drop(blocker);
    assert_eq!(q.usage().unwrap(), baseline);
    drop((raster, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}

#[test]
#[ignore = "release-only generated24MP row-major current-native stream measurement"]
fn release_24mp_current_native_stream() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    for cache_tiles in [2, 24] {
        let bounds = Rect::new(0, 0, 6000, 4000);
        let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, bounds, &[0; 4]).unwrap();
        let q = quota(8 << 20);
        let store = Arc::new(Generated { descriptor, reads: AtomicU64::new(0), last: AtomicU64::new(0), bytes_read: AtomicU64::new(0) });
        let raster = BackedRaster::new(store.clone(), q.clone(), cache_tiles, &ctl()).unwrap();
        let mut output_lease = q.reserve(QuotaKind::Resident, 65536 * 2 + 128).unwrap();
        let mut output = Vec::new();
        output.try_reserve_exact(65536).unwrap();
        output_lease.reconcile_capacity(output.capacity() as u64 + 128).unwrap();
        output.resize(65536, 0);
        let start = std::time::Instant::now();
        let mut offset = 0_u64;
        let mut sampled = 0_u64;
        while offset < descriptor.logical_bytes() {
            let count = ((descriptor.logical_bytes() - offset).min(65536)) as usize;
            raster.read_current_exact_at(offset, &mut output[..count], &ctl()).unwrap();
            assert!(output[..count].iter().enumerate().all(|(i, b)| *b == pattern(offset + i as u64)));
            offset += count as u64;
            sampled = sampled.max(q.usage().unwrap().resident);
        }
        println!(
            "generated24MP exact current-native stream {}B /64KiB/cache{}: {:?}; store calls={}; bytes read={}; amplification={:.3}; sampled retained after reads={}B (not transient/RSS)",
            offset,
            cache_tiles,
            start.elapsed(),
            store.reads.load(Ordering::SeqCst),
            store.bytes_read.load(Ordering::SeqCst),
            store.bytes_read.load(Ordering::SeqCst) as f64 / offset as f64,
            sampled
        );
        drop(output);
        drop(output_lease);
        drop((raster, store));
        assert_eq!(q.usage().unwrap().resident, 0);
    }
    Ok(())
}
