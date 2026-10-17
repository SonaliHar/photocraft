//! Controlled one-shot legacy stroke rendering. Private surfaces are published
//! only after successful generation, coverage accumulation and compositing.
use super::*;
use crate::{
    Stroke,
    brush::{DualBrush, PatternStyle, Texture},
    dynamics::{DabBuilder, DualBuilder, Smoother, StepInput, apply_pose},
};
use photocraft_color::{SampleType, read_sample, write_sample};
use photocraft_raster::Interrupt;
use std::mem::size_of;
const MAX_WORK: u64 = 8_000_000_000;
const MAX_POINTS: usize = 100_000;
const MAX_DABS: usize = 1_000_000;
const MIN_SPEED_STEP: f64 = 0.5;
const MAX_AIRBRUSH_DABS_PER_SEGMENT: usize = 512;
#[derive(Clone, Copy, Debug)]
pub struct StrokeLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for StrokeLimits {
    fn default() -> Self {
        Self { resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StrokeError {
    Cancelled,
    InvalidInput(&'static str),
    Unsupported(&'static str),
    Limit,
    Allocation,
    Raster,
}
impl std::fmt::Display for StrokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("stroke cancelled"),
            Self::InvalidInput(s) => write!(f, "invalid stroke: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported stroke: {s}"),
            Self::Limit => f.write_str("stroke resource limit"),
            Self::Allocation => f.write_str("stroke allocation failed"),
            Self::Raster => f.write_str("invalid stroke raster"),
        }
    }
}
impl std::error::Error for StrokeError {}
type Result<T> = std::result::Result<T, StrokeError>;
struct Budget<'a, 'b> {
    control: &'a Interrupt<'b>,
    limits: StrokeLimits,
    work: u64,
    forward: Option<&'a dyn Fn(u64) -> Result<()>>,
}
impl Budget<'_, '_> {
    fn poll(&self) -> Result<()> {
        self.control.check().map_err(|_| StrokeError::Cancelled)?;
        if let Some(forward) = self.forward {
            forward(0)?;
        }
        Ok(())
    }
    fn tick(&mut self, n: u64) -> Result<()> {
        self.poll()?;
        if let Some(forward) = self.forward {
            forward(n)?;
        }
        self.work = self.work.checked_add(n).filter(|n| *n <= self.limits.work_units).ok_or(StrokeError::Limit)?;
        Ok(())
    }
    fn vector<T>(&self, n: usize) -> Result<Vec<T>> {
        self.poll()?;
        let bytes = n.checked_mul(size_of::<T>()).ok_or(StrokeError::Limit)?;
        if bytes as u64 > self.limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| StrokeError::Allocation)?;
        if out.capacity().checked_mul(size_of::<T>()).is_none_or(|v| v > bytes) {
            return Err(StrokeError::Limit);
        }
        Ok(out)
    }
    fn raster<T>(&self, value: std::result::Result<T, photocraft_raster::RasterInputError>) -> Result<T> {
        value.map_err(|error| match error {
            photocraft_raster::RasterInputError::Cancelled => StrokeError::Cancelled,
            photocraft_raster::RasterInputError::AllocationFailed => StrokeError::Allocation,
            photocraft_raster::RasterInputError::LimitExceeded => StrokeError::Limit,
            _ => {
                if self.control.cancelled() {
                    StrokeError::Cancelled
                } else {
                    StrokeError::Raster
                }
            }
        })
    }
}
fn at<T: Copy>(values: &[T], index: usize) -> Result<T> {
    values.get(index).copied().ok_or(StrokeError::InvalidInput("internal index"))
}
fn area(rect: Rect) -> Result<usize> {
    usize::try_from(i64::from(rect.x1) - i64::from(rect.x0))
        .ok()
        .and_then(|w| usize::try_from(i64::from(rect.y1) - i64::from(rect.y0)).ok().and_then(|h| w.checked_mul(h)))
        .ok_or(StrokeError::InvalidInput("bounds"))
}
fn check_rect(rect: Rect) -> Result<()> {
    if [rect.x0, rect.y0, rect.x1, rect.y1].iter().any(|v| !(-(1 << 28)..=(1 << 28)).contains(v)) {
        return Err(StrokeError::InvalidInput("coordinates"));
    }
    area(rect)?;
    Ok(())
}
fn finite(values: &[f32]) -> Result<()> {
    if values.iter().any(|v| !v.is_finite()) { Err(StrokeError::InvalidInput("nonfinite settings")) } else { Ok(()) }
}
// The numeric dynamics/renderer never read raw tip/pattern payload fields after
// preparation. Keep those fields inert in their private scalar settings copies;
// the real supplied assets are prepared separately and exactly below.
fn numeric_brush(b: &BrushSettings) -> BrushSettings {
    let t = &b.texture;
    let d = &b.dual_brush;
    BrushSettings {
        size: b.size,
        hardness: b.hardness,
        spacing: b.spacing,
        spacing_enabled: b.spacing_enabled,
        opacity: b.opacity,
        flow: b.flow,
        pressure_size: b.pressure_size,
        pressure_opacity: b.pressure_opacity,
        color: b.color,
        color_cmyk: b.color_cmyk,
        background: b.background,
        erase: b.erase,
        mode: b.mode,
        tip: TipShape::Round,
        angle: b.angle,
        roundness: b.roundness,
        flip_x: b.flip_x,
        flip_y: b.flip_y,
        aliased: b.aliased,
        square: b.square,
        shape_dynamics: b.shape_dynamics.clone(),
        scattering: b.scattering.clone(),
        texture: Texture {
            enabled: t.enabled,
            pattern: Pattern::default(),
            invert: t.invert,
            scale: t.scale,
            brightness: t.brightness,
            contrast: t.contrast,
            each_tip: t.each_tip,
            mode: t.mode,
            depth: t.depth,
            depth_jitter: t.depth_jitter,
        },
        dual_brush: DualBrush {
            enabled: d.enabled,
            mode: d.mode,
            tip: TipShape::Round,
            size: d.size,
            hardness: d.hardness,
            roundness: d.roundness,
            angle: d.angle,
            spacing: d.spacing,
            scatter: d.scatter,
            both_axes: d.both_axes,
            count: d.count,
            flip: d.flip,
        },
        color_dynamics: b.color_dynamics.clone(),
        transfer: b.transfer.clone(),
        pose: b.pose.clone(),
        noise: b.noise,
        wet_edges: b.wet_edges,
        build_up: b.build_up,
        build_up_rate: b.build_up_rate,
        smoothing: b.smoothing.clone(),
        protect_texture: b.protect_texture,
        locks: b.locks.clone(),
        mixer: b.mixer.clone(),
        seed: b.seed,
    }
    .bounded_for_render()
}
fn validate_brush(b: &BrushSettings) -> Result<()> {
    finite(&[
        b.size,
        b.hardness,
        b.spacing,
        b.opacity,
        b.flow,
        b.angle,
        b.roundness,
        b.build_up_rate,
        b.shape_dynamics.tilt_scale,
        b.dual_brush.size,
        b.dual_brush.hardness,
        b.dual_brush.roundness,
        b.dual_brush.angle,
        b.dual_brush.spacing,
        b.dual_brush.scatter,
        b.color_dynamics.hue_jitter,
        b.color_dynamics.saturation_jitter,
        b.color_dynamics.brightness_jitter,
        b.color_dynamics.purity,
        b.pose.tilt_x,
        b.pose.tilt_y,
        b.pose.rotation,
        b.pose.pressure,
        b.texture.scale,
        b.texture.brightness,
        b.texture.contrast,
        b.texture.depth,
        b.smoothing.amount,
    ])?;
    finite(&b.color)?;
    finite(&b.background)?;
    if let Some((rgb, inks)) = b.color_cmyk {
        finite(&rgb)?;
        finite(&inks)?;
    }
    for d in [
        &b.shape_dynamics.size,
        &b.shape_dynamics.angle,
        &b.shape_dynamics.roundness,
        &b.scattering.scatter,
        &b.scattering.count_jitter,
        &b.texture.depth_jitter,
        &b.color_dynamics.fg_bg,
        &b.transfer.opacity,
        &b.transfer.flow,
        &b.transfer.wetness,
        &b.transfer.mix,
    ] {
        finite(&[d.jitter, d.minimum])?;
    }
    Ok(())
}
fn asset_pixels(b: &BrushSettings) -> Result<u64> {
    let mut n = 0u64;
    // Stored presets retain both preview and optional full immutable assets.
    for stored in [&b.tip, &b.dual_brush.tip]
        .into_iter()
        .filter_map(|tip| if let TipShape::Stored(s) = tip { Some(s) } else { None })
        .chain(std::iter::once(&b.texture.pattern).filter_map(|pattern| if let Pattern::Stored(s) = pattern { Some(s) } else { None }))
    {
        n = n.checked_add(stored.key.capacity() as u64).ok_or(StrokeError::Limit)?;
        for tile in std::iter::once(stored.preview.as_ref()).chain(stored.full.as_deref()) {
            let count = (tile.width as u64)
                .checked_mul(tile.height as u64)
                .filter(|n| *n > 0 && *n == tile.data.len() as u64)
                .ok_or(StrokeError::InvalidInput("stored asset"))?;
            n = n
                .checked_add((tile.data.len() as u64).checked_mul(2).and_then(|bytes| bytes.checked_add(32)).ok_or(StrokeError::Limit)?)
                .and_then(|n| n.checked_add(count))
                .ok_or(StrokeError::Limit)?;
        }
    }
    for tip in [&b.tip, &b.dual_brush.tip] {
        if let Some(tile) = tip.bitmap() {
            let count = (tile.width as u64)
                .checked_mul(tile.height as u64)
                .filter(|n| *n == tile.data.len() as u64 && *n > 0)
                .ok_or(StrokeError::InvalidInput("sampled tip"))?;
            n = n
                .checked_add(
                    count
                        .checked_mul(12)
                        .and_then(|n| n.checked_add((tile.data.len() as u64).checked_mul(2).and_then(|bytes| bytes.checked_add(32))?))
                        .ok_or(StrokeError::Limit)?,
                )
                .ok_or(StrokeError::Limit)?;
        }
    }
    if b.texture.enabled {
        let count = match &b.texture.pattern {
            Pattern::Procedural { size, seed, .. } => {
                u64::from(*seed).checked_mul(0x0100_0000_01B3).and_then(|n| n.checked_add(17)).ok_or(StrokeError::InvalidInput("procedural seed overflow"))?;
                u64::from((*size).clamp(8, 1024)).pow(2)
            }
            Pattern::Tile(tile) => {
                let count = (tile.width as u64)
                    .checked_mul(tile.height as u64)
                    .filter(|n| *n == tile.data.len() as u64 && *n > 0)
                    .ok_or(StrokeError::InvalidInput("texture tile"))?;
                n = n
                    .checked_add((tile.data.len() as u64).checked_mul(2).and_then(|bytes| bytes.checked_add(32)).ok_or(StrokeError::Limit)?)
                    .ok_or(StrokeError::Limit)?;
                count
            }
            Pattern::Stored(stored) => {
                let tile = stored.bitmap();
                let count = (tile.width as u64)
                    .checked_mul(tile.height as u64)
                    .filter(|n| *n == tile.data.len() as u64 && *n > 0)
                    .ok_or(StrokeError::InvalidInput("stored texture"))?;
                n = n
                    .checked_add((tile.data.len() as u64).checked_mul(2).and_then(|bytes| bytes.checked_add(32)).ok_or(StrokeError::Limit)?)
                    .ok_or(StrokeError::Limit)?;
                count
            }
        };
        n = n.checked_add(count.checked_mul(8).ok_or(StrokeError::Limit)?).ok_or(StrokeError::Limit)?;
    }
    Ok(n)
}
fn mips(tip: &TipShape, budget: &mut Budget<'_, '_>) -> Result<Option<Arc<Mips>>> {
    let Some(tile) = tip.bitmap() else {
        return Ok(None);
    };
    budget.poll()?;
    let limits =
        StrokeLimits { resident_bytes: budget.limits.resident_bytes, work_units: budget.limits.work_units.checked_sub(budget.work).ok_or(StrokeError::Limit)? };
    let requirements = Mips::controlled_requirements(tile, limits, budget.control)?;
    budget.tick(requirements.work_units)?;
    let mip = Mips::try_new_with_interrupt(tile, limits, budget.control)?;
    budget.poll()?;
    Ok(Some(Arc::new(mip)))
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

fn pattern(brush: &BrushSettings, budget: &mut Budget<'_, '_>) -> Result<Option<Arc<PatternImage>>> {
    let texture = &brush.texture;
    if !texture.enabled {
        return Ok(None);
    }
    let (width, height, mut data) = match &texture.pattern {
        Pattern::Tile(tile) => {
            let mut data = budget.vector(tile.data.len())?;
            for value in tile.data.iter() {
                budget.tick(1)?;
                data.push(f32::from(*value) / 65535.0);
            }
            (tile.width as usize, tile.height as usize, data)
        }
        Pattern::Stored(stored) => {
            let tile = stored.bitmap();
            let mut data = budget.vector(tile.data.len())?;
            for value in tile.data.iter() {
                budget.tick(1)?;
                data.push(f32::from(*value) / 65535.0);
            }
            (tile.width as usize, tile.height as usize, data)
        }
        Pattern::Procedural { style, size, seed } => {
            let style = *style;
            let n = (*size).clamp(8, 1024) as usize;
            let s = n as f32;
            let salt = u64::from(*seed) * 0x0100_0000_01B3 + 17;
            let mut data = budget.vector(n * n)?;
            for y in 0..n {
                budget.poll()?;
                for x in 0..n {
                    budget.tick(256)?;
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
        budget.tick(1)?;
        *value = texture.level(*value);
    }
    Ok(Some(Arc::new(PatternImage { width, height, data })))
}
fn lerp_pt(a: &StrokePoint, b: &StrokePoint, f: f64) -> StrokePoint {
    let ff = f as f32;
    StrokePoint {
        x: a.x + (b.x - a.x) * f,
        y: a.y + (b.y - a.y) * f,
        pressure: a.pressure + (b.pressure - a.pressure) * ff,
        tilt_x: a.tilt_x + (b.tilt_x - a.tilt_x) * ff,
        tilt_y: a.tilt_y + (b.tilt_y - a.tilt_y) * ff,
        rotation: a.rotation + (b.rotation - a.rotation) * ff,
        wheel: a.wheel + (b.wheel - a.wheel) * ff,
        time: a.time + (b.time - a.time) * f,
    }
}

#[derive(Clone, Debug)]
struct ControlledWalker {
    last: Option<StrokePoint>,
    /// Path distance from `last` to the next step.
    next_at: f64,
    /// First point, emitted lazily once the initial direction is known.
    pending_first: Option<StrokePoint>,
    initial_dir: Option<f32>,
    dir: f32,
    step: u64,
    /// Airbrush interval in ms (None = off).
    interval: Option<f64>,
    time_acc: f64,
    /// Speed spacing (Spacing unchecked): one step per this many ms of stroke time.
    speed: Option<f64>,
    speed_acc: f64,
    grid: Option<f64>,
}

impl ControlledWalker {
    fn new(build_up_interval_ms: Option<f64>) -> Self {
        Self {
            last: None,
            next_at: 0.0,
            pending_first: None,
            initial_dir: None,
            dir: 0.0,
            step: 0,
            interval: build_up_interval_ms,
            time_acc: 0.0,
            speed: None,
            speed_acc: 0.0,
            grid: None,
        }
    }

    fn pixel_grid(mut self, offset: f64) -> Self {
        self.grid = Some(offset);
        self
    }
    fn grid_steps(&mut self, a: &StrokePoint, p: &StrokePoint, offset: f64, out: &mut impl FnMut(StepInput) -> Result<()>) -> Result<bool> {
        let cell = |v: f64| if offset > 0.0 { v.floor() } else { v.round() };
        let (x0, y0) = (cell(a.x), cell(a.y));
        let (dx, dy) = (cell(p.x) - x0, cell(p.y) - y0);
        let n = dx.abs().max(dy.abs());
        if !n.is_finite() || n > 1e7 {
            return Ok(false);
        }
        for i in 1..=n as u64 {
            let i = i as f64;
            let mut q = lerp_pt(a, p, i / n);
            q.x = x0 + (dx * i / n).round() + offset;
            q.y = y0 + (dy * i / n).round() + offset;
            self.emit(q, out)?;
        }
        Ok(true)
    }

    /// Space steps by stroke time instead of distance ([`SPEED_SPACING_MS`]): faster movement
    /// spreads them out. Segments without timestamps are spaced by distance.
    fn speed_spacing(mut self, interval_ms: f64) -> Self {
        self.speed = Some(interval_ms.max(0.1));
        self
    }

    fn emit(&mut self, point: StrokePoint, out: &mut impl FnMut(StepInput) -> Result<()>) -> Result<()> {
        let s = StepInput { point, direction: self.dir, initial_direction: self.initial_dir.unwrap_or(self.dir), step: self.step };
        self.step = self.step.checked_add(1).ok_or(StrokeError::Limit)?;
        out(s)
    }

    fn push(&mut self, p: StrokePoint, step_len: &impl Fn(&StrokePoint) -> f64, out: &mut impl FnMut(StepInput) -> Result<()>) -> Result<()> {
        let Some(a) = self.last else {
            self.last = Some(p);
            self.pending_first = Some(p);
            self.next_at = step_len(&p);
            return Ok(());
        };
        let (dx, dy) = (p.x - a.x, p.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-9 {
            self.dir = (-dy).atan2(dx) as f32;
            if self.initial_dir.is_none() {
                self.initial_dir = Some(self.dir);
            }
        }
        if let Some(f) = self.pending_first.take() {
            self.emit(f, out)?;
        }
        if len > 1e-9 {
            // Speed spacing needs the segment's duration; without timestamps (a straight ⇧-click
            // line, or input that carries no times) the segment is spaced by distance instead.
            let dt = p.time - a.time;
            match self.speed.filter(|_| dt > 0.0 && dt.is_finite()) {
                None => {
                    if !match self.grid {
                        Some(g) => self.grid_steps(&a, &p, g, out)?,
                        None => false,
                    } {
                        while self.next_at <= len + 1e-9 {
                            let q = lerp_pt(&a, &p, (self.next_at / len).min(1.0));
                            self.emit(q, out)?;
                            self.next_at += step_len(&q).max(0.25);
                        }
                        self.next_at -= len;
                    }
                }
                Some(iv) => {
                    self.speed_acc += dt;
                    let max_n = (len / MIN_SPEED_STEP).ceil().max(1.0) as usize;
                    let mut n = 0;
                    while self.speed_acc >= iv && n < max_n {
                        self.speed_acc -= iv;
                        let f = ((dt - self.speed_acc) / dt).clamp(0.0, 1.0);
                        self.emit(lerp_pt(&a, &p, f), out)?;
                        n += 1;
                    }
                    if n == max_n {
                        self.speed_acc = self.speed_acc.rem_euclid(iv);
                    }
                }
            }
        }
        if let Some(iv) = self.interval {
            let dt = p.time - a.time;
            if dt > 0.0 && dt.is_finite() {
                if dt > iv * MAX_AIRBRUSH_DABS_PER_SEGMENT as f64 {
                    // A stalled or hostile timestamp must not replay an arbitrarily long backlog.
                    self.time_acc = 0.0;
                    self.emit(p, out)?;
                    self.last = Some(p);
                    return Ok(());
                }
                self.time_acc += dt;
                let mut n = 0;
                while self.time_acc >= iv && n < MAX_AIRBRUSH_DABS_PER_SEGMENT {
                    self.time_acc -= iv;
                    let f = ((dt - self.time_acc) / dt).clamp(0.0, 1.0);
                    self.emit(lerp_pt(&a, &p, f), out)?;
                    n += 1;
                }
            }
        }
        self.last = Some(p);
        Ok(())
    }

    fn finish(&mut self, out: &mut impl FnMut(StepInput) -> Result<()>) -> Result<()> {
        if let Some(f) = self.pending_first.take() {
            self.emit(f, out)?;
        }
        Ok(())
    }
}

fn generate(
    brush: &BrushSettings,
    points: &[StrokePoint],
    zoom: f32,
    steps: usize,
    primary_limit: usize,
    dual_limit: usize,
    budget: &mut Budget<'_, '_>,
) -> Result<(Vec<Dab>, Vec<Dab>)> {
    let mut primary: Vec<Dab> = budget.vector(primary_limit)?;
    let mut dual: Vec<Dab> = budget.vector(dual_limit)?;
    let mut scratch: Vec<StrokePoint> = budget.vector(64)?;
    let mut emitted: Vec<StepInput> = budget.vector(steps)?;
    let mut smoother = Smoother::new(&brush.smoothing, zoom);
    let interval = brush.build_up.then(|| 1000.0 / f64::from(brush.build_up_rate.clamp(0.1, 1000.0)));
    let mut walker =
        if brush.spacing_enabled { ControlledWalker::new(interval) } else { ControlledWalker::new(interval).speed_spacing(crate::dynamics::SPEED_SPACING_MS) };
    if brush.aliased && brush.size * brush.spacing.max(0.01) <= 1.0 {
        walker = walker.pixel_grid(f64::from(crate::render::grid_center(0.0, 0.0, brush.size).0));
    }
    let mut dual_walker = brush.dual_brush.enabled.then(|| ControlledWalker::new(None));
    let mut builder = DabBuilder::new(brush);
    let mut dual_builder = brush.dual_brush.enabled.then(|| DualBuilder::new(brush));
    // Keep separately admitted walker/build buffers and the original budget explicit.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        points: &[StrokePoint],
        brush: &BrushSettings,
        walker: &mut ControlledWalker,
        dual_walker: &mut Option<ControlledWalker>,
        builder: &mut DabBuilder,
        dual_builder: &mut Option<DualBuilder>,
        emitted: &mut Vec<StepInput>,
        primary: &mut Vec<Dab>,
        dual: &mut Vec<Dab>,
        budget: &mut Budget<'_, '_>,
    ) -> Result<()> {
        for p in points {
            budget.poll()?;
            let p = apply_pose(brush, *p);
            emitted.clear();
            let length = |p: &StrokePoint| builder.step_len(p);
            walker.push(p, &length, &mut |step| {
                budget.tick(1)?;
                if emitted.len() >= emitted.capacity() {
                    return Err(StrokeError::Limit);
                }
                emitted.push(step);
                Ok(())
            })?;
            for step in &*emitted {
                budget.tick(1)?;
                let count = if brush.scattering.enabled { brush.scattering.count.clamp(1, 16) as usize } else { 1 };
                if primary.len().checked_add(count).is_none_or(|n| n > primary.capacity()) {
                    return Err(StrokeError::Limit);
                }
                builder.build(step, primary);
            }
            if let (Some(walker), Some(builder)) = (dual_walker.as_mut(), dual_builder.as_mut()) {
                emitted.clear();
                let length = |p: &StrokePoint| builder.step_len(p);
                walker.push(p, &length, &mut |step| {
                    budget.tick(1)?;
                    if emitted.len() >= emitted.capacity() {
                        return Err(StrokeError::Limit);
                    }
                    emitted.push(step);
                    Ok(())
                })?;
                for step in &*emitted {
                    budget.tick(1)?;
                    if dual.len().checked_add(brush.dual_brush.count.clamp(1, 16) as usize).is_none_or(|n| n > dual.capacity()) {
                        return Err(StrokeError::Limit);
                    }
                    builder.build(step, dual);
                }
            }
        }
        Ok(())
    }
    for p in points {
        budget.poll()?;
        scratch.clear();
        smoother.push(*p, &mut scratch);
        walk(&scratch, brush, &mut walker, &mut dual_walker, &mut builder, &mut dual_builder, &mut emitted, &mut primary, &mut dual, budget)?;
    }
    scratch.clear();
    smoother.finish(&mut scratch);
    walk(&scratch, brush, &mut walker, &mut dual_walker, &mut builder, &mut dual_builder, &mut emitted, &mut primary, &mut dual, budget)?;
    emitted.clear();
    walker.finish(&mut |step| {
        budget.tick(1)?;
        if emitted.len() >= emitted.capacity() {
            return Err(StrokeError::Limit);
        }
        emitted.push(step);
        Ok(())
    })?;
    for step in &emitted {
        budget.tick(1)?;
        let count = if brush.scattering.enabled { brush.scattering.count.clamp(1, 16) as usize } else { 1 };
        if primary.len().checked_add(count).is_none_or(|n| n > primary.capacity()) {
            return Err(StrokeError::Limit);
        }
        builder.build(step, &mut primary);
    }
    if let (Some(walker), Some(builder)) = (dual_walker.as_mut(), dual_builder.as_mut()) {
        emitted.clear();
        walker.finish(&mut |step| {
            budget.tick(1)?;
            if emitted.len() >= emitted.capacity() {
                return Err(StrokeError::Limit);
            }
            emitted.push(step);
            Ok(())
        })?;
        for step in &emitted {
            budget.tick(1)?;
            if dual.len().checked_add(brush.dual_brush.count.clamp(1, 16) as usize).is_none_or(|n| n > dual.capacity()) {
                return Err(StrokeError::Limit);
            }
            builder.build(step, &mut dual);
        }
    }
    budget.poll()?;
    Ok((primary, dual))
}
fn cov_tiles(rect: Rect) -> Result<u64> {
    if rect.is_empty() {
        return Ok(0);
    }
    check_rect(rect)?;
    let w = i64::from((rect.x1 - 1).div_euclid(COV_TILE)) - i64::from(rect.x0.div_euclid(COV_TILE)) + 1;
    let h = i64::from((rect.y1 - 1).div_euclid(COV_TILE)) - i64::from(rect.y0.div_euclid(COV_TILE)) + 1;
    u64::try_from(w).ok().and_then(|w| u64::try_from(h).ok().and_then(|h| w.checked_mul(h))).ok_or(StrokeError::Limit)
}
fn accumulate(
    cov: &mut CoverageMap,
    rect: Rect,
    vals: &[f32],
    ceil: f32,
    color: Option<&[f32]>,
    depth: Option<&[f32]>,
    budget: &mut Budget<'_, '_>,
) -> Result<()> {
    if rect.is_empty() {
        return Ok(());
    }
    cov.bounds = cov.bounds.union(&rect);
    let w = rect.width() as usize;
    let nc = cov.nc;
    let want_col = color.is_some() && nc > 0;
    let (t0x, t0y) = (rect.x0.div_euclid(COV_TILE), rect.y0.div_euclid(COV_TILE));
    let (t1x, t1y) = ((rect.x1 - 1).div_euclid(COV_TILE), (rect.y1 - 1).div_euclid(COV_TILE));
    for ty in t0y..=t1y {
        budget.poll()?;
        for tx in t0x..=t1x {
            budget.tick(1)?;
            let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&rect);
            if tr.is_empty() {
                continue;
            }
            let mut any = false;
            for y in tr.y0..tr.y1 {
                budget.poll()?;
                let row = (y - rect.y0) as usize * w;
                let start = row + (tr.x0 - rect.x0) as usize;
                let end = row + (tr.x1 - rect.x0) as usize;
                if vals.get(start..end).ok_or(StrokeError::InvalidInput("coverage row"))?.iter().any(|v| *v > 0.0) {
                    any = true;
                    break;
                }
            }
            if !any {
                continue;
            }
            if !cov.tiles.contains_key(&(tx, ty)) {
                if cov.tiles.len() >= cov.tiles.capacity() {
                    return Err(StrokeError::Limit);
                }
                let mut values = budget.vector((COV_TILE * COV_TILE) as usize)?;
                values.resize((COV_TILE * COV_TILE) as usize, 0.0);
                let mut col = budget.vector(if want_col { (COV_TILE * COV_TILE) as usize * nc } else { 0 })?;
                if want_col {
                    col.resize((COV_TILE * COV_TILE) as usize * nc, 0.0);
                }
                cov.tiles.insert((tx, ty), CovTile { cov: values, col, depth: Vec::new() });
            }
            let tile = cov.tiles.get_mut(&(tx, ty)).ok_or(StrokeError::InvalidInput("coverage tile"))?;
            if want_col && tile.col.is_empty() {
                tile.col = budget.vector((COV_TILE * COV_TILE) as usize * nc)?;
                tile.col.resize((COV_TILE * COV_TILE) as usize * nc, 0.0);
            }
            if depth.is_some() && tile.depth.is_empty() {
                tile.depth = budget.vector((COV_TILE * COV_TILE) as usize)?;
                tile.depth.resize((COV_TILE * COV_TILE) as usize, 0.0);
            }
            for y in tr.y0..tr.y1 {
                budget.poll()?;
                let row = (y - rect.y0) as usize * w;
                let trow = ((y - ty * COV_TILE) * COV_TILE) as usize;
                for x in tr.x0..tr.x1 {
                    budget.tick(1)?;
                    let v = at(vals, row + (x - rect.x0) as usize)?;
                    if v <= 0.0 {
                        continue;
                    }
                    let ti = trow + (x - tx * COV_TILE) as usize;
                    if let Some(dp) = depth {
                        let dv = at(dp, row + (x - rect.x0) as usize)?;
                        let slot = tile.depth.get_mut(ti).ok_or(StrokeError::InvalidInput("depth index"))?;
                        *slot = slot.max(dv);
                    }
                    let c = at(&tile.cov, ti)?;
                    let nv = if ceil > c { c + v * (ceil - c) } else { c };
                    if nv <= c {
                        continue;
                    }
                    if let (true, Some(col)) = (want_col, color) {
                        let k = if c < 1.0 { (nv - c) / (1.0 - c) } else { 0.0 };
                        let p = tile.col.get_mut(ti * nc..(ti + 1) * nc).ok_or(StrokeError::InvalidInput("coverage color"))?;
                        for (i, value) in p.iter_mut().enumerate() {
                            *value = at(col, i)? * k + *value * (1.0 - k);
                        }
                    }
                    *tile.cov.get_mut(ti).ok_or(StrokeError::InvalidInput("coverage index"))? = nv;
                }
            }
        }
    }
    Ok(())
}
fn raster_dabs(
    ctx: &BrushContext,
    format: PixelFormat,
    dabs: &[Dab],
    dual: bool,
    cov: &mut CoverageMap,
    maximum: usize,
    budget: &mut Budget<'_, '_>,
) -> Result<()> {
    let mut scratch = budget.vector(maximum)?;
    let mut row = budget.vector(maximum)?;
    let mut depths = budget.vector(maximum)?;
    let mut depth_row = budget.vector(maximum)?;
    let mut native = [0.0f32; 8];
    for dab in dabs {
        budget.poll()?;
        let rect = ctx.dab_rect(dab, dual);
        check_rect(rect)?;
        let length = area(rect)?;
        if length > scratch.capacity() {
            return Err(StrokeError::Limit);
        }
        scratch.clear();
        depths.clear();
        let wet = !dual && ctx.brush.wet_edges;
        // Reusing the legacy rasterizer one absolute row at a time preserves
        // every coordinate/math operation while bounding cancellation latency.
        for y in rect.y0..rect.y1 {
            budget.poll()?;
            let strip = Rect::new(rect.x0, y, rect.x1, y + 1);
            ctx.rasterize_into(dab, dual, strip, &mut row, wet.then_some(&mut depth_row));
            scratch.extend_from_slice(&row);
            if wet {
                depths.extend_from_slice(&depth_row);
            }
        }
        let col = if !dual && cov.nc > 0 {
            from_rgba_into(&format, dab.color, &mut native);
            Some(&native[..cov.nc])
        } else {
            None
        };
        accumulate(cov, rect, &scratch, if dual { 1.0 } else { dab.opacity }, col, wet.then_some(&depths[..]), budget)?;
        if !dual {
            cov.bounds = cov.bounds.union(&rect);
        }
    }
    Ok(())
}

fn native_tiles(rect: Rect) -> Result<u64> {
    if rect.is_empty() {
        return Ok(0);
    }
    check_rect(rect)?;
    let size = photocraft_geom::TILE_SIZE;
    let w = i64::from((rect.x1 - 1).div_euclid(size)) - i64::from(rect.x0.div_euclid(size)) + 1;
    let h = i64::from((rect.y1 - 1).div_euclid(size)) - i64::from(rect.y0.div_euclid(size)) + 1;
    u64::try_from(w).ok().and_then(|w| u64::try_from(h).ok().and_then(|h| w.checked_mul(h))).ok_or(StrokeError::Limit)
}
fn clone_surface(source: &Surface, budget: &mut Budget<'_, '_>) -> Result<Surface> {
    budget.raster(source.try_validate_tile_origins_with(budget.control))?;
    let mut out = budget.raster(Surface::try_with_default_bytes(source.format(), source.default_bytes()))?;
    // Standard BTreeMap/Arc publication has no stable fallible allocator API;
    // map/tile headroom is precharged and every bounded insertion is polled.
    for (position, tile) in source.tiles() {
        budget.tick(1)?;
        out.put_tiles(std::iter::once((*position, Arc::clone(tile))));
    }
    budget.poll()?;
    Ok(out)
}
fn resident(source: &Surface, budget: &mut Budget<'_, '_>) -> Result<u64> {
    let minimum = (source.tile_count() as u64).checked_mul(256).ok_or(StrokeError::Limit)?;
    if minimum > budget.limits.resident_bytes {
        return Err(StrokeError::Limit);
    }
    let mut bytes = (source.default_bytes().len() as u64).checked_add(minimum).ok_or(StrokeError::Limit)?;
    for (_, tile) in source.tiles() {
        budget.tick(1)?;
        bytes = bytes.checked_add(tile.bytes().len() as u64).filter(|n| *n <= budget.limits.resident_bytes).ok_or(StrokeError::Limit)?;
    }
    Ok(bytes)
}
fn sort_keys(keys: &mut [(i32, i32)], budget: &mut Budget<'_, '_>) -> Result<()> {
    fn sift(keys: &mut [(i32, i32)], mut root: usize, end: usize, budget: &mut Budget<'_, '_>) -> Result<()> {
        loop {
            budget.tick(1)?;
            let child = root.checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(StrokeError::Limit)?;
            if child >= end {
                return Ok(());
            }
            let right = child + 1;
            let largest = if right < end && at(keys, right)? > at(keys, child)? { right } else { child };
            if at(keys, root)? >= at(keys, largest)? {
                return Ok(());
            }
            keys.swap(root, largest);
            root = largest;
        }
    }
    for root in (0..keys.len() / 2).rev() {
        sift(keys, root, keys.len(), budget)?;
    }
    for end in (1..keys.len()).rev() {
        budget.tick(1)?;
        keys.swap(0, end);
        sift(keys, 0, end, budget)?;
    }
    Ok(())
}
// Keep source, selected coverage, transparency policy and original budget explicit.
#[allow(clippy::too_many_arguments)]
fn composite(
    ctx: &BrushContext,
    cov: &CoverageMap,
    dual: Option<&CoverageMap>,
    pre: &Surface,
    target: &mut Surface,
    selection: Option<&Surface>,
    lock_transparency: bool,
    budget: &mut Budget<'_, '_>,
) -> Result<Rect> {
    let mut keys = budget.vector(cov.tiles.len())?;
    for key in cov.tiles.keys() {
        budget.tick(1)?;
        keys.push(*key);
    }
    sort_keys(&mut keys, budget)?;
    composite_keys(ctx, cov, dual, &keys, pre, target, selection, lock_transparency, budget)
}
#[allow(clippy::too_many_arguments)]
fn composite_keys(
    ctx: &BrushContext,
    cov: &CoverageMap,
    dual: Option<&CoverageMap>,
    keys: &[(i32, i32)],
    pre: &Surface,
    target: &mut Surface,
    selection: Option<&Surface>,
    lock_transparency: bool,
    budget: &mut Budget<'_, '_>,
) -> Result<Rect> {
    let bounds = cov.bounds;
    let fmt = target.format();
    let n = fmt.channels();
    let a_idx = alpha_index(&fmt);
    let nc = fmt.mode.color_channels();
    let b = &ctx.brush;
    let opacity = b.opacity.clamp(0.0, 1.0);
    let mut src = [0.0f32; 8];
    from_rgba_into(&fmt, b.color, &mut src);
    if let (photocraft_color::ColorMode::Cmyk, Some((rgb, inks))) = (fmt.mode, b.color_cmyk)
        && rgb == [b.color[0], b.color[1], b.color[2]]
    {
        src.get_mut(..4).ok_or(StrokeError::InvalidInput("CMYK inks"))?.copy_from_slice(&inks);
    }
    if let Some(a) = a_idx {
        *src.get_mut(a).ok_or(StrokeError::InvalidInput("alpha index"))? = b.color[3];
    }
    let count = (COV_TILE * COV_TILE) as usize;
    let mut raw = budget.vector(count * fmt.bytes_per_pixel())?;
    raw.resize(count * fmt.bytes_per_pixel(), 0);
    let mut region = budget.vector(count * n)?;
    let mut selected_raw = budget.vector(count * selection.map_or(0, |surface| surface.format().bytes_per_pixel()))?;
    selected_raw.resize(selected_raw.capacity(), 0);
    let mut selected = budget.vector(count * selection.map_or(0, Surface::channels))?;
    for &(tx, ty) in keys {
        budget.poll()?;
        let Some(tile) = cov.tiles.get(&(tx, ty)) else {
            continue;
        };
        let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&bounds);
        if tr.is_empty() {
            continue;
        }
        let pixels = area(tr)?;
        let raw_length = pixels.checked_mul(fmt.bytes_per_pixel()).ok_or(StrokeError::Limit)?;
        budget.raster(pre.try_read_interleaved_into_with(tr, raw.get_mut(..raw_length).ok_or(StrokeError::Limit)?, budget.control))?;
        region.clear();
        for sample in 0..pixels * n {
            budget.tick(1)?;
            region.push(read_sample(raw.get(..raw_length).ok_or(StrokeError::Limit)?, fmt.sample, sample));
        }
        selected.clear();
        if let Some(selection) = selection {
            let format = selection.format();
            let length = pixels.checked_mul(format.bytes_per_pixel()).ok_or(StrokeError::Limit)?;
            budget.raster(selection.try_read_interleaved_into_with(tr, selected_raw.get_mut(..length).ok_or(StrokeError::Limit)?, budget.control))?;
            for sample in 0..pixels * selection.channels() {
                budget.tick(1)?;
                selected.push(read_sample(selected_raw.get(..length).ok_or(StrokeError::Limit)?, format.sample, sample));
            }
        }
        let w = tr.width() as usize;
        for y in tr.y0..tr.y1 {
            budget.poll()?;
            for x in tr.x0..tr.x1 {
                budget.tick(1)?;
                let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                let c = at(&tile.cov, ti)?;
                if c <= 0.0 {
                    continue;
                }
                let i = (y - tr.y0) as usize * w + (x - tr.x0) as usize;
                let m = ctx.stroke_mask_wet(c, tile.depth.get(ti).copied(), dual.map(|surface| surface.get(x, y)), x, y);
                let s = if let Some(selection) = selection { at(&selected, i * selection.channels())? } else { 1.0 };
                let mut k = (m * opacity * s).min(1.0);
                if k <= 0.0 {
                    continue;
                }
                if b.mode == BlendMode::Dissolve {
                    k = if hash2(x, y, b.seed) < k { 1.0 } else { 0.0 };
                    if k == 0.0 {
                        continue;
                    }
                }
                let px = region.get_mut(i * n..(i + 1) * n).ok_or(StrokeError::InvalidInput("region index"))?;
                if b.erase {
                    if let (Some(a), false) = (a_idx, lock_transparency) {
                        *px.get_mut(a).ok_or(StrokeError::InvalidInput("alpha index"))? *= 1.0 - k;
                    }
                    if fmt.sample == SampleType::U8 && !b.aliased {
                        dither8(px, x, y);
                    }
                    continue;
                }
                if lock_transparency && a_idx.is_some_and(|a| px.get(a).is_some_and(|value| *value <= 0.0)) {
                    continue;
                }
                if cov.nc > 0 && !tile.col.is_empty() {
                    let p = tile.col.get(ti * nc..(ti + 1) * nc).ok_or(StrokeError::InvalidInput("color index"))?;
                    for ch in 0..nc {
                        *src.get_mut(ch).ok_or(StrokeError::InvalidInput("source index"))? = at(p, ch)? / c;
                    }
                }
                if matches!(b.mode, BlendMode::Normal | BlendMode::Dissolve) {
                    over_native(&fmt, px, src.get(..n).ok_or(StrokeError::InvalidInput("source channels"))?, k, lock_transparency);
                } else {
                    let d = to_rgba(&fmt, px);
                    let sr = to_rgba(&fmt, src.get(..n).ok_or(StrokeError::InvalidInput("source channels"))?);
                    let mut out = photocraft_color::blend::composite(b.mode, d, sr, k);
                    if lock_transparency {
                        out[3] = d[3];
                    }
                    let mut encoded = [0.0f32; 8];
                    from_rgba_into(&fmt, out, &mut encoded);
                    px.copy_from_slice(encoded.get(..n).ok_or(StrokeError::InvalidInput("encoded channels"))?);
                }
                if fmt.sample == SampleType::U8 && !b.aliased {
                    dither8(px, x, y);
                }
            }
        }
        for (sample, value) in region.iter().enumerate() {
            budget.tick(1)?;
            write_sample(raw.get_mut(..raw_length).ok_or(StrokeError::Limit)?, fmt.sample, sample, *value);
        }
        budget.raster(target.try_write_interleaved_with(tr, raw.get(..raw_length).ok_or(StrokeError::Limit)?, budget.control))?;
    }
    budget.poll()?;
    Ok(bounds)
}
/// Private output only: the caller owns admission, sorted key selection and
/// dirty-set publication. Both ordinary one-shot and live routes share math.
#[allow(clippy::too_many_arguments)]
pub(super) fn composite_existing_with_interrupt(
    ctx: &BrushContext,
    cov: &CoverageMap,
    dual: Option<&CoverageMap>,
    keys: &[(i32, i32)],
    pre: &Surface,
    target: &Surface,
    selection: Option<&Surface>,
    lock_transparency: bool,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    let mut budget = Budget { control, limits, work: 0, forward: None };
    budget.poll()?;
    let mut output = clone_surface(target, &mut budget)?;
    composite_keys(ctx, cov, dual, keys, pre, &mut output, selection, lock_transparency, &mut budget)?;
    budget.poll()?;
    Ok(output)
}
/// Restore the previous smoothing tail and composite the replacement union on
/// one private native surface. Equations mirror legacy read_region/write_region.
#[allow(clippy::too_many_arguments)]
pub(super) fn composite_existing_with_restore_with_interrupt(
    ctx: &BrushContext,
    cov: &CoverageMap,
    dual: Option<&CoverageMap>,
    keys: &[(i32, i32)],
    before: Rect,
    pre: &Surface,
    target: &Surface,
    selection: Option<&Surface>,
    lock_transparency: bool,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    let mut budget = Budget { control, limits, work: 0, forward: None };
    budget.poll()?;
    let standard = StrokeLimits::default();
    let format = target.format();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
        return Err(StrokeError::Limit);
    }
    if pre.format() != format || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32) || format.channels() == 0 || format.channels() > 8
    {
        return Err(StrokeError::Unsupported("ordinary union native format"));
    }
    if selection.is_some_and(|s| !matches!(s.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) || s.channels() == 0 || s.channels() > 8) {
        return Err(StrokeError::Unsupported("ordinary union selection format"));
    }
    let cells = (COV_TILE * COV_TILE) as u64;
    let native_tile = mul_checked((photocraft_geom::TILE_SIZE * photocraft_geom::TILE_SIZE) as u64, format.bytes_per_pixel() as u64)?;
    let maps = (target.tile_count() as u64).checked_mul(512).ok_or(StrokeError::Limit)?;
    let output = (keys.len() as u64)
        .checked_mul(4)
        .and_then(|n| n.checked_mul(native_tile.checked_add(512)?))
        .and_then(|n| n.checked_add(maps))
        .and_then(|n| n.checked_add(target.default_bytes().len() as u64))
        .ok_or(StrokeError::Limit)?;
    let sample = (format.bytes_per_pixel() as u64)
        .checked_add((format.channels() as u64).checked_mul(4).ok_or(StrokeError::Limit)?)
        .and_then(|n| n.checked_add(selection.map_or(0, |s| s.format().bytes_per_pixel() as u64 + s.channels() as u64 * 4)))
        .ok_or(StrokeError::Limit)?;
    // Both restoration and compositor scratch coexist through final publication.
    let scratch = cells.checked_mul(sample).and_then(|n| n.checked_mul(2)).ok_or(StrokeError::Limit)?;
    let work = (keys.len() as u64)
        .checked_mul(cells)
        .and_then(|n| n.checked_mul(4096))
        .and_then(|n| {
            n.checked_add((keys.len() as u64).checked_mul((target.tile_count() as u64).checked_add((keys.len() as u64).checked_mul(4)?)?)?.checked_mul(512)?)
        })
        .ok_or(StrokeError::Limit)?;
    if output.checked_add(scratch).is_none_or(|n| n > limits.resident_bytes) || work > limits.work_units {
        return Err(StrokeError::Limit);
    }
    let mut output = clone_surface(target, &mut budget)?;
    let n = format.channels();
    let count = (COV_TILE * COV_TILE) as usize;
    let length = count.checked_mul(format.bytes_per_pixel()).ok_or(StrokeError::Limit)?;
    let mut raw = budget.vector(length)?;
    raw.resize(length, 0);
    let mut values = budget.vector(count.checked_mul(n).ok_or(StrokeError::Limit)?)?;
    for &(tx, ty) in keys {
        budget.poll()?;
        let x = tx.checked_mul(COV_TILE).ok_or(StrokeError::Limit)?;
        let y = ty.checked_mul(COV_TILE).ok_or(StrokeError::Limit)?;
        let rect = Rect::new(x, y, x.checked_add(COV_TILE).ok_or(StrokeError::Limit)?, y.checked_add(COV_TILE).ok_or(StrokeError::Limit)?).intersect(&before);
        if rect.is_empty() {
            continue;
        }
        let pixels = area(rect)?;
        let bytes = pixels.checked_mul(format.bytes_per_pixel()).ok_or(StrokeError::Limit)?;
        budget.raster(pre.try_read_interleaved_into_with(rect, raw.get_mut(..bytes).ok_or(StrokeError::Limit)?, control))?;
        values.clear();
        let samples = pixels.checked_mul(n).ok_or(StrokeError::Limit)?;
        for sample in 0..samples {
            budget.tick(1)?;
            values.push(read_sample(raw.get(..bytes).ok_or(StrokeError::Limit)?, format.sample, sample));
        }
        for (sample, value) in values.iter().enumerate() {
            budget.tick(1)?;
            write_sample(raw.get_mut(..bytes).ok_or(StrokeError::Limit)?, format.sample, sample, *value);
        }
        budget.raster(output.try_write_interleaved_with(rect, raw.get(..bytes).ok_or(StrokeError::Limit)?, control))?;
    }
    composite_keys(ctx, cov, dual, keys, pre, &mut output, selection, lock_transparency, &mut budget)?;
    budget.poll()?;
    Ok(output)
}
fn mul_checked(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(StrokeError::Limit)
}
pub fn apply_stroke_with_interrupt(
    target: &mut Surface,
    stroke: &Stroke,
    selection: Option<&Surface>,
    lock_transparency: bool,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<Rect> {
    render_stroke_with_interrupt(target, &stroke.brush, &stroke.points, selection, lock_transparency, 1.0, limits, control)
}
/// Fixed straight preset reflection, preserving upstream pass order and stylus fields.
/// Path-based engine symmetry is a separate caller-owned geometry policy.
pub fn reflected_passes_with_interrupt(
    symmetry: crate::symmetry::PresetSymmetry,
    points: &[StrokePoint],
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<Vec<Vec<StrokePoint>>> {
    let mut budget = Budget { control, limits, work: 0, forward: None };
    budget.poll()?;
    if limits.resident_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK {
        return Err(StrokeError::Limit);
    }
    if points.len() > MAX_POINTS
        || !symmetry.rotation.is_finite()
        || symmetry.center.iter().any(|v| !v.is_finite() || v.abs() > crate::symmetry::MAX_COORDINATE)
    {
        return Err(StrokeError::InvalidInput("symmetry geometry"));
    }
    let count = symmetry.mirror_count();
    let bytes = (points.len() as u64)
        .checked_mul(count as u64)
        .and_then(|n| n.checked_mul(size_of::<StrokePoint>() as u64))
        .and_then(|n| n.checked_add((count as u64) * size_of::<Vec<StrokePoint>>() as u64))
        .ok_or(StrokeError::Limit)?;
    let work = (points.len() as u64).checked_mul(count as u64).and_then(|n| n.checked_mul(16)).ok_or(StrokeError::Limit)?;
    if bytes > limits.resident_bytes || work > limits.work_units {
        return Err(StrokeError::Limit);
    }
    let initial = match symmetry.mode {
        crate::symmetry::SymmetryMode::Horizontal => 0.0,
        crate::symmetry::SymmetryMode::Diagonal => 45.0,
        _ => 90.0,
    };
    let angle = (initial + symmetry.rotation).to_radians();
    let first = [angle.cos(), angle.sin()];
    if first.iter().any(|v| !v.is_finite()) {
        return Err(StrokeError::InvalidInput("symmetry angle"));
    }
    let directions = [first, [-first[1], first[0]]];
    let mut copies = budget.vector(count)?;
    for pass in 0..count {
        let mut output = budget.vector(points.len())?;
        for point in points {
            budget.tick(16)?;
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(StrokeError::InvalidInput("symmetry point"));
            }
            let mut reflected = *point;
            if pass == 2 {
                reflected.x = 2.0 * symmetry.center[0] - point.x;
                reflected.y = 2.0 * symmetry.center[1] - point.y;
            } else {
                let direction = directions.get(pass).copied().ok_or(StrokeError::InvalidInput("symmetry pass"))?;
                let [dx, dy] = [point.x - symmetry.center[0], point.y - symmetry.center[1]];
                let along = dx * direction[0] + dy * direction[1];
                reflected.x = symmetry.center[0] + 2.0 * along * direction[0] - dx;
                reflected.y = symmetry.center[1] + 2.0 * along * direction[1] - dy;
            }
            if !reflected.x.is_finite() || !reflected.y.is_finite() || reflected.x.abs() > f64::from(1 << 27) || reflected.y.abs() > f64::from(1 << 27) {
                return Err(StrokeError::InvalidInput("reflected coordinate"));
            }
            output.push(reflected);
        }
        copies.push(output);
    }
    budget.poll()?;
    Ok(copies)
}
/// Borrowed independent stroke passes (primary first). All coverage is max-unioned
/// before opacity/selection/native compositing once; equal coverage keeps the first
/// pass's color, while wet-edge depth takes its independent upstream maximum.
// Public controlled entry keeps source/selection policy, zoom, limits and cancellation explicit.
#[allow(clippy::too_many_arguments)]
pub fn render_stroke_passes_with_interrupt(
    target: &mut Surface,
    brush: &BrushSettings,
    passes: &[&[StrokePoint]],
    selection: Option<&Surface>,
    lock_transparency: bool,
    zoom: f32,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<Rect> {
    let mut budget = Budget { control, limits, work: 0, forward: None };
    budget.poll()?;
    if limits.resident_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK || passes.len() > 4 {
        return Err(StrokeError::Limit);
    }
    let count = passes.iter().filter(|p| !p.is_empty()).count();
    if count == 0 {
        return Ok(Rect::EMPTY);
    }
    let total_points = passes.iter().try_fold(0usize, |n, p| n.checked_add(p.len()).ok_or(StrokeError::Limit))?;
    if total_points > MAX_POINTS.checked_mul(4).ok_or(StrokeError::Limit)? {
        return Err(StrokeError::Limit);
    }
    // Each live pass owns at most one conservative partition, including its source
    // census; a separate partition covers merge map growth and final native output.
    // This is deliberately lower than the single-pass maximum, not a total-RSS claim.
    let share = limits.resident_bytes / ((count + 1) as u64);
    if share == 0 {
        return Err(StrokeError::Limit);
    }
    let mut combined: Option<(BrushContext, CoverageMap, CoverageMap)> = None;
    for points in passes.iter().copied().filter(|p| !p.is_empty()) {
        budget.limits.resident_bytes = share;
        let prepared = prepare_pass(target, brush, points, selection, zoom, &mut budget);
        budget.limits = limits;
        let (ctx, cov, dual) = prepared?;
        if let Some((_, to, to_dual)) = combined.as_mut() {
            union_max_controlled(to, cov, share, &mut budget)?;
            union_max_controlled(to_dual, dual, share, &mut budget)?;
        } else {
            combined = Some((ctx, cov, dual));
        }
    }
    let (ctx, cov, dual) = combined.ok_or(StrokeError::InvalidInput("missing stroke pass"))?;
    let native = native_tiles(cov.bounds)?;
    let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
        .checked_mul(photocraft_geom::TILE_SIZE as u64)
        .and_then(|n| n.checked_mul(target.format().bytes_per_pixel() as u64))
        .ok_or(StrokeError::Limit)?;
    let output = native
        .checked_mul(tile_bytes + 512)
        .and_then(|n| n.checked_add((target.tile_count() as u64) * 512))
        .and_then(|n| n.checked_add(1 << 20))
        .ok_or(StrokeError::Limit)?;
    if output > share {
        return Err(StrokeError::Limit);
    }
    budget.tick(
        native
            .checked_mul((target.tile_count() as u64).checked_add(native).ok_or(StrokeError::Limit)?)
            .and_then(|n| n.checked_mul(32))
            .ok_or(StrokeError::Limit)?,
    )?;
    let mut output = clone_surface(target, &mut budget)?;
    let damage = composite(&ctx, &cov, brush.dual_brush.enabled.then_some(&dual), target, &mut output, selection, lock_transparency, &mut budget)?;
    #[cfg(test)]
    publication_test::arrive();
    budget.poll()?;
    *target = output;
    Ok(damage)
}
fn union_max_controlled(to: &mut CoverageMap, other: CoverageMap, merge_limit: u64, budget: &mut Budget<'_, '_>) -> Result<()> {
    if to.nc != other.nc {
        return Err(StrokeError::InvalidInput("coverage channels"));
    }
    let prospective = to.tiles.len().checked_add(other.tiles.len()).ok_or(StrokeError::Limit)?;
    if (prospective as u64).checked_mul(512).is_none_or(|n| n > merge_limit) {
        return Err(StrokeError::Limit);
    }
    budget.poll()?;
    to.tiles.try_reserve(other.tiles.len()).map_err(|_| StrokeError::Allocation)?;
    if (to.tiles.capacity() as u64).checked_mul(256).is_none_or(|n| n > merge_limit) {
        return Err(StrokeError::Limit);
    }
    to.bounds = to.bounds.union(&other.bounds);
    for (key, mut source) in other.tiles {
        budget.tick(1)?;
        if let Some(target) = to.tiles.get_mut(&key) {
            if target.depth.is_empty() {
                target.depth = std::mem::take(&mut source.depth);
            } else {
                for (dst, src) in target.depth.iter_mut().zip(&source.depth) {
                    budget.tick(1)?;
                    *dst = dst.max(*src);
                }
            }
            for (index, (dst, src)) in target.cov.iter_mut().zip(&source.cov).enumerate() {
                budget.tick(1)?;
                if *src > *dst {
                    *dst = *src;
                    let start = index.checked_mul(to.nc).ok_or(StrokeError::Limit)?;
                    let end = start.checked_add(to.nc).ok_or(StrokeError::Limit)?;
                    if let (Some(dst), Some(src)) = (target.col.get_mut(start..end), source.col.get(start..end)) {
                        for (d, s) in dst.iter_mut().zip(src) {
                            budget.tick(1)?;
                            *d = *s;
                        }
                    }
                }
            }
        } else {
            to.tiles.insert(key, source);
        }
    }
    budget.poll()?;
    Ok(())
}
// Public controlled entry keeps source/selection policy, zoom, limits and cancellation explicit.
#[allow(clippy::too_many_arguments)]
pub fn render_stroke_with_interrupt(
    target: &mut Surface,
    brush: &BrushSettings,
    points: &[StrokePoint],
    selection: Option<&Surface>,
    lock_transparency: bool,
    zoom: f32,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<Rect> {
    // Single-pass keeps its previous admitted maximum and exact equations.
    let mut budget = Budget { control, limits, work: 0, forward: None };
    budget.poll()?;
    if limits.resident_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK {
        return Err(StrokeError::Limit);
    }
    if points.is_empty() {
        return Ok(Rect::EMPTY);
    }
    let (ctx, cov, dual) = prepare_pass(target, brush, points, selection, zoom, &mut budget)?;
    let mut output = clone_surface(target, &mut budget)?;
    let damage = composite(&ctx, &cov, brush.dual_brush.enabled.then_some(&dual), target, &mut output, selection, lock_transparency, &mut budget)?;
    #[cfg(test)]
    publication_test::arrive();
    budget.poll()?;
    *target = output;
    Ok(damage)
}

// Replay only the unchanged smoother, not the walker or dab builders. Each
// push emits at most 64 positions and finish emits at most one; the fixed
// scratch dies before the separately admitted generation buffers are allocated.
// Per segment, 8 * L1 length plus two entries conservatively covers the
// distance walk (minimum increment .25), pixel-grid walk (rounded cell endpoints)
// and speed walk (at most ceil(length / .5)). The factor two above 4 * L1 also
// leaves floating arithmetic slack; no path-length contraction theorem or
// rectangle-diagonal-times-input-count assumption is needed. Build-up contributes
// at most 512 further entries per emitted position (514 includes segment slack).
fn smoothed_step_bound(brush: &BrushSettings, points: &[StrokePoint], zoom: f32, retained: u64, budget: &mut Budget<'_, '_>) -> Result<u64> {
    let scratch_bytes = (64u64)
        .checked_mul(size_of::<StrokePoint>() as u64)
        .and_then(|n| n.checked_add(size_of::<Vec<StrokePoint>>() as u64))
        .and_then(|n| n.checked_add(size_of::<Smoother>() as u64))
        .and_then(|n| n.checked_add(4096))
        .ok_or(StrokeError::Limit)?;
    budget.poll()?;
    if retained.checked_add(scratch_bytes).is_none_or(|n| n > budget.limits.resident_bytes) {
        return Err(StrokeError::Limit);
    }
    budget.tick(64)?;
    let mut scratch = budget.vector(64)?;
    let mut smoother = Smoother::new(&brush.smoothing, zoom);
    let mut last: Option<StrokePoint> = None;
    let mut steps = 2u64;
    let air = if brush.build_up { 514u64 } else { 2 };
    // Consume the very same computed positions used by generate(). Pose affects
    // pressure/tilt/rotation only, so it does not change this geometric bound.
    let mut measure = |positions: &[StrokePoint], budget: &mut Budget<'_, '_>| -> Result<()> {
        for p in positions {
            budget.tick(64)?;
            if !p.x.is_finite() || !p.y.is_finite() {
                return Err(StrokeError::InvalidInput("smoothed coordinates"));
            }
            if let Some(previous) = last {
                let travel = ((p.x - previous.x).abs() + (p.y - previous.y).abs()) * 8.0;
                let travel = travel.ceil();
                if !travel.is_finite() || travel > MAX_DABS as f64 {
                    return Err(StrokeError::Limit);
                }
                steps = steps.checked_add(travel as u64).ok_or(StrokeError::Limit)?;
            }
            steps = steps.checked_add(air).filter(|n| *n <= MAX_DABS as u64).ok_or(StrokeError::Limit)?;
            last = Some(*p);
        }
        Ok(())
    };
    for p in points {
        // Bound the full 64-iteration smoother push, including its scalar powf,
        // before it runs. Measurement and later emission are independent work.
        budget.tick(64 * 256)?;
        scratch.clear();
        smoother.push(*p, &mut scratch);
        measure(&scratch, budget)?;
    }
    budget.tick(256)?;
    scratch.clear();
    smoother.finish(&mut scratch);
    measure(&scratch, budget)?;
    budget.poll()?;
    Ok(steps)
}

fn prepare_pass(
    target: &Surface,
    brush: &BrushSettings,
    points: &[StrokePoint],
    selection: Option<&Surface>,
    zoom: f32,
    budget: &mut Budget<'_, '_>,
) -> Result<(BrushContext, CoverageMap, CoverageMap)> {
    let limits = budget.limits;
    budget.poll()?;
    if limits.resident_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK {
        return Err(StrokeError::Limit);
    }
    if points.is_empty() {
        return Err(StrokeError::InvalidInput("empty private pass"));
    }
    let fmt = target.format();
    if !matches!(fmt.sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Err(StrokeError::Unsupported("native wide stroke arithmetic"));
    }
    if fmt.channels() == 0 || fmt.channels() > 8 {
        return Err(StrokeError::Unsupported("channel count"));
    }
    if let Some(selection) = selection
        && (!matches!(selection.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) || selection.channels() == 0 || selection.channels() > 8)
    {
        return Err(StrokeError::Unsupported("selection arithmetic"));
    }
    if points.len() > MAX_POINTS || !zoom.is_finite() {
        return Err(StrokeError::InvalidInput("points/zoom"));
    }
    validate_brush(brush)?;
    let assets = asset_pixels(brush)?;
    let mut distance = 0.0;
    let mut last: Option<&StrokePoint> = None;
    for point in points {
        budget.tick(1)?;
        if !point.x.is_finite() || !point.y.is_finite() || !point.time.is_finite() || point.x.abs() > f64::from(1 << 27) || point.y.abs() > f64::from(1 << 27) {
            return Err(StrokeError::InvalidInput("point coordinates/time"));
        }
        finite(&[point.pressure, point.tilt_x, point.tilt_y, point.rotation, point.wheel])?;
        if let Some(previous) = last {
            distance += (point.x - previous.x).hypot(point.y - previous.y);
        }
        last = Some(point);
    }
    let mut retained_sources = None;
    let steps = if brush.smoothing.amount > 0.0 {
        let source = resident(target, budget)?;
        let selected = if let Some(selection) = selection { resident(selection, budget)? } else { 0 };
        let source_maps = (target.tile_count() as u64)
            .checked_add(selection.map_or(0, |surface| surface.tile_count() as u64))
            .and_then(|n| n.checked_mul(768))
            .ok_or(StrokeError::Limit)?;
        let retained = source
            .checked_add(selected)
            .and_then(|n| n.checked_add(source_maps))
            .and_then(|n| n.checked_add(assets.checked_mul(2)?))
            .and_then(|n| n.checked_add((points.len() as u64).checked_mul(size_of::<StrokePoint>() as u64)?))
            .ok_or(StrokeError::Limit)?;
        let steps = smoothed_step_bound(brush, points, zoom, retained, budget)?;
        retained_sources = Some((source, selected, source_maps));
        steps
    } else {
        let air = if brush.build_up { 514u64 } else { 2 };
        let smoothed = (points.len() as u64).checked_add(1).ok_or(StrokeError::Limit)?;
        let steps = (distance * 4.0).ceil();
        if !steps.is_finite() || steps > MAX_DABS as f64 {
            return Err(StrokeError::Limit);
        }
        (steps as u64).checked_add(smoothed.checked_mul(air).ok_or(StrokeError::Limit)?).and_then(|n| n.checked_add(2)).ok_or(StrokeError::Limit)?
    };
    let count = if brush.scattering.enabled { brush.scattering.count.clamp(1, 16) as u64 } else { 1 };
    let primary = steps.checked_mul(count).filter(|n| *n <= MAX_DABS as u64).ok_or(StrokeError::Limit)?;
    let dual = if brush.dual_brush.enabled {
        steps.checked_mul(brush.dual_brush.count.clamp(1, 16) as u64).filter(|n| *n <= MAX_DABS as u64).ok_or(StrokeError::Limit)?
    } else {
        0
    };
    let (source, selected, source_maps) = if let Some(retained) = retained_sources {
        retained
    } else {
        let source = resident(target, budget)?;
        let selected = if let Some(selection) = selection { resident(selection, budget)? } else { 0 };
        let source_maps = (target.tile_count() as u64)
            .checked_add(selection.map_or(0, |surface| surface.tile_count() as u64))
            .and_then(|n| n.checked_mul(768))
            .ok_or(StrokeError::Limit)?;
        (source, selected, source_maps)
    };
    let generation = primary
        .checked_add(dual)
        .and_then(|n| n.checked_mul(size_of::<Dab>() as u64))
        .and_then(|n| n.checked_add(steps.checked_mul(size_of::<StepInput>() as u64)?))
        .and_then(|n| n.checked_add(assets.checked_mul(2)?))
        .and_then(|n| n.checked_add((points.len() as u64).checked_mul(size_of::<StrokePoint>() as u64)?))
        .and_then(|n| n.checked_add(4096))
        .ok_or(StrokeError::Limit)?;
    let base = source.checked_add(selected).and_then(|n| n.checked_add(source_maps)).and_then(|n| n.checked_add(generation)).ok_or(StrokeError::Limit)?;
    let generation_work = steps.checked_mul(256).and_then(|n| n.checked_add(assets.checked_mul(256)?)).ok_or(StrokeError::Limit)?;
    if base > limits.resident_bytes || generation_work > limits.work_units.saturating_sub(budget.work) {
        return Err(StrokeError::Limit);
    }
    let numeric = numeric_brush(brush);
    let (primary, dual) = generate(&numeric, points, zoom, steps as usize, primary as usize, dual as usize, budget)?;
    // Sample preparation is admitted before allocation; no unchecked BrushContext
    // constructor or raw asset clone occurs in this controlled lane.
    let ctx = BrushContext {
        brush: numeric,
        tip: mips(&brush.tip, budget)?,
        dual_tip: if brush.dual_brush.enabled { mips(&brush.dual_brush.tip, budget)? } else { None },
        texture: pattern(brush, budget)?,
    };
    let per_color = brush.color_dynamics.enabled && !brush.erase;
    let nc = if per_color { fmt.mode.color_channels() } else { 0 };
    let mut total_pixels = 0u64;
    let mut tiles = 0u64;
    let mut maximum = 0usize;
    let mut union = Rect::EMPTY;
    for (dab, is_dual) in primary.iter().map(|dab| (dab, false)).chain(dual.iter().map(|dab| (dab, true))) {
        budget.poll()?;
        if !dab.center.x.is_finite() || !dab.center.y.is_finite() {
            return Err(StrokeError::InvalidInput("dab center"));
        }
        finite(&[dab.radius, dab.alpha, dab.opacity, dab.angle, dab.roundness, dab.proj_scale, dab.proj_angle, dab.depth])?;
        finite(&dab.color)?;
        let rect = ctx.dab_rect(dab, is_dual);
        check_rect(rect)?;
        let pixels = area(rect)?;
        maximum = maximum.max(pixels);
        total_pixels = total_pixels.checked_add(pixels as u64).ok_or(StrokeError::Limit)?;
        tiles = tiles.checked_add(cov_tiles(rect)?).ok_or(StrokeError::Limit)?;
        if !is_dual {
            union = union.union(&rect);
        }
    }
    let cov_count = tiles;
    let native = native_tiles(union)?;
    let tile_bytes = photocraft_geom::TILE_SIZE as u64 * photocraft_geom::TILE_SIZE as u64 * fmt.bytes_per_pixel() as u64;
    let extra = cov_count
        .checked_mul((COV_TILE * COV_TILE) as u64)
        .and_then(|n| n.checked_mul((nc as u64 + 2) * 4))
        .and_then(|n| n.checked_add(tiles.checked_mul(2048)?))
        .and_then(|n| n.checked_add(native.checked_mul(tile_bytes + 512)?))
        .and_then(|n| n.checked_add((maximum as u64).checked_mul(16)?))
        .and_then(|n| n.checked_add(1 << 20))
        .ok_or(StrokeError::Limit)?;
    let source_tiles = target.tile_count() as u64;
    let work = total_pixels
        .checked_mul(256)
        .and_then(|n| n.checked_add(tiles.checked_mul(native.checked_add(source_tiles)?)?.checked_mul(32)?))
        .and_then(|n| n.checked_add(source_tiles.checked_mul(source_tiles)?.checked_mul(16)?))
        .and_then(|n| n.checked_add(generation_work))
        .ok_or(StrokeError::Limit)?;
    if base.checked_add(extra).is_none_or(|n| n > limits.resident_bytes) || work > limits.work_units.saturating_sub(budget.work) {
        return Err(StrokeError::Limit);
    }
    let mut cov = CoverageMap::new(nc);
    let mut dual_cov = CoverageMap::new(0);
    let tile_capacity = usize::try_from(tiles).map_err(|_| StrokeError::Limit)?;
    cov.tiles.try_reserve(tile_capacity).map_err(|_| StrokeError::Allocation)?;
    if brush.dual_brush.enabled {
        dual_cov.tiles.try_reserve(tile_capacity).map_err(|_| StrokeError::Allocation)?;
    }
    let actual_map = (cov.tiles.capacity() as u64).checked_add(dual_cov.tiles.capacity() as u64).and_then(|n| n.checked_mul(128)).ok_or(StrokeError::Limit)?;
    if actual_map > tiles.checked_mul(2048).and_then(|n| n.checked_add(4096)).ok_or(StrokeError::Limit)? {
        return Err(StrokeError::Limit);
    }
    raster_dabs(&ctx, fmt, &primary, false, &mut cov, maximum, budget)?;
    raster_dabs(&ctx, fmt, &dual, true, &mut dual_cov, maximum, budget)?;
    budget.poll()?;
    Ok((ctx, cov, dual_cov))
}

#[cfg(test)]
pub(super) mod publication_test {
    use std::{
        cell::RefCell,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    std::thread_local! { static ARMED: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) }; }
    pub struct Fence(Arc<AtomicBool>);
    impl Fence {
        pub fn arm() -> Self {
            let flag = Arc::new(AtomicBool::new(false));
            ARMED.with(|v| *v.borrow_mut() = Some(flag.clone()));
            Self(flag)
        }
        pub fn cancelled(&self) -> bool {
            self.0.load(Ordering::Relaxed)
        }
    }
    impl Drop for Fence {
        fn drop(&mut self) {
            ARMED.with(|v| *v.borrow_mut() = None);
        }
    }
    pub fn arrive() {
        ARMED.with(|v| {
            if let Some(flag) = v.borrow().as_ref() {
                flag.store(true, Ordering::Relaxed);
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_curve_admission_preserves_exact_legacy_native_pixels_at_all_zooms() {
        let points: Vec<_> = (0..40)
            .map(|i| {
                let t = f64::from(i) / 39.0;
                StrokePoint::new(40.0 + 120.0 * t, 60.0 + 30.0 * (7.0 * t).sin(), (0.3 + 0.7 * t) as f32)
            })
            .collect();
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16] {
            for zoom in [0.25, 1.0, 4.0, 16.0] {
                for (amount, pulled_string) in [(0.1, false), (0.5, false), (0.3, true)] {
                    let mut brush = BrushSettings { size: 40.0, hardness: 0.0, seed: 71, ..Default::default() };
                    brush.smoothing.amount = amount;
                    brush.smoothing.pulled_string = pulled_string;
                    let mut expected = Surface::new(format);
                    expected.fill_rect(Rect::new(0, 0, 200, 120), &[0.1, 0.2, 0.3, 1.0]);
                    let mut actual = expected.clone();
                    let damage = crate::render_stroke(&mut expected, &brush, &points, None, false, zoom);
                    let actual_damage =
                        render_stroke_with_interrupt(&mut actual, &brush, &points, None, false, zoom, StrokeLimits::default(), &Interrupt::NONE).unwrap();
                    assert_eq!(actual_damage, damage, "{format:?} {zoom} {amount} {pulled_string}");
                    assert_eq!(actual, expected, "{format:?} {zoom} {amount} {pulled_string}");
                    assert!(actual.rgba(40, 60)[0] < 0.09);
                }
            }
        }
    }

    #[test]
    fn smoothing_geometry_bound_covers_timestamps_grid_speed_buildup_scatter_and_dual() {
        let points = [
            StrokePoint { x: -7.25, y: 1.2, time: 0.0, ..Default::default() },
            StrokePoint { x: 18.75, y: -11.3, time: 2048.0, pressure: 0.2, ..Default::default() },
            StrokePoint { x: 18.75, y: -11.3, time: 8192.0, ..Default::default() },
            StrokePoint { x: -19.1, y: 14.5, time: 8193.0, pressure: 0.7, ..Default::default() },
        ];
        for mode in 0..6 {
            for pulled in [false, true] {
                for zoom in [0.01, 0.25, 16.0] {
                    let mut brush = BrushSettings { size: 1.0, spacing: 0.01, seed: 93, ..Default::default() };
                    brush.smoothing.amount = 0.9;
                    brush.smoothing.pulled_string = pulled;
                    brush.spacing_enabled = mode != 1;
                    brush.aliased = mode == 2;
                    brush.build_up = mode == 3;
                    brush.build_up_rate = 1000.0;
                    brush.scattering.enabled = mode == 4;
                    brush.scattering.count = 3;
                    brush.dual_brush.enabled = mode == 5;
                    brush.dual_brush.count = 2;
                    let numeric = numeric_brush(&brush);
                    let mut budget = Budget { control: &Interrupt::NONE, limits: StrokeLimits::default(), work: 0, forward: None };
                    let steps = smoothed_step_bound(&numeric, &points, zoom, 0, &mut budget).unwrap();
                    let primary_limit = steps * if brush.scattering.enabled { 3 } else { 1 };
                    let dual_limit = if brush.dual_brush.enabled { steps * 2 } else { 0 };
                    assert!(primary_limit <= MAX_DABS as u64 && dual_limit <= MAX_DABS as u64);
                    let (primary, dual) = generate(&numeric, &points, zoom, steps as usize, primary_limit as usize, dual_limit as usize, &mut budget).unwrap();
                    let mut legacy = crate::dynamics::DabGenerator::new(&numeric, zoom);
                    let mut expected = Vec::new();
                    let mut expected_dual = Vec::new();
                    legacy.push(&points, &mut expected, &mut expected_dual);
                    legacy.finish(&mut expected, &mut expected_dual);
                    assert_eq!(primary, expected, "mode={mode} pulled={pulled} zoom={zoom}");
                    assert_eq!(dual, expected_dual, "mode={mode} pulled={pulled} zoom={zoom}");
                }
            }
        }
    }

    #[test]
    fn smoothing_over_cap_and_planning_work_resident_cancel_refuse_before_publication() {
        let mut brush = BrushSettings { size: 4.0, ..Default::default() };
        brush.smoothing.amount = 0.3;
        let far = [StrokePoint::new(-1_000_000.0, 0.0, 1.0), StrokePoint::new(1_000_000.0, 0.0, 1.0)];
        let before = Surface::new(PixelFormat::RGBA16);
        let mut actual = before.clone();
        assert_eq!(
            render_stroke_with_interrupt(&mut actual, &brush, &far, None, false, 1.0, StrokeLimits::default(), &Interrupt::NONE),
            Err(StrokeError::Limit)
        );
        assert_eq!(actual, before);
        let near = [StrokePoint::new(2.0, 2.0, 1.0), StrokePoint::new(10.0, 4.0, 1.0)];
        for limits in
            [StrokeLimits { resident_bytes: 1, work_units: MAX_WORK }, StrokeLimits { resident_bytes: StrokeLimits::default().resident_bytes, work_units: 100 }]
        {
            assert_eq!(render_stroke_with_interrupt(&mut actual, &brush, &near, None, false, 1.0, limits, &Interrupt::NONE), Err(StrokeError::Limit));
            assert_eq!(actual, before);
        }
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4;
        let mut budget = Budget { control: &Interrupt::cancel_only(&cancel), limits: StrokeLimits::default(), work: 0, forward: None };
        assert_eq!(smoothed_step_bound(&brush, &near, 1.0, 0, &mut budget), Err(StrokeError::Cancelled));
        assert!(calls.load(std::sync::atomic::Ordering::Relaxed) >= 5);
        assert_eq!(actual, before);
    }

    #[test]
    fn controlled_matches_current_brush_pencil_wet_block_cmyk_and_native_depths() {
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::CMYKA8] {
            for mode in 0..9 {
                let mut brush =
                    BrushSettings { size: 5.0, pressure_size: false, opacity: 0.7, flow: 0.3, color: [0.2, 0.7, 0.3, 1.0], seed: 7, ..Default::default() };
                if mode == 1 {
                    brush.aliased = true;
                }
                if mode == 2 {
                    brush.erase = true;
                }
                if mode == 3 {
                    brush.mode = BlendMode::Multiply;
                }
                if mode == 4 {
                    brush.color_dynamics.enabled = true;
                    brush.color_dynamics.hue_jitter = 0.1;
                    brush.dual_brush.enabled = true;
                    brush.dual_brush.size = 3.0;
                    brush.texture.enabled = true;
                    brush.texture.pattern = Pattern::Procedural { style: PatternStyle::Canvas, size: 8, seed: 1 };
                }
                if mode == 5 {
                    brush.wet_edges = true;
                }
                if mode == 6 {
                    brush.aliased = true;
                    brush.size = 1.0;
                    brush.spacing = 0.5;
                }
                if mode == 7 {
                    brush.aliased = true;
                    brush.square = true;
                }
                if mode == 8 {
                    brush.color_cmyk = Some(([0.2, 0.7, 0.3], [0.12, 0.23, 0.34, 0.45]));
                    brush.smoothing.enabled = false;
                    brush.smoothing.amount = 0.8;
                }
                let points = [StrokePoint::new(3.0, 3.0, 1.0), StrokePoint::new(12.0, 5.0, 0.6), StrokePoint::new(6.0, 9.0, 0.9)];
                let mut expected = Surface::new(format);
                let background = if format.mode == photocraft_color::ColorMode::Grayscale {
                    vec![0.6, 0.8]
                } else if format.mode == photocraft_color::ColorMode::Cmyk {
                    vec![0.1, 0.2, 0.3, 0.4, 0.8]
                } else {
                    vec![0.6, 0.5, 0.4, 0.8]
                };
                expected.fill_rect(Rect::new(-3, -3, 18, 18), &background);
                let mut actual = expected.clone();
                let mut selection = Surface::new(PixelFormat::GRAY8);
                selection.fill_rect(Rect::new(-3, -3, 9, 18), &[0.6]);
                let damage = super::super::render_stroke(&mut expected, &brush, &points, Some(&selection), mode == 3, 1.0);
                let controlled =
                    render_stroke_with_interrupt(&mut actual, &brush, &points, Some(&selection), mode == 3, 1.0, StrokeLimits::default(), &Interrupt::NONE)
                        .unwrap();
                assert_eq!(damage, controlled);
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn sampled_assets_and_smoothing_match_current_dab_and_stroke_order() {
        let mut brush = BrushSettings { size: 5.0, pressure_size: false, seed: 11, ..Default::default() };
        brush.tip = TipShape::Sampled(crate::GrayTile::from_fn(3, 2, |x, y| (x + y) as f32 / 3.0));
        brush.texture.enabled = true;
        brush.texture.pattern = Pattern::Tile(crate::GrayTile::from_fn(2, 3, |x, y| (x + y) as f32 / 3.0));
        brush.smoothing.amount = 0.3;
        let points = [
            StrokePoint::new(2.0, 3.0, 1.0),
            StrokePoint { x: 9.0, y: 8.0, time: 32.0, ..Default::default() },
            StrokePoint { x: 5.0, y: 12.0, time: 64.0, ..Default::default() },
        ];
        let stroke = Stroke { brush: brush.clone(), points: points.to_vec() };
        let mut budget = Budget { control: &Interrupt::NONE, limits: StrokeLimits::default(), work: 0, forward: None };
        let numeric = numeric_brush(&brush);
        let (dabs, _) = generate(&numeric, &points, 1.0, 100_000, 100_000, 0, &mut budget).unwrap();
        assert_eq!(dabs, crate::dabs(&stroke));
        let mut expected = Surface::new(PixelFormat::RGBA32F);
        let mut actual = expected.clone();
        let damage = super::super::render_stroke(&mut expected, &brush, &points, None, false, 1.0);
        assert_eq!(render_stroke_with_interrupt(&mut actual, &brush, &points, None, false, 1.0, StrokeLimits::default(), &Interrupt::NONE).unwrap(), damage);
        assert_eq!(actual, expected);
    }
    #[test]
    fn stored_preview_and_loaded_assets_match_current_renderer() {
        let preview = Arc::new(crate::GrayTile::from_fn(2, 2, |x, y| (x + y) as f32 / 2.0));
        let full = Arc::new(crate::GrayTile::from_fn(3, 4, |x, y| (x + y) as f32 / 5.0));
        for loaded in [false, true] {
            let stored = crate::StoredTile { key: "fixture".into(), width: 3, height: 4, preview: preview.clone(), full: loaded.then(|| full.clone()) };
            let mut brush = BrushSettings { size: 4.0, pressure_size: false, tip: TipShape::Stored(stored.clone()), ..Default::default() };
            brush.texture.enabled = true;
            brush.texture.pattern = Pattern::Stored(stored);
            brush.wet_edges = true;
            let points = [StrokePoint::new(1.0, 2.0, 1.0), StrokePoint::new(7.0, 6.0, 0.7)];
            let mut expected = Surface::new(PixelFormat::RGBA16);
            let mut actual = expected.clone();
            let damage = crate::render_stroke(&mut expected, &brush, &points, None, false, 1.0);
            assert_eq!(
                render_stroke_with_interrupt(&mut actual, &brush, &points, None, false, 1.0, StrokeLimits::default(), &Interrupt::NONE).unwrap(),
                damage
            );
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn reflected_passes_and_coverage_union_match_current_upstream_symmetry() {
        let points = [StrokePoint::new(3.0, 3.0, 1.0), StrokePoint { x: 9.0, y: 7.0, time: 32.0, pressure: 0.7, ..Default::default() }];
        for mode in crate::symmetry::SymmetryMode::ALL {
            let axis = crate::symmetry::PresetSymmetry::new(mode, [6.0, 6.0], 17.0).unwrap();
            let mirrors = reflected_passes_with_interrupt(axis, &points, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(mirrors, axis.reflected_passes(&points));
            for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8] {
                let mut brush = BrushSettings { size: 5.0, opacity: 0.5, flow: 0.7, pressure_size: false, color: [0.3, 0.7, 0.2, 1.0], ..Default::default() };
                brush.wet_edges = true;
                brush.color_dynamics.enabled = true;
                brush.color_dynamics.hue_jitter = 0.2;
                brush.dual_brush.enabled = true;
                brush.dual_brush.size = 4.0;
                let mut expected = Surface::new(format);
                let mut actual = expected.clone();
                let mut primary = StrokeRenderer::new(&brush, Some(format), 1.0);
                primary.push(&points);
                primary.finish();
                let others: Vec<_> = mirrors
                    .iter()
                    .map(|p| {
                        let mut r = StrokeRenderer::new(&brush, Some(format), 1.0);
                        r.push(p);
                        r.finish();
                        r
                    })
                    .collect();
                let pre = expected.clone();
                let damage = primary.composite_union_many(others.iter(), &pre, &mut expected, None, false);
                let passes: Vec<&[StrokePoint]> = std::iter::once(points.as_slice()).chain(mirrors.iter().map(Vec::as_slice)).collect();
                assert_eq!(
                    render_stroke_passes_with_interrupt(&mut actual, &brush, &passes, None, false, 1.0, StrokeLimits::default(), &Interrupt::NONE).unwrap(),
                    damage
                );
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn duplicate_pass_has_one_opacity_ceiling_and_late_union_cancel_is_atomic() {
        let brush = BrushSettings { size: 4.0, opacity: 0.4, flow: 0.8, pressure_size: false, ..Default::default() };
        let points = [StrokePoint::new(2.0, 2.0, 1.0), StrokePoint::new(8.0, 6.0, 1.0)];
        let passes = [points.as_slice(), points.as_slice()];
        let before = Surface::new(PixelFormat::RGBA16);
        let mut expected = before.clone();
        let mut actual = before.clone();
        render_stroke_with_interrupt(&mut expected, &brush, &points, None, false, 1.0, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let observe = || {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            false
        };
        render_stroke_passes_with_interrupt(&mut actual, &brush, &passes, None, false, 1.0, StrokeLimits::default(), &Interrupt::cancel_only(&observe))
            .unwrap();
        assert_eq!(actual, expected);
        assert!(calls.load(std::sync::atomic::Ordering::Relaxed) > 1);
        let fence = publication_test::Fence::arm();
        let cancel = || fence.cancelled();
        let mut failed = before.clone();
        assert_eq!(
            render_stroke_passes_with_interrupt(&mut failed, &brush, &passes, None, false, 1.0, StrokeLimits::default(), &Interrupt::cancel_only(&cancel))
                .unwrap_err(),
            StrokeError::Cancelled
        );
        assert!(fence.cancelled());
        assert_eq!(failed, before);
    }
    #[test]
    fn last_fence_cancel_and_budget_or_wide_refusal_preserve_caller_surface() {
        let brush = BrushSettings { size: 3.0, pressure_size: false, ..Default::default() };
        let points = [StrokePoint::new(2.0, 2.0, 1.0), StrokePoint::new(6.0, 2.0, 1.0)];
        let before = Surface::new(PixelFormat::RGBA16);
        let mut rehearsal = before.clone();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let observe = || {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            false
        };
        render_stroke_with_interrupt(&mut rehearsal, &brush, &points, None, false, 1.0, StrokeLimits::default(), &Interrupt::cancel_only(&observe)).unwrap();
        assert!(calls.load(std::sync::atomic::Ordering::Relaxed) > 1);
        let fence = publication_test::Fence::arm();
        let cancel = || fence.cancelled();
        let mut actual = before.clone();
        assert_eq!(
            render_stroke_with_interrupt(&mut actual, &brush, &points, None, false, 1.0, StrokeLimits::default(), &Interrupt::cancel_only(&cancel))
                .unwrap_err(),
            StrokeError::Cancelled
        );
        assert!(fence.cancelled());
        assert_eq!(actual, before);
        assert!(
            render_stroke_with_interrupt(
                &mut actual,
                &brush,
                &points,
                None,
                false,
                1.0,
                StrokeLimits { resident_bytes: 1, work_units: MAX_WORK },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(actual, before);
        let mut wide = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, SampleType::F64, true));
        let before = wide.clone();
        assert!(matches!(
            render_stroke_with_interrupt(&mut wide, &brush, &points, None, false, 1.0, StrokeLimits::default(), &Interrupt::NONE),
            Err(StrokeError::Unsupported(_))
        ));
        assert_eq!(wide, before);
    }
}

#[path = "native_stroke_controlled.rs"]
pub mod native;
