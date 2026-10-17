//! Regression drafts, UNRUN. No font assets or source authority are constructed.
use super::*;
struct Budget {
    used: u64,
    limit: u64,
}
impl PreparationBudget for Budget {
    type Lease = ();
    fn reserve_prepared_bytes(&mut self, _: u64) -> Result<()> {
        Ok(())
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
        self.used = self.used.checked_add(n).filter(|v| *v <= self.limit).ok_or_else(invalid)?;
        Ok(())
    }
}
#[test]
fn nested_metadata_walk_debits_cumulative_work_before_processing_each_field() {
    let fields = vec![FilterArgument {
        name: "layout".into(),
        value: FilterValue::Config(vec![FilterArgument {
            name: "font-size".into(),
            value: FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float64Bits(32f64.to_bits())),
        }]),
    }];
    let mut denied = Budget { used: 0, limit: 128 };
    let mut visits = 0;
    assert!(arguments(&fields, 0, &mut visits, &mut denied, &Interrupt::NONE).is_err());
    assert_eq!(visits, 1);
    let consumed = denied.used;
    assert!(arguments(&fields, 0, &mut visits, &mut denied, &Interrupt::NONE).is_err());
    assert_eq!(denied.used, consumed);
}
#[test]
fn cancelled_or_overdeep_metadata_cannot_reach_font_or_profile_preparation() {
    let fields = vec![FilterArgument { name: "layout".into(), value: FilterValue::Bool(false) }];
    let mut budget = Budget { used: 0, limit: 10_000 };
    let mut visits = 0;
    assert!(matches!(
        arguments(&fields, 0, &mut visits, &mut budget, &Interrupt::cancel_only(&|| true)),
        Err(CompositionError::Raster(RasterInputError::Cancelled))
    ));
    assert_eq!(visits, 0);
    assert!(arguments(&fields, 9, &mut visits, &mut budget, &Interrupt::NONE).is_err());
    assert_eq!(visits, 0);
}

#[test]
fn admitted_v19_normal_auto_context_keeps_its_actual_linear_resolution() {
    use photocraft_doc::import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition};
    // The existing stem-text importer admits this exact raw tuple, then uses
    // its Normal28 Auto policy; it does not derive composition from NL storage.
    let mut policy = GimpComposition {
        compatibility_epoch: "xcf-19-normal".into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: -1,
        stored_operator: -1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    };
    assert!(normal_text_resolution(&policy));
    policy.resolved =
        Some(ResolvedGimpComposition { blend_space: GimpSpace::ProfileRgb, composite_space: GimpSpace::ProfileRgb, operator: GimpOperator::Union });
    assert!(!normal_text_resolution(&policy));
    policy.resolved = None;
    assert!(!normal_text_resolution(&policy));
}
