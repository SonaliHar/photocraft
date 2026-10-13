//! The qualified Color Exchange epoch stores nonlinear sRGB literal operands,
//! including on Gray owners. Swatches use CMS; edits retain native operand bits.
use photocraft_cms::{
    Builtin,
    transform::{TransformOptions, cached},
};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};

fn bits(value: &FilterValue) -> Option<[u32; 4]> {
    let FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None } = value else { return None };
    if components.len() != 4 {
        return None;
    }
    let mut result = [0; 4];
    for (target, component) in result.iter_mut().zip(components) {
        let ExactNumber::Float32Bits(bits) = component else { return None };
        let value = f32::from_bits(*bits);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return None;
        }
        *target = *bits;
    }
    Some(result)
}

pub(super) fn valid_arguments(arguments: &[FilterArgument]) -> bool {
    if arguments.len() != 5 {
        return false;
    }
    ["from-color", "to-color", "red-threshold", "green-threshold", "blue-threshold"].into_iter().all(|name| {
        let mut values=arguments.iter().filter(|argument| argument.name==name);
        let Some(argument)=values.next() else { return false };
        if values.next().is_some() { return false; }
        if matches!(name,"from-color"|"to-color") { bits(&argument.value).is_some() }
        else { matches!(argument.value,FilterValue::Number(ExactNumber::Float32Bits(bits)) if f32::from_bits(bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(bits))) }
    })
}

/// Exact Color to Alpha schema; Color Exchange keeps its separate five fields.
pub(super) fn valid_color_to_alpha_arguments(arguments: &[FilterArgument]) -> bool {
    arguments.len() == 3 && ["color", "transparency-threshold", "opacity-threshold"].into_iter().all(|name| {
        let mut matches = arguments.iter().filter(|argument| argument.name == name);
        let Some(argument) = matches.next() else { return false };
        if matches.next().is_some() { return false; }
        if name == "color" { bits(&argument.value).is_some() }
        else { matches!(argument.value, FilterValue::Number(ExactNumber::Float32Bits(bits)) if f32::from_bits(bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(bits))) }
    })
}

pub(super) fn picker(value: &FilterValue) -> Option<[f32; 3]> {
    let bits = bits(value)?;
    let transform = cached(Builtin::Srgb.profile(), Builtin::LinearSrgb.profile(), TransformOptions { bpc: false, ..Default::default() }).ok()?;
    let mut result = [0.; 3];
    let input = bits.map(|bits| f64::from(f32::from_bits(bits)));
    transform.try_eval_f64(input.get(..3)?, &mut result).ok()?;
    result.iter().all(|value| value.is_finite()).then(|| result.map(|value| value.clamp(0., 1.) as f32))
}

pub(super) fn patch(original: &FilterValue, rgb: [f32; 3]) -> Option<FilterValue> {
    if rgb.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return None;
    }
    let mut bits = bits(original)?;
    for (target, value) in bits.iter_mut().take(3).zip(rgb) {
        *target = value.to_bits();
    }
    // Alpha is a retained source operand, not controlled by this RGB picker.
    Some(FilterValue::Color { components: bits.map(ExactNumber::Float32Bits).to_vec(), encoding: SampleEncoding::NonlinearProfile, profile: None })
}

/// Floating operand controls dispatch only actual value edits. The standard
/// HSV picker can mark a conversion round trip as changed merely on opening.
pub(super) fn edit(ui: &mut egui::Ui, original: &FilterValue, label: egui::Id, name: &str) -> Option<FilterValue> {
    let original_bits = bits(original)?;
    let display = picker(original)?;
    let mut rgb = [0.; 3];
    for (target, value) in rgb.iter_mut().zip(original_bits) {
        *target = f32::from_bits(value);
    }
    let color = egui::Color32::from(egui::Rgba::from_rgb(display[0], display[1], display[2]));
    let response = ui.add(egui::Button::new("").fill(color).min_size(ui.spacing().interact_size));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), name));
    let response = response.labelled_by(label);
    let mut changed = false;
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        for (value, name) in rgb.iter_mut().zip([tl!("Red"), tl!("Green"), tl!("Blue")]) {
            ui.horizontal(|ui| {
                let label = ui.label(name);
                changed |= ui.add(egui::DragValue::new(value).range(0.0..=1.0).speed(0.01)).labelled_by(label.id).changed();
            });
        }
    });
    if changed { patch(original, rgb) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn color() -> FilterValue {
        FilterValue::Color {
            components: [0.5_f32, 0.25, 0.75, -0.0].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
            encoding: SampleEncoding::NonlinearProfile,
            profile: None,
        }
    }
    #[test]
    fn picker_uses_declared_transfer_and_preserves_alpha_and_unchanged_bits() {
        let original = color();
        let rgb = picker(&original).unwrap();
        assert!((rgb[0] - 0.21404114).abs() < 2e-6);
        assert!((rgb[1] - 0.05087609).abs() < 2e-6);
        assert_eq!(patch(&original, [0.5, 0.25, 0.75]).unwrap(), original);
        let edited = patch(&original, [0., 1., 0.]).unwrap();
        let FilterValue::Color { components, .. } = &edited else { panic!("typed literal") };
        assert_eq!(components[3], ExactNumber::Float32Bits((-0.0_f32).to_bits()));
        let one_component = patch(&original, [0.5, 0.25, 0.]).unwrap();
        let FilterValue::Color { components, .. } = one_component else { panic!("typed literal") };
        assert_eq!(&components[..2], &[ExactNumber::Float32Bits(0.5_f32.to_bits()), ExactNumber::Float32Bits(0.25_f32.to_bits())]);
        let wire = serde_json::to_value(&edited).unwrap();
        assert_eq!(wire["Color"]["encoding"], "nonlinear_profile");
        assert!(wire["Color"]["profile"].is_null());
        assert_eq!(serde_json::from_value::<FilterValue>(wire).unwrap(), edited);
    }
    #[test]
    fn malformed_color_and_picker_values_do_not_produce_patches() {
        for rgb in [[f32::NAN, 0., 0.], [f32::INFINITY, 0., 0.], [-0.01, 0., 0.], [1.001, 0., 0.]] {
            assert!(patch(&color(), rgb).is_none());
        }
        for mutation in 0..7 {
            let mut value = color();
            let FilterValue::Color { components, encoding, profile } = &mut value else { panic!("typed literal") };
            match mutation {
                0 => {
                    components.pop();
                }
                1 => components.push(ExactNumber::Float32Bits(0)),
                2 => components[0] = ExactNumber::Float64Bits(0),
                3 => components[3] = ExactNumber::Float32Bits(f32::NAN.to_bits()),
                4 => components[0] = ExactNumber::Float32Bits(1.01_f32.to_bits()),
                5 => *encoding = SampleEncoding::Linear,
                _ => *profile = Some(photocraft_doc::item_identity::ResourceId(1)),
            }
            assert!(picker(&value).is_none());
            assert!(patch(&value, [0., 0., 0.]).is_none());
        }
    }
    #[test]
    fn all_literal_arguments_must_be_unique_and_admitted_before_controls_enable() {
        let mut arguments: Vec<_> = ["from-color", "to-color"].into_iter().map(|name| FilterArgument { name: name.into(), value: color() }).collect();
        arguments.extend(
            ["red-threshold", "green-threshold", "blue-threshold"]
                .into_iter()
                .map(|name| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25_f32.to_bits())) }),
        );
        assert!(valid_arguments(&arguments));
        let original = arguments.clone();
        arguments[4].name = "red-threshold".into();
        assert!(!valid_arguments(&arguments));
        for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.1_f32).to_bits(), 1.01_f32.to_bits()] {
            let mut bad = original.clone();
            bad[2].value = FilterValue::Number(ExactNumber::Float32Bits(bits));
            assert!(!valid_arguments(&bad));
        }
        assert!(!valid_arguments(&original[..4]));
    }
    #[test]
    fn color_to_alpha_requires_separate_exact_three_field_schema() {
        let original = vec![
            FilterArgument { name: "color".into(), value: color() },
            FilterArgument { name: "transparency-threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0)) },
            FilterArgument { name: "opacity-threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1_f32.to_bits())) },
        ];
        assert!(valid_color_to_alpha_arguments(&original));
        assert!(!valid_arguments(&original), "Color Exchange must keep its exact five-field schema");
        for mutation in 0..10 {
            let mut bad = original.clone();
            match mutation {
                0 => {
                    bad.pop();
                }
                1 => bad.push(bad[0].clone()),
                2 => bad[2].name = "transparency-threshold".into(),
                3 => bad[0].name = "from-color".into(),
                4 => bad[1].value = FilterValue::Number(ExactNumber::Float64Bits(0)),
                5 => bad[1].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
                6 => bad[2].value = FilterValue::Number(ExactNumber::Float32Bits(f32::INFINITY.to_bits())),
                7 => bad[1].value = FilterValue::Number(ExactNumber::Float32Bits((-0.01_f32).to_bits())),
                8 => bad[2].value = FilterValue::Number(ExactNumber::Float32Bits(1.01_f32.to_bits())),
                _ => bad[0].value = FilterValue::Color { components: vec![ExactNumber::Float32Bits(0); 4], encoding: SampleEncoding::Linear, profile: None },
            }
            assert!(!valid_color_to_alpha_arguments(&bad), "mutation {mutation}");
        }
        let mut reordered = original;
        reordered.reverse();
        assert!(valid_color_to_alpha_arguments(&reordered));
    }
}
