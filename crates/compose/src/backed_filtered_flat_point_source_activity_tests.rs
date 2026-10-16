//! UNRUN measured activity descriptors on genuine paid synthetic CURRENT roots.
//! Actual original-XCF admission is tested at IO/UI; no L4 dependency or GIMP oracle.
use super::*;
#[path = "../../engine/src/accounted_filtered_flat_source_activity_fixture.rs"]
mod data;
fn activity(case: &data::Case, poison: Option<usize>) -> Fixture {
    let sample = if case.precision == 650 { SampleType::F32 } else { SampleType::F64 };
    let mut f = fixture(sample, Size::new(8, 6), &[None, Some(0), Some(1), Some(2)], poison);
    for index in 0..4 {
        let layer = &mut f.document.layers[index];
        layer.visible = case.visible[index];
        layer.opacity = f32::from_bits(case.opacity_bits[index]);
        let id = layer.id;
        let state = f.document.xcf_state.as_mut().unwrap();
        state.items.iter_mut().find(|row| row.item == ItemRef::Layer(id)).unwrap().visible = case.visible[index];
        if let Some(enabled) = case.enabled[index] {
            state.filter_stacks.iter_mut().find(|s| s.owner == ItemRef::Layer(id)).unwrap().nodes[0].enabled = enabled;
        }
    }
    f
}
#[test]
#[ignore = "UNRUN all12 measured activity states on paid synthetic CURRENT/sourceprep metadata, independent leaf+Normal math and inactive fullproof; no actualXCF/projection oracle"]
fn measured_activity_keeps_all_roots_prepared_and_proven() {
    // Independent public Parts/reference allocation is external harness, not q receipt.
    let nl = photocraft_cms::owned_lab::construct_with_control(
        photocraft_cms::Builtin::Srgb.profile(),
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        Default::default(),
        &|| false,
    )
    .unwrap();
    let linear = photocraft_cms::owned_linear_srgb_lab::construct_with_control(Default::default(), &|| false).unwrap();
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    struct Budget {
        q: BackingQuota,
        work: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = QuotaLease;
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
            self.q.reserve(QuotaKind::Resident, n).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
            self.work = self.work.checked_add(n).ok_or(CompositionError::Invalid("draft work"))?;
            Ok(())
        }
    }
    for case in &data::CASES {
        let sample = if case.precision == 650 { SampleType::F32 } else { SampleType::F64 };
        let (owner, _, _, _) = adopt(activity(case, None));
        let q = owner.quota().clone();
        let baseline = q.usage().unwrap();
        let mut budget = Budget { q: q.clone(), work: 0 };
        // This source-prep API consumes actual retained metadata/owners, not an
        // original-XCF authority token. UI/IO independently test real admission.
        let prepared = LeasedFilteredFlatPointPipeline::new_budgeted(owner.document(), &Interrupt::NONE, &mut budget, owner.clone()).unwrap();
        assert!(budget.work > 0);
        assert!(q.usage().unwrap().resident > baseline.resident);
        drop(prepared);
        assert_eq!(q.usage().unwrap(), baseline);
        let p = PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).unwrap();
        p.validate_operative(&ctl()).unwrap();
        for (x, y) in [(0, 0), (2, 0), (7, 5)] {
            let mut expected = [0.; 4];
            for index in 0..4 {
                let native = input(index, x, y, sample);
                let raw = if case.enabled[index] == Some(true) { leaf([None, Some(0), Some(1), Some(2)][index], native, &nl, &linear) } else { native };
                if case.visible[index] {
                    expected = working.normal(expected, raw, f64::from(f32::from_bits(case.opacity_bits[index])), GimpOperator::Union).unwrap();
                }
            }
            assert_eq!(
                p.render_band(Rect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits),
                expected.map(f64::to_bits),
                "{}",
                case.id
            );
        }
        drop(p);
        // Every actual activity case must still reject bad native in its last
        // root, even when that root is hidden/zero/disabled in the original state.
        let (bad, _, _, _) = adopt(activity(case, Some(3)));
        assert!(PreparedBackedFilteredFlatPointDocument::new(&bad, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).is_err(), "{}", case.id);
    }
    // All-inactive source state retains three filtered roots. Neither mask
    // padding nor actual current-profile mismatch is waived by its activity.
    let case = data::CASES.iter().find(|c| c.id == "f64-all-inactive").unwrap();
    for fault in 0..2 {
        let mut f = activity(case, None);
        if fault == 0 {
            f.document.xcf_state.as_mut().unwrap().filter_masks[2]
                .surface
                .try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes())
                .unwrap();
        } else {
            f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
        }
        if let Ok(owner) = AccountedDocument::new_readonly_filtered_flat_point_storage(f.document, f.retention, f.quota, &ctl()) {
            assert!(PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).is_err());
        }
    }
}
