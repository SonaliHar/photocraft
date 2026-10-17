//! Exact V2 loaded Exposure controls; display interpretation is not source authority.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
fn words(node: &FilterNode) -> Option<[u32; 2]> {
    if node.operation.name != "gegl:exposure" || node.operation.stored_version != "1:0" || node.arguments.len() != 2 {
        return None;
    }
    let mut words = [None; 2];
    for a in &node.arguments {
        let index = match a.name.as_str() {
            "exposure" => 0,
            "black-level" => 1,
            _ => return None,
        };
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = a.value else { return None };
        if !f32::from_bits(bits).is_finite() || words.get_mut(index)?.replace(bits).is_some() {
            return None;
        }
    }
    Some([words[0]?, words[1]?])
}
fn request(node: &FilterNode, values: [f32; 2], intent: [bool; 2], enabled: bool) -> Option<(&'static str, serde_json::Value)> {
    let raw = words(node)?;
    if !values.iter().all(|v| v.is_finite()) || !(-0.1f64..=0.1f64).contains(&f64::from(values[1])) {
        return None;
    }
    let mut arguments = Vec::new();
    for a in &node.arguments {
        let index = match a.name.as_str() {
            "exposure" => 0,
            "black-level" => 1,
            _ => return None,
        };
        // Only deliberate widget intent can replace a raw endpoint with zero.
        if *intent.get(index)? && values.get(index)?.to_bits() != *raw.get(index)? {
            arguments.push(json!({"name":a.name,"value":{"Number":{"Float32Bits":values.get(index)?.to_bits()}}}));
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
pub(super) fn edit(ui: &mut egui::Ui, node: &FilterNode, editable: bool, checkbox: bool) -> Option<(&'static str, serde_json::Value)> {
    let raw = words(node)?;
    // Fixed exact V2 endpoint interpretation only; no CMS/kernel work perframe.
    let initial = [f32::from_bits(raw[0]), if matches!(raw[1], 0x3dcc_cccd | 0xbdcc_cccd) { 0.0 } else { f32::from_bits(raw[1]) }];
    let mut values = initial;
    let mut changed = [false; 2];
    let mut enabled = node.enabled;
    ui.add_enabled_ui(editable, |ui| {
        if checkbox {
            let caption: String = node.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();
            ui.checkbox(&mut enabled, caption);
        }
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Exposure"));
            changed[0] = ui.add(egui::DragValue::new(&mut values[0]).speed(0.1).update_while_editing(false)).labelled_by(label.id).changed();
        });
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Black level"));
            let bound = f32::from_bits(0x3dcc_cccc);
            changed[1] =
                ui.add(egui::DragValue::new(&mut values[1]).range(-bound..=bound).speed(0.001).update_while_editing(false)).labelled_by(label.id).changed();
        });
    });
    // Comparing with effective initial values never turns passive display0 into a patch.
    let intent = changed;
    request(node, values, intent, enabled)
}
pub(super) fn controls(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    snapshot: &photocraft_engine::DocumentSnapshot,
    revision: u64,
    node: &FilterNode,
    editable: bool,
) {
    if let Some((id, params)) = edit(ui, node, editable, false) {
        run_checked(app, snapshot, revision, id, params)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "exposure_loaded_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
