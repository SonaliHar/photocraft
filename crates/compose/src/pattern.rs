//! Pattern paint: Pattern Overlay, pattern strokes and pattern fill layers.
//!
//! A pattern tiles the plane from an origin (the layer's frame when linked, else the canvas
//! origin, plus the phase), scaled and rotated about that origin. Samples are bilinear with
//! wrap-around on premultiplied colour, so integer placements at 100 % reproduce the tile
//! exactly and scaled ones stay seamless.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use photocraft_doc::{Pattern, Rect};

pub(crate) const PREPARED_PATTERN_BYTES: usize = 64 << 20;
const MAX_PREPARED_PATTERNS: usize = 64;
type PreparedSlot = Arc<OnceLock<Option<Arc<Tile>>>>;

struct PreparedState {
    slots: HashMap<usize, PreparedSlot>,
    reserved_bytes: usize,
}

/// Demand-prepared pixels owned by one rendering call, never by the document.
pub(crate) struct PreparedPatterns<'a> {
    patterns: &'a [Pattern],
    budget: usize,
    checked: bool,
    state: Mutex<PreparedState>,
}

impl<'a> PreparedPatterns<'a> {
    pub(crate) fn new(patterns: &'a [Pattern], budget: usize) -> Self {
        Self { patterns, budget, checked: false, state: Mutex::new(PreparedState { slots: HashMap::new(), reserved_bytes: 0 }) }
    }

    /// Eager checked cache: every immutable source is prepared before rendering.
    /// The conservative slot allowance also charges cache metadata. Legacy lazy
    /// callers retain their existing cache-exhaustion behavior.
    pub(crate) fn try_new_with_interrupt(
        patterns: &'a [Pattern],
        budget: usize,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Self, photocraft_raster::RasterInputError> {
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
            let tile = Tile::try_new_with_interrupt(p, budget - reserved, control)?;
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
        Ok(Self { patterns, budget, checked: true, state: Mutex::new(PreparedState { slots, reserved_bytes: reserved }) })
    }

    pub(crate) fn source(&self) -> &'a [Pattern] {
        self.patterns
    }

    pub(crate) fn get(&self, id: &str, name: &str) -> Option<Arc<Tile>> {
        let p = photocraft_doc::pattern::find(self.patterns, id, name)?;
        if p.is_empty() {
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
                slot.get_or_init(|| Tile::new(p).map(Arc::new)).clone()
            }
            None if self.checked => None,
            None => Tile::new(p).map(Arc::new),
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

/// A pattern converted once to straight-alpha RGBA for sampling.
pub struct Tile {
    w: usize,
    h: usize,
    /// Premultiplied RGBA.
    px: Vec<[f32; 4]>,
}

impl Tile {
    pub fn new(p: &Pattern) -> Option<Tile> {
        if p.is_empty() {
            return None;
        }
        let (w, h) = (p.width as usize, p.height as usize);
        let mut px = vec![[0.0f32; 4]; w * h];
        p.surface.read_rgba_into(p.rect(), &mut px);
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
        use photocraft_raster::RasterInputError as E;
        control.check().map_err(|_| E::Cancelled)?;
        p.surface.try_validate_tile_origins_with(control)?;
        let bytes = pattern_bytes(p)?;
        if bytes > budget.min(PREPARED_PATTERN_BYTES) {
            return Err(E::LimitExceeded);
        }
        if p.is_empty() {
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
                for q in out {
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
                let p = self.at(x0 + dx, y0 + dy);
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
