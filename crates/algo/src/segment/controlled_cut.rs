//! Bounded weighted companion of the existing optimized grid cut.
use super::{RgbImage, maxflow::Graph};
use photocraft_raster::{Interrupt, RasterInputError as E};
use std::cell::Cell;
pub(crate) struct Budget<'a, 'b> {
    pub left: &'a Cell<u64>,
    pub control: &'a Interrupt<'b>,
}
impl Budget<'_, '_> {
    pub fn ensure_work(&self, n: u64) -> Result<(), E> {
        self.control.check().map_err(|_| E::Cancelled)?;
        if n > self.left.get() {
            return Err(E::LimitExceeded);
        }
        Ok(())
    }
    pub fn work(&self, n: u64) -> Result<(), E> {
        self.control.check().map_err(|_| E::Cancelled)?;
        self.left.set(self.left.get().checked_sub(n).ok_or(E::LimitExceeded)?);
        Ok(())
    }
    pub fn reserve<T>(&self, n: usize) -> Result<Vec<T>, E> {
        self.work(1)?;
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
        if v.capacity() > n.saturating_add(16) {
            return Err(E::AllocationFailed);
        }
        Ok(v)
    }
    pub fn filled<T: Clone>(&self, n: usize, z: T) -> Result<Vec<T>, E> {
        let mut v = self.reserve(n)?;
        while v.len() < n {
            let end = v.len().saturating_add(1024).min(n);
            self.work((end - v.len()) as u64)?;
            v.resize(end, z.clone());
        }
        Ok(v)
    }
}
fn index(x: usize, y: usize, w: usize) -> Result<usize, E> {
    y.checked_mul(w).and_then(|v| v.checked_add(x)).ok_or(E::LimitExceeded)
}
fn at(img: &RgbImage, x: usize, y: usize) -> Result<[f32; 3], E> {
    img.px.get(index(x, y, img.w)?).copied().ok_or(E::LengthMismatch)
}
pub(crate) fn cut(img: &RgbImage, cf: &[f32], cb: &[f32], fixed: &[u8], weight: &dyn Fn(f32) -> f32, b: &Budget<'_, '_>) -> Result<Vec<bool>, E> {
    let n = img.w.checked_mul(img.h).ok_or(E::LimitExceeded)?;
    if [img.px.len(), cf.len(), cb.len(), fixed.len()].iter().any(|v| *v != n) {
        return Err(E::LengthMismatch);
    }
    b.ensure_work((n as u64).checked_mul(256).ok_or(E::LimitExceeded)?)?;
    let mut idx = b.filled(n, u32::MAX)?;
    let mut count = 0usize;
    let mut out = b.reserve(n)?;
    for (i, f) in fixed.iter().enumerate() {
        b.work(1)?;
        out.push(*f == crate::segment::HARD_FG);
        if *f == crate::segment::FREE {
            *idx.get_mut(i).ok_or(E::LengthMismatch)? = u32::try_from(count).map_err(|_| E::LimitExceeded)?;
            count += 1;
        }
    }
    if count == 0 {
        return Ok(out);
    }
    let mut graph = Graph::try_with_capacity(count, count.checked_mul(4).ok_or(E::LimitExceeded)?, b.control)?;
    for i in 0..n {
        b.work(1)?;
        let id = *idx.get(i).ok_or(E::LengthMismatch)?;
        if id != u32::MAX {
            graph.add_tweights(id as usize, *cb.get(i).ok_or(E::LengthMismatch)?, *cf.get(i).ok_or(E::LengthMismatch)?);
        }
    }
    // Same optimized sparse edge-start ordering as the legacy grid cut.
    let mut free = b.reserve(count)?;
    let sparse = count < n / 8;
    if sparse {
        for (i, id) in idx.iter().enumerate() {
            b.work(1)?;
            if *id != u32::MAX {
                free.push(i);
            }
        }
    }
    let mut visit = |i: usize| -> Result<(), E> {
        let x = i % img.w;
        let y = i / img.w;
        let a = at(img, x, y)?;
        for (dx, dy, len) in crate::segment::NB {
            b.work(1)?;
            let nx = x as i64 + i64::from(dx);
            let ny = y as i64 + i64::from(dy);
            if nx < 0 || ny < 0 || nx >= img.w as i64 || ny >= img.h as i64 {
                continue;
            }
            let j = index(nx as usize, ny as usize, img.w)?;
            let ia = *idx.get(i).ok_or(E::LengthMismatch)?;
            let ib = *idx.get(j).ok_or(E::LengthMismatch)?;
            if ia == u32::MAX && ib == u32::MAX {
                continue;
            }
            let wt = weight(crate::segment::d2(a, at(img, nx as usize, ny as usize)?)) / len;
            if !wt.is_finite() || wt < 0. {
                return Err(E::InvalidSample);
            }
            match (ia != u32::MAX, ib != u32::MAX) {
                (true, true) => graph.add_edge(ia as usize, ib as usize, wt, wt),
                (true, false) => crate::segment::add_fixed_link(&mut graph, ia as usize, *fixed.get(j).ok_or(E::LengthMismatch)?, wt),
                (false, true) => crate::segment::add_fixed_link(&mut graph, ib as usize, *fixed.get(i).ok_or(E::LengthMismatch)?, wt),
                _ => (),
            }
        }
        Ok(())
    };
    if sparse {
        for i in crate::segment::edge_starts::Starts::new(&free, img.w) {
            visit(i)?;
        }
    } else {
        for i in 0..n {
            visit(i)?;
        }
    }
    let stopped = Cell::new(None);
    let cancel = || match b.work(1) {
        Ok(()) => false,
        Err(e) => {
            stopped.set(Some(e));
            true
        }
    };
    if graph.maxflow_controlled(&cancel).is_none() {
        return Err(stopped.get().unwrap_or(E::AllocationFailed));
    }
    for (i, id) in idx.iter().enumerate() {
        b.work(1)?;
        if *id != u32::MAX {
            *out.get_mut(i).ok_or(E::LengthMismatch)? = graph.in_source(*id as usize);
        }
    }
    Ok(out)
}
