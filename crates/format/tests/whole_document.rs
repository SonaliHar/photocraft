//! Whole-document finite native integration through the charged owned archive codec.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent, PixelContentRef};
use photocraft_format::{
    LoadOptions, SaveOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{OwnedDocumentLimits, load_document_from_owned_archive, save_current_document_archive},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
    },
};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 8 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Defaults(NativePlaneDescriptor);
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        let end = offset.checked_add(out.len() as u64).ok_or(BackingError::Bounds)?;
        if end > self.0.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        ctl.check(out.len() as u64)?;
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.0.default_bytes()[((offset + i as u64) % self.0.default_bytes().len() as u64) as usize];
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Archive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    fail: AtomicBool,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        if self.fail.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Io)?);
        Ok(())
    }
}
fn archive(bytes: Vec<u8>, q: &BackingQuota) -> Arc<Archive> {
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    Arc::new(Archive { bytes, quota: q.clone(), fail: AtomicBool::new(false), _lease: lease })
}
fn native(layer: &BackedLayer) -> Vec<u8> {
    let d = layer.descriptor();
    let mut output = vec![77; d.logical_bytes() as usize];
    // Test-only small-plane whole copy. Production save/load uses bounded objects.
    for (i, bytes) in output.chunks_mut(65536).enumerate() {
        let width = d.bounds().width() as usize;
        let bpp = d.format().bytes_per_pixel();
        for (j, pixel) in bytes.chunks_exact_mut(bpp).enumerate() {
            let index = i * 65536 / bpp + j;
            let x = d.bounds().x0 + (index % width) as i32;
            let y = d.bounds().y0 + (index / width) as i32;
            layer.checked_view().read_logical_pixel_native_into(x, y, pixel, &control()).unwrap();
        }
    }
    output
}
fn backed(doc: &Document) -> &BackedLayer {
    match doc.layers[0].pixel_content().unwrap() {
        PixelContentRef::Backed(layer) => layer,
        _ => panic!("backing silently materialized or disappeared"),
    }
}
#[test]
fn whole_document_current_cow_native_ids_metadata_history_and_owner_lifetime() {
    for sample in [SampleType::U32, SampleType::F64] {
        let q = quota();
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let default = if sample == SampleType::U32 { 0x01000001_u32.to_ne_bytes().repeat(4) } else { (-0.0_f64).to_ne_bytes().repeat(4) };
        let d = NativePlaneDescriptor::new(format, Rect::new(-2, -1, 259, 3), &default).unwrap();
        let raster = BackedRaster::new(Arc::new(Defaults(d)), q.clone(), 2, &control()).unwrap();
        let original = BackedLayer::new(raster, d, &control()).unwrap();
        let edit = if sample == SampleType::U32 { 0x01000003_u32.to_ne_bytes().repeat(4) } else { f64::from_bits(0x3fd3333333333334).to_ne_bytes().repeat(4) };
        let current = original.try_edited_native(Rect::new(-1, 0, 0, 1), &edit, &control()).unwrap();
        let history = current.clone();
        let expected = native(&current);
        let original_expected = native(&original);
        let mut doc = Document::new("whole finite document", Size::new(512, 64), ColorMode::Rgb, sample);
        let mut layer = Layer::new("exact current", LayerContent::BackedRaster(current));
        layer.psd_blocks.push((*b"test", Arc::new(vec![19, 20, 21])));
        doc.layers.push(layer);
        let id = doc.layers[0].id;
        assert!(doc.has_backed_rasters().unwrap());
        let bytes = save_current_document_archive(&doc, Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &control()).unwrap().into_inner();
        let wire = photocraft_format::read_manifest(&bytes).unwrap();
        assert_eq!(wire.minimum_reader_version, 5);
        assert_eq!(wire.format_version, photocraft_format::FORMAT_VERSION);
        assert!(photocraft_format::load_from_bytes(&bytes).is_err(), "borrowed archive is not persistent immutable storage authority");
        let source = archive(bytes, &q);
        let loaded = load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).unwrap();
        assert_eq!(loaded.document().id, doc.id);
        assert_eq!(loaded.document().layers[0].id, id);
        assert_eq!(loaded.document().layers[0].name, doc.layers[0].name);
        assert_eq!(loaded.document().layers[0].psd_blocks, doc.layers[0].psd_blocks);
        assert_eq!(loaded.document().size, doc.size);
        assert_eq!(loaded.document().mode, doc.mode);
        assert_eq!(loaded.document().depth, doc.depth);
        assert_eq!(backed(loaded.document()).descriptor(), d);
        assert_eq!(native(backed(loaded.document())), expected);
        let source_weak = Arc::downgrade(&source);
        drop(source);
        drop(doc);
        assert!(source_weak.upgrade().is_some(), "Document must own authenticated source after operation/provider scope");
        assert_eq!(native(backed(loaded.document())), expected);
        assert_eq!(native(&history), expected);
        assert_eq!(native(&original), original_expected);
        drop(loaded);
        assert!(source_weak.upgrade().is_none());
        drop(history);
        drop(original);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn whole_document_late_backing_io_cancel_and_quota_never_publish_or_erase_source() {
    let q = quota();
    let format = PixelFormat::RGBA8;
    let d = NativePlaneDescriptor::new(format, Rect::new(-2, 3, 4, 5), &[1, 2, 3, 4]).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(Defaults(d)), q.clone(), 2, &control()).unwrap(), d, &control()).unwrap();
    let mut doc = Document::new("cancel", Size::new(32, 32), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("source", LayerContent::BackedRaster(layer.try_edited_native(Rect::new(-2, 3, -1, 4), &[9, 8, 7, 6], &control()).unwrap())));
    let old = doc.clone();
    let baseline = q.usage().unwrap();
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(save_current_document_archive(&doc, Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &ctl).is_err());
    assert_eq!(doc, old);
    assert_eq!(q.usage().unwrap(), baseline);
    let bytes = save_current_document_archive(&doc, Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &control()).unwrap().into_inner();
    let source = archive(bytes, &q);
    let retained = q.usage().unwrap();
    let loaded = load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).unwrap();
    source.fail.store(true, Ordering::SeqCst);
    let mut out = [99; 4];
    let before = out;
    assert!(backed(loaded.document()).checked_view().read_logical_pixel_native_into(-2, 3, &mut out, &control()).is_err());
    assert_eq!(out, before);
    drop(loaded);
    assert_eq!(q.usage().unwrap(), retained);
    let work = |_| Err(BackingError::Work);
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &ctl).is_err());
    assert_eq!(q.usage().unwrap(), retained);
    let held = q.reserve(QuotaKind::Resident, (64 << 20) - retained.resident).unwrap();
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).is_err());
    drop(held);
    assert_eq!(q.usage().unwrap(), retained);
    drop(source);
    drop(old);
    drop(doc);
    drop(layer);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn finite_reader_floor_is_selective_and_detects_a_falsely_low_declaration() {
    let resident = Document::new("resident", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let ordinary = photocraft_format::save_to_bytes(&resident, &SaveOptions::default()).unwrap();
    assert_eq!(photocraft_format::read_manifest(&ordinary).unwrap().minimum_reader_version, 2);
    let q = quota();
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(-1, 0, 1, 1), &[0; 4]).unwrap();
    let raster = BackedRaster::new(Arc::new(Defaults(d)), q.clone(), 1, &control()).unwrap();
    let layer = BackedLayer::new(raster, d, &control()).unwrap();
    let mut doc = resident.clone();
    doc.layers.push(Layer::group("nested", vec![Layer::new("owned", LayerContent::BackedRaster(layer))]));
    let bytes = save_current_document_archive(&doc, Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &control()).unwrap().into_inner();
    let mut manifest = photocraft_format::read_manifest(&bytes).unwrap();
    assert_eq!(manifest.minimum_reader_version, 5);
    manifest.minimum_reader_version = 4;
    let json = serde_json::to_vec(&manifest).unwrap();
    // All-default finite plane needs no tile object. This fixture specifically
    // tests the metadata floor before any unrelated payload read.
    let mut zip = photocraft_format::zip::stream::Writer::new(Cursor::new(Vec::new()), Default::default()).unwrap();
    zip.add("manifest.json", json.len() as u64, &mut Cursor::new(json), &Interrupt::NONE).unwrap();
    let falsely_old = zip.finish(&Interrupt::NONE).unwrap().into_inner();
    assert!(photocraft_format::read_manifest(&falsely_old).is_err());
    let source = archive(falsely_old, &q);
    let baseline = q.usage().unwrap();
    assert!(load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    drop(source);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn backing_read_cancellation_is_typed_and_does_not_publish_archive() {
    #[derive(Debug)]
    struct CancelStore {
        descriptor: NativePlaneDescriptor,
        signal: Arc<AtomicBool>,
    }
    impl NativePlaneStore for CancelStore {
        fn descriptor(&self) -> NativePlaneDescriptor {
            self.descriptor
        }
        fn read_exact_at(&self, _: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
            out.fill(17);
            self.signal.store(true, Ordering::SeqCst);
            ctl.check(0)
        }
    }
    let q = quota();
    let flag = Arc::new(AtomicBool::new(false));
    let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(-2, 3, 1, 6), &[0; 4]).unwrap();
    let store = Arc::new(CancelStore { descriptor: d, signal: flag.clone() });
    let layer = BackedLayer::new(BackedRaster::new(store, q.clone(), 2, &control()).unwrap(), d, &control()).unwrap();
    let mut doc = Document::new("typed backing cancel", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("owned", LayerContent::BackedRaster(layer)));
    let old = doc.clone();
    let before = q.usage().unwrap();
    let interrupted = || flag.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&interrupted);
    let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(
        save_current_document_archive(&doc, Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &ctl),
        Err(photocraft_format::FormatError::Cancelled)
    ));
    assert!(flag.load(Ordering::SeqCst));
    assert_eq!(doc, old);
    assert_eq!(q.usage().unwrap(), before);
    drop(old);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
