//! Exact masked Exposure controls; mask bytes and source provenance are never edited here.
use super::*;
use photocraft_doc::xcf_filters::{FilterNode,FilterValue,ExactNumber};
fn words(node:&FilterNode)->Option<[u32;2]>{
    if node.operation.name!="gegl:exposure" || node.operation.stored_version!="1:0" || node.arguments.len()!=2{return None}
    let mut words=[None;2];
    for a in &node.arguments {
        let index=match a.name.as_str(){"exposure"=>0,"black-level"=>1,_=>return None};
        let FilterValue::Number(ExactNumber::Float32Bits(bits))=a.value else{return None};
        if !f32::from_bits(bits).is_finite() || words.get_mut(index)?.replace(bits).is_some(){return None}
    }
    Some([words[0]?,words[1]?])
}
fn request(node:&FilterNode,values:[f32;2],intent:[bool;2],enabled:bool)->Option<(&'static str,serde_json::Value)>{
    let raw=words(node)?;
    if !values.iter().all(|v|v.is_finite()) || !(-0.1f64..=0.1f64).contains(&f64::from(values[1])){return None}
    let mut arguments=Vec::new();
    for a in &node.arguments {
        let index=match a.name.as_str(){"exposure"=>0,"black-level"=>1,_=>return None};
        // Only deliberate widget intent changes an exact stored word.
        if *intent.get(index)? && values.get(index)?.to_bits()!=*raw.get(index)? {
            arguments.push(json!({"name":a.name,"value":{"Number":{"Float32Bits":values.get(index)?.to_bits()}}}));
        }
    }
    if !arguments.is_empty(){let mut params=json!({"filterId":node.id.0.to_string(),"arguments":arguments});if enabled!=node.enabled{params.as_object_mut()?.insert("enabled".into(),json!(enabled));}Some(("xcf.filter.set",params))}
    else if enabled!=node.enabled{Some(("xcf.filter.enable",json!({"filterId":node.id.0.to_string(),"enabled":enabled})))}else{None}
}
pub(super) fn edit(ui:&mut egui::Ui,node:&FilterNode,editable:bool,checkbox:bool)->Option<(&'static str,serde_json::Value)>{
    let raw=words(node)?;
    // Strict stored words stay literal; there is no loaded-endpoint interpretation.
    let initial=[f32::from_bits(raw[0]),f32::from_bits(raw[1])];
    let mut values=initial;let mut changed=[false;2];let mut enabled=node.enabled;
    ui.add_enabled_ui(editable,|ui|{
        if checkbox{let caption:String=node.source_display_name.as_deref().filter(|s|!s.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();ui.checkbox(&mut enabled,caption);}
        ui.horizontal(|ui|{let label=ui.label(tl!("Exposure"));changed[0]=ui.add(egui::DragValue::new(&mut values[0]).speed(0.1).update_while_editing(false)).labelled_by(label.id).changed();});
        ui.horizontal(|ui|{let label=ui.label(tl!("Black level"));let bound=f32::from_bits(0x3dcc_cccc);changed[1]=ui.add(egui::DragValue::new(&mut values[1]).range(-bound..=bound).speed(0.001).update_while_editing(false)).labelled_by(label.id).changed();});
    });
    // Passive frames never rewrite untouched stored words.
    let intent=changed;
    request(node,values,intent,enabled)
}
pub(super) fn scope(snapshot:&photocraft_engine::DocumentSnapshot,node:&FilterNode)->bool {
    snapshot.accounted().is_some_and(|owner|owner.is_readonly_point_storage())
        && (node.operation.schema_epoch==photocraft_compose::EXPOSURE_MASKED_EPOCH
            || node.composition.as_ref().is_some_and(|p|p.compatibility_epoch==photocraft_compose::EXPOSURE_MASKED_EPOCH))
}
pub(super) fn controls(app:&mut PhotocraftApp,ui:&mut egui::Ui,snapshot:&photocraft_engine::DocumentSnapshot,revision:u64,node:&FilterNode,unlocked:bool){
    let editable=unlocked && snapshot.accounted().is_some_and(|owner|photocraft_engine::xcf_filter_cmds::masked_exposure_point_edit_candidate(owner,node));
    if let Some((id,params))=edit(ui,node,editable,true){run_checked(app,snapshot,revision,id,params)}
    if !editable {ui.label(tl!("Filter editing is unavailable"));}
}

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="exposure_masked_tests.rs"]
mod tests;
