//! Unrun registered native Luminosity replacement regressions.
use photocraft_color::{ColorMode,SampleType,try_write_sample_f64};
use photocraft_doc::{Document,Layer,LayerContent,SampleEncoding,Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt,Surface};
use serde_json::{Value,json};
fn session(sample:SampleType)->Session {
    let mut doc=Document::new("Native Luminosity",Size::new(3,1),ColorMode::Rgb,sample);doc.sample_encoding=Some(SampleEncoding::Linear);doc.icc_profile=Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let format=doc.pixel_format();let mut raw=Vec::new();for gray in [0.5,0.25,0.5]{let mut pixel=vec![0;format.bytes_per_pixel()];for channel in 0..3{try_write_sample_f64(&mut pixel,sample,channel,gray).unwrap();}try_write_sample_f64(&mut pixel,sample,3,0.75).unwrap();raw.extend_from_slice(&pixel);}
    doc.layers.push(Layer::new("Source pixels",LayerContent::Raster(Surface::try_from_interleaved_with(format,doc.bounds(),&raw,&Interrupt::NONE).unwrap())));let mut s=Session::new();s.add_document(doc,None).unwrap();s.tools.brush.pressure_size=false;s.tools.brush.smoothing.enabled=false;s
}
fn params()->Value{json!({"mode":"luminosity","sampling":"once","limits":"contiguous","tolerance":0,"antiAlias":false,"color":[0.8,0.8,0.8,1],"size":8,"hardness":1,"flow":1,"opacity":1,"smoothing":0,"seed":37,"points":[[0.5,0.5,1]]})}
#[test]
fn registered_native_replacement_changes_only_connected_match_and_one_history_step() {
    for sample in [SampleType::U32,SampleType::F64]{let mut s=session(sample);let before=s.active().unwrap().doc.clone();let old=before.layers[0].surface().unwrap().try_read_interleaved_with(Rect::new(0,0,3,1),&Interrupt::NONE).unwrap();let revision=s.active().unwrap().revision;let steps=s.active().unwrap().history.past_len();
        let response=s.execute("paint.colorReplacement",params()).unwrap();assert_eq!(response["nativeLuminosity"],true);let st=s.active().unwrap();let changed=st.doc.clone();let after=changed.layers[0].surface().unwrap().try_read_interleaved_with(Rect::new(0,0,3,1),&Interrupt::NONE).unwrap();let bpp=changed.pixel_format().bytes_per_pixel();
        assert_ne!(after.get(..bpp),old.get(..bpp));assert_eq!(after.get(bpp..),old.get(bpp..),"nonmatching barrier and disconnected equal color remain exact");assert_eq!(after.get(bpp-sample.bytes()..bpp),old.get(bpp-sample.bytes()..bpp));assert_eq!(st.history.past_len(),steps+1);assert_eq!(st.revision,revision+1);assert_eq!(changed.icc_profile,before.icc_profile);assert_eq!(changed.sample_encoding,before.sample_encoding);assert_eq!(changed.layers[0].id,before.layers[0].id);
        s.execute("edit.undo",Value::Null).unwrap();assert_eq!(*s.active().unwrap().doc,*before);s.execute("edit.redo",Value::Null).unwrap();assert_eq!(*s.active().unwrap().doc,*changed);
    }
}
#[test]
fn unsupported_native_modes_contexts_and_live_preview_keep_real_redo() {
    for sample in [SampleType::U32,SampleType::F64]{for case in ["color","nonlinear","floating","bad-selection"]{
        let mut s=session(sample);let mut p=params();
        match case{"color"=>p["mode"]=json!("color"),"nonlinear"=>{s.edit("Nonlinear",|doc,_|{doc.sample_encoding=Some(SampleEncoding::NonlinearProfile);doc.icc_profile=Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());Ok(())}).unwrap();},"bad-selection"=>{},_=>{}}
        if case=="floating"{s.execute("select.all",json!({})).unwrap();}
        s.edit("Real redo",|doc,_|{doc.name="Redo".into();Ok(())}).unwrap();s.execute("edit.undo",Value::Null).unwrap();
        if case=="floating"{s.execute("select.float",json!({"copy":true,"dx":1})).unwrap();}
        if case=="bad-selection"{s.active_mut().unwrap().doc.try_resident_mut().unwrap().selection=Some(Surface::new(photocraft_color::PixelFormat::RGBA32F));}
        let st=s.active().unwrap();let before=st.doc.clone();let revision=st.revision;let steps=st.history.past_len();let floating=st.floating.clone();
        assert!(s.execute("paint.colorReplacement",p.clone()).is_err());assert!(photocraft_engine::brush_cmds::live_color_replacement(&s,&p).is_err());let st=s.active().unwrap();assert!(st.doc.shares_state(&before));assert_eq!(st.revision,revision);assert_eq!(st.history.past_len(),steps);assert!(st.history.can_redo());if let Some(old)=floating{let retained=st.floating.as_ref().unwrap();assert!(std::sync::Arc::ptr_eq(&old.parts,&retained.parts));assert_eq!(old.offset,retained.offset);}
    }}
}
