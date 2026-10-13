use photocraft_format::zip::stream::{Limits, Reader, Writer};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn archive(names: &[String]) -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    for name in names {
        writer.add(name, 4, &mut Cursor::new(b"data"), &Interrupt::NONE).unwrap();
    }
    writer.finish(&Interrupt::NONE).unwrap().into_inner()
}
struct Counted {
    input: Cursor<Vec<u8>>,
    reads: Arc<AtomicU64>,
}
impl Read for Counted {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.input.read(output)
    }
}
impl Seek for Counted {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.input.seek(from)
    }
}
#[test]
fn zero_resident_quota_rejects_before_archive_read() {
    let bytes = archive(&["tile".into()]);
    let reads = Arc::new(AtomicU64::new(0));
    let q = BackingQuota::new(QuotaLimits::default());
    let work = |_| Ok(());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let result = Reader::new_accounted(Counted { input: Cursor::new(bytes), reads: reads.clone() }, Limits::default(), &ctl, &q);
    assert!(result.is_err(), "accounted reader must not bypass its persistent quota");
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
fn limit(bytes: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: bytes, index: 0, scratch: 0 })
}
#[test]
fn long_names_retained_index_and_exact_quota_boundary() {
    let names = vec![format!("z{}", "k".repeat(60000)), format!("a{}", "k".repeat(60000))];
    let bytes = archive(&names);
    let mut legacy = Reader::new(Cursor::new(bytes.clone()), Limits::default(), &Interrupt::NONE).unwrap();
    for name in &names {
        assert_eq!(legacy.read_by_name(name, 4, &Interrupt::NONE).unwrap(), b"data");
    }
    let q = limit(2 << 20);
    let observed = AtomicU64::new(0);
    let work = |_| {
        observed.fetch_max(q.usage().unwrap().resident, Ordering::SeqCst);
        Ok(())
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut reader = Reader::new_accounted(Cursor::new(bytes.clone()), Limits::default(), &ctl, &q).unwrap();
    let retained = q.usage().unwrap().resident;
    assert!(retained >= 120002);
    assert!(observed.load(Ordering::SeqCst) > retained + 120000, "directory, copied names and temporary records overlap");
    for name in &names {
        assert_eq!(reader.stored_size(name).unwrap(), 4);
        assert_eq!(reader.read_by_name(name, 4, &Interrupt::NONE).unwrap(), b"data");
    }
    assert_eq!(q.usage().unwrap().resident, retained);
    drop(reader);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let small = archive(&["a".into()]);
    let work = |_| Ok(());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let succeeds = |cap| {
        let quota = limit(cap);
        let result = Reader::new_accounted(Cursor::new(small.clone()), Limits::default(), &ctl, &quota);
        let ok = result.is_ok();
        drop(result);
        assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
        ok
    };
    let (mut low, mut high) = (0u64, 65536u64);
    assert!(succeeds(high));
    while low < high {
        let middle = (low + high) / 2;
        if succeeds(middle) { high = middle } else { low = middle + 1 }
    }
    assert!(succeeds(low));
    assert!(!succeeds(low - 1));
    assert!(succeeds(low + 1));
}
#[test]
fn work_cancel_shortio_duplicate_corrupt_and_drop_refund() {
    use std::sync::atomic::AtomicBool;
    let names = (0..64).rev().map(|i| format!("{i:03}-{}", "k".repeat(80))).collect::<Vec<_>>();
    let valid = archive(&names);
    for kind in 0..5 {
        let mut bytes = valid.clone();
        if kind == 3 {
            let centrals = bytes.windows(4).enumerate().filter_map(|(i, b)| (b == 0x0201_4b50u32.to_le_bytes()).then_some(i)).collect::<Vec<_>>();
            let name = bytes[centrals[0] + 46..centrals[0] + 46 + names[0].len()].to_vec();
            let second = centrals[1];
            let local = u64::from_le_bytes(bytes[second + 46 + names[1].len() + 20..second + 46 + names[1].len() + 28].try_into().unwrap()) as usize;
            bytes[second + 46..second + 46 + name.len()].copy_from_slice(&name);
            bytes[local + 30..local + 30 + name.len()].copy_from_slice(&name);
        }
        if kind == 4 {
            bytes.truncate(bytes.len() - 10);
        }
        struct Fault {
            inner: Cursor<Vec<u8>>,
            count: u64,
        }
        impl Read for Fault {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.count += 1;
                if self.count == 6 {
                    return Err(std::io::Error::other("injected late read"));
                }
                self.inner.read(out)
            }
        }
        impl Seek for Fault {
            fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
                self.inner.seek(from)
            }
        }
        let q = limit(2 << 20);
        let cancelled = AtomicBool::new(false);
        let total = AtomicU64::new(0);
        let check = || cancelled.load(Ordering::SeqCst);
        let interrupt = Interrupt::cancel_only(&check);
        let work = |n| {
            let units = total.fetch_add(n, Ordering::SeqCst) + n;
            if units > 25000 {
                if kind == 0 {
                    return Err(photocraft_raster::backing::BackingError::Work);
                }
                if kind == 1 {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        };
        let ctl = ReadControl { interrupt: &interrupt, work: &work };
        let result = if kind == 2 {
            Reader::new_accounted(Fault { inner: Cursor::new(bytes), count: 0 }, Limits::default(), &ctl, &q).map(|_| ())
        } else {
            Reader::new_accounted(Fault { inner: Cursor::new(bytes), count: u64::MAX / 2 }, Limits::default(), &ctl, &q).map(|_| ())
        };
        assert!(result.is_err(), "kind{kind}");
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn reentrant_control_reserves_same_ledger_and_nested_reader_without_lock() {
    use std::sync::atomic::AtomicBool;
    let bytes = archive(&["b".into(), "a".into()]);
    let q = limit(1 << 20);
    let called = AtomicBool::new(false);
    let work = |_| {
        assert!(q.usage().is_ok());
        let lease = q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, 17).unwrap();
        drop(lease);
        if !called.swap(true, Ordering::SeqCst) {
            let nested_work = |_| Ok(());
            let nested = ReadControl { interrupt: &Interrupt::NONE, work: &nested_work };
            let reader = Reader::new_accounted(Cursor::new(bytes.clone()), Limits::default(), &nested, &q).unwrap();
            assert!(reader.contains("a"));
            drop(reader);
        }
        Ok(())
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let reader = Reader::new_accounted(Cursor::new(bytes.clone()), Limits::default(), &ctl, &q).unwrap();
    assert!(called.load(Ordering::SeqCst));
    drop(reader);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn cancel_at_first_sort_step_discards_completed_names_and_index() {
    use std::sync::atomic::AtomicBool;
    let count = 48usize;
    let names = (0..count).rev().map(|i| format!("{i:03}")).collect::<Vec<_>>();
    let bytes = archive(&names);
    let reads = Arc::new(AtomicU64::new(0));
    let q = limit(1 << 20);
    let cancelled = AtomicBool::new(false);
    let reached = AtomicBool::new(false);
    let check = || cancelled.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&check);
    let expected_reads = (4 + 2 * count) as u64;
    let work = |units| {
        if units == 1 && reads.load(Ordering::SeqCst) == expected_reads {
            assert!(q.usage().unwrap().resident > 1000);
            reached.store(true, Ordering::SeqCst);
            cancelled.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let ctl = ReadControl { interrupt: &interrupt, work: &work };
    let result = Reader::new_accounted(Counted { input: Cursor::new(bytes), reads: reads.clone() }, Limits::default(), &ctl, &q);
    assert!(matches!(result, Err(photocraft_format::FormatError::Cancelled)));
    assert!(reached.load(Ordering::SeqCst), "cancelled during sort after all record reads");
    assert_eq!(reads.load(Ordering::SeqCst), expected_reads);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn accounted_and_legacy_reject_independently_forged_payload_overlap() {
    let mut bytes = archive(&["a".into(), "b".into()]);
    let central = bytes.windows(4).position(|b| b == 0x0201_4b50u32.to_le_bytes()).unwrap();
    let local_extra = 31;
    let central_extra = central + 47;
    for at in [local_extra + 4, local_extra + 12, central_extra + 4, central_extra + 12] {
        bytes[at..at + 8].copy_from_slice(&8u64.to_le_bytes());
    }
    assert!(matches!(Reader::new(Cursor::new(bytes.clone()), Limits::default(), &Interrupt::NONE), Err(photocraft_format::FormatError::Corrupt(_))));
    let q = limit(1 << 20);
    let work = |_| Ok(());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let error = Reader::new_accounted(Cursor::new(bytes), Limits::default(), &ctl, &q).err().unwrap();
    assert!(matches!(error, photocraft_format::FormatError::Corrupt(_)));
    assert!(error.to_string().contains("overlapping"));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
