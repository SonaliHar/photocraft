//! Deferred actual source/carrier and CURRENT integer hierarchy host continuity.
use super::*;
#[path="accounted_plain_integer_hierarchy_test_support.rs"]mod f;
use serde_json::json;
#[test]
#[ignore="UNRUN/uncompiled generated U8/150 U16/250 RGBNL hierarchy Source import, native13 Current anchors, nested select/compoundmetadata/disclosure/history and closed filter/pixel routes. No GIMP/numerical acceptance."]
fn plain_integer_hierarchy_preserves_actual_class_and_empty_anchors(){
 for depth in 0..2{
  let input=f::import(depth);let photocraft_io::OwnedXcfImport::NormalHierarchy(source)=&input else{panic!("hierarchy source")};f::observed(source.document().document(),depth);let mut s=Session::new();s.add_owned_normal_hierarchy_xcf_import(source,"integer-hierarchy.xcf",Some("authorized/integer-hierarchy.xcf"),&f::ctl()).unwrap();let original=s.active().unwrap().doc.clone();let owner=f::anchored(original.accounted().unwrap());let mut current=Session::new();current.add_owned_native_normal_hierarchy_archive(f::archive(&owner),"integer.pcraft",None,&f::ctl()).unwrap();let(group,leaf,empty)=f::ids(&current.active().unwrap().doc);let source_id=current.active().unwrap().source.clone();
  let query=crate::accounted_xcf_inspect::prepare(&current,&json!({}),&f::ctl()).unwrap();assert!(query.value().is_object());drop(query);
  for id in [group,leaf,empty]{current.execute("layer.select",json!({"layer":id.0})).unwrap();assert_eq!(current.active().unwrap().active_layer,Some(id));}
  let revision=current.active().unwrap().revision;let past=current.active().unwrap().history.past_len();let journal=current.journal.len();current.execute("layer.setExpanded",json!({"layer":group.0,"expanded":false})).unwrap();assert_eq!(current.active().unwrap().revision,revision);assert_eq!(current.active().unwrap().history.past_len(),past);assert_eq!(current.journal.len(),journal);
  current.execute("layer.setProps",json!({"layer":group.0,"name":"Integer group","opacity":0.375})).unwrap();let changed=current.active().unwrap().doc.clone();assert_eq!(changed.layer(group).unwrap().opacity.to_bits(),0.375f32.to_bits());f::observed(&changed,depth);f::payload(&original,&changed);assert_eq!(current.active().unwrap().source,source_id);assert!(current.undo_with_control(&f::ctl()).unwrap());assert!(current.redo_with_control(&f::ctl()).unwrap());
  let snapshot=current.active().unwrap().doc.clone();for(id,p)in [("xcf.filter.set",json!({"filterId":"1","exposure":1.0})),("xcf.filter.enable",json!({"filterId":"1","enabled":false})),("xcf.filter.move",json!({"filterId":"1","index":0})),("xcf.filter.remove",json!({"filterId":"1"})),("xcf.filter.insert",json!({})),("paint.stroke",json!({"points":[[1,1,1]]})),("select.all",json!({}))]{assert!(current.execute(id,p).is_err());assert!(snapshot.shares_state(&current.active().unwrap().doc));}
  let stack=snapshot.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st|st.owner==photocraft_doc::item_identity::ItemRef::Layer(leaf)).unwrap();assert_eq!(stack.revision,7);assert!(stack.nodes.is_empty());let mut reopen=Session::new();reopen.add_owned_native_normal_hierarchy_archive(f::archive(snapshot.accounted().unwrap()),"continued.pcraft",None,&f::ctl()).unwrap();f::same(&snapshot,&reopen.active().unwrap().doc);assert!(reopen.active().unwrap().doc.accounted().unwrap().is_readonly_normal_hierarchy_storage());
 }
}
