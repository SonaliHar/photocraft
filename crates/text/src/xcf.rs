//! Exact-authority, bounded XCF text regeneration foundations.
//!
//! Initial independently observed domain: Inter Regular, unhinted, no-AA white
//! 32 pixel dynamic LTR stems and bounded single-contour orthogonal glyphs. This does
//! not resolve fonts by name, consult a locator, or read the saved raster cache.
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};
mod config;
pub mod fixed_box_candidate;
pub mod outline;
pub mod outline_binary_candidate;
mod rectilinear;
pub mod runtime_font_input;
pub mod underline;
pub mod unit_layout;
pub mod unit_outline;
pub mod unit_raster_policy;
#[path = "xcf_units.rs"]
pub mod units;
pub use config::{
    QualifiedRectilinearText, QualifiedUnderlinedText, RectilinearLineLayout, TextColor, TextColorEncoding, prepare_qualified_rectilinear_with_charge,
    qualify_colored_rectilinear_source, qualify_layout_rectilinear_geometry, qualify_outline_source, qualify_placed_layout_colored_rectilinear_source,
    qualify_placed_spaced_colored_rectilinear_source, qualify_rectilinear_source, qualify_rectilinear_source_with_antialias, qualify_scaled_rectilinear_source,
    qualify_spaced_colored_rectilinear_source, qualify_stem_source, qualify_underlined_source, set_multiline_rectilinear_content, set_rectilinear_color,
    set_rectilinear_content, set_rectilinear_content_with_antialias, set_rectilinear_font_size, set_rectilinear_letter_spacing, set_rectilinear_line_spacing,
    set_rectilinear_origin, set_scaled_rectilinear_content, set_stem_content, set_underlined_ranges,
};
pub use rectilinear::{
    PreparedRectilinear, RectilinearAntialias, ScaledText, prepare_multiline_rectilinear_with_charge, prepare_rectilinear_with_charge,
    prepare_scaled_rectilinear_with_charge, prepare_spaced_rectilinear_with_charge, qualify_multiline_rectilinear_geometry, qualify_rectilinear_geometry,
    qualify_scaled_rectilinear_geometry, render_rectilinear_mode_with_charge, render_rectilinear_with_charge, render_scaled_rectilinear_with_charge,
};

/// Prepare the current retained text layout's explicit two-axis unit basis.
/// This does not authorize a font, validate the original AST against an evaluator,
/// or qualify rendering. Callers retain both axes instead of collapsing to scalar DPI.
pub fn prepare_source_unit_layout(
    item: &photocraft_doc::xcf_items::SourceItem,
    resolution: Option<&photocraft_doc::PhysicalResolution>,
    control: &Interrupt<'_>,
) -> Result<units::TextUnitLayout, TextSourceError> {
    use photocraft_doc::{xcf_filters::FilterValue, xcf_items::EditableSource};
    poll(control)?;
    let EditableSource::Text { configuration, .. } = &item.source else {
        return Err(TextSourceError::Unsupported("unit basis requires retained text source"));
    };
    if configuration.len() > 64 {
        return Err(TextSourceError::Budget);
    }
    let mut layout = None;
    for argument in configuration {
        poll(control)?;
        if argument.name.len() > 128 {
            return Err(TextSourceError::Budget);
        }
        if argument.name == "layout" {
            if layout.is_some() {
                return Err(TextSourceError::Unsupported("duplicate retained text layout"));
            }
            let FilterValue::Config(fields) = &argument.value else {
                return Err(TextSourceError::Unsupported("malformed retained text layout"));
            };
            layout = Some(fields.as_slice());
        }
    }
    units::resolve_layout(layout.ok_or(TextSourceError::Unsupported("missing retained text layout"))?, resolution, control)
}

const INTER_SHA: [u8; 32] = [
    0x40, 0xd6, 0x92, 0xfc, 0xe1, 0x88, 0xe4, 0x47, 0x1e, 0x2b, 0x3c, 0xba, 0x93, 0x7b, 0xe9, 0x67, 0x87, 0x8f, 0x63, 0x1a, 0xd3, 0xeb, 0xbb, 0xdc, 0xd5, 0x87,
    0x68, 0x7c, 0x7e, 0xbe, 0x0c, 0x82,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextSourceError {
    FontIdentity,
    Unsupported(&'static str),
    Budget,
    Allocation,
    Cancelled,
}
impl std::fmt::Display for TextSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XCF text regeneration: {self:?}")
    }
}
impl std::error::Error for TextSourceError {}
fn poll(ctl: &Interrupt<'_>) -> Result<(), TextSourceError> {
    if ctl.cancelled() { Err(TextSourceError::Cancelled) } else { Ok(()) }
}

/// Explicit caller-authorized borrowed bytes. Never persisted or obtained from
/// source resource paths; both retained SHA256 identity and qualified face match.
pub struct ExactFont<'a> {
    bytes: &'a [u8],
}
impl<'a> ExactFont<'a> {
    pub fn authorize(bytes: &'a [u8], retained_sha256: &[u8; 32], ctl: &Interrupt<'_>) -> Result<Self, TextSourceError> {
        poll(ctl)?;
        if bytes.len() != 411640 || retained_sha256 != &INTER_SHA {
            return Err(TextSourceError::FontIdentity);
        }
        crate::fonts::identify_authorized_font_bytes(bytes, retained_sha256, ctl).map_err(|error| match error {
            crate::fonts::FontIdentityError::Cancelled => TextSourceError::Cancelled,
            _ => TextSourceError::FontIdentity,
        })?;
        poll(ctl)?;
        Ok(Self { bytes })
    }
    /// Existing authorized OFL build asset, not a name lookup or fallback.
    pub fn bundled_inter(ctl: &Interrupt<'_>) -> Result<ExactFont<'static>, TextSourceError> {
        ExactFont::authorize(&crate::fonts::INTER_REGULAR[..], &INTER_SHA, ctl)
    }
}
#[derive(Clone, Copy)]
pub struct StemText<'a> {
    pub content: &'a str,
    pub origin: [i32; 2],
}
pub struct StemRendered {
    pub surface: Surface,
    pub rect: Rect,
    pub resident_bytes: u64,
}

/// Fixed-capacity public outline observer; curves/multiple contours are denied.
#[derive(Default)]
struct RectanglePen {
    points: [[f64; 2]; 4],
    count: usize,
    closed: bool,
    invalid: bool,
}
impl RectanglePen {
    fn point(&mut self, x: f32, y: f32) {
        if self.closed || !x.is_finite() || !y.is_finite() {
            self.invalid = true;
            return;
        }
        if let Some(p) = self.points.get_mut(self.count) {
            *p = [f64::from(x), f64::from(y)];
            self.count += 1;
        } else {
            self.invalid = true
        }
    }
}
impl OutlinePen for RectanglePen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.count != 0 {
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

/// Charges simultaneous output interleaved bytes and aligned raster tiles.
/// Font authority bytes are borrowed and remain the caller's residency charge.
pub fn render_stems(font: &ExactFont<'_>, request: StemText<'_>, max_resident: u64, ctl: &Interrupt<'_>) -> Result<StemRendered, TextSourceError> {
    render_stems_with_charge(font, request, max_resident, ctl, &mut |_| Ok(()))
}
/// Caller aggregate lease is acquired before output allocations. The caller
/// owns release on failure and after the returned surface is destroyed.
pub fn render_stems_with_charge(
    font: &ExactFont<'_>,
    request: StemText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<StemRendered, TextSourceError> {
    poll(ctl)?;
    let count = request.content.len();
    if !(1..=8).contains(&count) || request.content.bytes().any(|b| b != b'I') || request.origin.into_iter().any(|n| !(-(1 << 29)..=(1 << 29)).contains(&n)) {
        return Err(TextSourceError::Unsupported("unqualified text or placement"));
    }
    let face = skrifa::FontRef::new(font.bytes).map_err(|_| TextSourceError::FontIdentity)?;
    let location = LocationRef::default();
    let size = Size::new(32.);
    let glyph = face.charmap().map('I').ok_or(TextSourceError::FontIdentity)?;
    let advance = face.glyph_metrics(size, location).advance_width(glyph).ok_or(TextSourceError::FontIdentity)?;
    let ascent = face.metrics(size, location).ascent.ceil();
    if !advance.is_finite() || !ascent.is_finite() || !(0.0..=64.).contains(&advance) || !(0.0..=64.).contains(&ascent) {
        return Err(TextSourceError::FontIdentity);
    }
    let outline = face.outline_glyphs().get(glyph).ok_or(TextSourceError::FontIdentity)?;
    let mut pen = RectanglePen::default();
    let mut outline_memory = [0_u8; 16384];
    outline
        .draw(
            DrawSettings::unhinted(size, location)
                .with_memory(Some(outline_memory.get_mut(..outline.draw_memory_size(skrifa::outline::Hinting::None)).ok_or(TextSourceError::Budget)?)),
            &mut pen,
        )
        .map_err(|_| TextSourceError::FontIdentity)?;
    if pen.invalid || !pen.closed || pen.count != 4 {
        return Err(TextSourceError::Unsupported("nonrectangular glyph"));
    }
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (index, p) in pen.points.iter().enumerate() {
        let next = pen.points.get((index + 1) % 4).ok_or(TextSourceError::FontIdentity)?;
        if p[0] != next[0] && p[1] != next[1] {
            return Err(TextSourceError::Unsupported("nonaxis glyph"));
        }
        bounds[0] = bounds[0].min(p[0]);
        bounds[1] = bounds[1].min(p[1]);
        bounds[2] = bounds[2].max(p[0]);
        bounds[3] = bounds[3].max(p[1]);
    }
    let x = f64::from(request.origin[0]);
    let baseline = f64::from(request.origin[1]) + f64::from(ascent);
    let [left, bottom, right, top] = bounds;
    let x0 = x + left;
    let y0 = baseline - top;
    let x1 = x + f64::from(advance.round()) * (count - 1) as f64 + right;
    let y1 = baseline - bottom;
    let rect = Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32);
    if [rect.x0, rect.y0, rect.x1, rect.y1].into_iter().any(|n| !(-(1 << 29)..=(1 << 29)).contains(&n)) {
        return Err(TextSourceError::Unsupported("text output exceeds safe coordinates"));
    }
    let width = usize::try_from(rect.width()).map_err(|_| TextSourceError::Budget)?;
    let height = usize::try_from(rect.height()).map_err(|_| TextSourceError::Budget)?;
    let bytes = width.checked_mul(height).and_then(|n| n.checked_mul(4)).ok_or(TextSourceError::Budget)?;
    let tiles_x = i64::from(rect.x1 - 1).div_euclid(256) - i64::from(rect.x0).div_euclid(256) + 1;
    let tiles_y = i64::from(rect.y1 - 1).div_euclid(256) - i64::from(rect.y0).div_euclid(256) + 1;
    let resident = (bytes as u64)
        .checked_add((tiles_x as u64).checked_mul(tiles_y as u64).and_then(|n| n.checked_mul(256 * 256 * 4)).ok_or(TextSourceError::Budget)?)
        .ok_or(TextSourceError::Budget)?;
    if resident > max_resident {
        return Err(TextSourceError::Budget);
    }
    charge(resident)?;
    poll(ctl)?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(bytes).map_err(|_| TextSourceError::Allocation)?;
    pixels.resize(bytes, 0);
    for row in 0..height {
        poll(ctl)?;
        let py = f64::from(rect.y0) + row as f64;
        let vertical = (y1.min(py + 1.) - y0.max(py)).max(0.);
        for column in 0..width {
            let px = f64::from(rect.x0) + column as f64;
            let mut coverage = 0.;
            for glyph in 0..count {
                let gx = x0 + glyph as f64 * f64::from(advance.round());
                coverage += ((gx + (right - left)).min(px + 1.) - gx.max(px)).max(0.) * vertical;
            }
            if coverage >= 0.5 {
                let offset = (row * width + column) * 4;
                if let Some(pixel) = pixels.get_mut(offset..offset + 4) {
                    pixel.fill(255);
                } else {
                    return Err(TextSourceError::Budget);
                }
            }
        }
    }
    let surface = Surface::try_from_interleaved_with(PixelFormat::RGBA8, rect, &pixels, ctl)
        .map_err(|_| if ctl.cancelled() { TextSourceError::Cancelled } else { TextSourceError::Allocation })?;
    poll(ctl)?;
    Ok(StemRendered { surface, rect, resident_bytes: resident })
}
