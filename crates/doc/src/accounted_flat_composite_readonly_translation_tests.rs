//! UNRUN custody/atomic-copy regressions, not source or rendering eligibility.
//! Expected metadata clones and raw comparison buffers are external harness data.
use super::super::tests::{ctl, fixture, leaf, owned, quota};
use super::*;
use crate::{ColorMode, Layer, SampleType};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn three(sample: SampleType) -> (AccountedDocument, Arc<AtomicU64>) {
    let q = quota();
    let (mut doc, guard, reads) = fixture(&q, ColorMode::Rgb, sample);
    let mut third = Layer::new("unrelated", doc.layers[0].content.clone());
    third.import_compositing = doc.layers[0].import_compositing.clone();
    let mut row = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(doc.layers[0].id)).unwrap().clone();
    row.item = ItemRef::Layer(third.id);
    row.name = third.name.clone();
    row.source_tattoo = None;
    doc.layers[0].effects.reference = Some((-0., 12.5));
    doc.layers[1].effects.reference = Some((7.25, -0.));
    doc.layers.push(third);
    doc.xcf_state.as_mut().unwrap().items.insert(1, row);
    (AccountedDocument::from_owned_readonly_flat_composite(doc, guard, q, &ctl()).unwrap(), reads)
}
fn raw(layer: &BackedLayer, x: i32, y: i32) -> Vec<u8> {
    let mut out = vec![0; layer.format().bytes_per_pixel()];
    layer.checked_view().read_pixel_native_into(x, y, &mut out, &ctl()).unwrap();
    out
}

#[test]
#[ignore = "UNRUN: two negative/nonaligned native views and unrelated leaf preserve all five depths/raw defaults/canonical policy without materialization"]
fn bulk_moves_only_native_views_bounds_and_reference() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let (owner, reads) = three(sample);
        let ids = [owner.document().layers[1].id, owner.document().layers[0].id];
        let moved = owner.try_with_flat_composite_layers_translation(&ids, -257, 259, &ctl()).unwrap();
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert!(moved.is_readonly_flat_composite_storage());
        assert!(Arc::ptr_eq(&moved.owner.retention, &owner.owner.retention));
        assert!(Arc::ptr_eq(moved.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
        let mut expected = owner.document().clone();
        for index in 0..2 {
            let old = leaf(&owner, index);
            let new = leaf(&moved, index);
            let b = old.operative_bounds();
            let translated = Rect::new(b.x0 - 257, b.y0 + 259, b.x1 - 257, b.y1 + 259);
            assert_eq!(new.operative_bounds(), translated);
            assert_eq!(new.format(), old.format());
            assert_eq!(new.checked_view().default_bytes(), old.checked_view().default_bytes());
            for (x, y) in [(b.x0, b.y0), (b.x0 + 255, b.y0 + 1), (b.x1 - 1, b.y1 - 1)] {
                if b.contains(x, y) {
                    assert_eq!(raw(old, x, y), raw(new, x - 257, y + 259));
                }
            }
            expected.layers[index].content = moved.document().layers[index].content.clone();
            let reference = expected.layers[index].effects.reference.as_mut().unwrap();
            *reference = (reference.0 + -257f64, reference.1 + 259f64);
            expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(ids[1 - index])).unwrap().raster_bounds = Some(translated);
        }
        assert_eq!(&expected, moved.document());
        assert_eq!(owner.document().layers[2], moved.document().layers[2]);
        assert_eq!(raw(leaf(&owner, 2), -3, -2), raw(leaf(&moved, 2), -3, -2));
    }
}

#[test]
#[ignore = "UNRUN: zero shares actual owner at full quota after gates; invalid IDs/overflow/links refuse atomically before copy"]
fn guarded_zero_and_all_target_refusals() {
    let q = quota();
    let (owner, reads) = owned(&q, ColorMode::Rgb, SampleType::F32);
    let ids = [owner.document().layers[0].id, owner.document().layers[1].id];
    let before = q.usage().unwrap();
    for (targets, dx) in [(vec![], 0), (vec![ids[0], ids[0]], 0), (vec![ids[0], LayerId(u64::MAX)], 1), (ids.to_vec(), i32::MAX)] {
        assert!(owner.try_with_flat_composite_layers_translation(&targets, dx, 0, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let index = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let used = q.usage().unwrap();
    let resident = q.reserve(QuotaKind::Resident, (64 << 20) - used.resident).unwrap();
    let noop = owner.try_with_flat_composite_layers_translation(&ids, 0, 0, &ctl()).unwrap();
    assert!(Arc::ptr_eq(&noop.owner, &owner.owner));
    assert!(owner.try_with_flat_composite_layers_translation(&ids, 1, 0, &ctl()).is_err());
    drop(noop);
    drop(resident);
    drop(index);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let (mut bad, _) = owned(&q, ColorMode::Rgb, SampleType::F32);
    let bad_id = bad.document().layers[0].id;
    Arc::get_mut(&mut bad.owner).unwrap().document.layers[0].link_group = Some(1);
    assert!(bad.try_with_flat_composite_layers_translation(&[bad_id], 0, 0, &ctl()).is_err());
}

#[test]
#[ignore = "UNRUN: repeated view moves retain direct source root, release predecessor model and preserve exact upstream inert reference arithmetic"]
fn repeated_views_do_not_retain_previous_models() {
    let (owner, _) = three(SampleType::F64);
    let id = owner.document().layers[0].id;
    let root = owner.owner.retention.clone();
    let previous = Arc::downgrade(&owner.owner);
    let mut current = owner.try_with_flat_composite_layers_translation(&[id], 1, 0, &ctl()).unwrap();
    drop(owner);
    assert!(previous.upgrade().is_none());
    for _ in 0..8 {
        current = current.try_with_flat_composite_layers_translation(&[id], -1, 1, &ctl()).unwrap();
    }
    assert!(Arc::ptr_eq(&root, &current.owner.retention));
    assert_eq!(leaf(&current, 0).operative_bounds(), Rect::new(-10, 6, 253, 267));
    let reference = current.document().layers[0].effects.reference.unwrap();
    assert_eq!(reference.0.to_bits(), (-7f64).to_bits());
    assert_eq!(reference.1.to_bits(), 20.5f64.to_bits());
    // On a nonzero vector the zero axis still adds +0.0, matching ordinary Move.
    let q = quota();
    let (mut doc, guard, _) = fixture(&q, ColorMode::Rgb, SampleType::F32);
    doc.layers[0].effects.reference = Some((f64::NAN, -0.));
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned_readonly_flat_composite(doc, guard, q, &ctl()).unwrap();
    let changed = owner.try_with_flat_composite_layers_translation(&[id], 1, 0, &ctl()).unwrap();
    let got = changed.document().layers[0].effects.reference.unwrap();
    assert!(got.0.is_nan());
    assert_eq!(got.1.to_bits(), 0f64.to_bits());
    assert_eq!(owner.document().layers[0].effects.reference.unwrap().1.to_bits(), (-0f64).to_bits());
}

#[test]
#[ignore = "UNRUN: cumulative work and post-reservation cancellation refund plan/copy fees without publishing any target"]
fn work_and_cancel_failures_refund_atomically() {
    let (owner, reads) = three(SampleType::F32);
    let ids = [owner.document().layers[0].id, owner.document().layers[1].id];
    let q = owner.quota();
    let before = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &count };
    owner.validate_flat_composite_layers_translation(&ids, 2, -3, &c).unwrap();
    let remaining = AtomicU64::new(used.load(Ordering::Relaxed));
    let limited = |n| remaining.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &limited };
    assert!(matches!(owner.try_with_flat_composite_layers_translation(&ids, 2, -3, &c), Err(BackingError::Work)));
    assert_eq!(q.usage().unwrap(), before);
    let stop = AtomicBool::new(false);
    let cancelled = || stop.load(Ordering::Relaxed);
    let interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
    let work = |_| {
        if q.usage().unwrap().index > before.index {
            stop.store(true, Ordering::Relaxed);
        }
        Ok(())
    };
    let c = ReadControl { interrupt: &interrupt, work: &work };
    assert!(matches!(owner.try_with_flat_composite_layers_translation(&ids, 2, -3, &c), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(leaf(&owner, 0).operative_bounds(), Rect::new(-3, -2, 260, 259));
}
