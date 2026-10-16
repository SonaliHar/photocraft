//! Narrow owned Semi-Flatten controls. The registered transaction owns full proof.
use super::*;
use photocraft_doc::{SampleEncoding, xcf_filters::FilterNode};
use photocraft_engine::DocumentSnapshot;

const EPOCH: &str = "xcf22-gimp-semi-flatten-literal-v1";
pub(super) fn scope(document: &DocumentSnapshot, node: &FilterNode) -> bool {
    document.accounted().is_some_and(|owner| owner.is_readonly_point_storage()) && node.operation.name == "gimp:semi-flatten"
}
pub(super) fn bits(value: &FilterValue) -> Option<([u32; 4], SampleEncoding)> {
    let FilterValue::Color { components, encoding, profile: None } = value else { return None };
    if !matches!(encoding, SampleEncoding::Linear | SampleEncoding::NonlinearProfile) || components.len() != 4 {
        return None;
    }
    let mut words = [0; 4];
    for (word, component) in words.iter_mut().zip(components) {
        let ExactNumber::Float32Bits(raw) = component else { return None };
        let value = f32::from_bits(*raw);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return None;
        }
        *word = *raw;
    }
    Some((words, *encoding))
}
pub(super) fn candidate(document: &DocumentSnapshot, layer: LayerId, node: &FilterNode) -> bool {
    if !scope(document, node)
        || document.mode != photocraft_doc::ColorMode::Rgb
        || document.size.is_empty()
        || document.size.width > 262_144
        || document.size.height > 262_144
        || document.size.area() > (1 << 30)
        || document.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || document.source_color_context.as_ref().is_none_or(|source| {
            source.original_icc.is_some()
                || !matches!((document.depth, source.source_precision), (photocraft_doc::SampleType::F32, 650) | (photocraft_doc::SampleType::F64, 750))
        })
    {
        return false;
    }
    let [owner] = document.layers.as_slice() else { return false };
    let photocraft_doc::LayerContent::BackedRaster(backed) = &owner.content else { return false };
    let Some(state) = document.xcf_state.as_ref() else { return false };
    let [stack] = state.filter_stacks.as_slice() else { return false };
    let [retained] = stack.nodes.as_slice() else { return false };
    let [argument] = node.arguments.as_slice() else { return false };
    owner.id == layer
        && backed.operative_bounds() == document.bounds()
        && stack.owner == ItemRef::Layer(layer)
        && std::ptr::eq(retained, node)
        && node.id.0 != 0
        && node.operation.stored_version == "0:0"
        && node.operation.schema_epoch == EPOCH
        && node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == EPOCH)
        && argument.name == "color"
        && bits(&argument.value).is_some()
}
pub(super) fn patch(original: &FilterValue, rgb: [f32; 3]) -> Option<FilterValue> {
    if rgb.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return None;
    }
    let (mut words, encoding) = bits(original)?;
    for (word, value) in words.iter_mut().take(3).zip(rgb) {
        *word = value.to_bits();
    }
    // Alpha is ignored by this operation but retained exactly, including signed zero.
    Some(FilterValue::Color { components: words.map(ExactNumber::Float32Bits).to_vec(), encoding, profile: None })
}
pub(super) fn picker(value: &FilterValue) -> Option<[f32; 3]> {
    use photocraft_cms::{
        Builtin,
        transform::{TransformOptions, cached},
    };
    let (words, encoding) = bits(value)?;
    let source = if encoding == SampleEncoding::Linear { Builtin::LinearSrgb } else { Builtin::Srgb };
    let transform = cached(source.profile(), Builtin::LinearSrgb.profile(), TransformOptions { bpc: false, ..Default::default() }).ok()?;
    let input = words.map(|word| f64::from(f32::from_bits(word)));
    let mut output = [0.; 3];
    transform.try_eval_f64(input.get(..3)?, &mut output).ok()?;
    output.iter().all(|v| v.is_finite()).then(|| output.map(|v| v.clamp(0., 1.) as f32))
}
fn edit(ui: &mut egui::Ui, original: &FilterValue) -> Option<FilterValue> {
    let (words, _) = bits(original)?;
    let display = picker(original)?;
    let mut rgb = [f32::from_bits(words[0]), f32::from_bits(words[1]), f32::from_bits(words[2])];
    let color = egui::Color32::from(egui::Rgba::from_rgb(display[0], display[1], display[2]));
    let response = ui.add(egui::Button::new("").fill(color).min_size(ui.spacing().interact_size));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), tl!("Color")));
    let mut changed = false;
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        for (value, label) in rgb.iter_mut().zip([tl!("Red"), tl!("Green"), tl!("Blue")]) {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                changed |= ui.add(egui::DragValue::new(value).range(0.0..=1.0).speed(0.01)).labelled_by(label.id).changed();
            });
        }
    });
    if changed { patch(original, rgb) } else { None }
}
pub(super) fn controls(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    document: &DocumentSnapshot,
    revision: u64,
    layer: LayerId,
    node: &FilterNode,
    unlocked: bool,
    caption: &str,
) {
    if !candidate(document, layer, node) {
        ui.label(tl!("Filter editing is unavailable"));
        return;
    }
    let Some(argument) = node.arguments.first() else { return };
    let mut enabled = node.enabled;
    let mut color = None;
    ui.add_enabled_ui(unlocked, |ui| {
        ui.checkbox(&mut enabled, caption);
        ui.horizontal(|ui| {
            ui.label(tl!("Color"));
            color = edit(ui, &argument.value);
        });
    });
    if let Some(value) = color.filter(|value| value != &argument.value) {
        let mut params = json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"color","value":value}]});
        if enabled != node.enabled {
            if let Some(object) = params.as_object_mut() {
                object.insert("enabled".into(), json!(enabled));
            }
        }
        run_checked(app, document, revision, "xcf.filter.set", params);
    } else if enabled != node.enabled {
        run_checked(app, document, revision, "xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":enabled}));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "semi_flatten_tests.rs"]
mod tests;
