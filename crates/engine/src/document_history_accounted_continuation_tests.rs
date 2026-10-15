//! UNRUN typed accounted metadata continuation regression drafts.
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
    let raster = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    doc.layers.push(Layer::new("second", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap())));
    doc.source_color_context = Some(photocraft_doc::SourceColorContext { original_icc: Some(Arc::new(vec![1; 64])), source_precision: 675 });
    let held = q.reserve(QuotaKind::Resident, 16384).unwrap();
    (AccountedDocument::from_owned(doc, Arc::new(held), q.clone(), &control()).unwrap(), target, q)
}

fn edit(id: photocraft_doc::LayerId, opacity: f32) -> AccountedEdit<'static> {
    AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: Some(opacity) }
}
fn continue_edit(history: &mut DocumentHistory, edit: AccountedEdit<'_>, target: &LayerTarget, c: &ReadControl<'_>) -> Result<DocumentSnapshot, BackingError> {
    let expected_history = history.clone();
    let expected = history.current()?.accounted().ok_or(BackingError::Integrity)?.clone();
    history.try_continue_accounted_metadata(&expected_history, &expected, edit, "metadata", target, c)
}
#[test]
fn several_frames_and_explicit_different_layer_keep_one_undo_anchor() {
    let (initial, target, _) = fixture();
    let first = target.active.unwrap();
    let second = initial.document().layers[1].id;
    let mut history = DocumentHistory::new_accounted(initial.clone(), 2, "open", &target, &control()).unwrap();
    history.try_commit_accounted(edit(first, 0.75), "metadata", &target, &control()).unwrap();
    continue_edit(&mut history, edit(first, 0.5), &target, &control()).unwrap();
    let last = continue_edit(&mut history, edit(second, 0.25), &target, &control()).unwrap();
    let Branch::Accounted(h) = &history.branch else { panic!() };
    assert_eq!(h.cursor, 1);
    assert_eq!(h.states.len(), 2);
    assert_eq!(last.layers[0].opacity, 0.5);
    assert_eq!(last.layers[1].opacity, 0.25);
    let old = history.undo(&last, &control()).unwrap().unwrap();
    assert!(old.snapshot().accounted().unwrap().shares_model(&initial));
    let restored = history.redo(old.snapshot(), &control()).unwrap().unwrap();
    assert!(restored.snapshot().shares_state(&last));
}
#[test]
fn exact_noop_keeps_initial_or_redo_history_but_changed_continuation_refuses() {
    let (initial, target, _) = fixture();
    let id = target.active.unwrap();
    let mut history = DocumentHistory::new_accounted(initial.clone(), 4, "open", &target, &control()).unwrap();
    let before = history.clone();
    continue_edit(&mut history, edit(id, 1.0), &target, &control()).unwrap();
    assert!(history.shares_accounted_history(&before));
    assert!(continue_edit(&mut history, edit(id, 0.5), &target, &control()).is_err());
    let changed = history.try_commit_accounted(edit(id, 0.5), "metadata", &target, &control()).unwrap();
    history.undo(&changed, &control()).unwrap();
    let before = history.clone();
    continue_edit(&mut history, edit(id, 1.0), &target, &control()).unwrap();
    assert!(history.shares_accounted_history(&before));
    assert!(history.can_redo());
    assert!(continue_edit(&mut history, edit(id, 0.25), &target, &control()).is_err());
    assert!(history.shares_accounted_history(&before));
}
#[test]
fn request_history_limit_or_target_mismatch_refuses_and_paid_focus_rebase_is_allowed() {
    let (initial, target, _) = fixture();
    let id = target.active.unwrap();
    let second = initial.document().layers[1].id;
    let mut history = DocumentHistory::new_accounted(initial, 4, "open", &target, &control()).unwrap();
    let current = history.try_commit_accounted(edit(id, 0.5), "metadata", &target, &control()).unwrap();
    let expected = history.clone();
    history.max_states = 3;
    assert!(history.try_continue_accounted_metadata(&expected, current.accounted().unwrap(), edit(id, 0.25), "metadata", &target, &control()).is_err());
    history.max_states = 4;
    let focused = LayerTarget { active: Some(second), selected: vec![second] };
    assert!(continue_edit(&mut history, edit(id, 0.25), &focused, &control()).is_err());
    history.set_current_layers_borrowed(&focused, &control()).unwrap();
    let request = history.clone();
    let owner = history.current().unwrap().accounted().unwrap().clone();
    assert!(history.try_continue_accounted_metadata(&expected, &owner, edit(id, 0.25), "metadata", &focused, &control()).is_err());
    history.try_continue_accounted_metadata(&request, &owner, edit(id, 0.25), "metadata", &focused, &control()).unwrap();
}
#[test]
fn cancellation_and_slot_quota_refusal_preserve_latest_frame_and_quota() {
    let (initial, target, q) = fixture();
    let id = target.active.unwrap();
    let mut history = DocumentHistory::new_accounted(initial, 4, "open", &target, &control()).unwrap();
    history.try_commit_accounted(edit(id, 0.5), "metadata", &target, &control()).unwrap();
    let before = history.clone();
    let usage = q.usage().unwrap();
    let late = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(continue_edit(&mut history, edit(id, 0.25), &target, &late).is_err());
    assert_eq!(q.usage().unwrap(), usage);
    let held = q.reserve(QuotaKind::Index, (8 << 20) - usage.index).unwrap();
    assert!(continue_edit(&mut history, edit(id, 0.25), &target, &control()).is_err());
    drop(held);
    assert_eq!(q.usage().unwrap(), usage);
    assert!(history.shares_accounted_history(&before));
}

#[test]
fn continuation_preserves_first_changed_label_across_different_frame_labels() {
    let (initial, target, _) = fixture();
    let id = target.active.unwrap();
    let mut history = DocumentHistory::new_accounted(initial, 4, "open", &target, &control()).unwrap();
    history.try_commit_accounted(edit(id, 0.75), "Layer Properties", &target, &control()).unwrap();
    let expected = history.clone();
    let current = history.current().unwrap().accounted().unwrap().clone();
    let last = history
        .try_continue_accounted_metadata(
            &expected,
            &current,
            AccountedEdit::LayerMetadata { layer: id, name: None, visible: Some(false), opacity: None },
            "Layer Visibility",
            &target,
            &control(),
        )
        .unwrap();
    assert_eq!(history.undo_label(), Some("Layer Properties"));
    let old = history.undo(&last, &control()).unwrap().unwrap();
    assert_eq!(history.redo_label(), Some("Layer Properties"));
    let restored = history.redo(old.snapshot(), &control()).unwrap().unwrap();
    assert!(!restored.snapshot().layers[0].visible);
    assert_eq!(history.undo_label(), Some("Layer Properties"));
}
