//! Candidate unhinted vector plan from exact authorized font and positioned glyphs.
//! Coordinates retain independent X/Y em scales; no raster/cache/source admission.
use super::{
    TextSourceError, poll,
    unit_layout::{AuthorizedSourceFont, PreparedUnitLayout},
};
use photocraft_raster::Interrupt;
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};
type Result<T> = std::result::Result<T, TextSourceError>;
const MAX_COMMANDS: usize = 16384;
const SCRATCH: usize = 65536;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Move([f64; 2]),
    Line([f64; 2]),
    Quad([f64; 2], [f64; 2]),
    Cubic([f64; 2], [f64; 2], [f64; 2]),
    Close,
}
#[derive(Clone, Copy, Debug)]
pub struct GlyphSpan {
    pub glyph_id: u32,
    pub start: usize,
    pub end: usize,
}
pub struct PreparedUnitOutline {
    pub raster_policy: super::unit_raster_policy::UnhintedRasterPolicy,
    pub commands: Vec<Command>,
    pub glyphs: Vec<GlyphSpan>,
    pub retained_bytes: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
    pub commands: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: 32 * 1024 * 1024, work_units: 8_000_000, commands: MAX_COMMANDS }
    }
}
struct Pen<'a, 'b> {
    commands: &'a mut Vec<Command>,
    position: [f64; 2],
    scale: [f64; 2],
    max: usize,
    control: &'a Interrupt<'b>,
    error: Option<TextSourceError>,
}
impl Pen<'_, '_> {
    fn point(&mut self, x: f32, y: f32) -> [f64; 2] {
        let p = [self.position[0] + f64::from(x) * self.scale[0], self.position[1] - f64::from(y) * self.scale[1]];
        if !x.is_finite() || !y.is_finite() || p.iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29)) {
            self.error = Some(TextSourceError::Unsupported("candidate outline coordinate domain"));
        }
        p
    }
    fn push(&mut self, c: Command) {
        if self.error.is_some() {
            return;
        }
        if let Err(e) = poll(self.control) {
            self.error = Some(e);
            return;
        }
        if self.commands.len() >= self.max || self.commands.len() >= self.commands.capacity() {
            self.error = Some(TextSourceError::Budget);
            return;
        }
        self.commands.push(c);
    }
}
impl OutlinePen for Pen<'_, '_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.push(Command::Move(p));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.push(Command::Line(p));
    }
    fn quad_to(&mut self, x: f32, y: f32, ex: f32, ey: f32) {
        let a = self.point(x, y);
        let b = self.point(ex, ey);
        self.push(Command::Quad(a, b));
    }
    fn curve_to(&mut self, x: f32, y: f32, cx: f32, cy: f32, ex: f32, ey: f32) {
        let a = self.point(x, y);
        let b = self.point(cx, cy);
        let c = self.point(ex, ey);
        self.push(Command::Cubic(a, b, c));
    }
    fn close(&mut self) {
        self.push(Command::Close);
    }
}
/// Font identity must match the privately bound layout plan; caller cannot replace
/// it by name. Only unhinted default-instance outlines are supported. No raster
/// antialiasing, bidi, source transform or color-space policy is guessed here.
pub fn prepare(
    font: &AuthorizedSourceFont<'_>,
    layout: &PreparedUnitLayout,
    limits: Limits,
    control: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<PreparedUnitOutline> {
    poll(control)?;
    let d = Limits::default();
    if limits.resident_bytes > d.resident_bytes
        || limits.work_units > d.work_units
        || limits.commands > d.commands
        || limits.commands == 0
        || layout.glyphs.len() > 256
    {
        return Err(TextSourceError::Budget);
    }
    if font.resource != layout.font_resource || font.sha256 != layout.font_sha256 || font.index != layout.font_index {
        return Err(TextSourceError::FontIdentity);
    }
    if layout.pixels_per_font_unit.iter().any(|v| !v.is_finite() || *v <= 0.0 || *v > 4096.0) {
        return Err(TextSourceError::Unsupported("candidate outline scale"));
    }
    let command_bytes = (limits.commands as u64).checked_mul(std::mem::size_of::<Command>() as u64).ok_or(TextSourceError::Budget)?;
    let span_bytes = (layout.glyphs.len() as u64).checked_mul(std::mem::size_of::<GlyphSpan>() as u64).ok_or(TextSourceError::Budget)?;
    let held = (font.bytes.len() as u64).checked_add(layout.retained_bytes).ok_or(TextSourceError::Budget)?;
    let resident =
        held.checked_add(command_bytes).and_then(|n| n.checked_add(span_bytes)).and_then(|n| n.checked_add(SCRATCH as u64)).ok_or(TextSourceError::Budget)?;
    let work = (layout.glyphs.len() as u64)
        .checked_mul(limits.commands as u64)
        .and_then(|n| n.checked_add(font.bytes.len() as u64 / 4))
        .ok_or(TextSourceError::Budget)?;
    if resident > limits.resident_bytes || work > limits.work_units {
        return Err(TextSourceError::Budget);
    }
    // Charge new simultaneous capacity; borrowed font/layout held envelope is
    // already owned by the caller and checked above, not allocated by this step.
    charge(command_bytes.checked_add(span_bytes).and_then(|n| n.checked_add(SCRATCH as u64)).ok_or(TextSourceError::Budget)?)?;
    poll(control)?;
    let mut commands = Vec::new();
    commands.try_reserve_exact(limits.commands).map_err(|_| TextSourceError::Allocation)?;
    let mut glyphs = Vec::new();
    glyphs.try_reserve_exact(layout.glyphs.len()).map_err(|_| TextSourceError::Allocation)?;
    let mut scratch = Vec::new();
    scratch.try_reserve_exact(SCRATCH).map_err(|_| TextSourceError::Allocation)?;
    scratch.resize(SCRATCH, 0u8);
    let actual_commands = (commands.capacity() as u64).checked_mul(std::mem::size_of::<Command>() as u64).ok_or(TextSourceError::Budget)?;
    let actual_spans = (glyphs.capacity() as u64).checked_mul(std::mem::size_of::<GlyphSpan>() as u64).ok_or(TextSourceError::Budget)?;
    if actual_commands > command_bytes || actual_spans > span_bytes || scratch.capacity() > SCRATCH {
        return Err(TextSourceError::Budget);
    }
    let face = skrifa::FontRef::from_index(font.bytes, font.index).map_err(|_| TextSourceError::FontIdentity)?;
    let outlines = face.outline_glyphs();
    for g in &layout.glyphs {
        poll(control)?;
        if g.position.iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29)) {
            return Err(TextSourceError::Unsupported("candidate glyph position"));
        }
        let start = commands.len();
        if let Some(outline) = outlines.get(skrifa::GlyphId::new(g.glyph_id)) {
            let memory = outline.draw_memory_size(skrifa::outline::Hinting::None);
            let mut pen = Pen { commands: &mut commands, position: g.position, scale: layout.pixels_per_font_unit, max: limits.commands, control, error: None };
            outline
                .draw(
                    DrawSettings::unhinted(Size::unscaled(), LocationRef::default())
                        .with_memory(Some(scratch.get_mut(..memory).ok_or(TextSourceError::Budget)?)),
                    &mut pen,
                )
                .map_err(|_| TextSourceError::FontIdentity)?;
            if let Some(error) = pen.error {
                return Err(error);
            }
        } else if !g.empty_outline_allowed {
            return Err(TextSourceError::Unsupported("candidate font has no supported glyph outline"));
        }
        // Empty glyphs such as space retain a zero-length span and their shaped advance.
        glyphs.push(GlyphSpan { glyph_id: g.glyph_id, start, end: commands.len() });
    }
    poll(control)?;
    Ok(PreparedUnitOutline {
        raster_policy: layout.raster_policy,
        commands,
        glyphs,
        retained_bytes: actual_commands.checked_add(actual_spans).ok_or(TextSourceError::Budget)?,
    })
}
