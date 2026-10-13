use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    BackedLayer, Document, Layer, LayerContent,
    accounted_document::{AccountedDocument, AccountedEdit},
};
use photocraft_engine::document_snapshot::DocumentSnapshot;
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    dropped: Arc<AtomicUsize>,
    fail: bool,
}
impl Drop for Store {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        if self.fail {
            return Err(BackingError::Io);
        };
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.descriptor.default_bytes()[((offset + i as u64) % 32) as usize];
        }
        Ok(())
    }
}
fn owner(fail: bool) -> (AccountedDocument, BackingQuota, Arc<AtomicUsize>) {
    let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 0, index: 1 << 20 });
    let retention = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let descriptor =
        NativePlaneDescriptor::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(-2, 1, 2, 3), &(-0.0f64).to_ne_bytes().repeat(4))
            .unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    let raster = BackedRaster::new(Arc::new(Store { descriptor, dropped: dropped.clone(), fail }), quota.clone(), 2, &control()).unwrap();
    let layer = BackedLayer::new(raster, descriptor, &control()).unwrap();
    let mut document = Document::new("snapshot", Size::new(4, 4), ColorMode::Rgb, SampleType::F64);
    document.layers.push(Layer::new("current", LayerContent::BackedRaster(layer)));
    (AccountedDocument::from_owned(document, Arc::new(retention), quota.clone(), &control()).unwrap(), quota, dropped)
}
#[test]
fn accounted_snapshot_retains_exact_native_history_and_weak_payload_lifetime() {
    let (owner, q, dropped) = owner(false);
    let original = DocumentSnapshot::from_accounted(owner).unwrap();
    let mut history = original.clone();
    let weak = original.downgrade();
    assert!(weak.is_current(&history));
    assert!(history.resident_arc().is_none());
    assert!(history.try_resident_mut().is_err());
    let bytes = f64::from_bits(0x3fd0000000000001).to_ne_bytes().repeat(4);
    let changed = original
        .try_accounted_edit(AccountedEdit::NativePixels { layer: original.layers[0].id, rect: Rect::new(-1, 1, 0, 2), bytes: &bytes }, &control())
        .unwrap();
    assert!(!original.shares_state(&changed));
    assert!(!weak.is_current(&changed));
    let LayerContent::BackedRaster(previous) = &history.layers[0].content else { panic!("history erased") };
    let mut out = [0u8; 32];
    previous.checked_view().read_logical_pixel_native_into(-1, 1, &mut out, &control()).unwrap();
    assert_eq!(out, (-0.0f64).to_ne_bytes().repeat(4).as_slice());
    let LayerContent::BackedRaster(current) = &changed.layers[0].content else { panic!("current erased") };
    current.checked_view().read_logical_pixel_native_into(-1, 1, &mut out, &control()).unwrap();
    assert_eq!(out, bytes.as_slice());
    let job = weak.upgrade().unwrap();
    drop(original);
    drop(history);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(job);
    drop(changed);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert!(q.usage().unwrap().index > 0);
    drop(weak);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn resident_identity_and_native_refusals_do_not_detach_retention() {
    let doc = Arc::new(Document::new("resident", Size::new(2, 2), ColorMode::Rgb, SampleType::U8));
    let mut snapshot = DocumentSnapshot::from_resident(doc.clone()).unwrap();
    let weak = snapshot.downgrade();
    assert!(snapshot.shares_state(&weak.upgrade().unwrap()));
    snapshot.try_resident_mut().unwrap().name = "changed".into();
    assert!(!weak.is_current(&snapshot));
    assert_eq!(doc.name, "resident");
    let (owner, q, dropped) = owner(true);
    assert!(DocumentSnapshot::from_resident(Arc::new(owner.document().clone())).is_err());
    let snapshot = DocumentSnapshot::from_accounted(owner).unwrap();
    let before = q.usage().unwrap();
    let weak = snapshot.downgrade();
    assert!(
        snapshot
            .try_accounted_edit(AccountedEdit::NativePixels { layer: snapshot.layers[0].id, rect: Rect::new(-1, 1, 0, 2), bytes: &[0; 32] }, &control())
            .is_err()
    );
    assert!(weak.is_current(&snapshot));
    assert_eq!(q.usage().unwrap(), before);
    drop(snapshot);
    drop(weak);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn structural_equality_is_borrowed_and_never_reads_native_backing() {
    let resident = Document::new("same", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let a = DocumentSnapshot::from_resident(Arc::new(resident.clone())).unwrap();
    let mut b = DocumentSnapshot::from_resident(Arc::new(resident)).unwrap();
    assert_eq!(a, b);
    assert!(!a.shares_state(&b));
    b.try_resident_mut().unwrap().name = "different".into();
    assert_ne!(a, b);
    let (owner, quota, dropped) = owner(true);
    let a = DocumentSnapshot::from_accounted(owner).unwrap();
    let b = a.clone();
    let usage = quota.usage().unwrap();
    assert_eq!(a, b);
    assert!(a.shares_state(&b));
    assert_eq!(quota.usage().unwrap(), usage);
    drop(a);
    drop(b);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn resident_mask_edit_detaches_shared_owner_and_undo_restores_exact_native_bits() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut mask = photocraft_doc::LayerMask::reveal_all();
    let mut surface = photocraft_raster::Surface::try_with_default_bytes(format, &(-0.0_f64).to_ne_bytes()).unwrap();
    surface.try_write_pixel_f64(-1, 0, &[f64::from_bits(0x3fd0000000000001)]).unwrap();
    mask.surface = Arc::new(surface);
    let original = mask.surface.clone();
    let mut document = Document::new("mask history", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("layer", document.pixel_format());
    let id = layer.id;
    layer.mask = Some(mask);
    document.layers.push(layer);
    let mut session = photocraft_engine::Session::new();
    session.add_document(document, None).unwrap();
    let rect = Rect::new(-1, 0, 0, 1);
    let native = original.try_read_interleaved_with(rect, &Interrupt::NONE).unwrap();
    session
        .edit("mask write", |doc, _| {
            let mask = doc.layer_mut(id).unwrap().mask.as_mut().unwrap();
            Arc::make_mut(&mut mask.surface).try_write_pixel_f64(-1, 0, &[0.75]).unwrap();
            Ok(())
        })
        .unwrap();
    let current = session.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.clone();
    assert!(!Arc::ptr_eq(&original, &current));
    assert_eq!(original.try_read_interleaved_with(rect, &Interrupt::NONE).unwrap(), native);
    assert_eq!(current.default_bytes(), &(-0.0_f64).to_ne_bytes());
    assert!(session.undo());
    let restored = &session.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface;
    assert!(Arc::ptr_eq(&original, restored));
    assert_eq!(restored.try_read_interleaved_with(rect, &Interrupt::NONE).unwrap(), native);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.sample_channel(-1, 0, 0), 0.75);
}
