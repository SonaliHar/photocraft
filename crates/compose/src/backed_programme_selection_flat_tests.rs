//! Deferred guarded CURRENT flat programmes. All model/native fixtures remain
//! genuinely prepaid; selected codec-copy helper is synthetic CURRENT only.
use super::*;
use crate::backed_stock_point_context::programme_selection_tests::{Budget, selected_copy};
use photocraft_doc::accounted_document::ProgrammeNativeStorageKind;
fn owner(sample: SampleType, masked: bool, poison: bool) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    let mut f = fixture(sample, Size::new(8, 6), &[None, Some(0), Some(1), Some(2)], poison.then_some(1));
    if masked {
        let s = f.document.xcf_state.as_mut().unwrap();
        let n = &mut s.filter_stacks[0].nodes[0];
        let k = crate::gimp_masked_mixed::Kind::InvertGamma;
        n.operation = OperationIdentity { name: k.operation_name().into(), stored_version: k.version().into(), schema_epoch: k.epoch().into() };
        n.arguments.clear();
        n.composition.as_mut().unwrap().compatibility_epoch = k.epoch().into();
        let id = n.mask.unwrap();
        s.filter_masks.iter_mut().find(|m| id == ItemRef::Mask(m.id)).unwrap().surface.try_write_pixel_f64(0, 0, &[0.375]).unwrap();
    }
    if poison {
        f.document.layers[1].visible = false;
        let id = f.document.layers[1].id;
        f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
    }
    let Fixture { document, retention, quota, rasters, fails, .. } = f;
    let d = if masked {
        AccountedDocument::new_readonly_masked_filtered_flat_point_storage(document, retention, quota, &ctl()).unwrap()
    } else {
        AccountedDocument::new_readonly_filtered_flat_point_storage(document, retention, quota, &ctl()).unwrap()
    };
    (d, rasters, fails)
}
#[test]
#[ignore = "UNRUN/uncompiled: actual guarded flat+masked CURRENT header paths, exact programme output despite edit coverage, raw Source/opacitySource refusal and foreign token; hidden malformed native still fails, same-meter control and warm/cold refunds. Synthetic CURRENT only."]
fn selected_flat_header_preserves_full_recipe_proof() {
    for sample in [SampleType::F32, SampleType::F64] {
        for masked in [false, true] {
            let (original, rasters, fails) = owner(sample, masked, false);
            let kind = if masked { ProgrammeNativeStorageKind::MaskedFilteredFlatPoint } else { ProgrammeNativeStorageKind::FilteredFlatPoint };
            let selected = selected_copy(&original, kind, 0.375, |_| {});
            let q = selected.quota().clone();
            let before = if masked {
                Context::new_masked(&original, Default::default(), &ctl()).unwrap()
            } else {
                Context::new(&original, Default::default(), &ctl()).unwrap()
            };
            let after = if masked {
                Context::new_masked(&selected, Default::default(), &ctl()).unwrap()
            } else {
                Context::new(&selected, Default::default(), &ctl()).unwrap()
            };
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
            assert!(LeasedFilteredFlatPointPipeline::new_budgeted(selected.document(), &Interrupt::NONE, &mut budget, ()).is_err());
            assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(selected.document(), &Interrupt::NONE, &mut budget, ()).is_err());
            if masked {
                assert!(
                    crate::backed_masked_filtered_flat_point_context::LeasedMaskedFilteredFlatPointPipeline::new_budgeted(
                        selected.document(),
                        &Interrupt::NONE,
                        &mut budget,
                        ()
                    )
                    .is_err()
                );
            }
            let proof = selected.checked_programme_selection(&ctl()).unwrap();
            let baseline = q.usage().unwrap();
            let purpose = if masked { Purpose::MaskedCurrent } else { Purpose::Current };
            assert!(
                compile_current(original.document(), purpose, None, Some(&proof), 128, &Interrupt::NONE, &mut NativeBudget { quota: &q, control: &ctl() })
                    .is_err()
            );
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
            rasters[3].clear_cache().unwrap();
            let baseline = q.usage().unwrap();
            fails[3].store(true, Ordering::Relaxed);
            assert!(after.validate_operative(&ctl()).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            fails[3].store(false, Ordering::Relaxed);
            drop((before, after));
            let (poisoned, _, _) = owner(sample, masked, true);
            let hidden = selected_copy(&poisoned, kind, 0., |_| {});
            let ctx = if masked {
                Context::new_masked(&hidden, Default::default(), &ctl()).unwrap()
            } else {
                Context::new(&hidden, Default::default(), &ctl()).unwrap()
            };
            assert!(ctx.validate_operative(&ctl()).is_err());
        }
    }
}
