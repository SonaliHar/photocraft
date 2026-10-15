//! Owned Hue-Chroma transport controls; the registered transaction owns full proof.
use super::*;
use photocraft_doc::{SampleEncoding, xcf_filters::FilterNode};
use photocraft_engine::DocumentSnapshot;

const EPOCH: &str = "xcf22-gegl-hue-chroma-lch-float-v1";
const NAMES: [&str; 3] = ["hue", "chroma", "lightness"];
pub(super) fn scope(document: &DocumentSnapshot, node: &FilterNode) -> bool {
    document.accounted().is_some_and(|owner| owner.is_readonly_point_storage()) && node.operation.name == "gegl:hue-chroma"
}
fn valid(index: usize, value: f32) -> bool {
    value.is_finite() && if index == 0 { (-180.0..=180.0).contains(&value) } else { (-100.0..=100.0).contains(&value) }
}
fn words(node: &FilterNode) -> Option<[u32; 3]> {
    if node.arguments.len() != 3 {
        return None;
    }
    let mut result = [0; 3];
    let mut seen = [false; 3];
    for argument in &node.arguments {
        let index = NAMES.iter().position(|name| *name == argument.name)?;
        let FilterValue::Number(ExactNumber::Float32Bits(raw)) = &argument.value else { return None };
        if *seen.get(index)? || !valid(index, f32::from_bits(*raw)) {
            return None;
        }
        *seen.get_mut(index)? = true;
        *result.get_mut(index)? = *raw;
    }
    seen.iter().all(|value| *value).then_some(result)
}
pub(super) fn candidate(document: &DocumentSnapshot, layer: LayerId, node: &FilterNode) -> bool {
    if !scope(document, node)
        || document.mode != photocraft_doc::ColorMode::Rgb
        || document.size != photocraft_doc::Size::new(8, 6)
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
    owner.id == layer
        && backed.operative_bounds() == document.bounds()
        && stack.owner == ItemRef::Layer(layer)
        && std::ptr::eq(retained, node)
        && node.id.0 != 0
        && node.operation.stored_version == "0:0"
        && node.operation.schema_epoch == EPOCH
        && node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == EPOCH)
        && words(node).is_some()
}
pub(super) fn request(node: &FilterNode, values: [f32; 3], enabled: bool) -> Option<(&'static str, serde_json::Value)> {
    let old = words(node)?;
    if values.iter().enumerate().any(|(index, value)| !valid(index, *value)) {
        return None;
    }
    let next = values.map(f32::to_bits);
    if old != next {
        // Preserve stored argument order. Only changed words enter the atomic patch.
        let arguments: Vec<_> = node
            .arguments
            .iter()
            .filter_map(|argument| {
                let index = NAMES.iter().position(|name| *name == argument.name)?;
                let raw = *next.get(index)?;
                (*old.get(index)? != raw).then(|| json!({"name":argument.name,"value":{"Number":{"Float32Bits":raw}}}))
            })
            .collect();
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
    let Some(raw) = words(node) else { return };
    let mut values = raw.map(f32::from_bits);
    let mut enabled = node.enabled;
    let mut changed = false;
    ui.add_enabled_ui(unlocked, |ui| {
        changed |= ui.checkbox(&mut enabled, caption).changed();
        for (index, (value, label)) in values.iter_mut().zip([tl!("Hue"), tl!("Chroma"), tl!("Lightness")]).enumerate() {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                let range = if index == 0 { -180.0..=180.0 } else { -100.0..=100.0 };
                changed |= ui.add(egui::DragValue::new(value).range(range).speed(0.1).update_while_editing(false)).labelled_by(label.id).changed();
            });
        }
    });
    if changed {
        if let Some((command, params)) = request(node, values, enabled) {
            run_checked(app, document, revision, command, params);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "hue_chroma_tests.rs"]
mod tests;
