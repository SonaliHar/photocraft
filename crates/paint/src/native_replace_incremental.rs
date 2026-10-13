//! Sparse native PRE/WORK state. Preview encoding is never authoritative WORK.
//! Only delta dabs execute; speculative tail uses a separate COW state/Once sample.
use super::*;
use crate::{BrushContext,Dab};
const SIDE:i32=32;
const CELLS:usize=32*32;
struct WorkTile { key:(i32,i32), values:Arc<Vec<[f64;4]>> }
pub struct NativeLuminosityState {
    tiles:Vec<WorkTile>, once:Option<[f64;3]>, bounds:Rect, dabs:usize, work:u64,
}
impl NativeLuminosityState {
    pub fn new()->Self {Self{tiles:Vec::new(),once:None,bounds:Rect::EMPTY,dabs:0,work:0}}
    pub fn bounds(&self)->Rect {self.bounds}
    pub fn retained_bytes(&self)->Result<u64> {add(mul(self.tiles.capacity() as u64,size_of::<WorkTile>() as u64)?,mul(self.tiles.len() as u64,(CELLS*32+64) as u64)?)}
    pub fn try_clone_with_interrupt(&self,limits:StrokeLimits,c:&Interrupt<'_>)->Result<Self> {
        poll(c)?;let bytes=add(self.retained_bytes()?,mul(self.tiles.len() as u64,size_of::<WorkTile>() as u64)?)?;
        if bytes>limits.resident_bytes||add(self.work,self.tiles.len() as u64)?>limits.work_units{return Err(StrokeError::Limit);}
        let mut tiles=Vec::new();tiles.try_reserve_exact(self.tiles.len()).map_err(|_|StrokeError::Allocation)?;
        if add(self.retained_bytes()?,mul(tiles.capacity() as u64,size_of::<WorkTile>() as u64)?)?>limits.resident_bytes{return Err(StrokeError::Limit);}
        for t in &self.tiles {poll(c)?;tiles.push(WorkTile{key:t.key,values:Arc::clone(&t.values)});}
        Ok(Self{tiles,once:self.once,bounds:self.bounds,dabs:self.dabs,work:add(self.work,mul(self.tiles.len() as u64,4)?)?})
    }
    fn pixel_mut(&mut self,pre:&Surface,x:i32,y:i32,c:&Interrupt<'_>)->Result<&mut [f64;4]> {
        let key=(x.div_euclid(SIDE),y.div_euclid(SIDE));
        let at=match self.tiles.binary_search_by_key(&key,|t|t.key) {Ok(i)=>i,Err(i)=>{
            if self.tiles.len()==self.tiles.capacity(){return Err(StrokeError::Limit);}
            let mut values=vector(CELLS,[0.0;4],c)?;
            let x0=key.0.checked_mul(SIDE).ok_or(StrokeError::Limit)?;let y0=key.1.checked_mul(SIDE).ok_or(StrokeError::Limit)?;
            for dy in 0..SIDE {poll(c)?;for dx in 0..SIDE {let at=(dy*SIDE+dx) as usize;*values.get_mut(at).ok_or(StrokeError::Limit)?=pixels(pre,x0.checked_add(dx).ok_or(StrokeError::Limit)?,y0.checked_add(dy).ok_or(StrokeError::Limit)?)?;}}
            self.tiles.insert(i,WorkTile{key,values:Arc::new(values)});i
        }};
        let t=self.tiles.get_mut(at).ok_or(StrokeError::Limit)?;
        if Arc::get_mut(&mut t.values).is_none() {
            let mut values=Vec::new();values.try_reserve_exact(CELLS).map_err(|_|StrokeError::Allocation)?;
            if values.capacity()!=CELLS{return Err(StrokeError::Limit);}
            for chunk in t.values.chunks(256){poll(c)?;values.extend_from_slice(chunk);}t.values=Arc::new(values);
        }
        let at=(y.rem_euclid(SIDE)*SIDE+x.rem_euclid(SIDE)) as usize;
        Arc::get_mut(&mut t.values).and_then(|v|v.get_mut(at)).ok_or(StrokeError::Limit)
    }
    /// Transactional delta: caller's state is untouched on any refusal/cancellation.
    pub fn apply_dabs_with_interrupt(&mut self,pre:&Surface,ctx:&BrushContext,dabs:&[Dab],settings:&NativeLuminositySettings,limits:StrokeLimits,control:&Interrupt<'_>)->Result<Rect> {
        poll(control)?;let standard=StrokeLimits::default();
        if limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units||self.dabs.checked_add(dabs.len()).is_none_or(|n|n>100_000){return Err(StrokeError::Limit);}
        if pre.format().mode!=ColorMode::Rgb||!matches!(pre.format().sample,SampleType::U32|SampleType::F64){return Err(StrokeError::InvalidInput("native RGB replacement domain"));}
        for v in settings.foreground.into_iter().chain(settings.background){unit(v)?;}unit(settings.tolerance)?;unit(settings.opacity)?;
        if settings.anti_alias&&settings.limits!=Limits::FindEdges&&settings.tolerance>0.0&&settings.tolerance*0.25==0.0{return Err(StrokeError::InvalidInput("unrepresentable tolerance falloff"));}
        let mut changed=Rect::EMPTY;let mut max_dab=0usize;let mut visits=0u64;let mut tile_upper=0u64;
        for d in dabs {poll(control)?;if !d.center.x.is_finite()||!d.center.y.is_finite()||d.center.x.abs()>1_000_000.0||d.center.y.abs()>1_000_000.0||!d.radius.is_finite()||d.radius<=0.0||!d.alpha.is_finite(){return Err(StrokeError::InvalidInput("replacement dab geometry"));}
            let r=ctx.dab_rect(d,false);let n=count(r)?;changed=changed.union(&r);max_dab=max_dab.max(n);visits=add(visits,n as u64)?;
            if !r.is_empty(){let w=i64::from((r.x1-1).div_euclid(SIDE))-i64::from(r.x0.div_euclid(SIDE))+1;let h=i64::from((r.y1-1).div_euclid(SIDE))-i64::from(r.y0.div_euclid(SIDE))+1;tile_upper=add(tile_upper,mul(w as u64,h as u64)?)?;}
        }
        let prospective=add(self.tiles.len() as u64,tile_upper)?;
        let scratch=add(mul(max_dab as u64,(9+size_of::<usize>()) as u64)?,512)?;
        let held=retained(pre,limits.resident_bytes,control)?;
        let peak=add(held,add(mul(self.retained_bytes()?,2)?,add(mul(prospective,(CELLS*32+128) as u64)?,scratch)?)?)?;
        let work=add(self.work,add(self.tiles.len() as u64,add(mul(visits,2048)?,mul(tile_upper,add(mul(prospective,64)?,CELLS as u64*16)?)?)?)?)?;
        if peak>limits.resident_bytes||work>limits.work_units{return Err(StrokeError::Limit);}
        let mut next=self.try_clone_with_interrupt(limits,control)?;let upper=usize::try_from(prospective).map_err(|_|StrokeError::Limit)?;
        next.tiles.try_reserve_exact(upper.saturating_sub(next.tiles.len())).map_err(|_|StrokeError::Allocation)?;
        if next.tiles.capacity()>upper{return Err(StrokeError::Limit);}
        next.apply_private(pre,ctx,dabs,settings,max_dab,control)?;next.dabs+=dabs.len();next.work=work;next.bounds=next.bounds.union(&changed);poll(control)?;*self=next;Ok(changed)
    }
    fn apply_private(&mut self,pre:&Surface,ctx:&BrushContext,dabs:&[Dab],settings:&NativeLuminositySettings,max_dab:usize,control:&Interrupt<'_>)->Result<()> {
        let mut weights=vector(max_dab,0.0f64,control)?;let mut keep=vector(max_dab,0u8,control)?;let mut stack:Vec<usize>=Vec::new();stack.try_reserve_exact(max_dab).map_err(|_|StrokeError::Allocation)?;
        let mut footprint:Vec<f32>=Vec::new();footprint.try_reserve_exact(64).map_err(|_|StrokeError::Allocation)?;
        if weights.capacity()!=max_dab||keep.capacity()!=max_dab||stack.capacity()!=max_dab||footprint.capacity()>128{return Err(StrokeError::Limit);}
    let soft=settings.anti_alias&&settings.limits!=Limits::FindEdges;
    for dab in dabs{
        poll(control)?;let r=ctx.dab_rect(dab,false);let count=count(r)?;let w=usize::try_from(r.width()).map_err(|_|StrokeError::Limit)?;if w==0{return Err(StrokeError::Limit);}
        let cx=dab.center.x.floor() as i32;let cy=dab.center.y.floor() as i32;
        let sampled=match settings.sampling{Sampling::BackgroundSwatch=>settings.background,Sampling::Continuous=>{let v=pixels(pre,cx,cy)?;[unit(v[0])?,unit(v[1])?,unit(v[2])?]},Sampling::Once=>{if let Some(v)=self.once{v}else{let v=pixels(pre,cx,cy)?;let color=[unit(v[0])?,unit(v[1])?,unit(v[2])?];self.once=Some(color);color}}};
        for chunk in weights.get_mut(..count).ok_or(StrokeError::Limit)?.chunks_mut(4096){poll(control)?;chunk.fill(0.0);}
        for y in r.y0..r.y1{let mut x=r.x0;while x<r.x1{poll(control)?;let end=i64::from(x).checked_add(64).ok_or(StrokeError::Limit)?.min(i64::from(r.x1));let end=i32::try_from(end).map_err(|_|StrokeError::Limit)?;let row=Rect::new(x,y,end,y.checked_add(1).ok_or(StrokeError::Limit)?);
            // Pre-reserved <=64-pixel row spans preserve each footprint equation;
            // no full-dab uninterruptible rasterization or Vec growth.
            ctx.rasterize_into(dab,false,row,&mut footprint,None);
            if footprint.len()!=count_row(row)?||footprint.capacity()>128{return Err(StrokeError::Limit);}
            for (offset,cov) in footprint.iter().copied().enumerate(){let px=x.checked_add(i32::try_from(offset).map_err(|_|StrokeError::Limit)?).ok_or(StrokeError::Limit)?;let cov=f64::from(cov);unit(cov)?;if cov<=0.0{continue;}let pre=pixels(pre,px,y)?;let alpha=unit(pre[3])?;if alpha==0.0{continue;}let color=[unit(pre[0])?,unit(pre[1])?,unit(pre[2])?];let distance=(color[0]-sampled[0]).abs().max((color[1]-sampled[1]).abs()).max((color[2]-sampled[2]).abs());let matched=if distance>settings.tolerance{0.0}else if soft&&settings.tolerance>0.0{((settings.tolerance-distance)/(settings.tolerance*0.25)).clamp(0.0,1.0)}else{1.0};*weights.get_mut(index(r,px,y)?).ok_or(StrokeError::Limit)?=matched;}
            x=end;
        }}
        if settings.limits!=Limits::Discontiguous{
            for chunk in keep.get_mut(..count).ok_or(StrokeError::Limit)?.chunks_mut(4096){poll(control)?;chunk.fill(0);}stack.clear();let start=index(r,cx.clamp(r.x0,r.x1-1),cy.clamp(r.y0,r.y1-1))?;
            if *weights.get(start).ok_or(StrokeError::Limit)?>0.0{*keep.get_mut(start).ok_or(StrokeError::Limit)?=1;stack.push(start);}
            while let Some(i)=stack.pop(){poll(control)?;let x=i%w;let y=i/w;let h=usize::try_from(r.height()).map_err(|_|StrokeError::Limit)?;let mut neighbor=|j:usize|->Result<()>{if *keep.get(j).ok_or(StrokeError::Limit)?==0&&*weights.get(j).ok_or(StrokeError::Limit)?>0.0{*keep.get_mut(j).ok_or(StrokeError::Limit)?=1;if stack.len()>=stack.capacity(){return Err(StrokeError::Limit);}stack.push(j);}Ok(())};if x>0{neighbor(i-1)?;}if x+1<w{neighbor(i+1)?;}if y>0{neighbor(i-w)?;}if y+1<h{neighbor(i+w)?;}}
            for (i,(weight,accepted))in weights.get_mut(..count).ok_or(StrokeError::Limit)?.iter_mut().zip(keep.get(..count).ok_or(StrokeError::Limit)?).enumerate(){if i.is_multiple_of(4096){poll(control)?;}if *accepted==0{*weight=0.0;}}
        }
        // Re-evaluate identical upstream coverage after flood acceptance; WORK
        // persists across every dab, whereas matching always reads immutable PRE.
        for y in r.y0..r.y1{let mut x=r.x0;while x<r.x1{poll(control)?;let end=i32::try_from((i64::from(x)+64).min(i64::from(r.x1))).map_err(|_|StrokeError::Limit)?;let row=Rect::new(x,y,end,y.checked_add(1).ok_or(StrokeError::Limit)?);ctx.rasterize_into(dab,false,row,&mut footprint,None);for(offset,cov)in footprint.iter().copied().enumerate(){let px=x.checked_add(i32::try_from(offset).map_err(|_|StrokeError::Limit)?).ok_or(StrokeError::Limit)?;let k=f64::from(cov)*weights.get(index(r,px,y)?).copied().ok_or(StrokeError::Limit)?;if k<=0.0{continue;}let current=self.pixel_mut(pre,px,y,control)?;let color=[unit(current[0])?,unit(current[1])?,unit(current[2])?];let blend=luminosity(color,settings.foreground)?;for channel in 0..3{let old=current.get_mut(channel).ok_or(StrokeError::Limit)?;*old=unit(*old+(blend.get(channel).copied().ok_or(StrokeError::Limit)?-*old)*k)?;}}x=end;}}
    }
        Ok(())
    }
    /// Rebuild native display from immutable PRE, mixing opacity/selection once.
    /// Alpha/outside/default words remain raw. No preview samples return to WORK.
    pub fn output_work_units(&self,pre:&Surface,region:Rect)->Result<u64> {
        add(mul(count(region)? as u64,2048)?,mul(mul(tile_count(region)?,add(pre.tile_count() as u64,tile_count(region)?)?)?,256)?)
    }
    pub fn work_units(&self)->u64 {self.work}
    pub fn output_with_interrupt(&self,pre:&Surface,selection:Option<&Surface>,lock:bool,settings:&NativeLuminositySettings,limits:StrokeLimits,c:&Interrupt<'_>)->Result<(Surface,Rect)> {
        self.output_region_with_interrupt(pre,pre,self.bounds,selection,lock,settings,limits,c)
    }
    /// Restore only the preceding tail/new delta region against immutable PRE.
    /// Unaffected display tiles remain native copies of the preceding projection.
    #[allow(clippy::too_many_arguments)]
    pub fn output_region_with_interrupt(&self,pre:&Surface,base:&Surface,region:Rect,selection:Option<&Surface>,lock:bool,settings:&NativeLuminositySettings,limits:StrokeLimits,c:&Interrupt<'_>)->Result<(Surface,Rect)> {
        poll(c)?;let standard=StrokeLimits::default();if limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units{return Err(StrokeError::Limit);}
        if pre.format()!=base.format()||selection.is_some_and(|s|s.format().mode!=ColorMode::Grayscale||s.format().alpha){return Err(StrokeError::InvalidInput("native output/Gray coverage domain"));}unit(settings.opacity)?;
        let fmt=pre.format();let native_tile=mul((TILE_SIZE*TILE_SIZE) as u64,fmt.bytes_per_pixel() as u64)?;
        let held=add(retained(pre,limits.resident_bytes,c)?,add(retained(base,limits.resident_bytes,c)?,add(self.retained_bytes()?,selection.map_or(Ok(0),|s|retained(s,limits.resident_bytes,c))?)?)?)?;
        let out=add(mul(add(base.tile_count() as u64,tile_count(region)?)?,add(native_tile,512)?)?,mul(native_tile,4)?)?;
        if add(held,out)?>limits.resident_bytes||add(self.work,self.output_work_units(base,region)?)?>limits.work_units{return Err(StrokeError::Limit);}
        let mut staged=Surface::try_with_default_bytes(fmt,base.default_bytes()).map_err(raster)?;
        for(coord,tile)in base.tiles(){poll(c)?;staged.put_tiles(std::iter::once((*coord,Arc::clone(tile))));}
        let bpp=fmt.bytes_per_pixel();let mut changed=false;
        for tile in region.tiles() {poll(c)?;let r=tile.rect().intersect(&region);
            let mut raw=pre.try_read_interleaved_with(r,c).map_err(raster)?;if add(held,add(out,raw.capacity() as u64)?)?>limits.resident_bytes{return Err(StrokeError::Limit);}
            let width=usize::try_from(r.width()).map_err(|_|StrokeError::Limit)?;if width==0{return Err(StrokeError::Limit);}
            for(i,encoded)in raw.chunks_exact_mut(bpp).enumerate(){if i.is_multiple_of(256){poll(c)?;}let x=r.x0.checked_add((i%width) as i32).ok_or(StrokeError::Limit)?;let y=r.y0.checked_add((i/width) as i32).ok_or(StrokeError::Limit)?;
                let key=(x.div_euclid(SIDE),y.div_euclid(SIDE));let Ok(at)=self.tiles.binary_search_by_key(&key,|t|t.key)else{continue;};
                let original=pixels(pre,x,y)?;let at_pixel=(y.rem_euclid(SIDE)*SIDE+x.rem_euclid(SIDE)) as usize;
                let work=self.tiles.get(at).and_then(|t|t.values.get(at_pixel)).ok_or(StrokeError::Limit)?;
                if (0..3).all(|ch|original.get(ch).zip(work.get(ch)).is_some_and(|(a,b)|a.to_bits()==b.to_bits())){continue;}
                let mut k=settings.opacity;if let Some(mask)=selection{let mut v=[0.0];mask.try_read_pixel_f64(x,y,&mut v).map_err(raster)?;k*=unit(v[0])?;}if k==0.0{continue;}
                let alpha=unit(original[3])?;if alpha==0.0{continue;}
                for ch in 0..3{let d=unit(*original.get(ch).ok_or(StrokeError::Limit)?)?;let s=unit(*work.get(ch).ok_or(StrokeError::Limit)?)?;let value=if fmt.alpha&&!lock&&alpha>1.0e-6{(d*alpha+(s*alpha-d*alpha)*k)/alpha}else{d+(s-d)*k};unit(value)?;try_write_sample_f64(encoded,fmt.sample,ch,value).map_err(|_|StrokeError::InvalidInput("native replacement sample"))?;}
            }
            let old=base.try_read_interleaved_with(r,c).map_err(raster)?;
            if raw!=old{staged.try_write_interleaved_with(r,&raw,c).map_err(raster)?;changed=true;}
        }
        poll(c)?;Ok((staged,if changed{region}else{Rect::EMPTY}))
    }
}
impl Default for NativeLuminosityState {fn default()->Self{Self::new()}}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings,StrokePoint};
    use photocraft_color::PixelFormat;
    fn fixture(sample:SampleType)->Surface {Surface::with_default(PixelFormat::new(ColorMode::Rgb,sample,true),&[0.5,0.5,0.5,0.75])}
    fn painter()->StrokeRenderer {
        let b=BrushSettings{size:4.0,hardness:1.0,pressure_size:false,pressure_opacity:false,flow:0.35,..Default::default()};
        let mut r=StrokeRenderer::try_new_with_interrupt(&b,None,1.0,StrokeLimits::default(),&Interrupt::NONE).unwrap().record_dabs();
        r.push_with_interrupt(&[StrokePoint::new(0.5,0.5,1.0),StrokePoint::new(3.5,0.5,1.0)],StrokeLimits::default(),&Interrupt::NONE).unwrap();r.finish_with_interrupt(StrokeLimits::default(),&Interrupt::NONE).unwrap();r
    }
    fn settings()->NativeLuminositySettings {NativeLuminositySettings{foreground:[0.8;3],background:[0.5;3],tolerance:1.0,sampling:Sampling::Once,limits:Limits::Contiguous,anti_alias:true,opacity:0.6}}
    #[test]
    fn delta_work_and_final_quantization_match_finished_one_shot() {
        let renderer=painter();for sample in [SampleType::U32,SampleType::F64]{let pre=fixture(sample);let mut expected=pre.clone();let cfg=settings();
            try_apply_luminosity(&mut expected,&renderer,&cfg,None,false,StrokeLimits::default(),&Interrupt::NONE).unwrap();
            let mut state=NativeLuminosityState::new();for dab in renderer.dabs(){state.apply_dabs_with_interrupt(&pre,&renderer.ctx,std::slice::from_ref(dab),&cfg,StrokeLimits::default(),&Interrupt::NONE).unwrap();}
            let(actual,_)=state.output_with_interrupt(&pre,None,false,&cfg,StrokeLimits::default(),&Interrupt::NONE).unwrap();
            let r=state.bounds();assert_eq!(actual.try_read_interleaved_with(r,&Interrupt::NONE).unwrap(),expected.try_read_interleaved_with(r,&Interrupt::NONE).unwrap());
        }
    }
    #[test]
    fn speculative_once_tail_and_cancel_do_not_change_permanent_work() {
        let renderer=painter();let pre=fixture(SampleType::F64);let cfg=settings();let mut state=NativeLuminosityState::new();
        let first=renderer.dabs().first().unwrap();state.apply_dabs_with_interrupt(&pre,&renderer.ctx,std::slice::from_ref(first),&cfg,StrokeLimits::default(),&Interrupt::NONE).unwrap();
        let(before,_)=state.output_with_interrupt(&pre,None,false,&cfg,StrokeLimits::default(),&Interrupt::NONE).unwrap();let once=state.once;
        let mut tail=state.try_clone_with_interrupt(StrokeLimits::default(),&Interrupt::NONE).unwrap();tail.apply_dabs_with_interrupt(&pre,&renderer.ctx,renderer.dabs(),&cfg,StrokeLimits::default(),&Interrupt::NONE).unwrap();drop(tail);
        let cancel=||true;assert_eq!(state.apply_dabs_with_interrupt(&pre,&renderer.ctx,renderer.dabs(),&cfg,StrokeLimits::default(),&Interrupt::cancel_only(&cancel)),Err(StrokeError::Cancelled));
        let(after,_)=state.output_with_interrupt(&pre,None,false,&cfg,StrokeLimits::default(),&Interrupt::NONE).unwrap();assert_eq!(before,after);assert_eq!(once,state.once);
    }
}
