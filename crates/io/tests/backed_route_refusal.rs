//! Ordinary export must refuse hidden/nested owned backing before reading or publishing output.
use photocraft_doc::{BackedLayer, ColorMode, Document, Layer, LayerContent, SampleType, Size};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl, quota::{BackingQuota, QuotaLimits, QuotaUsage}}};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

#[derive(Debug)]
struct Unreadable {
    descriptor: NativePlaneDescriptor,
    reads: AtomicUsize,
}
impl NativePlaneStore for Unreadable {
    fn descriptor(&self) -> NativePlaneDescriptor { self.descriptor }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Err(BackingError::Io)
    }
}

#[test]
fn hidden_nested_backing_is_not_exported_as_empty_pixels() {
    let mut doc = Document::new("own refusal fixture", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), Rect::new(-1, -1, 2, 2), &[0, 0, 0, 0]).unwrap();
    let source = Arc::new(Unreadable { descriptor, reads: AtomicUsize::new(0) });
    let quota = BackingQuota::new(QuotaLimits { resident: 2 << 20, scratch: 0, index: 1 << 20 });
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let raster = BackedRaster::new(source.clone(), quota.clone(), 1, &control).unwrap();
    let mut hidden = Layer::new("hidden source", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control).unwrap()));
    hidden.visible = false;
    doc.layers.push(Layer::group("nested", vec![hidden]));
    let before = doc.clone();
    let usage = quota.usage().unwrap();
    assert!(photocraft_io::document_to_image(&doc, &mut Vec::new()).is_err());
    assert!(photocraft_io::document_to_psd(&doc).is_err());
    assert!(photocraft_io::document_to_psd_with(&doc, &Default::default()).is_err());
    assert!(photocraft_io::try_document_to_psd_with(&doc, &Default::default()).is_err());
    assert!(photocraft_io::save_native_bytes(&doc, Default::default(), &Interrupt::NONE).is_err());
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert_eq!(quota.usage().unwrap(), usage);
    assert_eq!(doc, before);
    drop((doc, before, source));
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
