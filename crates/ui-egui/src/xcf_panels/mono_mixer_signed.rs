//! Exact signed Mono weights retain their canonical stored numeric width.
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
use serde_json::{Value, json};

pub(super) const EPOCH: &str = "xcf-22-stock-mono-mixer-signed-precision-v1";

fn weight(number: &ExactNumber) -> Option<f64> {
    let value = match number {
        ExactNumber::Float32Bits(bits) => f64::from(f32::from_bits(*bits)),
        ExactNumber::Float64Bits(bits) => f64::from_bits(*bits),
        _ => return None,
    };
    (value.is_finite() && (-5.0..=5.0).contains(&value)).then_some(value)
}

pub(super) fn valid_arguments(arguments: &[FilterArgument]) -> bool {
    arguments.len() == 4
        && ["red", "green", "blue", "preserve-luminosity"].into_iter().all(|name| {
            let mut matches = arguments.iter().filter(|argument| argument.name == name);
            let Some(argument) = matches.next() else { return false };
            matches.next().is_none()
                && match &argument.value {
                    FilterValue::Bool(_) => name == "preserve-luminosity",
                    FilterValue::Number(number) => name != "preserve-luminosity" && weight(number).is_some(),
                    _ => false,
                }
        })
}

pub(super) fn edit_weight(ui: &mut egui::Ui, label: &str, editable: bool, number: &ExactNumber) -> Option<Value> {
    weight(number)?;
    match number {
        ExactNumber::Float32Bits(bits) => {
            let mut current = f32::from_bits(*bits);
            let changed = ui
                .horizontal(|ui| {
                    let label = ui.label(label);
                    // The default display precision may round retained bits when its
                    // formatted text gains focus. Use a round-trip string and commit
                    // typed text only when accepted; opening/Escape stays read-only.
                    ui.add_enabled(
                        editable,
                        egui::DragValue::new(&mut current)
                            .speed(0.01)
                            .range(-5.0..=5.0)
                            .custom_formatter(|value, _| (value as f32).to_string())
                            .update_while_editing(false),
                    )
                    .labelled_by(label.id)
                    .changed()
                })
                .inner;
            (changed && current.to_bits() != *bits).then(|| json!({"Number":{"Float32Bits":current.to_bits()}}))
        }
        ExactNumber::Float64Bits(bits) => {
            let mut current = f64::from_bits(*bits);
            let changed = ui
                .horizontal(|ui| {
                    let label = ui.label(label);
                    ui.add_enabled(
                        editable,
                        egui::DragValue::new(&mut current)
                            .speed(0.01)
                            .range(-5.0..=5.0)
                            .custom_formatter(|value, _| value.to_string())
                            .update_while_editing(false),
                    )
                    .labelled_by(label.id)
                    .changed()
                })
                .inner;
            (changed && current.to_bits() != *bits).then(|| json!({"Number":{"Float64Bits":format!("0x{:016x}",current.to_bits())}}))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_mono_schema_preserves_supported_widths_and_refuses_hostile_shapes() {
        let arguments = vec![
            FilterArgument { name: "red".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.5_f32).to_bits())) },
            FilterArgument { name: "green".into(), value: FilterValue::Number(ExactNumber::Float64Bits(1.5_f64.to_bits())) },
            FilterArgument { name: "blue".into(), value: FilterValue::Number(ExactNumber::Float64Bits((-0.0_f64).to_bits())) },
            FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) },
        ];
        assert!(valid_arguments(&arguments));
        let mut reversed = arguments.clone();
        reversed.reverse();
        assert!(valid_arguments(&reversed));
        for value in [-5.00001, 5.00001, f64::NAN, f64::INFINITY] {
            let mut invalid = arguments.clone();
            invalid[0].value = FilterValue::Number(ExactNumber::Float64Bits(value.to_bits()));
            assert!(!valid_arguments(&invalid));
        }
        for index in 0..4 {
            let mut invalid = arguments.clone();
            invalid[index].name = "unknown".into();
            assert!(!valid_arguments(&invalid));
            let mut invalid = arguments.clone();
            invalid[index].value = FilterValue::String("0".into());
            assert!(!valid_arguments(&invalid));
            let mut invalid = arguments.clone();
            invalid[index] = arguments[(index + 1) % 4].clone();
            assert!(!valid_arguments(&invalid));
        }
        assert!(!valid_arguments(&arguments[..3]));
    }
}
