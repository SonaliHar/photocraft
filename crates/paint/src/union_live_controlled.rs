//! Controlled upstream live max-union and smoothing-tail transactions.
//! Legacy APIs remain untouched. Standard hash/BTree/Arc allocator internals
//! remain managed by the library, with prospective and actual capacity admission.
use super::*;
use photocraft_raster::Interrupt;
use std::mem::size_of;
type Result<T> = std::result::Result<T, StrokeError>;
const MAX_KEYS: usize = 100_000;
#[path = "union_finished_controlled.rs"]
mod union_finished_controlled;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| StrokeError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(StrokeError::Limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(StrokeError::Limit)
}
struct Budget<'a, 'b> {
    bytes: u64,
    work: u64,
    control: &'a Interrupt<'b>,
}
impl Budget<'_, '_> {
    fn limits(&self) -> StrokeLimits {
        StrokeLimits { resident_bytes: self.bytes, work_units: self.work }
    }
    fn charge(&mut self, bytes: u64, work: u64) -> Result<()> {
        poll(self.control)?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or(StrokeError::Limit)?;
        self.work = self.work.checked_sub(work).ok_or(StrokeError::Limit)?;
        Ok(())
    }
    fn vector<T>(&mut self, count: usize) -> Result<Vec<T>> {
        let bytes = mul(count as u64, size_of::<T>() as u64)?;
        if bytes > self.bytes {
            return Err(StrokeError::Limit);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(count).map_err(|_| StrokeError::Allocation)?;
        self.charge(mul(out.capacity() as u64, size_of::<T>() as u64)?, count as u64)?;
        Ok(out)
    }
    fn copy<T: Copy>(&mut self, values: &[T]) -> Result<Vec<T>> {
        let mut out = self.vector(values.len())?;
        for chunk in values.chunks(256) {
            self.charge(0, chunk.len() as u64)?;
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }
    fn set(&mut self, count: usize) -> Result<HashSet<(i32, i32)>> {
        if count > MAX_KEYS || mul(count as u64, 128)? > self.bytes {
            return Err(StrokeError::Limit);
        }
        let mut out = HashSet::new();
        out.try_reserve(count).map_err(|_| StrokeError::Allocation)?;
        self.charge(mul(out.capacity() as u64, 64)?, count as u64)?;
        Ok(out)
    }
    fn insert(&mut self, set: &mut HashSet<(i32, i32)>, key: (i32, i32)) -> Result<()> {
        self.charge(0, 1)?;
        if !set.contains(&key) {
            if set.len() >= set.capacity() || set.len() >= MAX_KEYS {
                return Err(StrokeError::Limit);
            }
            set.insert(key);
        }
        Ok(())
    }
    fn clone_renderer(&mut self, source: &StrokeRenderer) -> Result<StrokeRenderer> {
        let req = source.incremental_clone_requirements(self.limits(), self.control)?;
        let output = source
            .try_clone_with_interrupt(StrokeLimits { resident_bytes: add(req.retained_bytes, req.staged_bytes)?, work_units: req.work_units }, self.control)?;
        self.charge(req.staged_bytes, req.work_units)?;
        Ok(output)
    }
}
fn tile_rect(tx: i32, ty: i32) -> Result<Rect> {
    let x = tx.checked_mul(COV_TILE).ok_or(StrokeError::Limit)?;
    let y = ty.checked_mul(COV_TILE).ok_or(StrokeError::Limit)?;
    Ok(Rect::new(x, y, x.checked_add(COV_TILE).ok_or(StrokeError::Limit)?, y.checked_add(COV_TILE).ok_or(StrokeError::Limit)?))
}
fn rect_keys(rect: Rect) -> Result<usize> {
    if rect.is_empty() {
        return Ok(0);
    }
    let x = rect.x1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE);
    let y = rect.y1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE);
    let columns = u64::try_from(i64::from(x) - i64::from(rect.x0.div_euclid(COV_TILE)) + 1).map_err(|_| StrokeError::Limit)?;
    let rows = u64::try_from(i64::from(y) - i64::from(rect.y0.div_euclid(COV_TILE)) + 1).map_err(|_| StrokeError::Limit)?;
    usize::try_from(mul(columns, rows)?).map_err(|_| StrokeError::Limit)
}
fn add_rect(set: &mut HashSet<(i32, i32)>, rect: Rect, budget: &mut Budget<'_, '_>) -> Result<()> {
    if rect.is_empty() {
        return Ok(());
    }
    for ty in rect.y0.div_euclid(COV_TILE)..=rect.y1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE) {
        for tx in rect.x0.div_euclid(COV_TILE)..=rect.x1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE) {
            budget.insert(set, (tx, ty))?;
        }
    }
    Ok(())
}
fn sorted(set: &HashSet<(i32, i32)>, budget: &mut Budget<'_, '_>) -> Result<Vec<(i32, i32)>> {
    let mut keys = budget.vector(set.len())?;
    budget.charge(0, mul(set.capacity() as u64, 2)?)?;
    for key in set {
        budget.charge(0, 1)?;
        keys.push(*key);
    }
    fn sift(keys: &mut [(i32, i32)], mut root: usize, end: usize, budget: &mut Budget<'_, '_>) -> Result<()> {
        loop {
            budget.charge(0, 1)?;
            let child = root.checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(StrokeError::Limit)?;
            if child >= end {
                return Ok(());
            }
            let right = child + 1;
            let largest = if right < end && keys.get(right) > keys.get(child) { right } else { child };
            if keys.get(root) >= keys.get(largest) {
                return Ok(());
            }
            keys.swap(root, largest);
            root = largest;
        }
    }
    for root in (0..keys.len() / 2).rev() {
        sift(&mut keys, root, set.len(), budget)?;
    }
    for end in (1..keys.len()).rev() {
        budget.charge(0, 1)?;
        keys.swap(0, end);
        sift(&mut keys, 0, end, budget)?;
    }
    Ok(keys)
}
fn pending_work(renderer: &StrokeRenderer, control: &Interrupt<'_>) -> Result<u64> {
    let mut pixels = 0u64;
    let mut bound = Rect::EMPTY;
    let mut dual_bound = Rect::EMPTY;
    for (dab, dual) in renderer.dab_buf.iter().map(|d| (d, false)).chain(renderer.dual_buf.iter().map(|d| (d, true))) {
        poll(control)?;
        let rect = renderer.ctx.dab_rect(dab, dual);
        let bytes =
            photocraft_raster::checked_region_byte_len(rect, PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false)).map_err(|_| StrokeError::Limit)?;
        pixels = add(pixels, bytes as u64 / 4)?;
        if dual {
            dual_bound = dual_bound.union(&rect);
        } else {
            bound = bound.union(&rect);
        }
    }
    let tiles = add(rect_keys(bound)? as u64, rect_keys(dual_bound)? as u64)?;
    let old = add(renderer.cov.tiles.capacity() as u64, renderer.dual.as_ref().map_or(0, |d| d.tiles.capacity()) as u64)?;
    // Each nonempty raster row span also consumes one bookkeeping unit;
    // there cannot be more spans than pixels. Pay it before rasterization.
    add(mul(pixels, 2049)?, mul(mul(tiles, add(old, tiles)?)?, 64)?)
}
fn tail(source: &StrokeRenderer, budget: &mut Budget<'_, '_>) -> Result<Option<StrokeRenderer>> {
    let mut candidate = budget.clone_renderer(source)?;
    candidate.dab_buf.clear();
    candidate.dual_buf.clear();
    candidate.all_dabs = None;
    let req = candidate.generator.finish_requirements(&candidate.dab_buf, &candidate.dual_buf, budget.limits(), budget.control)?;
    candidate.generator.finish_with_interrupt(
        &mut candidate.dab_buf,
        &mut candidate.dual_buf,
        StrokeLimits { resident_bytes: add(req.retained_bytes, req.staged_bytes)?, work_units: req.work_units },
        budget.control,
    )?;
    budget.charge(req.staged_bytes, req.work_units)?;
    if candidate.dab_buf.is_empty() && candidate.dual_buf.is_empty() {
        return Ok(None);
    }
    let mut count = 0usize;
    for (dab, dual) in candidate.dab_buf.iter().map(|d| (d, false)).chain(candidate.dual_buf.iter().map(|d| (d, true))) {
        budget.charge(0, 1)?;
        count = count.checked_add(rect_keys(candidate.ctx.dab_rect(dab, dual))?).filter(|n| *n <= MAX_KEYS).ok_or(StrokeError::Limit)?;
    }
    let mut keys = budget.set(count)?;
    for (dab, dual) in candidate.dab_buf.iter().map(|d| (d, false)).chain(candidate.dual_buf.iter().map(|d| (d, true))) {
        add_rect(&mut keys, candidate.ctx.dab_rect(dab, dual), budget)?;
    }
    // Same subset as upstream; full private clone avoids borrowed tile mutation.
    for map in std::iter::once(&mut candidate.cov).chain(candidate.dual.iter_mut()) {
        budget.charge(0, mul(map.tiles.capacity() as u64, 2)?)?;
        let mut cancelled = false;
        map.tiles.retain(|key, _| {
            if budget.control.cancelled() {
                cancelled = true;
                false
            } else {
                keys.contains(key)
            }
        });
        if cancelled {
            return Err(StrokeError::Cancelled);
        }
        map.dirty.clear();
    }
    candidate.pending_admission(0, 0, budget.limits(), budget.control)?;
    let before = candidate.incremental_clone_requirements(budget.limits(), budget.control)?.retained_bytes;
    let raster_work = pending_work(&candidate, budget.control)?;
    let raster_limits = StrokeLimits { resident_bytes: budget.bytes, work_units: raster_work };
    budget.charge(0, raster_work)?;
    candidate.raster_pending_with_interrupt(raster_limits, budget.control)?;
    // All subset coverage is dirty, including unchanged base pixels in the tail.
    let count = candidate.cov.tiles.len();
    let mut dirty = budget.set(count)?;
    for key in candidate.cov.tiles.keys() {
        budget.insert(&mut dirty, *key)?;
    }
    candidate.cov.dirty = dirty;
    let after = candidate.incremental_clone_requirements(budget.limits(), budget.control)?;
    budget.charge(after.retained_bytes.saturating_sub(before), after.work_units)?;
    poll(budget.control)?;
    Ok(Some(candidate))
}
fn copy_tile(source: &CovTile, budget: &mut Budget<'_, '_>) -> Result<CovTile> {
    Ok(CovTile { cov: budget.copy(&source.cov)?, depth: budget.copy(&source.depth)?, col: budget.copy(&source.col)? })
}
fn subset_merge(
    to: &mut CoverageMap,
    source: &CoverageMap,
    tail: Option<&CoverageMap>,
    keys: &[(i32, i32)],
    budget: &mut Budget<'_, '_>,
    first: bool,
) -> Result<()> {
    if to.nc != source.nc {
        return Err(StrokeError::InvalidInput("union coverage channels"));
    }
    to.bounds = if first { tail.map_or(source.bounds, |t| t.bounds) } else { to.bounds.union(&tail.map_or(source.bounds, |t| t.bounds)) };
    for key in keys {
        budget.charge(0, 1)?;
        let Some(tile) = tail.and_then(|t| t.tiles.get(key)).or_else(|| source.tiles.get(key)) else {
            continue;
        };
        if let Some(dst) = to.tiles.get_mut(key) {
            if dst.depth.is_empty() {
                dst.depth = budget.copy(&tile.depth)?;
            } else {
                for (a, b) in dst.depth.iter_mut().zip(&tile.depth) {
                    budget.charge(0, 1)?;
                    *a = a.max(*b);
                }
            }
            for (index, (a, b)) in dst.cov.iter_mut().zip(&tile.cov).enumerate() {
                budget.charge(0, 1)?;
                if *b > *a {
                    *a = *b;
                    let start = index.checked_mul(to.nc).ok_or(StrokeError::Limit)?;
                    let end = start.checked_add(to.nc).ok_or(StrokeError::Limit)?;
                    if let (Some(a), Some(b)) = (dst.col.get_mut(start..end), tile.col.get(start..end)) {
                        for (a, b) in a.iter_mut().zip(b) {
                            budget.charge(0, 1)?;
                            *a = *b;
                        }
                    }
                }
            }
        } else {
            if to.tiles.len() >= to.tiles.capacity() {
                return Err(StrokeError::Limit);
            }
            to.tiles.insert(*key, copy_tile(tile, budget)?);
        }
    }
    Ok(())
}
fn empty_map(nc: usize, count: usize, budget: &mut Budget<'_, '_>) -> Result<CoverageMap> {
    if count > MAX_KEYS || mul(count as u64, 512)? > budget.bytes {
        return Err(StrokeError::Limit);
    }
    let mut map = CoverageMap::new(nc);
    map.tiles.try_reserve(count).map_err(|_| StrokeError::Allocation)?;
    budget.charge(mul(map.tiles.capacity() as u64, 256)?, count as u64)?;
    Ok(map)
}
impl StrokeRenderer {
    pub fn try_tail_preview_with_interrupt(&self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<Option<Self>> {
        poll(control)?;
        let req = self.incremental_clone_requirements(limits, control)?;
        let standard = StrokeLimits::default();
        if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
            return Err(StrokeError::Limit);
        }
        let mut budget = Budget { bytes: limits.resident_bytes.checked_sub(req.retained_bytes).ok_or(StrokeError::Limit)?, work: limits.work_units, control };
        tail(self, &mut budget)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn try_composite_union_live_with_interrupt(
        passes: &mut [&mut Self],
        live: &mut UnionLive,
        full: bool,
        pre: &Surface,
        target: &mut Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<Rect> {
        poll(control)?;
        if passes.len() > 4 {
            return Err(StrokeError::Limit);
        }
        if passes.is_empty() {
            return Ok(Rect::EMPTY);
        }
        let standard = StrokeLimits::default();
        if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
            return Err(StrokeError::Limit);
        }
        if pre.format() != target.format() || !matches!(target.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
            return Err(StrokeError::Unsupported("ordinary union live format"));
        }
        let mut retained = mul(live.tail_keys.capacity() as u64, 64)?;
        let mut initial_work = live.tail_keys.capacity() as u64;
        for pass in passes.iter() {
            let req = pass.incremental_clone_requirements(limits, control)?;
            retained = add(retained, req.retained_bytes)?;
            initial_work = add(initial_work, req.work_units)?;
        }
        // Surfaces are retained here; the keyed compositor stages native maps.
        for surface in [Some(pre), Some(&*target), selection].into_iter().flatten() {
            retained = add(retained, super::incremental_composite_controlled::retained(surface, limits.resident_bytes, control)?)?;
        }
        let mut budget = Budget {
            bytes: limits.resident_bytes.checked_sub(retained).ok_or(StrokeError::Limit)?,
            work: limits.work_units.checked_sub(initial_work).ok_or(StrokeError::Limit)?,
            control,
        };
        let mut tails = budget.vector(passes.len())?;
        for pass in passes.iter() {
            tails.push(tail(pass, &mut budget)?);
        }
        let mut prospective = live.tail_keys.len();
        let mut tail_count = 0usize;
        for (pass, tail) in passes.iter().zip(&tails) {
            prospective = prospective
                .checked_add(pass.cov.dirty.len())
                .and_then(|n| n.checked_add(pass.dual.as_ref().map_or(0, |d| d.dirty.len())))
                .ok_or(StrokeError::Limit)?;
            if full {
                prospective = prospective
                    .checked_add(pass.cov.tiles.len())
                    .and_then(|n| n.checked_add(pass.dual.as_ref().map_or(0, |d| d.tiles.len())))
                    .ok_or(StrokeError::Limit)?;
            }
            if let Some(t) = tail {
                tail_count = tail_count
                    .checked_add(t.cov.tiles.len())
                    .and_then(|n| n.checked_add(t.dual.as_ref().map_or(0, |d| d.tiles.len())))
                    .ok_or(StrokeError::Limit)?;
            }
        }
        let mut keys = budget.set(prospective.checked_add(tail_count).ok_or(StrokeError::Limit)?)?;
        let mut tail_keys = budget.set(tail_count)?;
        for key in &live.tail_keys {
            budget.insert(&mut keys, *key)?;
        }
        for pass in passes.iter() {
            for key in &pass.cov.dirty {
                budget.insert(&mut keys, *key)?;
            }
            if let Some(d) = &pass.dual {
                for key in &d.dirty {
                    budget.insert(&mut keys, *key)?;
                }
            }
            if full {
                for key in pass.cov.tiles.keys() {
                    budget.insert(&mut keys, *key)?;
                }
                if let Some(d) = &pass.dual {
                    for key in d.tiles.keys() {
                        budget.insert(&mut keys, *key)?;
                    }
                }
            }
        }
        for t in tails.iter().flatten() {
            for key in t.cov.tiles.keys() {
                budget.insert(&mut tail_keys, *key)?;
                budget.insert(&mut keys, *key)?;
            }
            if let Some(d) = &t.dual {
                for key in d.tiles.keys() {
                    budget.insert(&mut tail_keys, *key)?;
                    budget.insert(&mut keys, *key)?;
                }
            }
        }
        let keys = sorted(&keys, &mut budget)?;
        let first = passes.first().ok_or(StrokeError::InvalidInput("missing union pass"))?;
        let mut cov = empty_map(first.cov.nc, keys.len(), &mut budget)?;
        let mut dual = if first.dual.is_some() { Some(empty_map(0, keys.len(), &mut budget)?) } else { None };
        for (index, (pass, tail)) in passes.iter().zip(&tails).enumerate() {
            subset_merge(&mut cov, &pass.cov, tail.as_ref().map(|t| &t.cov), &keys, &mut budget, index == 0)?;
            if let (Some(to), Some(from)) = (dual.as_mut(), pass.dual.as_ref()) {
                subset_merge(to, from, tail.as_ref().and_then(|t| t.dual.as_ref()), &keys, &mut budget, index == 0)?;
            }
        }
        let after = cov.bounds;
        let mut damage = Rect::EMPTY;
        for &(tx, ty) in &keys {
            budget.charge(0, 1)?;
            let tile = tile_rect(tx, ty)?;
            damage = damage.union(&tile.intersect(&live.bounds));
            if cov.tiles.contains_key(&(tx, ty)) {
                damage = damage.union(&tile.intersect(&after));
            }
        }
        let output = controlled::composite_existing_with_restore_with_interrupt(
            &first.ctx,
            &cov,
            dual.as_ref(),
            &keys,
            live.bounds,
            pre,
            target,
            selection,
            lock_transparency,
            budget.limits(),
            control,
        )?;
        #[cfg(test)]
        controlled::publication_test::arrive();
        poll(control)?;
        *target = output;
        live.tail_keys = tail_keys;
        live.bounds = after;
        for pass in passes.iter_mut() {
            pass.cov.dirty.clear();
            if let Some(dual) = pass.dual.as_mut() {
                dual.dirty.clear();
            }
        }
        Ok(damage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tails_and_incremental_distinct_union_match_upstream_across_frames() {
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8] {
            let brush = BrushSettings {
                size: 5.0,
                pressure_size: false,
                flow: 0.4,
                opacity: 0.6,
                wet_edges: true,
                smoothing: crate::brush::Smoothing { amount: 0.6, catch_up: true, catch_up_on_end: true, ..Default::default() },
                ..Default::default()
            };
            let pre = Surface::new(format);
            let mut a = pre.clone();
            let mut b = pre.clone();
            let mut old_live = UnionLive::default();
            let mut new_live = UnionLive::default();
            let mut old = [StrokeRenderer::new(&brush, Some(format), 1.0), StrokeRenderer::new(&brush, Some(format), 1.0)];
            let mut new = [
                old[0].try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap(),
                old[1].try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap(),
            ];
            for frame in 0..3 {
                let p = StrokePoint { x: frame as f64 * 4.0 + 2.0, y: 3.0 + frame as f64, time: frame as f64 * 16.0, ..Default::default() };
                let mirror = StrokePoint { x: 12.0 - p.x, ..p };
                old[0].push(&[p]);
                old[1].push(&[mirror]);
                new[0].push(&[p]);
                new[1].push(&[mirror]);
                let mut oa = old.iter_mut().collect::<Vec<_>>();
                let mut nb = new.iter_mut().collect::<Vec<_>>();
                let da = StrokeRenderer::composite_union_live(&mut oa, &mut old_live, frame == 0, &pre, &mut a, None, false);
                let db = StrokeRenderer::try_composite_union_live_with_interrupt(
                    &mut nb,
                    &mut new_live,
                    frame == 0,
                    &pre,
                    &mut b,
                    None,
                    false,
                    StrokeLimits::default(),
                    &Interrupt::NONE,
                )
                .unwrap();
                assert_eq!(da, db);
                assert_eq!(a, b);
                assert_eq!(old_live.bounds, new_live.bounds);
                assert_eq!(old_live.tail_keys, new_live.tail_keys);
            }
            let baseline = old[0].tail_preview();
            let current = new[0].try_tail_preview_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(baseline.is_some(), current.is_some());
            if let (Some(mut baseline), Some(mut current)) = (baseline, current) {
                let mut a = pre.clone();
                let mut b = pre.clone();
                baseline.composite(&pre, &mut a, None, false, true);
                current.composite(&pre, &mut b, None, false, true);
                assert_eq!(a, b);
            }
        }
    }
    #[test]
    fn late_union_refusal_preserves_target_live_and_dirty_passes() {
        let brush = BrushSettings { size: 9.0, pressure_size: false, ..Default::default() };
        let format = PixelFormat::RGBA32F;
        let pre = Surface::new(format);
        let mut target = pre.clone();
        let mut pass = StrokeRenderer::new(&brush, Some(format), 1.0);
        pass.push(&[StrokePoint::new(2.0, 3.0, 1.0)]);
        let dirty = pass.cov.dirty.clone();
        let mut live = UnionLive::default();
        let fence = controlled::publication_test::Fence::arm();
        let stop = || fence.cancelled();
        assert!(matches!(
            StrokeRenderer::try_composite_union_live_with_interrupt(
                &mut [&mut pass],
                &mut live,
                true,
                &pre,
                &mut target,
                None,
                false,
                StrokeLimits::default(),
                &Interrupt::cancel_only(&stop)
            ),
            Err(StrokeError::Cancelled)
        ));
        assert!(fence.cancelled());
        assert_eq!(target, pre);
        assert_eq!(live.bounds, Rect::EMPTY);
        assert!(live.tail_keys.is_empty());
        assert_eq!(pass.cov.dirty, dirty);
    }
}
