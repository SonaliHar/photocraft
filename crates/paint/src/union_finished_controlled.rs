//! Finished-pass max-union without renderer/generator clones or smoothing tails.
//! Reuses the same ordered subset merge and checked ordinary compositor as live.
use super::*;
impl StrokeRenderer {
    #[allow(clippy::too_many_arguments)]
    pub fn try_composite_union_many_with_interrupt(
        &self,
        others: &[&Self],
        pre: &Surface,
        target: &mut Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<Rect> {
        poll(control)?;
        let standard = StrokeLimits::default();
        let format = target.format();
        if others.len() > 3 || limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
            return Err(StrokeError::Limit);
        }
        if pre.format() != format
            || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
            || format.channels() == 0
            || format.channels() > 8
        {
            return Err(StrokeError::Unsupported("ordinary finished union format"));
        }
        if selection.is_some_and(|s| !matches!(s.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) || s.channels() == 0 || s.channels() > 8)
        {
            return Err(StrokeError::Unsupported("ordinary finished union selection"));
        }
        let mut passes = [self; 4];
        for (index, pass) in others.iter().enumerate() {
            *passes.get_mut(index + 1).ok_or(StrokeError::Limit)? = *pass;
        }
        let count = others.len() + 1;
        let passes = passes.get(..count).ok_or(StrokeError::Limit)?;
        let mut prospective = 0usize;
        let mut retained = 0u64;
        let mut census_work = 0u64;
        for pass in passes {
            poll(control)?;
            prospective = prospective.checked_add(pass.cov.tiles.len()).filter(|n| *n <= MAX_KEYS).ok_or(StrokeError::Limit)?;
            let req = pass.incremental_clone_requirements(limits, control)?;
            retained = add(retained, req.retained_bytes)?;
            census_work = add(census_work, req.work_units)?;
        }
        for surface in [Some(pre), Some(&*target), selection].into_iter().flatten() {
            retained = add(retained, super::super::incremental_composite_controlled::retained(surface, limits.resident_bytes, control)?)?;
            if retained > limits.resident_bytes {
                return Err(StrokeError::Limit);
            }
        }
        // Pay every possible merged primary+dual tile, lookup/order work, and
        // native output/scratch before allocating any private union containers.
        let cells = (COV_TILE * COV_TILE) as u64;
        let merge_bytes = add(mul(mul(prospective as u64, cells)?, mul(add(self.cov.nc as u64, 3)?, 4)?)?, mul(prospective as u64, 1024)?)?;
        let native_tile = mul((photocraft_geom::TILE_SIZE * photocraft_geom::TILE_SIZE) as u64, format.bytes_per_pixel() as u64)?;
        let output =
            add(mul(prospective as u64, mul(add(native_tile, 512)?, 4)?)?, add(mul(target.tile_count() as u64, 512)?, target.default_bytes().len() as u64)?)?;
        let scratch = mul(
            cells,
            add(
                add(format.bytes_per_pixel() as u64, mul(format.channels() as u64, 4)?)?,
                selection.map_or(0, |s| s.format().bytes_per_pixel() as u64 + s.channels() as u64 * 4),
            )?,
        )?;
        let backend_work = add(
            mul(mul(prospective as u64, cells)?, 2048)?,
            mul(mul(prospective as u64, add(target.tile_count() as u64, mul(prospective as u64, 4)?)?)?, 256)?,
        )?;
        let merge_work = add(mul(prospective as u64, 256)?, mul(mul(prospective as u64, cells)?, mul(add(self.cov.nc as u64, 8)?, count as u64)?)?)?;
        if add(retained, add(merge_bytes, add(output, scratch)?)?)? > limits.resident_bytes
            || add(census_work, add(merge_work, backend_work)?)? > limits.work_units
        {
            return Err(StrokeError::Limit);
        }
        let mut budget = Budget {
            bytes: limits.resident_bytes.checked_sub(retained).ok_or(StrokeError::Limit)?,
            work: limits.work_units.checked_sub(census_work).ok_or(StrokeError::Limit)?,
            control,
        };
        let mut keys = budget.set(prospective)?;
        for pass in passes {
            budget.charge(0, pass.cov.tiles.capacity() as u64)?;
            for key in pass.cov.tiles.keys() {
                budget.insert(&mut keys, *key)?;
            }
        }
        let keys = sorted(&keys, &mut budget)?;
        let mut cov = empty_map(self.cov.nc, keys.len(), &mut budget)?;
        let mut dual = if self.dual.is_some() { Some(empty_map(0, keys.len(), &mut budget)?) } else { None };
        for (index, pass) in passes.iter().enumerate() {
            subset_merge(&mut cov, &pass.cov, None, &keys, &mut budget, index == 0)?;
            if let (Some(to), Some(from)) = (dual.as_mut(), pass.dual.as_ref()) {
                subset_merge(to, from, None, &keys, &mut budget, index == 0)?;
            }
        }
        if add(output, scratch)? > budget.bytes || backend_work > budget.work {
            return Err(StrokeError::Limit);
        }
        let bounds = cov.bounds;
        let staged = super::super::controlled::composite_existing_with_interrupt(
            &self.ctx,
            &cov,
            dual.as_ref(),
            &keys,
            pre,
            target,
            selection,
            lock_transparency,
            budget.limits(),
            control,
        )?;
        poll(control)?;
        *target = staged;
        Ok(bounds)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finished_passes_match_upstream_without_generating_another_tail() {
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8] {
            let brush = BrushSettings {
                size: 5.0,
                pressure_size: false,
                opacity: 0.5,
                flow: 0.4,
                wet_edges: true,
                smoothing: crate::brush::Smoothing { amount: 0.6, catch_up_on_end: true, ..Default::default() },
                ..Default::default()
            };
            let pre = Surface::new(format);
            let mut primary = StrokeRenderer::new(&brush, Some(format), 1.0);
            let mut mirror = StrokeRenderer::new(&brush, Some(format), 1.0);
            primary.push(&[StrokePoint::new(-2.0, 3.0, 1.0), StrokePoint::new(9.0, 4.0, 0.7)]);
            mirror.push(&[StrokePoint::new(12.0, 3.0, 1.0), StrokePoint::new(1.0, 4.0, 0.7)]);
            primary.finish();
            mirror.finish();
            let primary_dirty = primary.cov.dirty.clone();
            let mirror_dirty = mirror.cov.dirty.clone();
            let count = primary.dab_count();
            let mut expected = pre.clone();
            let mut actual = pre.clone();
            let a = primary.composite_union_many([&mirror, &mirror], &pre, &mut expected, None, false);
            let b = primary
                .try_composite_union_many_with_interrupt(&[&mirror, &mirror], &pre, &mut actual, None, false, StrokeLimits::default(), &Interrupt::NONE)
                .unwrap();
            assert_eq!(a, b);
            assert_eq!(expected, actual);
            assert_eq!(primary.dab_count(), count);
            assert_eq!(primary.cov.dirty, primary_dirty);
            assert_eq!(mirror.cov.dirty, mirror_dirty);
        }
    }
    #[test]
    fn finished_union_cancel_or_excess_passes_never_publish() {
        let format = PixelFormat::RGBA32F;
        let pre = Surface::new(format);
        let mut target = pre.clone();
        let mut renderer = StrokeRenderer::new(&BrushSettings::default(), Some(format), 1.0);
        renderer.push(&[StrokePoint::new(1.0, 2.0, 1.0)]);
        renderer.finish();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            polls.load(std::sync::atomic::Ordering::Relaxed) > 60
        };
        assert!(matches!(
            renderer.try_composite_union_many_with_interrupt(
                &[&renderer],
                &pre,
                &mut target,
                None,
                false,
                StrokeLimits::default(),
                &Interrupt::cancel_only(&stop)
            ),
            Err(StrokeError::Cancelled)
        ));
        assert_eq!(target, pre);
        assert!(
            renderer
                .try_composite_union_many_with_interrupt(
                    &[&renderer, &renderer, &renderer, &renderer],
                    &pre,
                    &mut target,
                    None,
                    false,
                    StrokeLimits::default(),
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert_eq!(target, pre);
    }
}
