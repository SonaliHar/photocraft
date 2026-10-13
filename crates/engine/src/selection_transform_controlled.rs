//! Captured controlled Transform Selection; preserves upstream outline warp equations.
use crate::{DocumentSnapshot,EngineError,Result,Session};
use photocraft_algo::{transform::{Homography,Interp},wide_geometry::GeometryLimits};
use photocraft_color::{ColorMode,SampleType};
use photocraft_geom::Rect;
use photocraft_geom::warp::Warp;
use photocraft_raster::{Interrupt,MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value,json};
const ID:&str="select.transformSelection";
fn bad(msg:impl Into<String>)->EngineError{EngineError::BadParams{cmd:ID.into(),msg:msg.into()}}
fn poll(c:&Interrupt<'_>)->Result<()>{c.check().map_err(|_|EngineError::Cancelled)}
fn failure(c:&Interrupt<'_>,e:impl std::fmt::Display)->EngineError{if c.cancelled(){EngineError::Cancelled}else{bad(e.to_string())}}
fn decimal(v:&Value)->Result<u64>{let s=v.as_str().filter(|s|!s.is_empty()&&s.len()<=20&&s.bytes().all(|b|b.is_ascii_digit())&&(s.len()==1||!s.starts_with('0'))).ok_or_else(||bad("captured IDs require canonical decimal strings"))?;s.parse().map_err(|_|bad("identity overflow"))}
fn number(v:&Value)->Result<f64>{v.as_f64().filter(|v|v.is_finite()&&v.abs()<=1_000_000.0).ok_or_else(||bad("finite bounded coordinate required"))}
fn array<const N:usize>(v:&Value)->Result<[f64;N]>{let a=v.as_array().filter(|a|a.len()==N).ok_or_else(||bad("incorrect coordinate array shape"))?;let mut out=[0.0;N];for (dst,v) in out.iter_mut().zip(a){*dst=number(v)?;}Ok(out)}
fn census(v:&Value,depth:usize,count:&mut usize)->Result<()>{
    *count=count.checked_add(1).filter(|n|*n<=120000).ok_or_else(||bad("warp node count limit"))?;
    if depth>8{return Err(bad("warp nesting limit"));}
    match v{Value::Array(a)=>{if a.len()>40000{return Err(bad("warp array limit"));}for v in a{census(v,depth+1,count)?;}},Value::Object(o)=>{if o.len()>12||o.keys().any(|k|k.len()>32){return Err(bad("warp object limit"));}for v in o.values(){census(v,depth+1,count)?;}},Value::String(s)=>{if s.len()>64{return Err(bad("warp string limit"));}},Value::Number(_)=>{number(v)?;},Value::Bool(_)|Value::Null=>{}}
    Ok(())
}
fn envelope(p:&Value)->Result<()>{
    if p.is_null(){return Ok(());}
    let o=p.as_object().filter(|o|o.len()<=25&&o.keys().all(|k|matches!(k.as_str(),"rect"|"reference"|"scaleX"|"scaleY"|"rotate"|"skewX"|"skewY"|"dx"|"dy"|"x"|"y"|"quad"|"corners"|"matrix"|"interpolation"|"warp"|"style"|"mesh"|"grid"|"bend"|"hDistort"|"vDistort"|"vertical"|"documentId"|"expectedRevision"))).ok_or_else(||bad("bounded Transform Selection parameters required"))?;
    for (k,v) in o{match k.as_str(){
        "documentId"|"expectedRevision"=>{decimal(v)?;},
        "rect"=>{array::<4>(v)?;},"matrix"=>{array::<6>(v)?;},
        "quad"|"corners"=>{let a=v.as_array().filter(|a|a.len()==4).ok_or_else(||bad("quad requires four points"))?;for p in a{array::<2>(p)?;}},
        "reference"=>match v{Value::Array(_)=>{array::<2>(v)?;},Value::String(s)if matches!(s.as_str(),"center"|"topLeft"|"top"|"topRight"|"left"|"right"|"bottomLeft"|"bottom"|"bottomRight")=>{},_=>return Err(bad("invalid reference"))},
        "interpolation"=>{if !matches!(v.as_str(),Some("nearest"|"nearestNeighbor"|"bilinear"|"bicubic")){return Err(bad("unsupported interpolation"));}},
        "vertical"=>{if !v.is_boolean(){return Err(bad("vertical must be boolean"));}},
        "grid"=>{let a=v.as_array().filter(|a|a.len()==2).ok_or_else(||bad("grid requires two integers"))?;for v in a{if v.as_u64().is_none_or(|n|!(1..=64).contains(&n)){return Err(bad("grid must be 1..64"));}}},
        "warp"|"mesh"=>census(v,0,&mut 0)?,
        "style"=>{if v.as_str().is_none_or(|s|s.len()>32){return Err(bad("bounded warp style required"));}},
        _=>{let n=number(v)?;if matches!(k.as_str(),"skewX"|"skewY")&&n.abs()>=89.0{return Err(bad("skew must be less than 89 degrees"));}if matches!(k.as_str(),"scaleX"|"scaleY")&&(n==0.0||n.abs()>10000.0){return Err(bad("scale must be nonzero and bounded"));}}
    }}Ok(())
}
enum Map{Homography(Homography),Mesh(Warp)}
impl Map{fn identity(&self)->bool{match self{Self::Homography(h)=>h.0.iter().zip(Homography::IDENTITY.0).all(|(a,b)|(a-b).abs()<1e-12),Self::Mesh(w)=>w.is_identity()}}}
fn mapping(p:&Value,b:Rect)->Result<Map>{
    let rect=match p.get("rect"){Some(v)=>array::<4>(v)?,None=>[f64::from(b.x0),f64::from(b.y0),f64::from(b.x1),f64::from(b.y1)]};
    if rect[2]<rect[0]||rect[3]<rect[1]{return Err(bad("reversed transform rectangle"));}
    if ["warp","style","mesh","grid"].iter().any(|k|p.get(*k).is_some()){
        let r=[rect[0].floor(),rect[1].floor(),rect[2].ceil(),rect[3].ceil()];
        let w=crate::warp_cmds::warp_from_params(ID,p,r)?;
        if w.bounds.iter().any(|v|!v.is_finite()||v.abs()>1_000_000.0)||[w.bend,w.h_distort,w.v_distort].iter().any(|v|!v.is_finite()||v.abs()>100.0){return Err(bad("invalid bounded warp"));}
        return Ok(Map::Mesh(w));
    }
    let h=if let Some(v)=p.get("quad").or_else(||p.get("corners")){
        let a=v.as_array().ok_or_else(||bad("invalid quad"))?;let mut q=[[0.0;2];4];for (dst,v) in q.iter_mut().zip(a){*dst=array::<2>(v)?;}Homography::rect_to_quad(rect,q).ok_or_else(||bad("degenerate quad"))?
    }else if let Some(v)=p.get("matrix"){let [a,b,c,d,e,f]=array::<6>(v)?;Homography([a,c,e,b,d,f,0.0,0.0,1.0])}else{
        let [x0,y0,x1,y1]=rect;let (cx,cy)=((x0+x1)/2.0,(y0+y1)/2.0);
        let [px,py]=match p.get("reference"){
            Some(Value::Array(_))=>array::<2>(p.get("reference").ok_or_else(||bad("reference"))?)?,
            Some(Value::String(k))=>match k.as_str(){"topLeft"=>[x0,y0],"top"=>[cx,y0],"topRight"=>[x1,y0],"left"=>[x0,cy],"right"=>[x1,cy],"bottomLeft"=>[x0,y1],"bottom"=>[cx,y1],"bottomRight"=>[x1,y1],_=>[cx,cy]},_=>[cx,cy]};
        let get=|key:&str,default:f64|p.get(key).and_then(Value::as_f64).unwrap_or(default);
        let (sx,sy)=(get("scaleX",100.0)/100.0,get("scaleY",100.0)/100.0);let a=get("rotate",0.0).to_radians();let (sa,ca)=a.sin_cos();
        let (kx,ky)=(get("skewX",0.0).to_radians().tan(),get("skewY",0.0).to_radians().tan());let (dx,dy)=(get("dx",get("x",0.0)),get("dy",get("y",0.0)));
        let t=|x,y|Homography([1.0,0.0,x,0.0,1.0,y,0.0,0.0,1.0]);
        t(px+dx,py+dy).mul(&Homography([ca,-sa,0.0,sa,ca,0.0,0.0,0.0,1.0])).mul(&Homography([1.0,kx,0.0,ky,1.0,0.0,0.0,0.0,1.0])).mul(&Homography([sx,0.0,0.0,0.0,sy,0.0,0.0,0.0,1.0])).mul(&t(-px,-py))
    };
    if h.0.iter().any(|v|!v.is_finite())||h.inverse().is_none(){return Err(bad("the transform collapses the selection"));}Ok(Map::Homography(h))
}
fn context(session:&Session,p:&Value,c:&Interrupt<'_>)->Result<()>{
    envelope(p)?;poll(c)?;let st=session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"),p.get("expectedRevision")){(None,None)=>{},(Some(id),Some(rev))if decimal(id)?==st.doc.id.0&&decimal(rev)?==st.revision=>{},_=>return Err(bad("captured selection target changed"))}
    if st.revision==u64::MAX||st.floating.is_some()||st.coalesce.is_some()||st.doc.xcf_state.as_ref().is_some_and(|s|s.floating.is_some())||session.coalesce_request.is_some()||session.color_restrict.is_some()||session.command_has_pending_job()||session.command_has_job_on(st.doc.id){return Err(bad("selection target is not idle"));}
    if st.channel_view.color_hidden.len()>16||st.channel_view.color_hidden.capacity()>16||st.channel_view.alpha_visible.len()>10000||st.channel_view.alpha_visible.capacity()>10000{return Err(bad("channel view capacity limit"));}
    let source=st.doc.selection.as_ref().ok_or_else(||bad("no selection"))?;
    if source.format().mode!=ColorMode::Grayscale||source.format().alpha||source.tile_count()>100000{return Err(bad("bounded Gray selection required"));}
    crate::selection_inverse_controlled::selection_write_authority(&st.doc,c)
}
pub(crate) fn dispatch_preflight(session:&Session,p:&Value,_background:bool)->Result<()>{
    context(session,p,&Interrupt::NONE)?;let source=session.active().and_then(|s|s.doc.selection.as_ref()).ok_or_else(||bad("no selection"))?;
    let b=source.try_content_bounds_with_interrupt(&Interrupt::NONE).map_err(|e|failure(&Interrupt::NONE,e))?;
    let map=mapping(p,b)?;if !map.identity()&&matches!(source.format().sample,SampleType::U32|SampleType::F64){return Err(bad("native nonidentity selection interpolation is not qualified"));}Ok(())
}
fn limits(doc:&photocraft_doc::Document,c:&Interrupt<'_>)->Result<GeometryLimits>{
    let l=photocraft_io::NativeSaveLimits::default().preparation;
    let model=photocraft_io::retained_native_model_bytes(doc,l,c).map_err(|e|failure(c,e))? as u64;
    let payload=photocraft_io::retained_native_payload_bytes(doc,l,c).map_err(|e|failure(c,e))? as u64;
    let raster=DocumentSnapshot::resident_raster_footprint_of(doc,c)?;
    let base=raster.retained_bytes.checked_add(raster.clone_metadata_bytes.checked_mul(4).ok_or_else(||bad("clone headroom"))?).and_then(|n|n.checked_add(model.checked_mul(4)?)).and_then(|n|n.checked_add(payload)).ok_or_else(||bad("retained headroom"))?;
    Ok(GeometryLimits{resident_bytes:MAX_CHECKED_RASTER_BYTES.checked_sub(base).ok_or_else(||bad("retained document limit"))?,work_units:GeometryLimits::default().work_units})
}
pub(crate) fn run(session:&mut Session,p:&Value)->Result<Value>{
    context(session,p,&Interrupt::NONE)?;let st=session.active().ok_or(EngineError::NoDocument)?;let source=st.doc.selection.as_ref().ok_or_else(||bad("no selection"))?;
    let b=source.try_content_bounds_with_interrupt(&Interrupt::NONE).map_err(|e|failure(&Interrupt::NONE,e))?;let map=mapping(p,b)?;
    if map.identity(){return Ok(json!({"changed":false,"selected":true}));}
    if matches!(source.format().sample,SampleType::U32|SampleType::F64){return Err(bad("native nonidentity selection interpolation is not qualified"));}
    let admitted=limits(&st.doc,&Interrupt::NONE)?;st.doc.validate_resident()?;
    let baseline=st.doc.clone();let worker=baseline.clone();let revision=st.revision;let view=st.channel_view.clone();let interp=Interp::parse(p.get("interpolation").and_then(Value::as_str).unwrap_or("bilinear"));
    crate::jobs::run(session,"Transform Selection",true,move |ctx|ctx.stage(0.0,0.95,"Transform Selection",|c|{
        poll(c)?;let source=worker.selection.as_ref().ok_or_else(||bad("selection disappeared"))?;
        let transformed=match map{Map::Homography(h)=>photocraft_algo::selection_transform_controlled::try_warp_gray_homography_with_limits(source,&h,interp,admitted,c),Map::Mesh(w)=>photocraft_algo::selection_transform_controlled::try_warp_gray_mesh_with_limits(source,&|x,y|w.map(x,y),interp,admitted,c)}.map_err(|e|failure(c,e))?;
        let b=transformed.try_content_bounds_with_interrupt(c).map_err(|e|failure(c,e))?;let selection=(!b.is_empty()).then_some(transformed);poll(c)?;Ok((selection,ctx.clone()))
    }),move |session,(selection,ctx)|{
        let st=session.active().ok_or(EngineError::NoDocument)?;
        if st.doc.id!=baseline.id||st.revision!=revision||!baseline.downgrade().is_current(&st.doc)||st.channel_view!=view||st.floating.is_some()||st.coalesce.is_some()||session.command_has_job_on(st.doc.id)||session.command_has_pending_job()||session.coalesce_request.is_some()||session.color_restrict.is_some(){return Err(bad("selection target changed during preparation"));}
        ctx.stage(0.95,1.0,"Transform Selection",|c|{
            poll(c)?;
            let unchanged={let current=session.active().ok_or(EngineError::NoDocument)?;
                crate::selection_inverse_controlled::selection_write_authority(&current.doc,c)?;
                current.doc.selection==selection};
            if unchanged{return Ok(json!({"changed":false,"selected":selection.is_some()}));}
            let selected=selection.is_some();let bounds=selection.as_ref().map(|s|s.try_content_bounds_with_interrupt(c)).transpose().map_err(|e|failure(c,e))?;
            let response=json!({"changed":true,"selected":selected,"bounds":bounds.map(|r|[r.x0,r.y0,r.width() as i32,r.height() as i32])});
            poll(c)?;
            session.edit_with_interrupt("Transform Selection",c,move |doc,_|{crate::selection_inverse_controlled::selection_write_authority(doc,c)?;doc.selection=selection;poll(c)})?;
            Ok(response)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::Surface;
    fn session(sample:SampleType)->Session {
        let mut s=Session::new();let mut doc=photocraft_doc::Document::new("selection transform draft",photocraft_geom::Size::new(12,9),ColorMode::Rgb,sample);
        let mut selection=Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale,sample,false));selection.fill_rect(Rect::new(2,3,6,7),&[1.0]);doc.selection=Some(selection);s.add_document(doc,None).unwrap();s
    }
    #[test]
    fn native_identity_is_raw_noop_but_nonidentity_refuses_before_interpretation() {
        for sample in [SampleType::U32,SampleType::F64] {
            let mut s=session(sample);let before=s.active().unwrap().doc.clone();let revision=s.active().unwrap().revision;
            assert_eq!(run(&mut s,&json!({"matrix":[1,0,0,1,0,0]})).unwrap()["changed"],false);
            assert!(run(&mut s,&json!({"dx":1})).is_err());assert_eq!(s.active().unwrap().doc,before);assert_eq!(s.active().unwrap().revision,revision);
        }
    }
    #[test]
    fn registered_outline_edit_moves_selection_only_with_one_undo_and_capture_refusal() {
        let mut s=session(SampleType::U16);let before=s.active().unwrap().doc.clone();let revision=s.active().unwrap().revision;
        let p=json!({"dx":3,"dy":-2,"documentId":before.id.0.to_string(),"expectedRevision":revision.to_string()});
        s.execute(ID,p.clone()).unwrap();assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(),Rect::new(5,1,9,5));
        assert_eq!(s.active().unwrap().doc.layers,before.layers);assert_eq!(s.active().unwrap().doc.physical_resolution,before.physical_resolution);
        assert!(dispatch_preflight(&s,&p,false).is_err());s.execute("edit.undo",Value::Null).unwrap();assert_eq!(s.active().unwrap().doc.selection,before.selection);
    }
    #[test]
    fn hostile_payloads_never_publish_selection_changes() {
        let s=session(SampleType::U8);let before=s.active().unwrap().doc.clone();
        for p in [json!({"matrix":[1,2]}),json!({"corners":[[0,0],[1,0],[1,1],[0,"bad"]]}),json!({"grid":[u64::MAX,1]}),json!({"scaleX":0}),json!({"rect":[0,0,1e20,3]}),json!({"documentId":"01","expectedRevision":"0"})] {
            assert!(dispatch_preflight(&s,&p,false).is_err());assert_eq!(s.active().unwrap().doc,before);
        }
    }
}
