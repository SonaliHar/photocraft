//! Aggregate admission for the complete live painter set, before publication.
//! Document model/map clone headroom is reserved here, not cloned by this helper.
//! The model census is a measured public-container slice, not all allocator/RSS
//! storage; legacy parameter/axis copies remain the parent caller's responsibility.
use crate::{DocumentSnapshot,EngineError,Result};
use photocraft_doc::Document;
use photocraft_paint::{BrushSettings,render::StrokeRenderer,stroke_controlled::{StrokeError,StrokeLimits}};
use photocraft_color::PixelFormat;
use photocraft_raster::Interrupt;
#[cfg(test)]
use std::mem::size_of;
fn bad(message:&str)->EngineError{EngineError::Other(format!("live stroke preparation: {message}"))}
fn poll(control:&Interrupt<'_>)->Result<()>{control.check().map_err(|_|EngineError::Cancelled)}
fn add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or_else(||bad("accounting overflow"))}
fn mul(a:u64,b:u64)->Result<u64>{a.checked_mul(b).ok_or_else(||bad("accounting overflow"))}
fn paint(error:StrokeError)->EngineError{match error{StrokeError::Cancelled=>EngineError::Cancelled,error=>bad(&error.to_string())}}
fn io(error:photocraft_io::IoError)->EngineError{match error{photocraft_io::IoError::Cancelled=>EngineError::Cancelled,error=>bad(&error.to_string())}}
/// The caller supplies the actual resolved target format and brush. This grants
/// no source/profile authority and never changes brush colors or layer content.
/// Preset symmetry currently has at most3 mirrors; stored-path symmetry has1.
#[cfg(test)]
pub(super) fn prepare_renderers(
    doc:&Document,brush:&BrushSettings,format:PixelFormat,zoom:f32,mirror_count:usize,control:&Interrupt<'_>,
)->Result<(StrokeRenderer,Vec<StrokeRenderer>,Vec<bool>)>{
    poll(control)?;if mirror_count>3{return Err(bad("mirror count limit"));}
    let standard=StrokeLimits::default();let count=mirror_count.checked_add(1).ok_or_else(||bad("mirror count overflow"))?;
    let per=StrokeRenderer::new_requirements(brush,Some(format),zoom,standard,control).map_err(paint)?;
    // These existing censuses have standard bounded traversal/index limits and
    // reject backed/unavailable native payloads; no ambient resource reads.
    let native=photocraft_io::NativeSaveLimits::default().preparation;
    let model=photocraft_io::retained_native_model_bytes(doc,native,control).map_err(io)? as u64;
    let payload=photocraft_io::retained_native_payload_bytes(doc,native,control).map_err(io)? as u64;
    let raster=DocumentSnapshot::resident_raster_footprint_of(doc,control)?;
    // The native payload census already includes shared raster bytes. Count it
    // once, then the sparse/default clone metadata separately. Also retain the
    // independent raster cap above rather than silently weakening its domain.
    let document_held=add(payload,add(model,raster.clone_metadata_bytes)?)?;
    let document_stage=add(mul(model,3)?,mul(raster.clone_metadata_bytes,3)?)?;
    let painter_held=mul(per.retained_bytes,count as u64)?;
    let painter_stage=mul(per.staged_bytes,count as u64)?;
    let array_stage=add(mul(mirror_count as u64,size_of::<StrokeRenderer>() as u64)?,mul(mirror_count as u64,size_of::<bool>() as u64)?)?;
    let fixed_held=add(document_held,document_stage)?;
    let admitted=add(fixed_held,add(painter_held,add(painter_stage,array_stage)?)?)?;
    // Include complete constructor work, post-construction state census, and
    // conservative public-model/sparse metadata visits before any renderer copy.
    let outer_work=add(mul(model,4)?,mul(raster.clone_metadata_bytes,4)?)?;
    let work=add(outer_work,mul(add(mul(per.work_units,2)?,256)?,count as u64)?)?;
    if admitted>standard.resident_bytes||work>standard.work_units{return Err(bad("aggregate document/painter limit"));}
    poll(control)?;
    let mut mirrors=Vec::new();mirrors.try_reserve_exact(mirror_count).map_err(|_|bad("mirror allocation failed"))?;
    let mut distinct=Vec::new();distinct.try_reserve_exact(mirror_count).map_err(|_|bad("mirror flag allocation failed"))?;
    let actual_arrays=add(mul(mirrors.capacity() as u64,size_of::<StrokeRenderer>() as u64)?,mul(distinct.capacity() as u64,size_of::<bool>() as u64)?)?;
    let fixed_held=add(fixed_held,actual_arrays)?;
    if add(fixed_held,add(painter_held,painter_stage)?)?>standard.resident_bytes{return Err(bad("actual mirror capacity limit"));}
    let per_limits=StrokeLimits{resident_bytes:add(per.retained_bytes,per.staged_bytes)?,work_units:per.work_units};
    let primary=StrokeRenderer::try_new_with_interrupt(brush,Some(format),zoom,per_limits,control).map_err(paint)?;
    let mut actual=0u64;let mut actual_work=outer_work;
    let census=primary.incremental_clone_requirements(standard,control).map_err(paint)?;
    actual=add(actual,census.retained_bytes)?;actual_work=add(actual_work,add(per.work_units,census.work_units)?)?;
    if add(fixed_held,add(actual,mul(add(per.retained_bytes,per.staged_bytes)?,mirror_count as u64)?)?)?>standard.resident_bytes||actual_work>standard.work_units{return Err(bad("primary retained painter limit"));}
    for index in 0..mirror_count{
        poll(control)?;
        let mirror=StrokeRenderer::try_new_with_interrupt(brush,Some(format),zoom,per_limits,control).map_err(paint)?;
        let census=mirror.incremental_clone_requirements(standard,control).map_err(paint)?;
        actual=add(actual,census.retained_bytes)?;actual_work=add(actual_work,add(per.work_units,census.work_units)?)?;
        let left=mirror_count.checked_sub(index+1).ok_or_else(||bad("mirror staging count"))?;
        if add(fixed_held,add(actual,mul(add(per.retained_bytes,per.staged_bytes)?,left as u64)?)?)?>standard.resident_bytes||actual_work>standard.work_units{return Err(bad("aggregate retained painter limit"));}
        mirrors.push(mirror);distinct.push(false);
    }
    poll(control)?;Ok((primary,mirrors,distinct))
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn all_configured_mirrors_preserve_initial_state_and_false_flags(){
        let mut session=crate::Session::new();session.execute("file.new",serde_json::json!({"width":8,"height":8})).unwrap();
        let doc=&session.active().unwrap().doc;let brush=BrushSettings{size:3.0,..Default::default()};
        for count in 0..=3{let(primary,mirrors,flags)=prepare_renderers(doc,&brush,PixelFormat::RGBA8,1.0,count,&Interrupt::NONE).unwrap();assert_eq!(mirrors.len(),count);assert_eq!(flags,vec![false;count]);assert_eq!(primary.dab_count(),0);for mirror in mirrors{assert_eq!(mirror.dab_count(),0);assert_eq!(mirror.smoothing_string(),primary.smoothing_string());}}
        assert!(prepare_renderers(doc,&brush,PixelFormat::RGBA8,1.0,4,&Interrupt::NONE).is_err());
    }
    #[test]
    fn cancellation_never_returns_partial_renderer_set_or_changes_document(){
        let mut session=crate::Session::new();session.execute("file.new",serde_json::json!({"width":8,"height":8})).unwrap();let state=session.active().unwrap();let before=state.doc.clone();
        let brush=BrushSettings{tip:photocraft_paint::brush::TipShape::Sampled(photocraft_paint::tile::GrayTile{width:257,height:129,data:vec![12345;257*129].into()}),..Default::default()};
        let n=std::cell::Cell::new(0);let stop=||{n.set(n.get()+1);n.get()>100;};
        assert!(matches!(prepare_renderers(&state.doc,&brush,PixelFormat::RGBA8,1.0,3,&Interrupt::cancel_only(&stop)),Err(EngineError::Cancelled)));
        assert!(state.doc.shares_state(&before));
    }
}

/// Reserve the complete existing document and planned constructor set, then
/// partition remaining storage among all renderers and final output. Actual
/// generated dabs/coverage remain guarded by each controlled renderer stage.
pub(super) fn one_shot_limits(doc:&Document,brush:&BrushSettings,format:PixelFormat,zoom:f32,mirror_count:usize,control:&Interrupt<'_>)->Result<StrokeLimits>{
    poll(control)?;
    let count=mirror_count.checked_add(1).filter(|count|*count<=4).ok_or_else(||bad("mirror count limit"))?;
    let standard=StrokeLimits::default();
    let set=StrokeRenderer::renderer_set_requirements(brush,Some(format),zoom,count,standard,control).map_err(paint)?;
    let native=photocraft_io::NativeSaveLimits::default().preparation;
    let input=photocraft_io::retained_native_input_bytes(doc,native,control).map_err(io)? as u64;
    let raster=DocumentSnapshot::resident_raster_footprint_of(doc,control)?;
    let fixed=add(input,add(mul(raster.clone_metadata_bytes,3)?,1024)?)?;
    if add(fixed,add(set.retained_bytes,set.staged_bytes)?)?>standard.resident_bytes||set.work_units>standard.work_units{return Err(bad("aggregate one-shot painter limit"));}
    let share=standard.resident_bytes.checked_sub(fixed).ok_or_else(||bad("one-shot storage headroom"))?/((count+1) as u64);
    let constructor=add(set.per_renderer.retained_bytes,set.per_renderer.staged_bytes)?;
    if constructor>share{return Err(bad("one-shot constructor partition"));}
    let work=standard.work_units.checked_sub(set.work_units).ok_or_else(||bad("one-shot construction work"))?/(2*count as u64+1);
    if set.per_renderer.work_units>work{return Err(bad("one-shot constructor work partition"));}
    poll(control)?;
    Ok(StrokeLimits{resident_bytes:share,work_units:work})
}
