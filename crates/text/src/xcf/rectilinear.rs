//! Exact-font single-contour orthogonal glyphs with center or union-area coverage.
//! Shaping uses the caller-authorized font directly, never font fallback.
use super::{ExactFont, StemRendered, StemText, TextSourceError, poll};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};

// Conservative bounded shaping workspace for the one immutable approved face
// and at most eight ASCII characters. Acquired before HarfRust's allocations.
const SHAPING_RESIDENT: u64 = 2 * 1024 * 1024;
/// Source-declared rasterization; coverage integrates the current orthogonal
/// outline, with no phase adjustment and no source cache sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectilinearAntialias {
    None,
    AreaCoverage,
}
/// Exact pixel-unit sizes with independently authored AA controls. The original
/// 32-pixel no-AA APIs retain their prior semantics.
#[derive(Clone, Copy)]
pub struct ScaledText<'a> {
    pub content: &'a str,
    pub origin: [i32; 2],
    pub size_px: f32,
}
fn qualified_size(size: f32) -> Result<(), TextSourceError> {
    if matches!(size, 16. | 24. | 32. | 40.) { Ok(()) } else { Err(TextSourceError::Unsupported("unqualified text pixel size")) }
}
#[derive(Clone, Copy)]
struct Polygon {
    points: [[f64; 2]; 32],
    len: usize,
    closed: bool,
    invalid: bool,
}
impl Default for Polygon {
    fn default() -> Self {
        Self { points: [[0.; 2]; 32], len: 0, closed: false, invalid: false }
    }
}
impl Polygon {
    fn point(&mut self, x: f32, y: f32) {
        if self.closed || !x.is_finite() || !y.is_finite() || x.abs() > 64. || y.abs() > 64. {
            self.invalid = true;
            return;
        }
        if let Some(p) = self.points.get_mut(self.len) {
            *p = [x.into(), y.into()];
            self.len += 1;
        } else {
            self.invalid = true;
        }
    }
    fn validate(&self) -> Result<(), TextSourceError> {
        if self.invalid || self.len < 4 || !self.closed {
            return Err(TextSourceError::Unsupported("nonorthogonal or multicontour glyph"));
        }
        for (i, a) in self.points.iter().take(self.len).enumerate() {
            let b = self.points.get((i + 1) % self.len).ok_or(TextSourceError::FontIdentity)?;
            if a[0] != b[0] && a[1] != b[1] {
                return Err(TextSourceError::Unsupported("diagonal glyph edge"));
            }
        }
        Ok(())
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        let mut winding = 0_i32;
        for (i, a) in self.points.iter().take(self.len).enumerate() {
            let Some(b) = self.points.get((i + 1) % self.len) else { return false };
            if a[1] <= y && b[1] > y && a[0] > x {
                winding += 1;
            }
            if b[1] <= y && a[1] > y && a[0] > x {
                winding -= 1;
            }
        }
        winding != 0
    }
}
impl OutlinePen for Polygon {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.len != 0 {
            self.invalid = true;
        }
        self.point(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.point(x, y);
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.invalid = true;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.invalid = true;
    }
    fn close(&mut self) {
        self.closed = true;
    }
}

/// Stack-only source-glyph eligibility. This does not fabricate shaped output
/// or infer kerning. Charged preparation separately validates actual shaping,
/// so unsupported substitutions remain an error before output allocation.
pub fn qualify_rectilinear_geometry(font: &ExactFont<'_>, request: StemText<'_>, ctl: &Interrupt<'_>) -> Result<(), TextSourceError> {
    qualify_scaled_rectilinear_geometry(font, ScaledText { content: request.content, origin: request.origin, size_px: 32. }, ctl)
}
pub fn qualify_scaled_rectilinear_geometry(font: &ExactFont<'_>, request: ScaledText<'_>, ctl: &Interrupt<'_>) -> Result<(), TextSourceError> {
    poll(ctl)?;
    qualified_size(request.size_px)?;
    if !(1..=8).contains(&request.content.len())
        || request.content.bytes().any(|b| !(32..=126).contains(&b))
        || request.origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v))
    {
        return Err(TextSourceError::Unsupported("unqualified text or placement"));
    }
    let face = skrifa::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let size = Size::new(request.size_px);
    let location = LocationRef::default();
    let mut ink = false;
    let mut outline_memory = [0_u8; 16384];
    for c in request.content.chars() {
        poll(ctl)?;
        let glyph = face.charmap().map(c).ok_or(TextSourceError::FontIdentity)?;
        let advance = face.glyph_metrics(size, location).advance_width(glyph).ok_or(TextSourceError::FontIdentity)?;
        if !advance.is_finite() || !(0.0..=64.).contains(&advance) {
            return Err(TextSourceError::Unsupported("invalid source glyph advance"));
        }
        if let Some(outline) = face.outline_glyphs().get(glyph) {
            let mut pen = Polygon::default();
            outline
                .draw(
                    DrawSettings::unhinted(size, location)
                        .with_memory(Some(outline_memory.get_mut(..outline.draw_memory_size(skrifa::outline::Hinting::None)).ok_or(TextSourceError::Budget)?)),
                    &mut pen,
                )
                .map_err(|_| TextSourceError::FontIdentity)?;
            if pen.len != 0 {
                pen.validate()?;
                ink = true;
            } else if c != ' ' {
                return Err(TextSourceError::Unsupported("empty source glyph"));
            }
        } else if c != ' ' {
            return Err(TextSourceError::Unsupported("missing source glyph outline"));
        }
    }
    poll(ctl)?;
    if ink { Ok(()) } else { Err(TextSourceError::Unsupported("empty text ink")) }
}

/// Semantic geometry constraints, not a source-string whitelist. Unknown
/// outlines/shaping or empty ink are explicit unsupported. Caller retains all
/// aggregate leases until failure cleanup or destruction of the output.
pub fn prepare_rectilinear_with_charge(
    font: &ExactFont<'_>,
    request: StemText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<PreparedRectilinear, TextSourceError> {
    prepare_scaled_rectilinear_with_charge(font, ScaledText { content: request.content, origin: request.origin, size_px: 32. }, max_resident, ctl, charge)
}
pub fn prepare_scaled_rectilinear_with_charge(
    font: &ExactFont<'_>,
    request: ScaledText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<PreparedRectilinear, TextSourceError> {
    prepare_spaced_rectilinear_with_charge(font, request, 0., max_resident, ctl, charge)
}
/// Additional interglyph spacing in the independently observed 0/2/4 pixel
/// domain. There is no initial or trailing spacing and no cache-based metric.
pub fn prepare_spaced_rectilinear_with_charge(
    font: &ExactFont<'_>,
    request: ScaledText<'_>,
    letter_spacing_px: f32,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<PreparedRectilinear, TextSourceError> {
    prepare_spaced_impl(font, request, letter_spacing_px, max_resident, ctl, charge, None)
}
#[allow(clippy::too_many_arguments)]
fn prepare_spaced_impl(
    font: &ExactFont<'_>,
    request: ScaledText<'_>,
    letter_spacing_px: f32,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
    underlined: Option<&super::underline::UnderlineRuns>,
) -> Result<PreparedRectilinear, TextSourceError> {
    poll(ctl)?;
    if !matches!(letter_spacing_px, 0. | 2. | 4.) || (letter_spacing_px != 0. && !matches!(request.size_px, 16. | 32. | 40.)) {
        return Err(TextSourceError::Unsupported("unqualified text letter spacing"));
    }
    qualified_size(request.size_px)?;
    if !(1..=8).contains(&request.content.len())
        || request.content.bytes().any(|b| !(32..=126).contains(&b))
        || request.origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v))
    {
        return Err(TextSourceError::Unsupported("unqualified text or placement"));
    }
    if max_resident < SHAPING_RESIDENT {
        return Err(TextSourceError::Budget);
    }
    charge(SHAPING_RESIDENT)?;
    poll(ctl)?;
    let shaping_font = harfrust::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let data = harfrust::ShaperData::new(&shaping_font);
    let shaper = data.shaper(&shaping_font).build();
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(request.content);
    buffer.set_direction(harfrust::Direction::LeftToRight);
    buffer.set_script(harfrust::script::LATIN);
    let shaped = shaper.shape(buffer, harfrust::ShapeOptions::default());
    poll(ctl)?;
    if shaped.glyph_infos().len() != request.content.len() || shaped.glyph_positions().len() != request.content.len() {
        return Err(TextSourceError::Unsupported("unqualified glyph substitution"));
    }
    let face = skrifa::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let size = Size::new(request.size_px);
    let location = LocationRef::default();
    let metrics = face.metrics(size, location);
    let ascent = metrics.ascent.ceil();
    let upem = f64::from(shaper.units_per_em());
    if upem <= 0. || !ascent.is_finite() || !(0.0..=64.).contains(&ascent) {
        return Err(TextSourceError::FontIdentity);
    }
    let mut polygons = [Polygon::default(); 16];
    let mut len = 0;
    let mut advance = 0.;
    let mut edges = [0f64; 9];
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut outline_memory = [0_u8; 16384];
    for (index, (info, pos)) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()).enumerate() {
        poll(ctl)?;
        if underlined.is_some() && usize::try_from(info.cluster).ok() != Some(index) {
            return Err(TextSourceError::Unsupported("unqualified underline glyph clusters"));
        }
        *edges.get_mut(index).ok_or(TextSourceError::Budget)? = advance;
        if info.glyph_id == 0 || pos.y_advance != 0 || pos.y_offset != 0 || pos.x_offset != 0 {
            return Err(TextSourceError::Unsupported("unqualified positioned glyph"));
        }
        let step = (f64::from(pos.x_advance) * f64::from(request.size_px) / upem).round();
        if !(0.0..=64.).contains(&step) {
            return Err(TextSourceError::Unsupported("invalid shaped advance"));
        }
        if let Some(outline) = face.outline_glyphs().get(skrifa::GlyphId::new(info.glyph_id)) {
            let mut p = Polygon::default();
            outline
                .draw(
                    DrawSettings::unhinted(size, location)
                        .with_memory(Some(outline_memory.get_mut(..outline.draw_memory_size(skrifa::outline::Hinting::None)).ok_or(TextSourceError::Budget)?)),
                    &mut p,
                )
                .map_err(|_| TextSourceError::FontIdentity)?;
            if p.len != 0 {
                p.validate()?;
                for point in p.points.iter_mut().take(p.len) {
                    point[0] += f64::from(request.origin[0]) + advance;
                    point[1] = f64::from(request.origin[1]) + f64::from(ascent) - point[1];
                    bounds[0] = bounds[0].min(point[0]);
                    bounds[1] = bounds[1].min(point[1]);
                    bounds[2] = bounds[2].max(point[0]);
                    bounds[3] = bounds[3].max(point[1]);
                }
                *polygons.get_mut(len).ok_or(TextSourceError::Budget)? = p;
                len += 1;
            }
        } else if info.glyph_id != face.charmap().map(' ').ok_or(TextSourceError::FontIdentity)?.to_u32() {
            return Err(TextSourceError::Unsupported("missing glyph outline"));
        }
        advance += step + f64::from(letter_spacing_px);
    }
    advance -= f64::from(letter_spacing_px);
    *edges.get_mut(request.content.len()).ok_or(TextSourceError::Budget)? = advance;
    if let Some(plan) = underlined {
        let decoration = metrics.underline.ok_or(TextSourceError::FontIdentity)?;
        let top = f64::from(request.origin[1]) + f64::from(ascent) + f64::from(-decoration.offset).floor();
        let thickness = f64::from(decoration.thickness).ceil();
        if !top.is_finite() || !(1.0..=8.).contains(&thickness) {
            return Err(TextSourceError::Unsupported("invalid exact-font underline metrics"));
        }
        let mut start = 0;
        while start < request.content.len() {
            poll(ctl)?;
            if !plan.bytes.get(start).copied().ok_or(TextSourceError::Budget)? {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < request.content.len() && plan.bytes.get(end).copied().ok_or(TextSourceError::Budget)? {
                end += 1;
            }
            let x0 = f64::from(request.origin[0]) + edges.get(start).copied().ok_or(TextSourceError::Budget)?;
            let x1 = f64::from(request.origin[0]) + edges.get(end).copied().ok_or(TextSourceError::Budget)?;
            let coordinates = [x0, top, x1, top + thickness];
            if x1 <= x0 || coordinates.iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29)) {
                return Err(TextSourceError::Unsupported("unsafe underline bounds"));
            }
            let mut p = Polygon::default();
            p.points.get_mut(..4).ok_or(TextSourceError::Budget)?.copy_from_slice(&[[x0, top], [x1, top], [x1, top + thickness], [x0, top + thickness]]);
            p.len = 4;
            p.closed = true;
            *polygons.get_mut(len).ok_or(TextSourceError::Budget)? = p;
            len += 1;
            bounds[0] = bounds[0].min(x0);
            bounds[1] = bounds[1].min(top);
            bounds[2] = bounds[2].max(x1);
            bounds[3] = bounds[3].max(top + thickness);
            start = end;
        }
    }
    if len == 0 || bounds.into_iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29)) {
        return Err(TextSourceError::Unsupported("empty or unsafe text ink"));
    }
    let rect = Rect::new(bounds[0].floor() as i32, bounds[1].floor() as i32, bounds[2].ceil() as i32, bounds[3].ceil() as i32);
    let height = f64::from(ascent) - f64::from(metrics.descent.floor());
    let layout_max = [f64::from(request.origin[0]) + advance, f64::from(request.origin[1]) + height];
    if !height.is_finite() || !(1.0..=128.).contains(&height) || advance <= 0. || layout_max.into_iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29))
    {
        return Err(TextSourceError::Unsupported("unsafe text layout bounds"));
    }
    let layout_rect = Rect::new(request.origin[0], request.origin[1], layout_max[0] as i32, layout_max[1] as i32);
    Ok(PreparedRectilinear { polygons, len, rect, layout_rect, size_px: request.size_px })
}
/// Two left-aligned lines with measured additional baseline spacing. The
/// existing single-line entry points remain strict: newline layout is an
/// explicit opt-in and never silently flattened into one shaped run.
pub fn prepare_multiline_rectilinear_with_charge(
    font: &ExactFont<'_>,
    request: ScaledText<'_>,
    line_spacing_px: f32,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<PreparedRectilinear, TextSourceError> {
    poll(ctl)?;
    if !matches!(request.size_px, 16. | 32. | 40.) || !matches!(line_spacing_px, 0. | 2. | 4.) {
        return Err(TextSourceError::Unsupported("unqualified multiline size or spacing"));
    }
    let (first, second) = request.content.split_once('\n').ok_or(TextSourceError::Unsupported("multiline requires exactly two lines"))?;
    if first.is_empty() || second.is_empty() || request.content.len() > 9 || first.bytes().chain(second.bytes()).any(|v| !(32..=126).contains(&v)) {
        return Err(TextSourceError::Unsupported("unqualified multiline content"));
    }
    // Both shaping leases remain charged until the caller destroys the result;
    // this also gives a conservative aggregate bound to the borrowed API.
    if max_resident < SHAPING_RESIDENT.checked_mul(2).ok_or(TextSourceError::Budget)? {
        return Err(TextSourceError::Budget);
    }
    let mut prepared = prepare_scaled_rectilinear_with_charge(font, ScaledText { content: first, ..request }, max_resident, ctl, charge)?;
    let line_height = i32::try_from(prepared.layout_rect.height()).map_err(|_| TextSourceError::Budget)?;
    let second_y = request.origin[1]
        .checked_add(line_height)
        .and_then(|v| v.checked_add(line_spacing_px as i32))
        .ok_or(TextSourceError::Unsupported("unsafe multiline origin"))?;
    let next = prepare_scaled_rectilinear_with_charge(
        font,
        ScaledText { content: second, origin: [request.origin[0], second_y], ..request },
        max_resident,
        ctl,
        charge,
    )?;
    for polygon in next.polygons.iter().take(next.len) {
        poll(ctl)?;
        *prepared.polygons.get_mut(prepared.len).ok_or(TextSourceError::Budget)? = *polygon;
        prepared.len = prepared.len.checked_add(1).ok_or(TextSourceError::Budget)?;
    }
    prepared.rect = prepared.rect.union(&next.rect);
    prepared.layout_rect = prepared.layout_rect.union(&next.layout_rect);
    poll(ctl)?;
    Ok(prepared)
}
/// Stack-only semantic eligibility; actual shaping still acquires its explicit
/// workspace leases during preparation.
pub fn qualify_multiline_rectilinear_geometry(
    font: &ExactFont<'_>,
    request: ScaledText<'_>,
    line_spacing_px: f32,
    ctl: &Interrupt<'_>,
) -> Result<(), TextSourceError> {
    poll(ctl)?;
    let (first, second) = request.content.split_once('\n').ok_or(TextSourceError::Unsupported("multiline requires exactly two lines"))?;
    if !matches!(request.size_px, 16. | 32. | 40.)
        || !matches!(line_spacing_px, 0. | 2. | 4.)
        || request.content.len() > 9
        || first.is_empty()
        || second.is_empty()
    {
        return Err(TextSourceError::Unsupported("unqualified multiline layout"));
    }
    qualify_scaled_rectilinear_geometry(font, ScaledText { content: first, ..request }, ctl)?;
    let face = skrifa::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let metrics = face.metrics(Size::new(request.size_px), LocationRef::default());
    let height = metrics.ascent.ceil() - metrics.descent.floor();
    if !height.is_finite() || !(1. ..=128.).contains(&height) {
        return Err(TextSourceError::FontIdentity);
    }
    let y = request.origin[1]
        .checked_add(height as i32)
        .and_then(|v| v.checked_add(line_spacing_px as i32))
        .ok_or(TextSourceError::Unsupported("unsafe multiline origin"))?;
    qualify_scaled_rectilinear_geometry(font, ScaledText { content: second, origin: [request.origin[0], y], ..request }, ctl)
}

/// Immutable bounded current-glyph geometry; contains no source cache pixels.
pub struct PreparedRectilinear {
    polygons: [Polygon; 16],
    len: usize,
    rect: Rect,
    layout_rect: Rect,
    size_px: f32,
}
impl PreparedRectilinear {
    /// Current dynamic layout box, distinct from ink coverage and retained cache.
    /// Bounds use the qualified font's rounded line metrics and shaped advances.
    pub fn layout_rect(&self) -> Rect {
        self.layout_rect
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }
    /// Output lease is acquired before allocating interleaved bytes or tiles.
    pub fn render_with_charge(
        &self,
        max_resident: u64,
        ctl: &Interrupt<'_>,
        charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
    ) -> Result<StemRendered, TextSourceError> {
        self.render_mode_with_charge(RectilinearAntialias::None, max_resident, ctl, charge)
    }
    /// Fixed-stack orthogonal arrangement integrates the union, including
    /// overlapping current glyphs. Source backing pixels are never accessed.
    fn area(&self, x: f64, y: f64, ctl: &Interrupt<'_>) -> Result<f64, TextSourceError> {
        let mut xs = [0_f64; 514];
        let mut ys = [0_f64; 514];
        xs[0] = x;
        xs[1] = x + 1.;
        ys[0] = y;
        ys[1] = y + 1.;
        let mut nx = 2;
        let mut ny = 2;
        for polygon in self.polygons.iter().take(self.len) {
            for point in polygon.points.iter().take(polygon.len) {
                if point[0] > x && point[0] < x + 1. {
                    *xs.get_mut(nx).ok_or(TextSourceError::Budget)? = point[0];
                    nx += 1;
                }
                if point[1] > y && point[1] < y + 1. {
                    *ys.get_mut(ny).ok_or(TextSourceError::Budget)? = point[1];
                    ny += 1;
                }
            }
        }
        let xs = xs.get_mut(..nx).ok_or(TextSourceError::Budget)?;
        let ys = ys.get_mut(..ny).ok_or(TextSourceError::Budget)?;
        xs.sort_unstable_by(f64::total_cmp);
        ys.sort_unstable_by(f64::total_cmp);
        let mut area = 0.;
        for rows in ys.windows(2) {
            poll(ctl)?;
            let y0 = *rows.first().ok_or(TextSourceError::Budget)?;
            let y1 = *rows.get(1).ok_or(TextSourceError::Budget)?;
            if y1 == y0 {
                continue;
            }
            for cols in xs.windows(2) {
                let x0 = *cols.first().ok_or(TextSourceError::Budget)?;
                let x1 = *cols.get(1).ok_or(TextSourceError::Budget)?;
                if x1 != x0 && self.polygons.iter().take(self.len).any(|p| p.contains((x0 + x1) * 0.5, (y0 + y1) * 0.5)) {
                    area += (x1 - x0) * (y1 - y0);
                }
            }
        }
        if !area.is_finite() {
            return Err(TextSourceError::Budget);
        }
        Ok(area.clamp(0., 1.))
    }
    /// Acquires the same output lease before either rasterization mode.
    pub fn render_mode_with_charge(
        &self,
        antialias: RectilinearAntialias,
        max_resident: u64,
        ctl: &Interrupt<'_>,
        charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
    ) -> Result<StemRendered, TextSourceError> {
        self.render_color_with_charge(antialias, [1.; 4], max_resident, ctl, charge)
    }
    /// `encoded_rgba` is explicitly converted by the caller to this output
    /// surface's working encoding. No profile or source cache is consulted.
    pub fn render_color_with_charge(
        &self,
        antialias: RectilinearAntialias,
        encoded_rgba: [f32; 4],
        max_resident: u64,
        ctl: &Interrupt<'_>,
        charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
    ) -> Result<StemRendered, TextSourceError> {
        poll(ctl)?;
        if encoded_rgba.iter().any(|v| !v.is_finite() || !(0. ..=1.).contains(v)) {
            return Err(TextSourceError::Unsupported("invalid converted text color"));
        }
        if antialias == RectilinearAntialias::None && self.size_px != 32. {
            return Err(TextSourceError::Unsupported("unqualified scaled no-AA boundary behavior"));
        }
        let rect = self.rect;
        let width = usize::try_from(rect.width()).map_err(|_| TextSourceError::Budget)?;
        let height = usize::try_from(rect.height()).map_err(|_| TextSourceError::Budget)?;
        let bytes = width.checked_mul(height).and_then(|v| v.checked_mul(4)).ok_or(TextSourceError::Budget)?;
        let tx = i64::from(rect.x1 - 1).div_euclid(256) - i64::from(rect.x0).div_euclid(256) + 1;
        let ty = i64::from(rect.y1 - 1).div_euclid(256) - i64::from(rect.y0).div_euclid(256) + 1;
        let output = (bytes as u64)
            .checked_add((tx as u64).checked_mul(ty as u64).and_then(|v| v.checked_mul(256 * 256 * 4)).ok_or(TextSourceError::Budget)?)
            .ok_or(TextSourceError::Budget)?;
        let resident = SHAPING_RESIDENT.checked_add(output).ok_or(TextSourceError::Budget)?;
        if resident > max_resident {
            return Err(TextSourceError::Budget);
        }
        charge(output)?;
        poll(ctl)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(bytes).map_err(|_| TextSourceError::Allocation)?;
        if pixels.capacity() > bytes {
            return Err(TextSourceError::Budget);
        }
        pixels.resize(bytes, 0);
        for row in 0..height {
            poll(ctl)?;
            for col in 0..width {
                let x = f64::from(rect.x0) + col as f64 + 0.5;
                let y = f64::from(rect.y0) + row as f64 + 0.5;
                let coverage = match antialias {
                    RectilinearAntialias::None => {
                        if self.polygons.iter().take(self.len).any(|p| p.contains(x, y)) {
                            255
                        } else {
                            0
                        }
                    }
                    RectilinearAntialias::AreaCoverage => (self.area(x - 0.5, y - 0.5, ctl)? * 255.).round() as u8,
                };
                let alpha = (f64::from(coverage) * f64::from(encoded_rgba.get(3).copied().ok_or(TextSourceError::Budget)?)).round() as u8;
                if alpha != 0 {
                    let offset = (row * width + col) * 4;
                    let mut rgba = encoded_rgba.map(|v| (v * 255.).round() as u8);
                    *rgba.get_mut(3).ok_or(TextSourceError::Budget)? = alpha;
                    pixels.get_mut(offset..offset + 4).ok_or(TextSourceError::Budget)?.copy_from_slice(&rgba);
                }
            }
        }
        let surface = Surface::try_from_interleaved_with(PixelFormat::RGBA8, rect, &pixels, ctl)
            .map_err(|_| if ctl.cancelled() { TextSourceError::Cancelled } else { TextSourceError::Allocation })?;
        poll(ctl)?;
        Ok(StemRendered { surface, rect, resident_bytes: resident })
    }
}
pub fn render_rectilinear_with_charge(
    font: &ExactFont<'_>,
    request: StemText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<StemRendered, TextSourceError> {
    let prepared = prepare_rectilinear_with_charge(font, request, max_resident, ctl, charge)?;
    prepared.render_with_charge(max_resident, ctl, charge)
}
pub fn render_rectilinear_mode_with_charge(
    font: &ExactFont<'_>,
    request: StemText<'_>,
    antialias: RectilinearAntialias,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<StemRendered, TextSourceError> {
    let prepared = prepare_rectilinear_with_charge(font, request, max_resident, ctl, charge)?;
    prepared.render_mode_with_charge(antialias, max_resident, ctl, charge)
}
/// Scaled AA only: observed no-AA scaled contours have distinct boundary
/// behavior and are deliberately unavailable through this additive API.
pub fn render_scaled_rectilinear_with_charge(
    font: &ExactFont<'_>,
    request: ScaledText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<StemRendered, TextSourceError> {
    let prepared = prepare_scaled_rectilinear_with_charge(font, request, max_resident, ctl, charge)?;
    prepared.render_mode_with_charge(RectilinearAntialias::AreaCoverage, max_resident, ctl, charge)
}

pub(super) fn prepare_underlined(
    font: &ExactFont<'_>,
    request: super::QualifiedUnderlinedText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    resident: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
    work: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<PreparedRectilinear, TextSourceError> {
    poll(ctl)?;
    if !matches!(request.request.size_px, 16. | 32.) || request.antialias == RectilinearAntialias::None && request.request.size_px != 32. {
        return Err(TextSourceError::Unsupported("unqualified underline size"));
    }
    qualify_scaled_rectilinear_geometry(font, request.request, ctl)?;
    work(4096)?;
    let prepared = prepare_spaced_impl(font, request.request, 0., max_resident, ctl, resident, Some(&request.runs))?;
    let edges = prepared.polygons.iter().take(prepared.len).try_fold(0u64, |n, p| n.checked_add(p.len as u64).ok_or(TextSourceError::Budget))?;
    let pixels = u64::from(prepared.rect.width()).checked_mul(u64::from(prepared.rect.height())).ok_or(TextSourceError::Budget)?;
    let complexity = if request.antialias == RectilinearAntialias::AreaCoverage {
        edges.checked_add(2).and_then(|v| v.checked_pow(3)).ok_or(TextSourceError::Budget)?
    } else {
        edges
    };
    work(pixels.checked_mul(complexity).ok_or(TextSourceError::Budget)?)?;
    poll(ctl)?;
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        let mut p = Polygon::default();
        p.points[..4].copy_from_slice(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
        p.len = 4;
        p.closed = true;
        p
    }
    #[test]
    fn area_integrates_union_and_partial_cells_not_sum_or_center() {
        let mut p =
            PreparedRectilinear { polygons: [Polygon::default(); 16], len: 2, rect: Rect::new(0, 0, 1, 1), layout_rect: Rect::new(0, 0, 1, 1), size_px: 32. };
        p.polygons[0] = rectangle(0.25, 0.25, 0.75, 0.75);
        p.polygons[1] = p.polygons[0];
        assert_eq!(p.area(0., 0., &Interrupt::NONE).unwrap(), 0.25);
        p.polygons[1] = rectangle(0.5, 0.25, 1., 0.75);
        assert_eq!(p.area(0., 0., &Interrupt::NONE).unwrap(), 0.375);
        assert_eq!(p.area(1., 0., &Interrupt::NONE).unwrap(), 0.);
        assert!(matches!(p.area(0., 0., &Interrupt::cancel_only(&|| true)), Err(TextSourceError::Cancelled)));
    }
}
