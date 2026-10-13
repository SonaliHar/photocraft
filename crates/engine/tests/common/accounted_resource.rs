use photocraft_doc::{BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, Rect, SampleType, Size};
use photocraft_raster::backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl, quota::BackingQuota};
use std::sync::Arc;
#[derive(Debug)]
struct ZeroPlane {
    descriptor: NativePlaneDescriptor,
}
impl NativePlaneStore for ZeroPlane {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).is_none_or(|end| end > 16) {
            return Err(BackingError::Bounds);
        }
        output.fill(0);
        Ok(())
    }
}
pub fn document(quota: &BackingQuota, control: &ReadControl<'_>) -> Document {
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 2, 2), &[0; 4]).unwrap();
    let raster = BackedRaster::new(Arc::new(ZeroPlane { descriptor }), quota.clone(), 2, control).unwrap();
    let layer = BackedLayer::new(raster, descriptor, control).unwrap();
    let mut document = Document::new("owned native fixture", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    document.layers.push(Layer::new("native zero", LayerContent::BackedRaster(layer)));
    document
}
