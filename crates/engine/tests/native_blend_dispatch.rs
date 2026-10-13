//! Unrun integration regressions for explicit native Linear unit paint blending.
//! No imported/GIMP blend, numeric mask, HDR, or profile-fallback qualification.
use photocraft_cms::{Builtin,Intent,Transform};
use photocraft_color::{BlendMode,ColorMode,PixelFormat,SampleType};
use photocraft_doc::{AlphaChannel,Document,Layer,LayerContent,LayerId,LayerMask,SampleEncoding,Size};
use photocraft_engine::{Session,brush_cmds::{self,LiveStroke}};
use photocraft_geom::Rect;
use photocraft_paint::{StrokePoint,StrokeRenderer,stroke_controlled::StrokeLimits,wide_blend_controlled::NativeUnitBlend};
use photocraft_raster::{Interrupt,Surface};
use serde_json::{Value,json};
use std::sync::Arc;

fn document(sample:SampleType,locked:bool)->(Document,LayerId) {document_in(ColorMode::Rgb,sample,locked)}
fn document_in(mode:ColorMode,sample:SampleType,locked:bool)->(Document,LayerId) {
    let mut doc=Document::new("Explicit native blend",Size::new(8,8),mode,sample);
    doc.sample_encoding=Some(SampleEncoding::Linear);
    doc.icc_profile=Some(if mode==ColorMode::Rgb {Builtin::LinearSrgb.profile().to_bytes()} else {photocraft_cms::builtin::linear_gray().to_bytes()});
    let format=doc.pixel_format();let mut bytes=Vec::new();
    for _ in 0..64 {
        if sample==SampleType::U32 {
            let colors=if mode==ColorMode::Rgb {vec![0x8000_0001u32,0x4000_0001,0xbfff_ffff]} else {vec![0x8000_0001u32]};
            for value in colors {bytes.extend_from_slice(&value.to_ne_bytes());}bytes.extend_from_slice(&0xbfff_ffffu32.to_ne_bytes());
        } else {
            let colors=if mode==ColorMode::Rgb {vec![f64::from_bits(0x3fe0_0000_0000_0001),0.25,0.75]} else {vec![f64::from_bits(0x3fe0_0000_0000_0001)]};
            for value in colors {bytes.extend_from_slice(&value.to_ne_bytes());}bytes.extend_from_slice(&0.75f64.to_ne_bytes());
        }
    }
    let mut layer=Layer::new("Native pixels",LayerContent::Raster(Surface::try_from_interleaved_with(format,doc.bounds(),&bytes,&Interrupt::NONE).unwrap()));
    layer.locks.transparency=locked;let id=layer.id;doc.layers.push(layer);(doc,id)
}
fn session(doc:Document)->Session {
    let mut s=Session::new();s.add_document(doc,None).unwrap();
    s.tools.brush.pressure_size=false;s.tools.brush.pressure_opacity=false;
    s.tools.brush.smoothing.enabled=false;
    s
}
fn params(mode:&str)->Value {json!({"mode":mode,"target":"pixels","size":3.0,"hardness":1.0,"opacity":0.5,"flow":1.0,"smoothing":0.0,"seed":17,"color":[0.5,0.6,0.7,0.8],"points":[[3.0,3.0,1.0],[5.0,3.0,1.0]]})}
fn pixels(s:&Session,id:LayerId)->Vec<u8>{s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().try_read_interleaved_with(Rect::new(0,0,8,8),&Interrupt::NONE).unwrap()}
fn lower_reference(s:&Session,id:LayerId,p:&Value,mode:NativeUnitBlend,locked:bool)->Surface {
    let doc=&s.active().unwrap().doc;
    let pre=doc.layer(id).unwrap().surface().unwrap();let mut output=pre.clone();
    let mut brush=brush_cmds::resolve_brush(s,p,"paint.stroke").unwrap();
    brush.mode=match mode{NativeUnitBlend::Multiply=>BlendMode::Multiply,NativeUnitBlend::Screen=>BlendMode::Screen};
    let points=brush_cmds::parse_points(p,"paint.stroke").unwrap();
    let mut renderer=StrokeRenderer::try_new_with_interrupt(&brush,Some(pre.format()),1.0,StrokeLimits::default(),&Interrupt::NONE).unwrap();
    renderer.push_with_interrupt(&points,StrokeLimits::default(),&Interrupt::NONE).unwrap();
    renderer.finish_with_interrupt(StrokeLimits::default(),&Interrupt::NONE).unwrap();
    // Public sRGB tool color is explicitly converted to the fixture's current
    // Linear ICC; the lower oracle receives native color, not inferred metadata.
    let profile=if doc.mode==ColorMode::Rgb {Builtin::LinearSrgb.profile()} else {photocraft_cms::builtin::linear_gray()};
    let transform=Transform::new(Builtin::Srgb.profile(),profile,Intent::RelativeColorimetric,true).unwrap();
    let n=doc.mode.color_channels();let mut color=[0.0f64;4];transform.try_eval_f64(&[0.5,0.6,0.7],&mut color[..n]).unwrap();color[n]=0.8;
    let raw=color[..n+1].iter().flat_map(|value|value.to_ne_bytes()).collect::<Vec<_>>();
    let native=Surface::try_with_default_bytes(PixelFormat::new(doc.mode,SampleType::F64,true),&raw).unwrap();
    renderer.try_composite_union_many_wide_with_blend(&[],pre,&mut output,&native,None,locked,None,Some(mode),StrokeLimits::default(),&Interrupt::NONE).unwrap();output
}
#[test]
fn registered_native_blends_match_covered_lower_and_one_history_step() {
    for sample in [SampleType::U32,SampleType::F64] {for locked in [false,true] {
        for (name,mode) in [("multiply",NativeUnitBlend::Multiply),("screen",NativeUnitBlend::Screen)] {
            let(doc,id)=document(sample,locked);let mut s=session(doc);let p=params(name);
            let before=s.active().unwrap().doc.clone();let bytes=pixels(&s,id);let revision=s.active().unwrap().revision;let steps=s.active().unwrap().history.past_len();
            let expected=lower_reference(&s,id,&p,mode,locked);
            s.execute("paint.stroke",p).unwrap();
            let actual=s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();assert_eq!(actual,&expected);
            let after=pixels(&s,id);assert_ne!(after,bytes);
            let st=s.active().unwrap();assert_eq!(st.history.past_len(),steps+1);assert_eq!(st.revision,revision+1);assert_eq!(st.doc.layers.len(),1);assert_eq!(st.active_layer,Some(id));
            assert_eq!(st.doc.icc_profile,before.icc_profile);assert_eq!(st.doc.sample_encoding,before.sample_encoding);assert_eq!(st.doc.source_color_context,before.source_color_context);
            if locked {for (old,new) in bytes.chunks_exact(sample.bytes()*4).zip(after.chunks_exact(sample.bytes()*4)) {assert_eq!(&old[sample.bytes()*3..],&new[sample.bytes()*3..]);}}
            s.execute("edit.undo",Value::Null).unwrap();assert_eq!(*s.active().unwrap().doc,*before);
            s.execute("edit.redo",Value::Null).unwrap();assert_eq!(pixels(&s,id),after);
        }
    }}
}
#[test]
fn native_live_batches_match_registered_commit_and_keep_session_private() {
    for sample in [SampleType::U32,SampleType::F64] {for mode in ["multiply","screen"] {
        let(doc,id)=document(sample,true);let mut s=session(doc);let before=s.active().unwrap().doc.clone();let revision=s.active().unwrap().revision;
        let mut p=params(mode);p["points"]=json!([[3.0,3.0,1.0]]);
        let mut live=LiveStroke::begin(&s,&p).unwrap();live.push(&[StrokePoint::new(5.0,3.0,1.0)]).unwrap();
        let shown=live.doc.layer(id).unwrap().surface().unwrap().clone();
        assert!(s.active().unwrap().doc.shares_state(&before));assert_eq!(s.active().unwrap().revision,revision);
        p["points"]=json!([[3.0,3.0,1.0],[5.0,3.0,1.0]]);p["seed"]=json!(live.seed);
        // First exercise ordinary registered rendering against the shown result.
        s.execute("paint.stroke",p.clone()).unwrap();assert_eq!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap(),&shown);
        s.execute("edit.undo",Value::Null).unwrap();
        // A new live witness must capture the post-Undo revision, not reuse stale
        // authority from the first preview. Offer/commit still uses one edit.
        let live=LiveStroke::begin(&s,&p).unwrap();let offered=live.doc.layer(id).unwrap().surface().unwrap().clone();
        s.offer_live_stroke(live).unwrap();let steps=s.active().unwrap().history.past_len();
        s.execute("paint.stroke",p).unwrap();assert_eq!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap(),&offered);assert_eq!(s.active().unwrap().history.past_len(),steps+1);
    }}
}
#[test]
fn unqualified_contexts_and_numeric_targets_refuse_without_destroying_redo() {
    for sample in [SampleType::U32,SampleType::F64] {for case in ["implicit","nonlinear","no-icc","source-context","mask","channel"] {for mode in ["multiply","screen"] {
        let(mut doc,id)=document(sample,false);let mut p=params(mode);
        match case {
            "implicit"=>doc.sample_encoding=None,
            "nonlinear"=>{doc.sample_encoding=Some(SampleEncoding::NonlinearProfile);doc.icc_profile=Some(Builtin::Srgb.profile().to_bytes());},
            "no-icc"=>doc.icc_profile=None,
            "source-context"=>doc.source_color_context=Some(photocraft_doc::sample_encoding::SourceColorContext{original_icc:doc.icc_profile.clone(),source_precision:650}),
            "mask"=>{doc.layer_mut(id).unwrap().mask=Some(LayerMask{surface:Arc::new(Surface::with_default(PixelFormat::new(ColorMode::Grayscale,sample,false),&[1.0])),..LayerMask::reveal_all()});p["target"]=json!("mask");},
            "channel"=>{doc.channels.push(AlphaChannel::new("Numeric",Surface::with_default(PixelFormat::new(ColorMode::Grayscale,sample,false),&[1.0])));p["target"]=json!({"channel":0});},
            _=>unreachable!(),
        }
        let mut s=session(doc);s.edit("Real redo seed",|doc,_|{doc.name="Seed".into();Ok(())}).unwrap();s.execute("edit.undo",Value::Null).unwrap();
        let st=s.active().unwrap();let before=st.doc.clone();let revision=st.revision;let past=st.history.past_len();let active=st.active_layer;
        assert!(LiveStroke::begin(&s,&p).is_err(),"live {sample:?}/{case}/{mode}");
        assert!(s.execute("paint.stroke",p).is_err(),"registered {sample:?}/{case}/{mode}");
        let st=s.active().unwrap();assert!(st.doc.shares_state(&before));assert_eq!(st.revision,revision);assert_eq!(st.history.past_len(),past);assert_eq!(st.active_layer,active);assert!(st.history.can_redo());
        s.execute("edit.redo",Value::Null).unwrap();assert_eq!(s.active().unwrap().doc.name,"Seed");
    }}}
}

#[test]
fn gray_native_registered_blend_matches_lower_without_rgb_conversion_of_target() {
    for sample in [SampleType::U32,SampleType::F64] {for (name,blend) in [("multiply",NativeUnitBlend::Multiply),("screen",NativeUnitBlend::Screen)] {
        let(doc,id)=document_in(ColorMode::Grayscale,sample,true);let mut s=session(doc);let p=params(name);
        let before=pixels(&s,id);let expected=lower_reference(&s,id,&p,blend,true);
        let live=LiveStroke::begin(&s,&p).unwrap();assert_eq!(live.doc.layer(id).unwrap().surface().unwrap(),&expected);
        s.execute("paint.stroke",p).unwrap();assert_eq!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap(),&expected);
        s.execute("edit.undo",Value::Null).unwrap();assert_eq!(pixels(&s,id),before);
    }}
}
