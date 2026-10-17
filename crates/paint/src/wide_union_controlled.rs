//! Max-union of up to four upstream coverage passes with native wide painting.
//! Geometry stays the renderer's f32 coverage; native color/alpha equations are
//! delegated to checked normal/erase primitives or explicit native unit blends.
use super::*;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
use std::mem::size_of;
#[path = "wide_brush_match_controlled.rs"]
mod brush_match;

type Result<T> = std::result::Result<T, RasterInputError>;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn surface_bytes(surface: &Surface, limit: u64, control: &Interrupt<'_>) -> Result<u64> {
    poll(control)?;
    let mut bytes = add(surface.default_bytes().len() as u64, mul(surface.tile_count() as u64, 256)?)?;
    if bytes > limit {
        return Err(RasterInputError::LimitExceeded);
    }
    surface.try_validate_tile_origins_with(control)?;
    for (_, tile) in surface.tiles() {
        poll(control)?;
        bytes = add(bytes, tile.bytes().len() as u64)?;
        if bytes > limit {
            return Err(RasterInputError::LimitExceeded);
        }
    }
    Ok(bytes)
}
fn map_bytes(map: &CoverageMap, limit: u64, control: &Interrupt<'_>) -> Result<u64> {
    poll(control)?;
    let mut bytes = add(mul(map.tiles.capacity() as u64, 256)?, mul(map.dirty.capacity() as u64, 64)?)?;
    if bytes > limit || map.nc > 8 {
        return Err(RasterInputError::LimitExceeded);
    }
    let cells = (COV_TILE as usize).checked_mul(COV_TILE as usize).ok_or(RasterInputError::LimitExceeded)?;
    for (&(tx, ty), tile) in &map.tiles {
        poll(control)?;
        tx.checked_mul(COV_TILE).and_then(|v| v.checked_add(COV_TILE)).ok_or(RasterInputError::InvalidBounds)?;
        ty.checked_mul(COV_TILE).and_then(|v| v.checked_add(COV_TILE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.cov.len() != cells || (!tile.depth.is_empty() && tile.depth.len() != cells) {
            return Err(RasterInputError::LengthMismatch);
        }
        for values in [&tile.cov, &tile.depth, &tile.col] {
            bytes = add(bytes, mul(values.capacity() as u64, size_of::<f32>() as u64)?)?;
            if bytes > limit {
                return Err(RasterInputError::LimitExceeded);
            }
        }
    }
    Ok(bytes)
}
fn tiles_in(bounds: Rect) -> Result<u64> {
    if bounds.is_empty() {
        return Ok(0);
    }
    let first = photocraft_geom::TileCoord::containing(bounds.x0, bounds.y0);
    let last = photocraft_geom::TileCoord::containing(
        bounds.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
        bounds.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
    );
    let columns = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let rows = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    mul(columns, rows)
}
fn sparse_copy(source: &Surface, limit: u64, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let requested = mul(source.tile_count() as u64, (size_of::<(photocraft_geom::TileCoord, Arc<photocraft_raster::Tile>)>() + 256) as u64)?;
    if add(requested, source.default_bytes().len() as u64)? > limit {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut descriptors = Vec::new();
    descriptors.try_reserve_exact(source.tile_count()).map_err(|_| RasterInputError::AllocationFailed)?;
    if add(
        add(
            mul(descriptors.capacity() as u64, size_of::<(photocraft_geom::TileCoord, Arc<photocraft_raster::Tile>)>() as u64)?,
            mul(source.tile_count() as u64, 256)?,
        )?,
        source.default_bytes().len() as u64,
    )? > limit
    {
        return Err(RasterInputError::LimitExceeded);
    }
    for (coordinate, tile) in source.tiles() {
        poll(control)?;
        descriptors.push((*coordinate, Arc::clone(tile)));
    }
    let mut out = Surface::try_with_default_bytes(source.format(), source.default_bytes())?;
    // Standard BTreeMap nodes are bounded/prepaid, not individually fallible.
    for descriptor in descriptors {
        poll(control)?;
        out.put_tiles(std::iter::once(descriptor));
    }
    poll(control)?;
    Ok(out)
}
impl StrokeRenderer {
    /// `others` is in upstream mirror order, at most three borrowed passes.
    /// Coverage and wet depth are unioned independently before selection/opacity
    /// are applied once. Per-dab color and non-normal native modes stay closed.
    #[allow(clippy::too_many_arguments)]
    pub fn try_composite_union_many_wide(
        &self,
        others: &[&Self],
        pre: &Surface,
        target: &mut Surface,
        native_color: &Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
        clip: Option<Rect>,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<Rect> {
        self.try_composite_union_many_wide_with_blend(others, pre, target, native_color, selection, lock_transparency, clip, None, limits, control)
    }
    /// Caller explicitly selects a qualified native unit blend. This is not a
    /// source-operation/profile inference; None retains the existing Normal API.
    #[allow(clippy::too_many_arguments)]
    pub fn try_composite_union_many_wide_with_blend(
        &self,
        others: &[&Self],
        pre: &Surface,
        target: &mut Surface,
        native_color: &Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
        clip: Option<Rect>,
        blend: Option<crate::wide_blend_controlled::NativeUnitBlend>,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<Rect> {
        poll(control)?;
        let standard = StrokeLimits::default();
        let format = pre.format();
        if others.len() > 3 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > standard.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        let brush = &self.ctx.brush;
        let expected_mode = match blend {
            None => BlendMode::Normal,
            Some(crate::wide_blend_controlled::NativeUnitBlend::Multiply) => BlendMode::Multiply,
            Some(crate::wide_blend_controlled::NativeUnitBlend::Screen) => BlendMode::Screen,
        };
        if !matches!(format.sample, SampleType::U32 | SampleType::F64)
            || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || format != target.format()
            || format.mode != native_color.format().mode
            || format.alpha != native_color.format().alpha
            || self.fmt != Some(format)
            || self.per_dab_color
            || brush.mode != expected_mode
            || (blend.is_some() && brush.erase)
            || (brush.erase && (!format.alpha || lock_transparency))
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let mut passes = [self; 4];
        for (index, other) in others.iter().enumerate() {
            *passes.get_mut(index + 1).ok_or(RasterInputError::LimitExceeded)? = *other;
        }
        let count = others.len() + 1;
        let passes = passes.get(..count).ok_or(RasterInputError::LimitExceeded)?;
        let mut bounds = self.bounds();
        let mut coverage_tiles = 0u64;
        let mut capacities = 0u64;
        for pass in passes {
            bounds = bounds.union(&pass.bounds());
            coverage_tiles = add(coverage_tiles, pass.cov.tiles.len() as u64)?;
            capacities = add(capacities, mul(pass.cov.tiles.capacity() as u64, 256)?)?;
            if let Some(dual) = &pass.dual {
                capacities = add(capacities, mul(dual.tiles.capacity() as u64, 256)?)?;
            }
        }
        let bounds = clip.map_or(bounds, |clip| bounds.intersect(&clip));
        let output_bytes = photocraft_raster::checked_region_byte_len(bounds, format)? as u64;
        let output_tiles = tiles_in(bounds)?;
        let source_tiles = add(add(pre.tile_count() as u64, target.tile_count() as u64)?, native_color.tile_count() as u64)?;
        let source_tiles = add(source_tiles, selection.map_or(0, |selection| selection.tile_count() as u64))?;
        // Existing native tile application clones/validates growing sparse maps
        // once per disjoint 64-grid coverage tile. Charge that repeated scheduler
        // work, as well as all coverage/mask/native sample visits, before scans.
        let scheduler = mul(mul(coverage_tiles, add(source_tiles, add(output_tiles, coverage_tiles)?)?)?, 256)?;
        let sample_work = mul(mul(coverage_tiles, (COV_TILE * COV_TILE) as u64)?, add(mul(count as u64, 64)?, mul(format.channels() as u64, 256)?)?)?;
        let table_work = mul(mul(coverage_tiles, capacities / 256 + 1)?, count as u64)?;
        let work = add(add(scheduler, sample_work)?, table_work)?;
        if work > limits.work_units || output_bytes > 1_000_000_000 || capacities > limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut retained = capacities;
        for surface in [Some(pre), Some(&*target), Some(native_color), selection].into_iter().flatten() {
            retained = add(retained, surface_bytes(surface, limits.resident_bytes, control)?)?;
            if retained > limits.resident_bytes {
                return Err(RasterInputError::LimitExceeded);
            }
        }
        let mut admitted_work = work;
        for pass in passes {
            poll(control)?;
            // A bounded metadata census pays embedded/stored brush data before
            // equality scans. It does not clone or reconstruct the brush.
            let req = pass
                .incremental_clone_requirements(StrokeLimits { resident_bytes: limits.resident_bytes, work_units: limits.work_units }, control)
                .map_err(|error| match error {
                    StrokeError::Cancelled => RasterInputError::Cancelled,
                    _ => RasterInputError::LimitExceeded,
                })?;
            let comparison_work = add(req.work_units, 256)?;
            admitted_work = add(admitted_work, add(req.work_units, comparison_work)?)?;
            if admitted_work > limits.work_units {
                return Err(RasterInputError::LimitExceeded);
            }
            retained = add(retained, req.retained_bytes)?;
            if pass.per_dab_color || pass.fmt != self.fmt || !brush_match::equal(&pass.ctx.brush, brush, comparison_work, control)? {
                return Err(RasterInputError::UnsupportedConversion);
            }
            retained = add(retained, map_bytes(&pass.cov, limits.resident_bytes, control)?)?;
            if let Some(dual) = &pass.dual {
                retained = add(retained, map_bytes(dual, limits.resident_bytes, control)?)?;
            }
            if retained > limits.resident_bytes {
                return Err(RasterInputError::LimitExceeded);
            }
        }
        let scratch = mul((COV_TILE * COV_TILE) as u64, size_of::<f64>() as u64)?;
        let output_headroom = mul(output_tiles, add(mul((256 * 256) as u64, format.bytes_per_pixel() as u64)?, 256)?)?;
        let tile_scratch = mul((256 * 256) as u64, mul(format.bytes_per_pixel() as u64, 3)?)?;
        let copy_maps = mul(pre.tile_count() as u64, (size_of::<(photocraft_geom::TileCoord, Arc<photocraft_raster::Tile>)>() + 512) as u64)?;
        if add(retained, add(add(scratch, output_headroom)?, add(tile_scratch, copy_maps)?)?)? > limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        // One scratch allocation for the complete transaction, before any
        // output map staging; tile scans reuse it without growth.
        poll(control)?;
        let mut coverage = Vec::new();
        coverage.try_reserve_exact((COV_TILE * COV_TILE) as usize).map_err(|_| RasterInputError::AllocationFailed)?;
        let actual_scratch = mul(coverage.capacity() as u64, size_of::<f64>() as u64)?;
        if add(retained, add(add(actual_scratch, output_headroom)?, add(tile_scratch, copy_maps)?)?)? > limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut staged = sparse_copy(pre, limits.resident_bytes.checked_sub(add(retained, actual_scratch)?).ok_or(RasterInputError::LimitExceeded)?, control)?;
        // No allocated key set: visit each key in primary/mirror order, skipping
        // keys already seen in an earlier pass, and write each native tile once.
        for (index, pass) in passes.iter().enumerate() {
            for &(tx, ty) in pass.cov.tiles.keys() {
                poll(control)?;
                if passes.get(..index).ok_or(RasterInputError::LimitExceeded)?.iter().any(|earlier| earlier.cov.tiles.contains_key(&(tx, ty))) {
                    continue;
                }
                let x0 = tx.checked_mul(COV_TILE).ok_or(RasterInputError::InvalidBounds)?;
                let y0 = ty.checked_mul(COV_TILE).ok_or(RasterInputError::InvalidBounds)?;
                let area = Rect::new(
                    x0,
                    y0,
                    x0.checked_add(COV_TILE).ok_or(RasterInputError::InvalidBounds)?,
                    y0.checked_add(COV_TILE).ok_or(RasterInputError::InvalidBounds)?,
                )
                .intersect(&bounds);
                if area.is_empty() {
                    continue;
                }
                let pixels = usize::try_from(area.width())
                    .ok()
                    .and_then(|w| usize::try_from(area.height()).ok().and_then(|h| w.checked_mul(h)))
                    .ok_or(RasterInputError::InvalidBounds)?;
                if pixels > coverage.capacity() {
                    return Err(RasterInputError::LimitExceeded);
                }
                coverage.clear();
                for y in area.y0..area.y1 {
                    poll(control)?;
                    for x in area.x0..area.x1 {
                        let mut primary = self.cov.get(x, y);
                        let mut depth = self.cov.depth(x, y);
                        let mut dual = self.dual.as_ref().map(|map| map.get(x, y));
                        for other in others {
                            primary = primary.max(other.cov.get(x, y));
                            if let Some(value) = other.cov.depth(x, y) {
                                depth = Some(depth.map_or(value, |old| old.max(value)));
                            }
                            if let (Some(value), Some(map)) = (dual.as_mut(), other.dual.as_ref()) {
                                *value = value.max(map.get(x, y));
                            }
                        }
                        let value = if primary <= 0.0 { 0.0 } else { self.ctx.stroke_mask_wet(primary, depth, dual, x, y) };
                        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                            return Err(RasterInputError::InvalidSample);
                        }
                        coverage.push(f64::from(value));
                    }
                }
                if brush.erase {
                    crate::wide_erase::try_erase_coverage(
                        &mut staged,
                        area,
                        &coverage,
                        f64::from(brush.opacity),
                        selection,
                        false,
                        crate::wide_erase::EraseColorPolicy::Preserve,
                        crate::wide_erase::EraseLimits::default(),
                        control,
                    )?;
                } else if let Some(blend) = blend {
                    crate::wide_blend_controlled::try_apply_unit_blend_coverage(
                        &mut staged,
                        native_color,
                        area,
                        &coverage,
                        f64::from(brush.opacity),
                        selection,
                        lock_transparency,
                        blend,
                        limits,
                        control,
                    )?;
                } else {
                    crate::wide::try_apply_normal_coverage(
                        &mut staged,
                        native_color,
                        area,
                        &coverage,
                        f64::from(brush.opacity),
                        selection,
                        lock_transparency,
                        control,
                    )?;
                }
            }
        }
        poll(control)?;
        *target = staged;
        Ok(bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_wide_passes_apply_opacity_once_and_keep_raw_outside() {
        for sample in [SampleType::U32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut pre = Surface::new(format);
            let raw = if sample == SampleType::U32 {
                [0x7fff_ffff_u32, 0x8000_0000, 7, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>()
            } else {
                [f64::from_bits(0x3fe0_0000_0000_0001), 0.25, -0.0, 1.0].into_iter().flat_map(f64::to_le_bytes).collect::<Vec<_>>()
            };
            pre.try_write_interleaved_with(Rect::new(100, 100, 101, 101), &raw, &Interrupt::NONE).unwrap();
            let color = Surface::try_with_default_bytes(
                PixelFormat::new(ColorMode::Rgb, SampleType::F64, true),
                &[1.0_f64, 0.0, 0.0, 1.0].into_iter().flat_map(f64::to_le_bytes).collect::<Vec<_>>(),
            )
            .unwrap();
            let brush = BrushSettings { size: 4.0, pressure_size: false, flow: 1.0, opacity: 0.5, ..Default::default() };
            let mut renderer = StrokeRenderer::new(&brush, Some(format), 1.0);
            renderer.push(&[StrokePoint::new(2.0, 2.0, 1.0)]);
            renderer.finish();
            let mut single = pre.clone();
            let mut many = pre.clone();
            renderer.try_composite_union_many_wide(&[], &pre, &mut single, &color, None, false, None, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            renderer
                .try_composite_union_many_wide(
                    &[&renderer, &renderer, &renderer],
                    &pre,
                    &mut many,
                    &color,
                    None,
                    false,
                    None,
                    StrokeLimits::default(),
                    &Interrupt::NONE,
                )
                .unwrap();
            assert_eq!(single, many);
            assert_eq!(many.try_read_interleaved_with(Rect::new(100, 100, 101, 101), &Interrupt::NONE).unwrap(), raw);
            let before = many.clone();
            let stop = || true;
            assert!(matches!(
                renderer.try_composite_union_many_wide(
                    &[&renderer],
                    &pre,
                    &mut many,
                    &color,
                    None,
                    false,
                    None,
                    StrokeLimits::default(),
                    &Interrupt::cancel_only(&stop)
                ),
                Err(RasterInputError::Cancelled)
            ));
            assert_eq!(many, before);
        }
    }
    #[test]
    fn excessive_pass_or_tiny_budget_refuses_without_publishing() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let pre = Surface::new(format);
        let mut target = pre.clone();
        let color = Surface::try_with_default_bytes(format, &[0.5_f64, 1.0].into_iter().flat_map(f64::to_le_bytes).collect::<Vec<_>>()).unwrap();
        let renderer = StrokeRenderer::new(&BrushSettings::default(), Some(format), 1.0);
        assert!(
            renderer
                .try_composite_union_many_wide(
                    &[&renderer, &renderer, &renderer, &renderer],
                    &pre,
                    &mut target,
                    &color,
                    None,
                    false,
                    None,
                    StrokeLimits::default(),
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert!(
            renderer
                .try_composite_union_many_wide(
                    &[],
                    &pre,
                    &mut target,
                    &color,
                    None,
                    false,
                    None,
                    StrokeLimits { resident_bytes: 1, ..Default::default() },
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert_eq!(target, pre);
    }
    #[test]
    fn explicit_blend_mapping_and_duplicate_union_keep_opacity_once() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let pre = Surface::try_with_default_bytes(format, &[0.25_f64, 1.0].into_iter().flat_map(f64::to_le_bytes).collect::<Vec<_>>()).unwrap();
        let color = Surface::try_with_default_bytes(format, &[0.5_f64, 1.0].into_iter().flat_map(f64::to_le_bytes).collect::<Vec<_>>()).unwrap();
        for (mode, blend) in [
            (BlendMode::Multiply, crate::wide_blend_controlled::NativeUnitBlend::Multiply),
            (BlendMode::Screen, crate::wide_blend_controlled::NativeUnitBlend::Screen),
        ] {
            let brush = BrushSettings { mode, size: 4.0, pressure_size: false, flow: 1.0, opacity: 0.5, ..Default::default() };
            let mut renderer = StrokeRenderer::new(&brush, Some(format), 1.0);
            renderer.push(&[StrokePoint::new(2.0, 2.0, 1.0)]);
            renderer.finish();
            let mut single = pre.clone();
            let mut many = pre.clone();
            assert!(
                renderer.try_composite_union_many_wide(&[], &pre, &mut single, &color, None, false, None, StrokeLimits::default(), &Interrupt::NONE).is_err()
            );
            assert_eq!(single, pre);
            renderer
                .try_composite_union_many_wide_with_blend(
                    &[],
                    &pre,
                    &mut single,
                    &color,
                    None,
                    false,
                    None,
                    Some(blend),
                    StrokeLimits::default(),
                    &Interrupt::NONE,
                )
                .unwrap();
            renderer
                .try_composite_union_many_wide_with_blend(
                    &[&renderer, &renderer, &renderer],
                    &pre,
                    &mut many,
                    &color,
                    None,
                    false,
                    None,
                    Some(blend),
                    StrokeLimits::default(),
                    &Interrupt::NONE,
                )
                .unwrap();
            assert_eq!(single, many);
            assert_ne!(single, pre);
        }
    }
}
