use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
pub(super) const EPOCH: &str = "xcf22-stock-alpha-clip-precision-v1";
pub(super) fn valid_arguments(arguments: &[FilterArgument]) -> bool {
    if arguments.len() != 4 {
        return false;
    }
    for name in ["clip-low", "clip-high"] {
        if !arguments.iter().any(|argument| argument.name == name && matches!(argument.value, FilterValue::Bool(_))) {
            return false;
        }
    }
    let read = |name| {
        arguments.iter().find(|argument| argument.name == name).and_then(|argument| match argument.value {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) => {
                let value = f32::from_bits(bits);
                (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
            }
            _ => None,
        })
    };
    match (read("low-limit"), read("high-limit")) {
        (Some(low), Some(high)) => low <= high,
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn arguments() -> Vec<FilterArgument> {
        vec![
            FilterArgument { name: "clip-low".into(), value: FilterValue::Bool(true) },
            FilterArgument { name: "low-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())) },
            FilterArgument { name: "clip-high".into(), value: FilterValue::Bool(true) },
            FilterArgument { name: "high-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
        ]
    }
    #[test]
    fn alpha_clip_schema_preserves_exact_bool_f32_ordered_unit_contract() {
        let good = arguments();
        assert!(valid_arguments(&good));
        let mut shuffled = good.clone();
        shuffled.reverse();
        assert!(valid_arguments(&shuffled));
        for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.01f32).to_bits(), 1.01f32.to_bits(), 0.8f32.to_bits()] {
            let mut bad = good.clone();
            bad[1].value = FilterValue::Number(ExactNumber::Float32Bits(bits));
            assert!(!valid_arguments(&bad));
        }
        let mut missing = good.clone();
        missing.pop();
        assert!(!valid_arguments(&missing));
        let mut extra = good.clone();
        extra.push(good[0].clone());
        assert!(!valid_arguments(&extra));
        let mut duplicate = good.clone();
        duplicate[3] = good[1].clone();
        assert!(!valid_arguments(&duplicate));
        let mut wrong = good.clone();
        wrong[0].value = FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits()));
        assert!(!valid_arguments(&wrong));
        let mut wrong = good.clone();
        wrong[1].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
        assert!(!valid_arguments(&wrong));
        for low in [false, true] {
            for high in [false, true] {
                let mut flags = good.clone();
                flags[0].value = FilterValue::Bool(low);
                flags[2].value = FilterValue::Bool(high);
                assert!(valid_arguments(&flags));
            }
        }
    }
}
