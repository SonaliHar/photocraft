//! UNRUN paid synthetic CURRENT storage tests. Selection/locks/history/render
//! authority are engine concerns; raw expected clones are external harness data.
use super::super::tests::{fixture, leaf};
use super::*;
use crate::PixelFormat;
use photocraft_raster::Interrupt;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn three() -> AccountedDocument {
    let (original, _) = fixture(PixelFormat::RGBA64F);
    let fee = original.quota().reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = original.document().clone();
    let mut third = doc.layers[1].clone();
    let old = third.id;
    third.id = LayerId::fresh();
    third.name = "untouched".into();
    let mut item = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(old)).unwrap().clone();
    item.item = ItemRef::Layer(third.id);
    item.name = third.name.clone();
    doc.xcf_state.as_mut().unwrap().items.push(item);
    doc.layers.push(third);
    for (index, layer) in doc.layers.iter_mut().enumerate() {
        layer.effects.reference = Some((if index == 0 { -0.0 } else { index as f64 + 0.25 }, -0.0));
    }
    AccountedDocument::from_owned(doc, Arc::new((original.clone(), fee)), original.quota().clone(), &ctl()).unwrap()
}
#[test]
fn two_distinct_targets_change_atomically_and_keep_third_leaf_and_metadata_exact() {
    let original = three();
    let ids = [original.document().layers[1].id, original.document().layers[0].id];
    let moved = original.try_with_imported_layers_translation(&ids, -257, 0, &ctl()).unwrap();
    let mut expected = original.document().clone();
    for index in [0, 1] {
        let id = expected.layers[index].id;
        expected.layers[index].content = moved.document().layers[index].content.clone();
        let r = expected.layers[index].effects.reference.as_mut().unwrap();
        *r = (r.0 - 257.0, r.1 + 0.0);
        expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().raster_bounds =
            Some(leaf(&moved, index).operative_bounds());
    }
    assert_eq!(moved.document(), &expected);
    assert_eq!(moved.document().layers[2], original.document().layers[2]);
    assert!(Arc::ptr_eq(&moved.owner.retention, &original.owner.retention));
    assert_eq!(moved.document().layers[0].effects.reference.unwrap().1.to_bits(), 0f64.to_bits());
    let mut a = [0; 32];
    let mut b = [0; 32];
    for index in [0, 1] {
        let p = leaf(&original, index).operative_bounds();
        leaf(&original, index).checked_view().read_pixel_native_into(p.x0, p.y0, &mut a, &ctl()).unwrap();
        leaf(&moved, index).checked_view().read_pixel_native_into(p.x0 - 257, p.y0, &mut b, &ctl()).unwrap();
        assert_eq!(a, b);
    }
}
#[test]
fn invalid_final_target_duplicate_or_geometry_refusal_does_not_publish_a_partial_move() {
    let original = three();
    let ids = [original.document().layers[0].id, original.document().layers[1].id];
    let q = original.quota();
    let baseline = q.usage().unwrap();
    for targets in [&[][..], &[ids[0], ids[0]][..], &[ids[0], LayerId(0)][..], &[ids[0], LayerId(u64::MAX)][..]] {
        assert!(original.try_with_imported_layers_translation(targets, 1, 1, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    assert!(original.try_with_imported_layers_translation(&ids, i32::MAX, 0, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let mut oversized = vec![ids[0]; 4097];
    assert!(original.validate_imported_layers_translation(&oversized, 0, 0, &ctl()).is_err());
    oversized.clear();
    assert_eq!(original.document().layers[0].effects.reference.unwrap().0.to_bits(), (-0.0f64).to_bits());
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = original.document().clone(); // Paid synthetic target linkage; no link expansion grant.
    doc.layers[1].link_group = Some(17);
    let linked = AccountedDocument::from_owned(doc, Arc::new((original.clone(), fee)), q.clone(), &ctl()).unwrap();
    assert!(linked.try_with_imported_layers_translation(&ids, 1, 0, &ctl()).is_err());
}
#[test]
fn one_model_peak_budget_is_sufficient_for_all_targets_and_zero_needs_no_plan_fee() {
    let original = three();
    let q = original.quota();
    let ids = [original.document().layers[0].id, original.document().layers[1].id];
    let baseline = q.usage().unwrap();
    let full = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(original.try_with_imported_layers_translation(&ids, 0, 0, &ctl()).unwrap().shares_model(&original));
    assert!(original.try_with_imported_layers_translation(&ids, 1, 0, &ctl()).is_err());
    drop(full);
    let logical = selection_census(original.document(), false, None, q, &ctl()).unwrap();
    let owner = (std::mem::size_of::<ModelOwner>() + 128) as u64;
    let model_index = owner + (logical.index - owner) * 2 + 4096;
    let model_payload = logical.payload * 2;
    let plans = 128 + 2 * ids.len() as u64 * std::mem::size_of::<Plan>() as u64;
    let index_allowance = model_index + plans;
    // Constrain the genuine component as well as aggregate resident headroom:
    // Index reservations also contribute to Resident on this actual ledger.
    let index_blocker = q.reserve(QuotaKind::Index, (8 << 20) - baseline.index - index_allowance).unwrap();
    let resident_blocker = q.reserve(QuotaKind::Resident, (64 << 20) - q.usage().unwrap().resident - index_allowance - model_payload).unwrap();
    let moved = original.try_with_imported_layers_translation(&ids, 1, 0, &ctl()).unwrap();
    assert_ne!(leaf(&moved, 0).operative_bounds(), leaf(&original, 0).operative_bounds());
    assert_ne!(leaf(&moved, 1).operative_bounds(), leaf(&original, 1).operative_bounds());
    drop(moved);
    drop(resident_blocker);
    drop(index_blocker);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn paid_plan_and_model_failure_refund_and_singleton_compatibility_remains_exact() {
    let original = three();
    let q = original.quota();
    let baseline = q.usage().unwrap();
    let ids = [original.document().layers[0].id, original.document().layers[1].id];
    let deny = |_: u64| Err(BackingError::Work);
    assert!(original.try_with_imported_layers_translation(&ids, 1, 1, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    let after_reservation = |_: u64| if q.usage().map_err(|_| BackingError::Integrity)?.index > baseline.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(matches!(
        original.try_with_imported_layers_translation(&ids, 1, 1, &ReadControl { interrupt: &Interrupt::NONE, work: &after_reservation }),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    let plan_envelope = 128 + 2 * ids.len() as u64 * std::mem::size_of::<Plan>() as u64;
    let model_phase = std::sync::atomic::AtomicBool::new(false);
    let after_model_reservation = |_: u64| {
        if q.usage().map_err(|_| BackingError::Integrity)?.index > baseline.index + plan_envelope {
            model_phase.store(true, std::sync::atomic::Ordering::Relaxed);
            return Err(BackingError::Cancelled);
        }
        Ok(())
    };
    assert!(matches!(
        original.try_with_imported_layers_translation(&ids, 1, 1, &ReadControl { interrupt: &Interrupt::NONE, work: &after_model_reservation }),
        Err(BackingError::Cancelled)
    ));
    assert!(model_phase.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(q.usage().unwrap(), baseline);
    let single = original.try_with_imported_layer_translation(ids[0], 3, -2, &ctl()).unwrap();
    let bulk = original.try_with_imported_layers_translation(&ids[..1], 3, -2, &ctl()).unwrap();
    assert_eq!(single.document(), bulk.document());
    assert_eq!(single.document().layers[0].effects.reference, bulk.document().layers[0].effects.reference);
}
