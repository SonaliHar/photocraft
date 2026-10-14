//! Controlled companion of segment::slic's exact grid/Lloyd/connectivity recipe.
//! No new source domain, region cap, or SLIC policy is inferred here. The enclosing
//! Subject owner reserves source plus the known 128-bytes/pixel private peak.
use crate::segment::{grabcut::controlled::Budget, slic::Superpixels};
use photocraft_raster::RasterInputError as E;
fn index(x:usize,y:usize,w:usize)->Result<usize,E> {y.checked_mul(w).and_then(|v|v.checked_add(x)).ok_or(E::LimitExceeded)}
fn at(lab:&[[f32;3]],x:usize,y:usize,w:usize,h:usize)->Result<[f32;3],E> {
    lab.get(index(x.min(w.checked_sub(1).ok_or(E::InvalidBounds)?),y.min(h.checked_sub(1).ok_or(E::InvalidBounds)?),w)?).copied().ok_or(E::LengthMismatch)
}
fn gradient(lab:&[[f32;3]],x:usize,y:usize,w:usize,h:usize,b:&Budget<'_, '_>)->Result<f32,E> {
    b.work(8)?;
    let left=at(lab,x.saturating_sub(1),y,w,h)?;let right=at(lab,x.checked_add(1).ok_or(E::LimitExceeded)?,y,w,h)?;
    let up=at(lab,x,y.saturating_sub(1),w,h)?;let down=at(lab,x,y.checked_add(1).ok_or(E::LimitExceeded)?,w,h)?;
    Ok(crate::segment::d2(left,right)+crate::segment::d2(up,down))
}
pub(super) fn slic_lab(lab:&[[f32;3]],w:usize,h:usize,target:usize,compactness:f32,iters:usize,b:&Budget<'_, '_>)->Result<Superpixels,E> {
    b.work(0)?;
    let n=w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if lab.len()!=n {return Err(E::LengthMismatch);}
    if !compactness.is_finite() {return Err(E::InvalidSample);}
    if n==0 {return Ok(Superpixels {w,h,labels:Vec::new(),count:0,step:1.0});}
    // Work admission is lower-only through the shared Budget. It is not an
    // arbitrary image dimension limit or a claim about total process memory.
    let iterations=iters.max(1);
    let iteration_work=u64::try_from(iterations).map_err(|_|E::LimitExceeded)?.checked_mul(256).ok_or(E::LimitExceeded)?.max(2048);
    b.ensure_work(u64::try_from(n).map_err(|_|E::LimitExceeded)?.checked_mul(iteration_work).ok_or(E::LimitExceeded)?)?;
    let _=u32::try_from(n).map_err(|_|E::LimitExceeded)?;
    let (wi,hi)=(i64::try_from(w).map_err(|_|E::InvalidBounds)?,i64::try_from(h).map_err(|_|E::InvalidBounds)?);
    for pixel in lab {b.work(1)?;if pixel.iter().any(|v|!v.is_finite()) {return Err(E::InvalidSample);}}
    let step=((n as f32/target.max(1) as f32).sqrt()).max(1.0);
    if !step.is_finite() {return Err(E::InvalidSample);}
    let mut centers:Vec<[f32;5]>=b.reserve(n)?;
    let mut y=step/2.0;
    while y<h as f32 {
        b.work(1)?;let mut x=step/2.0;
        while x<w as f32 {
            b.work(1)?;let (cx,cy)=(x as usize,y as usize);
            let (mut bx,mut by,mut best)=(cx,cy,f32::MAX);
            for yy in cy.saturating_sub(1)..=cy.checked_add(1).ok_or(E::LimitExceeded)?.min(h-1) {
                for xx in cx.saturating_sub(1)..=cx.checked_add(1).ok_or(E::LimitExceeded)?.min(w-1) {
                    let g=gradient(lab,xx,yy,w,h,b)?;
                    if g<best {(bx,by,best)=(xx,yy,g);}
                }
            }
            let c=lab.get(index(bx,by,w)?).copied().ok_or(E::LengthMismatch)?;
            if centers.len()>=n {return Err(E::LimitExceeded);}
            centers.push([c[0],c[1],c[2],bx as f32,by as f32]);
            let next=x+step;if !next.is_finite() || next<=x {return Err(E::InvalidBounds);}x=next;
        }
        let next=y+step;if !next.is_finite() || next<=y {return Err(E::InvalidBounds);}y=next;
    }
    let mut labels=b.filled(n,u32::MAX)?;let mut dist=b.filled(n,f32::MAX)?;
    let wxy=(compactness/step).powi(2);let radius=step.ceil() as i64;
    // Reusing the zeroed sums buffer preserves the same f64 summation order;
    // there is only one live sums allocation across all Lloyd iterations.
    let mut sums:Vec<[f64;6]>=b.filled(centers.len(),[0.;6])?;
    for _ in 0..iterations {
        b.work(1)?;
        for value in &mut dist {b.work(1)?;*value=f32::MAX;}
        for (k,c) in centers.iter().enumerate() {
            b.work(1)?;let label=u32::try_from(k).map_err(|_|E::LimitExceeded)?;
            let (cx,cy)=(c[3].round() as i64,c[4].round() as i64);
            let y0=cy.checked_sub(radius).ok_or(E::InvalidBounds)?.max(0);
            let y1=cy.checked_add(radius).and_then(|v|v.checked_add(1)).ok_or(E::InvalidBounds)?.min(hi);
            let x0=cx.checked_sub(radius).ok_or(E::InvalidBounds)?.max(0);
            let x1=cx.checked_add(radius).and_then(|v|v.checked_add(1)).ok_or(E::InvalidBounds)?.min(wi);
            for yy in y0..y1 {
                for xx in x0..x1 {
                    b.work(1)?;let i=index(usize::try_from(xx).map_err(|_|E::InvalidBounds)?,usize::try_from(yy).map_err(|_|E::InvalidBounds)?,w)?;
                    let p=lab.get(i).copied().ok_or(E::LengthMismatch)?;
                    let dc=crate::segment::d2(p,[c[0],c[1],c[2]]);let (dx,dy)=(xx as f32-c[3],yy as f32-c[4]);
                    let d=dc+(dx*dx+dy*dy)*wxy;
                    let previous=dist.get_mut(i).ok_or(E::LengthMismatch)?;
                    if d<*previous {*previous=d;*labels.get_mut(i).ok_or(E::LengthMismatch)?=label;}
                }
            }
        }
        for sum in &mut sums {b.work(1)?;*sum=[0.;6];}
        for (i,label) in labels.iter().enumerate() {
            b.work(1)?;if *label==u32::MAX {continue;}
            let p=lab.get(i).copied().ok_or(E::LengthMismatch)?;
            let sum=sums.get_mut(usize::try_from(*label).map_err(|_|E::LengthMismatch)?).ok_or(E::LengthMismatch)?;
            // Fixed fields, same operation and conversion order as legacy.
            let [sl,sa,sb,sx,sy,count]=sum;
            *sl+=p[0] as f64;*sa+=p[1] as f64;*sb+=p[2] as f64;*sx+=(i%w) as f64;*sy+=(i/w) as f64;*count+=1.;
        }
        for (center,sum) in centers.iter_mut().zip(&sums) {
            b.work(1)?;let [sl,sa,sb,sx,sy,count]=*sum;
            if count>0. {*center=[(sl/count) as f32,(sa/count) as f32,(sb/count) as f32,(sx/count) as f32,(sy/count) as f32];}
        }
    }
    for i in 0..n {
        b.work(1)?;
        if labels.get(i).copied().ok_or(E::LengthMismatch)?==u32::MAX {
            let neighbor=if i>0 {labels.get(i-1).copied().ok_or(E::LengthMismatch)?} else {0};
            *labels.get_mut(i).ok_or(E::LengthMismatch)?=neighbor;
        }
    }
    let (labels,count)=enforce_connectivity(&labels,w,h,((step*step)/4.0) as usize,b)?;
    b.work(0)?;Ok(Superpixels {w,h,labels,count,step})
}
fn enforce_connectivity(labels:&[u32],w:usize,h:usize,min_size:usize,b:&Budget<'_, '_>)->Result<(Vec<u32>,usize),E> {
    let n=w.checked_mul(h).ok_or(E::LimitExceeded)?;if labels.len()!=n {return Err(E::LengthMismatch);}
    let mut out=b.filled(n,u32::MAX)?;let mut next=0u32;let mut component:Vec<usize>=b.reserve(n)?;
    for start in 0..n {
        b.work(1)?;if out.get(start).copied().ok_or(E::LengthMismatch)?!=u32::MAX {continue;}
        let (sx,sy)=(start%w,start/w);let mut adjacent=None;
        // Legacy merge preference: left, then up (up wins when both exist).
        for neighbor in [if sx>0 {start.checked_sub(1)} else {None},if sy>0 {start.checked_sub(w)} else {None}].into_iter().flatten() {
            b.work(1)?;let label=out.get(neighbor).copied().ok_or(E::LengthMismatch)?;if label!=u32::MAX {adjacent=Some(label);}
        }
        component.clear();component.push(start);*out.get_mut(start).ok_or(E::LengthMismatch)?=next;
        let original=labels.get(start).copied().ok_or(E::LengthMismatch)?;let mut k=0usize;
        while k<component.len() {
            b.work(1)?;let i=component.get(k).copied().ok_or(E::LengthMismatch)?;k=k.checked_add(1).ok_or(E::LimitExceeded)?;
            let (x,y)=(i%w,i/w);
            for neighbor in [if x>0 {i.checked_sub(1)} else {None},if x.checked_add(1).is_some_and(|x|x<w) {i.checked_add(1)} else {None},
                if y>0 {i.checked_sub(w)} else {None},if y.checked_add(1).is_some_and(|y|y<h) {i.checked_add(w)} else {None}].into_iter().flatten() {
                b.work(1)?;
                if out.get(neighbor).copied().ok_or(E::LengthMismatch)?==u32::MAX && labels.get(neighbor).copied().ok_or(E::LengthMismatch)?==original {
                    *out.get_mut(neighbor).ok_or(E::LengthMismatch)?=next;
                    if component.len()>=n {return Err(E::LimitExceeded);}component.push(neighbor);
                }
            }
        }
        match adjacent {
            Some(label) if component.len()<min_size=>for &i in &component {b.work(1)?;*out.get_mut(i).ok_or(E::LengthMismatch)?=label;},
            _=>next=next.checked_add(1).ok_or(E::LimitExceeded)?,
        }
    }
    b.work(0)?;Ok((out,usize::try_from(next).map_err(|_|E::LimitExceeded)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::Interrupt;
    use std::cell::Cell;
    fn budget<'a,'b>(control:&'a Interrupt<'b>)->Budget<'a, 'b> {Budget {left:Cell::new(8_000_000_000),control}}
    #[test]
    fn exact_legacy_oracle_grid_gradient_lloyd_and_connectivity_order() {
        for (w,h) in [(1,17),(17,1),(3,5),(16,13)] {
            let lab:Vec<_>=(0..w*h).map(|i| crate::segment::slic::rgb_to_lab([
                (i%7) as f32/6.,((i/w)%5) as f32/4.,((i*3)%11) as f32/10.])).collect();
            for target in [0,1,4,w*h,w*h+3] {
                for iterations in [0,1,8] {
                    let expected=crate::segment::slic::slic_lab(&lab,w,h,target,10.,iterations);
                    let actual=slic_lab(&lab,w,h,target,10.,iterations,&budget(&Interrupt::NONE)).unwrap();
                    assert_eq!(actual.labels,expected.labels);assert_eq!(actual.count,expected.count);assert_eq!(actual.step.to_bits(),expected.step.to_bits());
                }
            }
        }
    }
    #[test]
    fn fragment_merge_uses_previous_adjacent_component_in_original_order() {
        let (labels,count)=enforce_connectivity(&[0,1,1,2],4,1,2,&budget(&Interrupt::NONE)).unwrap();
        assert_eq!(labels,vec![0,1,1,1]);assert_eq!(count,2);
        let (labels,count)=enforce_connectivity(&[0,1,2,3],2,2,2,&budget(&Interrupt::NONE)).unwrap();
        assert_eq!(labels,vec![0,0,0,0]);assert_eq!(count,1);
    }
    #[test]
    fn cancellation_and_low_work_never_return_partial_labelling_or_modify_input() {
        let lab=vec![[50.,1.,-2.];64];let original=lab.clone();
        for threshold in [1,15,100,500] {
            let polls=Cell::new(0usize);let cancel=||{let n=polls.get()+1;polls.set(n);n>=threshold};
            let control=Interrupt::cancel_only(&cancel);
            assert!(matches!(slic_lab(&lab,8,8,8,10.,8,&budget(&control)),Err(E::Cancelled)));
            assert_eq!(lab,original);
        }
        let small=Budget {left:Cell::new(64*2048-1),control:&Interrupt::NONE};
        assert!(matches!(slic_lab(&lab,8,8,8,10.,8,&small),Err(E::LimitExceeded)));
    }
    #[test]
    fn hostile_shape_nonfinite_and_empty_inputs_are_explicit_results() {
        assert!(matches!(slic_lab(&[],usize::MAX,2,1,10.,8,&budget(&Interrupt::NONE)),Err(E::LimitExceeded)));
        assert!(matches!(slic_lab(&[],1,1,1,10.,8,&budget(&Interrupt::NONE)),Err(E::LengthMismatch)));
        assert!(matches!(slic_lab(&[[f32::NAN,0.,0.]],1,1,1,10.,8,&budget(&Interrupt::NONE)),Err(E::InvalidSample)));
        let empty=slic_lab(&[],0,0,1,10.,8,&budget(&Interrupt::NONE)).unwrap();assert_eq!(empty.count,0);assert!(empty.labels.is_empty());
    }
}
