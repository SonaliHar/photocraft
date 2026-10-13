//! Exact canonical Float32 threshold; public gdouble .8 is not a legal .8f32.
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};

pub(super) const EPOCH: &str = "xcf-22-stock-red-eye-removal-precision-v1";
const MAX: f64 = 0.7999999523162842; // Float32 bits 3f4ccccc, strictly below .8.

fn valid(value: f32) -> bool {
    value.is_finite() && (0.0..=0.8).contains(&f64::from(value))
}

pub(super) fn valid_arguments(arguments: &[FilterArgument]) -> bool {
    matches!(arguments, [FilterArgument { name, value: FilterValue::Number(ExactNumber::Float32Bits(bits)) }]
        if name == "threshold" && valid(f32::from_bits(*bits)))
}

fn parse(text: &str) -> Option<f64> {
    if text.len() > 64 {
        return None;
    }
    text.parse::<f32>().ok().filter(|value| valid(*value)).map(f64::from)
}

pub(super) fn edit(ui: &mut egui::Ui, label: &str, editable: bool, bits: u32) -> Option<FilterValue> {
    let mut current = f32::from_bits(bits);
    if !valid(current) {
        return None;
    }
    let changed = ui
        .horizontal(|ui| {
            let label = ui.label(label);
            ui.add_enabled(
                editable,
                egui::DragValue::new(&mut current)
                    .speed(0.01)
                    .range(0.0..=MAX)
                    .custom_formatter(|value, _| (value as f32).to_string())
                    .custom_parser(parse)
                    .update_while_editing(false),
            )
            .labelled_by(label.id)
            .changed()
        })
        .inner;
    (changed && valid(current) && current.to_bits() != bits).then_some(FilterValue::Number(ExactNumber::Float32Bits(current.to_bits())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_float_boundary_and_width_are_not_normalized() {
        assert_eq!(parse("0.79999995"), Some(f64::from(f32::from_bits(0x3f4ccccc))));
        for text in ["0.8", "0.8000001", "-0.1", "NaN", "inf"] {
            assert_eq!(parse(text), None);
        }
        assert_eq!(parse("-0").unwrap().to_bits(), (-0.0_f64).to_bits());
        let valid = vec![FilterArgument { name: "threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25_f32.to_bits())) }];
        assert!(valid_arguments(&valid));
        let mut duplicate = valid.clone();
        duplicate.extend(valid.clone());
        assert!(!valid_arguments(&duplicate));
        assert!(!valid_arguments(&[FilterArgument { name: "threshold".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0.25_f64.to_bits())) }]));
    }
}
