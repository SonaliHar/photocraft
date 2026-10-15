use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_format::{
    LoadOptions, SaveOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{OwnedDocumentLimits, load_document_from_owned_archive},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    quota: BackingQuota,
    reads: AtomicU64,
    fail: AtomicU64,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Memory {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        let n = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail.load(Ordering::SeqCst) == n {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
        Ok(())
    }
}
fn quota(bytes: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: bytes, index: 4 << 20, scratch: 0 })
}
fn memory(bytes: Vec<u8>, q: &BackingQuota) -> Arc<Memory> {
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    Arc::new(Memory { bytes, quota: q.clone(), reads: AtomicU64::new(0), fail: AtomicU64::new(0), _lease: lease })
}
fn document(mode: ColorMode, sample: SampleType) -> Document {
    let format = PixelFormat::new(mode, sample, true);
    let native = match sample {
        SampleType::U8 => vec![17],
        SampleType::U16 => 0x0101_u16.to_ne_bytes().to_vec(),
        SampleType::U32 => 0x80000001_u32.to_ne_bytes().to_vec(),
        SampleType::F32 => (-0.0_f32).to_ne_bytes().to_vec(),
        SampleType::F64 => (-0.0_f64).to_ne_bytes().to_vec(),
    };
    let pixel = native.repeat(format.channels());
    let mut surface = Surface::try_with_default_bytes(format, &pixel).unwrap();
    let mut changed = pixel.clone();
    changed[0] ^= 1;
    surface.try_write_interleaved(Rect::new(-2, 1, -1, 2), &changed).unwrap();
    let mut doc = Document::new("whole model opaque and native", Size::new(8, 7), mode, sample);
    let mut layer = Layer::new("same owner", LayerContent::Raster(surface));
    layer.psd_blocks.push((*b"test", Arc::new(vec![7, 0, 255, 2])));
    doc.layers.push(layer);
    doc.metadata.xmp = Some("<xmp>retained</xmp>".into());
    doc
}

#[test]
fn owned_whole_resident_model_native_cow_history_and_source_lifetime() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let doc = document(mode, sample);
            let history = doc.clone();
            let bytes = photocraft_format::save_to_bytes(&doc, &SaveOptions::default()).unwrap();
            let q = quota(128 << 20);
            let source = memory(bytes, &q);
            let baseline = q.usage().unwrap();
            let weak = Arc::downgrade(&source);
            let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
            let owned = load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).unwrap();
            assert_eq!(owned.document(), &doc);
            assert!(q.usage().unwrap().resident > baseline.resident);
            assert_eq!(doc, history);
            let expected = doc.layers[0].surface().unwrap().try_read_interleaved(Rect::new(-3, 0, 1, 3)).unwrap();
            assert_eq!(owned.document().layers[0].surface().unwrap().try_read_interleaved(Rect::new(-3, 0, 1, 3)).unwrap(), expected);
            drop(source);
            assert!(weak.upgrade().is_some());
            drop(owned);
            assert!(weak.upgrade().is_none());
            assert_eq!(q.usage().unwrap().resident, 0);
        }
    }
}
#[test]
fn owned_late_io_work_and_cancellation_drop_model_objects_before_refunds() {
    let doc = document(ColorMode::Rgb, SampleType::F64);
    let bytes = photocraft_format::save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let q = quota(128 << 20);
    let source = memory(bytes, &q);
    let baseline = q.usage().unwrap();
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    source.fail.store(25, Ordering::SeqCst);
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    source.fail.store(0, Ordering::SeqCst);
    source.reads.store(0, Ordering::SeqCst);
    let work = AtomicU64::new(0);
    let charge = |n| {
        if work.fetch_add(n, Ordering::SeqCst) > 65536 { Err(BackingError::Work) } else { Ok(()) }
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&cancel);
    let charge = |n| {
        if n > 4096 {
            cancelled.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let ctl = ReadControl { interrupt: &interrupt, work: &charge };
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    drop(source);
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn owned_model_quota_denial_and_mixed_ledger_happen_without_document_publication() {
    let doc = document(ColorMode::Grayscale, SampleType::U32);
    let bytes = photocraft_format::save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let q = quota(512 << 10);
    let source = memory(bytes, &q);
    let baseline = q.usage().unwrap();
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    source.reads.store(0, Ordering::SeqCst);
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), quota(128 << 20), &ctl).is_err());
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    assert_eq!(q.usage().unwrap(), baseline);
    drop(source);
    assert_eq!(q.usage().unwrap().resident, 0);
}

/// UNRUN: the public handoff ledger is the genuine retained owner, not an equal
/// limits reconstruction; related metadata keeps its own charge through drop.
#[test]
fn owned_quota_accessor_exposes_actual_ledger_without_replacing_storage_ownership() {
    let doc = document(ColorMode::Grayscale, SampleType::F32);
    let q = quota(128 << 20);
    let source = memory(photocraft_format::save_to_bytes(&doc, &SaveOptions::default()).unwrap(), &q);
    let weak = Arc::downgrade(&source);
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let owned = load_document_from_owned_archive(source, &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).unwrap();
    let before = q.usage().unwrap();
    assert!(owned.quota().shares_ledger(&q));
    assert!(!owned.quota().shares_ledger(&quota(128 << 20)));
    assert_eq!(q.usage().unwrap(), before);
    let reply = owned.quota().reserve(QuotaKind::Resident, 137).unwrap();
    assert_eq!(q.usage().unwrap().resident, before.resident + 137);
    assert!(weak.upgrade().is_some());
    drop(owned);
    assert!(weak.upgrade().is_none());
    assert_eq!(q.usage().unwrap().resident, 137);
    drop(reply);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn owned_save_stream_preserves_shared_mapping_native_and_metadata() {
    let doc = document(ColorMode::Rgb, SampleType::F64);
    let original = photocraft_format::save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let q = quota(128 << 20);
    let source = memory(original, &q);
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let owned = load_document_from_owned_archive(source, &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).unwrap();
    let baseline = q.usage().unwrap();
    let saved = owned.save_archive(std::io::Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &ctl).unwrap().into_inner();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), doc);
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
fn owned_save_transport_all_native_depths_defaults_and_source_drop() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let expected = document(mode, sample);
            let historical = expected.clone();
            let q = quota(128 << 20);
            let source = memory(photocraft_format::save_to_bytes(&expected, &SaveOptions::default()).unwrap(), &q);
            let weak = Arc::downgrade(&source);
            let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
            let owned = load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).unwrap();
            drop(source);
            assert!(weak.upgrade().is_some());
            let baseline = q.usage().unwrap();
            let output = owned.save_archive(std::io::Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &ctl).unwrap().into_inner();
            assert_eq!(photocraft_format::load_from_bytes(&output).unwrap(), expected);
            assert_eq!(owned.document(), &historical);
            assert_eq!(q.usage().unwrap(), baseline);
            drop(owned);
            assert!(weak.upgrade().is_none());
            assert_eq!(q.usage().unwrap().resident, 0);
        }
    }
}

#[test]
fn owned_save_failed_output_work_cancel_and_quota_preserve_owner_and_refund() {
    struct Fail(std::io::Cursor<Vec<u8>>);
    impl std::io::Write for Fail {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected destination failure"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl std::io::Seek for Fail {
        fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(&mut self.0, p)
        }
    }
    let expected = document(ColorMode::Rgb, SampleType::U32);
    let q = quota(128 << 20);
    let source = memory(photocraft_format::save_to_bytes(&expected, &SaveOptions::default()).unwrap(), &q);
    let normal = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let owned = load_document_from_owned_archive(source, &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &normal).unwrap();
    let baseline = q.usage().unwrap();
    assert!(owned.save_archive(Fail(std::io::Cursor::new(Vec::new())), OwnedDocumentLimits::default(), &normal).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let visits = AtomicU64::new(0);
    let work = |n| {
        if visits.fetch_add(n, Ordering::SeqCst) > 8192 { Err(BackingError::Work) } else { Ok(()) }
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(owned.save_archive(std::io::Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &ctl).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let flag = AtomicBool::new(false);
    let cancel = || flag.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&cancel);
    let work = |n| {
        if n >= 4096 {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let ctl = ReadControl { interrupt: &interrupt, work: &work };
    assert!(matches!(
        owned.save_archive(std::io::Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &ctl),
        Err(photocraft_format::FormatError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    let pressure = q.reserve(QuotaKind::Resident, (128 << 20) - baseline.resident - 1).unwrap();
    let pressured = q.usage().unwrap();
    assert!(owned.save_archive(std::io::Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &normal).is_err());
    assert_eq!(q.usage().unwrap(), pressured);
    drop(pressure);
    assert_eq!(owned.document(), &expected);
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
fn save_profile_layer_count_is_checked_before_destination_or_model_staging() {
    let doc = Document::with_background("bounded layer count", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, photocraft_doc::Color::WHITE);
    let q = quota(64 << 20);
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let mut output = std::io::Cursor::new(Vec::new());
    let limits = OwnedDocumentLimits { max_layers: 0, ..Default::default() };
    assert!(photocraft_format::owned_document::save_current_document_archive(&doc, &mut output, limits, &q, &ctl).is_err());
    assert!(output.into_inner().is_empty());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    assert!(
        photocraft_format::owned_document::save_current_document_archive(
            &doc,
            std::io::Cursor::new(Vec::new()),
            OwnedDocumentLimits { max_layers: 1, ..Default::default() },
            &q,
            &ctl
        )
        .is_ok()
    );
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
