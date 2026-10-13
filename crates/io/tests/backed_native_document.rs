//! Actual owned native wire to borrowed Document to bounded CPU composition.
use photocraft_cms::Builtin;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_compose::{backed_document::PreparedBackedDocument, backed_normal::NormalBandLimits};
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent, PixelContentRef, SampleEncoding, WorkingProfileInterpretation};
use photocraft_format::{
    LoadOptions,
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
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 8 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Defaults(NativePlaneDescriptor);
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.0.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
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
    reads: AtomicU64,
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
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn archive(bytes: Vec<u8>, q: &BackingQuota) -> Arc<Archive> {
    let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    Arc::new(Archive { bytes, quota: q.clone(), fail: AtomicBool::new(false), reads: AtomicU64::new(0), _lease: lease })
}
fn pixels(format: PixelFormat, values: &[f64]) -> Vec<u8> {
    let mut bytes = vec![0; format.bytes_per_pixel()];
    for (sample, value) in bytes.chunks_exact_mut(format.sample.bytes()).zip(values) {
        try_write_sample_f64(sample, format.sample, 0, *value).unwrap();
    }
    bytes
}
fn backing(doc: &Document) -> &BackedLayer {
    match doc.layers[0].pixel_content().unwrap() {
        PixelContentRef::Backed(layer) => layer,
        _ => panic!("owned storage materialized"),
    }
}
fn read(layer: &BackedLayer, x: i32, y: i32) -> Vec<u8> {
    let mut output = vec![0; layer.format().bytes_per_pixel()];
    layer.checked_view().read_logical_pixel_native_into(x, y, &mut output, &control()).unwrap();
    output
}
fn fixture(mode: ColorMode, sample: SampleType, q: &BackingQuota) -> (Document, BackedLayer, BackedLayer, Vec<u8>, Vec<u8>) {
    let format = PixelFormat::new(mode, sample, true);
    let n = format.channels();
    let default = if sample == SampleType::U32 {
        [0x01000001_u32, 0x23000003, 0x45000005, 0x80000001][..n].iter().flat_map(|v| v.to_ne_bytes()).collect()
    } else {
        pixels(format, if mode == ColorMode::Rgb { &[-0.0, 0.25, 0.75, 0.5] } else { &[-0.0, 0.5] })
    };
    let edited_rgb = [f64::from_bits(0x3fd3333333333334), 0.45, 0.65, 0.75];
    let edited_gray = [f64::from_bits(0x3fd3333333333334), 0.75];
    let edit = pixels(format, if mode == ColorMode::Rgb { &edited_rgb } else { &edited_gray });
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(-2, -1, 260, 3), &default).unwrap();
    let raster = BackedRaster::new(Arc::new(Defaults(descriptor)), q.clone(), 2, &control()).unwrap();
    let original = BackedLayer::new(raster, descriptor, &control()).unwrap();
    let current = original.try_edited_native(Rect::new(-1, 0, 0, 1), &edit, &control()).unwrap();
    let history = current.clone();
    let mut doc = Document::new("owned codec CPU join", Size::new(512, 64), mode, sample);
    doc.layers.push(Layer::new("current exact owned source", LayerContent::BackedRaster(current)));
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    doc.icc_profile = Some(if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::SGray }.profile().to_bytes());
    (doc, original, history, default, edit)
}

#[test]
fn owned_archive_cpu_join_preserves_current_native_bits_and_history_and_source_lifetime() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U32, SampleType::F64] {
            let q = quota();
            let (doc, original, history, default, edit) = fixture(mode, sample, &q);
            let bytes = save_current_document_archive(&doc, Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &control()).unwrap().into_inner();
            let source = archive(bytes, &q);
            let weak = Arc::downgrade(&source);
            let loaded =
                load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).unwrap();
            assert_eq!(loaded.document().sample_encoding, doc.sample_encoding);
            assert_eq!(loaded.document().working_profile_interpretation, doc.working_profile_interpretation);
            assert_eq!(loaded.document().layers[0].id, doc.layers[0].id);
            assert_eq!(read(backing(loaded.document()), -1, 0), edit);
            assert_eq!(read(backing(loaded.document()), 0, 0), default);
            assert_eq!(read(&history, -1, 0), edit);
            assert_eq!(read(&original, -1, 0), default);
            drop((source, doc));
            assert!(weak.upgrade().is_some());
            let profile = if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::SGray }.profile();
            let prepared = PreparedBackedDocument::new(loaded.document(), profile, NormalBandLimits::default(), &control()).unwrap();
            let band = prepared.render_band(Rect::new(-1, 0, 1, 1), &control()).unwrap();
            for (output, raw) in band.pixels().iter().zip([edit.as_slice(), default.as_slice()]) {
                let values: Vec<f64> = raw.chunks_exact(sample.bytes()).map(|v| try_read_sample_f64(v, sample, 0).unwrap()).collect();
                let expected = if mode == ColorMode::Rgb { [values[0], values[1], values[2], values[3]] } else { [values[0], values[0], values[0], values[1]] };
                for (a, b) in output.iter().zip(expected) {
                    assert!((a - b).abs() < 1.0e-14, "{mode:?}/{sample:?}: {output:?} != {expected:?}");
                }
            }
            drop((band, prepared));
            drop(loaded);
            assert!(weak.upgrade().is_none());
            drop((original, history));
            assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        }
    }
}

#[test]
fn loaded_archive_late_read_failure_and_cancel_publish_no_cpu_band_and_allow_retry() {
    let q = quota();
    let (doc, original, history, default, edit) = fixture(ColorMode::Rgb, SampleType::F64, &q);
    let bytes = save_current_document_archive(&doc, Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &control()).unwrap().into_inner();
    let source = archive(bytes, &q);
    let loaded = load_document_from_owned_archive(source.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).unwrap();
    let prepared = PreparedBackedDocument::new(loaded.document(), Builtin::Srgb.profile(), NormalBandLimits::default(), &control()).unwrap();
    let reads = source.reads.load(Ordering::Relaxed);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(prepared.render_band(Rect::new(-1, 0, 1, 1), &ctl).is_err());
    assert_eq!(source.reads.load(Ordering::Relaxed), reads);
    source.fail.store(true, Ordering::Relaxed);
    assert!(prepared.render_band(Rect::new(-1, 0, 1, 1), &control()).is_err());
    assert!(source.reads.load(Ordering::Relaxed) > reads);
    assert_eq!(read(&history, -1, 0), edit);
    assert_eq!(read(&original, -1, 0), default);
    source.fail.store(false, Ordering::Relaxed);
    let band = prepared.render_band(Rect::new(-1, 0, 1, 1), &control()).unwrap();
    assert_eq!(band.pixels().len(), 2);
    drop((band, prepared));
    drop((loaded, source, doc, original, history));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
