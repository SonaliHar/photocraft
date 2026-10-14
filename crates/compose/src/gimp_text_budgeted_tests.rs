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
