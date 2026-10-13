use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
pub(super) const EPOCH: &str = "xcf22-stock-alpha-clip-finite-precision-v1";
pub(super) fn valid_arguments(arguments: &[FilterArgument]) -> bool {
    arguments.len() == 4
        && ["clip-low", "clip-high"]
            .iter()
            .all(|name| arguments.iter().any(|argument| argument.name == *name && matches!(argument.value, FilterValue::Bool(_))))
        && ["low-limit", "high-limit"].iter().all(|name| {
            arguments.iter().any(|argument| {
                argument.name == *name && matches!(argument.value, FilterValue::Number(ExactNumber::Float32Bits(bits)) if f32::from_bits(bits).is_finite())
            })
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_alpha_clip_preserves_unordered_f32_contract_without_widening_unit_epoch() {
        let arguments = |low: f32, high: f32| {
            vec![
                FilterArgument { name: "clip-low".into(), value: FilterValue::Bool(true) },
                FilterArgument { name: "low-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(low.to_bits())) },
                FilterArgument { name: "clip-high".into(), value: FilterValue::Bool(true) },
                FilterArgument { name: "high-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(high.to_bits())) },
            ]
        };
        for (low, high) in [(0.25, 0.75), (-0.125, 1.125), (0.875, 0.125), (f32::MIN, f32::MAX), (-0.0, 0.0)] {
            let mut good = arguments(low, high);
            assert!(valid_arguments(&good));
            good.reverse();
            assert!(valid_arguments(&good));
        }
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!valid_arguments(&arguments(value, 0.75)));
            assert!(!valid_arguments(&arguments(0.25, value)));
        }
        let good = arguments(0.25, 0.75);
        assert!(!valid_arguments(&good[..3]));
        for index in 0..4 {
            let mut unknown = good.clone();
            unknown[index].name = "unknown".into();
            assert!(!valid_arguments(&unknown));
            let mut duplicate = good.clone();
            duplicate.push(good[index].clone());
            assert!(!valid_arguments(&duplicate));
            let mut wrong = good.clone();
            wrong[index].value = FilterValue::String("0".into());
            assert!(!valid_arguments(&wrong));
        }
        let mut f64_argument = good;
        f64_argument[1].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
        assert!(!valid_arguments(&f64_argument));
        assert!(!super::super::alpha_clip::valid_arguments(&arguments(-0.125, 1.125)));
        assert!(!super::super::alpha_clip::valid_arguments(&arguments(0.875, 0.125)));
    }
}
