//! UNRUN paid current-entry replacement regression drafts.
use super::*;
use photocraft_doc::{BackedLayer, ColorMode, Layer, LayerContent, PixelFormat, SampleType};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        out.fill(0);
        Ok(())
    }
}
fn fixture() -> (AccountedDocument, LayerTarget, BackingQuota) {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 8 << 20, scratch: 8 << 20 });
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, Rect::new(0, 0, 1, 1), &[0; 16]).unwrap();
    let raster = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    let layer = Layer::new("initial", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap()));
    let target = LayerTarget { active: Some(layer.id), selected: Vec::new() };
    let mut doc = Document::new("source", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    doc.layers.push(layer);
    let held = q.reserve(QuotaKind::Resident, 16384).unwrap();
    (AccountedDocument::from_owned(doc, Arc::new(held), q.clone(), &control()).unwrap(), target, q)
}
#[test]
fn replacement_preserves_cursor_labels_targets_undo_redo_and_noop_identity() {
    let (initial, target, _) = fixture();
    let id = target.active.unwrap();
    let mut history = DocumentHistory::new_accounted(initial.clone(), 8, "initial", &target, &control()).unwrap();
    let second = history.try_commit_accounted(AccountedEdit::LayerName { layer: id, name: "second" }, "rename", &target, &control()).unwrap();
    let third = history.try_commit_accounted(AccountedEdit::LayerOpacity { layer: id, opacity: 0.25 }, "opacity", &target, &control()).unwrap();
    let at_second = history.undo(&third, &control()).unwrap().unwrap();
    let expected = at_second.snapshot().accounted().unwrap();
    assert!(expected.shares_model(second.accounted().unwrap()));
    let before = history.clone();
    let same = history.try_replace_accounted_current(expected, expected.clone(), &control()).unwrap();
    assert!(history.shares_accounted_history(&before));
    assert!(same.shares_state(&second));
    let Branch::Accounted(h) = &history.branch else { panic!() };
    let old_meta = h.states[h.cursor].metadata.clone();
    let old_cursor = h.cursor;
    let old_len = h.states.len();
    let prepared = expected.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).unwrap();
    let current = history.try_replace_accounted_current(expected, prepared, &control()).unwrap();
    let Branch::Accounted(h) = &history.branch else { panic!() };
    assert_eq!(h.cursor, old_cursor);
    assert_eq!(h.states.len(), old_len);
    assert!(Arc::ptr_eq(&h.states[h.cursor].metadata, &old_meta));
    let old = history.undo(&current, &control()).unwrap().unwrap();
    assert!(old.snapshot().accounted().unwrap().shares_model(&initial));
    let restored = history.redo(old.snapshot(), &control()).unwrap().unwrap();
    assert!(!restored.snapshot().layers[0].visible);
    let redo = history.redo(restored.snapshot(), &control()).unwrap().unwrap();
    assert!(redo.snapshot().shares_state(&third));
}
#[test]
fn wrong_current_or_ledger_and_cancel_or_quota_leave_history_unchanged() {
    let (initial, target, q) = fixture();
    let id = target.active.unwrap();
    let mut history = DocumentHistory::new_accounted(initial.clone(), 8, "initial", &target, &control()).unwrap();
    let prepared = initial.try_edit(AccountedEdit::LayerName { layer: id, name: "prepared" }, &control()).unwrap();
    let before = history.clone();
    assert!(history.try_replace_accounted_current(&prepared, prepared.clone(), &control()).is_err());
    let (other, _, _) = fixture();
    assert!(history.try_replace_accounted_current(&initial, other, &control()).is_err());
    let calls = std::cell::Cell::new(0);
    let late = ReadControl {
        interrupt: &Interrupt::NONE,
        work: &|_| {
            calls.set(calls.get() + 1);
            if calls.get() > 3 { Err(BackingError::Cancelled) } else { Ok(()) }
        },
    };
    let usage = q.usage().unwrap();
    assert!(history.try_replace_accounted_current(&initial, prepared.clone(), &late).is_err());
    assert_eq!(q.usage().unwrap(), usage);
    let blocker = q.reserve(QuotaKind::Index, (8 << 20) - usage.index).unwrap();
    assert!(history.try_replace_accounted_current(&initial, prepared.clone(), &control()).is_err());
    drop(blocker);
    assert!(history.shares_accounted_history(&before));
    assert_eq!(q.usage().unwrap(), usage);
}
