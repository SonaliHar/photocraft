//! Deferred generated-XCF integer hierarchy host fixture. This is not a GIMP
//! packet or producer numerical oracle. Raw/encoded/comparison buffers external;
//! actual source/native/model owners carry the real decoder quota and retention.
#![allow(dead_code)]
#[path="../../io/src/xcf_import/plain_quantized_hierarchy_fixture.rs"] mod data;
#[path="accounted_normal_hierarchy_test_support.rs"] mod old;
pub use old::{ctl,archive,encoded,payload,storage,same};
use photocraft_doc::{Document,Layer,LayerContent,LayerId,SampleType,accounted_document::AccountedDocument,item_identity::ItemRef,xcf_filters::FilterStack};
use photocraft_raster::backing::quota::{BackingQuota,QuotaLimits,QuotaUsage,QuotaKind};
use std::sync::Arc;
pub fn bytes(depth:usize)->Vec<u8>{data::image(depth)}
pub fn import(depth:usize)->photocraft_io::OwnedXcfImport{
 let q=BackingQuota::new_with_aggregate_initial(QuotaLimits{resident:512<<20,index:64<<20,scratch:0},512<<20,QuotaUsage::default()).unwrap().0;
 photocraft_io::import_owned_xcf_bytes_in_quota("integer-hierarchy.xcf",Arc::new(bytes(depth)),&Default::default(),&photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),q,&ctl()).unwrap()
}
pub fn ids(doc:&Document)->(LayerId,LayerId,LayerId){
 let mut work:Vec<&Layer>=doc.layers.iter().collect();let(mut group,mut leaf,mut empty)=(None,None,None);
 while let Some(layer)=work.pop(){match &layer.content{LayerContent::Group(g)=>{if g.children.is_empty(){empty.get_or_insert(layer.id);}else{group.get_or_insert(layer.id);}work.extend(g.children.iter());},LayerContent::BackedRaster(_)=>{leaf.get_or_insert(layer.id);},_=>panic!("fixture content")}}
 (group.unwrap(),leaf.unwrap(),empty.unwrap())
}
pub fn observed(doc:&Document,depth:usize){
 assert_eq!(doc.mode,photocraft_doc::ColorMode::Rgb);assert_eq!(doc.depth,match depth{0=>SampleType::U8,1=>SampleType::U16,2=>SampleType::U32,_=>panic!("fixture depth")});assert_eq!(doc.sample_encoding,Some(photocraft_doc::SampleEncoding::NonlinearProfile));assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision,match depth{0=>150,1=>250,2=>350,_=>panic!("fixture depth")});let s=doc.xcf_state.as_ref().unwrap();assert_eq!(s.provenance.format_version,12);assert!(s.filter_masks.is_empty());assert!(s.filter_stacks.iter().all(|s|s.nodes.is_empty()));let _=ids(doc);
 let mut todo:Vec<&Layer>=doc.layers.iter().collect();while let Some(l)=todo.pop(){if let LayerContent::Group(g)=&l.content{assert!(s.items.iter().find(|r|r.item==ItemRef::Layer(l.id)).unwrap().raster_bounds.is_none());todo.extend(g.children.iter());}else{let LayerContent::BackedRaster(b)=&l.content else{panic!("native")};assert_eq!(b.format().sample,doc.depth);assert_eq!(b.operative_bounds(),doc.bounds());let index=match l.name.as_str(){"LeafA"=>0,"LeafB"=>1,"Background"=>2,_=>panic!("fixture leaf name")};assert_eq!(old::raw(l),data::native(depth,1,index));}}
}
pub fn anchored(owner:&AccountedDocument)->AccountedDocument{
 let q=owner.quota().clone();let index=q.reserve(QuotaKind::Index,8<<20).unwrap();let resident=q.reserve(QuotaKind::Resident,16<<20).unwrap();let mut d=owner.document().clone();let leaf=ids(&d).1;let s=d.xcf_state.as_mut().unwrap();assert!(s.filter_stacks.iter().all(|s|s.nodes.is_empty()));if let Some(stack)=s.filter_stacks.iter_mut().find(|s|s.owner==ItemRef::Layer(leaf)){stack.revision=7}else{s.filter_stacks.push(FilterStack{owner:ItemRef::Layer(leaf),nodes:Vec::new(),revision:7});}
 AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d,Arc::new((owner.clone(),index,resident)),q,&ctl()).unwrap()
}
