//! UNRUN prepared-model history custody and publication regression drafts.
use super::*;
use photocraft_doc::{BackedLayer, ColorMode, Layer, LayerContent, PixelFormat, SampleType};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicBool, Ordering};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 8 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        output.fill(0);
        Ok(())
    }
}
fn fixture(q: &BackingQuota, identity: Option<&AccountedDocument>) -> (AccountedDocument, LayerTarget) {
    let retention = q.reserve(QuotaKind::Resident, 16384).unwrap();
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, Rect::new(0, 0, 1, 1), &[0; 16]).unwrap();
    let raster = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    let layer = Layer::new("initial", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap()));
    let target = LayerTarget { active: Some(layer.id), selected: vec![layer.id] };
    let mut doc = Document::new("prepared custody", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    if let Some(identity) = identity {
        doc.id = identity.document().id;
    }
    doc.layers.push(layer);
    (AccountedDocument::from_owned(doc, Arc::new(retention), q.clone(), &control()).unwrap(), target)
}
fn opacity(owner: &AccountedDocument, target: &LayerTarget, value: f32) -> AccountedDocument {
    owner.try_edit(AccountedEdit::LayerOpacity { layer: target.active.unwrap(), opacity: value }, &control()).unwrap()
}

#[test]
fn prepared_commit_reuses_exact_model_and_records_one_undo_entry() {
    let q = quota();
    let (initial, target) = fixture(&q, None);
    let prepared = opacity(&initial, &target, 0.25);
    let model = prepared.clone();
    let mut history = DocumentHistory::new_accounted(initial.clone(), 8, "Open", &target, &control()).unwrap();
    let current = history.try_commit_prepared_accounted(&initial, prepared, "Current filter", &target, &control()).unwrap();
    assert!(current.accounted().unwrap().shares_model(&model), "history must retain the prepared owner rather than copy its model");
    assert_eq!(history.past_len(), 1);
    let Branch::Accounted(owner) = &history.branch else { panic!() };
    assert_eq!(owner.states.len(), 2);
    assert_eq!(owner.states[owner.cursor].metadata.label, "Current filter");
    assert_eq!(owner.states[owner.cursor].metadata.target, target);
    let undone = history.undo(&current, &control()).unwrap().unwrap();
    assert!(undone.snapshot().accounted().unwrap().shares_model(&initial));
    let restored = history.redo(undone.snapshot(), &control()).unwrap().unwrap();
    assert!(restored.snapshot().accounted().unwrap().shares_model(&model));
}

#[test]
fn exact_prepared_noop_keeps_redo_history_identity_and_ignores_changed_only_limits() {
    let q = quota();
    let (initial, target) = fixture(&q, None);
    let next = opacity(&initial, &target, 0.5);
    let mut history = DocumentHistory::new_accounted(initial.clone(), 8, "Open", &target, &control()).unwrap();
    let current = history.try_commit_prepared_accounted(&initial, next, "Changed", &target, &control()).unwrap();
    let undone = history.undo(&current, &control()).unwrap().unwrap();
    let expected = undone.snapshot().accounted().unwrap();
    let before = history.clone();
    let usage = q.usage().unwrap();
    history.max_states = 0;
    history.max_bytes = 1;
    let same = history.try_commit_prepared_accounted(expected, expected.clone(), "ignored\0label", &target, &control()).unwrap();
    assert!(same.shares_state(undone.snapshot()));
    assert!(history.shares_accounted_history(&before));
    assert_eq!(q.usage().unwrap(), usage);
    history.max_states = before.max_states;
    history.max_bytes = before.max_bytes;
    assert!(history.redo(&same, &control()).unwrap().unwrap().snapshot().shares_state(&current));
}

#[test]
fn mismatched_current_document_or_actual_ledger_never_changes_history() {
    let q = quota();
    let (initial, target) = fixture(&q, None);
    let prepared = opacity(&initial, &target, 0.5);
    let mut history = DocumentHistory::new_accounted(initial.clone(), 8, "Open", &target, &control()).unwrap();
    let before = history.clone();
    assert!(history.try_commit_prepared_accounted(&prepared, prepared.clone(), "stale", &target, &control()).is_err());
    let (different_id, _) = fixture(&q, None);
    assert!(history.try_commit_prepared_accounted(&initial, different_id, "wrong ID", &target, &control()).is_err());
    let foreign = quota();
    let (same_id_foreign_quota, _) = fixture(&foreign, Some(&initial));
    assert!(history.try_commit_prepared_accounted(&initial, same_id_foreign_quota, "wrong quota", &target, &control()).is_err());
    assert!(history.shares_accounted_history(&before));
}

#[test]
fn changed_prepared_commit_obeys_history_limits_and_truncates_only_redo() {
    let q = quota();
    let (initial, target) = fixture(&q, None);
    let second = opacity(&initial, &target, 0.5);
    let third = opacity(&second, &target, 0.25);
    let mut history = DocumentHistory::new_accounted(initial.clone(), 1, "Open", &target, &control()).unwrap();
    history.try_commit_prepared_accounted(&initial, second.clone(), "Second", &target, &control()).unwrap();
    let current = history.try_commit_prepared_accounted(&second, third, "Third", &target, &control()).unwrap();
    assert_eq!(history.past_len(), 1);
    let undone = history.undo(&current, &control()).unwrap().unwrap();
    assert!(undone.snapshot().accounted().unwrap().shares_model(&second));
    let replacement = opacity(&second, &target, 0.75);
    let current = history.try_commit_prepared_accounted(&second, replacement, "New branch", &target, &control()).unwrap();
    assert!(history.redo(&current, &control()).unwrap().is_none());
    let before = history.clone();
    history.max_bytes = 1;
    let different = opacity(current.accounted().unwrap(), &target, 0.125);
    assert!(history.try_commit_prepared_accounted(current.accounted().unwrap(), different, "Unsupported byte budget", &target, &control()).is_err());
    assert!(history.shares_accounted_history(&before));
}

#[test]
fn cancellation_after_metadata_admission_and_quota_denial_preserve_all_history_owners() {
    let q = quota();
    let (initial, target) = fixture(&q, None);
    let prepared = opacity(&initial, &target, 0.25);
    let mut history = DocumentHistory::new_accounted(initial.clone(), 8, "Open", &target, &control()).unwrap();
    let before = history.clone();
    let _keep_prepared = prepared.clone();
    let usage = q.usage().unwrap();
    let observed_paid_copy = AtomicBool::new(false);
    let charge = |_: u64| {
        if q.usage().map_err(|_| BackingError::Quota)?.index > usage.index {
            observed_paid_copy.store(true, Ordering::Relaxed);
            Err(BackingError::Cancelled)
        } else {
            Ok(())
        }
    };
    let late = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    assert!(history.try_commit_prepared_accounted(&initial, prepared.clone(), "Late cancellation", &target, &late).is_err());
    assert!(observed_paid_copy.load(Ordering::Relaxed));
    assert!(history.shares_accounted_history(&before));
    assert_eq!(q.usage().unwrap(), usage);
    let blocker = q.reserve(QuotaKind::Index, (8 << 20) - usage.index).unwrap();
    assert!(history.try_commit_prepared_accounted(&initial, prepared, "Quota denial", &target, &control()).is_err());
    drop(blocker);
    assert!(history.shares_accounted_history(&before));
    assert_eq!(q.usage().unwrap(), usage);
}
