//! Deferred registered native stroke + trusted native Save As continuation.
use super::*;
#[path="../../../engine/src/accounted_native_stroke_test_support.rs"] mod fixture;
use std::sync::atomic::{AtomicU64,Ordering};
struct Directory(std::path::PathBuf);
impl Directory{fn new()->Self{static NEXT:AtomicU64=AtomicU64::new(0);let p=std::env::temp_dir().join(format!("photocraft-native-stroke-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));std::fs::create_dir(&p).unwrap();Self(p)}}
impl Drop for Directory{fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
#[test]
#[ignore="UNRUN/uncompiled real generated XCF plain adoption, registered native RGB/Gray U8/U16/F32 stroke, trusted native SaveAs title/reopen, original source custody and undo. No producer/GUI/performance claim."]
fn imported_plain_native_stroke_survives_trusted_save_and_undo(){
 let dir=Directory::new();for gray in [false,true]{for(sample,index)in [(photocraft_color::SampleType::U8,0),(photocraft_color::SampleType::U16,1),(photocraft_color::SampleType::F32,2)]{
  let input=fixture::import(gray,sample,false);let mut h=Headless::trusted_local();h.session.add_owned_plain_xcf_import(&input,"native-stroke.xcf",Some("authorized/native-stroke.xcf"),&fixture::ctl()).unwrap();h.session.tools.brush.pressure_size=false;let before=h.session.active().unwrap().doc.clone();let raw=fixture::payload(&before);let source=h.session.active().unwrap().source.clone();h.command_run("paint.stroke",fixture::patch()).unwrap();let changed=h.session.active().unwrap().doc.clone();assert_ne!(fixture::payload(&changed),raw);assert_eq!(h.session.active().unwrap().source,source);
  let dest=dir.0.join(format!("native-{gray}-{index}.pcraft"));h.save_owned_native_active(Some(&dest),Some("pcraft"),&fixture::ctl()).unwrap();let saved=h.session.active().unwrap().doc.clone();assert_eq!(saved.name,dest.file_name().unwrap().to_str().unwrap());let mut reopen=Headless::trusted_local();drop(reopen.open_owned_native_keep_current_bounded(&dest).unwrap());let after=reopen.session.active().unwrap().doc.clone();assert!(after.accounted().unwrap().is_legacy_storage());assert_eq!(after.name,saved.name);assert_eq!(fixture::payload(&after),fixture::payload(&saved));assert_eq!(after.xcf_state,saved.xcf_state);assert_eq!(after.source_color_context,saved.source_color_context);
  h.command_run("edit.undo",serde_json::json!({})).unwrap();assert_eq!(fixture::payload(&h.session.active().unwrap().doc),raw);assert_eq!(h.session.active().unwrap().source,source);
 }}
}
