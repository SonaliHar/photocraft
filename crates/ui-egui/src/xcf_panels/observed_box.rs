//! Bounded CURRENT Box controls; the registered transaction owns full proof.
use super::*;
use photocraft_doc::xcf_filters::FilterNode;
use photocraft_engine::DocumentSnapshot;

const EPOCH: &str = "xcf22-observed-box-geometry-v1";
pub(super) fn scope(document: &DocumentSnapshot, node: &FilterNode) -> bool {
    document.accounted().is_some_and(|owner| owner.is_readonly_point_storage() || owner.is_readonly_assigned_point_storage())
        && (node.operation.schema_epoch == EPOCH || node.composition.as_ref().is_some_and(|p| p.compatibility_epoch == EPOCH))
}
pub(super) fn radius(document: &DocumentSnapshot, layer: LayerId, node: &FilterNode) -> Option<i32> {
    if !scope(document, node) {
        return None;
    }
    let assigned = document.accounted()?.is_readonly_assigned_point_storage();
    if document.mode != photocraft_doc::ColorMode::Rgb
        || document.source_color_context.as_ref().is_none_or(|source| {
            let nonlinear = document.sample_encoding == Some(photocraft_doc::SampleEncoding::NonlinearProfile)
                && matches!((document.depth, source.source_precision), (photocraft_doc::SampleType::F32, 650) | (photocraft_doc::SampleType::F64, 750));
            let implicit_linear = !assigned && source.original_icc.is_none()
                && document.sample_encoding == Some(photocraft_doc::SampleEncoding::Linear)
                && matches!((document.depth, source.source_precision), (photocraft_doc::SampleType::F32, 600) | (photocraft_doc::SampleType::F64, 700));
            (!nonlinear && !implicit_linear) || assigned != source.original_icc.is_some()
        })
    {
        return None;
    }
    let [owner] = document.layers.as_slice() else { return None };
    let state = document.xcf_state.as_ref()?;
    let [stack] = state.filter_stacks.as_slice() else { return None };
    let [retained] = stack.nodes.as_slice() else { return None };
    let [argument] = node.arguments.as_slice() else { return None };
    if owner.id != layer
        || !matches!(owner.content, photocraft_doc::LayerContent::BackedRaster(_))
        || stack.owner != ItemRef::Layer(layer)
        || !std::ptr::eq(retained, node)
        || node.id.0 == 0
        || node.operation.name != "gegl:box-blur"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
        || node.composition.as_ref().is_none_or(|p| p.compatibility_epoch != EPOCH)
        || argument.name != "radius"
    {
        return None;
    }
    match argument.value {
        FilterValue::Number(ExactNumber::Signed { width: 32, value: value @ 0..=1000 }) => i32::try_from(value).ok(),
        _ => None,
    }
}
pub(super) fn request(node: &FilterNode, old_radius: i32, next_radius: i32, enabled: bool) -> Option<serde_json::Value> {
    if !(0..=1000).contains(&next_radius) || (old_radius == next_radius && node.enabled == enabled) {
        return None;
    }
    if old_radius == next_radius {
        Some(json!({"filterId":node.id.0.to_string(),"enabled":enabled}))
    } else {
        let mut result =
            json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":32,"value":next_radius}}}}]});
        if enabled != node.enabled {
            result.as_object_mut()?.insert("enabled".into(), json!(enabled));
        }
        Some(result)
    }
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
    let Some(old_radius) = radius(document, layer, node) else {
        ui.label(tl!("Filter editing is unavailable"));
        return;
    };
    let mut next_radius = old_radius;
    let mut enabled = node.enabled;
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled(unlocked, egui::Checkbox::new(&mut enabled, caption));
        let label = ui.label(tl!("Radius"));
        ui.add_enabled(unlocked, egui::DragValue::new(&mut next_radius).speed(1).range(0..=1000).update_while_editing(false)).labelled_by(label.id);
    });
    if let Some(params) = request(node, old_radius, next_radius, enabled) {
        run_checked(app, document, revision, "xcf.filter.set", params);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "observed_box_tests.rs"]
mod tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="assigned_f64_box_tests.rs"]
mod assigned_f64_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="linear_box_tests.rs"]
mod linear_tests;
