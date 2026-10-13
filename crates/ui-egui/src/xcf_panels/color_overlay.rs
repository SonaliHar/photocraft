//! The qualified Color Overlay epoch stores nonlinear sRGB literal operands,
//! including on Gray owners. Swatches use CMS; edits retain untouched native operand bits.
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

pub(super) const EPOCH: &str = "xcf-22-stock-color-overlay-six-precision-v1";

pub(super) fn valid_arguments(arguments: &[FilterArgument]) -> bool {
    arguments.len() == 2
        && ["value", "srgb"].into_iter().all(|name| {
            let mut values = arguments.iter().filter(|argument| argument.name == name);
            let Some(argument) = values.next() else { return false };
            if values.next().is_some() {
                return false;
            }
            if name == "value" { bits(&argument.value).is_some() } else { matches!(argument.value, FilterValue::Bool(_)) }
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

pub(super) fn patch(original: &FilterValue, rgba: [f32; 4]) -> Option<FilterValue> {
    if rgba.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return None;
    }
    let mut bits = bits(original)?;
    for (target, value) in bits.iter_mut().zip(rgba) {
        *target = value.to_bits();
    }
    Some(FilterValue::Color { components: bits.map(ExactNumber::Float32Bits).to_vec(), encoding: SampleEncoding::NonlinearProfile, profile: None })
}

/// Floating operand controls dispatch only actual value edits. The standard
/// HSV picker can mark a conversion round trip as changed merely on opening.
pub(super) fn edit(ui: &mut egui::Ui, original: &FilterValue, label: egui::Id, name: &str) -> Option<FilterValue> {
    let original_bits = bits(original)?;
    let display = picker(original)?;
    let mut rgba = [0.; 4];
    for (target, value) in rgba.iter_mut().zip(original_bits) {
        *target = f32::from_bits(value);
    }
    let color = egui::Color32::from(egui::Rgba::from_rgb(display[0], display[1], display[2]));
    let response = ui.add(egui::Button::new("").fill(color).min_size(ui.spacing().interact_size));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), name));
    let response = response.labelled_by(label);
    let mut changed = false;
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        for (value, name) in rgba.iter_mut().zip([tl!("Red"), tl!("Green"), tl!("Blue"), tl!("Alpha")]) {
            ui.horizontal(|ui| {
                let label = ui.label(name);
                changed |= ui.add(egui::DragValue::new(value).range(0.0..=1.0).speed(0.01)).labelled_by(label.id).changed();
            });
        }
    });
    if changed { patch(original, rgba) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn color() -> FilterValue {
        FilterValue::Color {
            components: [0.25_f32, 0.5, 0.75, -0.0].map(|value| ExactNumber::Float32Bits(value.to_bits())).to_vec(),
            encoding: SampleEncoding::NonlinearProfile,
            profile: None,
        }
    }
    #[test]
    fn rgba_changes_preserve_untouched_wire_bits_and_declared_swatch_transfer() {
        let original = color();
        assert_eq!(patch(&original, [0.25, 0.5, 0.75, -0.0]).unwrap(), original);
        let actual = picker(&original).unwrap();
        assert!((actual[0] - 0.05087609).abs() < 2e-6);
        let edited = patch(&original, [0.25, 0.5, 0.75, 1.]).unwrap();
        let FilterValue::Color { components, .. } = edited else { panic!("typed literal") };
        let FilterValue::Color { components: before, .. } = original else { panic!("typed literal") };
        assert_eq!(&components[..3], &before[..3]);
        assert_eq!(components[3], ExactNumber::Float32Bits(1_f32.to_bits()));
    }
    #[test]
    fn exact_two_argument_schema_rejects_wrong_types_domains_and_duplicates() {
        let args = vec![FilterArgument { name: "value".into(), value: color() }, FilterArgument { name: "srgb".into(), value: FilterValue::Bool(false) }];
        assert!(valid_arguments(&args));
        assert!(!valid_arguments(&args[..1]));
        for which in 0..12 {
            let mut bad = args.clone();
            match which {
                0 => bad.push(bad[0].clone()),
                1 => bad[1].name = "value".into(),
                2 => bad[0].name = "color".into(),
                3 => bad[1].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
                _ => {
                    let FilterValue::Color { components, encoding, profile } = &mut bad[0].value else { panic!("literal") };
                    match which {
                        4 => {
                            components.pop();
                        }
                        5 => components.push(ExactNumber::Float32Bits(0)),
                        6 => components[3] = ExactNumber::Float64Bits(0),
                        7 => components[3] = ExactNumber::Float32Bits(f32::NAN.to_bits()),
                        8 => components[3] = ExactNumber::Float32Bits(f32::INFINITY.to_bits()),
                        9 => components[3] = ExactNumber::Float32Bits(1.01_f32.to_bits()),
                        10 => *encoding = SampleEncoding::Linear,
                        _ => *profile = Some(photocraft_doc::item_identity::ResourceId(1)),
                    }
                }
            }
            assert!(!valid_arguments(&bad), "{which}");
        }
        for rgba in [[f32::NAN, 0., 0., 0.], [0., 0., 0., f32::INFINITY], [0., 0., 0., -0.01], [1.01, 0., 0., 0.]] {
            assert!(patch(&color(), rgba).is_none());
        }
    }
}
