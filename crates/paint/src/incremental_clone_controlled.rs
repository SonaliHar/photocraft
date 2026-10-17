//! Exact private snapshots of the upstream incremental painter, without replay.
//! This admits and copies retained state only. Existing incremental push/composite
//! still require their own work/allocation admission; no deep-cancellation claim.
use super::*;
use photocraft_raster::Interrupt;
use std::mem::size_of;
type Result<T> = std::result::Result<T, StrokeError>;
const MAP_ENTRY_BYTES: u64 = 256;
const SET_ENTRY_BYTES: u64 = 64;

#[derive(Clone, Copy, Debug, Default)]
pub struct IncrementalCloneRequirements {
    pub retained_bytes: u64,
    pub staged_bytes: u64,
    pub work_units: u64,
}
impl IncrementalCloneRequirements {
    pub(crate) fn add(&mut self, other: Self, limits: StrokeLimits) -> Result<()> {
        self.retained_bytes = sum(self.retained_bytes, other.retained_bytes)?;
        self.staged_bytes = sum(self.staged_bytes, other.staged_bytes)?;
        self.work_units = sum(self.work_units, other.work_units)?;
        if sum(self.retained_bytes, self.staged_bytes)? > limits.resident_bytes || self.work_units > limits.work_units {
            return Err(StrokeError::Limit);
        }
        Ok(())
    }
}
pub struct IncrementalStrokeState {
    pub renderer: StrokeRenderer,
    pub mirrors: Vec<StrokeRenderer>,
    pub union: Option<UnionLive>,
}
fn sum(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(StrokeError::Limit)
}
fn scaled(n: usize, bytes: usize) -> Result<u64> {
    (n as u64).checked_mul(bytes as u64).ok_or(StrokeError::Limit)
}
fn table_work(capacity: usize, entries: usize) -> Result<u64> {
    (capacity as u64).checked_mul((entries as u64).checked_add(1).ok_or(StrokeError::Limit)?).and_then(|n| n.checked_mul(2)).ok_or(StrokeError::Limit)
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| StrokeError::Cancelled)
}
fn limits(limits: StrokeLimits) -> Result<()> {
    let standard = StrokeLimits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units { Err(StrokeError::Limit) } else { Ok(()) }
}
pub(crate) fn vector_requirement<T>(source: &Vec<T>) -> Result<IncrementalCloneRequirements> {
    Ok(IncrementalCloneRequirements {
        retained_bytes: scaled(source.capacity(), size_of::<T>())?,
        staged_bytes: scaled(source.len(), size_of::<T>())?,
        work_units: (source.len() as u64).checked_mul(2).ok_or(StrokeError::Limit)?,
    })
}
pub(crate) fn fixed<T>() -> IncrementalCloneRequirements {
    IncrementalCloneRequirements { retained_bytes: size_of::<T>() as u64, staged_bytes: size_of::<T>() as u64, work_units: 1 }
}

pub(crate) struct CloneBudget<'a, 'b> {
    control: &'a Interrupt<'b>,
    remaining: u64,
    work: u64,
}
impl<'a, 'b> CloneBudget<'a, 'b> {
    pub(crate) fn new(requirements: IncrementalCloneRequirements, limits: StrokeLimits, control: &'a Interrupt<'b>) -> Result<Self> {
        poll(control)?;
        self::limits(limits)?;
        if requirements.work_units > limits.work_units {
            return Err(StrokeError::Limit);
        }
        let remaining = limits.resident_bytes.checked_sub(requirements.retained_bytes).ok_or(StrokeError::Limit)?;
        if requirements.staged_bytes > remaining {
            return Err(StrokeError::Limit);
        }
        Ok(Self { control, remaining, work: limits.work_units })
    }
    pub(crate) fn poll(&self) -> Result<()> {
        poll(self.control)
    }
    pub(crate) fn admit(&self, bytes: u64) -> Result<()> {
        self.poll()?;
        if bytes > self.remaining { Err(StrokeError::Limit) } else { Ok(()) }
    }
    pub(crate) fn charge(&mut self, bytes: u64) -> Result<()> {
        self.poll()?;
        self.remaining = self.remaining.checked_sub(bytes).ok_or(StrokeError::Limit)?;
        Ok(())
    }
    pub(crate) fn work(&mut self, n: u64) -> Result<()> {
        self.poll()?;
        self.work = self.work.checked_sub(n).ok_or(StrokeError::Limit)?;
        Ok(())
    }
    pub(crate) fn copy<T: Copy>(&mut self, source: &[T]) -> Result<Vec<T>> {
        self.poll()?;
        let requested = scaled(source.len(), size_of::<T>())?;
        if requested > self.remaining {
            return Err(StrokeError::Limit);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(source.len()).map_err(|_| StrokeError::Allocation)?;
        self.charge(scaled(out.capacity(), size_of::<T>())?)?;
        for chunk in source.chunks(256) {
            self.work(chunk.len() as u64)?;
            out.extend_from_slice(chunk);
        }
        self.poll()?;
        Ok(out)
    }
    fn string(&mut self, source: &str) -> Result<String> {
        self.poll()?;
        if source.len() as u64 > self.remaining {
            return Err(StrokeError::Limit);
        }
        let mut out = String::new();
        out.try_reserve_exact(source.len()).map_err(|_| StrokeError::Allocation)?;
        self.charge(out.capacity() as u64)?;
        for (index, character) in source.chars().enumerate() {
            if index.is_multiple_of(256) {
                self.poll()?;
            }
            self.work(character.len_utf8() as u64)?;
            out.push(character);
        }
        Ok(out)
    }
}

fn gray_requirement(tile: &crate::tile::GrayTile) -> Result<IncrementalCloneRequirements> {
    // Shared immutable samples have no Vec spare capacity and no clone payload.
    //32 bytes conservatively cover the Arc owner/header, not all allocator RSS.
    Ok(IncrementalCloneRequirements {
        retained_bytes: sum(scaled(tile.data.len(), size_of::<u16>())?, 32)?,
        staged_bytes: 0,
        work_units: (tile.data.len() as u64).checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(StrokeError::Limit)?,
    })
}
fn stored_requirement(tile: &crate::tile::StoredTile, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
    poll(control)?;
    let mut out = IncrementalCloneRequirements {
        retained_bytes: tile.key.capacity() as u64,
        staged_bytes: tile.key.len() as u64,
        work_units: (tile.key.len() as u64).checked_mul(2).ok_or(StrokeError::Limit)?,
    };
    // Both shared authorized bitmaps remain held, including the unloaded preview.
    for bitmap in std::iter::once(tile.preview.as_ref()).chain(tile.full.as_deref()) {
        poll(control)?;
        let mut req = gray_requirement(bitmap)?;
        req.staged_bytes = 0;
        req.work_units = 1;
        out.add(req, limits)?;
    }
    Ok(out)
}
fn tip_requirement(tip: &TipShape, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
    match tip {
        TipShape::Round => Ok(Default::default()),
        TipShape::Sampled(tile) => gray_requirement(tile),
        TipShape::Stored(tile) => stored_requirement(tile, limits, control),
    }
}
pub(crate) fn brush_requirements(brush: &BrushSettings, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
    poll(control)?;
    self::limits(limits)?;
    let mut out = fixed::<BrushSettings>();
    out.add(tip_requirement(&brush.tip, limits, control)?, limits)?;
    out.add(tip_requirement(&brush.dual_brush.tip, limits, control)?, limits)?;
    out.add(
        match &brush.texture.pattern {
            Pattern::Procedural { .. } => Default::default(),
            Pattern::Tile(tile) => gray_requirement(tile)?,
            Pattern::Stored(tile) => stored_requirement(tile, limits, control)?,
        },
        limits,
    )?;
    Ok(out)
}
fn copy_gray(tile: &crate::tile::GrayTile, budget: &mut CloneBudget<'_, '_>) -> Result<crate::tile::GrayTile> {
    budget.work(1)?;
    Ok(crate::tile::GrayTile { width: tile.width, height: tile.height, data: Arc::clone(&tile.data) })
}
fn copy_stored(tile: &crate::tile::StoredTile, budget: &mut CloneBudget<'_, '_>) -> Result<crate::tile::StoredTile> {
    Ok(crate::tile::StoredTile {
        key: budget.string(&tile.key)?,
        width: tile.width,
        height: tile.height,
        preview: Arc::clone(&tile.preview),
        full: tile.full.as_ref().map(Arc::clone),
    })
}
fn copy_tip(tip: &TipShape, budget: &mut CloneBudget<'_, '_>) -> Result<TipShape> {
    match tip {
        TipShape::Round => Ok(TipShape::Round),
        TipShape::Sampled(tile) => Ok(TipShape::Sampled(copy_gray(tile, budget)?)),
        TipShape::Stored(tile) => Ok(TipShape::Stored(copy_stored(tile, budget)?)),
    }
}
pub(crate) fn copy_brush(b: &BrushSettings, budget: &mut CloneBudget<'_, '_>) -> Result<BrushSettings> {
    budget.charge(size_of::<BrushSettings>() as u64)?;
    let t = &b.texture;
    let d = &b.dual_brush;
    Ok(BrushSettings {
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
        tip: copy_tip(&b.tip, budget)?,
        angle: b.angle,
        roundness: b.roundness,
        flip_x: b.flip_x,
        flip_y: b.flip_y,
        aliased: b.aliased,
        square: b.square,
        shape_dynamics: b.shape_dynamics.clone(),
        scattering: b.scattering.clone(),
        texture: crate::brush::Texture {
            enabled: t.enabled,
            pattern: match &t.pattern {
                Pattern::Procedural { style, size, seed } => Pattern::Procedural { style: *style, size: *size, seed: *seed },
                Pattern::Tile(tile) => Pattern::Tile(copy_gray(tile, budget)?),
                Pattern::Stored(tile) => Pattern::Stored(copy_stored(tile, budget)?),
            },
            invert: t.invert,
            scale: t.scale,
            brightness: t.brightness,
            contrast: t.contrast,
            each_tip: t.each_tip,
            mode: t.mode,
            depth: t.depth,
            depth_jitter: t.depth_jitter,
        },
        dual_brush: crate::brush::DualBrush {
            enabled: d.enabled,
            mode: d.mode,
            tip: copy_tip(&d.tip, budget)?,
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
    })
}

fn map_requirements(map: &CoverageMap, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
    poll(control)?;
    if map.nc > 8 {
        return Err(StrokeError::InvalidInput("coverage channel count"));
    }
    let slots = (map.tiles.capacity() as u64).checked_mul(MAP_ENTRY_BYTES).ok_or(StrokeError::Limit)?;
    let dirty = (map.dirty.capacity() as u64).checked_mul(SET_ENTRY_BYTES).ok_or(StrokeError::Limit)?;
    let mut out = IncrementalCloneRequirements {
        retained_bytes: sum(slots, dirty)?,
        staged_bytes: sum(slots, dirty)?,
        work_units: sum(table_work(map.tiles.capacity(), map.tiles.len())?, table_work(map.dirty.capacity(), map.dirty.len())?)?,
    };
    out.add(fixed::<CoverageMap>(), limits)?;
    // Capacity envelopes are checked before even enumerating sparse entries.
    if sum(out.retained_bytes, out.staged_bytes)? > limits.resident_bytes || out.work_units > limits.work_units {
        return Err(StrokeError::Limit);
    }
    for tile in map.tiles.values() {
        poll(control)?;
        for values in [&tile.cov, &tile.depth, &tile.col] {
            out.add(vector_requirement(values)?, limits)?;
        }
    }
    Ok(out)
}
fn copy_set(source: &HashSet<(i32, i32)>, budget: &mut CloneBudget<'_, '_>) -> Result<HashSet<(i32, i32)>> {
    budget.poll()?;
    if (source.capacity() as u64).checked_mul(SET_ENTRY_BYTES).ok_or(StrokeError::Limit)? > budget.remaining {
        return Err(StrokeError::Limit);
    }
    let mut out = HashSet::with_hasher(source.hasher().clone());
    out.try_reserve(source.capacity()).map_err(|_| StrokeError::Allocation)?;
    budget.charge((out.capacity() as u64).checked_mul(SET_ENTRY_BYTES).ok_or(StrokeError::Limit)?)?;
    for value in source {
        budget.work(1)?;
        out.insert(*value);
    }
    Ok(out)
}
fn copy_map(source: &CoverageMap, budget: &mut CloneBudget<'_, '_>) -> Result<CoverageMap> {
    budget.charge(size_of::<CoverageMap>() as u64)?;
    if (source.tiles.capacity() as u64).checked_mul(MAP_ENTRY_BYTES).ok_or(StrokeError::Limit)? > budget.remaining {
        return Err(StrokeError::Limit);
    }
    let mut tiles = HashMap::with_hasher(source.tiles.hasher().clone());
    tiles.try_reserve(source.tiles.capacity()).map_err(|_| StrokeError::Allocation)?;
    budget.charge((tiles.capacity() as u64).checked_mul(MAP_ENTRY_BYTES).ok_or(StrokeError::Limit)?)?;
    for (key, tile) in &source.tiles {
        budget.work(1)?;
        tiles.insert(*key, CovTile { cov: budget.copy(&tile.cov)?, depth: budget.copy(&tile.depth)?, col: budget.copy(&tile.col)? });
    }
    Ok(CoverageMap { tiles, nc: source.nc, bounds: source.bounds, dirty: copy_set(&source.dirty, budget)? })
}
fn context_requirements(context: &BrushContext, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
    let mut out = fixed::<BrushContext>();
    out.add(brush_requirements(&context.brush, limits, control)?, limits)?;
    for pyramid in [context.tip.as_deref(), context.dual_tip.as_deref()].into_iter().flatten() {
        poll(control)?;
        out.add(
            IncrementalCloneRequirements {
                retained_bytes: scaled(pyramid.levels.capacity(), size_of::<(usize, usize, Vec<f32>)>())?,
                staged_bytes: 0,
                work_units: (pyramid.levels.len() as u64).checked_mul(2).ok_or(StrokeError::Limit)?,
            },
            limits,
        )?;
        for (_, _, values) in &pyramid.levels {
            poll(control)?;
            out.add(IncrementalCloneRequirements { retained_bytes: scaled(values.capacity(), size_of::<f32>())?, staged_bytes: 0, work_units: 1 }, limits)?;
        }
    }
    if let Some(texture) = context.texture.as_deref() {
        out.add(IncrementalCloneRequirements { retained_bytes: scaled(texture.data.capacity(), size_of::<f32>())?, staged_bytes: 0, work_units: 1 }, limits)?;
    }
    Ok(out)
}
impl StrokeRenderer {
    pub fn incremental_clone_requirements(&self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
        poll(control)?;
        self::limits(limits)?;
        let mut out = fixed::<Self>();
        out.add(context_requirements(&self.ctx, limits, control)?, limits)?;
        out.add(self.generator.incremental_clone_requirements(limits, control)?, limits)?;
        out.add(map_requirements(&self.cov, limits, control)?, limits)?;
        if let Some(map) = &self.dual {
            out.add(map_requirements(map, limits, control)?, limits)?;
        }
        out.add(vector_requirement(&self.scratch)?, limits)?;
        out.add(vector_requirement(&self.depth_scratch)?, limits)?;
        out.add(vector_requirement(&self.dab_buf)?, limits)?;
        out.add(vector_requirement(&self.dual_buf)?, limits)?;
        if let Some(dabs) = &self.all_dabs {
            out.add(vector_requirement(dabs)?, limits)?;
        }
        poll(control)?;
        Ok(out)
    }
    fn clone_with_budget(&self, budget: &mut CloneBudget<'_, '_>) -> Result<Self> {
        budget.charge(size_of::<Self>() as u64)?;
        let ctx = BrushContext {
            brush: copy_brush(&self.ctx.brush, budget)?,
            tip: self.ctx.tip.as_ref().map(Arc::clone),
            dual_tip: self.ctx.dual_tip.as_ref().map(Arc::clone),
            texture: self.ctx.texture.as_ref().map(Arc::clone),
        };
        budget.charge(size_of::<BrushContext>() as u64)?;
        let output = Self {
            ctx,
            generator: self.generator.clone_with_budget(budget)?,
            cov: copy_map(&self.cov, budget)?,
            dual: self.dual.as_ref().map(|map| copy_map(map, budget)).transpose()?,
            fmt: self.fmt,
            per_dab_color: self.per_dab_color,
            dabs_done: self.dabs_done,
            scratch: budget.copy(&self.scratch)?,
            depth_scratch: budget.copy(&self.depth_scratch)?,
            dab_buf: budget.copy(&self.dab_buf)?,
            dual_buf: budget.copy(&self.dual_buf)?,
            all_dabs: self.all_dabs.as_ref().map(|dabs| budget.copy(dabs)).transpose()?,
        };
        budget.poll()?;
        Ok(output)
    }
    pub fn try_clone_with_interrupt(&self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<Self> {
        let req = self.incremental_clone_requirements(limits, control)?;
        let mut budget = CloneBudget::new(req, limits, control)?;
        let result = self.clone_with_budget(&mut budget)?;
        poll(control)?;
        Ok(result)
    }
}
impl UnionLive {
    pub fn incremental_clone_requirements(&self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<IncrementalCloneRequirements> {
        poll(control)?;
        self::limits(limits)?;
        let bytes = (self.tail_keys.capacity() as u64).checked_mul(SET_ENTRY_BYTES).ok_or(StrokeError::Limit)?;
        let mut req = fixed::<Self>();
        req.add(
            IncrementalCloneRequirements {
                retained_bytes: bytes,
                staged_bytes: bytes,
                work_units: table_work(self.tail_keys.capacity(), self.tail_keys.len())?,
            },
            limits,
        )?;
        Ok(req)
    }
    fn clone_with_budget(&self, budget: &mut CloneBudget<'_, '_>) -> Result<Self> {
        budget.charge(size_of::<Self>() as u64)?;
        Ok(Self { tail_keys: copy_set(&self.tail_keys, budget)?, bounds: self.bounds })
    }
    pub fn try_clone_with_interrupt(&self, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<Self> {
        let req = self.incremental_clone_requirements(limits, control)?;
        let mut budget = CloneBudget::new(req, limits, control)?;
        let result = self.clone_with_budget(&mut budget)?;
        poll(control)?;
        Ok(result)
    }
}
pub fn incremental_state_requirements(
    renderer: &StrokeRenderer,
    mirrors: &[StrokeRenderer],
    union: Option<&UnionLive>,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<IncrementalCloneRequirements> {
    poll(control)?;
    self::limits(limits)?;
    if mirrors.len() > 3 {
        return Err(StrokeError::Limit);
    }
    let mut req = renderer.incremental_clone_requirements(limits, control)?;
    req.add(
        IncrementalCloneRequirements {
            retained_bytes: scaled(mirrors.len(), size_of::<StrokeRenderer>())?,
            staged_bytes: scaled(mirrors.len(), size_of::<StrokeRenderer>())?,
            work_units: mirrors.len() as u64,
        },
        limits,
    )?;
    for mirror in mirrors {
        poll(control)?;
        req.add(mirror.incremental_clone_requirements(limits, control)?, limits)?;
    }
    if let Some(union) = union {
        req.add(union.incremental_clone_requirements(limits, control)?, limits)?;
    }
    Ok(req)
}
pub fn clone_incremental_state_with_interrupt(
    renderer: &StrokeRenderer,
    mirrors: &[StrokeRenderer],
    union: Option<&UnionLive>,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<IncrementalStrokeState> {
    let req = incremental_state_requirements(renderer, mirrors, union, limits, control)?;
    let mut budget = CloneBudget::new(req, limits, control)?;
    let renderer = renderer.clone_with_budget(&mut budget)?;
    let mut copied = Vec::new();
    copied.try_reserve_exact(mirrors.len()).map_err(|_| StrokeError::Allocation)?;
    budget.charge(scaled(copied.capacity(), size_of::<StrokeRenderer>())?)?;
    for mirror in mirrors {
        budget.work(1)?;
        copied.push(mirror.clone_with_budget(&mut budget)?);
    }
    let union = union.map(|union| union.clone_with_budget(&mut budget)).transpose()?;
    poll(control)?;
    Ok(IncrementalStrokeState { renderer, mirrors: copied, union })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloned_incremental_smoothing_tail_and_union_continue_like_upstream() {
        let brush = BrushSettings {
            size: 4.0,
            flow: 0.3,
            opacity: 0.6,
            wet_edges: true,
            seed: 19,
            smoothing: crate::brush::Smoothing { enabled: true, amount: 0.5, catch_up: true, catch_up_on_end: true, ..Default::default() },
            ..Default::default()
        };
        let pre = Surface::new(photocraft_color::PixelFormat::RGBA32F);
        let mut renderer = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
        renderer.push(&[StrokePoint { x: 2.0, y: 3.0, time: 0.0, ..Default::default() }, StrokePoint { x: 7.0, y: 5.0, time: 32.0, ..Default::default() }]);
        let mut old = renderer.clone();
        let mut cloned = renderer.try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(old.smoothing_string(), cloned.smoothing_string());
        assert_eq!(old.wants_time(), cloned.wants_time());
        let batch = [StrokePoint { x: 9.0, y: 8.0, time: 64.0, ..Default::default() }];
        old.push(&batch);
        cloned.push(&batch);
        old.finish();
        cloned.finish();
        let mut a = pre.clone();
        let mut b = pre.clone();
        let da = old.composite(&pre, &mut a, None, false, true);
        let db = cloned.composite(&pre, &mut b, None, false, true);
        assert_eq!(da, db);
        assert_eq!(a, b);
        let mut union = UnionLive::default();
        union.tail_keys.insert((1, 2));
        union.bounds = Rect::new(0, 0, 16, 16);
        let staged = clone_incremental_state_with_interrupt(&renderer, &[renderer.clone()], Some(&union), StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(staged.union.unwrap().tail_keys, union.tail_keys);
        assert_eq!(staged.mirrors.len(), 1);
    }
    #[test]
    fn raw_nan_signed_zero_and_shared_assets_survive_private_clone() {
        let tile = crate::tile::GrayTile { width: 1, height: 1, data: vec![65535].into() };
        let mut renderer =
            StrokeRenderer::new(&BrushSettings { tip: TipShape::Sampled(tile), ..Default::default() }, Some(photocraft_color::PixelFormat::RGBA32F), 1.0);
        renderer.scratch = vec![f32::from_bits(0x7fc01234), -0.0];
        let cloned = renderer.try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(renderer.scratch.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), cloned.scratch.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        assert!(Arc::ptr_eq(renderer.ctx.tip.as_ref().unwrap(), cloned.ctx.tip.as_ref().unwrap()));
        if let (TipShape::Sampled(before), TipShape::Sampled(after)) = (&renderer.ctx.brush.tip, &cloned.ctx.brush.tip) {
            assert!(Arc::ptr_eq(&before.data, &after.data));
        } else {
            panic!("sampled brush identity retained");
        }
        let req = renderer.incremental_clone_requirements(StrokeLimits::default(), &Interrupt::NONE).unwrap();
        assert!(
            renderer
                .try_clone_with_interrupt(StrokeLimits { resident_bytes: req.retained_bytes + req.staged_bytes - 1, ..Default::default() }, &Interrupt::NONE)
                .is_err()
        );
        let stop = || true;
        assert!(matches!(renderer.try_clone_with_interrupt(StrokeLimits::default(), &Interrupt::cancel_only(&stop)), Err(StrokeError::Cancelled)));
        assert_eq!(renderer.scratch[0].to_bits(), 0x7fc01234);
    }
}
