//! Pattern paint: Pattern Overlay, pattern strokes and pattern fill layers.
//!
//! A pattern tiles the plane from an origin (the layer's frame when linked, else the canvas
//! origin, plus the phase), scaled and rotated about that origin. Samples are bilinear with
//! wrap-around on premultiplied colour, so integer placements at 100 % reproduce the tile
//! exactly and scaled ones stay seamless.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use photocraft_doc::{ColorMode, Pattern, PixelFormat, Rect, SampleType};

pub(crate) const PREPARED_PATTERN_BYTES: usize = 64 << 20;
const MAX_PREPARED_PATTERNS: usize = 64;
type PreparedSlot = Arc<OnceLock<Option<Arc<Tile>>>>;
/// No new owners are implied by the borrowed source census.
pub(crate) struct CheckedPatternRequirements {
    pub prepared_bytes: usize,
    pub source_bytes: usize,
    pub work_units: u64,
}
fn checked_display_name<'a>(name: &'a str, c: &photocraft_raster::Interrupt<'_>) -> Result<&'a str, photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    let mut end = name.len();
    while end > 0 && name.as_bytes().get(end - 1) == Some(&0) {
        if end.is_multiple_of(1024) {
            c.check().map_err(|_| E::Cancelled)?;
        }
        end -= 1;
    }
    let n = name.get(..end).ok_or(E::LengthMismatch)?;
    if let Some(rest) = n.strip_prefix("$$$/") {
        for (i, b) in rest.bytes().enumerate() {
            if i.is_multiple_of(1024) {
                c.check().map_err(|_| E::Cancelled)?;
            }
            if b == b'=' {
                return rest.get(i + 1..).ok_or(E::LengthMismatch);
            }
        }
    }
    c.check().map_err(|_| E::Cancelled)?;
    Ok(n)
}


struct PreparedState {
    slots: HashMap<usize, PreparedSlot>,
    reserved_bytes: usize,
}

/// Demand-prepared pixels owned by one rendering call, never by the document.
pub(crate) struct PreparedPatterns<'a> {
    patterns: &'a [Pattern],
    budget: usize,
    checked: bool,
    /// The colour mode the patterns are painted into (see [`pattern_rgba`]).
    mode: ColorMode,
    state: Mutex<PreparedState>,
}

impl<'a> PreparedPatterns<'a> {
    pub(crate) fn new(patterns: &'a [Pattern], budget: usize) -> Self {
        Self { patterns, budget, mode: ColorMode::Rgb, checked: false, state: Mutex::new(PreparedState { slots: HashMap::new(), reserved_bytes: 0 }) }
    }

    /// Eager checked cache: every immutable source is prepared before rendering.
    /// The conservative slot allowance also charges cache metadata. Legacy lazy
    /// callers retain their existing cache-exhaustion behavior.
    pub(crate) fn try_new_with_interrupt(
        patterns: &'a [Pattern],
        budget: usize,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Self, photocraft_raster::RasterInputError> {
        Self::try_new_in_mode_with_interrupt(patterns,budget,ColorMode::Rgb,control)
    }
    pub(crate) fn try_new_in_mode_with_interrupt(patterns:&'a [Pattern],budget:usize,mode:ColorMode,control:&photocraft_raster::Interrupt<'_>)->Result<Self,photocraft_raster::RasterInputError>{
        use photocraft_raster::RasterInputError as E;
        control.check().map_err(|_| E::Cancelled)?;
        if patterns.len() > MAX_PREPARED_PATTERNS {
            return Err(E::LimitExceeded);
        }
        let budget = budget.min(PREPARED_PATTERN_BYTES);
        let mut total = patterns.len().checked_mul(1024).ok_or(E::LimitExceeded)?;
        for p in patterns {
            control.check().map_err(|_| E::Cancelled)?;
            total = total.checked_add(pattern_bytes(p)?).ok_or(E::LimitExceeded)?;
        }
        if total > budget {
            return Err(E::LimitExceeded);
        }
        let mut slots = HashMap::new();
        slots.try_reserve(patterns.len()).map_err(|_| E::AllocationFailed)?;
        let mut reserved = patterns.len() * 1024;
        for p in patterns {
            let tile = Tile::try_new_in_mode_with_interrupt(p, budget - reserved, mode, control)?;
            if let Some(ref tile) = tile {
                reserved = reserved.checked_add(tile.px.capacity().checked_mul(16).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
            }
            if reserved > budget {
                return Err(E::LimitExceeded);
            }
            let slot = OnceLock::new();
            // A fresh OnceLock cannot already contain a value.
            if slot.set(tile.map(Arc::new)).is_err() {
                return Err(E::UnsupportedConversion);
            }
            slots.insert(std::ptr::from_ref(p) as usize, Arc::new(slot));
        }
        control.check().map_err(|_| E::Cancelled)?;
        Ok(Self { patterns, budget, mode, checked: true, state: Mutex::new(PreparedState { slots, reserved_bytes: reserved }) })
    }

    pub(crate) fn in_mode(mut self, mode: ColorMode) -> Self {
        self.mode = mode;
        self
    }

    pub(crate) fn source(&self) -> &'a [Pattern] {
        self.patterns
    }

    /// Actual checked cache owners and borrowed source census. These are separate:
    /// the caller already charges its document and the pattern preparation lease.
    pub(crate) fn checked_requirements_with_interrupt(
        &self,
        c: &photocraft_raster::Interrupt<'_>,
    ) -> Result<CheckedPatternRequirements, photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        c.check().map_err(|_| E::Cancelled)?;
        if !self.checked || self.patterns.len() > MAX_PREPARED_PATTERNS {
            return Err(E::UnsupportedConversion);
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.slots.len() != self.patterns.len() {
            return Err(E::UnsupportedConversion);
        }
        let mut prepared_bytes = state.slots.capacity().checked_mul(1024).ok_or(E::LimitExceeded)?;
        let mut source_bytes = self.patterns.len().checked_mul(std::mem::size_of::<Pattern>()).ok_or(E::LimitExceeded)?;
        let mut work_units = 0u64;
        for p in self.patterns {
            c.check().map_err(|_| E::Cancelled)?;
            let names = p.id.len().checked_add(p.name.len()).ok_or(E::LimitExceeded)?;
            let work = (names as u64)
                .checked_mul(4)
                .and_then(|n| (p.surface.tile_count() as u64).checked_mul(32).and_then(|v| n.checked_add(v)))
                .ok_or(E::LimitExceeded)?;
            work_units = work_units.checked_add(work).ok_or(E::LimitExceeded)?;
            if work_units > 8_000_000_000 {
                return Err(E::LimitExceeded);
            }
            for chunk in p.id.as_bytes().chunks(4096).chain(p.name.as_bytes().chunks(4096)) {
                let _ = chunk;
                c.check().map_err(|_| E::Cancelled)?;
            }
            source_bytes = source_bytes
                .checked_add(p.id.capacity())
                .and_then(|n| n.checked_add(p.name.capacity()))
                .and_then(|n| n.checked_add(p.surface.default_bytes().len()))
                .and_then(|n| p.surface.tile_count().checked_mul(256).and_then(|v| n.checked_add(v)))
                .ok_or(E::LimitExceeded)?;
            for (_, tile) in p.surface.tiles() {
                c.check().map_err(|_| E::Cancelled)?;
                source_bytes = source_bytes.checked_add(tile.bytes().len()).ok_or(E::LimitExceeded)?;
            }
            let slot = state.slots.get(&(std::ptr::from_ref(p) as usize)).ok_or(E::UnsupportedConversion)?;
            let tile = slot.get().ok_or(E::UnsupportedConversion)?;
            if let Some(tile) = tile {
                let len = tile.w.checked_mul(tile.h).ok_or(E::LimitExceeded)?;
                if len != tile.px.len() || tile.w != p.width as usize || tile.h != p.height as usize || len == 0 {
                    return Err(E::LengthMismatch);
                }
                prepared_bytes = prepared_bytes.checked_add(tile.px.capacity().checked_mul(16).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
            }
        }
        if prepared_bytes > self.budget {
            return Err(E::LimitExceeded);
        }
        c.check().map_err(|_| E::Cancelled)?;
        Ok(CheckedPatternRequirements { prepared_bytes, source_bytes, work_units })
    }
    /// No lazy conversion: only an eagerly checked cache can serve texture reads.
    pub(crate) fn get_prepared_with_interrupt(
        &self,
        id: &str,
        name: &str,
        c: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Option<Arc<Tile>>, photocraft_raster::RasterInputError> {
        use photocraft_raster::RasterInputError as E;
        c.check().map_err(|_| E::Cancelled)?;
        if !self.checked || self.patterns.len() > MAX_PREPARED_PATTERNS {
            return Err(E::UnsupportedConversion);
        }
        let same = |a: &str, b: &str, insensitive: bool| -> Result<bool, E> {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.as_bytes().chunks(4096).zip(b.as_bytes().chunks(4096)) {
                c.check().map_err(|_| E::Cancelled)?;
                if if insensitive { !a.eq_ignore_ascii_case(b) } else { a != b } {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        let mut found = None;
        if !id.is_empty() {
            for p in self.patterns {
                c.check().map_err(|_| E::Cancelled)?;
                if same(&p.id, id, false)? {
                    found = Some(p);
                    break;
                }
            }
        }
        if found.is_none() && !name.is_empty() {
            for p in self.patterns {
                c.check().map_err(|_| E::Cancelled)?;
                if same(&p.name, name, false)? {
                    found = Some(p);
                    break;
                }
            }
        }
        if found.is_none() && !name.is_empty() {
            let name = checked_display_name(name, c)?;
            for p in self.patterns {
                c.check().map_err(|_| E::Cancelled)?;
                if same(checked_display_name(&p.name, c)?, name, true)? {
                    found = Some(p);
                    break;
                }
            }
        }
        let Some(p) = found else {
            return Ok(None);
        };
        if !p.is_paintable() {
            return Ok(None);
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let slot = state.slots.get(&(std::ptr::from_ref(p) as usize)).ok_or(E::UnsupportedConversion)?;
        let tile = slot.get().ok_or(E::UnsupportedConversion)?.clone();
        c.check().map_err(|_| E::Cancelled)?;
        Ok(tile)
    }

    pub(crate) fn get(&self, id: &str, name: &str) -> Option<Arc<Tile>> {
        let p = photocraft_doc::pattern::find(self.patterns, id, name)?;
        if !p.is_paintable() {
            return None;
        }
        let bytes = usize::try_from(p.width)
            .ok()
            .and_then(|w| usize::try_from(p.height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|pixels| pixels.checked_mul(std::mem::size_of::<[f32; 4]>()));
        // The borrowed immutable slice pins these addresses for the complete call.
        let key = std::ptr::from_ref(p) as usize;
        let slot = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = state.slots.get(&key) {
                Some(Arc::clone(slot))
            } else if let Some(bytes) = bytes
                && bytes <= self.budget.saturating_sub(state.reserved_bytes)
                && state.slots.len() < MAX_PREPARED_PATTERNS
            {
                let slot: PreparedSlot = Arc::new(OnceLock::new());
                // Reserve before initialization: concurrent pending buffers consume the budget too.
                state.reserved_bytes += bytes;
                state.slots.insert(key, Arc::clone(&slot));
                Some(slot)
            } else {
                None
            }
        };
        match slot {
            Some(slot) => {
                // Conversion must stay serial: Rayon work stealing can re-enter this slot
                // while its initializer is running inside an effect-map OnceLock.
                // Run on the requesting worker to preserve its active CMYK profile.
                slot.get_or_init(|| Tile::new_in(p, self.mode).map(Arc::new)).clone()
            }
            None if self.checked => None,
            None => Tile::new_in(p, self.mode).map(Arc::new),
        }
    }
}

pub(crate) fn checked_reservation_bytes(patterns: &[Pattern], token: &photocraft_raster::Interrupt<'_>) -> Result<usize, photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    if patterns.len() > MAX_PREPARED_PATTERNS {
        return Err(E::LimitExceeded);
    }
    let mut bytes = patterns.len().checked_mul(1024).ok_or(E::LimitExceeded)?;
    for p in patterns {
        token.check().map_err(|_| E::Cancelled)?;
        bytes = bytes.checked_add(pattern_bytes(p)?).ok_or(E::LimitExceeded)?;
    }
    if bytes > PREPARED_PATTERN_BYTES {
        return Err(E::LimitExceeded);
    }
    Ok(bytes)
}
fn pattern_bytes(p: &Pattern) -> Result<usize, photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    let w = i32::try_from(p.width).map_err(|_| E::InvalidBounds)?;
    let h = i32::try_from(p.height).map_err(|_| E::InvalidBounds)?;
    photocraft_raster::checked_region_byte_len(Rect::new(0, 0, w, h), photocraft_color::PixelFormat::RGBA32F)
}

/// A pattern's pixels (straight-alpha RGBA, row-major) as they look painted into a document of
/// colour mode `mode`. A pattern keeps its own colours (an RGB pattern stays RGB, in the
/// library and in the document), but in a Grayscale or CMYK document it paints that mode's
/// colours: what Edit › Fill writes into the document's pixels, and what Photoshop shows (the
/// pattern in colour in its picker, filled in grey). Empty for a pattern that isn't
/// [paintable](Pattern::is_paintable).
pub fn pattern_rgba(p: &Pattern, mode: ColorMode) -> Vec<[f32; 4]> {
    let Some(n) = p.is_paintable().then(|| usize::try_from(p.pixel_count()).ok()).flatten() else {
        return Vec::new();
    };
    let mut px = vec![[0.0f32; 4]; n];
    p.surface.read_rgba_into(p.rect(), &mut px);
    let paints_in = matches!(mode, ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone | ColorMode::Cmyk);
    if paints_in && p.surface.format().mode != mode {
        let fmt = PixelFormat::new(mode, SampleType::F32, true);
        let mut v = [0.0f32; 5];
        for q in &mut px {
            let n = photocraft_raster::from_rgba_into(&fmt, *q, &mut v);
            *q = photocraft_raster::to_rgba(&fmt, v.get(..n).unwrap_or_default());
        }
    }
    px
}

/// A pattern converted once to straight-alpha RGBA for sampling.
pub struct Tile {
    w: usize,
    h: usize,
    /// Premultiplied RGBA.
    px: Vec<[f32; 4]>,
}

impl Tile {
    pub fn new(p: &Pattern) -> Option<Tile> {
        Tile::new_in(p, ColorMode::Rgb)
    }

    /// The tile of `p` painted into a document of colour mode `mode` (see [`pattern_rgba`]).
    /// `None` when the pattern isn't [paintable](Pattern::is_paintable).
    pub fn new_in(p: &Pattern, mode: ColorMode) -> Option<Tile> {
        if !p.is_paintable() {
            return None;
        }
        let (w, h) = (p.width as usize, p.height as usize);
        let mut px = pattern_rgba(p, mode);
        for q in &mut px {
            for c in 0..3 {
                q[c] *= q[3];
            }
        }
        Some(Tile { w, h, px })
    }

    /// Checked display preparation; legacy callers retain the old constructor.
    pub fn try_new_with_interrupt(
        p: &Pattern,
        budget: usize,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Option<Tile>, photocraft_raster::RasterInputError> {
        Self::try_new_in_mode_with_interrupt(p,budget,ColorMode::Rgb,control)
    }
    fn try_new_in_mode_with_interrupt(p:&Pattern,budget:usize,mode:ColorMode,control:&photocraft_raster::Interrupt<'_>)->Result<Option<Tile>,photocraft_raster::RasterInputError>{
        use photocraft_raster::RasterInputError as E;
        control.check().map_err(|_| E::Cancelled)?;
        p.surface.try_validate_tile_origins_with(control)?;
        let bytes = pattern_bytes(p)?;
        if bytes > budget.min(PREPARED_PATTERN_BYTES) {
            return Err(E::LimitExceeded);
        }
        if !p.is_paintable() {
            return Ok(None);
        }
        let (w, h) = (p.width as usize, p.height as usize);
        let count = bytes / 16;
        let mut px = Vec::new();
        px.try_reserve_exact(count).map_err(|_| E::AllocationFailed)?;
        if px.capacity().checked_mul(16).is_none_or(|n| n > budget.min(PREPARED_PATTERN_BYTES)) {
            return Err(E::LimitExceeded);
        }
        while px.len() < count {
            control.check().map_err(|_| E::Cancelled)?;
            px.resize(px.len().saturating_add(4096).min(count), [0.0; 4]);
        }
        for y in 0..h {
            for x in (0..w).step_by(4096) {
                control.check().map_err(|_| E::Cancelled)?;
                let end = x.saturating_add(4096).min(w);
                let start = y.checked_mul(w).and_then(|n| n.checked_add(x)).ok_or(E::LimitExceeded)?;
                let out = px.get_mut(start..start + end - x).ok_or(E::LengthMismatch)?;
                // Preserve the existing display conversion and active CMYK
                // context, but bound each legacy read to at most 4096 pixels.
                p.surface.read_rgba_into(Rect::new(x as i32, y as i32, end as i32, y as i32 + 1), out);
                let paints_in=matches!(mode,ColorMode::Grayscale|ColorMode::Bitmap|ColorMode::Duotone|ColorMode::Cmyk);
                for q in out {
                    if paints_in && p.surface.format().mode!=mode {let fmt=PixelFormat::new(mode,SampleType::F32,true);let mut values=[0.0f32;5];let n=photocraft_raster::from_rgba_into(&fmt,*q,&mut values);*q=photocraft_raster::to_rgba(&fmt,values.get(..n).unwrap_or_default());}
                    for c in 0..3 {
                        q[c] *= q[3];
                    }
                }
            }
        }
        control.check().map_err(|_| E::Cancelled)?;
        Ok(Some(Tile { w, h, px }))
    }

    #[inline]
    fn at(&self, x: i64, y: i64) -> [f32; 4] {
        let xi = x.rem_euclid(self.w as i64) as usize;
        let yi = y.rem_euclid(self.h as i64) as usize;
        self.px[yi * self.w + xi]
    }

    /// Bilinear, wrapping sample at continuous tile coordinates (pixel centres at `i + 0.5`).
    /// Returns straight alpha.
    pub fn sample(&self, u: f64, v: f64) -> [f32; 4] {
        let (u, v) = (u - 0.5, v - 0.5);
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = ((u - x0) as f32, (v - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut acc = [0.0f32; 4];
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            if wy == 0.0 {
                continue;
            }
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                if wx == 0.0 {
                    continue;
                }
                let p = self.at(x0.saturating_add(dx), y0.saturating_add(dy));
                let k = wx * wy;
                for c in 0..4 {
                    acc[c] += p[c] * k;
                }
            }
        }
        if acc[3] <= 0.0 {
            return [0.0; 4];
        }
        [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3].min(1.0)]
    }
}

/// Where a pattern sits: document point → tile coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    origin: (f64, f64),
    /// Inverse rotation (cos, sin) and inverse scale.
    cs: (f64, f64),
    inv_scale: f64,
}

impl Placement {
    /// `frame`: the layer's frame (used when `link`); `phase` in pixels; `scale` as a fraction;
    /// `angle` in degrees counter-clockwise.
    pub fn new(frame: Rect, link: bool, phase: (f32, f32), scale: f32, angle: f32) -> Placement {
        let anchor = if frame.is_empty() { (0.0, 0.0) } else { (f64::from(frame.x0), f64::from(frame.y0)) };
        Self::anchored(anchor, link, phase, scale, angle)
    }

    /// Like [`Placement::new`], with the linked origin given as a point. Layer effects anchor
    /// linked patterns at the layer's effects reference point (PSD `fxrp`): Photoshop's
    /// pattern overlays and pattern strokes match pixel-for-pixel tiled from there.
    pub fn anchored(anchor: (f64, f64), link: bool, phase: (f32, f32), scale: f32, angle: f32) -> Placement {
        let base = if link { anchor } else { (0.0, 0.0) };
        let origin = (base.0 + f64::from(phase.0), base.1 + f64::from(phase.1));
        let s = f64::from(scale);
        let inv_scale = if s.is_finite() && s > 1e-3 { 1.0 / s } else { 1.0 };
        let a = f64::from(angle).to_radians();
        Placement { origin, cs: (a.cos(), a.sin()), inv_scale }
    }

    /// (origin, inverse rotation (cos, sin), inverse scale), for GPU shaders.
    pub fn parts(&self) -> ((f64, f64), (f64, f64), f64) {
        (self.origin, self.cs, self.inv_scale)
    }

    /// Tile coordinates of the document point `(x, y)`.
    #[inline]
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - self.origin.0, y - self.origin.1);
        // Undo a counter-clockwise rotation (y points down on screen).
        let (c, s) = self.cs;
        let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
        (rx * self.inv_scale, ry * self.inv_scale)
    }
}

/// Renders `tile` over `rect` (row-major straight RGBA).
pub fn render(tile: &Tile, place: &Placement, rect: Rect) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity(rect.width() as usize * rect.height() as usize);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let (u, v) = place.map(f64::from(x) + 0.5, f64::from(y) + 0.5);
            out.push(tile.sample(u, v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{PixelFormat, Surface};

    fn checker() -> Pattern {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        s.fill_rect(Rect::new(0, 0, 2, 2), &[0.0, 0.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(2, 2, 4, 4), &[0.0, 0.0, 1.0, 1.0]);
        Pattern::new("checker", s, 4, 4)
    }

    #[test]
    fn identity_placement_tiles_exactly() {
        let patterns = [checker()];
        let prepared = PreparedPatterns::new(&patterns, PREPARED_PATTERN_BYTES);
        let t = prepared.get(&patterns[0].id, "").unwrap();
        let again = prepared.get(&patterns[0].id, "").unwrap();
        assert!(Arc::ptr_eq(&t, &again));
        let uncached = PreparedPatterns::new(&patterns, 0);
        let fallback = uncached.get(&patterns[0].id, "").unwrap();
        let p = Placement::new(Rect::new(0, 0, 10, 10), false, (0.0, 0.0), 1.0, 0.0);
        let px = render(&t, &p, Rect::new(-4, 0, 8, 1));
        assert_eq!(px, render(&fallback, &p, Rect::new(-4, 0, 8, 1)));
        assert_eq!(px[0], [0.0, 0.0, 1.0, 1.0]); // x = -4 wraps to 0
        assert_eq!(px[6], [1.0, 0.0, 0.0, 1.0]); // x = 2
        assert_eq!(px[8], [0.0, 0.0, 1.0, 1.0]); // x = 4
    }

    #[test]
    fn patterns_too_large_to_paint_are_skipped_without_allocating() {
        // Only the size is declared (no tiles): a hostile file can claim any size.
        let p = Pattern { id: "big".into(), name: "Big".into(), width: 9000, height: 8000, surface: Surface::new(PixelFormat::RGBA8) };
        assert!(!p.is_paintable());
        assert!(Tile::new(&p).is_none());
        assert!(pattern_rgba(&p, ColorMode::Rgb).is_empty());
        let list = [p];
        assert!(PreparedPatterns::new(&list, 0).get("big", "").is_none());
    }

    #[test]
    fn link_phase_scale_and_angle_move_the_origin() {
        let t = Tile::new(&checker()).unwrap();
        let linked = Placement::new(Rect::new(2, 0, 10, 10), true, (0.0, 0.0), 1.0, 0.0);
        assert_eq!(render(&t, &linked, Rect::new(2, 0, 3, 1))[0], [0.0, 0.0, 1.0, 1.0]);
        let phased = Placement::new(Rect::EMPTY, false, (2.0, 0.0), 1.0, 0.0);
        assert_eq!(render(&t, &phased, Rect::new(2, 0, 3, 1))[0], [0.0, 0.0, 1.0, 1.0]);
        let scaled = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 2.0, 0.0);
        let row = render(&t, &scaled, Rect::new(0, 1, 8, 2));
        assert_eq!(row[2], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(row[6], [1.0, 0.0, 0.0, 1.0]);
        // 90°: x and y swap roles.
        let rot = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 1.0, 90.0);
        let (u, v) = rot.map(1.0, 3.0);
        assert!((u - -3.0).abs() < 1e-9 && (v - 1.0).abs() < 1e-9, "{u},{v}");
    }

    #[test]
    fn sampling_at_hostile_phases_does_not_overflow() {
        let patterns = [checker()];
        let prepared = PreparedPatterns::new(&patterns, PREPARED_PATTERN_BYTES);
        let t = prepared.get(&patterns[0].id, "").unwrap();
        for u in [1.0e30, -1.0e30, f64::INFINITY, f64::NAN] {
            let _ = t.sample(u, u);
        }
    }
}

#[cfg(test)]
mod checked_preparation_tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_raster::{Interrupt, RasterInputError, Surface};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn checked_pattern_polls_inside_conversion() {
        let pattern = Pattern {
            id: "own prepared".into(),
            name: "own original".into(),
            width: 4096,
            height: 128,
            surface: Surface::with_default(PixelFormat::RGBA16, &[0.2, 0.6, 0.8, 0.5]),
        };
        let before = pattern.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 170;
        assert!(matches!(Tile::try_new_with_interrupt(&pattern, PREPARED_PATTERN_BYTES, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled)));
        assert_eq!(pattern, before);
    }
    #[test]
    fn checked_pattern_exact_display_and_combined_budget() {
        use photocraft_color::{ColorMode, SampleType};
        for format in [
            PixelFormat::RGBA8,
            PixelFormat::RGBA16,
            PixelFormat::RGBA32F,
            PixelFormat::new(ColorMode::Rgb, SampleType::U32, true),
            PixelFormat::new(ColorMode::Rgb, SampleType::F64, true),
        ] {
            let mut pattern =
                Pattern { id: "own".into(), name: "original".into(), width: 19, height: 7, surface: Surface::with_default(format, &[0.19, 0.67, 0.83, 0.41]) };
            pattern.surface.fill_rect(Rect::new(1, 2, 8, 6), &[0.82, 0.37, 0.07, 0.69]);
            let before = pattern.clone();
            let legacy = Tile::new(&pattern).unwrap();
            let checked = Tile::try_new_with_interrupt(&pattern, PREPARED_PATTERN_BYTES, &Interrupt::NONE).unwrap().unwrap();
            assert_eq!(legacy.px, checked.px);
            assert_eq!(pattern, before);
            let patterns = [pattern.clone(), pattern];
            let bytes = pattern_bytes(&patterns[0]).unwrap();
            assert!(matches!(PreparedPatterns::try_new_with_interrupt(&patterns, bytes * 2 + 2047, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
            let cache = PreparedPatterns::try_new_with_interrupt(&patterns, bytes * 2 + 2048, &Interrupt::NONE).unwrap();
            assert_eq!(cache.get("own", "").unwrap().px, legacy.px);
        }
    }
    #[test]
    fn checked_pattern_bounds_and_zero_budget_fail_before_conversion() {
        let mut pattern = Pattern { id: "own".into(), name: "original".into(), width: 1, height: 1, surface: Surface::new(PixelFormat::RGBA8) };
        assert!(matches!(Tile::try_new_with_interrupt(&pattern, 0, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        pattern.width = u32::MAX;
        assert!(matches!(Tile::try_new_with_interrupt(&pattern, PREPARED_PATTERN_BYTES, &Interrupt::NONE), Err(RasterInputError::InvalidBounds)));
    }
}
