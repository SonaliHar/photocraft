//! Targeted mixed-root controls. Registered transactions own all pixel/profile proof.
use super::*;
use photocraft_doc::{item_identity::FilterId, xcf_filters::FilterNode};
use photocraft_engine::DocumentSnapshot;
#[path = "affine_mixed.rs"]
mod affine;
#[path="derived_rgb_mixed.rs"]
mod derived_rgb;
#[path="masked_mixed.rs"]
mod masked;
#[derive(Clone,Copy,PartialEq,Eq)]
enum Route{Original,Masked,Hierarchy}
fn route(snapshot:&DocumentSnapshot)->Option<Route>{let o=snapshot.accounted()?;if o.is_readonly_normal_hierarchy_storage(){Some(Route::Hierarchy)}else if o.is_readonly_masked_filtered_flat_point_storage(){Some(Route::Masked)}else if o.is_readonly_filtered_flat_point_storage(){Some(Route::Original)}else{None}}

struct Target {
    route:Route,
    snapshot: DocumentSnapshot,
    revision: u64,
    root: LayerId,
    stack_revision: u64,
    node_index:usize,
    stack_owner:ItemRef,
    filter: FilterId,
    opacity_bits:u64,
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
fn node_at<'a>(snapshot: &'a DocumentSnapshot, root: LayerId,index:usize) -> Option<(&'a FilterNode, u64)> {
    let owner=snapshot.accounted()?;let actual_route=route(snapshot)?;
    let state = snapshot.xcf_state.as_ref()?;
    if !(1..=4096).contains(&snapshot.layers.len()) || state.filter_stacks.len() > 4096 {
        return None;
    }
    state.filter_stacks.iter().try_fold(0usize,|n,stack|{
        if stack.nodes.len()>1024{return None}
        n.checked_add(stack.nodes.len()).filter(|n|*n<=4096)
    })?;
    // The actual hierarchy owner seals nested layer membership. Do not build a
    // full physical-mask/profile index in a per-frame widget lookup. The engine
    // resolves the exact nested ordinal again under the transaction control.
    if actual_route != Route::Hierarchy {
        let mut roots = snapshot.layers.iter().filter(|layer| layer.id == root);
        roots.next()?;
        if roots.next().is_some() { return None; }
    }
    let mut stacks = state.filter_stacks.iter().filter(|stack| stack.owner == ItemRef::Layer(root));
    let stack = stacks.next()?;
    if stacks.next().is_some() {
        return None;
    }
    let node=stack.nodes.get(index)?;
    (match actual_route{Route::Original=>photocraft_engine::filtered_flat_filter_edit_candidate(owner,root,node),Route::Masked=>photocraft_engine::masked_filtered_flat_filter_edit_candidate(owner,root,node),Route::Hierarchy=>photocraft_engine::normal_hierarchy_filter_edit_candidate(owner,root,node)}).then_some((node,stack.revision))
}
fn unlocked(snapshot: &DocumentSnapshot, root: LayerId) -> bool {
    let ordinary = snapshot.effective_locks(root);
    !ordinary.all
        && !ordinary.pixels
        && snapshot.xcf_state.as_ref().is_some_and(|state| state.effective_locks(ItemRef::Layer(root)).is_ok_and(|locks| !locks.content))
}
fn capture_snapshot(snapshot:&DocumentSnapshot,revision:u64,root:LayerId,index:usize)->Option<Target>{
    let (node,stack_revision)=node_at(snapshot,root,index)?;
    Some(Target{route:route(snapshot)?,snapshot:snapshot.clone(),revision,root,stack_revision,node_index:index,stack_owner:ItemRef::Layer(root),filter:node.id,opacity_bits:node.opacity_f64_bits})
}
#[cfg(test)]
fn capture_at(app:&PhotocraftApp,root:LayerId,index:usize)->Option<Target>{
    let state=app.session.active()?;capture_snapshot(&state.doc,state.revision,root,index)
}

fn validate(app: &PhotocraftApp, target: &Target) -> bool {
    idle(app)
        && app.session.active().is_some_and(|state| {
            route(&state.doc)==Some(target.route)
            && state.doc.id == target.snapshot.id
                && state.revision == target.revision
                && state.doc.shares_state(&target.snapshot)
                && unlocked(&state.doc, target.root)
                && target.stack_owner==ItemRef::Layer(target.root)
                && node_at(&state.doc, target.root,target.node_index).is_some_and(|(node, revision)| node.id == target.filter && revision == target.stack_revision && node.opacity_f64_bits==target.opacity_bits)
        })
}
fn submit(app: &mut PhotocraftApp, target: &Target, command: &str, params: serde_json::Value) {
    if !validate(app, target) {
        app.ui.status = "xcf.properties.stale-target".into();
        app.ui.status_error = true;
        return;
    }
    // Canonical edit/move wires retain their original keys. The engine captures and
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
fn family_controls(ui: &mut egui::Ui, node: &FilterNode, enabled_ui: bool) -> Option<(&'static str, serde_json::Value)> {
    let mut enabled = node.enabled;
    let caption: String = node.source_display_name.as_deref().filter(|name| !name.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();
    match node.operation.name.as_str() {
        "gegl:brightness-contrast" | "gegl:levels" => affine::edit(ui, node, enabled_ui, &caption),
        "gegl:color-temperature" | "gegl:sepia" | "gegl:alien-map" => derived_rgb::edit(ui,node,enabled_ui,&caption),
        "gegl:invert-linear" | "gegl:invert-gamma" => {
            if !node.arguments.is_empty(){return None}
            ui.add_enabled_ui(enabled_ui,|ui|{
                ui.checkbox(&mut enabled,caption);
                ui.label(tl!("No parameters"));
            });
            (enabled!=node.enabled).then(||("xcf.filter.enable",json!({"filterId":node.id.0.to_string(),"enabled":enabled})))
        }
        "gegl:exposure" => {
            if node.operation.schema_epoch==photocraft_compose::EXPOSURE_LOADED_MIXED_EPOCH {
                return exposure_loaded::edit(ui,node,enabled_ui,true);
            }
            let word=|name:&str|match &node.arguments.iter().find(|a|a.name==name)?.value {
                photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(bits))=>Some(*bits),
                _=>None,
            };
            let old=[word("exposure")?,word("black-level")?];
            let mut values=old.map(f32::from_bits);
            let black_max=f32::from_bits(0x3dcc_cccc);
            ui.add_enabled_ui(enabled_ui,|ui|{
                ui.checkbox(&mut enabled,caption);
                ui.horizontal(|ui|{let label=ui.label(tl!("Exposure"));ui.add(egui::DragValue::new(&mut values[0]).speed(0.1).update_while_editing(false)).labelled_by(label.id);});
                ui.horizontal(|ui|{let label=ui.label(tl!("Black level"));ui.add(egui::DragValue::new(&mut values[1]).range(-black_max..=black_max).speed(0.001).update_while_editing(false)).labelled_by(label.id);});
            });
            if !values.iter().all(|v|v.is_finite()) || !(-0.1f64..=0.1f64).contains(&f64::from(values[1])) {return None}
            let mut arguments=Vec::new();
            // Emit changed words in retained source argument order; preserve
            // unedited signed zero and do not normalize stored endpoint words.
            for argument in &node.arguments {
                let index=match argument.name.as_str(){"exposure"=>0,"black-level"=>1,_=>return None};
                let bits=values[index].to_bits();
                if bits!=old[index]{arguments.push(json!({"name":argument.name,"value":{"Number":{"Float32Bits":bits}}}));}
            }
            if !arguments.is_empty(){let mut params=json!({"filterId":node.id.0.to_string(),"arguments":arguments});if enabled!=node.enabled{params.as_object_mut()?.insert("enabled".into(),json!(enabled));}Some(("xcf.filter.set",params))}
            else if enabled!=node.enabled{Some(("xcf.filter.enable",json!({"filterId":node.id.0.to_string(),"enabled":enabled})))}else{None}
        }
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
// Keep the family builder separate: its exact loadedV2 early return must not
// bypass opacity, and its intentional-only black patch rules remain untouched.
fn opacity_request(node:&FilterNode,request:Option<(&'static str,serde_json::Value)>,opacity:f64,intent:bool)->Option<(&'static str,serde_json::Value)>{
    if !intent || opacity.to_bits()==node.opacity_f64_bits{return request}
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity){return None}
    let mut params=match request {Some((_,params))=>params,None=>json!({"filterId":node.id.0.to_string()})};
    params.as_object_mut()?.insert("opacityF64Bits".into(),json!(format!("0x{:016x}",opacity.to_bits())));
    Some(("xcf.filter.set",params))
}
fn controls(ui:&mut egui::Ui,node:&FilterNode,enabled_ui:bool,route:Route)->Option<(&'static str,serde_json::Value)>{
    if matches!(route,Route::Masked|Route::Hierarchy) && masked::scope(node){return masked::edit(ui,node,enabled_ui)}
    let request=family_controls(ui,node,enabled_ui);
    let mut percent=f64::from_bits(node.opacity_f64_bits)*100.0;
    let intent=ui.add_enabled_ui(enabled_ui,|ui|ui.horizontal(|ui|{
        let label=ui.label(tl!("Filter opacity"));
        ui.add(egui::DragValue::new(&mut percent).range(0.0..=100.0).speed(1.0).suffix("%").update_while_editing(false)).labelled_by(label.id).changed()
    }).inner).inner;
    // Percentage formatting is view-only. Never roundtrip untouched F64 words.
    if !intent{return request}
    opacity_request(node,request,percent/100.0,true)
}
pub(super) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, root: LayerId) {
    let Some((snapshot,revision))=app.session.active().map(|state|(state.doc.clone(),state.revision)) else{return};
    let count=snapshot.xcf_state.as_ref()
        .and_then(|state|state.filter_stacks.iter().find(|stack|stack.owner==ItemRef::Layer(root)))
        .map_or(0,|stack|stack.nodes.len());
    if count==0 {ui.label(tl!("No GIMP filters"));return;}
    if count>1024 {
        ui.label(tl!("Filter editing is unavailable"));return;
    }
    ui.separator();ui.label(tl!("GIMP Filters"));
    // Canonical/public XCF node order is UI order. Operative processing order
    // remains the independently qualified L3 pipeline's responsibility.
    for index in 0..count {
        let Some(target)=capture_snapshot(&snapshot,revision,root,index) else {ui.label(tl!("Filter editing is unavailable"));continue};
        let Some((node,_))=node_at(&target.snapshot,root,index) else {continue};
        ui.push_id((index,target.filter.0),|ui|{
            if let Some((command,params))=controls(ui,node,validate(app,&target),target.route) {
                submit(app,&target,command,params);
            }
            let enabled=validate(app,&target);
            let destination=ui.horizontal(|ui| {
                let mut to=None;
                if ui.add_enabled(enabled && index>0 && index-1<1024,egui::Button::new(tl!("Up"))).clicked(){to=index.checked_sub(1)}
                if ui.add_enabled(enabled && index.checked_add(1).is_some_and(|next|next<count&&next<1024),egui::Button::new(tl!("Down"))).clicked(){to=index.checked_add(1)}
                if ui.add_enabled(enabled && index>0,egui::Button::new(tl!("Move to Top"))).clicked(){to=Some(0)}
                to
            }).inner;
            if let Some(destination)=destination {
                submit(app,&target,"xcf.filter.move",serde_json::json!({"filterId":target.filter.0.to_string(),"index":destination}));
            }
            if ui.add_enabled(validate(app,&target),egui::Button::new(tl!("Remove"))).clicked() {
                submit(app,&target,"xcf.filter.remove",serde_json::json!({"filterId":target.filter.0.to_string()}));
            }
            // Each admitted stack and existing wire destinations are bounded
            // to 1024; the document-wide node census stays at 4096.
        });
    }
}
// Historical singleton-per-root test fixtures explicitly target their index0;
// production capture/display always supplies the actual loop index above.
#[cfg(test)]
fn node(snapshot:&DocumentSnapshot,root:LayerId)->Option<(&FilterNode,u64)>{node_at(snapshot,root,0)}
#[cfg(test)]
fn capture(app:&PhotocraftApp,root:LayerId)->Option<Target>{capture_at(app,root,0)}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "filtered_flat_point_mutation_tests.rs"]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "filtered_flat_point_source_activity_tests.rs"]
mod source_activity;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_ordered_tests.rs"]
mod ordered_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_move_tests.rs"]
mod move_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_remove_tests.rs"]
mod remove_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_current_tests.rs"]
mod current_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_exposure_tests.rs"]
mod exposure_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_invert_tests.rs"]
mod invert_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_opacity_tests.rs"]
mod opacity_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="filtered_flat_point_source_opacity_tests.rs"]
mod source_opacity_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "filtered_flat_point_affine_tests.rs"]
mod affine_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="masked_filtered_flat_tests.rs"]
mod masked_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="normal_hierarchy_tests.rs"]
mod normal_hierarchy_tests;
