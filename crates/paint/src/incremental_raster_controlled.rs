//! Controlled pending rasterization. Each bounded row span uses the unchanged
//! upstream tip equations and CoverageMap accumulator. Hash-table internals
//! remain standard-library managed; all explicit vector/table growth is reserved.
use super::*;
use photocraft_raster::Interrupt;
type Result<T> = std::result::Result<T, StrokeError>;
struct Budget<'a, 'b> {
    control: &'a Interrupt<'b>,
    bytes: u64,
    work: u64,
}
impl Budget<'_, '_> {
    fn poll(&self) -> Result<()> {
        self.control.check().map_err(|_| StrokeError::Cancelled)
    }
    fn work(&mut self, n: u64) -> Result<()> {
        self.poll()?;
        self.work = self.work.checked_sub(n).ok_or(StrokeError::Limit)?;
        Ok(())
    }
    fn admit(&self, n: u64) -> Result<()> {
        self.poll()?;
        if n > self.bytes { Err(StrokeError::Limit) } else { Ok(()) }
    }
    fn charge(&mut self, n: u64) -> Result<()> {
        self.poll()?;
        self.bytes = self.bytes.checked_sub(n).ok_or(StrokeError::Limit)?;
        Ok(())
    }
    fn reserve<T>(&mut self, v: &mut Vec<T>, length: usize) -> Result<()> {
        let old = v.capacity();
        let width = std::mem::size_of::<T>() as u64;
        self.admit((length.saturating_sub(old) as u64).checked_mul(width).ok_or(StrokeError::Limit)?)?;
        if length > old {
            v.try_reserve_exact(length.saturating_sub(v.len())).map_err(|_| StrokeError::Allocation)?;
        }
        self.charge((v.capacity().saturating_sub(old) as u64).checked_mul(width).ok_or(StrokeError::Limit)?)
    }
    fn zero(&mut self, v: &mut Vec<f32>, count: usize) -> Result<()> {
        self.reserve(v, count)?;
        v.resize(count, 0.0);
        Ok(())
    }
    fn tile(&mut self, map: &mut CoverageMap, key: (i32, i32), color: bool, depth: bool) -> Result<()> {
        self.poll()?;
        let count = (COV_TILE * COV_TILE) as usize;
        if !map.tiles.contains_key(&key) {
            let old = map.tiles.capacity();
            if map.tiles.len() == old {
                self.admit((old as u64).checked_add(8).and_then(|n| n.checked_mul(256)).ok_or(StrokeError::Limit)?)?;
            }
            map.tiles.try_reserve(1).map_err(|_| StrokeError::Allocation)?;
            self.charge((map.tiles.capacity().saturating_sub(old) as u64).checked_mul(256).ok_or(StrokeError::Limit)?)?;
            let mut tile = CovTile { cov: Vec::new(), depth: Vec::new(), col: Vec::new() };
            self.zero(&mut tile.cov, count)?;
            map.tiles.insert(key, tile);
        }
        let tile = map.tiles.get_mut(&key).ok_or(StrokeError::Limit)?;
        if color && tile.col.is_empty() {
            self.zero(&mut tile.col, count.checked_mul(map.nc).ok_or(StrokeError::Limit)?)?;
        }
        if depth && tile.depth.is_empty() {
            self.zero(&mut tile.depth, count)?;
        }
        if !map.dirty.contains(&key) {
            let old = map.dirty.capacity();
            if map.dirty.len() == old {
                self.admit((old as u64).checked_add(8).and_then(|n| n.checked_mul(64)).ok_or(StrokeError::Limit)?)?;
            }
            map.dirty.try_reserve(1).map_err(|_| StrokeError::Allocation)?;
            self.charge((map.dirty.capacity().saturating_sub(old) as u64).checked_mul(64).ok_or(StrokeError::Limit)?)?;
        }
        Ok(())
    }
}
impl StrokeRenderer {
    // Only the private transaction candidate invokes this mutating helper.
    pub(super) fn raster_pending_with_interrupt(&mut self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<()> {
        let mut budget = Budget { control, bytes: limits.resident_bytes, work: limits.work_units };
        budget.poll()?;
        let mut dabs = std::mem::take(&mut self.dab_buf);
        let mut duals = std::mem::take(&mut self.dual_buf);
        let wet = self.ctx.brush.wet_edges;
        let mut native = [0.0f32; 8];
        if !dabs.is_empty() || (!duals.is_empty() && self.dual.is_some()) {
            budget.reserve(&mut self.scratch, 256)?;
            if wet {
                budget.reserve(&mut self.depth_scratch, 256)?;
            }
        }
        for (pending, dual) in [(&dabs, false), (&duals, true)] {
            if dual && self.dual.is_none() {
                continue;
            }
            for dab in pending {
                budget.poll()?;
                let rect = self.ctx.dab_rect(dab, dual);
                photocraft_raster::checked_region_byte_len(rect, PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false))
                    .map_err(|_| StrokeError::Limit)?;
                let col = match (!dual && self.per_dab_color, self.fmt) {
                    (true, Some(f)) => {
                        from_rgba_into(&f, dab.color, &mut native);
                        Some(native.get(..f.mode.color_channels()).ok_or(StrokeError::Limit)?)
                    }
                    _ => None,
                };
                for y in rect.y0..rect.y1 {
                    budget.poll()?;
                    let mut x = rect.x0;
                    while x < rect.x1 {
                        budget.work(1)?;
                        let end = i64::from(x).checked_add(256).ok_or(StrokeError::Limit)?.min(i64::from(rect.x1));
                        let tile_end = (i64::from(x.div_euclid(COV_TILE)) + 1) * i64::from(COV_TILE);
                        let end = i32::try_from(end.min(tile_end)).map_err(|_| StrokeError::Limit)?;
                        let next_y = y.checked_add(1).ok_or(StrokeError::Limit)?;
                        let span = Rect::new(x, y, end, next_y);
                        budget.work((end - x) as u64 * 2048)?;
                        self.ctx.rasterize_into(dab, dual, span, &mut self.scratch, (!dual && wet).then_some(&mut self.depth_scratch));
                        budget.poll()?;
                        let map = if dual { self.dual.as_mut().ok_or(StrokeError::Limit)? } else { &mut self.cov };
                        for tx in x.div_euclid(COV_TILE)..=end.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE) {
                            budget.poll()?;
                            let ty = y.div_euclid(COV_TILE);
                            let lo = i64::from(tx) * i64::from(COV_TILE);
                            let hi = lo + i64::from(COV_TILE);
                            let a = i64::from(x).max(lo) - i64::from(x);
                            let b = i64::from(end).min(hi) - i64::from(x);
                            let values = self
                                .scratch
                                .get(usize::try_from(a).map_err(|_| StrokeError::Limit)?..usize::try_from(b).map_err(|_| StrokeError::Limit)?)
                                .ok_or(StrokeError::Limit)?;
                            if values.iter().any(|&v| v > 0.0) {
                                let want_color = col.is_some() && map.nc > 0;
                                budget.tile(map, (tx, ty), want_color, !dual && wet)?;
                            }
                        }
                        map.accumulate(span, &self.scratch, if dual { 1.0 } else { dab.opacity }, col, (!dual && wet).then_some(self.depth_scratch.as_slice()));
                        budget.poll()?;
                        x = end;
                    }
                }
                // The original primary damage includes transparent portions too.
                if !dual {
                    self.cov.bounds = self.cov.bounds.union(&rect);
                }
            }
        }
        self.dabs_done = self.dabs_done.checked_add(dabs.len()).ok_or(StrokeError::Limit)?;
        if let Some(all) = self.all_dabs.as_mut() {
            let total = all.len().checked_add(dabs.len()).ok_or(StrokeError::Limit)?;
            budget.reserve(all, total)?;
            for chunk in dabs.chunks(256) {
                budget.work(chunk.len() as u64)?;
                all.extend_from_slice(chunk);
            }
        }
        dabs.clear();
        duals.clear();
        self.dab_buf = dabs;
        self.dual_buf = duals;
        budget.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_spans_preserve_full_dab_wet_dual_texture_and_color_accumulation() {
        for wet in [false, true] {
            let mut brush = BrushSettings { size: 300.0, pressure_size: false, flow: 0.3, opacity: 0.7, wet_edges: wet, ..Default::default() };
            brush.dual_brush.enabled = true;
            brush.dual_brush.size = 71.0;
            brush.texture.enabled = true;
            brush.texture.each_tip = true;
            brush.color_dynamics.enabled = true;
            let mut old = StrokeRenderer::new(&brush, Some(PixelFormat::RGBA32F), 1.0).record_dabs();
            let points = [crate::StrokePoint { x: -130.0, y: 17.0, ..Default::default() }, crate::StrokePoint { x: 170.0, y: 19.0, ..Default::default() }];
            old.generator.push(&points, &mut old.dab_buf, &mut old.dual_buf);
            old.generator.finish(&mut old.dab_buf, &mut old.dual_buf);
            let mut checked = old.try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
            old.raster_pending();
            checked.raster_pending_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(old.dabs(), checked.dabs());
            assert_eq!(old.dab_count(), checked.dab_count());
            assert_eq!(old.bounds(), checked.bounds());
            for (key, tile) in &old.cov.tiles {
                let actual = checked.cov.tiles.get(key).unwrap();
                assert_eq!(tile.cov, actual.cov);
                assert_eq!(tile.depth, actual.depth);
                assert_eq!(tile.col, actual.col);
            }
            assert_eq!(old.cov.tiles.len(), checked.cov.tiles.len());
            let a = old.dual.as_ref().unwrap();
            let b = checked.dual.as_ref().unwrap();
            assert_eq!(a.bounds, b.bounds);
            assert_eq!(a.tiles.len(), b.tiles.len());
            for (key, tile) in &a.tiles {
                assert_eq!(tile.cov, b.tiles.get(key).unwrap().cov);
            }
        }
    }
    #[test]
    fn late_pending_cancel_does_not_publish_renderer_candidate() {
        let brush = BrushSettings { size: 80.0, pressure_size: false, ..Default::default() };
        let mut actual = StrokeRenderer::new(&brush, Some(PixelFormat::RGBA32F), 1.0);
        let first = [crate::StrokePoint { x: 0.0, y: 0.0, ..Default::default() }];
        actual.push(&first);
        let before = actual.try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            let n = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            n > 150
        };
        let next = [crate::StrokePoint { x: 30.0, y: 0.0, ..Default::default() }];
        assert!(matches!(actual.push_with_interrupt(&next, StrokeLimits::default(), &Interrupt::cancel_only(&stop)), Err(StrokeError::Cancelled)));
        assert_eq!(actual.dab_count(), before.dab_count());
        assert_eq!(actual.bounds(), before.bounds());
        assert_eq!(actual.smoothing_string(), before.smoothing_string());
        assert_eq!(actual.cov.tiles.len(), before.cov.tiles.len());
    }
}
