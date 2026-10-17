//! Closed current-layer native RGB Mixer Brush, full-load/clean-well only.
//! Upstream row-major pickup/deposition order is lifted to binary64 WORK, not
//! source-operation admission or bitwise equivalence to legacy f32 arithmetic.
use crate::{StrokeError, StrokeLimits, StrokeRenderer};
use photocraft_color::{ColorMode, SampleType, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::Arc;
type Result<T> = std::result::Result<T, StrokeError>;
#[derive(Clone, Copy, Debug)]
pub struct NativeMixerSettings {
    pub reservoir: [f64; 3],
    pub wet: f64,
    pub mix: f64,
    pub flow: f64,
}
pub struct NativeMixerOutput {
    pub surface: Surface,
    pub damage: Rect,
}
fn raster(error: RasterInputError) -> StrokeError {
    match error {
        RasterInputError::Cancelled => StrokeError::Cancelled,
        RasterInputError::AllocationFailed => StrokeError::Allocation,
        RasterInputError::LimitExceeded => StrokeError::Limit,
        _ => StrokeError::Raster,
    }
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| StrokeError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(StrokeError::Limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(StrokeError::Limit)
}
fn unit(v: f64) -> Result<f64> {
    if v.is_finite() && (0.0..=1.0).contains(&v) { Ok(v) } else { Err(StrokeError::InvalidInput("native Mixer requires finite unit samples")) }
}
fn count(r: Rect) -> Result<usize> {
    usize::try_from(r.width()).ok().and_then(|w| usize::try_from(r.height()).ok().and_then(|h| w.checked_mul(h))).ok_or(StrokeError::Limit)
}
fn index(r: Rect, x: i32, y: i32) -> Result<usize> {
    if !r.contains(x, y) {
        return Err(StrokeError::InvalidInput("Mixer coordinate"));
    }
    let w = usize::try_from(r.width()).map_err(|_| StrokeError::Limit)?;
    usize::try_from(i64::from(y) - i64::from(r.y0))
        .ok()
        .and_then(|row| row.checked_mul(w))
        .and_then(|row| usize::try_from(i64::from(x) - i64::from(r.x0)).ok().and_then(|x| row.checked_add(x)))
        .ok_or(StrokeError::Limit)
}
fn vector<T: Clone>(len: usize, value: T, c: &Interrupt<'_>) -> Result<Vec<T>> {
    poll(c)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| StrokeError::Allocation)?;
    if out.capacity() != len {
        return Err(StrokeError::Limit);
    }
    for i in 0..len {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        out.push(value.clone());
    }
    Ok(out)
}
fn pixels(surface: &Surface, x: i32, y: i32) -> Result<[f64; 4]> {
    let mut out = [0.0; 4];
    let n = surface.channels();
    surface.try_read_pixel_f64(x, y, out.get_mut(..n).ok_or(StrokeError::InvalidInput("Mixer format"))?).map_err(raster)?;
    if !surface.format().alpha {
        out[3] = 1.0;
    }
    Ok(out)
}
fn retained(s: &Surface, limit: u64, c: &Interrupt<'_>) -> Result<u64> {
    poll(c)?;
    let tile = (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_mul(s.format().bytes_per_pixel())).ok_or(StrokeError::Limit)?;
    if add(s.default_bytes().len() as u64, mul(s.tile_count() as u64, add(tile as u64, 256)?)?)? > limit {
        return Err(StrokeError::Limit);
    }
    s.try_validate_tile_origins_with(c).map_err(raster)?;
    if s.default_bytes().len() != s.format().bytes_per_pixel() {
        return Err(StrokeError::InvalidInput("Mixer default length"));
    }
    let mut n = add(s.default_bytes().len() as u64, mul(s.tile_count() as u64, 256)?)?;
    for (_, t) in s.tiles() {
        poll(c)?;
        if t.bytes().len() != tile {
            return Err(StrokeError::InvalidInput("Mixer tile length"));
        }
        n = add(n, tile as u64)?;
    }
    Ok(n)
}
fn tile_count(r: Rect) -> Result<u64> {
    if r.is_empty() {
        return Ok(0);
    }
    let x1 = r.x1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(TILE_SIZE);
    let y1 = r.y1.checked_sub(1).ok_or(StrokeError::Limit)?.div_euclid(TILE_SIZE);
    let w = u64::try_from(i64::from(x1) - i64::from(r.x0.div_euclid(TILE_SIZE)) + 1).map_err(|_| StrokeError::Limit)?;
    let h = u64::try_from(i64::from(y1) - i64::from(r.y0.div_euclid(TILE_SIZE)) + 1).map_err(|_| StrokeError::Limit)?;
    mul(w, h)
}

/// Renderer was generated with opacity exactly one and record_dabs enabled.
/// Full load keeps amount exactly one; pickup begins empty and is discarded by
/// caller's mandatory clean-after policy. No computed native well enters f32.
#[allow(clippy::too_many_arguments)]
pub fn try_apply_full_load(
    pre: &Surface,
    renderer: &StrokeRenderer,
    settings: NativeMixerSettings,
    selection: Option<&Surface>,
    lock: bool,
    limits: StrokeLimits,
    c: &Interrupt<'_>,
) -> Result<NativeMixerOutput> {
    poll(c)?;
    let standard = StrokeLimits::default();
    let fmt = pre.format();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
        return Err(StrokeError::Limit);
    }
    if fmt.mode != ColorMode::Rgb
        || !matches!(fmt.sample, SampleType::U32 | SampleType::F64)
        || renderer.ctx.brush.opacity.to_bits() != 1.0f32.to_bits()
        || selection.is_some_and(|s| s.format().mode != ColorMode::Grayscale || s.format().alpha)
    {
        return Err(StrokeError::InvalidInput("native full-load RGB Mixer domain"));
    }
    for v in settings.reservoir {
        unit(v)?;
    }
    unit(settings.wet)?;
    unit(settings.mix)?;
    unit(settings.flow)?;
    let dabs = renderer.dabs();
    if dabs.len() > 100_000 || dabs.len() != renderer.dab_count() {
        return Err(StrokeError::InvalidInput("bounded recorded Mixer dabs required"));
    }
    let mut bounds = Rect::EMPTY;
    let mut visits = 0u64;
    for d in dabs {
        poll(c)?;
        if !d.center.x.is_finite()
            || !d.center.y.is_finite()
            || d.center.x.abs() > 1_000_000.0
            || d.center.y.abs() > 1_000_000.0
            || !d.radius.is_finite()
            || d.radius <= 0.0
            || !d.alpha.is_finite()
            || !d.wet.is_finite()
            || !d.mix.is_finite()
        {
            return Err(StrokeError::InvalidInput("native Mixer dab geometry/settings"));
        }
        let r = renderer.ctx.dab_rect(d, false);
        visits = add(visits, count(r)? as u64)?;
        bounds = bounds.union(&r);
    }
    let n = count(bounds)?;
    let tiles = tile_count(bounds)?;
    let native_tile = mul((TILE_SIZE * TILE_SIZE) as u64, fmt.bytes_per_pixel() as u64)?;
    let req = renderer.incremental_clone_requirements(limits, c)?;
    let held = add(req.retained_bytes, add(retained(pre, limits.resident_bytes, c)?, selection.map_or(Ok(0), |s| retained(s, limits.resident_bytes, c))?)?)?;
    let scratch = add(mul(n as u64, 32)?, 512)?;
    let output = add(mul(tiles, add(native_tile, 512)?)?, add(mul(pre.tile_count() as u64, 512)?, mul(native_tile, 4)?)?)?;
    let work = add(req.work_units, add(mul(visits, 2048)?, add(mul(n as u64, 2048)?, mul(mul(tiles, add(pre.tile_count() as u64, tiles)?)?, 256)?)?)?)?;
    if add(held, add(scratch, output)?)? > limits.resident_bytes || work > limits.work_units {
        return Err(StrokeError::Limit);
    }
    let mut values = vector(n, [0.0f64; 4], c)?;
    let mut footprint: Vec<f32> = Vec::new();
    footprint.try_reserve_exact(64).map_err(|_| StrokeError::Allocation)?;
    if values.capacity() != n || footprint.capacity() > 128 {
        return Err(StrokeError::Limit);
    }
    for y in bounds.y0..bounds.y1 {
        poll(c)?;
        for x in bounds.x0..bounds.x1 {
            if x.rem_euclid(64) == 0 {
                poll(c)?;
            }
            *values.get_mut(index(bounds, x, y)?).ok_or(StrokeError::Limit)? = pixels(pre, x, y)?;
        }
    }
    let mut pickup: Option<[f64; 4]> = None;
    let threshold = f64::from(1.0e-6f32);
    for dab in dabs {
        poll(c)?;
        let r = renderer.ctx.dab_rect(dab, false).intersect(&bounds);
        let wet = settings.wet * f64::from(dab.wet.clamp(0.0, 1.0));
        let mix = settings.mix * f64::from(dab.mix.clamp(0.0, 1.0));
        let mut acc = [0.0f64; 3];
        let (mut wsum, mut asum) = (0.0f64, 0.0f64);
        // The complete dab is reduced before deposition; reads CURRENT WORK,
        // not PRE or a sampleAllLayers composition. Span boundaries don't alter
        // row-major floating addition order.
        for y in r.y0..r.y1 {
            let mut x = r.x0;
            while x < r.x1 {
                poll(c)?;
                let end = i32::try_from((i64::from(x) + 64).min(i64::from(r.x1))).map_err(|_| StrokeError::Limit)?;
                let row = Rect::new(x, y, end, y.checked_add(1).ok_or(StrokeError::Limit)?);
                renderer.ctx.rasterize_into(dab, false, row, &mut footprint, None);
                if footprint.len() != count(row)? || footprint.capacity() > 128 {
                    return Err(StrokeError::Limit);
                }
                for (offset, cov) in footprint.iter().copied().enumerate() {
                    let w = unit(f64::from(cov))?;
                    if w <= 0.0 {
                        continue;
                    }
                    let px = x.checked_add(i32::try_from(offset).map_err(|_| StrokeError::Limit)?).ok_or(StrokeError::Limit)?;
                    let current = values.get(index(bounds, px, y)?).ok_or(StrokeError::Limit)?;
                    let al = unit(current[3])?;
                    for ch in 0..3 {
                        let color = unit(*current.get(ch).ok_or(StrokeError::Limit)?)?;
                        let sum = acc.get_mut(ch).ok_or(StrokeError::Limit)?;
                        *sum += color * w * al;
                        if !sum.is_finite() {
                            return Err(StrokeError::InvalidInput("native Mixer pickup overflow"));
                        }
                    }
                    wsum += w;
                    asum += w * al;
                    if !wsum.is_finite() || !asum.is_finite() {
                        return Err(StrokeError::InvalidInput("native Mixer weight overflow"));
                    }
                }
                x = end;
            }
        }
        if asum > threshold && wet > 0.0 {
            let ca = unit(asum / wsum.max(threshold))?;
            let mut canvas = [1.0f64; 4];
            for ch in 0..3 {
                *canvas.get_mut(ch).ok_or(StrokeError::Limit)? = unit(*acc.get(ch).ok_or(StrokeError::Limit)? / asum)?;
            }
            pickup = Some(match pickup {
                None => canvas,
                Some(old) => {
                    let mut next = [0.0; 4];
                    for ch in 0..4 {
                        let p = *old.get(ch).ok_or(StrokeError::Limit)?;
                        let v = *canvas.get(ch).ok_or(StrokeError::Limit)?;
                        *next.get_mut(ch).ok_or(StrokeError::Limit)? = unit(p + (v - p) * f64::from(crate::mixer::PICKUP_RATE) * wet * ca)?;
                    }
                    next
                }
            });
        }
        let wr = 1.0 * (1.0 - mix * wet);
        let wp = if pickup.is_some() { mix * wet } else { 0.0 };
        let total = wr + wp;
        if total > threshold {
            let mut paint = [0.0; 4];
            for ch in 0..4 {
                let rv = if ch < 3 { *settings.reservoir.get(ch).ok_or(StrokeError::Limit)? } else { 1.0 };
                let pv = pickup.as_ref().and_then(|p| p.get(ch)).copied().unwrap_or(0.0);
                *paint.get_mut(ch).ok_or(StrokeError::Limit)? = unit((rv * wr + pv * wp) / total)?;
            }
            let dep = total.min(1.0) * settings.flow;
            for y in r.y0..r.y1 {
                let mut x = r.x0;
                while x < r.x1 {
                    poll(c)?;
                    let end = i32::try_from((i64::from(x) + 64).min(i64::from(r.x1))).map_err(|_| StrokeError::Limit)?;
                    let row = Rect::new(x, y, end, y.checked_add(1).ok_or(StrokeError::Limit)?);
                    renderer.ctx.rasterize_into(dab, false, row, &mut footprint, None);
                    for (offset, cov) in footprint.iter().copied().enumerate() {
                        let k = unit(f64::from(cov))? * dep;
                        if k <= 0.0 {
                            continue;
                        }
                        let px = x.checked_add(i32::try_from(offset).map_err(|_| StrokeError::Limit)?).ok_or(StrokeError::Limit)?;
                        let dst = values.get_mut(index(bounds, px, y)?).ok_or(StrokeError::Limit)?;
                        if fmt.alpha {
                            let da = unit(dst[3])?;
                            let sa = paint[3] * k;
                            let oa = unit(sa + da * (1.0 - sa))?;
                            if oa > 0.0 {
                                for ch in 0..3 {
                                    let d = unit(*dst.get(ch).ok_or(StrokeError::Limit)?)?;
                                    let s = *paint.get(ch).ok_or(StrokeError::Limit)?;
                                    *dst.get_mut(ch).ok_or(StrokeError::Limit)? = unit((s * sa + d * da * (1.0 - sa)) / oa)?;
                                }
                            }
                            if !lock {
                                dst[3] = oa;
                            }
                        } else {
                            for ch in 0..3 {
                                let d = unit(*dst.get(ch).ok_or(StrokeError::Limit)?)?;
                                *dst.get_mut(ch).ok_or(StrokeError::Limit)? = unit(d + (*paint.get(ch).ok_or(StrokeError::Limit)? - d) * k)?;
                            }
                        }
                    }
                    x = end;
                }
            }
        }
        // amount *= 1 - 0.08*(1-load), with load exactly one: remains one.
    }
    let mut staged = Surface::try_with_default_bytes(fmt, pre.default_bytes()).map_err(raster)?;
    for (coord, tile) in pre.tiles() {
        poll(c)?;
        staged.put_tiles(std::iter::once((*coord, Arc::clone(tile))));
    }
    let bpp = fmt.bytes_per_pixel();
    let mut changed_any = false;
    for tile in bounds.tiles() {
        poll(c)?;
        let r = tile.rect().intersect(&bounds);
        let mut raw = pre.try_read_interleaved_with(r, c).map_err(raster)?;
        let w = usize::try_from(r.width()).map_err(|_| StrokeError::Limit)?;
        if w == 0 {
            return Err(StrokeError::Limit);
        }
        if add(held, add(scratch, add(output, raw.capacity() as u64)?)?)? > limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        let mut changed = false;
        for (i, encoded) in raw.chunks_exact_mut(bpp).enumerate() {
            if i.is_multiple_of(256) {
                poll(c)?;
            }
            let x = r.x0.checked_add((i % w) as i32).ok_or(StrokeError::Limit)?;
            let y = r.y0.checked_add((i / w) as i32).ok_or(StrokeError::Limit)?;
            let original = pixels(pre, x, y)?;
            let value = values.get(index(bounds, x, y)?).ok_or(StrokeError::Limit)?;
            if original.iter().zip(value).all(|(a, b)| a.to_bits() == b.to_bits()) {
                continue;
            }
            let mut k = 1.0;
            if let Some(s) = selection {
                let mut sel = [0.0];
                s.try_read_pixel_f64(x, y, &mut sel).map_err(raster)?;
                k = unit(sel[0])?;
            }
            if k <= 0.0 {
                continue;
            }
            let mut before = [0u8; 32];
            before.get_mut(..bpp).ok_or(StrokeError::Limit)?.copy_from_slice(encoded);
            let da = unit(original[3])?;
            let sa = unit(value[3])?;
            let oa = unit(da + (sa - da) * k)?;
            for ch in 0..3 {
                let d = unit(*original.get(ch).ok_or(StrokeError::Limit)?)?;
                let s = unit(*value.get(ch).ok_or(StrokeError::Limit)?)?;
                let result = if fmt.alpha && !lock && oa > threshold { (d * da + (s * sa - d * da) * k) / oa } else { d + (s - d) * k };
                unit(result)?;
                try_write_sample_f64(encoded, fmt.sample, ch, result).map_err(|_| StrokeError::InvalidInput("native Mixer output"))?;
            }
            if fmt.alpha && !lock {
                try_write_sample_f64(encoded, fmt.sample, 3, oa).map_err(|_| StrokeError::InvalidInput("native Mixer alpha"))?;
            }
            changed |= encoded != before.get(..bpp).ok_or(StrokeError::Limit)?;
        }
        if changed {
            staged.try_write_interleaved_with(r, &raw, c).map_err(raster)?;
            changed_any = true;
        }
    }
    poll(c)?;
    Ok(NativeMixerOutput { surface: staged, damage: if changed_any { bounds } else { Rect::EMPTY } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_color::PixelFormat;
    fn renderer() -> StrokeRenderer {
        let brush = BrushSettings { size: 8.0, hardness: 1.0, opacity: 1.0, flow: 1.0, pressure_size: false, pressure_opacity: false, ..Default::default() };
        let mut r = StrokeRenderer::try_new_with_interrupt(&brush, None, 1.0, StrokeLimits::default(), &Interrupt::NONE).unwrap().record_dabs();
        r.push_with_interrupt(&[StrokePoint::new(2.5, 2.5, 1.0)], StrokeLimits::default(), &Interrupt::NONE).unwrap();
        r.finish_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        r
    }
    fn settings() -> NativeMixerSettings {
        NativeMixerSettings { reservoir: [1.0, 0.0, 0.0], wet: 0.0, mix: 0.0, flow: 1.0 }
    }
    #[test]
    fn final_selection_mix_and_prospective_alpha_lock_are_distinct() {
        let r = renderer();
        let pre = Surface::with_default(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), &[0.0, 0.0, 0.0, 0.0]);
        let selection = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &[0.5]);
        let free = try_apply_full_load(&pre, &r, settings(), Some(&selection), false, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let locked = try_apply_full_load(&pre, &r, settings(), Some(&selection), true, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(pixels(&free.surface, 2, 2).unwrap(), [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(
            pixels(&locked.surface, 2, 2).unwrap(),
            [0.5, 0.0, 0.0, 0.0],
            "locked transparent color must follow upstream prospective-alpha equation, not ordinary brush alpha-zero skip"
        );
        assert_eq!(pixels(&pre, 2, 2).unwrap(), [0.0; 4]);
    }
    #[test]
    fn zero_coverage_and_cancel_leave_original_native_words() {
        for sample in [SampleType::U32, SampleType::F64] {
            let pre = Surface::with_default(PixelFormat::new(ColorMode::Rgb, sample, true), &[0.5, 0.25, 0.75, 0.5]);
            let r = renderer();
            let mask = Surface::new(PixelFormat::GRAY8);
            let out = try_apply_full_load(&pre, &r, settings(), Some(&mask), false, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert!(out.damage.is_empty());
            assert_eq!(out.surface, pre);
            let cancel = || true;
            assert!(matches!(
                try_apply_full_load(&pre, &r, settings(), None, false, StrokeLimits::default(), &Interrupt::cancel_only(&cancel)),
                Err(StrokeError::Cancelled)
            ));
            assert!(matches!(
                try_apply_full_load(&pre, &r, settings(), None, false, StrokeLimits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
                Err(StrokeError::Limit)
            ));
        }
    }
    #[test]
    fn wet_current_work_retains_f64_adjacent_colors() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let mut pre = Surface::with_default(fmt, &[0.5, 0.5, 0.5, 1.0]);
        let high = f64::from_bits(0.5f64.to_bits() + 8);
        let mut pixel = vec![0; fmt.bytes_per_pixel()];
        for ch in 0..3 {
            try_write_sample_f64(&mut pixel, SampleType::F64, ch, high).unwrap();
        }
        try_write_sample_f64(&mut pixel, SampleType::F64, 3, 1.0).unwrap();
        pre.try_write_interleaved_with(Rect::new(3, 0, 4, 1), &pixel, &Interrupt::NONE).unwrap();
        let brush = BrushSettings {
            size: 1.0,
            hardness: 1.0,
            aliased: true,
            square: true,
            opacity: 1.0,
            flow: 1.0,
            spacing: 1.0,
            pressure_size: false,
            pressure_opacity: false,
            ..Default::default()
        };
        let mut r = StrokeRenderer::try_new_with_interrupt(&brush, None, 1.0, StrokeLimits::default(), &Interrupt::NONE).unwrap().record_dabs();
        r.push_with_interrupt(&[StrokePoint::new(0.5, 0.5, 1.0), StrokePoint::new(3.5, 0.5, 1.0)], StrokeLimits::default(), &Interrupt::NONE).unwrap();
        r.finish_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let out = try_apply_full_load(
            &pre,
            &r,
            NativeMixerSettings { reservoir: [0.5; 3], wet: 1.0, mix: 1.0, flow: 1.0 },
            None,
            false,
            StrokeLimits::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        let p = pixels(&out.surface, 3, 0).unwrap();
        assert!(p[0] > 0.5 && p[0] < high, "sequential pickup must distinguish adjacent F64 colors that collapse to one f32");
        assert_eq!(p[3], 1.0);
    }
}
