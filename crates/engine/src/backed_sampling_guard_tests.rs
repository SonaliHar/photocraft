//! Metadata-only refusal must not read native backing or change sampled scope.
use super::*;
use photocraft_doc::{BackedLayer, Color, ColorMode, LayerContent, PixelFormat, SampleType, accounted_document::AccountedDocument};
use photocraft_geom::{Rect, Size};
use photocraft_raster::backing::{
    BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct NeverRead {
    descriptor: NativePlaneDescriptor,
    reads: Arc<AtomicUsize>,
}
impl NativePlaneStore for NeverRead {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Err(BackingError::Io)
    }
}
fn fixture() -> (Document, BackingQuota, LayerId, LayerId, Arc<AtomicUsize>) {
    let read = crate::read_control_none();
    let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 4 << 20, scratch: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(1, 1, 3, 3), &[255; 4]).unwrap();
    let raster = BackedRaster::new(Arc::new(NeverRead { descriptor, reads: Arc::clone(&reads) }), quota.clone(), 1, &read).unwrap();
    let backed = BackedLayer::new(raster, descriptor, &read).unwrap();
    let mut doc = Document::with_background("scope", Size::new(4, 4), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let background = doc.layers[0].id;
    let layer = Layer::new("native top", LayerContent::BackedRaster(backed));
    let id = layer.id;
    doc.layers.push(layer);
    (doc, quota, id, background, reads)
}
#[test]
fn backed_hit_is_not_transparent_clickthrough_but_outside_extent_is_skipped() {
    let (mut doc, _, id, background, reads) = fixture();
    let before = doc.clone();
    assert!(crate::pick_cmds::layers_at_checked(&doc, 1, 1, &Interrupt::NONE).unwrap_err().to_string().contains("backed-pixel-sampling-unsupported"));
    assert!(crate::pick_cmds::auto_select_target_checked(&doc, 1, 1, false, &Interrupt::NONE).is_err());
    assert_eq!(crate::pick_cmds::layers_at_checked(&doc, 0, 0, &Interrupt::NONE).unwrap(), vec![background]);
    assert_eq!(doc, before);
    doc.layer_mut(id).unwrap().visible = false;
    assert_eq!(crate::pick_cmds::layers_at_checked(&doc, 1, 1, &Interrupt::NONE).unwrap(), vec![background]);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
#[test]
fn active_backed_selection_refuses_before_merged_fallback_and_keeps_history() {
    let (doc, quota, id, _, reads) = fixture();
    let retention = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let owner = AccountedDocument::from_owned(doc, Arc::new(retention), quota, &crate::read_control_none()).unwrap();
    let mut session = Session::new();
    session.add_accounted_document(owner, None, &crate::read_control_none()).unwrap();
    session.active_mut().unwrap().active_layer = Some(id);
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    assert!(
        sampled_pixels(&session, "select.colorRange", false, &Interrupt::NONE).unwrap_err().to_string().contains("active-backed-pixel-sampling-unsupported")
    );
    assert!(crate::selection_cmds::ColorRangeSamples::new(&session, &serde_json::json!({"sampleAllLayers":false})).is_err());
    assert!(session.execute("layer.pickAt", serde_json::json!({"x":1,"y":1,"select":false})).is_err());
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(before.shares_state(&session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().active_layer, Some(id));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    // Explicit all-layer metadata policy is unchanged; actual rendering keeps
    // its existing independent backed-document admission/refusal boundary.
    assert!(sampled_pixels(&session, "select.colorRange", true, &Interrupt::NONE).is_ok());
    assert!(matches!(sampled_pixels(&session, "select.colorRange", false, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
}
