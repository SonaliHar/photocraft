//! UNRUN held-reader capture, real native wire and ownership regression drafts.
use super::*;
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType};
use photocraft_geom::Size;
use photocraft_raster::{
    Interrupt, Surface,
    backing::quota::{QuotaLimits, QuotaUsage},
};
use std::{
    io::{self, Cursor},
    sync::atomic::{AtomicU64, Ordering},
};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota(resident: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident, scratch: 0, index: resident.min(8 << 20) })
}

struct Partial {
    source: Cursor<Vec<u8>>,
    calls: usize,
    largest_request: usize,
    interrupt_next: bool,
}
impl Read for Partial {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        self.largest_request = self.largest_request.max(output.len());
        if self.interrupt_next {
            self.interrupt_next = false;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        self.interrupt_next = true;
        let count = output.len().min(997);
        self.source.read(&mut output[..count])
    }
}

#[test]
fn partial_interrupted_capture_outlives_reader_and_retains_exact_same_ledger_fee() {
    let source: Vec<u8> = (0..(CHUNK * 2 + 13)).map(|n| (n % 251) as u8).collect();
    let q = quota(1 << 20);
    let mut reader = Partial { source: Cursor::new(source.clone()), calls: 0, largest_request: 0, interrupt_next: true };
    let archive = MemoryArchive::capture(&mut reader, source.len() as u64, q.clone(), &control()).unwrap();
    assert!(reader.calls > 3);
    assert!(reader.largest_request <= CHUNK);
    drop(reader);
    assert_eq!(archive.bytes, source);
    assert_eq!(q.usage().unwrap().scratch, 0);
    assert!(q.usage().unwrap().resident >= archive.bytes.capacity() as u64);
    archive.validate_quota(&q).unwrap();
    assert_eq!(archive.validate_quota(&quota(1 << 20)), Err(BackingError::Quota));
    let shared: Arc<dyn ImmutableCanonicalArchive> = Arc::new(archive);
    let held = shared.clone();
    drop(shared);
    let mut tail = vec![0; 37];
    held.read_exact_at((source.len() - tail.len()) as u64, &mut tail, &control()).unwrap();
    assert_eq!(tail, source[source.len() - 37..]);
    let mut too_large = vec![0; CHUNK + 1];
    assert_eq!(held.read_exact_at(0, &mut too_large, &control()), Err(BackingError::Bounds));
    drop(held);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

struct NeverRead(usize);
impl Read for NeverRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        self.0 += 1;
        Err(io::Error::other("must not read"))
    }
}
#[test]
fn invalid_codec_profile_refuses_before_capture_allocation_or_reader_access() {
    let q = quota(1 << 20);
    let mut reader = NeverRead(0);
    let limits = OwnedDocumentLimits { max_layers: OwnedDocumentLimits::default().max_layers + 1, ..Default::default() };
    assert!(matches!(open_accounted_native_from_reader(&mut reader, 1234, limits, q.clone(), &control()), Err(AccountedNativeError::Format(_))));
    assert_eq!(reader.0, 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn oversized_zero_and_quota_denial_happen_before_reader_access_and_refund() {
    for length in [0, MAX_CAPTURE_BYTES + 1, u64::MAX] {
        let q = quota(1024);
        let mut reader = NeverRead(0);
        assert!(matches!(MemoryArchive::capture(&mut reader, length, q.clone(), &control()), Err(AccountedNativeError::Backing(BackingError::Bounds))));
        assert_eq!(reader.0, 0);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let q = quota(16);
    let mut reader = NeverRead(0);
    assert!(matches!(MemoryArchive::capture(&mut reader, 1000, q.clone(), &control()), Err(AccountedNativeError::Backing(BackingError::Quota))));
    assert_eq!(reader.0, 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

struct InterruptedForever(usize);
impl Read for InterruptedForever {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        self.0 += 1;
        Err(io::Error::from(io::ErrorKind::Interrupted))
    }
}
struct Overcount;
impl Read for Overcount {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        Ok(out.len() + 1)
    }
}
#[test]
fn short_trailing_overcount_and_unending_interruption_publish_nothing() {
    let q = quota(1 << 20);
    for (bytes, length) in [(vec![1, 2], 3), (vec![1, 2, 3], 2)] {
        assert!(matches!(
            MemoryArchive::capture(&mut Cursor::new(bytes), length, q.clone(), &control()),
            Err(AccountedNativeError::Backing(BackingError::Length))
        ));
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    assert!(matches!(MemoryArchive::capture(&mut Overcount, 1, q.clone(), &control()), Err(AccountedNativeError::Backing(BackingError::Bounds))));
    let mut interrupted = InterruptedForever(0);
    assert!(matches!(MemoryArchive::capture(&mut interrupted, 1, q.clone(), &control()), Err(AccountedNativeError::Io(_))));
    assert_eq!(interrupted.0, usize::from(INTERRUPTED_RETRIES) + 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn actual_work_denial_and_cancellation_before_each_capture_phase_refund_storage() {
    let source = vec![9u8; CHUNK + 3];
    for threshold in [0, 1, 2, 100, CHUNK as u64 * 2 + 2, u64::MAX] {
        let q = quota(1 << 20);
        let used = AtomicU64::new(0);
        let debit = |n| {
            let old = used.fetch_add(n, Ordering::SeqCst);
            if old.checked_add(n).is_none_or(|sum| sum > threshold) { Err(BackingError::Work) } else { Ok(()) }
        };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
        let result = MemoryArchive::capture(&mut Cursor::new(source.clone()), source.len() as u64, q.clone(), &ctl);
        if threshold == u64::MAX {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
        drop(result);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    for cancel_at in [1, 3, 7, 12] {
        let q = quota(1 << 20);
        let calls = AtomicU64::new(0);
        let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= cancel_at;
        let interrupt = Interrupt::cancel_only(&cancel);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(
            MemoryArchive::capture(&mut Cursor::new(source.clone()), source.len() as u64, q.clone(), &ctl),
            Err(AccountedNativeError::Backing(BackingError::Cancelled))
        ));
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    assert_eq!(source, vec![9u8; CHUNK + 3]);
}

#[test]
fn real_native_wire_load_retains_storage_after_source_drop_and_preserves_native_words() {
    for (sample, pixel) in [
        (SampleType::U16, vec![0x01, 0x80, 0x23, 0x41, 0xfe, 0x0f, 0xff, 0xff]),
        (SampleType::F32, [0.125f32, 0.25, 0.75, 1.0].into_iter().flat_map(f32::to_ne_bytes).collect()),
    ] {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut doc = Document::new("held reader native", Size::new(2, 1), ColorMode::Rgb, sample);
        doc.layers.push(Layer::new("exact words", LayerContent::Raster(Surface::try_with_default_bytes(format, &pixel).unwrap())));
        doc.selection = Some(Surface::try_with_default_bytes(PixelFormat::GRAY8, &[127]).unwrap());
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let q = quota(64 << 20);
        let mut source = Cursor::new(bytes);
        let length = source.get_ref().len() as u64;
        let owned = open_accounted_native_from_reader(&mut source, length, Default::default(), q.clone(), &control()).unwrap();
        drop(source);
        assert_eq!(owned.document().name, doc.name);
        assert_eq!(owned.document().layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), pixel.repeat(2));
        assert_eq!(owned.document().selection.as_ref().unwrap().try_read_interleaved(doc.bounds()).unwrap(), vec![127, 127]);
        assert!(q.usage().unwrap().resident >= length);
        let accounted = owned.into_accounted_document(&control()).unwrap();
        assert!(accounted.quota().shares_ledger(&q));
        assert_eq!(accounted.document().layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), pixel.repeat(2));
        drop(accounted);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let q = quota(1 << 20);
    assert!(open_accounted_native_from_reader(&mut Cursor::new(vec![0u8; 128]), 128, Default::default(), q.clone(), &control()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
