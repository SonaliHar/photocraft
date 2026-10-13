//! Sequential native RGB unit Luminosity replacement, not imported-operation admission.
//! Caller supplies explicit current Linear/profile-native swatches. Geometry and
//! dab footprints retain upstream f32 equations; authoritative pixel WORK is f64.
use crate::{StrokeError,StrokeLimits,StrokeRenderer};
use crate::replace::{Sampling,Limits};
use photocraft_color::{ColorMode,SampleType,try_write_sample_f64};
use photocraft_geom::{Rect,TILE_SIZE};
use photocraft_raster::{Interrupt,Surface,RasterInputError};
use std::{mem::size_of,sync::Arc};
type Result<T>=std::result::Result<T,StrokeError>;
#[derive(Clone,Copy,Debug)]
pub struct NativeLuminositySettings {
    pub foreground:[f64;3],pub background:[f64;3],pub tolerance:f64,
    pub sampling:Sampling,pub limits:Limits,pub anti_alias:bool,pub opacity:f64,
}
fn raster(error:RasterInputError)->StrokeError {match error {RasterInputError::Cancelled=>StrokeError::Cancelled,RasterInputError::AllocationFailed=>StrokeError::Allocation,RasterInputError::LimitExceeded=>StrokeError::Limit,_=>StrokeError::Raster}}
fn poll(c:&Interrupt<'_>)->Result<()> {c.check().map_err(|_|StrokeError::Cancelled)}
fn add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or(StrokeError::Limit)}
fn mul(a:u64,b:u64)->Result<u64>{a.checked_mul(b).ok_or(StrokeError::Limit)}
fn unit(v:f64)->Result<f64>{if v.is_finite()&&(0.0..=1.0).contains(&v){Ok(v)}else{Err(StrokeError::InvalidInput("native replacement requires finite unit samples"))}}
fn count(r:Rect)->Result<usize>{usize::try_from(r.width()).ok().and_then(|w|usize::try_from(r.height()).ok().and_then(|h|w.checked_mul(h))).ok_or(StrokeError::Limit)}
fn index(r:Rect,x:i32,y:i32)->Result<usize>{if !r.contains(x,y){return Err(StrokeError::InvalidInput("replacement coordinate"));}let w=usize::try_from(r.width()).map_err(|_|StrokeError::Limit)?;usize::try_from(i64::from(y)-i64::from(r.y0)).ok().and_then(|row|row.checked_mul(w)).and_then(|row|usize::try_from(i64::from(x)-i64::from(r.x0)).ok().and_then(|x|row.checked_add(x))).ok_or(StrokeError::Limit)}
fn vector<T:Clone>(len:usize,value:T,c:&Interrupt<'_>)->Result<Vec<T>>{poll(c)?;let mut out=Vec::new();out.try_reserve_exact(len).map_err(|_|StrokeError::Allocation)?;if out.capacity()!=len{return Err(StrokeError::Limit);}for i in 0..len{if i.is_multiple_of(1024){poll(c)?;}out.push(value.clone());}Ok(out)}
fn pixels(surface:&Surface,x:i32,y:i32)->Result<[f64;4]>{let mut out=[0.0;4];let n=surface.channels();surface.try_read_pixel_f64(x,y,out.get_mut(..n).ok_or(StrokeError::InvalidInput("replacement format"))?).map_err(raster)?;if !surface.format().alpha{out[3]=1.0;}Ok(out)}
fn retained(s:&Surface,limit:u64,c:&Interrupt<'_>)->Result<u64>{poll(c)?;let tile=(TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n|n.checked_mul(s.format().bytes_per_pixel())).ok_or(StrokeError::Limit)?;if add(s.default_bytes().len() as u64,mul(s.tile_count() as u64,add(tile as u64,256)?)?)?>limit{return Err(StrokeError::Limit);}s.try_validate_tile_origins_with(c).map_err(raster)?;if s.default_bytes().len()!=s.format().bytes_per_pixel(){return Err(StrokeError::InvalidInput("replacement default length"));}let mut n=add(s.default_bytes().len() as u64,mul(s.tile_count() as u64,256)?)?;for(_,t)in s.tiles(){poll(c)?;if t.bytes().len()!=tile{return Err(StrokeError::InvalidInput("replacement tile length"));}n=add(n,tile as u64)?;}Ok(n)}
fn tile_count(r:Rect)->Result<u64>{if r.is_empty(){return Ok(0);}let x1=r.x1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(TILE_SIZE);let y1=r.y1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(TILE_SIZE);let w=u64::try_from(i64::from(x1)-i64::from(r.x0.div_euclid(TILE_SIZE))+1).map_err(|_|StrokeError::Limit)?;let h=u64::try_from(i64::from(y1)-i64::from(r.y0.div_euclid(TILE_SIZE))+1).map_err(|_|StrokeError::Limit)?;mul(w,h)}
fn lum(c:[f64;3])->f64{0.3*c[0]+0.59*c[1]+0.11*c[2]}
// Same SetLum/ClipColor term order and original pre-clip extrema as blend.rs,
// lifted into unfused binary64. This is native-unit math, not ICC luminance.
fn luminosity(c:[f64;3],foreground:[f64;3])->Result<[f64;3]>{
    let delta=lum(foreground)-lum(c);let mut out=c.map(|v|v+delta);let l=lum(out);let min=out[0].min(out[1]).min(out[2]);let max=out[0].max(out[1]).max(out[2]);
    if min<0.0{let d=l-min;for v in &mut out{*v=if l>0.0&&d>0.0{l+(*v-l)*l/d}else{l};}}
    if max>1.0{let d=max-l;for v in &mut out{*v=if l<1.0&&d>0.0{l+(*v-l)*(1.0-l)/d}else{l};}}
    for v in out{unit(v)?;}Ok(out)
}
/// Finished renderer must have record_dabs enabled. No f32 Region, source-over,
/// max-union, or per-dab U32 quantization substitutes for the replacement work.
#[allow(clippy::too_many_arguments)]
pub fn try_apply_luminosity(target:&mut Surface,renderer:&StrokeRenderer,settings:&NativeLuminositySettings,selection:Option<&Surface>,lock:bool,limits:StrokeLimits,control:&Interrupt<'_>)->Result<Rect>{
    poll(control)?;let standard=StrokeLimits::default();let fmt=target.format();
    if limits.resident_bytes>standard.resident_bytes||limits.work_units>standard.work_units{return Err(StrokeError::Limit);}
    if fmt.mode!=ColorMode::Rgb||!matches!(fmt.sample,SampleType::U32|SampleType::F64)||selection.is_some_and(|s|s.format().mode!=ColorMode::Grayscale||s.format().alpha){return Err(StrokeError::InvalidInput("native replacement RGB/Gray coverage domain"));}
    for v in settings.foreground.into_iter().chain(settings.background){unit(v)?;}unit(settings.tolerance)?;unit(settings.opacity)?;
    if settings.anti_alias&&settings.limits!=Limits::FindEdges&&settings.tolerance>0.0&&settings.tolerance*0.25==0.0{return Err(StrokeError::InvalidInput("unrepresentable replacement tolerance falloff"));}
    let dabs=renderer.dabs();if dabs.len()>100_000||dabs.len()!=renderer.dab_count(){return Err(StrokeError::InvalidInput("replacement requires bounded recorded dabs"));}
    let mut bounds=Rect::EMPTY;let mut max_dab=0usize;let mut visits=0u64;
    for dab in dabs{poll(control)?;if !dab.center.x.is_finite()||!dab.center.y.is_finite()||dab.center.x.abs()>1_000_000.0||dab.center.y.abs()>1_000_000.0||!dab.radius.is_finite()||dab.radius<=0.0||!dab.alpha.is_finite(){return Err(StrokeError::InvalidInput("replacement dab geometry"));}let r=renderer.ctx.dab_rect(dab,false);let n=count(r)?;bounds=bounds.union(&r);max_dab=max_dab.max(n);visits=add(visits,n as u64)?;}
    let n=count(bounds)?;if n==0||settings.opacity==0.0{return Ok(Rect::EMPTY);}
    let tiles=tile_count(bounds)?;let native_tile=mul((TILE_SIZE*TILE_SIZE) as u64,fmt.bytes_per_pixel() as u64)?;
    let constructor=renderer.incremental_clone_requirements(limits,control)?;
    let held=add(add(retained(target,limits.resident_bytes,control)?,selection.map_or(Ok(0),|s|retained(s,limits.resident_bytes,control))?)?,constructor.retained_bytes)?;
    let scratch=add(mul(n as u64,size_of::<[f64;4]>() as u64)?,add(mul(max_dab as u64,(8+1+size_of::<usize>()) as u64)?,512)?)?;
    let output=add(mul(tiles,add(native_tile,512)?)?,add(mul(target.tile_count() as u64,512)?,mul(native_tile,4)?)?)?;
    let work=add(constructor.work_units,add(mul(visits,1024)?,add(mul(n as u64,1024)?,mul(mul(tiles,add(target.tile_count() as u64,tiles)?)?,256)?)?)?)?;
    if add(held,add(scratch,output)?)?>limits.resident_bytes||work>limits.work_units{return Err(StrokeError::Limit);}
    let mut values=vector(n,[0.0f64;4],control)?;let mut weights=vector(max_dab,0.0f64,control)?;let mut keep=vector(max_dab,0u8,control)?;let mut stack:Vec<usize>=Vec::new();stack.try_reserve_exact(max_dab).map_err(|_|StrokeError::Allocation)?;let mut footprint:Vec<f32>=Vec::new();footprint.try_reserve_exact(64).map_err(|_|StrokeError::Allocation)?;
    let actual=add(mul(values.capacity() as u64,32)?,add(mul(weights.capacity() as u64,8)?,add(keep.capacity() as u64,add(mul(stack.capacity() as u64,size_of::<usize>() as u64)?,mul(footprint.capacity() as u64,4)?)?)?)?)?;
    if add(held,add(actual,output)?)?>limits.resident_bytes{return Err(StrokeError::Limit);}
    for y in bounds.y0..bounds.y1{poll(control)?;for x in bounds.x0..bounds.x1{if x.rem_euclid(64)==0{poll(control)?;}*values.get_mut(index(bounds,x,y)?).ok_or(StrokeError::Limit)?=pixels(target,x,y)?;}}
    let soft=settings.anti_alias&&settings.limits!=Limits::FindEdges;let mut once=None;
    for dab in dabs{
        poll(control)?;let r=renderer.ctx.dab_rect(dab,false);let count=count(r)?;let w=usize::try_from(r.width()).map_err(|_|StrokeError::Limit)?;if w==0{return Err(StrokeError::Limit);}
        let cx=dab.center.x.floor() as i32;let cy=dab.center.y.floor() as i32;
        let sampled=match settings.sampling{Sampling::BackgroundSwatch=>settings.background,Sampling::Continuous=>{let v=pixels(target,cx,cy)?;[unit(v[0])?,unit(v[1])?,unit(v[2])?]},Sampling::Once=>{if let Some(v)=once{v}else{let v=pixels(target,cx,cy)?;let color=[unit(v[0])?,unit(v[1])?,unit(v[2])?];once=Some(color);color}}};
        for chunk in weights.get_mut(..count).ok_or(StrokeError::Limit)?.chunks_mut(4096){poll(control)?;chunk.fill(0.0);}
        for y in r.y0..r.y1{let mut x=r.x0;while x<r.x1{poll(control)?;let end=i64::from(x).checked_add(64).ok_or(StrokeError::Limit)?.min(i64::from(r.x1));let end=i32::try_from(end).map_err(|_|StrokeError::Limit)?;let row=Rect::new(x,y,end,y.checked_add(1).ok_or(StrokeError::Limit)?);
            // Pre-reserved <=64-pixel row spans preserve each footprint equation;
            // no full-dab uninterruptible rasterization or Vec growth.
            renderer.ctx.rasterize_into(dab,false,row,&mut footprint,None);
            if footprint.len()!=count_row(row)?||footprint.capacity()>128{return Err(StrokeError::Limit);}
            for (offset,cov) in footprint.iter().copied().enumerate(){let px=x.checked_add(i32::try_from(offset).map_err(|_|StrokeError::Limit)?).ok_or(StrokeError::Limit)?;let cov=f64::from(cov);unit(cov)?;if cov<=0.0{continue;}let pre=pixels(target,px,y)?;let alpha=unit(pre[3])?;if alpha==0.0{continue;}let color=[unit(pre[0])?,unit(pre[1])?,unit(pre[2])?];let distance=(color[0]-sampled[0]).abs().max((color[1]-sampled[1]).abs()).max((color[2]-sampled[2]).abs());let matched=if distance>settings.tolerance{0.0}else if soft&&settings.tolerance>0.0{((settings.tolerance-distance)/(settings.tolerance*0.25)).clamp(0.0,1.0)}else{1.0};*weights.get_mut(index(r,px,y)?).ok_or(StrokeError::Limit)?=matched;}
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
        for y in r.y0..r.y1{let mut x=r.x0;while x<r.x1{poll(control)?;let end=i32::try_from((i64::from(x)+64).min(i64::from(r.x1))).map_err(|_|StrokeError::Limit)?;let row=Rect::new(x,y,end,y.checked_add(1).ok_or(StrokeError::Limit)?);renderer.ctx.rasterize_into(dab,false,row,&mut footprint,None);for(offset,cov)in footprint.iter().copied().enumerate(){let px=x.checked_add(i32::try_from(offset).map_err(|_|StrokeError::Limit)?).ok_or(StrokeError::Limit)?;let k=f64::from(cov)*weights.get(index(r,px,y)?).copied().ok_or(StrokeError::Limit)?;if k<=0.0{continue;}let current=values.get_mut(index(bounds,px,y)?).ok_or(StrokeError::Limit)?;let color=[unit(current[0])?,unit(current[1])?,unit(current[2])?];let blend=luminosity(color,settings.foreground)?;for channel in 0..3{let old=current.get_mut(channel).ok_or(StrokeError::Limit)?;*old=unit(*old+(blend.get(channel).copied().ok_or(StrokeError::Limit)?-*old)*k)?;}}x=end;}}
    }
    let mut staged=Surface::try_with_default_bytes(fmt,target.default_bytes()).map_err(raster)?;for(coord,tile)in target.tiles(){poll(control)?;staged.put_tiles(std::iter::once((*coord,Arc::clone(tile))));}
    let bpp=fmt.bytes_per_pixel();let mut changed_any=false;
    for tile in bounds.tiles(){poll(control)?;let r=tile.rect().intersect(&bounds);let mut raw=target.try_read_interleaved_with(r,control).map_err(raster)?;let w=usize::try_from(r.width()).map_err(|_|StrokeError::Limit)?;if w==0{return Err(StrokeError::Limit);}
        if add(held,add(actual,add(output,raw.capacity() as u64)?)?)?>limits.resident_bytes{return Err(StrokeError::Limit);}let mut changed=false;
        for(i,encoded)in raw.chunks_exact_mut(bpp).enumerate(){if i.is_multiple_of(256){poll(control)?;}let x=r.x0.checked_add(i32::try_from(i%w).map_err(|_|StrokeError::Limit)?).ok_or(StrokeError::Limit)?;let y=r.y0.checked_add(i32::try_from(i/w).map_err(|_|StrokeError::Limit)?).ok_or(StrokeError::Limit)?;let mut k=settings.opacity;if let Some(mask)=selection{let mut v=[0.0];mask.try_read_pixel_f64(x,y,&mut v).map_err(raster)?;k*=unit(v[0])?;}if k==0.0{continue;}let pre=pixels(target,x,y)?;let work=*values.get(index(bounds,x,y)?).ok_or(StrokeError::Limit)?;
            if (0..3).all(|ch|pre.get(ch).zip(work.get(ch)).is_some_and(|(a,b)|a.to_bits()==b.to_bits())){continue;}let alpha=unit(pre[3])?;if alpha==0.0{continue;}let mut original=[0u8;32];original.get_mut(..bpp).ok_or(StrokeError::Limit)?.copy_from_slice(encoded);
            for channel in 0..3{let d=unit(pre.get(channel).copied().ok_or(StrokeError::Limit)?)?;let s=unit(work.get(channel).copied().ok_or(StrokeError::Limit)?)?;let value=if fmt.alpha&&!lock&&alpha>1.0e-6{(d*alpha+(s*alpha-d*alpha)*k)/alpha}else{d+(s-d)*k};unit(value)?;try_write_sample_f64(encoded,fmt.sample,channel,value).map_err(|_|StrokeError::InvalidInput("replacement output sample"))?;}
            // Replacement never changes alpha; retain original native alpha bits.
            changed|=encoded!=original.get(..bpp).ok_or(StrokeError::Limit)?;
        }
        if changed{staged.try_write_interleaved_with(r,&raw,control).map_err(raster)?;changed_any=true;}
    }
    poll(control)?;if changed_any{*target=staged;Ok(bounds)}else{Ok(Rect::EMPTY)}
}
fn count_row(r:Rect)->Result<usize>{count(r)}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings,StrokePoint};
    use photocraft_color::PixelFormat;
    fn renderer()->StrokeRenderer {
        let brush=BrushSettings{size:8.0,hardness:1.0,flow:1.0,pressure_size:false,pressure_opacity:false,..Default::default()};
        let mut out=StrokeRenderer::try_new_with_interrupt(&brush,None,1.0,StrokeLimits::default(),&Interrupt::NONE).unwrap().record_dabs();
        out.push_with_interrupt(&[StrokePoint::new(0.5,0.5,1.0)],StrokeLimits::default(),&Interrupt::NONE).unwrap();out.finish_with_interrupt(StrokeLimits::default(),&Interrupt::NONE).unwrap();out
    }
    fn settings()->NativeLuminositySettings {NativeLuminositySettings{foreground:[0.75;3],background:[0.5;3],tolerance:0.0,sampling:Sampling::Once,limits:Limits::Discontiguous,anti_alias:false,opacity:1.0}}
    fn source(sample:SampleType,colors:&[f64])->Surface {
        let format=PixelFormat::new(ColorMode::Rgb,sample,true);let mut raw=Vec::new();
        for color in colors{let mut pixel=vec![0;format.bytes_per_pixel()];for channel in 0..3{try_write_sample_f64(&mut pixel,sample,channel,*color).unwrap();}try_write_sample_f64(&mut pixel,sample,3,0.75).unwrap();raw.extend_from_slice(&pixel);}
        Surface::try_from_interleaved_with(format,Rect::new(0,0,colors.len() as i32,1),&raw,&Interrupt::NONE).unwrap()
    }
    #[test]
    fn exact_tolerance_distinguishes_adjacent_u32_and_f64_values() {
        let painter=renderer();
        for sample in [SampleType::U32,SampleType::F64] {
            let (a,b)=if sample==SampleType::U32{(f64::from(0x8000_0000u32)/f64::from(u32::MAX),f64::from(0x8000_0001u32)/f64::from(u32::MAX))}else{(0.5,f64::from_bits(0.5f64.to_bits()+1))};
            let mut target=source(sample,&[a,b]);let rect=Rect::new(0,0,2,1);let before=target.try_read_interleaved_with(rect,&Interrupt::NONE).unwrap();
            try_apply_luminosity(&mut target,&painter,&settings(),None,false,StrokeLimits::default(),&Interrupt::NONE).unwrap();let after=target.try_read_interleaved_with(rect,&Interrupt::NONE).unwrap();let bpp=target.format().bytes_per_pixel();
            assert_ne!(after.get(..bpp),before.get(..bpp));assert_eq!(after.get(bpp..),before.get(bpp..),"adjacent code must not match after f32 collapse");
            assert_eq!(after.get(bpp-sample.bytes()..bpp),before.get(bpp-sample.bytes()..bpp),"alpha stays raw");
        }
    }
    #[test]
    fn full_dab_flood_does_not_cross_a_nonmatching_barrier() {
        let painter=renderer();let original=source(SampleType::F64,&[0.5,0.25,0.5]);let before=original.try_read_interleaved_with(Rect::new(0,0,3,1),&Interrupt::NONE).unwrap();
        let mut contiguous=original.clone();let mut settings=settings();settings.limits=Limits::Contiguous;
        try_apply_luminosity(&mut contiguous,&painter,&settings,None,true,StrokeLimits::default(),&Interrupt::NONE).unwrap();let bytes=contiguous.try_read_interleaved_with(Rect::new(0,0,3,1),&Interrupt::NONE).unwrap();assert_ne!(bytes.get(..32),before.get(..32));assert_eq!(bytes.get(64..96),before.get(64..96));
        let mut discontiguous=original;settings.limits=Limits::Discontiguous;
        try_apply_luminosity(&mut discontiguous,&painter,&settings,None,true,StrokeLimits::default(),&Interrupt::NONE).unwrap();assert_ne!(discontiguous.try_read_interleaved_with(Rect::new(2,0,3,1),&Interrupt::NONE).unwrap(),before.get(64..96).unwrap());
    }
    #[test]
    fn zero_selection_and_hidden_payloads_preserve_original_encoded_words() {
        let painter=renderer();let mut target=source(SampleType::F64,&[0.5]);let before=target.clone();let selection=Surface::new(PixelFormat::GRAY8);
        assert!(try_apply_luminosity(&mut target,&painter,&settings(),Some(&selection),false,StrokeLimits::default(),&Interrupt::NONE).unwrap().is_empty());assert_eq!(target,before);
        let format=PixelFormat::new(ColorMode::Rgb,SampleType::F64,true);let raw=[f64::from_bits(0x7ff8_0000_0000_1234),-0.0,0.5,-0.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();let mut hidden=Surface::try_from_interleaved_with(format,Rect::new(0,0,1,1),&raw,&Interrupt::NONE).unwrap();let before=hidden.clone();let mut setting=settings();setting.sampling=Sampling::BackgroundSwatch;
        try_apply_luminosity(&mut hidden,&painter,&setting,None,false,StrokeLimits::default(),&Interrupt::NONE).unwrap();assert_eq!(hidden,before);
    }
    #[test]
    fn cancel_and_tiny_budget_do_not_publish_partial_native_work() {
        let painter=renderer();let mut target=source(SampleType::F64,&[0.5,0.5]);let before=target.clone();
        let polls=std::cell::Cell::new(0usize);let late=||{polls.set(polls.get()+1);polls.get()>30};
        assert!(matches!(try_apply_luminosity(&mut target,&painter,&settings(),None,false,StrokeLimits::default(),&Interrupt::cancel_only(&late)),Err(StrokeError::Cancelled)));assert_eq!(target,before);
        assert!(try_apply_luminosity(&mut target,&painter,&settings(),None,false,StrokeLimits{resident_bytes:1,..Default::default()},&Interrupt::NONE).is_err());assert_eq!(target,before);
    }
}
