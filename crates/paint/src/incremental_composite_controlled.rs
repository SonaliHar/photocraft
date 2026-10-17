//! Controlled ordinary live compositing; renderer dirty sets and target publish
//! together only after the private native output passes its final token fence.
use super::*;
use photocraft_raster::Interrupt;
use std::mem::size_of;
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
fn raster<T>(value: std::result::Result<T, photocraft_raster::RasterInputError>) -> Result<T> {
    value.map_err(|error| match error {
        photocraft_raster::RasterInputError::Cancelled => StrokeError::Cancelled,
        photocraft_raster::RasterInputError::AllocationFailed => StrokeError::Allocation,
        photocraft_raster::RasterInputError::LimitExceeded => StrokeError::Limit,
        _ => StrokeError::Raster,
    })
}
pub(super) fn retained(surface: &Surface, limit: u64, control: &Interrupt<'_>) -> Result<u64> {
    poll(control)?;
    let mut bytes = add(surface.default_bytes().len() as u64, mul(surface.tile_count() as u64, 256)?)?;
    if bytes > limit {
        return Err(StrokeError::Limit);
    }
    raster(surface.try_validate_tile_origins_with(control))?;
    for (_, tile) in surface.tiles() {
        poll(control)?;
        bytes = add(bytes, tile.bytes().len() as u64)?;
        if bytes > limit {
            return Err(StrokeError::Limit);
        }
    }
    Ok(bytes)
}
fn sort(keys: &mut [(i32, i32)], control: &Interrupt<'_>) -> Result<()> {
    fn sift(keys: &mut [(i32, i32)], mut root: usize, end: usize, control: &Interrupt<'_>) -> Result<()> {
        loop {
            poll(control)?;
            let child = root.checked_mul(2).and_then(|v| v.checked_add(1)).ok_or(StrokeError::Limit)?;
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
        sift(keys, root, keys.len(), control)?;
    }
    for end in (1..keys.len()).rev() {
        poll(control)?;
        keys.swap(0, end);
        sift(keys, 0, end, control)?;
    }
    Ok(())
}
impl StrokeRenderer {
    #[allow(clippy::too_many_arguments)]
    pub fn try_composite_with_interrupt(
        &mut self,
        pre: &Surface,
        target: &mut Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
        all: bool,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<Rect> {
        poll(control)?;
        let standard = StrokeLimits::default();
        let format = target.format();
        if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
            return Err(StrokeError::Limit);
        }
        if pre.format() != format
            || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
            || format.channels() == 0
            || format.channels() > 8
        {
            return Err(StrokeError::Unsupported("ordinary live compositing format"));
        }
        if selection.is_some_and(|s| !matches!(s.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) || s.channels() == 0 || s.channels() > 8)
        {
            return Err(StrokeError::Unsupported("ordinary live selection format"));
        }
        let count = if all { self.cov.tiles.len() } else { self.cov.dirty.len() };
        let capacity = if all { self.cov.tiles.capacity() } else { self.cov.dirty.capacity() };
        // Reject prospective work/capacity before enumerating any hash buckets.
        let keys_bytes = mul(count as u64, size_of::<(i32, i32)>() as u64)?;
        let key_work = add(capacity as u64, mul(count as u64, 128)?)?;
        if keys_bytes > limits.resident_bytes || key_work > limits.work_units {
            return Err(StrokeError::Limit);
        }
        let painter = self.incremental_clone_requirements(limits, control)?;
        let mut held = painter.retained_bytes;
        for surface in [Some(pre), Some(&*target), selection].into_iter().flatten() {
            held = add(held, retained(surface, limits.resident_bytes, control)?)?;
            if held > limits.resident_bytes {
                return Err(StrokeError::Limit);
            }
        }
        let native_tile = mul((photocraft_geom::TILE_SIZE * photocraft_geom::TILE_SIZE) as u64, format.bytes_per_pixel() as u64)?;
        // One64-grid region can intersect four256-grid native tiles. Retain all
        // staged payload/map growth plus complete native read/encode scratch.
        let output =
            add(mul(count as u64, mul(add(native_tile, 512)?, 4)?)?, add(mul(target.tile_count() as u64, 512)?, target.default_bytes().len() as u64)?)?;
        let sample_width = add(
            add(format.bytes_per_pixel() as u64, mul(format.channels() as u64, 4)?)?,
            selection.map_or(0, |s| s.format().bytes_per_pixel() as u64 + s.channels() as u64 * 4),
        )?;
        let scratch = mul((COV_TILE * COV_TILE) as u64, sample_width)?;
        let staged = add(keys_bytes, add(output, scratch)?)?;
        let map_work = mul(mul(count as u64, add(target.tile_count() as u64, mul(count as u64, 4)?)?)?, 256)?;
        let pixel_work = mul(mul(count as u64, (COV_TILE * COV_TILE) as u64)?, 2048)?;
        let work = add(add(painter.work_units, key_work)?, add(map_work, pixel_work)?)?;
        if add(held, staged)? > limits.resident_bytes || work > limits.work_units {
            return Err(StrokeError::Limit);
        }
        let mut keys = Vec::new();
        keys.try_reserve_exact(count).map_err(|_| StrokeError::Allocation)?;
        let actual_keys = mul(keys.capacity() as u64, size_of::<(i32, i32)>() as u64)?;
        if add(held, add(actual_keys, add(output, scratch)?)?)? > limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        if all {
            for key in self.cov.tiles.keys() {
                poll(control)?;
                keys.push(*key);
            }
        } else {
            for key in &self.cov.dirty {
                poll(control)?;
                keys.push(*key);
            }
        }
        sort(&mut keys, control)?;
        let bounds = self.bounds();
        let mut damage = Rect::EMPTY;
        for &(tx, ty) in &keys {
            poll(control)?;
            if !self.cov.tiles.contains_key(&(tx, ty)) {
                continue;
            }
            let x = tx.checked_mul(COV_TILE).ok_or(StrokeError::Limit)?;
            let y = ty.checked_mul(COV_TILE).ok_or(StrokeError::Limit)?;
            let rect =
                Rect::new(x, y, x.checked_add(COV_TILE).ok_or(StrokeError::Limit)?, y.checked_add(COV_TILE).ok_or(StrokeError::Limit)?).intersect(&bounds);
            damage = damage.union(&rect);
        }
        let remaining = StrokeLimits {
            resident_bytes: limits.resident_bytes.checked_sub(add(held, actual_keys)?).ok_or(StrokeError::Limit)?,
            work_units: limits.work_units.checked_sub(add(painter.work_units, key_work)?).ok_or(StrokeError::Limit)?,
        };
        let output = controlled::composite_existing_with_interrupt(
            &self.ctx,
            &self.cov,
            self.dual.as_ref(),
            &keys,
            pre,
            target,
            selection,
            lock_transparency,
            remaining,
            control,
        )?;
        poll(control)?;
        *target = output;
        self.cov.dirty.clear();
        if let Some(dual) = self.dual.as_mut() {
            dual.dirty.clear();
        }
        Ok(if all { bounds } else { damage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_dirty_and_full_composite_keep_native_legacy_words() {
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::CMYKA8] {
            for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Dissolve] {
                let brush = BrushSettings {
                    size: 7.0,
                    pressure_size: false,
                    flow: 0.3,
                    opacity: 0.6,
                    mode,
                    color: [0.2, 0.7, 0.4, 0.8],
                    wet_edges: true,
                    ..Default::default()
                };
                let pre = Surface::new(format);
                let mut old = StrokeRenderer::new(&brush, Some(format), 1.0);
                old.push(&[StrokePoint::new(-2.0, 3.0, 1.0), StrokePoint::new(8.0, 5.0, 0.7)]);
                old.finish();
                let mut checked = old.try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
                let mut a = pre.clone();
                let mut b = pre.clone();
                let da = old.composite(&pre, &mut a, None, false, false);
                let db = checked.try_composite_with_interrupt(&pre, &mut b, None, false, false, StrokeLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(da, db);
                assert_eq!(a, b);
                assert_eq!(old.cov.dirty, checked.cov.dirty);
                let da = old.composite(&pre, &mut a, None, false, true);
                let db = checked.try_composite_with_interrupt(&pre, &mut b, None, false, true, StrokeLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(da, db);
                assert_eq!(a, b);
            }
        }
    }
    #[test]
    fn late_composite_refusal_preserves_target_and_dirty_keys() {
        let brush = BrushSettings { size: 9.0, pressure_size: false, ..Default::default() };
        let format = PixelFormat::RGBA32F;
        let pre = Surface::new(format);
        let mut target = pre.clone();
        let mut renderer = StrokeRenderer::new(&brush, Some(format), 1.0);
        renderer.push(&[StrokePoint::new(2.0, 3.0, 1.0)]);
        renderer.finish();
        let dirty = renderer.cov.dirty.clone();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            polls.load(std::sync::atomic::Ordering::Relaxed) > 100
        };
        assert!(matches!(
            renderer.try_composite_with_interrupt(&pre, &mut target, None, false, false, StrokeLimits::default(), &Interrupt::cancel_only(&stop)),
            Err(StrokeError::Cancelled)
        ));
        assert_eq!(target, pre);
        assert_eq!(renderer.cov.dirty, dirty);
        assert!(
            renderer
                .try_composite_with_interrupt(&pre, &mut target, None, false, false, StrokeLimits { resident_bytes: 1, ..Default::default() }, &Interrupt::NONE)
                .is_err()
        );
        assert_eq!(target, pre);
        assert_eq!(renderer.cov.dirty, dirty);
    }
}
