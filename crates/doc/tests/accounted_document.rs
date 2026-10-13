use photocraft_doc::{
    BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, Size,
    accounted_document::{AccountedDocument, AccountedEdit, AccountedHistory},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: AtomicU64,
    fail: AtomicBool,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        let n = self.descriptor.format().bytes_per_pixel() as u64;
        for (i, value) in output.iter_mut().enumerate() {
            *value = self.descriptor.default_bytes()[((offset + i as u64) % n) as usize];
        }
        Ok(())
    }
}
struct Retention {
    source: Weak<Store>,
    dropped_after_source: Arc<AtomicBool>,
    _lease: QuotaLease,
}
impl Drop for Retention {
    fn drop(&mut self) {
        self.dropped_after_source.store(self.source.upgrade().is_none(), Ordering::Relaxed);
    }
}
fn fixture() -> (Document, Arc<Retention>, Arc<Store>, BackingQuota, Arc<AtomicBool>) {
    let q = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 0, index: 1 << 20 });
    // Prepaid fixture model envelope; the transfer moves this existing ownership.
    let lease = q.reserve(QuotaKind::Resident, 16 << 10).unwrap();
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let bytes: Vec<_> = [0.25_f64, -0.0, 0.5, 1.0].into_iter().flat_map(f64::to_le_bytes).collect();
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(-1, -1, 3, 2), &bytes).unwrap();
    let store = Arc::new(Store { descriptor, reads: AtomicU64::new(0), fail: AtomicBool::new(false) });
    let backing = BackedRaster::new(store.clone(), q.clone(), 1, &control()).unwrap();
    let mut doc = Document::new("retained native model", Size::new(3, 2), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(Layer::new("current layer", LayerContent::BackedRaster(BackedLayer::new(backing, descriptor, &control()).unwrap())));
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Arc::new(Retention { source: Arc::downgrade(&store), dropped_after_source: dropped.clone(), _lease: lease });
    (doc, guard, store, q, dropped)
}
fn pixel(doc: &Document) -> Vec<u8> {
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("backing lost") };
    let mut bytes = vec![0; backed.format().bytes_per_pixel()];
    backed.checked_view().read_pixel_native_into(0, 0, &mut bytes, &control()).unwrap();
    bytes
}
#[test]
fn adopted_owner_clone_shares_charge_and_drops_payload_before_retention_guard() {
    let (doc, guard, store, q, dropped) = fixture();
    let weak = Arc::downgrade(&store);
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    let snapshot = owner.clone();
    let weak_model = owner.downgrade();
    let same_weak = snapshot.downgrade();
    assert!(weak_model.ptr_eq(&same_weak));
    assert_eq!(weak_model.upgrade().unwrap().model_identity(), owner.model_identity());
    drop(same_weak);
    assert!(owner.shares_model(&snapshot));
    assert_eq!(q.usage().unwrap(), before);
    drop(store);
    drop(owner);
    assert!(weak.upgrade().is_some());
    drop(snapshot);
    assert!(weak.upgrade().is_none());
    assert!(dropped.load(Ordering::Relaxed));
    assert!(weak_model.upgrade().is_none());
    // Weak Arc control blocks retain their own allocation charge, but not
    // source/archive/model payloads. Releasing the last weak returns that too.
    assert!(q.usage().unwrap().index > 0);
    drop(weak_model);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn charged_history_edits_preserve_native_bits_undo_redo_and_failed_redo_tail() {
    let (doc, guard, store, q, dropped) = fixture();
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let original = pixel(owner.document());
    let mut history = AccountedHistory::new(owner.clone(), 5, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.25 }, &control()).unwrap();
    assert!(!history.current().unwrap().shares_model(&owner));
    assert_eq!(owner.document().layers[0].opacity, 1.0);
    assert_eq!(pixel(history.current().unwrap().document()), original);
    let edited: Vec<_> = [0.25000000000000006_f64, -0.0, 0.75, 0.5].into_iter().flat_map(f64::to_le_bytes).collect();
    history.try_commit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &edited }, &control()).unwrap();
    assert_eq!(pixel(history.current().unwrap().document()), edited);
    assert_eq!(pixel(owner.document()), original);
    assert!(history.undo(&control()).unwrap());
    let before = q.usage().unwrap();
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert_eq!(history.try_commit(AccountedEdit::LayerName { layer: id, name: "refused" }, &work), Err(BackingError::Work));
    assert_eq!(history.len(), 3);
    assert_eq!(q.usage().unwrap(), before);
    assert!(history.redo(&control()).unwrap());
    assert_eq!(pixel(history.current().unwrap().document()), edited);
    assert!(history.undo(&control()).unwrap());
    history.try_commit(AccountedEdit::LayerName { layer: id, name: "current accepted" }, &control()).unwrap();
    assert!(!history.redo(&control()).unwrap());
    assert_eq!(history.current().unwrap().document().layers[0].name, "current accepted");
    assert_eq!(owner.document().layers[0].name, "current layer");
    drop(store);
    drop(history);
    drop(owner);
    assert!(dropped.load(Ordering::Relaxed));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn unknown_copy_collections_and_native_read_errors_never_publish_edits() {
    let (mut doc, guard, store, q, _) = fixture();
    let id = doc.layers[0].id;
    doc.notes.push(Default::default());
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()), Err(BackingError::Unsupported)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop((owner, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let (doc, guard, store, q, _) = fixture();
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    store.fail.store(true, Ordering::Relaxed);
    assert!(matches!(
        owner.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 32] }, &control()),
        Err(BackingError::Io)
    ));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().layers[0].opacity, 1.0);
    drop((owner, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn adoption_validation_failure_drops_model_before_refunding_guard() {
    let (doc, guard, store, q, dropped) = fixture();
    drop(store);
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(AccountedDocument::from_owned(doc, guard, q.clone(), &work), Err(BackingError::Work)));
    assert!(dropped.load(Ordering::Relaxed));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn denied_model_copy_and_bounded_history_slots_preserve_original_without_io() {
    let (doc, guard, store, q, _) = fixture();
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    let pressure = q.reserve(QuotaKind::Resident, (32 << 20) - before.resident - 1).unwrap();
    let charged = q.usage().unwrap();
    assert!(matches!(owner.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.1 }, &control()), Err(BackingError::Quota)));
    assert_eq!(owner.document().layers[0].opacity, 1.0);
    assert_eq!(q.usage().unwrap(), charged);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(pressure);
    let mut history = AccountedHistory::new(owner.clone(), 1, &control()).unwrap();
    let before = q.usage().unwrap();
    history.try_commit(AccountedEdit::LayerVisibility { layer: id, visible: true }, &control()).unwrap();
    assert!(history.current().unwrap().shares_model(&owner));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(history.try_commit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()), Err(BackingError::Quota));
    assert_eq!(q.usage().unwrap(), before);
    assert!(history.current().unwrap().document().layers[0].visible);
    assert!(!history.undo(&control()).unwrap());
    assert!(!history.redo(&control()).unwrap());
    drop((history, owner, store));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
