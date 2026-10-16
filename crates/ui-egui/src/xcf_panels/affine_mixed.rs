//! Exact affine mixed-filter widgets; registered transactions prove the result.
use super::*;
use photocraft_compose::{AffineKind, BrightnessContrastParameters, GeglLevelsParameters};

const BRIGHTNESS_NAMES: [&str; 2] = ["brightness", "contrast"];
const LEVELS_NAMES: [&str; 4] = ["in-low", "in-high", "out-low", "out-high"];

fn kind(node: &FilterNode) -> Option<AffineKind> {
    if node.operation.stored_version != "0:0" {
        return None;
    }
    let kind = match node.operation.name.as_str() {
        "gegl:brightness-contrast" => AffineKind::BrightnessContrast,
        "gegl:levels" => AffineKind::GeglLevels,
        _ => return None,
    };
    (node.operation.schema_epoch == kind.epoch() && node.composition.as_ref().is_some_and(|p| p.compatibility_epoch == kind.epoch())).then_some(kind)
}

fn request<const N: usize>(node: &FilterNode, names: [&str; N], values: [f32; N], enabled: bool) -> Option<(&'static str, serde_json::Value)> {
    if node.arguments.len() != N {
        return None;
    }
    let mut arguments = Vec::new();
    // Walk retained source order. Unchanged words, including signed zero, never
    // roundtrip through a reconstructed parameter list.
    for argument in &node.arguments {
        let index = names.iter().position(|name| *name == argument.name)?;
        let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(old)) = &argument.value else {
            return None;
        };
        let bits = values.get(index)?.to_bits();
        if bits != *old {
            arguments.push(json!({"name":argument.name,"value":{"Number":{"Float32Bits":bits}}}));
        }
    }
    if !arguments.is_empty() {
        let mut params = json!({"filterId":node.id.0.to_string(),"arguments":arguments});
        if enabled != node.enabled {
            params.as_object_mut()?.insert("enabled".into(), json!(enabled));
        }
        Some(("xcf.filter.set", params))
    } else if enabled != node.enabled {
        Some(("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":enabled})))
    } else {
        None
    }
}

fn brightness_request(node: &FilterNode, values: [f32; 2], enabled: bool) -> Option<(&'static str, serde_json::Value)> {
    if kind(node)? != AffineKind::BrightnessContrast {
        return None;
    }
    photocraft_compose::brightness_contrast_mixed_parameters(&node.arguments).ok()?;
    let [brightness, contrast] = values.map(f32::to_bits);
    BrightnessContrastParameters::from_words(brightness, contrast).ok()?;
    request(node, BRIGHTNESS_NAMES, values, enabled)
}

pub(super) fn levels_request(node: &FilterNode, values: [f32; 4], enabled: bool) -> Option<(&'static str, serde_json::Value)> {
    if kind(node)? != AffineKind::GeglLevels {
        return None;
    }
    photocraft_compose::gegl_levels_mixed_parameters(&node.arguments).ok()?;
    GeglLevelsParameters::from_words(values.map(f32::to_bits)).ok()?;
    request(node, LEVELS_NAMES, values, enabled)
}

pub(super) fn edit(ui: &mut egui::Ui, node: &FilterNode, enabled_ui: bool, caption: &str) -> Option<(&'static str, serde_json::Value)> {
    let mut enabled = node.enabled;
    match kind(node)? {
        AffineKind::BrightnessContrast => {
            let old = photocraft_compose::brightness_contrast_mixed_parameters(&node.arguments).ok()?.raw_words();
            let mut values = old.map(f32::from_bits);
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                scalar_widgets(ui, &mut values, [tl!("Brightness"), tl!("Contrast")], [(-3., 3.), (-5., 5.)]);
            });
            brightness_request(node, values, enabled)
        }
        AffineKind::GeglLevels => {
            let old = photocraft_compose::gegl_levels_mixed_parameters(&node.arguments).ok()?.raw_words();
            let mut values = old.map(f32::from_bits);
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                for (value, label) in values.iter_mut().zip([tl!("Input low"), tl!("Input high"), tl!("Output low"), tl!("Output high")]) {
                    ui.horizontal(|ui| {
                        let label = ui.label(label);
                        ui.add(egui::DragValue::new(value).speed(0.01).update_while_editing(false)).labelled_by(label.id);
                    });
                }
            });
            levels_request(node, values, enabled)
        }
    }
}
