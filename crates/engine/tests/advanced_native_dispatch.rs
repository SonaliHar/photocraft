//! Unrun registered regression drafts for legacy advanced brush admission.
use photocraft_color::{ColorMode,PixelFormat,SampleType};
use photocraft_doc::{AlphaChannel,Document,Layer,LayerContent,LayerMask,Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt,Surface};
use serde_json::{Value,json};
fn fixture(sample:SampleType)->Session {
    let mut doc=Document::new("Advanced brush native retention",Size::new(8,8),ColorMode::Rgb,sample);
    let format=doc.pixel_format();let mut raw=vec![0;format.bytes_per_pixel()];
    for (i,value) in [f64::from_bits(0x3fc9_9999_9999_999b),0.4,0.6,1.0].into_iter().enumerate() {photocraft_color::try_write_sample_f64(&mut raw,sample,i,value).unwrap();}
    let surface=Surface::try_with_default_bytes(format,&raw).unwrap();
    doc.layers.push(Layer::new("Native source",LayerContent::Raster(surface)));
    let mut s=Session::new();s.add_document(doc,None).unwrap();s.tools.brush.smoothing.enabled=false;s.tools.brush.pressure_size=false;s
}
fn params(command:&str)->Value {
    let mut p=json!({"points":[[3.0,3.0,1.0],[5.0,3.0,1.0]],"size":3,"hardness":1,"flow":1,"opacity":1,"smoothing":0,"color":[1,0,0,1],"seed":11});
    if command=="paint.mixerBrush" {p["wet"]=json!(0);p["load"]=json!(100);p["mix"]=json!(0);p["cleanAfterStroke"]=json!(false);}
    else {p["tolerance"]=json!(100);}
    p
}
#[test]
fn registered_advanced_wide_refusal_retains_native_words_reservoir_and_real_redo() {
    for sample in [SampleType::U32,SampleType::F64] {for command in ["paint.mixerBrush","paint.colorReplacement"] {
        let mut s=fixture(sample);s.tools.mixer.load([0.1,0.2,0.3,0.4]);s.tools.mixer.pickup=Some([0.7,0.6,0.5,0.4]);
        s.edit("Seed",|doc,_|{doc.name="Real redo".into();Ok(())}).unwrap();s.execute("edit.undo",Value::Null).unwrap();
        let st=s.active().unwrap();let before=st.doc.clone();let revision=st.revision;let history=st.history.past_len();let active=st.active_layer;let mixer=s.tools.mixer.clone();
        let bytes=st.doc.layers[0].surface().unwrap().try_read_interleaved_with(Rect::new(0,0,8,8),&Interrupt::NONE).unwrap();
        assert!(s.execute(command,params(command)).is_err(),"{sample:?}/{command}");
        let st=s.active().unwrap();assert!(st.doc.shares_state(&before));assert_eq!(st.revision,revision);assert_eq!(st.history.past_len(),history);assert_eq!(st.active_layer,active);assert!(st.history.can_redo());assert_eq!(s.tools.mixer,mixer);
        assert_eq!(st.doc.layers[0].surface().unwrap().try_read_interleaved_with(Rect::new(0,0,8,8),&Interrupt::NONE).unwrap(),bytes);
        s.execute("edit.redo",Value::Null).unwrap();assert_eq!(s.active().unwrap().doc.name,"Real redo");
    }}
}
#[test]
fn ordinary_advanced_brush_routes_still_edit_and_undo() {
    for sample in [SampleType::U8,SampleType::U16,SampleType::F32] {for command in ["paint.mixerBrush","paint.colorReplacement"] {
        let mut s=fixture(sample);let before=s.active().unwrap().doc.clone();let steps=s.active().unwrap().history.past_len();
        s.execute(command,params(command)).unwrap();let changed=s.active().unwrap().doc.clone();
        assert_ne!(*changed,*before,"{sample:?}/{command}");assert_eq!(s.active().unwrap().history.past_len(),steps+1);
        s.execute("edit.undo",Value::Null).unwrap();assert_eq!(*s.active().unwrap().doc,*before);
        s.execute("edit.redo",Value::Null).unwrap();assert_eq!(*s.active().unwrap().doc,*changed);
    }}
}
#[test]
fn mixer_sample_all_refuses_visible_wide_source_even_on_ordinary_target() {
    for sample in [SampleType::U32,SampleType::F64] {
        let mut s=fixture(SampleType::F32);
        s.edit("Add wide source",|doc,active|{
            let format=PixelFormat::new(ColorMode::Rgb,sample,true);
            let source=Surface::with_default(format,&[0.2,0.4,0.6,1.0]);
            let target=*active;doc.layers.push(Layer::new("Wide sampled contributor",LayerContent::Raster(source)));*active=target;Ok(())
        }).unwrap();
        let before=s.active().unwrap().doc.clone();let mixer=s.tools.mixer.clone();let revision=s.active().unwrap().revision;
        let mut p=params("paint.mixerBrush");p["sampleAllLayers"]=json!(true);
        assert!(s.execute("paint.mixerBrush",p.clone()).is_err());assert!(s.active().unwrap().doc.shares_state(&before));assert_eq!(s.active().unwrap().revision,revision);assert_eq!(s.tools.mixer,mixer);
        // The same narrow target without whole-scene pickup remains supported.
        p["sampleAllLayers"]=json!(false);s.execute("paint.mixerBrush",p).unwrap();
    }
}

#[test]
fn ordinary_numeric_mask_on_wide_document_is_not_blanket_refused() {
    for sample in [SampleType::U32,SampleType::F64] {
        let mut s=fixture(sample);
        s.edit("Add ordinary mask",|doc,_|{doc.layers[0].mask=Some(LayerMask::reveal_all());Ok(())}).unwrap();
        let before=s.active().unwrap().doc.clone();let layer_bytes=before.layers[0].surface().unwrap().try_read_interleaved_with(Rect::new(0,0,8,8),&Interrupt::NONE).unwrap();
        let mut p=params("paint.mixerBrush");p["target"]=json!("mask");p["sampleAllLayers"]=json!(false);
        s.execute("paint.mixerBrush",p).unwrap();
        let after=&s.active().unwrap().doc;
        assert_eq!(after.layers[0].surface().unwrap().try_read_interleaved_with(Rect::new(0,0,8,8),&Interrupt::NONE).unwrap(),layer_bytes);
        assert_ne!(after.layers[0].mask,before.layers[0].mask);
        s.execute("edit.undo",Value::Null).unwrap();assert_eq!(*s.active().unwrap().doc,*before);
    }
}

fn floating_narrow_session_with_wide_scene()->Session {
    let mut s=fixture(SampleType::F32);
    s.edit("Wide scene contributor",|doc,_|{doc.layers.push(Layer::raster("Wide contributor",PixelFormat::new(ColorMode::Rgb,SampleType::F64,true)));Ok(())}).unwrap();
    // Make the visible contributor nonempty, so this is not a metadata-only
    // impossible scene. Preserve the original narrow active target.
    s.edit("Wide samples",|doc,_|{*doc.layers[1].surface_mut().unwrap()=Surface::with_default(PixelFormat::new(ColorMode::Rgb,SampleType::F64,true),&[0.2,0.4,0.6,1.0]);Ok(())}).unwrap();
    s.execute("select.all",json!({})).unwrap();
    s.edit("Real redo",|doc,_|{doc.name="Redo retained".into();Ok(())}).unwrap();s.execute("edit.undo",Value::Null).unwrap();
    s.execute("select.float",json!({"dx":1,"dy":0,"copy":true})).unwrap();s
}
#[test]
fn mixed_scene_refusal_precedes_floating_anchor_for_all_mixer_precedence_routes() {
    for setting in ["current","preset","patch","scalar"] {
        let mut s=floating_narrow_session_with_wide_scene();let mut p=params("paint.mixerBrush");
        s.tools.brush.mixer.sample_all_layers=false;
        match setting {
            "current"=>s.tools.brush.mixer.sample_all_layers=true,
            "preset"=>{s.tools.presets[0].brush.mixer.sample_all_layers=true;p["preset"]=json!(s.tools.presets[0].name);},
            "patch"=>p["brush"]=json!({"mixer":{"sampleAllLayers":true}}),
            "scalar"=>p["sampleAllLayers"]=json!(true),
            _=>unreachable!(),
        }
        let st=s.active().unwrap();let before=st.doc.clone();let revision=st.revision;let floating=st.floating.clone().unwrap();let past=st.history.past_len();let redo=st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();let mixer=s.tools.mixer.clone();
        assert!(s.execute("paint.mixerBrush",p).is_err(),"{setting}");
        let st=s.active().unwrap();assert!(st.doc.shares_state(&before));assert_eq!(st.revision,revision);assert_eq!(st.history.past_len(),past);assert_eq!(st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),redo);assert_eq!(s.tools.mixer,mixer);
        let retained=st.floating.as_ref().expect("refusal must not anchor the floating preview");assert_eq!(retained.offset,floating.offset);assert_eq!(retained.revision,floating.revision);assert!(std::sync::Arc::ptr_eq(&retained.parts,&floating.parts));
    }
}
#[test]
fn scalar_and_patch_override_preset_scene_pickup_without_false_wide_refusal() {
    for scalar_override in [false,true] {
        let mut s=fixture(SampleType::F32);
        s.edit("Wide contributor",|doc,_|{doc.layers.push(Layer::new("Wide samples",LayerContent::Raster(Surface::with_default(PixelFormat::new(ColorMode::Rgb,SampleType::F64,true),&[0.2,0.4,0.6,1.0]))));Ok(())}).unwrap();
        s.tools.presets[0].brush.mixer.sample_all_layers=true;
        let mut p=params("paint.mixerBrush");p["preset"]=json!(s.tools.presets[0].name);
        if scalar_override {p["brush"]=json!({"mixer":{"sampleAllLayers":true}});p["sampleAllLayers"]=json!(false);}
        else {p["brush"]=json!({"mixer":{"sampleAllLayers":false}});}
        s.execute("paint.mixerBrush",p).unwrap();
    }
}
#[test]
fn implicit_wide_numeric_views_refuse_before_floating_normalization() {
    use photocraft_engine::channel_cmds::ChannelTarget;
    for target in ["channel","mask","quick-mask"] {
        let mut s=fixture(SampleType::F32);
        s.edit("Wide numeric target",|doc,_|{
            let numeric=Surface::with_default(PixelFormat::new(ColorMode::Grayscale,SampleType::F64,false),&[1.0]);
            match target {
                "channel"=>doc.channels.push(AlphaChannel::new("Wide",numeric)),
                "mask"=>doc.layers[0].mask=Some(LayerMask{surface:std::sync::Arc::new(numeric),..LayerMask::reveal_all()}),
                "quick-mask"=>doc.quick_mask=Some(AlphaChannel::new("Quick Mask",numeric)),
                _=>unreachable!(),
            }Ok(())
        }).unwrap();
        s.execute("select.all",json!({})).unwrap();s.execute("select.float",json!({"dx":1,"copy":true})).unwrap();
        s.active_mut().unwrap().channel_view.target=match target {"channel"=>ChannelTarget::Alpha(0),"mask"=>ChannelTarget::LayerMask,_=>ChannelTarget::Composite};
        let before=s.active().unwrap().doc.clone();let revision=s.active().unwrap().revision;let floating=s.active().unwrap().floating.clone().unwrap();
        let mut p=params("paint.mixerBrush");p.as_object_mut().unwrap().remove("target");
        assert!(s.execute("paint.mixerBrush",p).is_err());
        let st=s.active().unwrap();assert!(st.doc.shares_state(&before));assert_eq!(st.revision,revision);let retained=st.floating.as_ref().unwrap();assert!(std::sync::Arc::ptr_eq(&retained.parts,&floating.parts));assert_eq!(retained.offset,floating.offset);
    }
}
