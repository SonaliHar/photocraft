//! Registry regression drafts: destructive native edits must not create adjustment layers.
//! Intentionally unrun during implementation-first integration.
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, LayerId, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};

fn fixture(mode:ColorMode, sample:SampleType)->(Session,LayerId,Vec<u8>){
    let mut doc=Document::new("Native adjustment dispatch",Size::new(1,1),mode,sample);
    let mut layer=Layer::raster("Original pixels",doc.pixel_format());let id=layer.id;
    let codes=if mode==ColorMode::Rgb{vec![64u32,128,192]}else{vec![128]};
    let mut bytes=Vec::new();
    for code in codes{
        match sample{
            SampleType::U32=>bytes.extend_from_slice(&(code*(u32::MAX/255)).to_ne_bytes()),
            SampleType::F64=>bytes.extend_from_slice(&(f64::from(code)/255.0).to_ne_bytes()),
            _=>panic!("fixture only native wide depths"),
        }
    }
    let alpha=match sample{SampleType::U32=>17u32.to_ne_bytes().to_vec(),SampleType::F64=>0.75f64.to_ne_bytes().to_vec(),_=>panic!("fixture depth")};
    bytes.extend_from_slice(&alpha);
    layer.content=LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(),Rect::new(-1,0,0,1),&bytes,&Interrupt::NONE).unwrap());
    doc.layers.push(layer);let mut session=Session::new();session.add_document(doc,None).unwrap();(session,id,bytes)
}
fn raw(session:&Session,id:LayerId)->Vec<u8>{session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().to_interleaved(Rect::new(-1,0,0,1))}

#[test]
fn registered_image_adjustments_edit_existing_native_raster_once_and_preserve_alpha(){
    for mode in [ColorMode::Rgb,ColorMode::Grayscale]{for depth in [SampleType::U32,SampleType::F64]{
        for (kind,params) in [("invert",json!({})),("threshold",json!({"level":128})),("posterize",json!({"levels":4}))]{
            let(mut session,id,before_bytes)=fixture(mode,depth);
            let st=session.active().unwrap();let before=st.doc.clone();let revision=st.revision;let steps=st.history.past_len();
            session.execute(&format!("image.adjustments.{kind}"),params).unwrap();
            let st=session.active().unwrap();let changed=raw(&session,id);
            assert_eq!(st.doc.layers.len(),1,"{kind} must edit pixels, not insert a layer");
            assert_eq!(st.active_layer,Some(id));assert!(matches!(st.doc.layer(id).unwrap().content,LayerContent::Raster(_)));
            assert_ne!(changed,before_bytes,"{kind}/{mode:?}/{depth:?} reaches the destructive route");
            assert_eq!(changed.get(changed.len()-depth.bytes()..).unwrap(),before_bytes.get(before_bytes.len()-depth.bytes()..).unwrap(),"alpha is raw");
            assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().default_bytes(),before.layer(id).unwrap().surface().unwrap().default_bytes());
            assert_eq!(st.doc.size,before.size);assert_eq!(st.doc.icc_profile,before.icc_profile);assert_eq!(st.doc.sample_encoding,before.sample_encoding);
            assert_eq!(st.history.past_len(),steps+1);assert_eq!(st.revision,revision+1);
            session.execute("edit.undo",Value::Null).unwrap();assert_eq!(raw(&session,id),before_bytes);
            session.execute("edit.redo",Value::Null).unwrap();assert_eq!(raw(&session,id),changed);
        }
    }}
}
#[test]
fn registered_adjustment_layer_creation_keeps_original_native_pixels(){
    for depth in [SampleType::U32,SampleType::F64]{for kind in ["invert","threshold","posterize"]{
        let(mut session,id,before_bytes)=fixture(ColorMode::Rgb,depth);let steps=session.active().unwrap().history.past_len();
        let output=session.execute(&format!("layer.newAdjustmentLayer.{kind}"),json!({"level":128,"levels":4})).unwrap();
        let st=session.active().unwrap();let added=LayerId(output["layer"].as_u64().unwrap());
        assert_ne!(added,id);assert_eq!(st.doc.layers.len(),2);assert_eq!(raw(&session,id),before_bytes);
        let adjustment=&st.doc.layer(added).unwrap().content;
        assert!(match(kind,adjustment){("invert",LayerContent::Adjustment(Adjustment::Invert))|("threshold",LayerContent::Adjustment(Adjustment::Threshold{..}))|("posterize",LayerContent::Adjustment(Adjustment::Posterize{..}))=>true,_=>false});
        assert_eq!(st.history.past_len(),steps+1);
        session.execute("edit.undo",Value::Null).unwrap();assert_eq!(session.active().unwrap().doc.layers.len(),1);assert_eq!(raw(&session,id),before_bytes);
    }}
}
#[test]
fn destructive_noop_and_stale_capture_keep_real_redo_and_layer_count(){
    let(mut session,id,bytes)=fixture(ColorMode::Rgb,SampleType::U32);
    session.edit("Seed",|doc,_|{doc.name="Seed".into();Ok(())}).unwrap();session.execute("edit.undo",Value::Null).unwrap();
    let st=session.active().unwrap();let before=st.doc.clone();let revision=st.revision;
    for kind in ["invert","threshold","posterize"]{
        assert!(session.execute(&format!("image.adjustments.{kind}"),json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
        session.execute(&format!("image.adjustments.{kind}"),json!({"opacity":0})).unwrap();
        let st=session.active().unwrap();assert!(st.doc.shares_state(&before));assert_eq!(st.revision,revision);assert!(st.history.can_redo());assert_eq!(raw(&session,id),bytes);
    }
}

#[test]
fn registered_desaturate_keeps_captured_params_and_gray_identity_redo(){
    for mode in [ColorMode::Rgb,ColorMode::Grayscale]{for depth in [SampleType::U32,SampleType::F64]{
        let(mut session,id,before_bytes)=fixture(mode,depth);
        session.edit("Seed",|doc,_|{doc.name="Seed".into();Ok(())}).unwrap();
        session.execute("edit.undo",Value::Null).unwrap();
        let st=session.active().unwrap();let before=st.doc.clone();let revision=st.revision;
        let captured=json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()});
        assert!(session.execute("image.adjustments.desaturate",captured).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(raw(&session,id),before_bytes);assert!(session.active().unwrap().history.can_redo());
        session.execute("image.adjustments.desaturate",json!({"opacity":0})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision,revision);assert!(session.active().unwrap().history.can_redo());
        if mode==ColorMode::Grayscale {
            session.execute("image.adjustments.desaturate",Value::Null).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision,revision);assert!(session.active().unwrap().history.can_redo());
        }
    }}
}

fn hidden_fixture(sample:SampleType,hidden_alpha:u64,hidden_nonfinite:bool,edges_visible:bool)->(Session,LayerId,Vec<u8>){
    let mut doc=Document::new("Hidden native adjustment words",Size::new(3,1),ColorMode::Rgb,sample);
    doc.sample_encoding=Some(photocraft_doc::SampleEncoding::Linear);doc.icc_profile=Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let mut bytes=Vec::new();
    for index in 0..3{
        if sample==SampleType::F64{
            let colors=if index==1&&hidden_nonfinite{[0x7ff8_0000_0000_0073u64,f64::INFINITY.to_bits(),(-0.0f64).to_bits()]}else{[0.25f64.to_bits(),0.5f64.to_bits(),0.75f64.to_bits()]};
            for word in colors{bytes.extend_from_slice(&word.to_ne_bytes());}
            let alpha=if index==1{hidden_alpha}else if edges_visible{1.0f64.to_bits()}else{0.0f64.to_bits()};bytes.extend_from_slice(&alpha.to_ne_bytes());
        }else{
            for word in [u32::MAX/4,u32::MAX/2,u32::MAX-1]{bytes.extend_from_slice(&word.to_ne_bytes());}
            let alpha=if index==1{hidden_alpha as u32}else if edges_visible{u32::MAX}else{0};bytes.extend_from_slice(&alpha.to_ne_bytes());
        }
    }
    let mut layer=Layer::raster("Pixels",doc.pixel_format());let id=layer.id;layer.content=LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(),Rect::new(0,0,3,1),&bytes,&Interrupt::NONE).unwrap());doc.layers.push(layer);
    let mut session=Session::new();session.add_document(doc,None).unwrap();(session,id,bytes)
}
fn native_cases()->[(&'static str,Value);8]{[("invert",json!({})),("threshold",json!({"level":128})),("posterize",json!({"levels":4})),("levels",json!({"outWhite":128})),("curves",json!({"points":[[0,255],[255,0]]})),("exposure",json!({"exposure":1})),("brightnessContrast",json!({"brightness":1,"contrast":1,"legacy":true})),("desaturate",json!({}))]}
#[test]
fn registered_native_adjustments_skip_hidden_color_words_inside_visible_extent(){
    for sample in [SampleType::U32,SampleType::F64]{for(kind,params)in native_cases(){
        let alpha=if sample==SampleType::F64{(-0.0f64).to_bits()}else{0};
        let(mut session,id,before)=hidden_fixture(sample,alpha,true,true);let steps=session.active().unwrap().history.past_len();
        session.execute(&format!("image.adjustments.{kind}"),params).unwrap();
        let source=session.active().unwrap().doc.layer(id).unwrap().surface().unwrap();let after=source.to_interleaved(Rect::new(0,0,3,1));let bpp=source.format().bytes_per_pixel();
        assert_eq!(after.get(bpp..2*bpp),before.get(bpp..2*bpp),"{kind}/{sample:?}: complete hidden native pixel remains raw");
        assert_ne!(after.get(..bpp),before.get(..bpp),"visible neighbors still transform");
        assert_eq!(session.active().unwrap().history.past_len(),steps+1);
    }}
}
#[test]
fn registered_native_adjustments_do_not_treat_nan_alpha_as_transparent(){
    let alpha=0x7ff8_0000_0000_0042u64;
    for(kind,params)in native_cases(){
        let(mut session,id,before)=hidden_fixture(SampleType::F64,alpha,false,true);
        session.execute(&format!("image.adjustments.{kind}"),params).unwrap();
        let after=session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().to_interleaved(Rect::new(0,0,3,1));
        assert_ne!(after.get(32..56),before.get(32..56),"{kind}: NaN <=0 is false, finite colors transform");
        assert_eq!(after.get(56..64),before.get(56..64),"NaN alpha payload remains raw");
    }
}
#[test]
fn hidden_only_registered_native_noops_keep_real_redo(){
    for(kind,params)in native_cases(){
        let(mut session,id,before_bytes)=hidden_fixture(SampleType::F64,(-0.0f64).to_bits(),true,false);
        session.edit("Seed",|doc,_|{doc.name="Seed".into();Ok(())}).unwrap();session.execute("edit.undo",Value::Null).unwrap();
        let before=session.active().unwrap().doc.clone();let revision=session.active().unwrap().revision;
        session.execute(&format!("image.adjustments.{kind}"),params).unwrap();
        let state=session.active().unwrap();assert!(state.doc.shares_state(&before),"{kind}");assert_eq!(state.revision,revision);assert!(state.history.can_redo());
        assert_eq!(state.doc.layer(id).unwrap().surface().unwrap().to_interleaved(Rect::new(0,0,3,1)),before_bytes);
    }
}
