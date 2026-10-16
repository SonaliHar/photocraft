//! Deferred selected CURRENT contracts. Trusted synthetic codec adoption owns
//! each bounded cloned model/map/default under real prospective guards; genuine
//! native/source payload is retained from the original paid fixture. No Source claim.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument};
use photocraft_raster::Surface;
use std::{any::Any, sync::Arc};
pub(crate) struct Budget {
    pub(crate) quota: BackingQuota,
    pub(crate) used: u64,
}
impl PreparationBudget for Budget {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.quota.reserve(QuotaKind::Resident, n).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.used = self.used.checked_add(n).ok_or(CompositionError::Invalid("test work"))?;
        Ok(())
    }
}
impl crate::backed_normal_hierarchy_context::HierarchyPreparationBudget for Budget {
    fn reserve_hierarchy_index(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.quota.reserve(QuotaKind::Index, n).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
    }
}
impl crate::backed_masked_filtered_flat_point_context::MaskedFilterPreparationBudget for Budget {
    fn reserve_filter_mask_proof_index(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.quota.reserve(QuotaKind::Index, n).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
    }
}
// This caller owns incoming actual allocations as required by the trusted codec
// constructor; it does not assert a free lease or produce an original XCF role.
pub(crate) fn selected_copy(owner: &AccountedDocument, kind: ProgrammeNativeStorageKind, value: f64, change: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mut d = owner.document().clone();
    let f = PixelFormat::new(ColorMode::Grayscale, d.depth, false);
    let mut raw = [0; 8];
    let word = raw.get_mut(..f.bytes_per_pixel()).unwrap();
    photocraft_color::try_write_sample_f64(word, f.sample, 0, value).unwrap();
    d.selection = Some(Surface::try_with_default_bytes(f, word).unwrap());
    change(&mut d);
    AccountedDocument::from_owned_programme_rehydrating_selection(kind, d, Arc::new((owner.clone(), index, resident)), q, &tests::ctl()).unwrap()
}
fn configured(role: usize, sample: SampleType) -> AccountedDocument {
    let (mut d, q, guard, _, _, _) = tests::fixture(2, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    let state = d.xcf_state.as_mut().unwrap();
    state.items[0].composition.as_mut().unwrap().compatibility_epoch = "xcf-22-normal".into();
    let n = &mut state.filter_stacks[0].nodes[0];
    if role == 0 {
        n.operation.schema_epoch = crate::EXPOSURE_LOADED_POINT_EPOCH.into();
        n.composition.as_mut().unwrap().compatibility_epoch = crate::EXPOSURE_LOADED_POINT_EPOCH.into();
        n.arguments = vec![
            FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) },
            FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0x3dcc_cccd)) },
        ];
    } else if role == 1 {
        let k = crate::gimp_masked_scalar::Kind::InvertGamma;
        n.operation.name = k.operation_name().into();
        n.operation.stored_version = k.version().into();
        n.operation.schema_epoch = k.epoch().into();
        n.composition.as_mut().unwrap().compatibility_epoch = k.epoch().into();
        n.arguments.clear();
        state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.375]).unwrap();
    } else {
        n.operation.schema_epoch = crate::gimp_masked_exposure::EPOCH.into();
        n.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_masked_exposure::EPOCH.into();
        state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.375]).unwrap();
    }
    tests::adopt(d, q, guard)
}
#[test]
#[ignore = "UNRUN/uncompiled: actual paid Point loaded/selected-masked branches, raw Source/public bare pipeline refusal, exact unselected programme output, foreign Doc token and disabled semantic refusal; original controls/refund. Synthetic CURRENT only."]
fn selected_point_keeps_programmes_and_raw_boundaries() {
    for sample in [SampleType::F32, SampleType::F64] {
        for role in 0..3 {
            let original = configured(role, sample);
            let selected = selected_copy(&original, ProgrammeNativeStorageKind::Point, 0.5 + 2f64.powi(-42), |_| {});
            let q = selected.quota().clone();
            let basis = Builtin::Srgb.profile();
            let before = PreparedBackedStockPointContext::new(&original, basis, Default::default(), &tests::ctl()).unwrap();
            let after = PreparedBackedStockPointContext::new(&selected, basis, Default::default(), &tests::ctl()).unwrap();
            before.validate_operative(&tests::ctl()).unwrap();
            after.validate_operative(&tests::ctl()).unwrap();
            let area = selected.document().bounds();
            let a = before.render_wide_band(area, &tests::ctl()).unwrap();
            let b = after.render_wide_band(area, &tests::ctl()).unwrap();
            assert_eq!(a.pixels(), b.pixels());
            drop((a, b));
            assert_eq!(selected.document().xcf_state, original.document().xcf_state);
            assert_eq!(selected.document().icc_profile, original.document().icc_profile);
            assert!(PreparedStockPointPipeline::prepare(selected.document(), basis, &q, &tests::ctl()).is_err());
            let mut budget = Budget { quota: q.clone(), used: 0 };
            assert!(PreparedStockPointPipeline::new_budgeted(selected.document(), &Interrupt::NONE, &mut budget, ()).is_err());
            let proof = selected.checked_programme_selection(&tests::ctl()).unwrap();
            assert!(proof.matches_owner(&selected));
            assert!(!proof.matches_document(original.document()));
            let baseline = q.usage().unwrap();
            assert!(
                prepare_plan_current(
                    original.document(),
                    Some(basis),
                    PreparationPurpose::Current,
                    Some(&proof),
                    128,
                    &Interrupt::NONE,
                    &mut NativeBudget { quota: &q, control: &tests::ctl() }
                )
                .is_err()
            );
            assert_eq!(q.usage().unwrap(), baseline);
            drop(proof);
            let baseline = q.usage().unwrap();
            let work = |_| Err(BackingError::Work);
            let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
            assert!(after.validate_operative(&c).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            let yes = || true;
            let i = Interrupt::cancel_only(&yes);
            assert!(after.validate_operative(&ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            drop((before, after));
            let malformed = selected_copy(&original, ProgrammeNativeStorageKind::Point, 0.375, |d| {
                let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                n.enabled = false;
                n.operation.schema_epoch = "unsupported-selected-disabled".into();
            });
            assert!(PreparedBackedStockPointContext::new(&malformed, basis, Default::default(), &tests::ctl()).is_err());
        }
    }
}
