//! Fallible admitted construction of the original incremental stroke renderer.
//! No reinterpretation of brush colors, stored resources or target format.
use super::incremental_clone_controlled::{CloneBudget, brush_requirements, copy_brush, fixed};
use super::*;
use crate::brush::{MAX_BRUSH_SIZE, MAX_SCATTER, PatternStyle};
use crate::tile::GrayTile;
use photocraft_raster::Interrupt;
use std::mem::size_of;
type Result<T> = std::result::Result<T, StrokeError>;
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(StrokeError::Limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(StrokeError::Limit)
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| StrokeError::Cancelled)
}
fn valid(tile: &GrayTile) -> bool {
    tile.width > 0 && tile.height > 0 && (tile.width as usize).checked_mul(tile.height as usize) == Some(tile.data.len())
}
fn vector<T>(budget: &mut CloneBudget<'_, '_>, count: usize) -> Result<Vec<T>> {
    budget.admit(mul(count as u64, size_of::<T>() as u64)?)?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| StrokeError::Allocation)?;
    budget.charge(mul(out.capacity() as u64, size_of::<T>() as u64)?)?;
    Ok(out)
}
fn tip_requirements(tip: &TipShape, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
    let Some(tile) = tip.bitmap().filter(|tile| valid(tile)) else {
        return Ok(Default::default());
    };
    let req = Mips::controlled_requirements(tile, limits, control)?;
    Ok(IncrementalCloneRequirements { retained_bytes: 0, staged_bytes: req.staged_bytes, work_units: req.work_units })
}
fn make_tip(tip: &TipShape, limits: StrokeLimits, control: &Interrupt<'_>, budget: &mut CloneBudget<'_, '_>) -> Result<Option<Arc<Mips>>> {
    let Some(tile) = tip.bitmap().filter(|tile| valid(tile)) else {
        return Ok(None);
    };
    let req = Mips::controlled_requirements(tile, limits, control)?;
    budget.admit(req.staged_bytes)?;
    budget.charge(req.staged_bytes)?;
    budget.work(req.work_units)?;
    // Restrict the lower constructor to its prepaid complete pyramid quota.
    let mip =
        Mips::try_new_with_interrupt(tile, StrokeLimits { resident_bytes: add(req.retained_bytes, req.staged_bytes)?, work_units: req.work_units }, control)?;
    budget.poll()?;
    Ok(Some(Arc::new(mip)))
}
impl StrokeRenderer {
    pub fn new_requirements(
        brush: &BrushSettings,
        format: Option<PixelFormat>,
        zoom: f32,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<IncrementalCloneRequirements> {
        poll(control)?;
        if !zoom.is_finite() || format.is_some_and(|f| f.channels() == 0 || f.channels() > 8) {
            return Err(StrokeError::InvalidInput("renderer format/zoom"));
        }
        let mut req = fixed::<Self>();
        req.add(fixed::<BrushContext>(), limits)?;
        req.add(fixed::<DabGenerator>(), limits)?;
        let copied = brush_requirements(brush, limits, control)?;
        for _ in 0..(2 + usize::from(brush.dual_brush.enabled)) {
            req.add(copied, limits)?;
        }
        req.add(tip_requirements(&brush.tip, limits, control)?, limits)?;
        if brush.dual_brush.enabled {
            req.add(tip_requirements(&brush.dual_brush.tip, limits, control)?, limits)?;
        }
        if brush.texture.enabled {
            let count = match &brush.texture.pattern {
                Pattern::Procedural { size, seed, .. } => {
                    u64::from(*seed)
                        .checked_mul(0x0100_0000_01B3)
                        .and_then(|v| v.checked_add(17))
                        .ok_or(StrokeError::InvalidInput("procedural seed overflow"))?;
                    u64::from((*size).clamp(8, 1024)).pow(2)
                }
                pattern => pattern.bitmap().filter(|tile| valid(tile)).map_or(0, |tile| tile.data.len() as u64),
            };
            req.add(IncrementalCloneRequirements { retained_bytes: 0, staged_bytes: mul(count, 4)?, work_units: mul(count, 257)? }, limits)?;
        }
        poll(control)?;
        Ok(req)
    }
    pub fn try_new_with_interrupt(
        brush: &BrushSettings,
        format: Option<PixelFormat>,
        zoom: f32,
        limits: StrokeLimits,
        control: &Interrupt<'_>,
    ) -> Result<Self> {
        let req = Self::new_requirements(brush, format, zoom, limits, control)?;
        let mut budget = CloneBudget::new(req, limits, control)?;
        budget.charge(size_of::<Self>() as u64)?;
        budget.charge(size_of::<BrushContext>() as u64)?;
        let mut bounded = copy_brush(brush, &mut budget)?;
        // Exactly bounded_for_render's four scalar updates, without its deep clone.
        let size = |v: f32| if v.is_finite() { v.clamp(0.5, MAX_BRUSH_SIZE) } else { 0.5 };
        let scatter = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, MAX_SCATTER) };
        bounded.size = size(bounded.size);
        bounded.dual_brush.size = size(bounded.dual_brush.size);
        bounded.scattering.scatter.jitter = scatter(bounded.scattering.scatter.jitter);
        bounded.dual_brush.scatter = scatter(bounded.dual_brush.scatter);
        let tip = make_tip(&bounded.tip, limits, control, &mut budget)?;
        let dual_tip = if bounded.dual_brush.enabled { make_tip(&bounded.dual_brush.tip, limits, control, &mut budget)? } else { None };
        let texture = pattern(&bounded, &mut budget)?;
        let generator = DabGenerator::new_with_budget(&bounded, zoom, &mut budget)?;
        let per_dab_color = brush.color_dynamics.enabled && !brush.erase && format.is_some();
        let nc = if per_dab_color { format.map_or(0, |f| f.mode.color_channels()) } else { 0 };
        let dual = brush.dual_brush.enabled.then(|| CoverageMap::new(0));
        let ctx = BrushContext { brush: bounded, tip, dual_tip, texture };
        let output = Self {
            ctx,
            generator,
            cov: CoverageMap::new(nc),
            dual,
            fmt: format,
            per_dab_color,
            dabs_done: 0,
            scratch: Vec::new(),
            depth_scratch: Vec::new(),
            dab_buf: Vec::new(),
            dual_buf: Vec::new(),
            all_dabs: None,
        };
        budget.poll()?;
        Ok(output)
    }
}

#[inline]
fn proc_smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Tileable value noise with `period` lattice cells over the tile.
fn proc_noise(x: f32, y: f32, period: i32, salt: u64) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (proc_smooth(x - x0), proc_smooth(y - y0));
    let (xi, yi) = (x0 as i32, y0 as i32);
    let h = |i: i32, j: i32| hash2(i.rem_euclid(period), j.rem_euclid(period), salt);
    let a = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * tx;
    let b = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * tx;
    a + (b - a) * ty
}

/// Tileable fractal noise in 0..1 (`cells` lattice cells across at the base octave).
fn proc_fbm(x: f32, y: f32, size: f32, cells: i32, octaves: u32, salt: u64) -> f32 {
    let (mut sum, mut amp, mut norm, mut c) = (0.0, 1.0, 0.0, cells);
    for o in 0..octaves {
        sum += proc_noise(x / size * c as f32, y / size * c as f32, c, salt.wrapping_add(o as u64 * 7919)) * amp;
        norm += amp;
        amp *= 0.5;
        c *= 2;
    }
    sum / norm
}

fn pattern(brush: &BrushSettings, budget: &mut CloneBudget<'_, '_>) -> Result<Option<Arc<PatternImage>>> {
    let texture = &brush.texture;
    if !texture.enabled {
        return Ok(None);
    }
    if texture.pattern.bitmap().is_some_and(|tile| !valid(tile)) {
        return Ok(None);
    }
    let (width, height, mut data) = match &texture.pattern {
        Pattern::Tile(tile) => {
            let mut data = vector(budget, tile.data.len())?;
            for value in tile.data.iter() {
                budget.work(1)?;
                data.push(f32::from(*value) / 65535.0);
            }
            (tile.width as usize, tile.height as usize, data)
        }
        Pattern::Stored(stored) => {
            let tile = stored.bitmap();
            let mut data = vector(budget, tile.data.len())?;
            for value in tile.data.iter() {
                budget.work(1)?;
                data.push(f32::from(*value) / 65535.0);
            }
            (tile.width as usize, tile.height as usize, data)
        }
        Pattern::Procedural { style, size, seed } => {
            let style = *style;
            let n = (*size).clamp(8, 1024) as usize;
            let s = n as f32;
            let salt = u64::from(*seed) * 0x0100_0000_01B3 + 17;
            let mut data = vector(budget, n * n)?;
            for y in 0..n {
                budget.poll()?;
                for x in 0..n {
                    budget.work(256)?;
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let v = match style {
                        PatternStyle::Noise => proc_fbm(fx, fy, s, 8, 4, salt),
                        PatternStyle::Paper => {
                            let base = proc_fbm(fx, fy, s, 16, 4, salt);
                            let grain = hash2(x as i32, y as i32, salt ^ 0x55);
                            ((base - 0.5) * 2.2 + 0.5 + (grain - 0.5) * 0.35).clamp(0.0, 1.0)
                        }
                        PatternStyle::Canvas => {
                            // Over/under weave: threads every 8 px, alternating which direction is on top.
                            let period = 8.0;
                            let (cx, cy) = ((fx / period).floor() as i32, (fy / period).floor() as i32);
                            let (u, v) = ((fx / period).fract(), (fy / period).fract());
                            let warp = (std::f32::consts::PI * u).sin();
                            let weft = (std::f32::consts::PI * v).sin();
                            let top = if (cx + cy) % 2 == 0 { warp } else { weft };
                            (0.25 + 0.65 * top + (hash2(x as i32, y as i32, salt) - 0.5) * 0.15).clamp(0.0, 1.0)
                        }
                        PatternStyle::Dots => {
                            let period = 8.0;
                            let (u, v) = ((fx / period).fract() - 0.5, (fy / period).fract() - 0.5);
                            let d = (u * u + v * v).sqrt();
                            1.0 - proc_smooth(((0.35 - d) / 0.1 + 0.5).clamp(0.0, 1.0))
                        }
                    };

                    data.push(v);
                }
            }
            (n, n, data)
        }
    };
    for value in &mut data {
        budget.work(1)?;
        *value = texture.level(*value);
    }
    Ok(Some(Arc::new(PatternImage { width, height, data })))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_construction_keeps_brush_mips_texture_and_continuation() {
        for procedural in [false, true] {
            let tile = GrayTile { width: 5, height: 7, data: (0..35).map(|i| (i * 1739) as u16).collect::<Vec<_>>().into() };
            let mut brush = BrushSettings { tip: TipShape::Sampled(tile.clone()), size: 8.0, pressure_size: false, ..Default::default() };
            brush.dual_brush.enabled = true;
            brush.dual_brush.tip = TipShape::Sampled(tile.clone());
            brush.texture.enabled = true;
            brush.texture.pattern = if procedural { Pattern::Procedural { style: PatternStyle::Paper, size: 17, seed: 7 } } else { Pattern::Tile(tile) };
            let format = PixelFormat::RGBA32F;
            let mut old = StrokeRenderer::new(&brush, Some(format), 1.25);
            let mut checked = StrokeRenderer::try_new_with_interrupt(&brush, Some(format), 1.25, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(old.ctx.brush, checked.ctx.brush);
            for (a, b) in old.ctx.tip.as_ref().unwrap().levels.iter().zip(&checked.ctx.tip.as_ref().unwrap().levels) {
                assert_eq!((a.0, a.1), (b.0, b.1));
                assert_eq!(a.2, b.2);
            }
            assert_eq!(old.ctx.texture.as_ref().unwrap().data, checked.ctx.texture.as_ref().unwrap().data);
            let points = [StrokePoint::new(-2.0, 3.0, 1.0), StrokePoint::new(11.0, 9.0, 0.7)];
            old.push(&points);
            checked.push_with_interrupt(&points, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            old.finish();
            checked.finish_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
            let pre = Surface::new(format);
            let mut a = pre.clone();
            let mut b = pre.clone();
            old.composite(&pre, &mut a, None, false, true);
            checked.composite(&pre, &mut b, None, false, true);
            assert_eq!(a, b);
        }
    }
    #[test]
    fn constructor_tiny_budget_and_cancel_never_return_partial_renderer() {
        let brush = BrushSettings { tip: TipShape::Sampled(GrayTile { width: 257, height: 129, data: vec![12345; 257 * 129].into() }), ..Default::default() };
        assert!(matches!(
            StrokeRenderer::try_new_with_interrupt(&brush, None, 1.0, StrokeLimits { resident_bytes: 1, ..Default::default() }, &Interrupt::NONE),
            Err(StrokeError::Limit)
        ));
        let count = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            count.load(std::sync::atomic::Ordering::Relaxed) > 80
        };
        assert!(matches!(
            StrokeRenderer::try_new_with_interrupt(&brush, None, 1.0, StrokeLimits::default(), &Interrupt::cancel_only(&stop)),
            Err(StrokeError::Cancelled)
        ));
        assert!(matches!(
            StrokeRenderer::try_new_with_interrupt(&brush, None, f32::NAN, StrokeLimits::default(), &Interrupt::NONE),
            Err(StrokeError::InvalidInput(_))
        ));
    }
}
