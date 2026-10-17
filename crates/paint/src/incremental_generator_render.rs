//! Controlled incremental generation joined to the unchanged raster equations.
//! Pending rasterization uses bounded row spans with fallible buffer/table
//! reservations. Standard hash allocator internals remain library-managed.
use super::*;
use photocraft_raster::Interrupt;
type Result<T> = std::result::Result<T, StrokeError>;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| StrokeError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(StrokeError::Limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(StrokeError::Limit)
}
fn cells(bounds: Rect) -> Result<u64> {
    if bounds.is_empty() {
        return Ok(0);
    }
    let columns = i64::from(bounds.x1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE)) - i64::from(bounds.x0.div_euclid(COV_TILE)) + 1;
    let rows = i64::from(bounds.y1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE)) - i64::from(bounds.y0.div_euclid(COV_TILE)) + 1;
    mul(u64::try_from(columns).map_err(|_| StrokeError::Limit)?, u64::try_from(rows).map_err(|_| StrokeError::Limit)?)
}
impl StrokeRenderer {
    pub(super) fn pending_admission(&self, held: u64, used_work: u64, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        self.dabs_done.checked_add(self.dab_buf.len()).ok_or(StrokeError::Limit)?;
        let recorded = self.all_dabs.as_ref().map_or(0, Vec::len);
        if recorded.checked_add(self.dab_buf.len()).is_none_or(|n| n > 1_000_000) {
            return Err(StrokeError::Limit);
        }
        let mut bounds = Rect::EMPTY;
        let mut dual_bounds = Rect::EMPTY;
        let mut max_pixels = 0u64;
        let mut pixels = 0u64;
        for (dabs, dual) in [(&self.dab_buf, false), (&self.dual_buf, true)] {
            if dabs.len() > 1_000_000 {
                return Err(StrokeError::Limit);
            }
            for dab in dabs {
                poll(control)?;
                let rect = self.ctx.dab_rect(dab, dual);
                let bytes = photocraft_raster::checked_region_byte_len(rect, PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false))
                    .map_err(|_| StrokeError::Limit)? as u64;
                let area = bytes / 4;
                max_pixels = max_pixels.max(area);
                pixels = add(pixels, area)?;
                if dual {
                    dual_bounds = dual_bounds.union(&rect);
                } else {
                    bounds = bounds.union(&rect);
                }
            }
        }
        let primary_cells = cells(bounds)?;
        let dual_cells = cells(dual_bounds)?;
        let primary_tile = add(mul((COV_TILE * COV_TILE) as u64, mul((1 + usize::from(self.ctx.brush.wet_edges) + self.cov.nc) as u64, 4)?)?, 256)?;
        let dual_tile = add(mul((COV_TILE * COV_TILE) as u64, 4)?, 256)?;
        let staged = mul(
            add(
                add(mul(primary_cells, primary_tile)?, mul(dual_cells, dual_tile)?)?,
                add(mul(max_pixels, 8)?, mul(self.dab_buf.len() as u64, std::mem::size_of::<Dab>() as u64)?)?,
            )?,
            2,
        )?;
        let table_count = add(primary_cells, dual_cells)?;
        let old_maps = add(self.cov.tiles.capacity() as u64, self.dual.as_ref().map_or(0, |m| m.tiles.capacity()) as u64)?;
        let work = add(mul(pixels, 2049)?, mul(mul(table_count, add(old_maps, table_count)?)?, 64)?)?;
        if add(held, staged)? > limits.resident_bytes || add(used_work, work)? > limits.work_units {
            return Err(StrokeError::Limit);
        }
        poll(control)
    }
    fn generated_delta(&mut self, points: &[StrokePoint], finish: bool, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        let clone = self.incremental_clone_requirements(limits, control)?;
        let held = add(clone.retained_bytes, clone.staged_bytes)?;
        let remaining = StrokeLimits {
            resident_bytes: limits.resident_bytes.checked_sub(held).ok_or(StrokeError::Limit)?,
            work_units: limits.work_units.checked_sub(clone.work_units).ok_or(StrokeError::Limit)?,
        };
        let generated = if finish {
            self.generator.finish_requirements(&self.dab_buf, &self.dual_buf, remaining, control)?
        } else {
            self.generator.push_requirements(points, &self.dab_buf, &self.dual_buf, remaining, control)?
        };
        let mut candidate = self.try_clone_with_interrupt(limits, control)?;
        if finish {
            candidate.generator.finish_with_interrupt(&mut candidate.dab_buf, &mut candidate.dual_buf, remaining, control)?;
        } else {
            candidate.generator.push_with_interrupt(points, &mut candidate.dab_buf, &mut candidate.dual_buf, remaining, control)?;
        }
        candidate.pending_admission(
            add(held, add(generated.retained_bytes, generated.staged_bytes)?)?,
            add(clone.work_units, generated.work_units)?,
            limits,
            control,
        )?;
        poll(control)?;
        let raster_limits = StrokeLimits {
            resident_bytes: limits.resident_bytes.checked_sub(add(held, add(generated.retained_bytes, generated.staged_bytes)?)?).ok_or(StrokeError::Limit)?,
            work_units: limits.work_units.checked_sub(add(clone.work_units, generated.work_units)?).ok_or(StrokeError::Limit)?,
        };
        candidate.raster_pending_with_interrupt(raster_limits, control)?;
        poll(control)?;
        // Publication only after the complete generated/rasterized state passes
        // the same bounded retained-state census used by live transactions.
        candidate.incremental_clone_requirements(limits, control)?;
        poll(control)?;
        *self = candidate;
        Ok(())
    }
    pub fn push_with_interrupt(&mut self, points: &[StrokePoint], limits: StrokeLimits, control: &Interrupt<'_>) -> Result<()> {
        self.generated_delta(points, false, limits, control)
    }
    pub fn finish_with_interrupt(&mut self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<()> {
        self.generated_delta(&[], true, limits, control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_incremental_renderer_matches_legacy_across_chunk_and_finish() {
        let brush = BrushSettings {
            size: 3.0,
            pressure_size: false,
            flow: 0.4,
            opacity: 0.7,
            wet_edges: true,
            smoothing: crate::brush::Smoothing { amount: 0.5, ..Default::default() },
            ..Default::default()
        };
        let format = PixelFormat::RGBA32F;
        let mut old = StrokeRenderer::new(&brush, Some(format), 1.0);
        let mut checked = StrokeRenderer::new(&brush, Some(format), 1.0);
        let points = [
            StrokePoint { x: 2.0, y: 3.0, time: 0.0, ..Default::default() },
            StrokePoint { x: 5.0, y: 5.0, time: 16.0, ..Default::default() },
            StrokePoint { x: 8.0, y: 6.0, time: 48.0, ..Default::default() },
        ];
        for chunk in points.chunks(1) {
            old.push(chunk);
            checked.push_with_interrupt(chunk, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        }
        old.finish();
        checked.finish_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let pre = Surface::new(format);
        let mut a = pre.clone();
        let mut b = pre.clone();
        old.composite(&pre, &mut a, None, false, true);
        checked.composite(&pre, &mut b, None, false, true);
        assert_eq!(a, b);
        assert_eq!(old.dab_count(), checked.dab_count());
        let before = checked.try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let stop = || true;
        assert!(matches!(checked.push_with_interrupt(&points, StrokeLimits::default(), &Interrupt::cancel_only(&stop)), Err(StrokeError::Cancelled)));
        assert_eq!(checked.dab_count(), before.dab_count());
        assert_eq!(checked.smoothing_string(), before.smoothing_string());
    }
}
