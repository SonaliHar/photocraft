//! CURRENT parameter controls for the genuine owned Gaussian group class only.
//! This is a bounded UI hint; registered preparation owns semantic admission.
use super::*;
use photocraft_doc::xcf_filters::FilterNode;
use photocraft_engine::DocumentSnapshot;

pub(super) fn candidate(document: &DocumentSnapshot, layer: LayerId, node: &FilterNode) -> bool {
    if !document.accounted().is_some_and(|owner| owner.is_readonly_gaussian_group_storage()) {
        return false;
    }
    let Some(state) = &document.xcf_state else { return false };
    let [stack] = state.filter_stacks.as_slice() else { return false };
    let [retained] = stack.nodes.as_slice() else { return false };
    if stack.owner != ItemRef::Layer(layer)
        || !std::ptr::eq(retained, node)
        || !document.layer(layer).is_some_and(|layer| matches!(&layer.content, photocraft_doc::LayerContent::Group(group) if group.children.len() == 2))
        || node.operation.name != "gegl:gaussian-blur"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != "xcf22-isolated-small-auto-v1"
        || node.arguments.len() != 5
    {
        return false;
    }
    let mut seen = 0u8;
    for argument in &node.arguments {
        let flag = match (argument.name.as_str(), &argument.value) {
            ("std-dev-x", FilterValue::Number(ExactNumber::Float32Bits(bits))) if valid(f32::from_bits(*bits)) => 1,
            ("std-dev-y", FilterValue::Number(ExactNumber::Float32Bits(bits))) if valid(f32::from_bits(*bits)) => 2,
            ("filter", FilterValue::Enum { domain, value: 0 }) if domain == "gegl:gaussian-blur::filter" => 4,
            ("abyss-policy", FilterValue::Enum { domain, value: 1 }) if domain == "gegl:gaussian-blur::abyss-policy" => 8,
            ("clip-extent", FilterValue::Bool(true)) => 16,
            _ => return false,
        };
        if seen & flag != 0 {
            return false;
        }
        seen |= flag;
    }
    seen == 31
}
fn valid(value: f32) -> bool {
    value.is_finite() && (0.0..=15.0).contains(&value)
}
fn parse(text: &str) -> Option<f64> {
    if text.len() > 64 {
        return None;
    }
    text.parse::<f32>().ok().filter(|v| valid(*v)).map(f64::from)
}
fn edit(ui: &mut egui::Ui, label: &str, enabled: bool, bits: u32) -> Option<u32> {
    let mut value = f32::from_bits(bits);
    if !valid(value) {
        return None;
    }
    let changed = ui
        .horizontal(|ui| {
            let label = ui.label(label);
            ui.add_enabled(
                enabled,
                egui::DragValue::new(&mut value)
                    .speed(0.01)
                    .range(0.0..=15.0)
                    .custom_formatter(|value, _| (value as f32).to_string())
                    .custom_parser(parse)
                    .update_while_editing(false),
            )
            .labelled_by(label.id)
            .changed()
        })
        .inner;
    (changed && valid(value) && value.to_bits() != bits).then_some(value.to_bits())
}
pub(super) fn controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, document: &DocumentSnapshot, revision: u64, node: &FilterNode, editable: bool) {
    for (name, label) in [("std-dev-x", tl!("Horizontal standard deviation")), ("std-dev-y", tl!("Vertical standard deviation"))] {
        let Some(FilterArgument { value: FilterValue::Number(ExactNumber::Float32Bits(bits)), .. }) =
            node.arguments.iter().find(|argument| argument.name == name)
        else {
            return;
        };
        if let Some(bits) = edit(ui, label, editable, *bits) {
            run_checked(
                app,
                document,
                revision,
                "xcf.filter.set",
                json!({"filterId":node.id.0.to_string(),"arguments":[{"name":name,"value":{"Number":{"Float32Bits":bits}}}]}),
            );
        }
    }
    ui.label(tl!("Filter: Auto"));
    ui.label(tl!("Abyss policy: Clamp"));
    ui.label(tl!("Clip extent: On"));
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "gaussian_group/parameter_tests.rs"]
mod parameter_tests;
