//! Exact stock mixed kinds with fixed RGB/HSL models. Widgets do no CMS/gains or rendering.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Temperature,
    Sepia,
    AlienRgb,
    AlienHsl,
}
fn kind(node: &FilterNode) -> Option<Kind> {
    if node.operation.stored_version != "0:0" {
        return None;
    }
    let (kind, epoch) = match node.operation.name.as_str() {
        "gegl:color-temperature" => (Kind::Temperature, photocraft_compose::COLOR_TEMPERATURE_MIXED_EPOCH),
        "gegl:sepia" => (Kind::Sepia, photocraft_compose::SEPIA_MIXED_EPOCH),
        "gegl:alien-map" if node.operation.schema_epoch==photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH => (Kind::AlienHsl,photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH),
        "gegl:alien-map" => (Kind::AlienRgb, photocraft_compose::ALIEN_MAP_RGB_MIXED_EPOCH),
        _ => return None,
    };
    (node.operation.schema_epoch == epoch && node.composition.as_ref().is_some_and(|p| p.compatibility_epoch == epoch)).then_some(kind)
}
const TEMP: [&str; 2] = ["original-temperature", "intended-temperature"];
const FREQUENCY: [&str; 3] = ["cpn-1-frequency", "cpn-2-frequency", "cpn-3-frequency"];
const PHASE: [&str; 3] = ["cpn-1-phaseshift", "cpn-2-phaseshift", "cpn-3-phaseshift"];
const KEEP: [&str; 3] = ["cpn-1-keep", "cpn-2-keep", "cpn-3-keep"];
fn request(node: &FilterNode, numbers: &[(&str, u32)], bools: &[(&str, bool)], enabled: bool) -> Option<(&'static str, serde_json::Value)> {
    let k = kind(node)?;
    match k {
        Kind::Temperature => {
            photocraft_compose::color_temperature_mixed_parameters(&node.arguments).ok()?;
            if numbers.len() != 2
                || !bools.is_empty()
                || numbers.iter().any(|(n, b)| !TEMP.contains(n) || !f32::from_bits(*b).is_finite() || !(1000.0..=12000.0).contains(&f32::from_bits(*b)))
            {
                return None;
            }
        }
        Kind::Sepia => {
            photocraft_compose::sepia_mixed_parameters(&node.arguments).ok()?;
            if numbers.len() != 1
                || numbers[0].0 != "scale"
                || !f32::from_bits(numbers[0].1).is_finite()
                || !(0.0..=1.0).contains(&f32::from_bits(numbers[0].1))
                || bools.len() != 1
                || bools[0].0 != "srgb"
            {
                return None;
            }
        }
        Kind::AlienRgb | Kind::AlienHsl => {
            if k==Kind::AlienRgb {photocraft_compose::alien_map_rgb_mixed_parameters(&node.arguments).ok()?;}else{photocraft_compose::alien_map_hsl_mixed_parameters(&node.arguments).ok()?;}
            if numbers.len() != 6 || bools.len() != 3 {
                return None;
            }
            for (n, b) in numbers {
                let v = f32::from_bits(*b);
                let max = if FREQUENCY.contains(n) {
                    20.
                } else if PHASE.contains(n) {
                    360.
                } else {
                    return None;
                };
                if !v.is_finite() || !(0.0..=max).contains(&v) {
                    return None;
                }
            }
            if bools.iter().any(|(n, _)| !KEEP.contains(n)) {
                return None;
            }
        }
    }
    // Only fixed local tuples enter here. Reject duplicates before source-order lookup.
    if numbers.iter().enumerate().any(|(i, (n, _))| numbers.iter().take(i).any(|(old, _)| old == n))
        || bools.iter().enumerate().any(|(i, (n, _))| bools.iter().take(i).any(|(old, _)| old == n))
    {
        return None;
    }
    let mut arguments = Vec::new();
    for a in &node.arguments {
        let value = match &a.value {
            FilterValue::Number(ExactNumber::Float32Bits(old)) => {
                let bits = numbers.iter().find(|(n, _)| *n == a.name)?.1;
                if bits == *old {
                    continue;
                }
                json!({"Number":{"Float32Bits":bits}})
            }
            FilterValue::Bool(old) => {
                let value = bools.iter().find(|(n, _)| *n == a.name)?.1;
                if value == *old {
                    continue;
                }
                json!({"Bool":value})
            }
            FilterValue::Enum { domain, value } if matches!(k,Kind::AlienRgb|Kind::AlienHsl) && *value==if k==Kind::AlienRgb{0}else{1} && a.name == "color-model" && domain == "gegl:alien-map/color-model" => continue,
            _ => return None,
        };
        arguments.push(json!({"name":a.name,"value":value}));
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
pub(super) fn edit(ui: &mut egui::Ui, node: &FilterNode, enabled_ui: bool, caption: &str) -> Option<(&'static str, serde_json::Value)> {
    let mut enabled = node.enabled;
    match kind(node)? {
        Kind::Temperature => {
            let old = photocraft_compose::color_temperature_mixed_parameters(&node.arguments).ok()?.raw_words();
            let mut v = old.map(f32::from_bits);
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                scalar_widgets(ui, &mut v, [tl!("Original temperature"), tl!("Intended temperature")], [(1000., 12000.); 2]);
            });
            request(node, &[(TEMP[0], v[0].to_bits()), (TEMP[1], v[1].to_bits())], &[], enabled)
        }
        Kind::Sepia => {
            let old = photocraft_compose::sepia_mixed_parameters(&node.arguments).ok()?;
            let mut scale = [f32::from_bits(old.raw_scale_word())];
            let mut srgb = old.srgb();
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                scalar_widgets(ui, &mut scale, [tl!("Scale")], [(0., 1.)]);
                ui.checkbox(&mut srgb, tl!("Use sRGB"));
            });
            request(node, &[("scale", scale[0].to_bits())], &[("srgb", srgb)], enabled)
        }
        Kind::AlienRgb | Kind::AlienHsl => {
            let k=kind(node)?;
            let ((f,p),initial_keep)=if k==Kind::AlienRgb {
                let old=photocraft_compose::alien_map_rgb_mixed_parameters(&node.arguments).ok()?;(old.raw_words(),old.keep())
            }else{
                let old=photocraft_compose::alien_map_hsl_mixed_parameters(&node.arguments).ok()?;(old.raw_words(),old.keep())
            };
            let mut f = f.map(f32::from_bits);
            let mut p = p.map(f32::from_bits);
            let mut keep = initial_keep;
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                ui.label(if k==Kind::AlienRgb{tl!("RGB")}else{tl!("HSL")});
                scalar_widgets(ui, &mut f, [tl!("Channel 1 frequency"), tl!("Channel 2 frequency"), tl!("Channel 3 frequency")], [(0., 20.); 3]);
                scalar_widgets(ui, &mut p, [tl!("Channel 1 phase"), tl!("Channel 2 phase"), tl!("Channel 3 phase")], [(0., 360.); 3]);
                for (value, label) in keep.iter_mut().zip([tl!("Keep channel 1"), tl!("Keep channel 2"), tl!("Keep channel 3")]) {
                    ui.checkbox(value, label);
                }
            });
            request(
                node,
                &[
                    (FREQUENCY[0], f[0].to_bits()),
                    (FREQUENCY[1], f[1].to_bits()),
                    (FREQUENCY[2], f[2].to_bits()),
                    (PHASE[0], p[0].to_bits()),
                    (PHASE[1], p[1].to_bits()),
                    (PHASE[2], p[2].to_bits()),
                ],
                &[(KEEP[0], keep[0]), (KEEP[1], keep[1]), (KEEP[2], keep[2])],
                enabled,
            )
        }
    }
}

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="derived_rgb_mixed_tests.rs"]
mod tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="derived_hsl_mixed_tests.rs"]
mod hsl_tests;
