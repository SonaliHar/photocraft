//! Allocation-free admission for a planned set of identical stroke renderers.
//! Immutable raw sample owners are counted once; prepared mip/texture output is
//! paid separately for every constructor. This is not future stroke/RSS budget.
use super::*;
use crate::tile::{GrayTile,StoredTile};
use photocraft_raster::Interrupt;
use std::mem::size_of;
type Result<T>=std::result::Result<T,StrokeError>;
#[derive(Clone,Copy,Debug)]
pub struct RendererSetRequirements{
    pub renderer_count:usize,
    /// Caller brush/keys and shared immutable raw sample owners, once.
    pub retained_bytes:u64,
    /// Every constructor's owned/prepared storage and planned control arrays.
    pub staged_bytes:u64,
    pub work_units:u64,
    pub shared_sample_bytes:u64,
    /// Descriptor/false-flag/borrowed-reference array admission included above.
    pub array_bytes:u64,
    /// Conservative prepared Arc owner/object allowance included in staged_bytes.
    pub prepared_owner_bytes:u64,
    /// Unchanged independent conservative single-constructor guard/quota.
    pub per_renderer:IncrementalCloneRequirements,
}
fn poll(control:&Interrupt<'_>)->Result<()>{control.check().map_err(|_|StrokeError::Cancelled)}
fn add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or(StrokeError::Limit)}
fn mul(a:u64,b:u64)->Result<u64>{a.checked_mul(b).ok_or(StrokeError::Limit)}
struct RawOwners<'a>{seen:[Option<&'a Arc<[u16]>>;6],count:usize,samples:u64,other:u64}
impl<'a> RawOwners<'a>{
    fn sample(&mut self,tile:&'a GrayTile,control:&Interrupt<'_>)->Result<()>{
        poll(control)?;
        for prior in self.seen.iter().take(self.count).flatten(){poll(control)?;if Arc::ptr_eq(prior,&tile.data){return Ok(());}}
        if self.count>=self.seen.len(){return Err(StrokeError::Limit);}
        *self.seen.get_mut(self.count).ok_or(StrokeError::Limit)?=Some(&tile.data);self.count=self.count.checked_add(1).ok_or(StrokeError::Limit)?;
        // Exact slice length plus a conservative Arc owner/header allowance.
        self.samples=add(self.samples,add(mul(tile.data.len() as u64,size_of::<u16>() as u64)?,32)?)?;Ok(())
    }
    fn stored(&mut self,tile:&'a StoredTile,control:&Interrupt<'_>)->Result<()>{
        poll(control)?;self.other=add(self.other,tile.key.capacity() as u64)?;
        // Outer GrayTile Arc objects are conservative per stored appearance;
        // their immutable sample payloads still deduplicate by actual authority.
        self.other=add(self.other,add(size_of::<GrayTile>() as u64,32)?)?;self.sample(tile.preview.as_ref(),control)?;
        if let Some(full)=&tile.full{self.other=add(self.other,add(size_of::<GrayTile>() as u64,32)?)?;self.sample(full.as_ref(),control)?;}Ok(())
    }
    fn tip(&mut self,tip:&'a TipShape,control:&Interrupt<'_>)->Result<()>{match tip{TipShape::Round=>poll(control),TipShape::Sampled(tile)=>self.sample(tile,control),TipShape::Stored(tile)=>self.stored(tile,control)}}
}
impl StrokeRenderer{
    pub fn renderer_set_requirements(brush:&BrushSettings,format:Option<PixelFormat>,zoom:f32,renderer_count:usize,limits:StrokeLimits,control:&Interrupt<'_>)->Result<RendererSetRequirements>{
        poll(control)?;let standard=StrokeLimits::default();
        if renderer_count==0||renderer_count>4||limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units{return Err(StrokeError::Limit);}
        // Do not weaken the old constructor's own source/preparation guard.
        let per_renderer=Self::new_requirements(brush,format,zoom,limits,control)?;
        let mut raw=RawOwners{seen:[None;6],count:0,samples:0,other:size_of::<BrushSettings>() as u64};
        raw.tip(&brush.tip,control)?;raw.tip(&brush.dual_brush.tip,control)?;
        match &brush.texture.pattern{Pattern::Procedural{..}=>{},Pattern::Tile(tile)=>raw.sample(tile,control)?,Pattern::Stored(tile)=>raw.stored(tile,control)?,}
        let retained_bytes=add(raw.samples,raw.other)?;
        let mirrors=renderer_count.checked_sub(1).ok_or(StrokeError::Limit)?;
        let array_bytes=mul(mirrors as u64,add(add(size_of::<StrokeRenderer>() as u64,size_of::<bool>() as u64)?,size_of::<&StrokeRenderer>() as u64)?)?;
        let valid=|tile:&GrayTile|tile.width>0&&tile.height>0&&(tile.width as usize).checked_mul(tile.height as usize)==Some(tile.data.len());
        let tips=usize::from(brush.tip.bitmap().is_some_and(valid))+usize::from(brush.dual_brush.enabled&&brush.dual_brush.tip.bitmap().is_some_and(valid));
        let pattern=brush.texture.enabled&&match &brush.texture.pattern{Pattern::Procedural{..}=>true,pattern=>pattern.bitmap().is_some_and(valid)};
        let prepared_owner_bytes=mul(add(mul(tips as u64,add(size_of::<Mips>() as u64,32)?)?,if pattern{add(size_of::<PatternImage>() as u64,32)?}else{0})?,renderer_count as u64)?;
        let staged_bytes=add(add(mul(per_renderer.staged_bytes,renderer_count as u64)?,array_bytes)?,prepared_owner_bytes)?;
        // Include independent constructor and post-construction retained census
        // plus fixed bounded authority/key metadata comparison work.
        let work_units=add(mul(add(mul(per_renderer.work_units,2)?,256)?,renderer_count as u64)?,256)?;
        if add(retained_bytes,staged_bytes)?>limits.resident_bytes||work_units>limits.work_units{return Err(StrokeError::Limit);}
        poll(control)?;Ok(RendererSetRequirements{renderer_count,retained_bytes,staged_bytes,work_units,shared_sample_bytes:raw.samples,array_bytes,prepared_owner_bytes,per_renderer})
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn aliases_count_once_but_each_prepared_renderer_is_paid(){
        let data:Arc<[u16]>=vec![12345;35].into();let tile=GrayTile{width:5,height:7,data:data.clone()};
        let mut brush=BrushSettings{tip:TipShape::Sampled(tile.clone()),..Default::default()};brush.dual_brush.enabled=true;brush.dual_brush.tip=TipShape::Sampled(tile.clone());brush.texture.enabled=true;brush.texture.pattern=Pattern::Tile(tile);
        let one=StrokeRenderer::renderer_set_requirements(&brush,Some(PixelFormat::RGBA32F),1.0,1,StrokeLimits::default(),&Interrupt::NONE).unwrap();
        let four=StrokeRenderer::renderer_set_requirements(&brush,Some(PixelFormat::RGBA32F),1.0,4,StrokeLimits::default(),&Interrupt::NONE).unwrap();
        assert_eq!(one.shared_sample_bytes,35*2+32);assert_eq!(one.retained_bytes,four.retained_bytes);assert_eq!(four.staged_bytes,one.staged_bytes*4+four.array_bytes);
        assert!(Arc::ptr_eq(&data,match &brush.tip{TipShape::Sampled(t)=>&t.data,_=>panic!("sampled")}));
    }
    #[test]
    fn aggregate_limit_and_cancel_refuse_before_any_renderer_allocation(){
        let brush=BrushSettings::default();let full=StrokeRenderer::renderer_set_requirements(&brush,Some(PixelFormat::RGBA32F),1.0,4,StrokeLimits::default(),&Interrupt::NONE).unwrap();
        let small=StrokeLimits{resident_bytes:full.retained_bytes+full.staged_bytes-1,..Default::default()};
        assert!(matches!(StrokeRenderer::renderer_set_requirements(&brush,Some(PixelFormat::RGBA32F),1.0,4,small,&Interrupt::NONE),Err(StrokeError::Limit)));
        let stop=||true;assert!(matches!(StrokeRenderer::renderer_set_requirements(&brush,None,1.0,1,StrokeLimits::default(),&Interrupt::cancel_only(&stop)),Err(StrokeError::Cancelled)));
        assert!(StrokeRenderer::renderer_set_requirements(&brush,None,1.0,5,StrokeLimits::default(),&Interrupt::NONE).is_err());
    }
}
