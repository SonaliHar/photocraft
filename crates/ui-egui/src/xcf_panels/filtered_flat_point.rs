//! Targeted mixed-root controls. Registered transactions own all pixel/profile proof.
use super::*;
use photocraft_doc::{item_identity::FilterId, xcf_filters::FilterNode};
use photocraft_engine::DocumentSnapshot;

struct Target {
    snapshot: DocumentSnapshot,
    revision: u64,
    root: LayerId,
    stack_revision: u64,
    filter: FilterId,
}
fn idle(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|state| {
        state.revision != u64::MAX
            && state.floating.is_none()
            && state.coalesce.is_none()
            && state.doc.xcf_state.as_ref().is_some_and(|xcf| xcf.floating.is_none())
    }) && app.ui.transform.is_none()
        && !app.session.has_jobs()
}
fn node<'a>(snapshot: &'a DocumentSnapshot, root: LayerId) -> Option<(&'a FilterNode, u64)> {
    let owner = snapshot.accounted().filter(|owner| owner.is_readonly_filtered_flat_point_storage())?;
    let state = snapshot.xcf_state.as_ref()?;
    if !(2..=4096).contains(&snapshot.layers.len()) || state.filter_stacks.len() > 4096 {
        return None;
    }
    let mut roots = snapshot.layers.iter().filter(|layer| layer.id == root);
    roots.next()?;
    if roots.next().is_some() {
        return None;
    }
    let mut stacks = state.filter_stacks.iter().filter(|stack| stack.owner == ItemRef::Layer(root));
    let stack = stacks.next()?;
    if stacks.next().is_some() {
        return None;
    }
    let [node] = stack.nodes.as_slice() else { return None };
    photocraft_engine::filtered_flat_filter_edit_candidate(owner, root, node).then_some((node, stack.revision))
}
fn unlocked(snapshot: &DocumentSnapshot, root: LayerId) -> bool {
    let ordinary = snapshot.effective_locks(root);
    !ordinary.all
        && !ordinary.pixels
        && snapshot.xcf_state.as_ref().is_some_and(|state| state.effective_locks(ItemRef::Layer(root)).is_ok_and(|locks| !locks.content))
}
fn capture(app: &PhotocraftApp, root: LayerId) -> Option<Target> {
    let state = app.session.active()?;
    let (node, stack_revision) = node(&state.doc, root)?;
    Some(Target { snapshot: state.doc.clone(), revision: state.revision, root, stack_revision, filter: node.id })
}
fn validate(app: &PhotocraftApp, target: &Target) -> bool {
    idle(app)
        && app.session.active().is_some_and(|state| {
            state.doc.id == target.snapshot.id
                && state.revision == target.revision
                && state.doc.shares_state(&target.snapshot)
                && unlocked(&state.doc, target.root)
                && node(&state.doc, target.root).is_some_and(|(node, revision)| node.id == target.filter && revision == target.stack_revision)
        })
}
fn submit(app: &mut PhotocraftApp, target: &Target, command: &str, params: serde_json::Value) {
    if !validate(app, target) {
        app.ui.status = "xcf.properties.stale-target".into();
        app.ui.status_error = true;
        return;
    }
    // Canonical wire stays filterId/arguments/enabled. The engine captures and
    // rechecks its transaction target; no unsupported revision fields are added.
    run_checked(app, &target.snapshot, target.revision, command, params);
}
fn scalar_widgets<const N: usize>(ui: &mut egui::Ui, values: &mut [f32; N], labels: [&str; N], ranges: [(f32, f32); N]) {
    for ((value, label), (min, max)) in values.iter_mut().zip(labels).zip(ranges) {
        ui.horizontal(|ui| {
            let label = ui.label(label);
            ui.add(egui::DragValue::new(value).range(min..=max).speed(0.1).update_while_editing(false)).labelled_by(label.id);
        });
    }
}
fn controls(ui: &mut egui::Ui, node: &FilterNode, enabled_ui: bool) -> Option<(&'static str, serde_json::Value)> {
    let mut enabled = node.enabled;
    let caption: String = node.source_display_name.as_deref().filter(|name| !name.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();
    match node.operation.name.as_str() {
        "gegl:hue-chroma" => {
            let mut values = hue_chroma::words(node)?.map(f32::from_bits);
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                scalar_widgets(ui, &mut values, [tl!("Hue"), tl!("Chroma"), tl!("Lightness")], [(-180., 180.), (-100., 100.), (-100., 100.)]);
            });
            hue_chroma::request(node, values, enabled)
        }
        "gegl:vibrance" => {
            let mut values = vibrance::words(node)?.map(f32::from_bits);
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                scalar_widgets(ui, &mut values, [tl!("Vibrance"), tl!("Saturation")], [(-100., 100.), (0., 10.)]);
            });
            vibrance::request(node, values, enabled)
        }
        "gimp:semi-flatten" => {
            let [argument] = node.arguments.as_slice() else { return None };
            let (words, _) = semi_flatten::bits(&argument.value)?;
            let mut rgb = [f32::from_bits(*words.first()?), f32::from_bits(*words.get(1)?), f32::from_bits(*words.get(2)?)];
            ui.add_enabled_ui(enabled_ui, |ui| {
                ui.checkbox(&mut enabled, caption);
                // Edit stored literal components; no new display/profile work.
                scalar_widgets(ui, &mut rgb, [tl!("Red"), tl!("Green"), tl!("Blue")], [(0., 1.); 3]);
            });
            let value = semi_flatten::patch(&argument.value, rgb)?;
            if value != argument.value {
                let mut params = json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"color","value":value}]});
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
        _ => None,
    }
}
pub(super) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, root: LayerId) {
    let Some(target) = capture(app, root) else {
        ui.label(tl!("Filter editing is unavailable"));
        return;
    };
    let Some((node, _)) = node(&target.snapshot, root) else { return };
    ui.separator();
    ui.label(tl!("GIMP Filters"));
    if let Some((command, params)) = controls(ui, node, validate(app, &target)) {
        submit(app, &target, command, params);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "filtered_flat_point_mutation_tests.rs"]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "filtered_flat_point_source_activity_tests.rs"]
mod source_activity;
