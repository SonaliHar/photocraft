//! Test-only generated plain XCF transport fixture. No external producer oracle.
//! Raw input, expected byte captures and encoded transport are external harness
//! allocations; actual decoded native planes/model/source retain real paid quota.
use photocraft_raster::{Interrupt,backing::{ReadControl,quota::{BackingQuota,QuotaLimits,QuotaUsage}}};
#[path="../../xcf/tests/common/mod.rs"] mod xcf;
pub fn ctl()->ReadControl<'static>{ReadControl{interrupt:&Interrupt::NONE,work:&|_|Ok(())}}
pub fn import(gray:bool,sample:photocraft_color::SampleType,alpha_lock:bool)->photocraft_io::OwnedPlainXcfImport{
 let (precision,color,alpha)=match sample{
  photocraft_color::SampleType::U8=>(150,vec![63],vec![191]),
  photocraft_color::SampleType::U16=>(250,0x4001u16.to_be_bytes().to_vec(),0xc001u16.to_be_bytes().to_vec()),
  photocraft_color::SampleType::F32=>(650,0.25000003f32.to_be_bytes().to_vec(),0.75000006f32.to_be_bytes().to_vec()),_=>panic!("fixture format")};
 let mut pixel=Vec::new();for _ in 0..if gray{1}else{3}{pixel.extend_from_slice(&color)}pixel.extend_from_slice(&alpha);
 let raw=pixel.repeat(257*259);let mut options=xcf::FixtureOptions{model:if gray{1}else{0},..Default::default()};if alpha_lock{options.layer_properties.push((10,1u32.to_be_bytes().to_vec()));}
 let q=BackingQuota::new_with_aggregate_initial(QuotaLimits{resident:512<<20,index:64<<20,scratch:0},512<<20,QuotaUsage::default()).unwrap().0;
 let limits=photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
 photocraft_io::import_owned_staged_plain_bytes_in_quota("native-stroke.xcf",std::sync::Arc::new(xcf::image_custom(13,precision,0,257,259,&raw,&options)),&Default::default(),&limits,q,&ctl()).unwrap()
}
pub fn payload(doc:&photocraft_doc::Document)->Vec<u8>{
 let photocraft_doc::LayerContent::BackedRaster(r)=&doc.layers[0].content else{panic!("native layer")};let bpp=r.format().bytes_per_pixel();let rect=r.operative_bounds();let mut result=vec![0;rect.width()as usize*rect.height()as usize*bpp];
 for y in rect.y0..rect.y1{for x in rect.x0..rect.x1{let at=((y-rect.y0)as usize*rect.width()as usize+(x-rect.x0)as usize)*bpp;r.checked_view().read_logical_pixel_native_into(x,y,&mut result[at..at+bpp],&ctl()).unwrap();}}result
}
pub fn patch()->serde_json::Value{serde_json::json!({"points":[[254.0,254.0,0.7],[257.0,258.0,1.0]],"size":7.0,"hardness":0.6,"spacing":0.3,"color":[0.8,0.2,0.1,0.75],"seed":53})}
