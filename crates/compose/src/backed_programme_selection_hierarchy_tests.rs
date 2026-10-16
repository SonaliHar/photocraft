//! Deferred selected actual-tree CURRENT proof. All cloned model/map/default
//! allocations use the bounded same-quota trusted-adoption helper; no raw Source claim.
use super::*;
use crate::backed_stock_point_context::programme_selection_tests::{Budget, selected_copy};
use photocraft_doc::accounted_document::ProgrammeNativeStorageKind;
fn current(sample: SampleType, masked: bool) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    let mut f = fixture(sample, Size::new(8, 6), true);
    if masked {
        let s = f.doc.xcf_state.as_mut().unwrap();
        let n = &mut s.filter_stacks[0].nodes[0];
        let k = crate::gimp_masked_mixed::Kind::InvertGamma;
        n.operation = OperationIdentity { name: k.operation_name().into(), stored_version: k.version().into(), schema_epoch: k.epoch().into() };
        n.arguments.clear();
        n.composition.as_mut().unwrap().compatibility_epoch = k.epoch().into();
        let id = n.mask.unwrap();
        s.filter_masks.iter_mut().find(|m| id == ItemRef::Mask(m.id)).unwrap().surface.try_write_pixel_f64(0, 0, &[0.375]).unwrap();
    }
    drop(f.template);
    let d = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()).unwrap();
    (d, f.rasters, f.fails)
}
#[test]
#[ignore = "UNRUN/uncompiled: guarded BYTEzero and nonempty-mask tree CURRENT, canonical groupNone and exact native/programme display independent of editselection; all old raw Source/index boundaries, foreign token, complete hidden/cold-provider proof and Work/cancel cleanup. Synthetic CURRENT only."]
fn selected_tree_requires_actual_owner_and_keeps_all_subtree_proof() {
    for sample in [SampleType::F32, SampleType::F64] {
        for masked in [false, true] {
            let (original, rasters, fails) = current(sample, masked);
            let selected = selected_copy(&original, ProgrammeNativeStorageKind::NormalHierarchy, 0.5 + 2f64.powi(-42), |_| {});
            let q = selected.quota().clone();
            let before = Context::new_current(&original, Default::default(), &ctl()).unwrap();
            let after = Context::new_current(&selected, Default::default(), &ctl()).unwrap();
            before.validate_operative(&ctl()).unwrap();
            after.validate_operative(&ctl()).unwrap();
            let r = selected.document().bounds();
            let a = before.render_band(r, &ctl()).unwrap();
            let b = after.render_band(r, &ctl()).unwrap();
            assert_eq!(a.pixels(), b.pixels());
            drop((a, b));
            assert_eq!(selected.document().xcf_state, original.document().xcf_state);
            assert_eq!(selected.document().source_color_context, original.document().source_color_context);
            let mut budget = Budget { quota: q.clone(), used: 0 };
            assert!(LeasedNormalHierarchyPipeline::new_budgeted(selected.document(), &Interrupt::NONE, &mut budget, ()).is_err());
            assert!(LeasedNormalHierarchyPipeline::new_masked_source_budgeted(selected.document(), &Interrupt::NONE, &mut budget, ()).is_err());
            assert!(selected.document().checked_normal_hierarchy_index_with(NormalHierarchyLimits::STANDARD, &q, &ctl()).is_err());
            let proof = selected.checked_programme_selection(&ctl()).unwrap();
            assert_eq!(proof.storage_kind(), ProgrammeNativeStorageKind::NormalHierarchy);
            let baseline = q.usage().unwrap();
            ctl().check(128).unwrap();
            assert!(header_mode_current(original.document(), false, Some(&proof)).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            drop(proof);
            let baseline = q.usage().unwrap();
            let work = |_| Err(BackingError::Work);
            assert!(after.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            let yes = || true;
            let i = Interrupt::cancel_only(&yes);
            assert!(after.validate_operative(&ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            // Hidden subtree still traverses every native leaf; no pin-drop eviction claim.
            let hidden = selected_copy(&original, ProgrammeNativeStorageKind::NormalHierarchy, 0., |d| {
                let id = d.layers[1].id;
                d.layers[1].visible = false;
                d.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
            });
            let ctx = Context::new_current(&hidden, Default::default(), &ctl()).unwrap();
            ctx.validate_operative(&ctl()).unwrap();
            rasters[2].clear_cache().unwrap();
            let baseline = q.usage().unwrap();
            fails[2].store(true, Ordering::Relaxed);
            assert!(ctx.validate_operative(&ctl()).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            fails[2].store(false, Ordering::Relaxed);
            drop(ctx);
            drop((before, after));
        }
    }
}
