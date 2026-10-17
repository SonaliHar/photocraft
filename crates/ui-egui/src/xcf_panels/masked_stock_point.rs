//! Exact ordinary Point mask controls; no per-frame mask/CMS work or opacity grant.
use super::*;
use photocraft_compose::MaskedScalarKind as K;
use photocraft_doc::xcf_filters::{ExactNumber, FilterNode, FilterValue};
const BC: [&str; 2] = ["brightness", "contrast"];
const LEVELS: [&str; 4] = ["in-low", "in-high", "out-low", "out-high"];
fn words<const N: usize>(node: &FilterNode, names: [&str; N]) -> Option<[u32; N]> {
    if node.arguments.len() != N {
        return None;
    }
    let mut out = [None; N];
    for a in &node.arguments {
        let at = names.iter().position(|n| *n == a.name)?;
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = a.value else { return None };
        if !f32::from_bits(bits).is_finite() || out.get_mut(at)?.replace(bits).is_some() {
            return None;
        }
    }
    let mut result = [0; N];
    for (out, v) in result.iter_mut().zip(out) {
        *out = v?
    }
    Some(result)
}
fn request(node: &FilterNode, kind: K, values: &[f32], intent: &[bool], enabled: bool) -> Option<(&'static str, serde_json::Value)> {
    if values.len() != intent.len() {
        return None;
    }
    let names: &[&str] = match kind {
        K::InvertLinear | K::InvertGamma => {
            if !node.arguments.is_empty() || !values.is_empty() {
                return None;
            }
            &[]
        }
        K::BrightnessContrast => {
            let [brightness, contrast] = values else { return None };
            photocraft_compose::BrightnessContrastParameters::from_words(brightness.to_bits(), contrast.to_bits()).ok()?;
            &BC
        }
        K::GeglLevels => {
            let [a, b, c, d] = values else { return None };
            let words = [a.to_bits(), b.to_bits(), c.to_bits(), d.to_bits()];
            photocraft_compose::GeglLevelsParameters::from_words(words).ok()?;
            &LEVELS
        }
    };
    if node.arguments.len() != names.len() {
        return None;
    }
    let mut arguments = Vec::new();
    for a in &node.arguments {
        let i = names.iter().position(|n| *n == a.name)?;
        let FilterValue::Number(ExactNumber::Float32Bits(old)) = a.value else { return None };
        let v = values.get(i)?;
        if *intent.get(i)? && v.to_bits() != old {
            arguments.push(json!({"name":a.name,"value":{"Number":{"Float32Bits":v.to_bits()}}}));
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
pub(super) fn scope(snapshot: &photocraft_engine::DocumentSnapshot, node: &FilterNode) -> bool {
    let epoch = |s: &str| {
        matches!(
            s,
            photocraft_compose::INVERT_LINEAR_MASKED_EPOCH
                | photocraft_compose::INVERT_GAMMA_MASKED_EPOCH
                | photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_EPOCH
                | photocraft_compose::GEGL_LEVELS_MASKED_EPOCH
        )
    };
    snapshot.accounted().is_some_and(|o| o.is_readonly_point_storage())
        && (epoch(&node.operation.schema_epoch) || node.composition.as_ref().is_some_and(|p| epoch(&p.compatibility_epoch)))
}
pub(super) fn controls(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    snapshot: &photocraft_engine::DocumentSnapshot,
    revision: u64,
    node: &FilterNode,
    unlocked: bool,
) {
    let Some(kind) = snapshot.accounted().and_then(|o| photocraft_engine::xcf_filter_cmds::masked_scalar_point_edit_kind(o, node)) else {
        ui.label(tl!("Filter editing is unavailable"));
        return;
    };
    let caption: String = node.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();
    let mut enabled = node.enabled;
    let command = match kind {
        K::InvertLinear | K::InvertGamma => {
            ui.add_enabled_ui(unlocked, |ui| {
                ui.checkbox(&mut enabled, caption);
                ui.label(tl!("No parameters"));
            });
            request(node, kind, &[], &[], enabled)
        }
        K::BrightnessContrast => {
            let Some(old) = words(node, BC) else { return };
            let mut v = old.map(f32::from_bits);
            let mut intent = [false; 2];
            ui.add_enabled_ui(unlocked, |ui| {
                ui.checkbox(&mut enabled, caption);
                for (i, (value, (label, range))) in v.iter_mut().zip([(tl!("Brightness"), -3.0..=3.0), (tl!("Contrast"), -5.0..=5.0)]).enumerate() {
                    ui.horizontal(|ui| {
                        let label = ui.label(label);
                        intent[i] = ui.add(egui::DragValue::new(value).range(range).speed(0.01).update_while_editing(false)).labelled_by(label.id).changed();
                    });
                }
            });
            request(node, kind, &v, &intent, enabled)
        }
        K::GeglLevels => {
            let Some(old) = words(node, LEVELS) else { return };
            let mut v = old.map(f32::from_bits);
            let mut intent = [false; 4];
            ui.add_enabled_ui(unlocked, |ui| {
                ui.checkbox(&mut enabled, caption);
                for (i, (value, label)) in v.iter_mut().zip([tl!("Input low"), tl!("Input high"), tl!("Output low"), tl!("Output high")]).enumerate() {
                    ui.horizontal(|ui| {
                        let label = ui.label(label);
                        intent[i] = ui.add(egui::DragValue::new(value).speed(0.01).update_while_editing(false)).labelled_by(label.id).changed();
                    });
                }
            });
            request(node, kind, &v, &intent, enabled)
        }
    };
    if let Some((id, params)) = command {
        run_checked(app, snapshot, revision, id, params)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "masked_scalar_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
