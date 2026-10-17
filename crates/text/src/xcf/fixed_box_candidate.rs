//! Explicit external-font fixed-box32px candidate from six authored states.
//! No default source admission; only literal two-line I/II, LTR, white no-AA,
//! hint-none96x80 pixel boxes. Placement uses existing outlines and line metrics.
use super::{ExactFont, RectilinearAntialias, StemRendered, StemText, TextSourceError, poll, prepare_rectilinear_with_charge};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub requested_output_format: PixelFormat,
    pub content: &'a str,
    pub origin: [i32; 2],
    pub alignment: Alignment,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: 8 << 20, work_units: 1_000_000 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub peak_bytes: u64,
    pub work_units: u64,
}
pub fn requirements(request: Request<'_>, limits: Limits) -> Result<Requirements, TextSourceError> {
    if request.requested_output_format != PixelFormat::RGBA8
        || !matches!(request.content, "II\nI" | "I\nII")
        || request.origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v))
    {
        return Err(TextSourceError::Unsupported("unobserved fixed-box text candidate"));
    }
    let defaults = Limits::default();
    if limits.resident_bytes > defaults.resident_bytes || limits.work_units > defaults.work_units {
        return Err(TextSourceError::Budget);
    }
    // Two precharged shaping phases, final and one simultaneous glyph tile
    // envelope, exact font borrowed authority, plane/row and map/stack reserve.
    let peak_bytes = (4 << 20) + (2 * 4 * 256 * 256 * 4) + (96 * 80 * 4) + (96 * 4) + 411640 + 8192;
    let work_units = 96 * 80 * 64 + 65536;
    if peak_bytes > limits.resident_bytes || work_units > limits.work_units {
        return Err(TextSourceError::Budget);
    }
    Ok(Requirements { peak_bytes, work_units })
}
/// The caller's font must already be authorized against the exact external face.
/// No font registration, locator read, saved-cache sampling or font bytes persist.
pub fn render_with_charge(
    font: &ExactFont<'_>,
    request: Request<'_>,
    limits: Limits,
    c: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<StemRendered, TextSourceError> {
    poll(c)?;
    let req = requirements(request, limits)?;
    charge(req.peak_bytes)?;
    let (first, second) = request.content.split_once('\n').ok_or(TextSourceError::Unsupported("two lines required"))?;
    let [ox, oy] = request.origin;
    let rect = Rect::new(ox, oy, ox.checked_add(96).ok_or(TextSourceError::Budget)?, oy.checked_add(80).ok_or(TextSourceError::Budget)?);
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(96 * 80 * 4).map_err(|_| TextSourceError::Allocation)?;
    if pixels.capacity() != 96 * 80 * 4 {
        return Err(TextSourceError::Budget);
    }
    while pixels.len() < 96 * 80 * 4 {
        poll(c)?;
        pixels.resize(pixels.len().saturating_add(4096).min(96 * 80 * 4), 0u8);
    }
    let mut row = [0u8; 96 * 4];
    for (line_index, line) in [first, second].into_iter().enumerate() {
        poll(c)?;
        // The literal experiment measured advances 9/18 and line height 39.
        // Validate those metrics on the actual prepared outline below; never
        // shape a second time merely to obtain placement metadata.
        let expected_width = if line.len() == 1 { 9 } else { 18 };
        let remaining = 96i32 - expected_width;
        let dx = match request.alignment {
            Alignment::Left => 0,
            Alignment::Center => remaining.checked_add(1).ok_or(TextSourceError::Budget)? / 2,
            Alignment::Right => remaining,
        };
        let x = ox.checked_add(dx).ok_or(TextSourceError::Budget)?;
        let y = oy
            .checked_add(i32::try_from(line_index).map_err(|_| TextSourceError::Budget)?.checked_mul(39).ok_or(TextSourceError::Budget)?)
            .ok_or(TextSourceError::Budget)?;
        let prepared = prepare_rectilinear_with_charge(font, StemText { content: line, origin: [x, y] }, limits.resident_bytes, c, &mut |_| Ok(()))?;
        if prepared.layout_rect().width() != u32::try_from(expected_width).map_err(|_| TextSourceError::Budget)? || prepared.layout_rect().height() != 39 {
            return Err(TextSourceError::Unsupported("fixed-box candidate font metrics differ"));
        }
        let glyph = prepared.render_mode_with_charge(RectilinearAntialias::None, limits.resident_bytes, c, &mut |_| Ok(()))?;
        let ink = glyph.rect.intersect(&rect);
        for y in ink.y0..ink.y1 {
            poll(c)?;
            let length = usize::try_from(ink.width()).map_err(|_| TextSourceError::Budget)?.checked_mul(4).ok_or(TextSourceError::Budget)?;
            let bytes = row.get_mut(..length).ok_or(TextSourceError::Budget)?;
            glyph
                .surface
                .try_read_interleaved_into_with(Rect::new(ink.x0, y, ink.x1, y.checked_add(1).ok_or(TextSourceError::Budget)?), bytes, c)
                .map_err(|e| if matches!(e, photocraft_raster::RasterInputError::Cancelled) { TextSourceError::Cancelled } else { TextSourceError::Budget })?;
            let target = usize::try_from(y - oy)
                .map_err(|_| TextSourceError::Budget)?
                .checked_mul(96)
                .and_then(|n| usize::try_from(ink.x0 - ox).ok().and_then(|x| n.checked_add(x)))
                .and_then(|n| n.checked_mul(4))
                .ok_or(TextSourceError::Budget)?;
            pixels.get_mut(target..target.checked_add(length).ok_or(TextSourceError::Budget)?).ok_or(TextSourceError::Budget)?.copy_from_slice(bytes);
        }
    }
    let surface = Surface::try_from_interleaved_with(request.requested_output_format, rect, &pixels, c)
        .map_err(|e| if matches!(e, photocraft_raster::RasterInputError::Cancelled) { TextSourceError::Cancelled } else { TextSourceError::Budget })?;
    poll(c)?;
    Ok(StemRendered { surface, rect, resident_bytes: req.peak_bytes })
}

use photocraft_doc::{
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
    xcf_items::{EditableSource, SourceItem},
    xcf_resources::{ResourceKind, ResourceReference},
};
type SourceResult<T> = std::result::Result<T, TextSourceError>;
fn invalid() -> TextSourceError {
    TextSourceError::Unsupported("unqualified canonical text configuration")
}
fn field<'a>(fields: &'a [FilterArgument], name: &str) -> SourceResult<&'a FilterValue> {
    if fields.len() > 32 {
        return Err(invalid());
    }
    let mut found = None;
    for f in fields {
        if f.name == name {
            if found.is_some() {
                return Err(invalid());
            }
            found = Some(&f.value);
        }
    }
    found.ok_or_else(invalid)
}
fn config(value: &FilterValue) -> SourceResult<&[FilterArgument]> {
    if let FilterValue::Config(v) = value { Ok(v) } else { Err(invalid()) }
}
fn string(value: &FilterValue, expected: &str) -> bool {
    matches!(value,FilterValue::String(v) if v==expected)
}
fn uint(value: &FilterValue, width: u8, expected: u64) -> bool {
    matches!(value,FilterValue::Number(ExactNumber::Unsigned{width:w,value:v}) if *w==width && *v==expected)
}
fn signed(value: &FilterValue, expected: i64) -> bool {
    matches!(value,FilterValue::Number(ExactNumber::Signed{width:32,value:v}) if *v==expected)
}
fn names(fields: &[FilterArgument], expected: &[&str]) -> SourceResult<()> {
    if fields.len() != expected.len() {
        return Err(invalid());
    }
    for name in expected {
        field(fields, name)?;
    }
    Ok(())
}
fn hex_matches(hex: &str, bytes: &[u8]) -> bool {
    if bytes.len().checked_mul(2) != Some(hex.len()) {
        return false;
    }
    const DIGITS: &[u8] = b"0123456789abcdef";
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .zip(bytes)
        .all(|(h, b)| DIGITS.get(usize::from(b >> 4)) == h.first() && DIGITS.get(usize::from(b & 15)) == h.get(1))
}
struct Ast<'a>(&'a [FilterArgument]);
impl<'a> Ast<'a> {
    fn value(&self, index: usize, kind: &str) -> SourceResult<&'a FilterValue> {
        let node = self.0.get(index).ok_or_else(invalid)?;
        if node.name.len() > 64 {
            return Err(invalid());
        }
        let (prefix, suffix) = node.name.rsplit_once('-').ok_or_else(invalid)?;
        if prefix != kind || suffix.parse::<usize>().ok() != Some(index) {
            return Err(invalid());
        }
        Ok(&node.value)
    }
    fn atom(&self, index: usize, value: &str) -> SourceResult<()> {
        if string(self.value(index, "atom")?, value) { Ok(()) } else { Err(invalid()) }
    }
    fn quoted(&self, index: usize, value: &[u8]) -> SourceResult<()> {
        if let FilterValue::String(v) = self.value(index, "quoted-bytes-hex")?
            && hex_matches(v, value)
        {
            return Ok(());
        }
        Err(invalid())
    }
    fn number(&self, index: usize, expected: f64) -> SourceResult<()> {
        let v = config(self.value(index, "number")?)?;
        names(v, &["decimal", "f64-bits"])?;
        let decimal = field(v, "decimal")?;
        if !matches!(decimal,FilterValue::String(s) if s.len()<=32 && s.parse::<f64>().ok()==Some(expected))
            || !matches!(field(v,"f64-bits")?,FilterValue::Number(ExactNumber::Float64Bits(bits)) if *bits==expected.to_bits())
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn list(&self, index: usize) -> SourceResult<Ast<'a>> {
        Ok(Ast(config(self.value(index, "list")?)?))
    }
    fn scalar_atom(&self, key: &str, value: &str) -> SourceResult<()> {
        if self.0.len() != 2 {
            return Err(invalid());
        }
        self.atom(0, key)?;
        self.atom(1, value)
    }
    fn scalar_number(&self, key: &str, value: f64) -> SourceResult<()> {
        if self.0.len() != 2 {
            return Err(invalid());
        }
        self.atom(0, key)?;
        self.number(1, value)
    }
}
fn color(ast: Ast<'_>, rgb: &[u8]) -> SourceResult<()> {
    if ast.0.len() != 5 {
        return Err(invalid());
    }
    ast.atom(0, "color")?;
    ast.quoted(1, b"R'G'B'A float")?;
    ast.number(2, 16.)?;
    ast.quoted(3, rgb)?;
    ast.number(4, 0.)
}

/// Borrowed candidate binding only. ExactFont is separate runtime authority;
/// resource selectors cannot authorize a read or embed a font in the document.
/// This preserves the original AST, cache, current fields and resource metadata.
#[allow(clippy::too_many_arguments)]
pub fn bind_retained_source<'a>(
    item: &'a SourceItem,
    target: ItemRef,
    resource: &ResourceReference,
    _authorized_font: &ExactFont<'_>,
    original_origin: [i32; 2],
    requested_output_format: PixelFormat,
    control: &Interrupt<'_>,
) -> SourceResult<Request<'a>> {
    poll(control)?;
    if requested_output_format != PixelFormat::RGBA8 {
        return Err(invalid());
    }
    let EditableSource::Text { content, markup, font, direction, source_modified, configuration, current_origin } = &item.source else { return Err(invalid()) };
    if item.owner != target
        || !matches!(target, ItemRef::Layer(_))
        || item.transform.is_some()
        || *source_modified
        || markup.is_some()
        || *direction != 0
        || current_origin.is_some()
        || resource.id != *font
        || resource.kind != ResourceKind::Font
        || resource.name != "Inter Regular"
        || resource.collection.is_some()
        || resource.internal
        || resource.owned_bytes.is_some()
        || resource.content_hash.is_some()
        || resource.resolution.is_some()
        || !matches!(content.as_str(), "II\nI" | "I\nII")
    {
        return Err(invalid());
    }
    names(configuration, &["gimp-text-configuration", "source-flags", "parasite-flags", "layout", "font-selectors", "runs", "styles"])?;
    if !uint(field(configuration, "source-flags")?, 32, 0) || !uint(field(configuration, "parasite-flags")?, 32, 1) {
        return Err(invalid());
    }
    let layout = config(field(configuration, "layout")?)?;
    names(
        layout,
        &[
            "font-size",
            "font-size-unit",
            "language",
            "base-direction",
            "justification",
            "box-mode",
            "box-width",
            "box-height",
            "box-unit",
            "antialias",
            "hinting",
        ],
    )?;
    for (key, value) in [("font-size-unit", "pixels"), ("language", "C"), ("base-direction", "ltr"), ("box-mode", "fixed"), ("box-unit", "pixels")] {
        if !string(field(layout, key)?, value) {
            return Err(invalid());
        }
    }
    for (key, value) in [("font-size", 32f64), ("box-width", 96f64), ("box-height", 80f64)] {
        if !matches!(field(layout,key)?,FilterValue::Number(ExactNumber::Float64Bits(bits)) if *bits==value.to_bits()) {
            return Err(invalid());
        }
    }
    if field(layout, "antialias")? != &FilterValue::Bool(false) || field(layout, "hinting")? != &FilterValue::Bool(false) {
        return Err(invalid());
    }
    let (alignment, justify) = match field(layout, "justification")? {
        FilterValue::String(s) if s == "left" => (Alignment::Left, "left"),
        FilterValue::String(s) if s == "center" => (Alignment::Center, "center"),
        FilterValue::String(s) if s == "right" => (Alignment::Right, "right"),
        _ => return Err(invalid()),
    };
    let selectors = config(field(configuration, "font-selectors")?)?;
    names(selectors, &["name", "family", "style", "postscript-name", "requested-hash", "index", "version", "weight", "slant", "width"])?;
    for (key, value) in [
        ("name", "Inter Regular"),
        ("family", "Inter"),
        ("style", "Regular"),
        ("postscript-name", "Inter-Regular"),
        ("requested-hash", "40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"),
    ] {
        if !string(field(selectors, key)?, value) {
            return Err(invalid());
        }
    }
    if !uint(field(selectors, "index")?, 32, 0) || !uint(field(selectors, "version")?, 32, 262210) {
        return Err(invalid());
    }
    for (key, value) in [("weight", 80), ("slant", 0), ("width", 100)] {
        if !signed(field(selectors, key)?, value) {
            return Err(invalid());
        }
    }
    let runs = config(field(configuration, "runs")?)?;
    names(runs, &["run-0"])?;
    let run = config(field(runs, "run-0")?)?;
    names(run, &["start", "end", "styles"])?;
    if !uint(field(run, "start")?, 64, 0)
        || !uint(field(run, "end")?, 64, u64::try_from(content.len()).map_err(|_| TextSourceError::Budget)?)
        || !config(field(run, "styles")?)?.is_empty()
        || !config(field(configuration, "styles")?)?.is_empty()
    {
        return Err(invalid());
    }
    let raw = Ast(config(field(configuration, "gimp-text-configuration")?)?);
    if raw.0.len() != 26 {
        return Err(invalid());
    }
    let text = raw.list(0)?;
    if text.0.len() != 2 {
        return Err(invalid());
    }
    text.atom(0, "text")?;
    text.quoted(1, content.as_bytes())?;
    let f = raw.list(1)?;
    if f.0.len() != 12 {
        return Err(invalid());
    }
    f.atom(0, "font")?;
    f.quoted(1, b"GimpFont")?;
    for (index, key, value) in [
        (2, "fonthash", "40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"),
        (3, "fullname", "Inter Regular"),
        (4, "family", "Inter"),
        (5, "style", "Regular"),
        (6, "psname", "Inter-Regular"),
    ] {
        let n = f.list(index)?;
        if n.0.len() != 2 {
            return Err(invalid());
        }
        n.atom(0, key)?;
        n.quoted(1, value.as_bytes())?;
    }
    for (index, key, value) in [(7, "index", 0.), (8, "weight", 80.), (9, "slant", 0.), (10, "width", 100.), (11, "fontversion", 262210.)] {
        f.list(index)?.scalar_number(key, value)?;
    }
    for (index, key, value) in [
        (2, "font-size", 32.),
        (11, "box-width", 96.),
        (12, "box-height", 80.),
        (21, "outline-miter-limit", 2.),
        (23, "outline-dash-offset", 0.),
        (24, "outline-dash-info", 0.),
    ] {
        raw.list(index)?.scalar_number(key, value)?;
    }
    for (index, key, value) in [
        (3, "font-size-unit", "pixels"),
        (4, "antialias", "no"),
        (5, "hint-style", "none"),
        (7, "base-direction", "ltr"),
        (9, "justify", justify),
        (10, "box-mode", "fixed"),
        (13, "outline-direction", "outer"),
        (14, "box-unit", "pixels"),
        (15, "outline-custom-style", "solid-color"),
        (17, "outline-pattern", "NULL"),
        (18, "outline-unit", "pixels"),
        (19, "outline-cap-style", "butt"),
        (20, "outline-join-style", "miter"),
        (22, "outline-antialias", "yes"),
        (25, "hinting", "no"),
    ] {
        raw.list(index)?.scalar_atom(key, value)?;
    }
    let language = raw.list(6)?;
    if language.0.len() != 2 {
        return Err(invalid());
    }
    language.atom(0, "language")?;
    language.quoted(1, b"C")?;
    for (index, key, bytes) in [
        (8, "color", &[0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63][..]),
        (16, "outline-foreground", &[115, 128, 0, 63, 115, 128, 0, 63, 115, 128, 0, 63, 0, 0, 128, 63][..]),
    ] {
        let n = raw.list(index)?;
        if n.0.len() != 2 {
            return Err(invalid());
        }
        n.atom(0, key)?;
        color(n.list(1)?, bytes)?;
    }
    let request = Request { requested_output_format, content, origin: original_origin, alignment };
    requirements(request, Limits::default())?;
    poll(control)?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_envelope_is_closed_and_lower_only() {
        let valid = Request { requested_output_format: PixelFormat::RGBA8, content: "II\nI", origin: [8, 4], alignment: Alignment::Center };
        assert!(requirements(valid, Limits::default()).is_ok());
        for format in [PixelFormat::RGBA16, PixelFormat::RGBA32, PixelFormat::RGBA32F, PixelFormat::RGBA64F] {
            assert!(requirements(Request { requested_output_format: format, ..valid }, Limits::default()).is_err());
        }
        assert!(requirements(Request { requested_output_format: PixelFormat::RGBA8, content: "III\nI", ..valid }, Limits::default()).is_err());
        assert!(requirements(valid, Limits { resident_bytes: 1, ..Limits::default() }).is_err());
        assert!(requirements(valid, Limits { resident_bytes: (8 << 20) + 1, ..Limits::default() }).is_err());
    }
    #[test]
    fn measured_rows_use_real_outline_candidate() {
        let control = Interrupt::NONE;
        let font = ExactFont::bundled_inter(&control).expect("authorized existing Inter");
        for (alignment, first, second) in [(Alignment::Left, 3, 3), (Alignment::Center, 42, 47), (Alignment::Right, 81, 90)] {
            let rendered = render_with_charge(
                &font,
                Request { requested_output_format: PixelFormat::RGBA8, content: "II\nI", origin: [0, 0], alignment },
                Limits::default(),
                &control,
                &mut |_| Ok(()),
            )
            .expect("candidate");
            let mut row = [0u8; 384];
            for (y, x, count) in [(8, first, 6), (47, second, 3)] {
                rendered.surface.try_read_interleaved_into_with(Rect::new(0, y, 96, y + 1), &mut row, &control).expect("row");
                let xs: Vec<_> = row.as_chunks::<4>().0.iter().enumerate().filter_map(|(x, p)| (p[3] != 0).then_some(x)).collect();
                assert_eq!(xs.first(), Some(&x));
                assert_eq!(xs.len(), count);
            }
        }
    }
}
