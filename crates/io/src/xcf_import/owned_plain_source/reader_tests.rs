//! UNRUN reader capture and actual same-quota XCF handoff regression drafts.
use super::*;
use crate::xcf_fixture;
use photocraft_doc::LayerContent;
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaLimits, QuotaUsage},
};
use std::{
    io::{self, Cursor},
    sync::atomic::{AtomicU64, Ordering},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn limits() -> ResourceLimits {
    ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("scratchBytes", 0), ("residentBytes", 32 << 20), ("indexBytes", 8 << 20)]).unwrap()
}
fn quota(resident: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident, scratch: 0, index: resident.min(64 << 20) })
}
struct NeverRead(usize);
impl Read for NeverRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        self.0 += 1;
        Err(io::Error::other("must not read"))
    }
}
struct Partial {
    source: Cursor<Vec<u8>>,
    largest: usize,
    interrupted: bool,
}
impl Read for Partial {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.largest = self.largest.max(output.len());
        if self.interrupted {
            self.interrupted = false;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        self.interrupted = true;
        let count = output.len().min(997);
        self.source.read(&mut output[..count])
    }
}
#[test]
fn capture_partial_interrupted_exact_bytes_are_owned_and_charged_before_read() {
    let bytes: Vec<u8> = (0..(CHUNK * 2 + 17)).map(|n| (n % 251) as u8).collect();
    let q = quota(1 << 20);
    let mut source = Partial { source: Cursor::new(bytes.clone()), largest: 0, interrupted: true };
    let captured = Captured::read(&mut source, bytes.len() as u64, &DecodeOptions::default(), &q, 1 << 20, &control()).unwrap();
    assert_eq!(captured.bytes.as_slice(), bytes);
    assert!(source.largest <= CHUNK);
    assert!(q.usage().unwrap().resident >= captured.bytes.capacity() as u64);
    assert_eq!(q.usage().unwrap().scratch, 0);
    let weak = Arc::downgrade(&captured.bytes);
    drop(source);
    assert!(weak.upgrade().is_some());
    drop(captured);
    assert!(weak.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn invalid_limits_cancel_and_quota_denial_precede_capture_allocation_and_reads() {
    let q = quota(1 << 20);
    for l in [
        limits().lowered(&[("encodedBytes", 1)]).unwrap(),
        limits().lowered(&[("residentBytes", 0)]).unwrap(),
        ResourceLimits::for_policy(Default::default()).unwrap(),
    ] {
        let mut source = NeverRead(0);
        assert!(import_owned_staged_plain_from_reader_in_quota("x", &mut source, 2, &DecodeOptions::default(), &l, q.clone(), &control()).is_err());
        assert_eq!(source.0, 0);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let mut source = NeverRead(0);
    let no = || true;
    let options = DecodeOptions { cancelled: Some(&no), ..Default::default() };
    assert!(matches!(import_owned_staged_plain_from_reader_in_quota("x", &mut source, 2, &options, &limits(), q, &control()), Err(IoError::Cancelled)));
    assert_eq!(source.0, 0);
    assert!(import_owned_staged_plain_from_reader_in_quota("x", &mut source, 2, &DecodeOptions::default(), &limits(), quota(0), &control()).is_err());
    assert_eq!(source.0, 0);
}
#[test]
fn wrong_exact_length_fault_and_repeated_interruption_release_private_capture() {
    struct Interrupted;
    impl Read for Interrupted {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::Interrupted))
        }
    }
    let q = quota(1 << 20);
    for (source, length) in [(vec![1], 2), (vec![1, 2, 3], 2)] {
        assert!(Captured::read(&mut Cursor::new(source), length, &DecodeOptions::default(), &q, 1 << 20, &control()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    assert!(Captured::read(&mut Interrupted, 2, &DecodeOptions::default(), &q, 1 << 20, &control()).is_err());
    assert!(Captured::read(&mut NeverRead(0), 2, &DecodeOptions::default(), &q, 1 << 20, &control()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn supplied_read_control_charges_capture_and_retained_import_without_reset() {
    let raw = xcf_fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    let q = quota(512 << 20);
    let work = AtomicU64::new(0);
    let charge = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let owned = import_owned_staged_plain_from_reader_in_quota(
        "actual.xcf",
        &mut Cursor::new(&raw),
        raw.len() as u64,
        &DecodeOptions::default(),
        &limits(),
        q.clone(),
        &c,
    )
    .unwrap();
    assert!(owned.document().quota().shares_ledger(&q));
    assert!(work.load(Ordering::Relaxed) > 2 * raw.len() as u64);
    assert!(owned.source_read_only());
    let LayerContent::BackedRaster(layer) = &owned.document().document().layers[0].content else { panic!() };
    let mut pixel = [0; 4];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &c).unwrap();
    assert_eq!(pixel, [31, 61, 91, 255]);
    drop(owned);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn parent_work_denial_during_capture_refunds_storage_but_not_consumed_work() {
    let q = quota(1 << 20);
    let work = AtomicU64::new(0);
    let charge = |n| {
        let previous = work.fetch_add(n, Ordering::Relaxed);
        if previous.saturating_add(n) > CHUNK as u64 { Err(BackingError::Work) } else { Ok(()) }
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let mut source = Cursor::new(vec![0; CHUNK + 1]);
    assert!(Captured::read(&mut source, (CHUNK + 1) as u64, &DecodeOptions::default(), &q, 1 << 20, &c).is_err());
    assert!(work.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
