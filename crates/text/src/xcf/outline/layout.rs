//! Charged exact-font whole-string shaping and generic quadratic AA rendering.
use super::{CoverageWorkspace, Point, Result, coverage, quadratic_24_8};
use crate::xcf::{ExactFont, ScaledText, StemRendered, TextSourceError, poll};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};

const SHAPING: u64 = 2 * 1024 * 1024;
#[derive(Clone, Copy, Default)]
pub(crate) enum Command {
    #[default]
    Empty,
    Move(Point),
    Line(Point),
    Quad(Point, Point),
    Close,
}
pub(crate) struct Pen {
    pub(crate) commands: [Command; 256],
    pub(crate) len: usize,
    pub(crate) invalid: bool,
}
impl Pen {
    pub(crate) fn new() -> Self {
        Self { commands: [Command::Empty; 256], len: 0, invalid: false }
    }
    fn point(&mut self, x: f32, y: f32) -> Point {
        if !x.is_finite() || !y.is_finite() || x.abs() > 128. || y.abs() > 128. {
            self.invalid = true;
            return [0, 0];
        }
        [(f64::from(x) * 64.).round() as i64 * 4, (f64::from(y) * 64.).round() as i64 * 4]
    }
    fn push(&mut self, command: Command) {
        if let Some(slot) = self.commands.get_mut(self.len) {
            *slot = command;
            self.len += 1;
        } else {
            self.invalid = true;
        }
    }
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.push(Command::Move(p));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.push(Command::Line(p));
    }
    fn quad_to(&mut self, x: f32, y: f32, ex: f32, ey: f32) {
        let p = self.point(x, y);
        let end = self.point(ex, ey);
        self.push(Command::Quad(p, end));
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.invalid = true;
    }
    fn close(&mut self) {
        self.push(Command::Close);
    }
}
struct Glyph {
    points: Vec<Point>,
    ends: [usize; 32],
    contours: usize,
}
pub struct PreparedOutlineText {
    glyphs: Vec<Glyph>,
    rect: Rect,
    layout: Rect,
    retained: u64,
}
/// Stack-only primitive eligibility. Actual charged preparation separately
/// validates positioned glyphs and the complete whole-string layout.
pub fn qualify_outline_geometry(font: &ExactFont<'_>, request: ScaledText<'_>, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    if !matches!(request.size_px, 16. | 32. | 40.)
        || !(1..=8).contains(&request.content.len())
        || request.content.bytes().any(|b| !(32..=126).contains(&b))
        || request.origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v))
    {
        return Err(TextSourceError::Unsupported("unqualified outline text layout"));
    }
    let face = skrifa::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let mut memory = [0_u8; 16384];
    let mut ink = false;
    for c in request.content.chars() {
        poll(ctl)?;
        let id = face.charmap().map(c).ok_or(TextSourceError::Unsupported("missing source glyph"))?;
        if let Some(outline) = face.outline_glyphs().get(id) {
            let mut pen = Pen::new();
            outline
                .draw(
                    DrawSettings::unhinted(Size::new(request.size_px), LocationRef::default())
                        .with_memory(Some(memory.get_mut(..outline.draw_memory_size(skrifa::outline::Hinting::None)).ok_or(TextSourceError::Budget)?)),
                    &mut pen,
                )
                .map_err(|_| TextSourceError::FontIdentity)?;
            if pen.invalid {
                return Err(TextSourceError::Unsupported("unsupported or unbounded glyph outline"));
            }
            ink |= pen.len != 0;
        } else if c != ' ' {
            return Err(TextSourceError::Unsupported("missing source glyph outline"));
        }
    }
    poll(ctl)?;
    if ink { Ok(()) } else { Err(TextSourceError::Unsupported("empty text ink")) }
}
impl PreparedOutlineText {
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn layout_rect(&self) -> Rect {
        self.layout
    }
    pub fn render_with_charge(
        &self,
        max_resident: u64,
        ctl: &Interrupt<'_>,
        resident: &mut dyn FnMut(u64) -> Result<()>,
        work: &mut dyn FnMut(u64) -> Result<()>,
    ) -> Result<StemRendered> {
        poll(ctl)?;
        let width = usize::try_from(self.rect.width()).map_err(|_| TextSourceError::Budget)?;
        let height = usize::try_from(self.rect.height()).map_err(|_| TextSourceError::Budget)?;
        let bytes = width.checked_mul(height).and_then(|v| v.checked_mul(4)).ok_or(TextSourceError::Budget)?;
        let tx = i64::from(self.rect.x1 - 1).div_euclid(256) - i64::from(self.rect.x0).div_euclid(256) + 1;
        let ty = i64::from(self.rect.y1 - 1).div_euclid(256) - i64::from(self.rect.y0).div_euclid(256) + 1;
        let output = (bytes as u64)
            .checked_add((tx as u64).checked_mul(ty as u64).and_then(|v| v.checked_mul(256 * 256 * 4)).ok_or(TextSourceError::Budget)?)
            .ok_or(TextSourceError::Budget)?;
        let scratch = (super::CLIP_CAPACITY * std::mem::size_of::<Point>() * 2) as u64;
        let total = self.retained.checked_add(output).and_then(|v| v.checked_add(scratch)).ok_or(TextSourceError::Budget)?;
        if total > max_resident {
            return Err(TextSourceError::Budget);
        }
        let edges = self.glyphs.iter().try_fold(0_u64, |a, g| a.checked_add(g.points.len() as u64).ok_or(TextSourceError::Budget))?;
        work((width as u64).checked_mul(height as u64).and_then(|v| v.checked_mul(edges)).and_then(|v| v.checked_mul(5)).ok_or(TextSourceError::Budget)?)?;
        resident(output)?;
        let mut workspace = CoverageWorkspace::new(resident)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(bytes).map_err(|_| TextSourceError::Allocation)?;
        if pixels.capacity() > bytes {
            return Err(TextSourceError::Budget);
        }
        pixels.resize(bytes, 0);
        for row in 0..height {
            poll(ctl)?;
            for col in 0..width {
                let mut alpha = 0_u8;
                for glyph in &self.glyphs {
                    let mut views: [&[Point]; 32] = [&[]; 32];
                    let mut start = 0;
                    for (index, end) in glyph.ends.iter().take(glyph.contours).enumerate() {
                        *views.get_mut(index).ok_or(TextSourceError::Budget)? = glyph.points.get(start..*end).ok_or(TextSourceError::Budget)?;
                        start = *end;
                    }
                    alpha = alpha.saturating_add(coverage(
                        &mut workspace,
                        views.get(..glyph.contours).ok_or(TextSourceError::Budget)?,
                        [self.rect.x0 + col as i32, self.rect.y0 + row as i32],
                        true,
                        ctl,
                    )?);
                }
                if alpha != 0 {
                    pixels
                        .get_mut((row * width + col) * 4..(row * width + col) * 4 + 4)
                        .ok_or(TextSourceError::Budget)?
                        .copy_from_slice(&[255, 255, 255, alpha]);
                }
            }
        }
        let surface = Surface::try_from_interleaved_with(PixelFormat::RGBA8, self.rect, &pixels, ctl)
            .map_err(|_| if ctl.cancelled() { TextSourceError::Cancelled } else { TextSourceError::Allocation })?;
        poll(ctl)?;
        Ok(StemRendered { surface, rect: self.rect, resident_bytes: total })
    }
}

pub fn prepare_outline_with_charge(
    font: &ExactFont<'_>,
    request: ScaledText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    resident: &mut dyn FnMut(u64) -> Result<()>,
    work: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<PreparedOutlineText> {
    poll(ctl)?;
    if !matches!(request.size_px, 16. | 32. | 40.)
        || !(1..=8).contains(&request.content.len())
        || request.content.bytes().any(|b| !(32..=126).contains(&b))
        || request.origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v))
    {
        return Err(TextSourceError::Unsupported("unqualified outline text layout"));
    }
    let mut retained = 0_u64;
    let mut charge = |bytes: u64| {
        let next = retained.checked_add(bytes).ok_or(TextSourceError::Budget)?;
        if next > max_resident {
            return Err(TextSourceError::Budget);
        }
        resident(bytes)?;
        retained = next;
        Ok(())
    };
    charge(SHAPING)?;
    work(4096)?;
    let shaping_font = harfrust::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let data = harfrust::ShaperData::new(&shaping_font);
    let shaper = data.shaper(&shaping_font).build();
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(request.content);
    buffer.set_direction(harfrust::Direction::LeftToRight);
    buffer.set_script(harfrust::script::LATIN);
    let shaped = shaper.shape(buffer, harfrust::ShapeOptions::default());
    poll(ctl)?;
    if shaped.glyph_infos().is_empty() || shaped.glyph_infos().len() > 32 || shaped.glyph_infos().len() != shaped.glyph_positions().len() {
        return Err(TextSourceError::Unsupported("unsafe shaped text"));
    }
    let face = skrifa::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let size = Size::new(request.size_px);
    let metrics = face.metrics(size, LocationRef::default());
    let baseline = metrics.ascent.ceil();
    let height = baseline - metrics.descent.floor();
    if !baseline.is_finite() || !(0. ..=64.).contains(&baseline) || !height.is_finite() || !(1. ..=128.).contains(&height) || shaper.units_per_em() == 0 {
        return Err(TextSourceError::FontIdentity);
    }
    charge((32 * std::mem::size_of::<Glyph>()) as u64)?;
    let mut glyphs = Vec::new();
    glyphs.try_reserve_exact(32).map_err(|_| TextSourceError::Allocation)?;
    if glyphs.capacity() > 32 {
        return Err(TextSourceError::Budget);
    }
    let mut memory = [0_u8; 16384];
    let mut advance = 0_i64;
    let mut bounds = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        poll(ctl)?;
        if info.glyph_id == 0
            || !request.content.is_char_boundary(info.cluster as usize)
            || info.cluster as usize >= request.content.len()
            || [pos.x_offset, pos.y_offset, pos.y_advance] != [0; 3]
        {
            return Err(TextSourceError::Unsupported("unqualified positioned glyph"));
        }
        let step = (f64::from(pos.x_advance) * f64::from(request.size_px) / f64::from(shaper.units_per_em())).round();
        if !(0. ..=128.).contains(&step) {
            return Err(TextSourceError::Unsupported("unsafe glyph advance"));
        }
        if let Some(outline) = face.outline_glyphs().get(skrifa::GlyphId::new(info.glyph_id)) {
            let mut pen = Pen::new();
            outline
                .draw(
                    DrawSettings::unhinted(size, LocationRef::default())
                        .with_memory(Some(memory.get_mut(..outline.draw_memory_size(skrifa::outline::Hinting::None)).ok_or(TextSourceError::Budget)?)),
                    &mut pen,
                )
                .map_err(|_| TextSourceError::FontIdentity)?;
            if pen.invalid {
                return Err(TextSourceError::Unsupported("unsupported or unbounded glyph outline"));
            }
            if pen.len != 0 {
                charge((4096 * std::mem::size_of::<Point>()) as u64)?;
                let mut glyph = Glyph { points: Vec::new(), ends: [0; 32], contours: 0 };
                glyph.points.try_reserve_exact(4096).map_err(|_| TextSourceError::Allocation)?;
                if glyph.points.capacity() > 4096 {
                    return Err(TextSourceError::Budget);
                }
                let mut current = None;
                let mut open = false;
                for command in pen.commands.iter().take(pen.len) {
                    poll(ctl)?;
                    work(1)?;
                    let mut append = |p| {
                        if glyph.points.len() >= 4096 {
                            return Err(TextSourceError::Budget);
                        }
                        glyph.points.push(p);
                        Ok(())
                    };
                    match *command {
                        Command::Move(p) => {
                            if open {
                                return Err(TextSourceError::Unsupported("unclosed glyph contour"));
                            }
                            append(p)?;
                            current = Some(p);
                            open = true;
                        }
                        Command::Line(p) => {
                            if !open {
                                return Err(TextSourceError::Unsupported("glyph line without contour"));
                            }
                            append(p)?;
                            current = Some(p);
                        }
                        Command::Quad(control, end) => {
                            let start = current.ok_or(TextSourceError::Unsupported("glyph curve without contour"))?;
                            let poly = quadratic_24_8([start, control, end], ctl, &mut charge)?;
                            work(poly.len() as u64)?;
                            for p in poly.iter().skip(1) {
                                append(*p)?;
                            }
                            current = Some(end);
                        }
                        Command::Close => {
                            if !open {
                                return Err(TextSourceError::Unsupported("glyph closure without contour"));
                            }
                            *glyph.ends.get_mut(glyph.contours).ok_or(TextSourceError::Budget)? = glyph.points.len();
                            glyph.contours += 1;
                            current = None;
                            open = false;
                        }
                        Command::Empty => return Err(TextSourceError::Unsupported("invalid glyph command")),
                    }
                }
                if open || glyph.contours == 0 {
                    return Err(TextSourceError::Unsupported("unclosed glyph outline"));
                }
                for p in &mut glyph.points {
                    p[0] += (i64::from(request.origin[0]) + advance) * 256;
                    p[1] = (i64::from(request.origin[1]) + baseline as i64) * 256 - p[1];
                    if !super::point_valid(*p) {
                        return Err(TextSourceError::Unsupported("unsafe glyph placement"));
                    }
                    bounds[0] = bounds[0].min(p[0]);
                    bounds[1] = bounds[1].min(p[1]);
                    bounds[2] = bounds[2].max(p[0]);
                    bounds[3] = bounds[3].max(p[1]);
                }
                glyphs.push(glyph);
            }
        } else if info.glyph_id != face.charmap().map(' ').ok_or(TextSourceError::FontIdentity)?.to_u32() {
            return Err(TextSourceError::Unsupported("missing glyph outline"));
        }
        advance = advance.checked_add(step as i64).ok_or(TextSourceError::Budget)?;
    }
    if glyphs.is_empty() || advance == 0 {
        return Err(TextSourceError::Unsupported("empty text ink"));
    }
    let convert = |v: i64| i32::try_from(v).map_err(|_| TextSourceError::Budget);
    let rect = Rect::new(
        convert(bounds[0].div_euclid(256))?,
        convert(bounds[1].div_euclid(256))?,
        convert(-(-bounds[2]).div_euclid(256))?,
        convert(-(-bounds[3]).div_euclid(256))?,
    );
    let layout_max = [i64::from(request.origin[0]) + advance, i64::from(request.origin[1]) + height as i64];
    if layout_max.into_iter().any(|v| !(-(1_i64 << 29)..=(1_i64 << 29)).contains(&v)) {
        return Err(TextSourceError::Unsupported("unsafe text layout bounds"));
    }
    let layout = Rect::new(request.origin[0], request.origin[1], convert(layout_max[0])?, convert(layout_max[1])?);
    poll(ctl)?;
    Ok(PreparedOutlineText { glyphs, rect, layout, retained })
}
