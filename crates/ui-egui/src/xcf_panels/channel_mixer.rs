//! Exact signed Channel Mixer gains retain their canonical stored numeric width.
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
use serde_json::{Value, json};

pub(super) const EPOCH: &str = "xcf-22-stock-channel-mixer-precision-v1";

fn gain(number: &ExactNumber) -> Option<f64> {
    let value = match number {
        ExactNumber::Float32Bits(bits) => f64::from(f32::from_bits(*bits)),
        ExactNumber::Float64Bits(bits) => f64::from_bits(*bits),
        _ => return None,
    };
    (value.is_finite() && (-2.0..=2.0).contains(&value)).then_some(value)
}

pub(super) fn valid_arguments(arguments: &[FilterArgument]) -> bool {
    arguments.len() == 10
        && ["rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain", "preserve-luminosity"].into_iter().all(|name| {
            let mut matches = arguments.iter().filter(|argument| argument.name == name);
            let Some(argument) = matches.next() else { return false };
            matches.next().is_none()
                && match &argument.value {
                    FilterValue::Bool(_) => name == "preserve-luminosity",
                    FilterValue::Number(number) => name != "preserve-luminosity" && gain(number).is_some(),
                    _ => false,
                }
        })
}

fn parse_gain(text: &str) -> Option<f64> {
    if text.len() > 64 {
        return None;
    }
    let value = text.trim().parse::<f64>().ok()?;
    (value.is_finite() && (-2.0..=2.0).contains(&value)).then_some(value)
}

pub(super) fn edit_gain(ui: &mut egui::Ui, label: &str, editable: bool, number: &ExactNumber) -> Option<Value> {
    gain(number)?;
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
                            .range(-2.0..=2.0)
                            .custom_parser(parse_gain)
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
                            .range(-2.0..=2.0)
                            .custom_parser(parse_gain)
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
    fn channel_gain_schema_keeps_widths_signed_zero_and_refuses_malformed() {
        let mut args: Vec<_> = ["rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain"]
            .into_iter()
            .map(|name| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.0_f32).to_bits())) })
            .collect();
        args.push(FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(false) });
        assert!(valid_arguments(&args));
        args[0].value = FilterValue::Number(ExactNumber::Float64Bits((-2.0_f64).to_bits()));
        assert!(valid_arguments(&args));
        for v in [f64::NAN, f64::INFINITY, -2.0000001, 2.0000001] {
            let mut invalid = args.clone();
            invalid[0].value = FilterValue::Number(ExactNumber::Float64Bits(v.to_bits()));
            assert!(!valid_arguments(&invalid));
        }
        let mut invalid = args.clone();
        invalid[0] = args[1].clone();
        assert!(!valid_arguments(&invalid));
        let mut invalid = args.clone();
        invalid[0].value = FilterValue::Bool(false);
        assert!(!valid_arguments(&invalid));
        assert!(!valid_arguments(&args[..9]));
    }
}
