use photocraft_format::zip::stream::{Limits, Reader, Writer};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::{
    io::{Cursor, Seek, SeekFrom, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
struct Counted {
    output: Cursor<Vec<u8>>,
    seeks: Arc<AtomicU64>,
    writes: Arc<AtomicU64>,
}
impl Write for Counted {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.output.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}
impl Seek for Counted {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.seeks.fetch_add(1, Ordering::SeqCst);
        self.output.seek(p)
    }
}
#[test]
fn zero_quota_denies_before_output_seek_write() {
    let q = BackingQuota::new(QuotaLimits::default());
    let seeks = Arc::new(AtomicU64::new(0));
    let writes = Arc::new(AtomicU64::new(0));
    let out = Counted { output: Cursor::new(Vec::new()), seeks: seeks.clone(), writes: writes.clone() };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    assert!(Writer::new_accounted(out, Limits::default(), &ctl, &q).is_err(), "missing writer resident admission");
    assert_eq!(seeks.load(Ordering::SeqCst), 0);
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota(bytes: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: bytes, index: 0, scratch: 0 })
}
fn encode(names: &[String], capacity: u64) -> (Result<Vec<u8>, photocraft_format::FormatError>, u64) {
    let q = quota(capacity);
    let peak = AtomicU64::new(0);
    let work = |_| {
        peak.fetch_max(q.usage().unwrap().resident, Ordering::SeqCst);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let result = (|| {
        let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &control, &q)?;
        for (i, name) in names.iter().enumerate() {
            writer.add_accounted(name, 3, &mut Cursor::new([i as u8, 9, 7]), &control)?;
        }
        Ok(writer.finish_accounted(&control)?.into_inner())
    })();
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    (result, peak.load(Ordering::SeqCst))
}
#[test]
fn longnames_byte_exact_growth_overlap_and_exact_cap() {
    let names = vec![format!("z{}", "k".repeat(60000)), format!("a{}", "m".repeat(60000)), "x".into(), "b".into(), "c".into()];
    let (encoded, peak) = encode(&names, 4 << 20);
    let encoded = encoded.unwrap();
    assert!(peak > 300000);
    let mut legacy = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    for (i, name) in names.iter().enumerate() {
        legacy.add(name, 3, &mut Cursor::new([i as u8, 9, 7]), &Interrupt::NONE).unwrap();
    }
    let expected = legacy.finish(&Interrupt::NONE).unwrap().into_inner();
    assert_eq!(encoded, expected);
    let mut reader = Reader::new(Cursor::new(encoded), Limits::default(), &Interrupt::NONE).unwrap();
    for (i, name) in names.iter().enumerate() {
        assert_eq!(reader.read_by_name(name, 3, &Interrupt::NONE).unwrap(), [i as u8, 9, 7]);
    }
    let mut low = 0;
    let mut high = peak * 2;
    while low < high {
        let mid = low + (high - low) / 2;
        if encode(&names, mid).0.is_ok() { high = mid } else { low = mid + 1 }
    }
    assert!(encode(&names, low).0.is_ok());
    assert!(encode(&names, low + 1).0.is_ok());
    assert!(encode(&names, low - 1).0.is_err());
    // Observe the old central allocation PLUS its complete replacement during copy.
    let q = quota(4 << 20);
    let armed = AtomicBool::new(false);
    let overlap = AtomicU64::new(0);
    let first = 74 + names[0].len();
    let work = |n| {
        if armed.load(Ordering::SeqCst) && n == first as u64 {
            overlap.store(q.usage().unwrap().resident, Ordering::SeqCst)
        }
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &control, &q).unwrap();
    writer.add_accounted(&names[0], 0, &mut Cursor::new([]), &control).unwrap();
    let retained = q.usage().unwrap().resident;
    armed.store(true, Ordering::SeqCst);
    writer.add_accounted(&names[1], 0, &mut Cursor::new([]), &control).unwrap();
    assert_eq!(overlap.load(Ordering::SeqCst), retained + (first * 2) as u64);
    writer.finish_accounted(&control).unwrap();
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn strict_mode_duplicate_work_and_cancellation_poison_until_drop() {
    let q = quota(1 << 20);
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl(), &q).unwrap();
    assert!(writer.add("bypass", 0, &mut Cursor::new([]), &Interrupt::NONE).is_err());
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl(), &q).unwrap();
    assert!(writer.finish(&Interrupt::NONE).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl(), &q).unwrap();
    writer.add_accounted("entry", 3, &mut Cursor::new(b"abc"), &ctl()).unwrap();
    let retained = q.usage().unwrap();
    assert!(writer.add_accounted("entry", 0, &mut Cursor::new([]), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), retained);
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl(), &q).unwrap();
    let fail = |_| Err(photocraft_raster::backing::BackingError::Work);
    assert!(writer.add_accounted("entry", 3, &mut Cursor::new(b"abc"), &ReadControl { interrupt: &Interrupt::NONE, work: &fail }).is_err());
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl(), &q).unwrap();
    let cancel = AtomicBool::new(false);
    let calls = AtomicU64::new(0);
    let work = |_| {
        if calls.fetch_add(1, Ordering::SeqCst) > 25 {
            cancel.store(true, Ordering::SeqCst)
        }
        Ok(())
    };
    let check = || cancel.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&check);
    assert!(matches!(
        writer.add_accounted("entry", 70000, &mut Cursor::new(vec![7; 70000]), &ReadControl { interrupt: &interrupt, work: &work }),
        Err(photocraft_format::FormatError::Cancelled)
    ));
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn reentrant_quota_callback_and_finish_denial_refund() {
    let q = quota(1 << 20);
    let work = |_| {
        let temp = q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, 1).unwrap();
        drop(temp);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &control, &q).unwrap();
    writer.add_accounted("entry", 4, &mut Cursor::new(b"data"), &control).unwrap();
    let current = q.usage().unwrap().resident;
    let held = q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, (1 << 20) - current).unwrap();
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap().resident, held.bytes());
    drop(held);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

struct Fragmented {
    input: Cursor<Vec<u8>>,
    reads: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    cancel_at: u64,
    interrupt_forever: bool,
}
impl std::io::Read for Fragmented {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if self.interrupt_forever {
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        if n == self.cancel_at {
            self.cancel.store(true, Ordering::SeqCst)
        }
        let count = b.len().min(7);
        std::io::Read::read(&mut self.input, &mut b[..count])
    }
}
struct FaultOutput {
    output: Cursor<Vec<u8>>,
    writes: Arc<AtomicU64>,
    fail_at: u64,
    flush_fail: bool,
    fragment: usize,
}
impl Write for FaultOutput {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.fail_at {
            return Err(std::io::Error::from(std::io::ErrorKind::StorageFull));
        }
        self.output.write(&b[..b.len().min(self.fragment)])
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.flush_fail { Err(std::io::Error::from(std::io::ErrorKind::StorageFull)) } else { Ok(()) }
    }
}
impl Seek for FaultOutput {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.output.seek(p)
    }
}
#[test]
fn fragmented_mid_payload_cancel_io_short_overlong_and_finish_failure() {
    let q = quota(1 << 20);
    let cancel = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicU64::new(0));
    let writes = Arc::new(AtomicU64::new(0));
    let out = FaultOutput { output: Cursor::new(Vec::new()), writes: writes.clone(), fail_at: 0, flush_fail: false, fragment: 11 };
    let mut writer = Writer::new_accounted(out, Limits::default(), &ctl(), &q).unwrap();
    let check = || cancel.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&check);
    let mut input = Fragmented { input: Cursor::new(vec![7; 70000]), reads: reads.clone(), cancel: cancel.clone(), cancel_at: 20, interrupt_forever: false };
    assert!(matches!(
        writer.add_accounted("entry", 70000, &mut input, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(photocraft_format::FormatError::Cancelled)
    ));
    assert_eq!(reads.load(Ordering::SeqCst), 20);
    assert!(writes.load(Ordering::SeqCst) > 0, "header had genuinely been streamed before input cancellation");
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    for (flush_fail, fail_at) in [(false, 3), (true, 0)] {
        let writes = Arc::new(AtomicU64::new(0));
        let out = FaultOutput { output: Cursor::new(Vec::new()), writes, fail_at, flush_fail, fragment: 11 };
        let mut writer = Writer::new_accounted(out, Limits::default(), &ctl(), &q).unwrap();
        let result = writer.add_accounted("entry", 400, &mut Cursor::new(vec![2; 400]), &ctl());
        if flush_fail {
            result.unwrap();
            assert!(writer.finish_accounted(&ctl()).is_err())
        } else {
            assert!(result.is_err());
            assert!(writer.finish_accounted(&ctl()).is_err())
        }
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    for bytes in [vec![7; 2], vec![7; 4]] {
        let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl(), &q).unwrap();
        assert!(writer.add_accounted("entry", 3, &mut Cursor::new(bytes), &ctl()).is_err());
        assert!(writer.finish_accounted(&ctl()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let reads = Arc::new(AtomicU64::new(0));
    let mut input = Fragmented { input: Cursor::new(vec![1]), reads: reads.clone(), cancel, cancel_at: 0, interrupt_forever: true };
    let mut writer = Writer::new_accounted(Cursor::new(Vec::new()), Limits::default(), &ctl(), &q).unwrap();
    assert!(writer.add_accounted("entry", 1, &mut input, &ctl()).is_err());
    assert_eq!(reads.load(Ordering::SeqCst), 32);
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn raw_canonical_objects_reader_crc_and_old_bytes_are_identical() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_format::canonical_load::verify_tile;
    let format = PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true);
    let pixel =
        [(-0.0_f64).to_le_bytes(), 0x7ff8000000000001_u64.to_le_bytes(), 0.25_f64.to_le_bytes(), 0.75_f64.to_le_bytes(), 1.0_f64.to_le_bytes()].concat();
    let raw = pixel.repeat(256 * 256);
    assert!(raw.len() > 2 << 20);
    let hash = *blake3::hash(&raw).as_bytes();
    let mut name = String::from("tiles/");
    for byte in hash {
        name.push_str(&format!("{byte:02x}"))
    }
    name.push_str(".zst");
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0xc0, 0x38];
    frame.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    let total = raw.len().div_ceil(128 << 10);
    for (i, part) in raw.chunks(128 << 10).enumerate() {
        frame.extend_from_slice(&(((part.len() as u32) << 3) | u32::from(i + 1 == total)).to_le_bytes()[..3]);
        frame.extend_from_slice(part)
    }
    let q = quota(1 << 20);
    let writes = Arc::new(AtomicU64::new(0));
    let out = FaultOutput { output: Cursor::new(Vec::new()), writes, fail_at: 0, flush_fail: false, fragment: 113 };
    let mut writer = Writer::new_accounted(out, Limits::default(), &ctl(), &q).unwrap();
    let mut input = Fragmented {
        input: Cursor::new(frame.clone()),
        reads: Arc::new(AtomicU64::new(0)),
        cancel: Arc::new(AtomicBool::new(false)),
        cancel_at: 0,
        interrupt_forever: false,
    };
    writer.add_accounted(&name, frame.len() as u64, &mut input, &ctl()).unwrap();
    let bytes = writer.finish_accounted(&ctl()).unwrap().output.into_inner();
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut old = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    old.add(&name, frame.len() as u64, &mut Cursor::new(frame.clone()), &Interrupt::NONE).unwrap();
    assert_eq!(bytes, old.finish(&Interrupt::NONE).unwrap().into_inner());
    let mut reader = Reader::new_accounted(Cursor::new(bytes), Limits::default(), &ctl(), &q).unwrap();
    let mut verified = Vec::new();
    let token = verify_tile(&mut reader, format, hash, &q, &ctl(), |offset, part| {
        assert_eq!(offset, verified.len() as u64);
        verified.extend_from_slice(part);
        Ok(())
    })
    .unwrap();
    assert_eq!(token.hash(), hash);
    assert_eq!(verified, raw);
    assert_eq!(reader.read_by_name(&name, frame.len(), &Interrupt::NONE).unwrap(), frame);
    drop(reader);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn longname_comparison_and_metadata_quota_deny_before_new_writes() {
    let q = quota(1 << 20);
    let name = format!("dir/{}", "x".repeat(60000));
    let writes = Arc::new(AtomicU64::new(0));
    let seeks = Arc::new(AtomicU64::new(0));
    let output = Counted { output: Cursor::new(Vec::new()), seeks, writes: writes.clone() };
    let mut writer = Writer::new_accounted(output, Limits::default(), &ctl(), &q).unwrap();
    writer.add_accounted(&name, 0, &mut Cursor::new([]), &ctl()).unwrap();
    let retained = q.usage().unwrap();
    let before = writes.load(Ordering::SeqCst);
    let compared = AtomicU64::new(0);
    let cancel = AtomicBool::new(false);
    let work = |n| {
        if n == 8192 && compared.fetch_add(1, Ordering::SeqCst) == 2 {
            cancel.store(true, Ordering::SeqCst)
        }
        Ok(())
    };
    let check = || cancel.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&check);
    let other = format!("dir/{}y", "x".repeat(60000));
    assert!(matches!(
        writer.add_accounted(&other, 0, &mut Cursor::new([]), &ReadControl { interrupt: &interrupt, work: &work }),
        Err(photocraft_format::FormatError::Cancelled)
    ));
    assert_eq!(writes.load(Ordering::SeqCst), before);
    assert_eq!(q.usage().unwrap(), retained);
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let q = quota(16000);
    let writes = Arc::new(AtomicU64::new(0));
    let output = Counted { output: Cursor::new(Vec::new()), seeks: Arc::new(AtomicU64::new(0)), writes: writes.clone() };
    let mut writer = Writer::new_accounted(output, Limits::default(), &ctl(), &q).unwrap();
    assert!(writer.add_accounted("small", 0, &mut Cursor::new([]), &ctl()).is_err());
    assert_eq!(writes.load(Ordering::SeqCst), 0, "copy buffer denial precedes even local header output");
    assert!(q.usage().unwrap().resident > 0, "still-live private metadata remains charged until consumed writer drop");
    assert!(writer.finish_accounted(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
struct Generated {
    remaining: u64,
}
impl std::io::Read for Generated {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = b.len().min(self.remaining as usize);
        b[..n].fill(0x17);
        self.remaining -= n as u64;
        Ok(n)
    }
}
#[derive(Default)]
struct CountOnly {
    position: u64,
    length: u64,
}
impl Write for CountOnly {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.position = self.position.checked_add(b.len() as u64).ok_or_else(|| std::io::Error::other("counter overflow"))?;
        self.length = self.length.max(self.position);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for CountOnly {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        let n = match p {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        self.position = u64::try_from(n).map_err(|_| std::io::Error::other("counter seek"))?;
        Ok(self.position)
    }
}
#[test]
#[ignore = "scoped generated24MP STORE measurement; run --release"]
fn generated_24mp_writer_measurement() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("requires --release".into());
    }
    let bytes = 6144_u64 * 4096 * 4;
    for accounted in [false, true] {
        let q = quota(1 << 20);
        let peak = AtomicU64::new(0);
        let work = |_| {
            peak.fetch_max(q.usage().map_err(|_| photocraft_raster::backing::BackingError::Quota)?.resident, Ordering::SeqCst);
            Ok(())
        };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        let start = std::time::Instant::now();
        let mut input = Generated { remaining: bytes };
        let output = if accounted {
            let mut writer = Writer::new_accounted(CountOnly::default(), Limits::default(), &control, &q).map_err(|e| e.to_string())?;
            writer.add_accounted("generated-rgba", bytes, &mut input, &control).map_err(|e| e.to_string())?;
            writer.finish_accounted(&control).map_err(|e| e.to_string())?
        } else {
            let mut writer = Writer::new(CountOnly::default(), Limits::default()).map_err(|e| e.to_string())?;
            writer.add("generated-rgba", bytes, &mut input, &Interrupt::NONE).map_err(|e| e.to_string())?;
            writer.finish(&Interrupt::NONE).map_err(|e| e.to_string())?
        };
        println!(
            "accounted={accounted} generated_bytes={bytes} zip_bytes={} elapsed_ms={} callback_observed_resident={} final_usage={:?}",
            output.length,
            start.elapsed().as_secs_f64() * 1000.0,
            peak.load(Ordering::SeqCst),
            q.usage().map_err(|e| e.to_string())?
        );
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    Ok(())
}
