//! UNRUN genuine owned-model constructor metadata boundary draft.
use super::*;
use crate::{BackedLayer, ColorMode, Document, Layer, PixelFormat, SampleType, Size};
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        Err(BackingError::Io)
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
fn exact_owner_fee_is_separate_from_actual_existing_model_and_backing_guard() {
    let requirement = AccountedDocument::owned_model_reservation_bytes();
    assert_eq!(requirement, (std::mem::size_of::<ModelOwner>() + 128) as u64);
    for enough in [false, true] {
        let q = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 16, scratch: 0 });
        // Existing model ownership is a real retained fee, not an asserted count.
        let retention = Arc::new(q.reserve(QuotaKind::Resident, 8192).unwrap());
        let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, Rect::new(0, 0, 1, 1), &[0; 16]).unwrap();
        let backing = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
        let layer = Layer::new("retained", LayerContent::BackedRaster(BackedLayer::new(backing.clone(), descriptor, &control()).unwrap()));
        let mut doc = Document::new("model", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        doc.layers.push(layer);
        let allowance = requirement - u64::from(!enough);
        let blocker = q.reserve(QuotaKind::Index, (1 << 16) - q.usage().unwrap().index - allowance).unwrap();
        let before = q.usage().unwrap();
        let result = AccountedDocument::from_owned(doc, retention.clone(), q.clone(), &control());
        if enough {
            assert!(result.is_ok());
            assert_eq!(q.usage().unwrap().index, before.index + requirement);
        } else {
            assert!(matches!(result, Err(BackingError::Quota)));
            assert_eq!(q.usage().unwrap(), before);
        }
        drop(result);
        assert_eq!(q.usage().unwrap(), before);
        drop(blocker);
        drop(backing);
        drop(retention);
        assert_eq!(q.usage().unwrap(), photocraft_raster::backing::quota::QuotaUsage::default());
    }
}
