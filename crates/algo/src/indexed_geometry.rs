//! Exact geometry for CURRENT editable indexed rasters, never import provenance.
//! Ordinals, duplicate palette entries and optional native U8 alpha remain exact.
//! No RGB expansion, nearest-color search, transfer conversion or source admission.
use photocraft_doc::EditableIndexedRaster;
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use std::{mem::size_of, sync::Arc};

const MAX_PIXELS:u64=64<<20;
const CHUNK:usize=32<<10;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Origin { Keep, Rebase, Translate {dx:i32,dy:i32} }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Config { pub clip:Rect, pub origin:Origin }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Limits { pub resident_bytes:u64, pub work_units:u64 }
impl Default for Limits {
    fn default()->Self { Self {resident_bytes:photocraft_raster::MAX_CHECKED_RASTER_BYTES,work_units:if cfg!(target_arch="wasm32") {1<<36}else {1<<40}} }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Requirements {
    /// All retained source capacities, even when output shares their Arcs.
    pub source_retained_bytes:u64,
    pub palette_retained_bytes:u64,
    /// Conservative private plane bytes; full-extent outputs instead retain source capacity.
    pub output_retained_bytes:u64,
    pub output_index_bytes:u64,
    pub work_units:u64,
    /// Source plus output conservatively count shared planes twice. No input refund.
    pub peak_resident_bytes:u64,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Error { Cancelled, InvalidBounds, InvalidSource, Limit, RaisedLimit, Allocation }
impl std::fmt::Display for Error {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {write!(f,"exact indexed geometry: {self:?}")}
}
impl std::error::Error for Error {}
#[derive(Clone,Debug,PartialEq,Eq)]
pub enum Outcome { Empty, NonEmpty(EditableIndexedRaster) }
fn poll(control:&Interrupt<'_>)->Result<(),Error> {control.check().map_err(|_|Error::Cancelled)}
fn count(bounds:Rect)->Result<u64,Error> {
    let w=i64::from(bounds.x1)-i64::from(bounds.x0);
    let h=i64::from(bounds.y1)-i64::from(bounds.y0);
    if w<=0 || h<=0 || w>262144 || h>262144 {return Err(Error::InvalidBounds);}
    u64::try_from(w).ok().and_then(|w|w.checked_mul(u64::try_from(h).ok()?)).filter(|n|*n<=MAX_PIXELS).ok_or(Error::Limit)
}
fn translated(bounds:Rect,origin:Origin)->Result<Rect,Error> {
    match origin {
        Origin::Keep=>Ok(bounds),
        Origin::Rebase=>Ok(Rect::new(0,0,i32::try_from(i64::from(bounds.x1)-i64::from(bounds.x0)).map_err(|_|Error::InvalidBounds)?,i32::try_from(i64::from(bounds.y1)-i64::from(bounds.y0)).map_err(|_|Error::InvalidBounds)?)),
        Origin::Translate {dx,dy}=>Ok(Rect::new(bounds.x0.checked_add(dx).ok_or(Error::InvalidBounds)?,bounds.y0.checked_add(dy).ok_or(Error::InvalidBounds)?,bounds.x1.checked_add(dx).ok_or(Error::InvalidBounds)?,bounds.y1.checked_add(dy).ok_or(Error::InvalidBounds)?)),
    }
}
fn add(a:u64,b:u64)->Result<u64,Error> {a.checked_add(b).ok_or(Error::Limit)}
fn bytes(value:usize)->Result<u64,Error> {u64::try_from(value).map_err(|_|Error::Limit)}
fn admitted(source:&EditableIndexedRaster,config:&Config,limits:Limits,control:&Interrupt<'_>)->Result<(Rect,Rect,Requirements),Error> {
    poll(control)?;
    let standard=Limits::default();
    if limits.resident_bytes>standard.resident_bytes || limits.work_units>standard.work_units {return Err(Error::RaisedLimit);}
    if config.clip.x1<config.clip.x0 || config.clip.y1<config.clip.y0 {return Err(Error::InvalidBounds);}
    let source_count=count(source.bounds())?;
    if bytes(source.indices().len())?!=source_count || source.alpha().is_some_and(|alpha|alpha.len()!=source.indices().len())
        || source.palette().is_empty() || source.palette().len()>256 {return Err(Error::InvalidSource);}
    let retained_planes=add(bytes(source.indices().capacity())?,source.alpha().map_or(Ok(0),|a|bytes(a.capacity()))?)?;
    let source_retained=add(add(retained_planes,bytes(source.palette_capacity_bytes())?)?,size_of::<EditableIndexedRaster>() as u64+2*(size_of::<Vec<u8>>()+2*size_of::<usize>()) as u64)?;
    let clip=source.bounds().intersect(&config.clip);
    let palette=if clip.is_empty() {0}else {bytes(source.palette().len())?.checked_mul(3).ok_or(Error::Limit)?};
    let output_count=if clip.is_empty() {0}else {count(clip)?};
    let output_planes=if clip.is_empty() {0}else if clip==source.bounds() {retained_planes}else {output_count.checked_mul(if source.alpha().is_some() {2}else {1}).ok_or(Error::Limit)?};
    let output_index=if clip.is_empty() {0}else {size_of::<EditableIndexedRaster>() as u64+2*(size_of::<Vec<u8>>()+2*size_of::<usize>()) as u64};
    let output_retained=add(output_planes,palette)?;
    // One input ordinal validation plus output constructor validation, byte copy
    // and per-row/chunk visits. Palette copy is explicitly included.
    let work=add(add(source_count.checked_mul(3).ok_or(Error::Limit)?,output_count.checked_mul(8).ok_or(Error::Limit)?)?,add(palette,1024)?)?;
    let peak=add(source_retained,add(output_retained,output_index)?)?;
    if peak>limits.resident_bytes || work>limits.work_units {return Err(Error::Limit);}
    let output=if clip.is_empty() {Rect::EMPTY}else {translated(clip,config.origin)?};
    // Expensive validation only follows the metadata/retention/work admission.
    source.validate_with(control).map_err(|_|if control.cancelled() {Error::Cancelled}else {Error::InvalidSource})?;
    poll(control)?;
    Ok((clip,output,Requirements {source_retained_bytes:source_retained,palette_retained_bytes:palette,output_retained_bytes:output_retained,output_index_bytes:output_index,work_units:work,peak_resident_bytes:peak}))
}
/// Same admitted preflight used by apply. Cached/unknown enclosing document data
/// is not charged here; its owner must reserve it separately.
pub fn requirements(source:&EditableIndexedRaster,config:&Config,control:&Interrupt<'_>)->Result<Requirements,Error> {
    admitted(source,config,Limits::default(),control).map(|(_,_,requirements)|requirements)
}
fn copy_plane(plane:&[u8],source:Rect,clip:Rect,capacity_limit:u64,control:&Interrupt<'_>)->Result<Vec<u8>,Error> {
    let length=usize::try_from(count(clip)?).map_err(|_|Error::Limit)?;
    let mut output=Vec::new();poll(control)?;
    output.try_reserve_exact(length).map_err(|_|Error::Allocation)?;poll(control)?;
    if bytes(output.capacity())?>capacity_limit {return Err(Error::Limit);}
    // Reserve/capacity reconciliation precedes any copied sample bytes.
    let width=usize::try_from(i64::from(source.x1)-i64::from(source.x0)).map_err(|_|Error::InvalidBounds)?;
    let span=usize::try_from(i64::from(clip.x1)-i64::from(clip.x0)).map_err(|_|Error::InvalidBounds)?;
    let x=usize::try_from(i64::from(clip.x0)-i64::from(source.x0)).map_err(|_|Error::InvalidBounds)?;
    for y in clip.y0..clip.y1 {
        poll(control)?;
        let row=usize::try_from(i64::from(y)-i64::from(source.y0)).map_err(|_|Error::InvalidBounds)?;
        let start=row.checked_mul(width).and_then(|n|n.checked_add(x)).ok_or(Error::Limit)?;
        let end=start.checked_add(span).ok_or(Error::Limit)?;
        let input=plane.get(start..end).ok_or(Error::InvalidSource)?;
        for chunk in input.chunks(CHUNK) {poll(control)?;output.extend_from_slice(chunk);poll(control)?;}
    }
    if output.len()!=length {return Err(Error::InvalidSource);}poll(control)?;Ok(output)
}
/// Crops in source coordinates and then applies an explicit output-origin policy.
/// Empty intersections publish no ordinal carrier. Failure drops all private
/// arrays and leaves source planes/palette untouched.
pub fn try_crop(source:&EditableIndexedRaster,config:&Config,limits:Limits,control:&Interrupt<'_>)->Result<Outcome,Error> {
    let (clip,output,required)=admitted(source,config,limits,control)?;
    if clip.is_empty() {poll(control)?;return Ok(Outcome::Empty);}
    let mut palette=Vec::new();poll(control)?;
    palette.try_reserve_exact(source.palette().len()).map_err(|_|Error::Allocation)?;
    let actual_palette=bytes(palette.capacity())?.checked_mul(3).ok_or(Error::Limit)?;
    let full=clip==source.bounds();
    // Allocation accounting remains conservative even for shared input Arcs.
    let nominal_planes=required.output_retained_bytes.checked_sub(required.palette_retained_bytes).ok_or(Error::Limit)?;
    if add(required.source_retained_bytes,add(add(nominal_planes,actual_palette)?,required.output_index_bytes)?)?>limits.resident_bytes {return Err(Error::Limit);}
    palette.extend_from_slice(source.palette());poll(control)?;
    let plane_limit=limits.resident_bytes.checked_sub(add(required.source_retained_bytes,add(actual_palette,required.output_index_bytes)?)?).ok_or(Error::Limit)?;
    let indices=if full {source.indices().clone()}else {Arc::new(copy_plane(source.indices().as_slice(),source.bounds(),clip,plane_limit,control)?)};
    let remaining=plane_limit.checked_sub(bytes(indices.capacity())?).ok_or(Error::Limit)?;
    let alpha=match source.alpha() {Some(a)=>Some(if full {a.clone()}else {Arc::new(copy_plane(a.as_slice(),source.bounds(),clip,remaining,control)?)}),None=>None};
    let actual_planes=add(bytes(indices.capacity())?,alpha.as_ref().map_or(Ok(0),|a|bytes(a.capacity()))?)?;
    if add(required.source_retained_bytes,add(add(actual_planes,actual_palette)?,required.output_index_bytes)?)?>limits.resident_bytes {return Err(Error::Limit);}
    poll(control)?;
    let raster=EditableIndexedRaster::try_new_with_interrupt(output,indices,alpha,palette,control).map_err(|_|if control.cancelled() {Error::Cancelled}else {Error::InvalidSource})?;
    poll(control)?;Ok(Outcome::NonEmpty(raster))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(alpha:bool)->EditableIndexedRaster {
        EditableIndexedRaster::try_new(Rect::new(-2,-1,1,1),Arc::new(vec![0,1,2,2,1,0]),alpha.then(||Arc::new(vec![0,1,255,128,2,254])),vec![[7,8,9],[7,8,9],[99,88,77]]).unwrap()
    }
    #[test]
    fn crop_preserves_duplicate_ordinals_native_alpha_and_named_origin() {
        for has_alpha in [false,true] {
            let source=source(has_alpha);let before=source.clone();
            let config=Config {clip:Rect::new(-1,-1,1,1),origin:Origin::Rebase};
            let Outcome::NonEmpty(output)=try_crop(&source,&config,Limits::default(),&Interrupt::NONE).unwrap() else {panic!("nonempty expected")};
            assert_eq!(output.bounds(),Rect::new(0,0,2,2));assert_eq!(output.indices().as_slice(),&[1,2,1,0]);
            assert_eq!(output.palette(),source.palette());
            assert_eq!(output.alpha().map(|a|a.as_slice()),if has_alpha {Some(&[1,255,2,254][..])}else {None});
            assert_eq!(source,before);
        }
    }
    #[test]
    fn full_translation_shares_planes_empty_does_not_fabricate_ordinal() {
        let source=source(true);let config=Config {clip:source.bounds(),origin:Origin::Translate {dx:5,dy:-7}};
        let Outcome::NonEmpty(output)=try_crop(&source,&config,Limits::default(),&Interrupt::NONE).unwrap() else {panic!("nonempty expected")};
        assert_eq!(output.bounds(),Rect::new(3,-8,6,-6));assert!(Arc::ptr_eq(output.indices(),source.indices()));assert!(Arc::ptr_eq(output.alpha().unwrap(),source.alpha().unwrap()));
        assert_eq!(try_crop(&source,&Config {clip:Rect::new(5,5,6,6),origin:Origin::Keep},Limits::default(),&Interrupt::NONE).unwrap(),Outcome::Empty);
        assert!(try_crop(&source,&Config {clip:source.bounds(),origin:Origin::Translate {dx:i32::MAX,dy:0}},Limits::default(),&Interrupt::NONE).is_err());
    }
    #[test]
    fn lower_budget_and_row_copy_cancellation_publish_nothing() {
        use std::sync::atomic::{AtomicUsize,Ordering};
        let source=source(true);let before=source.clone();let config=Config {clip:Rect::new(-1,-1,1,1),origin:Origin::Keep};
        assert!(try_crop(&source,&config,Limits {resident_bytes:0,..Default::default()},&Interrupt::NONE).is_err());
        assert!(try_crop(&source,&config,Limits {work_units:0,..Default::default()},&Interrupt::NONE).is_err());
        let polls=AtomicUsize::new(0);let cancel=||polls.fetch_add(1,Ordering::Relaxed)>=12;
        assert_eq!(try_crop(&source,&config,Limits::default(),&Interrupt::cancel_only(&cancel)),Err(Error::Cancelled));assert_eq!(source,before);
    }
}
