//! Exact ordinary preview restoration without a whole-bounds float buffer.
//! Native target state remains private until restoration and the final poll pass.
use super::*;
use photocraft_color::{read_sample, write_sample};
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
    value.map_err(|e| match e {
        photocraft_raster::RasterInputError::Cancelled => StrokeError::Cancelled,
        photocraft_raster::RasterInputError::AllocationFailed => StrokeError::Allocation,
        photocraft_raster::RasterInputError::LimitExceeded => StrokeError::Limit,
        _ => StrokeError::Raster,
    })
}
fn span(rect: Rect, size: i32) -> Result<u64> {
    if rect.is_empty() {
        return Ok(0);
    }
    let last_x = rect.x1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(size);
    let last_y = rect.y1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(size);
    let w = u64::try_from(i64::from(last_x) - i64::from(rect.x0.div_euclid(size)) + 1).map_err(|_| StrokeError::Limit)?;
    let h = u64::try_from(i64::from(last_y) - i64::from(rect.y0.div_euclid(size)) + 1).map_err(|_| StrokeError::Limit)?;
    mul(w, h)
}
impl StrokeRenderer {
    /// Matches target.write_region(bounds,&pre.read_region(bounds)), including
    /// ordinary depth quantization. Does not mark or clear any renderer dirty set.
    pub fn try_restore_preview_with_interrupt(
        pre: &Surface,
        target: &mut Surface,
        bounds: Rect,
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
            return Err(StrokeError::Unsupported("ordinary preview restoration format"));
        }
        if bounds.is_empty() {
            return Ok(Rect::EMPTY);
        }
        let bytes = raster(photocraft_raster::checked_region_byte_len(bounds, format))? as u64;
        let regions = span(bounds, COV_TILE)?;
        let output_tiles = span(bounds, photocraft_geom::TILE_SIZE)?;
        let map_count = target.tile_count() as u64;
        let tile_bytes = mul((photocraft_geom::TILE_SIZE * photocraft_geom::TILE_SIZE) as u64, format.bytes_per_pixel() as u64)?;
        let output = add(mul(map_count, 512)?, add(target.default_bytes().len() as u64, mul(output_tiles, add(tile_bytes, 512)?)?)?)?;
        let scratch = mul((COV_TILE * COV_TILE) as u64, add(format.bytes_per_pixel() as u64, mul(format.channels() as u64, 4)?)?)?;
        let work = add(mul(bytes, 64)?, mul(mul(regions, add(map_count, add(output_tiles, regions)?)?)?, 256)?)?;
        if add(output, scratch)? > limits.resident_bytes || work > limits.work_units {
            return Err(StrokeError::Limit);
        }
        let held = add(
            super::incremental_composite_controlled::retained(pre, limits.resident_bytes, control)?,
            super::incremental_composite_controlled::retained(target, limits.resident_bytes, control)?,
        )?;
        if add(held, add(output, scratch)?)? > limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        // Shared immutable tile bytes stay shared; only sparse/default ownership
        // and changed native tiles are reserved. BTree node internals stay std.
        let mut candidate = raster(Surface::try_with_default_bytes(format, target.default_bytes()))?;
        for (position, tile) in target.tiles() {
            poll(control)?;
            candidate.put_tiles(std::iter::once((*position, Arc::clone(tile))));
        }
        let pixels = (COV_TILE * COV_TILE) as usize;
        let count = pixels.checked_mul(format.channels()).ok_or(StrokeError::Limit)?;
        let raw_count = pixels.checked_mul(format.bytes_per_pixel()).ok_or(StrokeError::Limit)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(raw_count).map_err(|_| StrokeError::Allocation)?;
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| StrokeError::Allocation)?;
        let actual = add(raw.capacity() as u64, mul(values.capacity() as u64, size_of::<f32>() as u64)?)?;
        if add(held, add(output, actual)?)? > limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        raw.resize(raw_count, 0);
        let first_x = bounds.x0.div_euclid(COV_TILE);
        let first_y = bounds.y0.div_euclid(COV_TILE);
        let last_x = bounds.x1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE);
        let last_y = bounds.y1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(COV_TILE);
        for ty in first_y..=last_y {
            for tx in first_x..=last_x {
                poll(control)?;
                let x = i64::from(tx) * i64::from(COV_TILE);
                let y = i64::from(ty) * i64::from(COV_TILE);
                let rect = Rect::new(
                    i32::try_from(x.max(i64::from(bounds.x0))).map_err(|_| StrokeError::Limit)?,
                    i32::try_from(y.max(i64::from(bounds.y0))).map_err(|_| StrokeError::Limit)?,
                    i32::try_from((x + i64::from(COV_TILE)).min(i64::from(bounds.x1))).map_err(|_| StrokeError::Limit)?,
                    i32::try_from((y + i64::from(COV_TILE)).min(i64::from(bounds.y1))).map_err(|_| StrokeError::Limit)?,
                );
                let length = raster(photocraft_raster::checked_region_byte_len(rect, format))?;
                let samples = length / format.sample.bytes();
                raster(pre.try_read_interleaved_into_with(rect, raw.get_mut(..length).ok_or(StrokeError::Limit)?, control))?;
                values.clear();
                for sample in 0..samples {
                    if sample.is_multiple_of(256) {
                        poll(control)?;
                    }
                    values.push(read_sample(raw.get(..length).ok_or(StrokeError::Limit)?, format.sample, sample));
                }
                for (sample, value) in values.iter().enumerate() {
                    if sample.is_multiple_of(256) {
                        poll(control)?;
                    }
                    write_sample(raw.get_mut(..length).ok_or(StrokeError::Limit)?, format.sample, sample, *value);
                }
                raster(candidate.try_write_interleaved_with(rect, raw.get(..length).ok_or(StrokeError::Limit)?, control))?;
            }
        }
        poll(control)?;
        *target = candidate;
        Ok(bounds)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_restoration_matches_ordinary_whole_region_across_tile_edges() {
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8] {
            let mut pre = Surface::new(format);
            let bounds = Rect::new(-3, -2, 131, 67);
            let mut source = vec![0.0; bounds.width() as usize * bounds.height() as usize * format.channels()];
            for (i, v) in source.iter_mut().enumerate() {
                *v = (i % 257) as f32 / 256.0;
            }
            pre.write_region(bounds, &source);
            let mut old = Surface::new(format);
            let mut current = old.clone();
            old.write_region(bounds, &pre.read_region(bounds));
            let damage = StrokeRenderer::try_restore_preview_with_interrupt(&pre, &mut current, bounds, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(damage, bounds);
            assert_eq!(old, current);
        }
    }
    #[test]
    fn late_restore_cancel_preserves_the_existing_target() {
        let format = PixelFormat::RGBA32F;
        let pre = Surface::new(format);
        let mut target = pre.clone();
        let before = target.clone();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            polls.load(std::sync::atomic::Ordering::Relaxed) > 40
        };
        assert!(matches!(
            StrokeRenderer::try_restore_preview_with_interrupt(
                &pre,
                &mut target,
                Rect::new(-3, -2, 131, 67),
                StrokeLimits::default(),
                &Interrupt::cancel_only(&stop)
            ),
            Err(StrokeError::Cancelled)
        ));
        assert_eq!(target, before);
    }
}
