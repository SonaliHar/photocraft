use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    BackedLayer, Document, Layer, LayerContent,
    accounted_document::{AccountedDocument, AccountedEdit},
};
use photocraft_engine::{document_history::DocumentHistory, document_snapshot::DocumentSnapshot};
use photocraft_geom::{Rect, Size};
use photocraft_ops::LayerTarget;
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
fn charged_history_preserves_native_target_and_owned_weak_lifetime() {
    let (doc, q, dropped) = owner(false);
    let id = doc.document().layers[0].id;
    let target = LayerTarget { active: Some(id), selected: vec![id] };
    let initial = DocumentSnapshot::from_accounted(doc.clone()).unwrap();
    let mut history = DocumentHistory::new_accounted(doc, 2, "Open", &target, &control()).unwrap();
    let before = q.usage().unwrap();
    let pixels = f64::from_bits(0x3fd0000000000001).to_ne_bytes().repeat(4);
    let changed = history
        .try_commit_accounted(AccountedEdit::NativePixels { layer: id, rect: Rect::new(-1, 1, 0, 2), bytes: &pixels }, "Paint", &target, &control())
        .unwrap();
    assert_eq!(history.past_len(), 1);
    assert_eq!(history.undo_label(), Some("Paint"));
    assert!(history.state(0).unwrap().unwrap().shares_state(&initial));
    let labels = history.entries(&control()).unwrap();
    assert_eq!(labels.values(), &["Open", "Paint"]);
    assert!(q.usage().unwrap().index > before.index);
    drop(labels);
    let pre_undo = q.usage().unwrap();
    let change = history.undo(&changed, &control()).unwrap().unwrap();
    assert_eq!(change.target(), &target);
    assert!(change.snapshot().shares_state(&initial));
    assert!(q.usage().unwrap().index > pre_undo.index);
    let (restored, target_owner) = change.into_parts();
    let redo = history.redo(&restored, &control()).unwrap().unwrap();
    assert!(redo.snapshot().shares_state(&changed));
    assert_eq!(history.redo_labels().count(), 0);
    drop(redo);
    drop(target_owner);
    drop(restored);
    let weak = changed.downgrade();
    drop(initial);
    drop(changed);
    drop(history);
    assert!(weak.upgrade().is_none());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    drop(weak);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn failed_commit_and_travel_do_not_mutate_redo_or_target_and_limits_refuse() {
    let (doc, q, _) = owner(false);
    let id = doc.document().layers[0].id;
    let target = LayerTarget { active: Some(id), selected: vec![id] };
    let mut h = DocumentHistory::new_accounted(doc, 2, "Open", &target, &control()).unwrap();
    let a = h.try_commit_accounted(AccountedEdit::LayerVisibility { layer: id, visible: false }, "Hide", &target, &control()).unwrap();
    let original = h.undo(&a, &control()).unwrap().unwrap();
    let current = original.snapshot().clone();
    drop(original);
    let before = q.usage().unwrap();
    let fail = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(h.try_commit_accounted(AccountedEdit::LayerVisibility { layer: id, visible: true }, "Never", &target, &fail), Err(BackingError::Work)));
    assert!(h.can_redo());
    assert_eq!(h.redo_label(), Some("Hide"));
    assert!(h.current().unwrap().shares_state(&current));
    assert_eq!(q.usage().unwrap(), before);
    assert!(h.redo(&current, &fail).is_err());
    assert!(h.can_redo());
    assert!(h.record("unsupported", current.clone(), target.clone(), &control()).is_err());
    h.max_bytes = 1;
    assert!(matches!(h.trim(&current, &control()), Err(BackingError::Unsupported)));
    assert!(h.can_redo());
    h.max_bytes = 0;
    let huge = "x".repeat(4097);
    assert!(h.set_current_label(&huge, &control()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let usage = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (1 << 20) - usage.index).unwrap();
    assert!(matches!(h.redo(&current, &control()), Err(BackingError::Quota)));
    assert!(h.can_redo());
    drop(blocker);
    assert!(h.pixel_bytes(&current).is_err());
    assert!(h.unique_bytes(&current).is_err());
    drop(a);
    drop(current);
    drop(h);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn bounded_history_replaces_oldest_and_resident_wrapper_keeps_legacy_order() {
    let (doc, q, _) = owner(false);
    let id = doc.document().layers[0].id;
    let target = LayerTarget::default();
    let mut h = DocumentHistory::new_accounted(doc, 2, "Open", &target, &control()).unwrap();
    for i in 0..5 {
        h.try_commit_accounted(AccountedEdit::LayerOpacity { layer: id, opacity: (i + 1) as f32 / 8. }, "Opacity", &target, &control()).unwrap();
    }
    assert_eq!(h.past_len(), 2);
    assert_eq!(h.entries(&control()).unwrap().values().len(), 3);
    assert!(h.purge_last(&control()).unwrap());
    assert_eq!(h.past_len(), 1);
    h.clear(&control()).unwrap();
    assert_eq!(h.past_len(), 0);
    drop(h);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut resident = DocumentHistory::new_resident(2);
    let old = DocumentSnapshot::from_resident(Arc::new(Document::new("a", Size::new(1, 1), ColorMode::Rgb, SampleType::U8))).unwrap();
    let mut changed = old.clone();
    changed.try_resident_mut().unwrap().name = "b".into();
    resident.record("Rename", old.clone(), target, &control()).unwrap();
    let undo = resident.undo(&changed, &control()).unwrap().unwrap();
    assert!(undo.snapshot().shares_state(&old));
    assert_eq!(resident.redo_label(), Some("Rename"));
    assert_eq!(resident.entries(&control()).unwrap().values(), &["Open"]);
    let redo = resident.redo(undo.snapshot(), &control()).unwrap().unwrap();
    assert!(redo.snapshot().shares_state(&changed));
}
#[test]
fn cloned_history_is_shared_until_charged_cow_and_late_cancel_is_atomic() {
    let (doc, q, _) = owner(false);
    let id = doc.document().layers[0].id;
    let target = LayerTarget { active: Some(id), selected: vec![id] };
    let mut original = DocumentHistory::new_accounted(doc, 3, "Open", &target, &control()).unwrap();
    let mut copy = original.clone();
    assert!(copy.shares_accounted_history(&original));
    let unchanged = q.usage().unwrap();
    original.set_current_layers_borrowed(&target, &control()).unwrap();
    let no_op = original.try_commit_accounted(AccountedEdit::LayerVisibility { layer: id, visible: true }, "Ignored", &target, &control()).unwrap();
    assert!(copy.shares_accounted_history(&original));
    assert_eq!(q.usage().unwrap(), unchanged);
    drop(no_op);
    let count = AtomicUsize::new(0);
    let work = |_| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    let measured = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    copy.try_commit_accounted(AccountedEdit::LayerVisibility { layer: id, visible: false }, "Hide", &target, &measured).unwrap();
    assert_eq!(original.past_len(), 0);
    assert_eq!(copy.past_len(), 1);
    assert!(!copy.shares_accounted_history(&original));
    let calls = count.load(Ordering::SeqCst);
    assert!(calls > 3);
    drop(copy);
    let before = q.usage().unwrap();
    let history_before = original.clone();
    let counter = AtomicUsize::new(0);
    let flag = std::sync::atomic::AtomicBool::new(false);
    let work = |_| {
        if counter.fetch_add(1, Ordering::SeqCst) + 1 == calls - 1 {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let cancelled = || flag.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let ctl = ReadControl { interrupt: &interrupt, work: &work };
    assert!(matches!(
        original.try_commit_accounted(AccountedEdit::LayerVisibility { layer: id, visible: false }, "Hide", &target, &ctl),
        Err(BackingError::Cancelled)
    ));
    assert!(original.shares_accounted_history(&history_before));
    assert_eq!(original.past_len(), 0);
    assert_eq!(q.usage().unwrap(), before);
    drop(history_before);
    drop(original);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn legacy_history_with_unaccounted_backing_refuses_before_history_publication() {
    let (doc, q, _) = owner(false);
    let mut legacy = photocraft_ops::History::new(2);
    legacy.record("forged", Arc::new(doc.document().clone()), LayerTarget::default());
    let mut h = DocumentHistory::resident(legacy);
    let current = DocumentSnapshot::from_resident(Arc::new(Document::new("resident", Size::new(1, 1), ColorMode::Rgb, SampleType::U8))).unwrap();
    assert!(matches!(h.state(0), Err(BackingError::Unsupported)));
    assert!(matches!(h.undo(&current, &control()), Err(BackingError::Unsupported)));
    assert_eq!(h.past_len(), 1);
    assert!(!h.can_redo());
    assert_eq!(h.undo_label(), Some("forged"));
    drop(h);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn resident_inspection_keeps_open_and_current_history_labels() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
    session
        .edit("named metadata", |doc, _| {
            doc.name = "changed".into();
            Ok(())
        })
        .unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.history.entry_labels().collect::<Vec<_>>(), vec!["Open", "named metadata"]);
    assert_eq!(photocraft_engine::inspect::document(state).unwrap()["history"], serde_json::json!(["Open", "named metadata"]));
}

#[test]
fn accounted_preferences_preserve_prepaid_history_and_inspect_refuses_before_clone() {
    let (owner, quota, dropped) = owner(true);
    let mut session = photocraft_engine::Session::new();
    session.add_accounted_document(owner, None, &control()).unwrap();
    let before = quota.usage().unwrap();
    let original = session.prefs().performance.history_states;
    let cap = session.active().unwrap().history.max_states;
    let rev = session.prefs.rev();
    assert!(session.edit_prefs(|p| p.performance.history_states = original + 1).is_err());
    assert_eq!(session.prefs().performance.history_states, original);
    assert_eq!(session.active().unwrap().history.max_states, cap);
    assert_eq!(session.prefs.rev(), rev);
    assert_eq!(quota.usage().unwrap(), before);
    session.edit_prefs(|p| p.general.beep_when_done = true).unwrap();
    assert!(session.prefs().general.beep_when_done);
    assert_eq!(session.active().unwrap().history.max_states, cap);
    assert_eq!(session.active().unwrap().history.max_bytes, 0);
    assert!(photocraft_engine::inspect::session(&session).is_err());
    assert!(photocraft_engine::inspect::document(session.active().unwrap()).is_err());
    assert_eq!(quota.usage().unwrap(), before);
    drop(session);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
