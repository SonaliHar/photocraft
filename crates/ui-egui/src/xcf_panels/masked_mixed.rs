//! Masked-mixed scalar controls. Full mask/profile/native authority stays in the engine recipe.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber,FilterValue};
pub(super) fn scope(n:&FilterNode)->bool{
 let e=|s:&str|matches!(s,photocraft_compose::EXPOSURE_MASKED_MIXED_EPOCH|photocraft_compose::INVERT_LINEAR_MASKED_MIXED_EPOCH|photocraft_compose::INVERT_GAMMA_MASKED_MIXED_EPOCH|photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_MIXED_EPOCH|photocraft_compose::GEGL_LEVELS_MASKED_MIXED_EPOCH);
 e(&n.operation.schema_epoch)||n.composition.as_ref().is_some_and(|p|e(&p.compatibility_epoch))
}
fn request(n:&FilterNode,names:&[&str],values:&[f32],intent:&[bool],enabled:bool)->Option<(&'static str,serde_json::Value)>{
 if n.arguments.len()!=names.len()||values.len()!=names.len()||intent.len()!=names.len(){return None}
 let mut args=Vec::new();
 for a in &n.arguments {let at=names.iter().position(|name|*name==a.name)?;let FilterValue::Number(ExactNumber::Float32Bits(old))=a.value else{return None};let v=values.get(at)?;if *intent.get(at)?&&v.to_bits()!=old{args.push(json!({"name":a.name,"value":{"Number":{"Float32Bits":v.to_bits()}}}));}}
 if !args.is_empty(){let mut p=json!({"filterId":n.id.0.to_string(),"arguments":args});if enabled!=n.enabled{p.as_object_mut()?.insert("enabled".into(),json!(enabled));}Some(("xcf.filter.set",p))}else if enabled!=n.enabled{Some(("xcf.filter.enable",json!({"filterId":n.id.0.to_string(),"enabled":enabled})))}else{None}
}
pub(super) fn edit(ui:&mut egui::Ui,n:&FilterNode,enabled_ui:bool)->Option<(&'static str,serde_json::Value)>{
 use photocraft_compose::MaskedMixedKind as K;
 let (kind,names,labels):(K,&[&str],&[&str])=match n.operation.name.as_str(){
 "gegl:exposure"=>(K::Exposure,&["exposure","black-level"],&[tl!("Exposure"),tl!("Black level")]),
 "gegl:invert-linear"=>(K::InvertLinear,&[],&[]),"gegl:invert-gamma"=>(K::InvertGamma,&[],&[]),
 "gegl:brightness-contrast"=>(K::BrightnessContrast,&["brightness","contrast"],&[tl!("Brightness"),tl!("Contrast")]),
 "gegl:levels"=>(K::GeglLevels,&["in-low","in-high","out-low","out-high"],&[tl!("Input low"),tl!("Input high"),tl!("Output low"),tl!("Output high")]),_=>return None};
 photocraft_compose::masked_mixed_parameters(kind,&n.arguments).ok()?;
 let mut values=[0f32;4];let mut intent=[false;4];let mut enabled=n.enabled;
 for (at,name) in names.iter().enumerate(){let a=n.arguments.iter().find(|a|a.name==*name)?;let FilterValue::Number(ExactNumber::Float32Bits(b))=a.value else{return None};*values.get_mut(at)?=f32::from_bits(b);}
 let caption:String=n.source_display_name.as_deref().filter(|s|!s.is_empty()).unwrap_or(&n.operation.name).chars().take(128).collect();
 ui.add_enabled_ui(enabled_ui,|ui|{ui.checkbox(&mut enabled,caption);for (at,(name,label)) in names.iter().zip(labels.iter()).enumerate(){ui.horizontal(|ui|{let label=ui.label(*label);if let (Some(value),Some(changed))=(values.get_mut(at),intent.get_mut(at)){let drag=egui::DragValue::new(value).speed(0.01).update_while_editing(false);let drag=match *name{"black-level"=>{let m=f32::from_bits(0x3dcc_cccc);drag.range(-m..=m)},"brightness"=>drag.range(-3.0..=3.0),"contrast"=>drag.range(-5.0..=5.0),_=>drag};*changed=ui.add(drag).labelled_by(label.id).changed();}});}if names.is_empty(){ui.label(tl!("No parameters"));}ui.label(tl!("Filter opacity: 100%"));});
 // Validate the complete proposed tuple without normalizing untouched words.
 let v=values.get(..names.len())?;
 match kind{K::Exposure=>{photocraft_compose::ExposureParameters::from_words(v.first()?.to_bits(),v.get(1)?.to_bits()).ok()?;},K::BrightnessContrast=>{photocraft_compose::BrightnessContrastParameters::from_words(v.first()?.to_bits(),v.get(1)?.to_bits()).ok()?;},K::GeglLevels=>{let [a,b,c,d]=v else{return None};photocraft_compose::GeglLevelsParameters::from_words([a.to_bits(),b.to_bits(),c.to_bits(),d.to_bits()]).ok()?;},_=>{}}
 request(n,names,v,intent.get(..names.len())?,enabled)
}
