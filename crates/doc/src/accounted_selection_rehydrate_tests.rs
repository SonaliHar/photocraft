//! UNRUN regression: loaded raw selection becomes a real editable owner.
use super::*;
use crate::{BackedLayer, ColorMode, Layer, PixelFormat, SampleType, Size};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};

#[derive(Debug)]
struct ZeroStore(NativePlaneDescriptor);
impl NativePlaneStore for ZeroStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        output.fill(0);
        Ok(())
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}

#[test]
fn rehydrated_selection_allows_metadata_edit_deselect_undo_and_redo() {
    let quota = BackingQuota::new(QuotaLimits { resident: 16 << 20, scratch: 16 << 20, index: 16 << 20 });
    let retention = Arc::new(quota.reserve(QuotaKind::Resident, 1 << 20).unwrap());
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, Rect::new(0, 0, 2, 1), &[0; 16]).unwrap();
    let raster = BackedRaster::new(Arc::new(ZeroStore(descriptor)), quota.clone(), 1, &control()).unwrap();
    let layer = Layer::new("original", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap()));
    let id = layer.id;
    let mut document = Document::new("loaded", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    document.layers.push(layer);
    document.source_color_context = Some(crate::SourceColorContext { original_icc: None, source_precision: 675 });
    document.selection = Some(Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &0.25f32.to_ne_bytes()).unwrap());
    let loaded = AccountedDocument::from_owned_rehydrating_selection(document, retention, quota, &control()).unwrap();
    loaded.validate_selection_replacement(&control()).unwrap();
    let renamed = loaded.try_edit(AccountedEdit::LayerName { layer: id, name: "renamed" }, &control()).unwrap();
    assert_eq!(renamed.document().selection.as_ref().unwrap().default_bytes(), &0.25f32.to_ne_bytes());
    let mut history = AccountedHistory::new(renamed.clone(), 4, &control()).unwrap();
    history.try_replace_selection(None, &control()).unwrap();
    assert!(history.current().unwrap().document().selection.is_none());
    history.undo(&control()).unwrap();
    assert!(history.current().unwrap().shares_model(&renamed));
    history.redo(&control()).unwrap();
    assert!(history.current().unwrap().document().selection.is_none());
}
