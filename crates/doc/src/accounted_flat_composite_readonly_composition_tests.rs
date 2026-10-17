//! UNRUN typed storage/copy drafts. No source, renderer, Session lock/history or
//! numerical acceptance is claimed. Expected clones/native buffers are external
//! harness data. All fixture/model/source storage retains real same-q preholds.
use super::super::tests::{ctl, fixture, leaf, owned, quota};
use super::*;
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MODES: [i32; 20] = [28, 23, 30, 31, 32, 33, 34, 35, 36, 41, 42, 43, 44, 45, 48, 49, 50, 51, 52, 53];
// Independent public-command contract copied from xcf_composition_cmds::blend,
// rather than asking the new production mapper to construct its own oracle.
const ORDINARY: [(i32, BlendMode); 20] = [
    (28, BlendMode::Normal),
    (23, BlendMode::Overlay),
    (30, BlendMode::Multiply),
    (31, BlendMode::Screen),
    (32, BlendMode::Difference),
    (33, BlendMode::LinearDodge),
    (34, BlendMode::Subtract),
    (35, BlendMode::Darken),
    (36, BlendMode::Lighten),
    (41, BlendMode::Divide),
    (42, BlendMode::ColorDodge),
    (43, BlendMode::ColorBurn),
    (44, BlendMode::HardLight),
    (45, BlendMode::SoftLight),
    (48, BlendMode::VividLight),
    (49, BlendMode::PinLight),
    (50, BlendMode::LinearLight),
    (51, BlendMode::HardMix),
    (52, BlendMode::Exclusion),
    (53, BlendMode::LinearBurn),
];
fn explicit(space: GimpSpace) -> (i32, SampleEncoding) {
    match space {
        GimpSpace::LinearRgb => (1, SampleEncoding::Linear),
        GimpSpace::ProfileRgb => (2, SampleEncoding::NonlinearProfile),
        GimpSpace::PerceptualRgb => (4, SampleEncoding::Perceptual),
        GimpSpace::Lab => panic!("not in the oracle subset"),
    }
}
fn request(layer: LayerId) -> FlatCompositeCompositionPatch {
    FlatCompositeCompositionPatch {
        layer,
        source_mode: 31,
        blend_space: GimpSpace::LinearRgb,
        composite_space: GimpSpace::ProfileRgb,
        operator: GimpOperator::Intersection,
    }
}
fn expected(doc: &mut Document, patch: FlatCompositeCompositionPatch) {
    let layer = doc.layers.iter_mut().find(|l| l.id == patch.layer).unwrap();
    layer.blend = ORDINARY.iter().find(|(mode, _)| *mode == patch.source_mode).unwrap().1;
    let (bw, be) = explicit(patch.blend_space);
    let (cw, ce) = explicit(patch.composite_space);
    layer.import_compositing = Some(ImportCompositing { blend_encoding: be, composite_encoding: ce, pass_through: false });
    let p = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(patch.layer)).unwrap().composition.as_mut().unwrap();
    p.source_mode = patch.source_mode;
    p.legacy = false;
    p.pass_through = false;
    p.stored_blend_space = bw;
    p.stored_composite_space = cw;
    p.stored_operator = match patch.operator {
        GimpOperator::Union => 1,
        GimpOperator::ClipToBackdrop => 2,
        GimpOperator::ClipToLayer => 3,
        GimpOperator::Intersection => 4,
    };
    p.resolved = Some(ResolvedGimpComposition { blend_space: patch.blend_space, composite_space: patch.composite_space, operator: patch.operator });
}

#[test]
#[ignore = "UNRUN: both targets/all20 ordinary modes/all RGB spaces+four operators synchronize exact current fields and preserve original epoch/model"]
fn exact_wire_subset_and_canonical_physical_synchronization() {
    let q = quota();
    let (owner, reads) = owned(&q, crate::ColorMode::Rgb, crate::SampleType::F32);
    for target in 0..2 {
        for mode in MODES {
            for blend in [GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpSpace::PerceptualRgb] {
                for composite in [GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpSpace::PerceptualRgb] {
                    for operator in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
                        let patch = FlatCompositeCompositionPatch {
                            layer: owner.document().layers[target].id,
                            source_mode: mode,
                            blend_space: blend,
                            composite_space: composite,
                            operator,
                        };
                        owner.validate_flat_composite_composition_patch(patch, &ctl()).unwrap();
                        let next = owner.try_with_flat_composite_composition_patch(patch, &ctl()).unwrap();
                        let mut oracle = owner.document().clone();
                        expected(&mut oracle, patch);
                        assert_eq!(next.document(), &oracle);
                        assert!(next.is_readonly_flat_composite_storage());
                        assert!(Arc::ptr_eq(&owner.owner.retention, &next.owner.retention));
                        assert!(Arc::ptr_eq(owner.document().icc_profile.as_ref().unwrap(), next.document().icc_profile.as_ref().unwrap()));
                    }
                }
            }
        }
    }
    assert_eq!(reads.load(Ordering::Relaxed), 0); // numeric policy copies do not read native pixels
}

#[test]
#[ignore = "UNRUN: raw Auto-to-explicit is changed, exact explicit noop needs no allocation even fullquota, and physical divergence is repaired"]
fn explicit_raw_noop_at_full_quota() {
    let q = quota();
    let (owner, _) = owned(&q, crate::ColorMode::Rgb, crate::SampleType::F64);
    let patch = request(owner.document().layers[0].id);
    let explicit = owner.try_with_flat_composite_composition_patch(patch, &ctl()).unwrap();
    assert!(!explicit.shares_model(&owner));
    let used = q.usage().unwrap();
    let index = q.reserve(QuotaKind::Index, (16 << 20) - used.index).unwrap();
    let resident = q.reserve(QuotaKind::Resident, (64 << 20) - q.usage().unwrap().resident).unwrap();
    let full = q.usage().unwrap();
    assert!(explicit.try_with_flat_composite_composition_patch(patch, &ctl()).unwrap().shares_model(&explicit));
    assert!(owner.try_with_flat_composite_composition_patch(patch, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), full);
    drop(resident);
    drop(index);
    let mut malformed_physical = explicit;
    Arc::get_mut(&mut malformed_physical.owner).unwrap().document.layers[0].blend = BlendMode::Normal;
    let repaired = malformed_physical.try_with_flat_composite_composition_patch(patch, &ctl()).unwrap();
    assert!(!repaired.shares_model(&malformed_physical));
    assert_eq!(repaired.document().layers[0].blend, BlendMode::Screen);
    assert!(repaired.try_with_flat_composite_composition_patch(patch, &ctl()).unwrap().shares_model(&repaired));
}

#[test]
#[ignore = "UNRUN: wrongclass/target/Lab/grain/legacy/unsupported mode and malformed prestate refuse before even a requested raw noop"]
fn invalid_requests_and_prestate_do_not_acquire_permissions() {
    let q = quota();
    let (owner, _) = owned(&q, crate::ColorMode::Rgb, crate::SampleType::U16);
    let good = request(owner.document().layers[0].id);
    for mode in [0, 3, 37, 46, 47, 54, 57, 61, i32::MAX] {
        assert!(owner.try_with_flat_composite_composition_patch(FlatCompositeCompositionPatch { source_mode: mode, ..good }, &ctl()).is_err());
    }
    for bad in [
        FlatCompositeCompositionPatch { layer: LayerId(0), ..good },
        FlatCompositeCompositionPatch { layer: LayerId(u64::MAX), ..good },
        FlatCompositeCompositionPatch { blend_space: GimpSpace::Lab, ..good },
        FlatCompositeCompositionPatch { composite_space: GimpSpace::Lab, ..good },
    ] {
        assert!(owner.validate_flat_composite_composition_patch(bad, &ctl()).is_err());
    }
    let (doc, guard, _) = fixture(&q, crate::ColorMode::Rgb, crate::SampleType::F32);
    let legacy = AccountedDocument::from_owned(doc, guard, q.clone(), &ctl()).unwrap();
    assert!(legacy.try_with_flat_composite_composition_patch(request(legacy.document().layers[0].id), &ctl()).is_err());
    for fault in 0..5 {
        let (base, _) = owned(&q, crate::ColorMode::Rgb, crate::SampleType::F32);
        let patch = request(base.document().layers[0].id);
        let mut explicit = base.try_with_flat_composite_composition_patch(patch, &ctl()).unwrap();
        let doc = &mut Arc::get_mut(&mut explicit.owner).unwrap().document;
        let state = doc.xcf_state.as_mut().unwrap();
        let row = state.items.iter_mut().find(|i| i.item == ItemRef::Layer(patch.layer)).unwrap();
        match fault {
            0 => row.name = "divergent".into(),
            1 => row.visible = false,
            2 => row.composition = None,
            3 => row.composition.as_mut().unwrap().stored_operator = 1,
            _ => state.current_filter_output_precision = crate::FilterOutputPrecision::Half,
        }
        let before = q.usage().unwrap();
        assert!(explicit.try_with_flat_composite_composition_patch(patch, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    assert!(owner.try_edit(AccountedEdit::LayerName { layer: good.layer, name: "no individual grant" }, &ctl()).is_err());
    assert!(owner.try_edit(AccountedEdit::NativePixels { layer: good.layer, rect: Rect::new(-3, -2, -2, -1), bytes: &[0; 8] }, &ctl()).is_err());
}

#[test]
#[ignore = "UNRUN: repeated composition/title/compoundmetadata copies retain direct source root and exact two-depth native/default/profile raw storage"]
fn direct_root_lifetime_and_raw_storage_survive_copies() {
    for sample in [crate::SampleType::U16, crate::SampleType::F64] {
        let q = quota();
        let (original, _) = owned(&q, crate::ColorMode::Rgb, sample);
        let id = original.document().layers[1].id;
        let weak = Arc::downgrade(&original.owner);
        let patch = request(id);
        let mut current = original.try_with_flat_composite_composition_patch(patch, &ctl()).unwrap();
        for index in 0..2 {
            let a = leaf(&original, index);
            let b = leaf(&current, index);
            assert_eq!(a.descriptor(), b.descriptor());
            assert_eq!(a.default_bytes(), b.default_bytes());
            assert!(a.try_content_equal(b, &ctl()).unwrap());
        }
        assert!(Arc::ptr_eq(&original.owner.retention, &current.owner.retention));
        drop(original);
        assert!(weak.upgrade().is_none());
        for n in 0..8 {
            let title = current.try_with_document_name(&format!("copy-{n}.pcraft"), &ctl()).unwrap();
            let metadata =
                title.try_edit(AccountedEdit::LayerMetadata { layer: id, name: Some("kept"), visible: Some(n % 2 == 0), opacity: Some(0.5) }, &ctl()).unwrap();
            let next = metadata
                .try_with_flat_composite_composition_patch(FlatCompositeCompositionPatch { source_mode: if n % 2 == 0 { 30 } else { 31 }, ..patch }, &ctl())
                .unwrap();
            assert!(Arc::ptr_eq(&current.owner.retention, &next.owner.retention));
            assert!(Arc::ptr_eq(current.document().icc_profile.as_ref().unwrap(), next.document().icc_profile.as_ref().unwrap()));
            assert!(Arc::ptr_eq(
                current.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
                next.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
            ));
            current = next;
        }
        current.validate_readonly_flat_composite_storage(&ctl()).unwrap();
        assert!(
            current
                .try_with_flat_composite_composition_patch(FlatCompositeCompositionPatch { source_mode: 31, ..patch }, &ctl())
                .unwrap()
                .shares_model(&current)
        );
    }
}

#[test]
#[ignore = "UNRUN: one cumulative work ledger and post-prehold caller cancellation roll back the entire private composition model"]
fn cumulative_work_and_cancellation_do_not_publish_partial_copy() {
    let q = quota();
    let (owner, _) = owned(&q, crate::ColorMode::Rgb, crate::SampleType::U32);
    let patch = request(owner.document().layers[0].id);
    let measured_work = AtomicU64::new(0);
    let count = |n| {
        measured_work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    owner.validate_flat_composite_composition_patch(patch, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    let before = q.usage().unwrap();
    let remaining = AtomicU64::new(measured_work.load(Ordering::Relaxed));
    let work = |n| remaining.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(matches!(
        owner.try_with_flat_composite_composition_patch(patch, &ReadControl { interrupt: &Interrupt::NONE, work: &work }),
        Err(BackingError::Work)
    ));
    assert_eq!(q.usage().unwrap(), before);
    let armed = AtomicBool::new(false);
    let cancelled = || armed.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let work = |_| {
        if q.usage().map_err(|_| BackingError::Quota)?.index > before.index {
            armed.store(true, Ordering::Relaxed);
        }
        Ok(())
    };
    assert!(matches!(
        owner.try_with_flat_composite_composition_patch(patch, &ReadControl { interrupt: &interrupt, work: &work }),
        Err(BackingError::Cancelled)
    ));
    assert!(armed.load(Ordering::Relaxed));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().layers[0].blend, BlendMode::Normal);
    assert_eq!(
        owner
            .document()
            .xcf_state
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|i| i.item == ItemRef::Layer(patch.layer))
            .unwrap()
            .composition
            .as_ref()
            .unwrap()
            .source_mode,
        47
    );
}
