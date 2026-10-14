//! Native Gray nearest mesh transport using the existing four-pixel tessellation,
//! triangle declaration/overlap rules and nearest coordinate schedule. Pixel
//! words are never decoded; this is not source-operation qualification.
use crate::{selection_nearest_controlled,transform::Homography,wide_geometry::GeometryLimits};
use photocraft_geom::{Rect,TILE_SIZE,warp::Warp};
use photocraft_raster::{Surface,Interrupt,RasterInputError,checked_region_byte_len};
use std::mem::size_of;
type Result<T>=std::result::Result<T,RasterInputError>;
const CELL:f64=4.0;
const MAX_CELLS:usize=512;
type Vertex=([f64;2],[f64;2]);
#[derive(Clone,Copy,Debug)]
pub struct NativeMeshRequirements {pub output_bounds:Rect,pub vertices:usize,pub triangles:usize,pub retained_bytes:u64,pub staged_bytes:u64,pub scratch_bytes:u64,pub work_units:u64}
fn poll(c:&Interrupt<'_>)->Result<()> {c.check().map_err(|_|RasterInputError::Cancelled)}
fn add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or(RasterInputError::LimitExceeded)}
fn mul(a:u64,b:u64)->Result<u64>{a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)}
fn count(r:Rect)->Result<usize>{usize::try_from(r.width()).ok().and_then(|w|usize::try_from(r.height()).ok().and_then(|h|w.checked_mul(h))).ok_or(RasterInputError::LimitExceeded)}
fn tiles(r:Rect)->Result<u64>{if r.is_empty(){return Ok(0);}let w=i64::from(r.x1-1).div_euclid(i64::from(TILE_SIZE))-i64::from(r.x0).div_euclid(i64::from(TILE_SIZE))+1;let h=i64::from(r.y1-1).div_euclid(i64::from(TILE_SIZE))-i64::from(r.y0).div_euclid(i64::from(TILE_SIZE))+1;mul(u64::try_from(w).map_err(|_|RasterInputError::InvalidBounds)?,u64::try_from(h).map_err(|_|RasterInputError::InvalidBounds)?)}
fn grid(bounds:Rect)->Result<(usize,usize,usize,usize)>{
    if bounds.is_empty(){return Ok((0,0,0,0));}if [bounds.x0,bounds.y0,bounds.x1,bounds.y1].iter().any(|v|i64::from(*v).abs()>1_000_000){return Err(RasterInputError::InvalidBounds);}
    let nx=((f64::from(bounds.width())/CELL).ceil() as usize).clamp(1,MAX_CELLS);let ny=((f64::from(bounds.height())/CELL).ceil() as usize).clamp(1,MAX_CELLS);
    let vertices=nx.checked_add(1).and_then(|x|ny.checked_add(1).and_then(|y|x.checked_mul(y))).ok_or(RasterInputError::LimitExceeded)?;
    let triangles=nx.checked_mul(ny).and_then(|n|n.checked_mul(2)).ok_or(RasterInputError::LimitExceeded)?;Ok((nx,ny,vertices,triangles))
}
fn warp_charge(w:&Warp,l:GeometryLimits,c:&Interrupt<'_>)->Result<(u64,u64)>{
    poll(c)?;if w.bounds.iter().any(|v|!v.is_finite()||v.abs()>1_000_000.0)||w.bounds[2]<w.bounds[0]||w.bounds[3]<w.bounds[1]||[w.bend,w.h_distort,w.v_distort].iter().any(|v|!v.is_finite()||v.abs()>100.0){return Err(RasterInputError::InvalidBounds);}
    let mut bytes=size_of::<Warp>() as u64;let mut work=64;
    if let Some(mesh)=&w.mesh{
        if !(2..=65).contains(&mesh.us.len())||!(2..=65).contains(&mesh.vs.len())||mesh.points.len()>37_249{return Err(RasterInputError::InvalidBounds);}
        bytes=add(bytes,add(mul(add(mesh.us.capacity() as u64,mesh.vs.capacity() as u64)?,8)?,mul(mesh.points.capacity() as u64,16)?)?)?;
        work=add(work,mul(add(add(mesh.us.len() as u64,mesh.vs.len() as u64)?,mesh.points.len() as u64)?,64)?)?;
        if bytes>l.resident_bytes||work>l.work_units{return Err(RasterInputError::LimitExceeded);}
        for knots in [&mesh.us,&mesh.vs]{if knots.first()!=Some(&0.0)||knots.last()!=Some(&1.0){return Err(RasterInputError::InvalidBounds);}for pair in knots.windows(2){poll(c)?;let(a,b)=(pair.first().copied().ok_or(RasterInputError::LengthMismatch)?,pair.get(1).copied().ok_or(RasterInputError::LengthMismatch)?);if !a.is_finite()||!b.is_finite()||b<=a{return Err(RasterInputError::InvalidBounds);}}}
        let columns=(mesh.us.len()-1).checked_mul(3).and_then(|n|n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;let rows=(mesh.vs.len()-1).checked_mul(3).and_then(|n|n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;
        if columns.checked_mul(rows)!=Some(mesh.points.len()){return Err(RasterInputError::LengthMismatch);}
        for p in &mesh.points{poll(c)?;if p.iter().any(|v|!v.is_finite()||v.abs()>1_000_000.0){return Err(RasterInputError::InvalidBounds);}}
    }
    Ok((bytes,work))
}
fn vertex(bounds:Rect,nx:usize,ny:usize,i:usize,j:usize,w:&Warp)->Result<Vertex>{
    let sx=f64::from(bounds.x0)+f64::from(bounds.width())*i as f64/nx as f64;let sy=f64::from(bounds.y0)+f64::from(bounds.height())*j as f64/ny as f64;
    let(dx,dy)=w.map(sx,sy);if [sx,sy,dx,dy].iter().any(|v|!v.is_finite()||v.abs()>1_000_000.0){return Err(RasterInputError::InvalidBounds);}Ok(([dx,dy],[sx,sy]))
}
// Shared with execution: half-open, padded pixel-center bbox, retaining the
// original determinant schedule and degenerate-triangle threshold.
fn triangle_clip(p0:[f64;2],p1:[f64;2],p2:[f64;2],clip:Rect)->Result<Option<(f64,Rect)>> {
    let det=(p1[0]-p0[0])*(p2[1]-p0[1])-(p2[0]-p0[0])*(p1[1]-p0[1]);
    if !det.is_finite(){return Err(RasterInputError::InvalidBounds);}
    if det.abs()<1e-12{return Ok(None);}
    let x0=(p0[0].min(p1[0]).min(p2[0])-0.5).floor().max(f64::from(clip.x0)) as i32;
    let y0=(p0[1].min(p1[1]).min(p2[1])-0.5).floor().max(f64::from(clip.y0)) as i32;
    let x1=(p0[0].max(p1[0]).max(p2[0])+0.5).ceil().min(f64::from(clip.x1)) as i32;
    let y1=(p0[1].max(p1[1]).max(p2[1])+0.5).ceil().min(f64::from(clip.y1)) as i32;
    let bounds=Rect::new(x0,y0,x1,y1);
    Ok(if bounds.is_empty(){None}else{Some((det,bounds))})
}
/// Allocation-free preflight, including deterministic vertex and triangle map passes.
/// Typed Warp::map has bounded validated control arrays; no caller callbacks.
pub fn requirements(source:&Surface,bounds:Rect,w:&Warp,l:GeometryLimits,c:&Interrupt<'_>)->Result<NativeMeshRequirements>{
    poll(c)?;let standard=GeometryLimits::default();if l.resident_bytes>standard.resident_bytes||l.work_units>standard.work_units{return Err(RasterInputError::LimitExceeded);}
    checked_region_byte_len(bounds,source.format())?;let source_req=selection_nearest_controlled::requirements(source,Rect::EMPTY,&Homography::IDENTITY,l,c)?;
    let(warp_bytes,warp_work)=warp_charge(w,l,c)?;let(nx,ny,vc,tc)=grid(bounds)?;
    let inventory=add(mul(vc as u64,size_of::<Vertex>() as u64)?,mul(tc as u64,size_of::<[usize;3]>() as u64)?)?;
    let retained=add(source_req.retained_bytes,warp_bytes)?;let preparation=add(source_req.work_units,add(warp_work,add(mul(vc as u64,2048)?,mul(tc as u64,4096)?)?)?)?;
    if add(retained,inventory)?>l.resident_bytes||preparation>l.work_units{return Err(RasterInputError::LimitExceeded);}
    let dst=if bounds.is_empty(){Rect::EMPTY}else{
        let(mut lo,mut hi)=([f64::MAX;2],[f64::MIN;2]);for j in 0..=ny{poll(c)?;for i in 0..=nx{poll(c)?;let(p,_)=vertex(bounds,nx,ny,i,j,w)?;lo[0]=lo[0].min(p[0]);lo[1]=lo[1].min(p[1]);hi[0]=hi[0].max(p[0]);hi[1]=hi[1].max(p[1]);}}
        Rect::new((lo[0].floor() as i32).checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,(lo[1].floor() as i32).checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,(hi[0].ceil() as i32).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,(hi[1].ceil() as i32).checked_add(1).ok_or(RasterInputError::InvalidBounds)?)
    };
    let nt=tiles(dst)?;let area=(TILE_SIZE*TILE_SIZE) as u64;let tile=mul(area,source.format().bytes_per_pixel() as u64)?;
    let output=add(source.default_bytes().len() as u64,mul(nt,add(tile,1024)?)?)?;
    let scratch=if dst.is_empty(){0}else{add(mul(area,(size_of::<[f64;2]>()+size_of::<u32>()) as u64)?,add(mul(tile,4)?,128)?)?};
    // Execution still visits every triangle header for every output tile. Keep
    // that charge, plus raw output initialization/copy and growing-map work.
    let base_work=add(preparation,add(mul(mul(nt,tc as u64)?,256)?,add(mul(mul(nt,tile)?,32)?,mul(mul(nt,add(source.tile_count() as u64,nt)?)?,256)?)?)?)?;
    if base_work>l.work_units{return Err(RasterInputError::LimitExceeded);}
    let mut work=base_work;
    // Tile clipping partitions each full-destination triangle bbox exactly:
    // summing its area counts all prospective containment tests, including
    // rejected pixels and overlapping/last-owner triangles. No inventory alloc.
    for j in 0..ny{poll(c)?;for i in 0..nx{poll(c)?;
        let(p00,_)=vertex(bounds,nx,ny,i,j,w)?;let(p10,_)=vertex(bounds,nx,ny,i+1,j,w)?;
        let(p11,_)=vertex(bounds,nx,ny,i+1,j+1,w)?;let(p01,_)=vertex(bounds,nx,ny,i,j+1,w)?;
        for(p0,p1,p2)in [(p00,p10,p11),(p00,p11,p01)]{
            if let Some((_,bbox))=triangle_clip(p0,p1,p2,dst)?{
                work=add(work,add(mul(count(bbox)? as u64,128)?,mul(tiles(bbox)?,64)?)?)?;
                if work>l.work_units{return Err(RasterInputError::LimitExceeded);}
            }
        }
    }}
    let staged=add(inventory,output)?;if add(retained,add(staged,scratch)?)?>l.resident_bytes||work>l.work_units{return Err(RasterInputError::LimitExceeded);}
    poll(c)?;Ok(NativeMeshRequirements{output_bounds:dst,vertices:vc,triangles:tc,retained_bytes:retained,staged_bytes:staged,scratch_bytes:scratch,work_units:work})
}
fn vector<T:Clone>(n:usize,value:T,c:&Interrupt<'_>)->Result<Vec<T>>{let mut v=Vec::new();v.try_reserve_exact(n).map_err(|_|RasterInputError::AllocationFailed)?;if v.capacity()!=n{return Err(RasterInputError::LimitExceeded);}for _ in 0..n{if v.len().is_multiple_of(256){poll(c)?;}v.push(value.clone());}Ok(v)}
pub fn try_warp_gray_nearest_mesh_with_limits(source:&Surface,bounds:Rect,w:&Warp,l:GeometryLimits,c:&Interrupt<'_>)->Result<Surface>{
    let req=requirements(source,bounds,w,l,c)?;let(nx,ny,_,_)=grid(bounds)?;
    let mut vertices:Vec<Vertex>=Vec::new();vertices.try_reserve_exact(req.vertices).map_err(|_|RasterInputError::AllocationFailed)?;let mut triangles:Vec<[usize;3]>=Vec::new();triangles.try_reserve_exact(req.triangles).map_err(|_|RasterInputError::AllocationFailed)?;
    if vertices.capacity()!=req.vertices||triangles.capacity()!=req.triangles{return Err(RasterInputError::LimitExceeded);}
    if !bounds.is_empty(){for j in 0..=ny{poll(c)?;for i in 0..=nx{poll(c)?;vertices.push(vertex(bounds,nx,ny,i,j,w)?);}}
        for j in 0..ny{poll(c)?;for i in 0..nx{let a=j.checked_mul(nx+1).and_then(|n|n.checked_add(i)).ok_or(RasterInputError::LimitExceeded)?;triangles.push([a,a+1,a+nx+2]);triangles.push([a,a+nx+2,a+nx+1]);}}
    }
    let fmt=source.format();let bpp=fmt.bytes_per_pixel();let mut output=Surface::try_with_default_bytes(fmt,source.default_bytes())?;
    for position in req.output_bounds.tiles(){poll(c)?;let t=position.rect().intersect(&req.output_bounds);let width=usize::try_from(t.width()).map_err(|_|RasterInputError::LimitExceeded)?;let n=count(t)?;
        let mut uv=vector(n,[f64::NAN;2],c)?;let mut owner=vector(n,u32::MAX,c)?;
        for(k,tri)in triangles.iter().enumerate(){poll(c)?;let[i0,i1,i2]=*tri;let(p0,s0)=*vertices.get(i0).ok_or(RasterInputError::LengthMismatch)?;let(p1,s1)=*vertices.get(i1).ok_or(RasterInputError::LengthMismatch)?;let(p2,s2)=*vertices.get(i2).ok_or(RasterInputError::LengthMismatch)?;
            let Some((det,bbox))=triangle_clip(p0,p1,p2,t)? else{continue;};
            for y in bbox.y0..bbox.y1{poll(c)?;let py=f64::from(y)+0.5;for x in bbox.x0..bbox.x1{if x.rem_euclid(64)==0{poll(c)?;}let px=f64::from(x)+0.5;
                let l1=((px-p0[0])*(p2[1]-p0[1])-(p2[0]-p0[0])*(py-p0[1]))/det;let l2=((p1[0]-p0[0])*(py-p0[1])-(px-p0[0])*(p1[1]-p0[1]))/det;let l0=1.0-l1-l2;
                if !l0.is_finite()||!l1.is_finite()||!l2.is_finite(){return Err(RasterInputError::InvalidBounds);}if l0< -1e-9||l1< -1e-9||l2< -1e-9{continue;}
                let at=usize::try_from(y-t.y0).ok().and_then(|row|row.checked_mul(width)).and_then(|n|usize::try_from(x-t.x0).ok().and_then(|x|n.checked_add(x))).ok_or(RasterInputError::InvalidBounds)?;
                *uv.get_mut(at).ok_or(RasterInputError::LengthMismatch)?=[l0*s0[0]+l1*s1[0]+l2*s2[0],l0*s0[1]+l1*s1[1]+l2*s2[1]];
                *owner.get_mut(at).ok_or(RasterInputError::LengthMismatch)?=u32::try_from(k).map_err(|_|RasterInputError::LimitExceeded)?;
            }}
        }
        let bytes=checked_region_byte_len(t,fmt)?;let mut raw=Vec::new();raw.try_reserve_exact(bytes).map_err(|_|RasterInputError::AllocationFailed)?;if raw.capacity()!=bytes{return Err(RasterInputError::LimitExceeded);}let mut nondefault=false;
        for(i,(q,who))in uv.iter().zip(&owner).enumerate(){if i.is_multiple_of(128){poll(c)?;}let mut sample=[0u8;8];let sample=sample.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;sample.copy_from_slice(source.default_bytes());
            if *who!=u32::MAX{if *who as usize>=triangles.len()||q.iter().any(|v|!v.is_finite()||v.abs()>f64::from(i32::MAX-2)){return Err(RasterInputError::InvalidBounds);}
                // Preserve the ordinary point sampler's subtraction/addition
                // schedule rather than algebraically rewriting its tie behavior.
                let(px,py)=(((q[0]-0.5)+0.5).floor() as i32,((q[1]-0.5)+0.5).floor() as i32);
                if bounds.contains(px,py){source.try_read_pixel_bytes(px,py,sample)?;}
            }
            nondefault|=sample!=source.default_bytes();if raw.len().checked_add(bpp).is_none_or(|n|n>raw.capacity()){return Err(RasterInputError::LimitExceeded);}raw.extend_from_slice(sample);
        }
        if raw.len()!=bytes{return Err(RasterInputError::LengthMismatch);}if nondefault{output.try_write_interleaved_with(t,&raw,c)?;}
    }
    poll(c)?;Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode,PixelFormat,SampleType,try_write_sample_f64};
    use photocraft_geom::warp::BezierMesh;
    fn source(sample:SampleType,r:Rect)->Surface {let fmt=PixelFormat::new(ColorMode::Grayscale,sample,false);let mut s=Surface::with_default(fmt,&[0.25]);let mut bytes=vec![0;checked_region_byte_len(r,fmt).unwrap()];for(i,p)in bytes.chunks_exact_mut(fmt.bytes_per_pixel()).enumerate(){try_write_sample_f64(p,sample,0,if i%2==0{0.5}else{0.75}).unwrap();}s.try_write_interleaved_with(r,&bytes,&Interrupt::NONE).unwrap();s}
    #[test]
    fn ordinary_nearest_coordinate_oracle_including_folded_last_triangle() {
        let r=Rect::new(0,0,8,4);let ordinary=source(SampleType::F32,r);let native=source(SampleType::F64,r);
        let bounds=[0.0,0.0,8.0,4.0];let fold=BezierMesh::fit(&|u,v|[16.0*u*(1.0-u),4.0*v],vec![0.0,1.0],vec![0.0,1.0]);
        for warp in [Warp::preset(photocraft_geom::warp::WarpStyle::Arc,30.0,bounds),Warp::custom(fold,bounds)] {
            let expected=crate::selection_transform_controlled::try_warp_gray_mesh_with_limits(&ordinary,&|x,y|warp.map(x,y),crate::transform::Interp::Nearest,GeometryLimits::default(),&Interrupt::NONE).unwrap();
            let actual=try_warp_gray_nearest_mesh_with_limits(&native,r,&warp,GeometryLimits::default(),&Interrupt::NONE).unwrap();
            let region=expected.try_content_bounds_with_interrupt(&Interrupt::NONE).unwrap().union(&actual.try_content_bounds_with_interrupt(&Interrupt::NONE).unwrap()).union(&r).inflate(2);
            for y in region.y0..region.y1{for x in region.x0..region.x1{let mut a=[0.0];let mut b=[0.0];expected.try_read_pixel_f64(x,y,&mut a).unwrap();actual.try_read_pixel_f64(x,y,&mut b).unwrap();assert_eq!(a[0].to_bits(),b[0].to_bits(),"coordinate/triangle owner at {x},{y}");}}
        }
    }
    #[test]
    fn modest_mesh_admission_and_exact_identity_coordinate_oracle() {
        let r=Rect::new(0,0,128,128);let native=source(SampleType::F64,r);
        let warp=Warp::none([0.0,0.0,128.0,128.0]);
        let req=requirements(&native,r,&warp,GeometryLimits::default(),&Interrupt::NONE).unwrap();
        assert_eq!(req.triangles,2048);
        assert!(req.work_units<GeometryLimits::default().work_units);
        // Identity tessellation's exact center/floor oracle is the raw input,
        // including all vacated/padded output pixels (the native default).
        let out=try_warp_gray_nearest_mesh_with_limits(&native,r,&warp,GeometryLimits::default(),&Interrupt::NONE).unwrap();
        assert!(selection_nearest_controlled::raw_equal_with_limits(&native,&out,GeometryLimits::default(),&Interrupt::NONE).unwrap());
        for tri in [([0.0,0.0],[4.0,0.0],[4.0,4.0]),([0.0,0.0],[4.0,4.0],[0.0,4.0])]{
            let(_,bbox)=triangle_clip(tri.0,tri.1,tri.2,req.output_bounds).unwrap().unwrap();
            let partition:u64=bbox.tiles().map(|t|count(t.rect().intersect(&bbox)).unwrap() as u64).sum();
            assert_eq!(partition,count(bbox).unwrap() as u64);
        }
    }
    #[test]
    fn raw_nan_default_signed_zero_offcanvas_and_empty_source() {
        let fmt=PixelFormat::new(ColorMode::Grayscale,SampleType::F64,false);let default=0x7ff8_0000_0000_0061u64.to_ne_bytes();let r=Rect::new(-3,1,0,2);let mut s=Surface::try_with_default_bytes(fmt,&default).unwrap();let words=[(-0.0f64).to_bits(),0x7ff8_0000_0000_0042u64,0.5f64.to_bits()+1];let bytes:Vec<u8>=words.into_iter().flat_map(u64::to_ne_bytes).collect();s.try_write_interleaved_with(r,&bytes,&Interrupt::NONE).unwrap();
        let mesh=BezierMesh::fit(&|u,v|[-3.0+3.0*u-7.0,1.0+v+2.0],vec![0.0,1.0],vec![0.0,1.0]);let warp=Warp::custom(mesh,[-3.0,1.0,0.0,2.0]);
        let out=try_warp_gray_nearest_mesh_with_limits(&s,r,&warp,GeometryLimits::default(),&Interrupt::NONE).unwrap();assert_eq!(out.default_bytes(),default);assert_eq!(out.try_read_interleaved_with(Rect::new(-10,3,-7,4),&Interrupt::NONE).unwrap(),bytes);assert_eq!(s.try_read_interleaved_with(r,&Interrupt::NONE).unwrap(),bytes);
        let empty=Surface::try_with_default_bytes(fmt,&default).unwrap();let out=try_warp_gray_nearest_mesh_with_limits(&empty,Rect::EMPTY,&warp,GeometryLimits::default(),&Interrupt::NONE).unwrap();assert_eq!(out.default_bytes(),default);assert_eq!(out.tile_count(),0);
    }
    #[test]
    fn malformed_mesh_limits_and_cancel_refuse_before_output_escape() {
        let r=Rect::new(0,0,8,4);let s=source(SampleType::U32,r);let before=s.clone();let mut warp=Warp::custom(BezierMesh::identity([0.0,0.0,8.0,4.0],1,1),[0.0,0.0,8.0,4.0]);warp.mesh.as_mut().unwrap().us.clear();assert!(try_warp_gray_nearest_mesh_with_limits(&s,r,&warp,GeometryLimits::default(),&Interrupt::NONE).is_err());
        let warp=Warp::none([0.0,0.0,8.0,4.0]);let cancel=||true;assert_eq!(try_warp_gray_nearest_mesh_with_limits(&s,r,&warp,GeometryLimits::default(),&Interrupt::cancel_only(&cancel)),Err(RasterInputError::Cancelled));assert!(try_warp_gray_nearest_mesh_with_limits(&s,r,&warp,GeometryLimits{resident_bytes:1,work_units:1},&Interrupt::NONE).is_err());assert_eq!(s,before);
    }
}
