//! Private live painter transactions preserving the upstream incremental state.
//! Generic resident Document cloning and the trusted legacy render callback have
//! entry/exit cancellation boundaries. Their internal allocators are not replaced.
use crate::brush_cmds::{LiveStroke, WideStroke};
use crate::symmetry_cmds::{PaintingSymmetry, SymmetryAxis};
use crate::{EngineError, Result};
use photocraft_geom::Rect;
use photocraft_paint::{StrokePoint, render::{incremental_state_requirements, clone_incremental_state_with_interrupt}, stroke_controlled::{StrokeLimits, StrokeError}};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
use std::{mem::size_of, sync::Arc};
fn bad(message: &str)->EngineError{EngineError::Other(format!("live stroke transaction: {message}"))}
fn poll(control:&Interrupt<'_>)->Result<()>{control.check().map_err(|_|EngineError::Cancelled)}
fn add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or_else(||bad("accounting overflow"))}
fn times(a:u64,b:u64)->Result<u64>{a.checked_mul(b).ok_or_else(||bad("accounting overflow"))}
fn paint(error:StrokeError)->EngineError{match error{StrokeError::Cancelled=>EngineError::Cancelled,error=>bad(&error.to_string())}}
#[derive(Default)]
struct Requirements{retained:u64,staged:u64,work:u64,visits:usize}
impl Requirements{
    fn add(&mut self,retained:u64,staged:u64,work:u64,limits:StrokeLimits)->Result<()>{self.retained=add(self.retained,retained)?;self.staged=add(self.staged,staged)?;self.work=add(self.work,work)?;if add(self.retained,self.staged)?>limits.resident_bytes||self.work>limits.work_units{return Err(bad("ownership/work limit"));}Ok(())}
    fn vector<T>(&mut self,len:usize,capacity:usize,limits:StrokeLimits)->Result<()>{self.add(times(capacity as u64,size_of::<T>() as u64)?,times(len as u64,size_of::<T>() as u64)?,times(len as u64,4)?,limits)}
    fn string(&mut self,text:&String,limits:StrokeLimits)->Result<()>{self.add(text.capacity() as u64,text.len() as u64,times(text.len() as u64,4)?,limits)}
    fn json(&mut self,value:&Value,depth:usize,limits:StrokeLimits,control:&Interrupt<'_>)->Result<()>{
        poll(control)?;self.visits=self.visits.checked_add(1).filter(|n|*n<=1_000_000).ok_or_else(||bad("parameter count limit"))?;if depth>16{return Err(bad("parameter depth limit"));}
        self.add(size_of::<Value>() as u64,size_of::<Value>() as u64,4,limits)?;
        match value{Value::String(s)=>self.string(s,limits)?,Value::Array(a)=>{self.vector::<Value>(a.len(),a.capacity(),limits)?;for value in a{self.json(value,depth+1,limits,control)?;}},Value::Object(o)=>{self.add(times(o.len() as u64,256)?,times(o.len() as u64,256)?,times(o.len() as u64,4)?,limits)?;for(key,value)in o{poll(control)?;self.string(key,limits)?;self.json(value,depth+1,limits,control)?;}},_=>{}}
        Ok(())
    }
    fn surface(&mut self,surface:&Surface,limits:StrokeLimits,control:&Interrupt<'_>)->Result<()>{
        poll(control)?;let count=surface.tile_count() as u64;let maps=times(count,256)?;let descriptors=times(count,size_of::<(photocraft_geom::TileCoord,Arc<photocraft_raster::Tile>)>() as u64)?;
        self.add(add(surface.default_bytes().len() as u64,maps)?,add(add(surface.default_bytes().len() as u64,maps)?,descriptors)?,times(count,4)?,limits)?;
        surface.try_validate_tile_origins_with(control).map_err(|e|if control.cancelled(){EngineError::Cancelled}else{bad(&e.to_string())})?;
        for(_,tile)in surface.tiles(){poll(control)?;self.add(tile.bytes().len() as u64,0,1,limits)?;}Ok(())
    }
    fn symmetry(&mut self,symmetry:Option<&PaintingSymmetry>,limits:StrokeLimits,control:&Interrupt<'_>)->Result<()>{
        if let Some(PaintingSymmetry::Path(axis))=symmetry{if axis.segments.len()>1024{return Err(bad("axis segment limit"));}self.string(&axis.source,limits)?;self.vector::<[[f64;2];2]>(axis.segments.len(),axis.segments.capacity(),limits)?;for segment in &axis.segments{poll(control)?;if !segment.iter().flatten().all(|x|x.is_finite()&&x.abs()<=1_000_000.0){return Err(bad("invalid stored axis"));}}}Ok(())
    }
}
struct Budget<'a,'b>{remaining:u64,control:&'a Interrupt<'b>}
impl Budget<'_,'_>{
    fn charge(&mut self,n:u64)->Result<()>{poll(self.control)?;self.remaining=self.remaining.checked_sub(n).ok_or_else(||bad("allocated capacity limit"))?;Ok(())}
    fn copy<T:Copy>(&mut self,source:&[T])->Result<Vec<T>>{poll(self.control)?;let requested=times(source.len() as u64,size_of::<T>() as u64)?;if requested>self.remaining{return Err(bad("copy ownership limit"));}let mut out=Vec::new();out.try_reserve_exact(source.len()).map_err(|_|bad("copy allocation failed"))?;self.charge(times(out.capacity() as u64,size_of::<T>() as u64)?)?;for chunk in source.chunks(256){poll(self.control)?;out.extend_from_slice(chunk);}Ok(out)}
    fn string(&mut self,source:&str)->Result<String>{poll(self.control)?;if source.len() as u64>self.remaining{return Err(bad("string ownership limit"));}let mut out=String::new();out.try_reserve_exact(source.len()).map_err(|_|bad("string allocation failed"))?;self.charge(out.capacity() as u64)?;for(i,c)in source.chars().enumerate(){if i.is_multiple_of(256){poll(self.control)?;}out.push(c);}Ok(out)}
    fn json(&mut self,source:&Value)->Result<Value>{poll(self.control)?;self.charge(size_of::<Value>() as u64)?;Ok(match source{Value::Null=>Value::Null,Value::Bool(v)=>Value::Bool(*v),Value::Number(n)=>Value::Number(n.clone()),Value::String(s)=>Value::String(self.string(s)?),Value::Array(a)=>{let mut out=Vec::new();let requested=times(a.len() as u64,size_of::<Value>() as u64)?;if requested>self.remaining{return Err(bad("parameter ownership limit"));}out.try_reserve_exact(a.len()).map_err(|_|bad("parameter allocation failed"))?;self.charge(times(out.capacity() as u64,size_of::<Value>() as u64)?)?;for value in a{poll(self.control)?;out.push(self.json(value)?);}Value::Array(out)},Value::Object(o)=>{self.charge(times(o.len() as u64,256)?)?;let mut out=serde_json::Map::new();for(key,value)in o{poll(self.control)?;out.insert(self.string(key)?,self.json(value)?);}Value::Object(out)}})}
    fn surface(&mut self,source:&Surface)->Result<Surface>{
        let count=source.tile_count();let descriptors=times(count as u64,size_of::<(photocraft_geom::TileCoord,Arc<photocraft_raster::Tile>)>() as u64)?;
        if add(descriptors,times(count as u64,256)?)?>self.remaining{return Err(bad("sparse map ownership limit"));}
        let mut tiles=Vec::new();tiles.try_reserve_exact(count).map_err(|_|bad("tile descriptor allocation failed"))?;self.charge(times(tiles.capacity() as u64,size_of::<(photocraft_geom::TileCoord,Arc<photocraft_raster::Tile>)>() as u64)?)?;
        for(coordinate,tile)in source.tiles(){poll(self.control)?;tiles.push((*coordinate,Arc::clone(tile)));}
        self.charge(add(source.default_bytes().len() as u64,times(count as u64,256)?)?)?;
        let mut out=Surface::try_with_default_bytes(source.format(),source.default_bytes()).map_err(|e|bad(&e.to_string()))?;
        // Existing BTreeMap/Arc publication has no fallible standard-library API;
        // every node is admitted before insertion and remains private until success.
        for tile in tiles{poll(self.control)?;out.put_tiles(std::iter::once(tile));}poll(self.control)?;Ok(out)
    }
    fn symmetry(&mut self,source:Option<&PaintingSymmetry>)->Result<Option<PaintingSymmetry>>{Ok(match source{None=>None,Some(PaintingSymmetry::Preset(p))=>Some(PaintingSymmetry::Preset(*p)),Some(PaintingSymmetry::Path(axis))=>Some(PaintingSymmetry::Path(SymmetryAxis{source:self.string(&axis.source)?,segments:self.copy(&axis.segments)?}))})}
}
fn symmetry_equal(a:Option<&PaintingSymmetry>,b:Option<&PaintingSymmetry>,control:&Interrupt<'_>)->Result<bool>{
    poll(control)?;Ok(match(a,b){(None,None)=>true,(Some(PaintingSymmetry::Preset(a)),Some(PaintingSymmetry::Preset(b)))=>a==b,(Some(PaintingSymmetry::Path(a)),Some(PaintingSymmetry::Path(b)))=>{if a.source.len()!=b.source.len()||a.segments.len()!=b.segments.len(){return Ok(false);}for(x,y)in a.source.as_bytes().chunks(1024).zip(b.source.as_bytes().chunks(1024)){poll(control)?;if x!=y{return Ok(false);}}for(x,y)in a.segments.iter().zip(&b.segments){poll(control)?;if x!=y{return Ok(false);}}true},_=>false})
}

/// `render` must be the existing trusted incremental painter, not a foreign
/// callback. It mutates only the private candidate. On refusal/cancellation the
/// original preview, renderer, points, union state and budget remain untouched.
pub(crate) fn push_transaction(
    live:&LiveStroke,points:&[StrokePoint],limits:StrokeLimits,control:&Interrupt<'_>,
    render:impl FnOnce(&mut LiveStroke,&[StrokePoint])->Result<Rect>,
)->Result<(LiveStroke,Rect)>{
    poll(control)?;let standard=StrokeLimits::default();if limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units{return Err(bad("limits must be lower-only"));}
    if live.points.len().checked_add(points.len()).is_none_or(|n|n>100_000){return Err(bad("accumulated point limit"));}
    for point in points{poll(control)?;if !(point.x.abs()<=crate::brush_cmds::MAX_COORD&&point.y.abs()<=crate::brush_cmds::MAX_COORD){return Err(bad("invalid point coordinates"));}}
    let document=live.doc.resident_arc().ok_or_else(||bad("requires a resident document"))?;
    let owner=live.doc.downgrade();let base=live.base;
    let native=photocraft_io::NativeSaveLimits::default().preparation;
    let convert=|error:photocraft_io::IoError|if control.cancelled(){EngineError::Cancelled}else{bad(&error.to_string())};
    let model=photocraft_io::retained_native_model_bytes(document,native,control).map_err(convert)? as u64;
    let payload=photocraft_io::retained_native_payload_bytes(document,native,control).map_err(convert)? as u64;
    let raster=crate::DocumentSnapshot::resident_raster_footprint_of(document,control)?;
    let mut req=Requirements::default();
    req.add(add(payload,add(model,raster.clone_metadata_bytes)?)?,add(times(model,3)?,times(raster.clone_metadata_bytes,3)?)?,1,limits)?;
    req.string(&live.cmd,limits)?;req.json(&live.params,0,limits,control)?;
    req.vector::<StrokePoint>(live.points.len(),live.points.capacity(),limits)?;req.vector::<bool>(live.mirror_distinct.len(),live.mirror_distinct.capacity(),limits)?;
    req.symmetry(live.symmetry.as_ref(),limits,control)?;req.surface(&live.pre,limits,control)?;if let Some(sel)=&live.sel{req.surface(sel,limits,control)?;}if let Some(wide)=&live.wide{req.surface(&wide.color,limits,control)?;}
    let painter=incremental_state_requirements(&live.renderer,&live.mirrors,live.union_live.as_ref().map(|(union,_)|union),limits,control).map_err(paint)?;
    req.add(painter.retained_bytes,painter.staged_bytes,painter.work_units,limits)?;
    // Also reserve the pending point/reflection bookkeeping before any private clone.
    // Its preparer owns the exact axis/nearest-segment requirements and cumulative ledger.
    let pending=super::live_batch_controlled::PreparedLiveBatch::prepare(live,points,&live.point_ledger,
        StrokeLimits{resident_bytes:limits.resident_bytes.checked_sub(add(req.retained,req.staged)?).ok_or_else(||bad("pending point ownership limit"))?,work_units:limits.work_units.checked_sub(req.work).ok_or_else(||bad("pending point work limit"))?},control)?;
    drop(pending);
    poll(control)?;
    // Exact existing Document clone: admitted model/map containers and shared
    // immutable payloads, with boundary polling rather than a fictitious deep clone API.
    let cloned_document=(**document).clone();poll(control)?;
    let doc=crate::DocumentSnapshot::from_resident_with_interrupt(Arc::new(cloned_document),control)?;
    let local_staged=req.staged.checked_sub(painter.staged_bytes).ok_or_else(||bad("staging accounting mismatch"))?;
    let mut budget=Budget{remaining:local_staged,control};
    // Document model/map staging is already owned by the generic clone.
    budget.charge(add(times(model,3)?,times(raster.clone_metadata_bytes,3)?)?)?;
    let cmd=budget.string(&live.cmd)?;let params=budget.json(&live.params)?;let cloned_points=budget.copy(&live.points)?;
    let symmetry=budget.symmetry(live.symmetry.as_ref())?;let distinct=budget.copy(&live.mirror_distinct)?;
    let pre=budget.surface(&live.pre)?;let sel=live.sel.as_ref().map(|surface|budget.surface(surface)).transpose()?;
    let wide=live.wide.as_ref().map(|wide|->Result<_>{Ok(WideStroke{blend:wide.blend,color:budget.surface(&wide.color)?,clip:wide.clip,budget:wide.budget.clone()})}).transpose()?;
    let painter_limits=StrokeLimits{resident_bytes:add(painter.retained_bytes,painter.staged_bytes)?,work_units:painter.work_units};
    let cloned=clone_incremental_state_with_interrupt(&live.renderer,&live.mirrors,live.union_live.as_ref().map(|(union,_)|union),painter_limits,control).map_err(paint)?;
    let union_live=cloned.union.map(|union|(union,live.union_live.as_ref().map_or(0,|(_,count)|*count)));
    let mut candidate=LiveStroke{doc,seed:live.seed,cmd,renderer:cloned.renderer,symmetry,mirrors:cloned.mirrors,mirror_distinct:distinct,union_live,pre,sel,lock:live.lock,layer:live.layer,params,tail:live.tail,wide,cmyk:live.cmyk.as_ref().map(Arc::clone),points:cloned_points,point_ledger:live.point_ledger,base};
    poll(control)?;let damage=render(&mut candidate,points)?;poll(control)?;
    if !owner.is_current(&live.doc)||live.base!=base||candidate.base!=base||candidate.seed!=live.seed||candidate.layer!=live.layer||candidate.doc.id!=live.doc.id
        ||!symmetry_equal(live.symmetry.as_ref(),candidate.symmetry.as_ref(),control)?{return Err(bad("captured live state changed"));}
    // Post-render admission is a final publication guard, not evidence that
    // legacy callback allocations were individually fallible or precharged.
    let candidate_raster=candidate.doc.resident_raster_footprint_with(control)?;
    if candidate_raster.retained_bytes>limits.resident_bytes{return Err(bad("candidate raster ownership limit"));}
    incremental_state_requirements(&candidate.renderer,&candidate.mirrors,candidate.union_live.as_ref().map(|(union,_)|union),limits,control).map_err(paint)?;
    poll(control)?;Ok((candidate,damage))
}

#[cfg(test)]
mod tests{
    use super::*;
    use serde_json::json;
    #[test]
    fn callback_error_and_late_cancel_preserve_complete_original_live_state(){
        let mut session=crate::Session::new();session.execute("file.new",json!({"width":8,"height":8})).unwrap();
        let live=LiveStroke::begin(&session,&json!({"points":[[2,2,1]],"size":2,"seed":3})).unwrap();
        let before=live.doc.clone();let points=live.points.clone();let smoothing=live.renderer.smoothing_string();let work=live.point_ledger.work_units();
        let batch=[StrokePoint::new(3.0,3.0,1.0)];
        assert!(push_transaction(&live,&batch,StrokeLimits::default(),&Interrupt::NONE,|candidate,_|{candidate.points.clear();candidate.renderer.finish();Err(bad("synthetic refusal"))}).is_err());
        assert!(live.doc.shares_state(&before));assert_eq!(live.points,points);assert_eq!(live.renderer.smoothing_string(),smoothing);assert_eq!(live.point_ledger.work_units(),work);
        let stop=std::cell::Cell::new(false);let check=||stop.get();
        let result=push_transaction(&live,&batch,StrokeLimits::default(),&Interrupt::cancel_only(&check),|candidate,_|{candidate.points.clear();stop.set(true);Ok(Rect::EMPTY)});
        assert!(matches!(result,Err(EngineError::Cancelled)));assert!(live.doc.shares_state(&before));assert_eq!(live.points,points);
    }
}
