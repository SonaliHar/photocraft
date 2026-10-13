//! Structural GPU refusal is independent of source provenance and visibility.
use photocraft_color::PixelFormat;
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: AtomicUsize,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        Err(BackingError::Io)
    }
}
#[test]
fn backed_gpu_plan_refuses_visible_hidden_and_nested_without_read_or_cache_allocation() {
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0, 0, 0, 255]).unwrap();
    let store = Arc::new(Store { descriptor, reads: AtomicUsize::new(0) });
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 0, index: 1 << 20 });
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let raster = BackedRaster::new(store.clone(), quota.clone(), 1, &control).unwrap();
    let layer = Layer::new("owned source", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control).unwrap()));
    let mut doc = Document::new("GPU refusal", Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    doc.layers.push(layer);
    let before = quota.usage().unwrap();
    assert!(photocraft_gpu::plan::plan(&doc).err().unwrap().0.contains("Native-backed"));
    doc.layers[0].visible = false;
    assert!(photocraft_gpu::plan::plan(&doc).is_err());
    let child = doc.layers.remove(0);
    let mut group = Layer::group("nested", Vec::new());
    group.children_mut().unwrap().push(child);
    doc.layers.push(group);
    assert!(photocraft_gpu::plan::plan(&doc).is_err());
    assert_eq!(store.reads.load(Ordering::Acquire), 0);
    assert_eq!(quota.usage().unwrap(), before);
}
