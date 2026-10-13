//! Canonical stored-source semantics for the independently observed stem domain.
use super::{RectilinearAntialias, StemText, TextSourceError, poll};
use photocraft_doc::{
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
    xcf_items::{EditableSource, SourceItem},
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_raster::Interrupt;
type Result<T> = std::result::Result<T, TextSourceError>;
fn invalid() -> TextSourceError {
    TextSourceError::Unsupported("unqualified canonical text configuration")
}
fn field<'a>(fields: &'a [FilterArgument], name: &str) -> Result<&'a FilterValue> {
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
fn config(value: &FilterValue) -> Result<&[FilterArgument]> {
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
fn names(fields: &[FilterArgument], expected: &[&str]) -> Result<()> {
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
    fn value(&self, index: usize, kind: &str) -> Result<&'a FilterValue> {
        let node = self.0.get(index).ok_or_else(invalid)?;
        let (prefix, suffix) = node.name.rsplit_once('-').ok_or_else(invalid)?;
        if prefix != kind || suffix.parse::<usize>().ok() != Some(index) {
            return Err(invalid());
        }
        Ok(&node.value)
    }
    fn atom(&self, index: usize, value: &str) -> Result<()> {
        if string(self.value(index, "atom")?, value) { Ok(()) } else { Err(invalid()) }
    }
    fn quoted(&self, index: usize, value: &[u8]) -> Result<()> {
        if let FilterValue::String(v) = self.value(index, "quoted-bytes-hex")?
            && hex_matches(v, value)
        {
            return Ok(());
        }
        Err(invalid())
    }
    fn number(&self, index: usize, expected: f64) -> Result<()> {
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
    fn list(&self, index: usize) -> Result<Ast<'a>> {
        Ok(Ast(config(self.value(index, "list")?)?))
    }
    fn scalar_atom(&self, key: &str, value: &str) -> Result<()> {
        if self.0.len() != 2 {
            return Err(invalid());
        }
        self.atom(0, key)?;
        self.atom(1, value)
    }
    fn scalar_number(&self, key: &str, value: f64) -> Result<()> {
        if self.0.len() != 2 {
            return Err(invalid());
        }
        self.atom(0, key)?;
        self.number(1, value)
    }
}
fn color(ast: Ast<'_>, rgb: &[u8]) -> Result<()> {
    if ast.0.len() != 5 {
        return Err(invalid());
    }
    ast.atom(0, "color")?;
    ast.quoted(1, b"R'G'B'A float")?;
    ast.number(2, 16.)?;
    ast.quoted(3, rgb)?;
    ast.number(4, 0.)
}

/// Serialized sample encoding is independent of the current working profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColorEncoding {
    Linear,
    Nonlinear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextColor {
    pub encoding: TextColorEncoding,
    pub rgba_f32_bits: [u32; 4],
}
impl TextColor {
    pub fn rgba(self) -> [f32; 4] {
        self.rgba_f32_bits.map(f32::from_bits)
    }
    pub fn checked(encoding: TextColorEncoding, rgba_f32_bits: [u32; 4]) -> Result<Self> {
        let color = Self { encoding, rgba_f32_bits };
        if color.rgba().iter().any(|v| !v.is_finite() || !(0. ..=1.).contains(v)) {
            return Err(invalid());
        }
        Ok(color)
    }
    fn format(self) -> &'static [u8] {
        match self.encoding {
            TextColorEncoding::Linear => b"RGBA float",
            TextColorEncoding::Nonlinear => b"R'G'B'A float",
        }
    }
}
fn source_color(item: &SourceItem) -> Result<TextColor> {
    let EditableSource::Text { configuration, .. } = &item.source else { return Err(invalid()) };
    let tree = Ast(config(field(configuration, "gimp-text-configuration")?)?);
    let node = tree.list(8)?;
    if node.0.len() != 2 {
        return Err(invalid());
    }
    node.atom(0, "color")?;
    let value = node.list(1)?;
    if value.0.len() != 5 {
        return Err(invalid());
    }
    value.atom(0, "color")?;
    let encoding = if value.quoted(1, b"RGBA float").is_ok() {
        TextColorEncoding::Linear
    } else {
        value.quoted(1, b"R'G'B'A float")?;
        TextColorEncoding::Nonlinear
    };
    value.number(2, 16.)?;
    value.number(4, 0.)?; // Embedded profiles are not silently discarded.
    let FilterValue::String(hex) = value.value(3, "quoted-bytes-hex")? else { return Err(invalid()) };
    if hex.len() != 32 {
        return Err(invalid());
    }
    let mut bytes = [0_u8; 16];
    for (slot, pair) in bytes.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0.iter()) {
        let digit = |v: u8| match v {
            b'0'..=b'9' => Some(v - b'0'),
            b'a'..=b'f' => Some(v - b'a' + 10),
            _ => None,
        };
        *slot = digit(*pair.first().ok_or_else(invalid)?).ok_or_else(invalid)? * 16 + digit(*pair.get(1).ok_or_else(invalid)?).ok_or_else(invalid)?;
    }
    let mut bits = [0_u32; 4];
    for (slot, chunk) in bits.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
        *slot = u32::from_le_bytes(*chunk);
    }
    TextColor::checked(encoding, bits)
}

/// Qualifies retained semantics before render. `origin` is caller-authoritative
/// placement, not inferred from cache pixels. The returned request must still
/// use explicit validated font authority and the appropriate document profile.
pub fn qualify_stem_source<'a>(item: &'a SourceItem, resource: &ResourceReference, origin: [i32; 2], ctl: &Interrupt<'_>) -> Result<StemText<'a>> {
    qualify_source(item, resource, origin, ctl, false, false, 32., None, None, false, None)
}
/// Qualifies stored layout/font/run semantics; the returned content must also
/// pass exact-font `prepare_rectilinear_with_charge` geometry qualification.
pub fn qualify_rectilinear_source<'a>(item: &'a SourceItem, resource: &ResourceReference, origin: [i32; 2], ctl: &Interrupt<'_>) -> Result<StemText<'a>> {
    qualify_source(item, resource, origin, ctl, true, false, 32., None, None, false, None)
}
/// Preserves the declared antialias mode independently of current content.
/// Every other layout/font/run constraint remains the existing scoped schema.
pub fn qualify_rectilinear_source_with_antialias<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<(StemText<'a>, RectilinearAntialias)> {
    poll(ctl)?;
    let EditableSource::Text { configuration, .. } = &item.source else { return Err(invalid()) };
    let layout = config(field(configuration, "layout")?)?;
    let FilterValue::Bool(aa) = field(layout, "antialias")? else { return Err(invalid()) };
    let request = qualify_source(item, resource, origin, ctl, true, *aa, 32., None, None, false, None)?;
    Ok((request, if *aa { RectilinearAntialias::AreaCoverage } else { RectilinearAntialias::None }))
}
// Separate explicit source semantics, placement, mode and cancellation avoid
// implicit defaults at the format boundary.
#[allow(clippy::too_many_arguments)]
fn qualify_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
    rectilinear: bool,
    antialias: bool,
    size_px: f64,
    declared_color: Option<TextColor>,
    declared_spacing: Option<f64>,
    allow_current_origin: bool,
    multiline_spacing: Option<Option<f64>>,
) -> Result<StemText<'a>> {
    qualify_source_inner(
        item,
        resource,
        origin,
        ctl,
        rectilinear,
        antialias,
        size_px,
        declared_color,
        declared_spacing,
        allow_current_origin,
        multiline_spacing,
        None,
    )
}
#[allow(clippy::too_many_arguments)]
fn qualify_source_inner<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
    rectilinear: bool,
    antialias: bool,
    size_px: f64,
    declared_color: Option<TextColor>,
    declared_spacing: Option<f64>,
    allow_current_origin: bool,
    multiline_spacing: Option<Option<f64>>,
    underline: Option<&super::underline::UnderlineRuns>,
) -> Result<StemText<'a>> {
    poll(ctl)?;
    let EditableSource::Text { content, markup, font, direction, source_modified, current_origin, configuration } = &item.source else {
        return Err(invalid());
    };
    if (current_origin.is_some() && !allow_current_origin)
        || current_origin.is_some_and(|declared| declared != origin)
        || origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v))
        || *source_modified
        || (markup.is_some() != underline.is_some())
        || *direction != 0
        || item.transform.is_some()
        || resource.id != *font
        || resource.kind != ResourceKind::Font
        || resource.name != "Inter Regular"
        || resource.collection.is_some()
        || resource.internal
        || resource.owned_bytes.is_some()
        || resource.content_hash.is_some()
    {
        return Err(invalid());
    }
    if multiline_spacing.is_some() {
        let (first, second) = content.split_once('\n').ok_or_else(invalid)?;
        if first.is_empty()
            || second.is_empty()
            || content.len() > 9
            || first.bytes().chain(second.bytes()).any(|c| !(32..=126).contains(&c))
            || declared_spacing.is_some()
        {
            return Err(invalid());
        }
    } else if !(1..=8).contains(&content.len()) || content.bytes().any(|c| if rectilinear { !(32..=126).contains(&c) } else { c != b'I' }) {
        return Err(invalid());
    }
    names(configuration, &["gimp-text-configuration", "source-flags", "parasite-flags", "layout", "font-selectors", "runs", "styles"])?;
    if !(uint(field(configuration, "source-flags")?, 32, 0) || underline.is_some() && uint(field(configuration, "source-flags")?, 32, 1))
        || !uint(field(configuration, "parasite-flags")?, 32, 1)
    {
        return Err(invalid());
    }
    let layout = config(field(configuration, "layout")?)?;
    if let Some(spacing) = multiline_spacing.flatten() {
        names(
            layout,
            &["font-size", "font-size-unit", "language", "base-direction", "justification", "box-mode", "box-unit", "antialias", "hinting", "line-spacing"],
        )?;
        if !matches!(field(layout,"line-spacing")?,FilterValue::Number(ExactNumber::Float64Bits(v)) if *v==spacing.to_bits()) {
            return Err(invalid());
        }
    } else if declared_spacing.is_some() {
        names(
            layout,
            &["font-size", "font-size-unit", "language", "base-direction", "justification", "box-mode", "box-unit", "antialias", "hinting", "letter-spacing"],
        )?;
        if !matches!(field(layout,"letter-spacing")?,FilterValue::Number(ExactNumber::Float64Bits(v)) if declared_spacing.is_some_and(|s| *v==s.to_bits())) {
            return Err(invalid());
        }
    } else {
        names(layout, &["font-size", "font-size-unit", "language", "base-direction", "justification", "box-mode", "box-unit", "antialias", "hinting"])?;
    }
    if !matches!(field(layout,"font-size")?,FilterValue::Number(ExactNumber::Float64Bits(v)) if *v==size_px.to_bits()) {
        return Err(invalid());
    }
    for (key, value) in
        [("font-size-unit", "pixels"), ("language", "C"), ("base-direction", "ltr"), ("justification", "left"), ("box-mode", "dynamic"), ("box-unit", "pixels")]
    {
        if !string(field(layout, key)?, value) {
            return Err(invalid());
        }
    }
    if field(layout, "antialias")? != &FilterValue::Bool(antialias) || field(layout, "hinting")? != &FilterValue::Bool(false) {
        return Err(invalid());
    }
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
    if let Some(plan) = underline {
        verify_underline_configuration(configuration, plan)?;
    } else {
        let runs = config(field(configuration, "runs")?)?;
        names(runs, &["run-0"])?;
        let run = config(field(runs, "run-0")?)?;
        names(run, &["start", "end", "styles"])?;
        if !uint(field(run, "start")?, 64, 0)
            || !uint(field(run, "end")?, 64, content.len() as u64)
            || !config(field(run, "styles")?)?.is_empty()
            || !config(field(configuration, "styles")?)?.is_empty()
        {
            return Err(invalid());
        }
    }
    let raw = Ast(config(field(configuration, "gimp-text-configuration")?)?);
    let extra = declared_spacing.is_some() || multiline_spacing.flatten().is_some();
    let shifted = |index: usize| index + usize::from(extra && index >= 10);
    if raw.0.len() != 24 + usize::from(extra) {
        return Err(invalid());
    }
    if let Some(spacing) = multiline_spacing.flatten() {
        raw.list(10)?.scalar_number("line-spacing", spacing)?;
    } else if let Some(spacing) = declared_spacing {
        raw.list(10)?.scalar_number("letter-spacing", spacing)?;
    }
    let text = raw.list(0)?;
    if text.0.len() != 2 {
        return Err(invalid());
    }
    if underline.is_some() {
        text.atom(0, "markup")?;
        text.quoted(1, markup.as_deref().ok_or_else(invalid)?.as_bytes())?;
    } else {
        text.atom(0, "text")?;
        text.quoted(1, content.as_bytes())?;
    }
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
    raw.list(2)?.scalar_number("font-size", size_px)?;
    for (index, key, value) in [
        (3, "font-size-unit", "pixels"),
        (4, "antialias", if antialias { "yes" } else { "no" }),
        (5, "hint-style", "none"),
        (7, "base-direction", "ltr"),
        (9, "justify", "left"),
        (10, "box-mode", "dynamic"),
        (11, "outline-direction", "outer"),
        (12, "box-unit", "pixels"),
        (13, "outline-custom-style", "solid-color"),
        (15, "outline-pattern", "NULL"),
        (16, "outline-unit", "pixels"),
        (17, "outline-cap-style", "butt"),
        (18, "outline-join-style", "miter"),
        (20, "outline-antialias", "yes"),
        (23, "hinting", "no"),
    ] {
        raw.list(shifted(index))?.scalar_atom(key, value)?;
    }
    let language = raw.list(6)?;
    if language.0.len() != 2 {
        return Err(invalid());
    }
    language.atom(0, "language")?;
    language.quoted(1, b"C")?;
    for (index, key, bytes) in [
        (8, "color", &[0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63][..]),
        (14, "outline-foreground", &[115, 128, 0, 63, 115, 128, 0, 63, 115, 128, 0, 63, 0, 0, 128, 63][..]),
    ] {
        let n = raw.list(shifted(index))?;
        if n.0.len() != 2 {
            return Err(invalid());
        }
        n.atom(0, key)?;
        if index == 8 {
            if let Some(declared) = declared_color {
                let parsed = source_color(item)?;
                if parsed != declared {
                    return Err(invalid());
                }
            } else {
                color(n.list(1)?, bytes)?;
            }
        } else {
            color(n.list(1)?, bytes)?;
        }
    }
    for (index, key, value) in [(19, "outline-miter-limit", 2.), (21, "outline-dash-offset", 0.), (22, "outline-dash-info", 0.)] {
        raw.list(shifted(index))?.scalar_number(key, value)?;
    }
    // No outline activation tag, transforms, or optional required fields.
    poll(ctl)?;
    Ok(StemText { content, origin })
}
/// Explicit AA quadratic/multicontour opt-in. Stored paragraph/font semantics
/// remain strict; generic outline and actual shaping checks are separate.
pub fn qualify_outline_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    original_origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<super::ScaledText<'a>> {
    let EditableSource::Text { configuration, current_origin, .. } = &item.source else { return Err(invalid()) };
    let layout = config(field(configuration, "layout")?)?;
    let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field(layout, "font-size")? else { return Err(invalid()) };
    let size = f64::from_bits(*bits);
    if !matches!(size, 16. | 32. | 40.) {
        return Err(invalid());
    }
    let origin = current_origin.unwrap_or(original_origin);
    let request = qualify_source(item, resource, origin, ctl, true, true, size, None, None, true, None)?;
    Ok(super::ScaledText { content: request.content, origin: request.origin, size_px: size as f32 })
}
/// Scaled AA has independent source controls; no-AA remains the original
/// 32-pixel domain because scaled center-boundary behavior is unqualified.
pub fn qualify_scaled_rectilinear_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<(super::ScaledText<'a>, RectilinearAntialias)> {
    qualify_scaled_impl(item, resource, origin, ctl, None, None, false)
}
pub fn qualify_colored_rectilinear_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<(super::ScaledText<'a>, RectilinearAntialias, TextColor)> {
    poll(ctl)?;
    let color = source_color(item)?;
    let (request, mode) = qualify_scaled_impl(item, resource, origin, ctl, Some(color), None, false)?;
    Ok((request, mode, color))
}
/// Additive operative-spacing qualification; never discard spacing in a
/// zero-spacing renderer. The caller must thread all four returned values.
pub fn qualify_spaced_colored_rectilinear_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<(super::ScaledText<'a>, RectilinearAntialias, TextColor, f32)> {
    qualify_spaced_impl(item, resource, origin, ctl, false)
}
/// Operative current placement is a PhotoCraft edit extension. Original source
/// AST, raster bounds and cache remain separate. Legacy qualifiers reject it.
pub fn qualify_placed_spaced_colored_rectilinear_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    original_origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<(super::ScaledText<'a>, RectilinearAntialias, TextColor, f32)> {
    qualify_spaced_impl(item, resource, original_origin, ctl, true)
}
/// Explicit operative line layout; additional spacing is never a substitute
/// for the font's checked line metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RectilinearLineLayout {
    SingleLine { letter_spacing_px: f32 },
    TwoLines { line_spacing_px: f32 },
}
#[derive(Clone, Copy)]
pub struct QualifiedRectilinearText<'a> {
    pub request: super::ScaledText<'a>,
    pub antialias: RectilinearAntialias,
    pub color: TextColor,
    pub layout: RectilinearLineLayout,
}
/// Additive layout opt-in. Legacy source qualifiers remain single-line only.
pub fn qualify_placed_layout_colored_rectilinear_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    original_origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<QualifiedRectilinearText<'a>> {
    poll(ctl)?;
    let EditableSource::Text { content, configuration, current_origin, .. } = &item.source else { return Err(invalid()) };
    if !content.contains('\n') {
        let (request, antialias, color, spacing) = qualify_placed_spaced_colored_rectilinear_source(item, resource, original_origin, ctl)?;
        return Ok(QualifiedRectilinearText { request, antialias, color, layout: RectilinearLineLayout::SingleLine { letter_spacing_px: spacing } });
    }
    if original_origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v)) {
        return Err(invalid());
    }
    let origin = current_origin.unwrap_or(original_origin);
    let layout = config(field(configuration, "layout")?)?;
    let declared_spacing = if layout.iter().any(|f| f.name == "line-spacing") {
        let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field(layout, "line-spacing")? else { return Err(invalid()) };
        let spacing = f64::from_bits(*bits);
        if !matches!(spacing, 2. | 4.) {
            return Err(invalid());
        }
        Some(spacing)
    } else {
        None
    };
    let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field(layout, "font-size")? else { return Err(invalid()) };
    let size_px = f64::from_bits(*bits);
    let FilterValue::Bool(aa) = field(layout, "antialias")? else { return Err(invalid()) };
    let color = source_color(item)?;
    if !matches!(size_px, 16. | 32. | 40.)
        || (!aa && size_px != 32.)
        || color.encoding != TextColorEncoding::Nonlinear
        || color.rgba_f32_bits != [1_f32.to_bits(); 4]
    {
        return Err(invalid());
    }
    let request = qualify_source(item, resource, origin, ctl, true, *aa, size_px, Some(color), None, true, Some(declared_spacing))?;
    Ok(QualifiedRectilinearText {
        request: super::ScaledText { content: request.content, origin, size_px: size_px as f32 },
        antialias: if *aa { RectilinearAntialias::AreaCoverage } else { RectilinearAntialias::None },
        color,
        layout: RectilinearLineLayout::TwoLines { line_spacing_px: declared_spacing.unwrap_or(0.) as f32 },
    })
}
pub fn prepare_qualified_rectilinear_with_charge(
    font: &super::ExactFont<'_>,
    source: QualifiedRectilinearText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<super::PreparedRectilinear> {
    match source.layout {
        RectilinearLineLayout::SingleLine { letter_spacing_px } => {
            super::prepare_spaced_rectilinear_with_charge(font, source.request, letter_spacing_px, max_resident, ctl, charge)
        }
        RectilinearLineLayout::TwoLines { line_spacing_px } => {
            super::prepare_multiline_rectilinear_with_charge(font, source.request, line_spacing_px, max_resident, ctl, charge)
        }
    }
}
pub fn qualify_layout_rectilinear_geometry(font: &super::ExactFont<'_>, source: QualifiedRectilinearText<'_>, ctl: &Interrupt<'_>) -> Result<()> {
    match source.layout {
        RectilinearLineLayout::SingleLine { .. } => super::qualify_scaled_rectilinear_geometry(font, source.request, ctl),
        RectilinearLineLayout::TwoLines { line_spacing_px } => super::qualify_multiline_rectilinear_geometry(font, source.request, line_spacing_px, ctl),
    }
}
fn qualify_spaced_impl<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    original_origin: [i32; 2],
    ctl: &Interrupt<'_>,
    allow_current_origin: bool,
) -> Result<(super::ScaledText<'a>, RectilinearAntialias, TextColor, f32)> {
    poll(ctl)?;
    let EditableSource::Text { configuration, current_origin, .. } = &item.source else { return Err(invalid()) };
    if original_origin.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v)) {
        return Err(invalid());
    }
    let origin = current_origin.unwrap_or(original_origin);
    let layout = config(field(configuration, "layout")?)?;
    if layout.len() > 10 {
        return Err(invalid());
    }
    let declared_spacing = if layout.iter().any(|f| f.name == "letter-spacing") {
        let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field(layout, "letter-spacing")? else { return Err(invalid()) };
        let spacing = f64::from_bits(*bits);
        if !matches!(spacing, 2. | 4.) {
            return Err(invalid());
        }
        Some(spacing)
    } else {
        None
    };
    let color = source_color(item)?;
    let (request, mode) = qualify_scaled_impl(item, resource, origin, ctl, Some(color), declared_spacing, allow_current_origin)?;
    if declared_spacing.is_some() && !matches!(request.size_px, 16. | 32. | 40.) {
        return Err(invalid());
    }
    Ok((request, mode, color, declared_spacing.unwrap_or(0.) as f32))
}
// Exact color, spacing and placement opt-ins remain independent at the boundary.
#[allow(clippy::too_many_arguments)]
fn qualify_scaled_impl<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
    declared_color: Option<TextColor>,
    declared_spacing: Option<f64>,
    allow_current_origin: bool,
) -> Result<(super::ScaledText<'a>, RectilinearAntialias)> {
    poll(ctl)?;
    let EditableSource::Text { configuration, .. } = &item.source else { return Err(invalid()) };
    let layout = config(field(configuration, "layout")?)?;
    let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field(layout, "font-size")? else { return Err(invalid()) };
    let size_px = f64::from_bits(*bits);
    let FilterValue::Bool(aa) = field(layout, "antialias")? else { return Err(invalid()) };
    if !matches!(size_px, 16. | 24. | 32. | 40.) || (!aa && size_px != 32.) {
        return Err(invalid());
    }
    let request = qualify_source(item, resource, origin, ctl, true, *aa, size_px, declared_color, declared_spacing, allow_current_origin, None)?;
    Ok((
        super::ScaledText { content: request.content, origin, size_px: size_px as f32 },
        if *aa { RectilinearAntialias::AreaCoverage } else { RectilinearAntialias::None },
    ))
}

/// Atomic semantic color edit; encoding and numeric bits stay explicit. The
/// caller must convert color through the active profile before rendering.
// Authority, placement, budget and cancellation are independently supplied.
#[allow(clippy::too_many_arguments)]
pub fn set_rectilinear_color(
    item: &mut SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    replacement: TextColor,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let replacement = TextColor::checked(replacement.encoding, replacement.rgba_f32_bits)?;
    let (request, _, current, spacing) = qualify_placed_spaced_colored_rectilinear_source(item, resource, origin, ctl)?;
    super::prepare_spaced_rectilinear_with_charge(font, request, spacing, max_resident, ctl, charge)?;
    if current == replacement {
        return Ok(false);
    }
    charge(64)?;
    let mut format = String::new();
    format.try_reserve_exact(replacement.format().len() * 2).map_err(|_| TextSourceError::Allocation)?;
    let mut samples = String::new();
    samples.try_reserve_exact(32).map_err(|_| TextSourceError::Allocation)?;
    use std::fmt::Write;
    for b in replacement.format() {
        write!(&mut format, "{b:02x}").map_err(|_| invalid())?;
    }
    for bits in replacement.rgba_f32_bits {
        for b in bits.to_le_bytes() {
            write!(&mut samples, "{b:02x}").map_err(|_| invalid())?;
        }
    }
    let EditableSource::Text { configuration, .. } = &mut item.source else {
        return Err(invalid());
    };
    let tree = configuration.iter_mut().find(|n| n.name == "gimp-text-configuration").ok_or_else(invalid)?;
    let FilterValue::Config(tree) = &mut tree.value else {
        return Err(invalid());
    };
    let node = config_mut(tree, 8)?;
    let color = config_mut(node, 1)?;
    let [format_node, sample_node] = color.get_disjoint_mut([1, 3]).map_err(|_| invalid())?;
    poll(ctl)?;
    format_node.value = FilterValue::String(format);
    sample_node.value = FilterValue::String(samples);
    Ok(true)
}

/// Current content edit preserves the original declared scale and mode.
pub fn set_scaled_rectilinear_content(
    item: &mut SourceItem,
    resource: &ResourceReference,
    replacement: super::ScaledText<'_>,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let (current, _, _, spacing) = qualify_placed_spaced_colored_rectilinear_source(item, resource, replacement.origin, ctl)?;
    let replacement = super::ScaledText { origin: current.origin, ..replacement };
    if current.size_px != replacement.size_px {
        return Err(invalid());
    }
    super::prepare_spaced_rectilinear_with_charge(font, current, spacing, max_resident, ctl, charge)?;
    super::prepare_spaced_rectilinear_with_charge(font, replacement, spacing, max_resident, ctl, charge)?;
    update_content(item, replacement.content, ctl)
}
/// Atomic typed scale edit. Original cache, backing raster and resource identity
/// remain immutable. Content and mode cannot be changed through this operation.
pub fn set_rectilinear_font_size(
    item: &mut SourceItem,
    resource: &ResourceReference,
    replacement: super::ScaledText<'_>,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let (current, mode, _, spacing) = qualify_placed_spaced_colored_rectilinear_source(item, resource, replacement.origin, ctl)?;
    let replacement = super::ScaledText { origin: current.origin, ..replacement };
    if mode != RectilinearAntialias::AreaCoverage || current.content != replacement.content {
        return Err(invalid());
    }
    let previous = current.size_px;
    super::prepare_spaced_rectilinear_with_charge(font, current, spacing, max_resident, ctl, charge)?;
    super::prepare_spaced_rectilinear_with_charge(font, replacement, spacing, max_resident, ctl, charge)?;
    if previous == replacement.size_px {
        return Ok(false);
    }
    let literal = match replacement.size_px {
        16. => "16",
        24. => "24",
        32. => "32",
        40. => "40",
        _ => return Err(invalid()),
    };
    let mut decimal = String::new();
    decimal.try_reserve_exact(literal.len()).map_err(|_| TextSourceError::Allocation)?;
    decimal.push_str(literal);
    let EditableSource::Text { configuration, .. } = &mut item.source else { return Err(invalid()) };
    let tree_index = configuration.iter().position(|a| a.name == "gimp-text-configuration").ok_or_else(invalid)?;
    let layout_index = configuration.iter().position(|a| a.name == "layout").ok_or_else(invalid)?;
    let [tree_node, layout_node] = configuration.get_disjoint_mut([tree_index, layout_index]).map_err(|_| invalid())?;
    let FilterValue::Config(tree) = &mut tree_node.value else { return Err(invalid()) };
    let size = config_mut(tree, 2)?;
    let number = config_mut(size, 1)?;
    let decimal_slot = number.iter().position(|a| a.name == "decimal").ok_or_else(invalid)?;
    let bits_slot = number.iter().position(|a| a.name == "f64-bits").ok_or_else(invalid)?;
    let [decimal_node, bits_node] = number.get_disjoint_mut([decimal_slot, bits_slot]).map_err(|_| invalid())?;
    let FilterValue::Config(layout) = &mut layout_node.value else { return Err(invalid()) };
    let layout_size = layout.iter_mut().find(|a| a.name == "font-size").ok_or_else(invalid)?;
    poll(ctl)?;
    let value = FilterValue::Number(ExactNumber::Float64Bits(f64::from(replacement.size_px).to_bits()));
    decimal_node.value = FilterValue::String(decimal);
    bits_node.value = value.clone();
    layout_size.value = value;
    Ok(true)
}

/// Atomic spacing edit. Zero is represented by omission, matching stock saved
/// configuration; original cache and opaque source bytes remain immutable.
#[allow(clippy::too_many_arguments)]
pub fn set_rectilinear_letter_spacing(
    item: &mut SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    replacement: f32,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let (current, _, _, spacing) = qualify_placed_spaced_colored_rectilinear_source(item, resource, origin, ctl)?;
    if !matches!(replacement, 0. | 2. | 4.) {
        return Err(invalid());
    }
    super::prepare_spaced_rectilinear_with_charge(font, current, spacing, max_resident, ctl, charge)?;
    super::prepare_spaced_rectilinear_with_charge(font, current, replacement, max_resident, ctl, charge)?;
    if spacing == replacement {
        return Ok(false);
    }
    update_optional_spacing(item, "letter-spacing", spacing, replacement, ctl, charge)
}
fn update_optional_spacing(
    item: &mut SourceItem,
    key: &str,
    spacing: f32,
    replacement: f32,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    // Construct the complete bounded delta before resolving its target slots.
    // Every potential allocation and callback failure precedes semantic mutation.
    let (raw, typed) = if replacement == 0. {
        (None, None)
    } else {
        let mut number = spacing_fields(2, charge)?;
        number.push(spacing_arg("decimal", FilterValue::String(spacing_string(if replacement == 2. { "2" } else { "4" }, charge)?), charge)?);
        number.push(spacing_arg("f64-bits", FilterValue::Number(ExactNumber::Float64Bits(f64::from(replacement).to_bits())), charge)?);
        let mut list = spacing_fields(2, charge)?;
        list.push(spacing_arg("atom-0", FilterValue::String(spacing_string(key, charge)?), charge)?);
        list.push(spacing_arg("number-1", FilterValue::Config(number), charge)?);
        (
            Some(spacing_arg("list-10", FilterValue::Config(list), charge)?),
            Some(spacing_arg(key, FilterValue::Number(ExactNumber::Float64Bits(f64::from(replacement).to_bits())), charge)?),
        )
    };
    let target_len = if replacement == 0. { 24 } else { 25 };
    charge((target_len * std::mem::size_of::<String>()) as u64)?;
    let mut names = Vec::new();
    names.try_reserve_exact(target_len).map_err(|_| TextSourceError::Allocation)?;
    for index in 0..target_len {
        poll(ctl)?;
        charge(7)?;
        let mut name = String::new();
        name.try_reserve_exact(7).map_err(|_| TextSourceError::Allocation)?;
        use std::fmt::Write;
        write!(&mut name, "list-{index}").map_err(|_| invalid())?;
        names.push(name);
    }
    let EditableSource::Text { configuration, .. } = &mut item.source else { return Err(invalid()) };
    let tree_index = configuration.iter().position(|a| a.name == "gimp-text-configuration").ok_or_else(invalid)?;
    let layout_index = configuration.iter().position(|a| a.name == "layout").ok_or_else(invalid)?;
    let [tree_node, layout_node] = configuration.get_disjoint_mut([tree_index, layout_index]).map_err(|_| invalid())?;
    let FilterValue::Config(tree) = &mut tree_node.value else { return Err(invalid()) };
    let FilterValue::Config(layout) = &mut layout_node.value else { return Err(invalid()) };
    let expected_len = if spacing == 0. { 24 } else { 25 };
    if tree.len() != expected_len
        || tree.iter().enumerate().any(|(i, n)| n.name.rsplit_once('-').is_none_or(|(kind, index)| kind != "list" || index.parse::<usize>().ok() != Some(i)))
    {
        return Err(invalid());
    }
    let old_layout_index = layout.iter().position(|a| a.name == key);
    if old_layout_index.is_some() != (spacing != 0.) {
        return Err(invalid());
    }
    // Place the optional field after font-size, matching the canonical mapper.
    let insert_layout_index = layout.iter().position(|a| a.name == "font-size").and_then(|i| i.checked_add(1)).ok_or_else(invalid)?;
    let insert_layout_index = insert_layout_index.checked_sub(usize::from(old_layout_index.is_some_and(|i| i < insert_layout_index))).ok_or_else(invalid)?;
    let final_layout_len = layout.len().checked_sub(usize::from(old_layout_index.is_some())).ok_or_else(invalid)?;
    if insert_layout_index > final_layout_len || old_layout_index.is_some_and(|i| layout.get(i).is_none()) || tree.len() <= 10 {
        return Err(invalid());
    }
    if spacing == 0. && replacement != 0. {
        charge(std::mem::size_of::<FilterArgument>() as u64)?;
        tree.try_reserve_exact(1).map_err(|_| TextSourceError::Allocation)?;
        charge(std::mem::size_of::<FilterArgument>() as u64)?;
        layout.try_reserve_exact(1).map_err(|_| TextSourceError::Allocation)?;
    }
    poll(ctl)?;
    // Positions and spare capacity are checked above; nothing fallible follows.
    if spacing != 0. {
        tree.remove(10);
    }
    if let Some(raw) = raw {
        tree.insert(10, raw);
    }
    if let Some(index) = old_layout_index {
        layout.remove(index);
    }
    if let Some(typed) = typed {
        layout.insert(insert_layout_index, typed);
    }
    for (node, name) in tree.iter_mut().zip(names) {
        node.name = name;
    }
    Ok(true)
}
fn spacing_string(value: &str, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<String> {
    charge(value.len() as u64)?;
    let mut out = String::new();
    out.try_reserve_exact(value.len()).map_err(|_| TextSourceError::Allocation)?;
    if out.capacity() > value.len() {
        return Err(TextSourceError::Budget);
    }
    out.push_str(value);
    Ok(out)
}
fn spacing_arg(name: &str, value: FilterValue, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<FilterArgument> {
    Ok(FilterArgument { name: spacing_string(name, charge)?, value })
}
fn spacing_fields(count: usize, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<Vec<FilterArgument>> {
    let bytes = count.checked_mul(std::mem::size_of::<FilterArgument>()).ok_or(TextSourceError::Budget)?;
    charge(bytes as u64)?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| TextSourceError::Allocation)?;
    if out.capacity() > count {
        return Err(TextSourceError::Budget);
    }
    Ok(out)
}

/// Changes current multiline content while keeping exactly two lines and the
/// retained line spacing, font identity, source cache and placement.
#[allow(clippy::too_many_arguments)]
pub fn set_multiline_rectilinear_content(
    item: &mut SourceItem,
    resource: &ResourceReference,
    original_origin: [i32; 2],
    replacement: &str,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let source = qualify_placed_layout_colored_rectilinear_source(item, resource, original_origin, ctl)?;
    let RectilinearLineLayout::TwoLines { line_spacing_px } = source.layout else { return Err(invalid()) };
    prepare_qualified_rectilinear_with_charge(font, source, max_resident, ctl, charge)?;
    super::prepare_multiline_rectilinear_with_charge(
        font,
        super::ScaledText { content: replacement, ..source.request },
        line_spacing_px,
        max_resident,
        ctl,
        charge,
    )?;
    charge(u64::try_from(replacement.len().checked_mul(3).ok_or(TextSourceError::Budget)?).map_err(|_| TextSourceError::Budget)?)?;
    update_content(item, replacement, ctl)
}
/// Zero is exactly the stock absent raw/typed line-spacing field.
#[allow(clippy::too_many_arguments)]
pub fn set_rectilinear_line_spacing(
    item: &mut SourceItem,
    resource: &ResourceReference,
    original_origin: [i32; 2],
    replacement: f32,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let source = qualify_placed_layout_colored_rectilinear_source(item, resource, original_origin, ctl)?;
    let RectilinearLineLayout::TwoLines { line_spacing_px } = source.layout else { return Err(invalid()) };
    prepare_qualified_rectilinear_with_charge(font, source, max_resident, ctl, charge)?;
    super::prepare_multiline_rectilinear_with_charge(font, source.request, replacement, max_resident, ctl, charge)?;
    if line_spacing_px == replacement {
        return Ok(false);
    }
    update_optional_spacing(item, "line-spacing", line_spacing_px, replacement, ctl, charge)
}

/// Atomically changes only current text content and its known UTF8 run/AST
/// declarations. Immutable source caches and original opaque records are untouched.
pub fn set_stem_content(item: &mut SourceItem, resource: &ResourceReference, origin: [i32; 2], replacement: &str, ctl: &Interrupt<'_>) -> Result<bool> {
    qualify_stem_source(item, resource, origin, ctl)?;
    if !(1..=8).contains(&replacement.len()) || replacement.bytes().any(|b| b != b'I') {
        return Err(invalid());
    }
    update_content(item, replacement, ctl)
}
/// Validates both current and proposed exact-font geometry before an atomic
/// content update. No source cache, resource identity or locator is modified.
pub fn set_rectilinear_content(
    item: &mut SourceItem,
    resource: &ResourceReference,
    replacement: StemText<'_>,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let current = qualify_rectilinear_source(item, resource, replacement.origin, ctl)?;
    super::prepare_rectilinear_with_charge(font, current, max_resident, ctl, charge)?;
    super::prepare_rectilinear_with_charge(font, replacement, max_resident, ctl, charge)?;
    update_content(item, replacement.content, ctl)
}
/// Updates content for an already qualified declared AA or no-AA source.
/// This does not mutate the mode, original cache or immutable font identity.
pub fn set_rectilinear_content_with_antialias(
    item: &mut SourceItem,
    resource: &ResourceReference,
    replacement: StemText<'_>,
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let (current, _) = qualify_rectilinear_source_with_antialias(item, resource, replacement.origin, ctl)?;
    super::prepare_rectilinear_with_charge(font, current, max_resident, ctl, charge)?;
    super::prepare_rectilinear_with_charge(font, replacement, max_resident, ctl, charge)?;
    update_content(item, replacement.content, ctl)
}
fn update_content(item: &mut SourceItem, replacement: &str, ctl: &Interrupt<'_>) -> Result<bool> {
    let mut content = String::new();
    content.try_reserve_exact(replacement.len()).map_err(|_| TextSourceError::Allocation)?;
    content.push_str(replacement);
    let mut hex = String::new();
    hex.try_reserve_exact(replacement.len() * 2).map_err(|_| TextSourceError::Allocation)?;
    const DIGITS: &[u8] = b"0123456789abcdef";
    for byte in replacement.bytes() {
        hex.push(char::from(*DIGITS.get(usize::from(byte >> 4)).ok_or_else(invalid)?));
        hex.push(char::from(*DIGITS.get(usize::from(byte & 15)).ok_or_else(invalid)?));
    }
    poll(ctl)?;
    let EditableSource::Text { content: current, configuration, .. } = &mut item.source else { return Err(invalid()) };
    if *current == content {
        return Ok(false);
    }
    // Prior qualification established every exact unique slot. Resolve all
    // mutable references before modifying any, so an error leaves the item intact.
    let tree_index = configuration.iter().position(|a| a.name == "gimp-text-configuration").ok_or_else(invalid)?;
    let run_index = configuration.iter().position(|a| a.name == "runs").ok_or_else(invalid)?;
    let [tree_node, run_node] = configuration.get_disjoint_mut([tree_index, run_index]).map_err(|_| invalid())?;
    let FilterValue::Config(tree) = &mut tree_node.value else { return Err(invalid()) };
    let text = config_mut(tree, 0)?;
    let text_slot = text.get_mut(1).ok_or_else(invalid)?;
    let FilterValue::Config(runs) = &mut run_node.value else { return Err(invalid()) };
    let run = config_mut(runs, 0)?;
    let end = run.iter_mut().find(|a| a.name == "end").ok_or_else(invalid)?;
    poll(ctl)?;
    text_slot.value = FilterValue::String(hex);
    end.value = FilterValue::Number(ExactNumber::Unsigned { width: 64, value: content.len() as u64 });
    *current = content;
    Ok(true)
}
fn config_mut(fields: &mut [FilterArgument], index: usize) -> Result<&mut Vec<FilterArgument>> {
    let node = fields.get_mut(index).ok_or_else(invalid)?;
    if let FilterValue::Config(value) = &mut node.value { Ok(value) } else { Err(invalid()) }
}

/// Atomic checked current placement, independent of original source bounds,
/// configuration, raster cache, transform and resource provenance.
#[allow(clippy::too_many_arguments)]
pub fn set_rectilinear_origin(
    item: &mut SourceItem,
    resource: &ResourceReference,
    original_origin: [i32; 2],
    replacement: [i32; 2],
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    poll(ctl)?;
    if replacement.into_iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(&v)) {
        return Err(TextSourceError::Unsupported("text placement exceeds safe coordinates"));
    }
    let (current, _, _, spacing) = qualify_placed_spaced_colored_rectilinear_source(item, resource, original_origin, ctl)?;
    super::prepare_spaced_rectilinear_with_charge(font, current, spacing, max_resident, ctl, charge)?;
    super::prepare_spaced_rectilinear_with_charge(font, super::ScaledText { origin: replacement, ..current }, spacing, max_resident, ctl, charge)?;
    let next = if replacement == original_origin { None } else { Some(replacement) };
    let EditableSource::Text { current_origin, .. } = &mut item.source else { return Err(invalid()) };
    if *current_origin == next {
        return Ok(false);
    }
    poll(ctl)?;
    *current_origin = next;
    Ok(true)
}

/// Single underline markup is an additive opt-in; ordinary source qualifiers
/// remain closed to all markup/styles. This never reads a source raster cache.
#[derive(Clone, Copy)]
pub struct QualifiedUnderlinedText<'a> {
    pub request: super::ScaledText<'a>,
    pub antialias: RectilinearAntialias,
    pub runs: super::underline::UnderlineRuns,
}
pub fn qualify_underlined_source<'a>(
    item: &'a SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ctl: &Interrupt<'_>,
) -> Result<QualifiedUnderlinedText<'a>> {
    poll(ctl)?;
    let EditableSource::Text { content, markup, configuration, .. } = &item.source else {
        return Err(invalid());
    };
    let runs = super::underline::parse_underline_markup(content, markup.as_deref().ok_or_else(invalid)?, ctl)?;
    let layout = config(field(configuration, "layout")?)?;
    let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field(layout, "font-size")? else {
        return Err(invalid());
    };
    let size = f64::from_bits(*bits);
    if !matches!(size, 16. | 32.) {
        return Err(invalid());
    }
    let FilterValue::Bool(aa) = field(layout, "antialias")? else {
        return Err(invalid());
    };
    if !*aa && size != 32. {
        return Err(TextSourceError::Unsupported("scaled mono glyph rasterization remains unqualified"));
    }
    let request = qualify_source_inner(item, resource, origin, ctl, true, *aa, size, None, None, false, None, Some(&runs))?;
    Ok(QualifiedUnderlinedText {
        request: super::ScaledText { content: request.content, origin: request.origin, size_px: size as f32 },
        antialias: if *aa { RectilinearAntialias::AreaCoverage } else { RectilinearAntialias::None },
        runs,
    })
}
fn verify_underline_configuration(configuration: &[FilterArgument], plan: &super::underline::UnderlineRuns) -> Result<()> {
    let styles = config(field(configuration, "styles")?)?;
    if styles.len() != plan.style_len {
        return Err(invalid());
    }
    for (index, style) in plan.styles.iter().take(plan.style_len).enumerate() {
        let fields = config(indexed_field(styles, "style", index)?)?;
        if style.tag == super::underline::Tag::Span {
            names(fields, &["tag", "underline"])?;
            if !string(field(fields, "underline")?, if style.underline == Some(true) { "single" } else { "none" }) {
                return Err(invalid());
            }
        } else {
            names(fields, &["tag"])?;
        }
        if !string(field(fields, "tag")?, style.tag.name()) {
            return Err(invalid());
        }
    }
    let runs = config(field(configuration, "runs")?)?;
    if runs.len() != plan.run_len {
        return Err(invalid());
    }
    for (index, run) in plan.runs.iter().take(plan.run_len).enumerate() {
        let fields = config(indexed_field(runs, "run", index)?)?;
        names(fields, &["start", "end", "styles"])?;
        if !uint(field(fields, "start")?, 64, run.start as u64) || !uint(field(fields, "end")?, 64, run.end as u64) {
            return Err(invalid());
        }
        let ids = config(field(fields, "styles")?)?;
        if ids.len() != run.depth {
            return Err(invalid());
        }
        for (order, id) in run.ids.iter().take(run.depth).enumerate() {
            if !uint(indexed_field(ids, "style-id", order)?, 64, *id as u64) {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn indexed_field<'a>(fields: &'a [FilterArgument], prefix: &str, index: usize) -> Result<&'a FilterValue> {
    if fields.len() > 32 {
        return Err(invalid());
    }
    let mut found = None;
    for f in fields {
        if let Some((p, i)) = f.name.rsplit_once('-')
            && p == prefix
            && i.bytes().all(|b| b.is_ascii_digit())
            && i.parse::<usize>().ok() == Some(index)
            && (i == "0" || !i.starts_with('0'))
        {
            if found.is_some() {
                return Err(invalid());
            }
            found = Some(&f.value);
        }
    }
    found.ok_or_else(invalid)
}

/// Atomic current underline run editing. Original opaque source bytes and cache
/// remain independent; only the current semantic markup/configuration changes.
#[allow(clippy::too_many_arguments)]
pub fn set_underlined_ranges(
    item: &mut SourceItem,
    resource: &ResourceReference,
    origin: [i32; 2],
    ranges: &[[usize; 2]],
    font: &super::ExactFont<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<bool> {
    let qualified = qualify_underlined_source(item, resource, origin, ctl)?;
    if ranges.len() > 8 {
        return Err(invalid());
    }
    let content = qualified.request.content;
    let mut last = 0;
    let mut selected = [false; 8];
    for &[start, end] in ranges {
        poll(ctl)?;
        if start < last || start >= end || end > content.len() || !content.is_char_boundary(start) || !content.is_char_boundary(end) {
            return Err(invalid());
        }
        selected.get_mut(start..end).ok_or_else(invalid)?.fill(true);
        last = end;
    }
    if selected == qualified.runs.underlined_bytes() {
        return Ok(false);
    }
    let mut markup = spacing_string("", charge)?;
    charge(128)?;
    markup.try_reserve_exact(128).map_err(|_| TextSourceError::Allocation)?;
    if markup.capacity() > 128 {
        return Err(TextSourceError::Budget);
    }
    markup.push_str("<markup>");
    let mut cursor = 0;
    for &[start, end] in ranges {
        markup.push_str(content.get(cursor..start).ok_or_else(invalid)?);
        markup.push_str("<u>");
        markup.push_str(content.get(start..end).ok_or_else(invalid)?);
        markup.push_str("</u>");
        cursor = end;
    }
    markup.push_str(content.get(cursor..).ok_or_else(invalid)?);
    markup.push_str("</markup>");
    let plan = super::underline::parse_underline_markup(content, &markup, ctl)?;
    let proposed = QualifiedUnderlinedText { runs: plan, ..qualified };
    let mut work = 0_u64;
    let _prepared = super::underline::prepare_underlined_with_charge(font, proposed, max_resident, ctl, charge, &mut |n| {
        work = work.checked_add(n).ok_or(TextSourceError::Budget)?;
        if work > 1_u64 << 36 { Err(TextSourceError::Budget) } else { Ok(()) }
    })?;
    let mut styles = spacing_fields(plan.style_len, charge)?;
    for (idx, style) in plan.styles.iter().take(plan.style_len).enumerate() {
        let mut fields = spacing_fields(if style.tag == super::underline::Tag::Span { 2 } else { 1 }, charge)?;
        fields.push(spacing_arg("tag", FilterValue::String(spacing_string(style.tag.name(), charge)?), charge)?);
        if let Some(value) = style.underline.filter(|_| style.tag == super::underline::Tag::Span) {
            fields.push(spacing_arg("underline", FilterValue::String(spacing_string(if value { "single" } else { "none" }, charge)?), charge)?);
        }
        styles.push(underline_indexed_arg("style", idx, FilterValue::Config(fields), charge)?);
    }
    let mut runs = spacing_fields(plan.run_len, charge)?;
    for (idx, run) in plan.runs.iter().take(plan.run_len).enumerate() {
        let mut ids = spacing_fields(run.depth, charge)?;
        for (order, id) in run.ids.iter().take(run.depth).enumerate() {
            ids.push(underline_indexed_arg("style-id", order, FilterValue::Number(ExactNumber::Unsigned { width: 64, value: *id as u64 }), charge)?);
        }
        let mut fields = spacing_fields(3, charge)?;
        fields.push(spacing_arg("start", FilterValue::Number(ExactNumber::Unsigned { width: 64, value: run.start as u64 }), charge)?);
        fields.push(spacing_arg("end", FilterValue::Number(ExactNumber::Unsigned { width: 64, value: run.end as u64 }), charge)?);
        fields.push(spacing_arg("styles", FilterValue::Config(ids), charge)?);
        runs.push(underline_indexed_arg("run", idx, FilterValue::Config(fields), charge)?);
    }
    let mut hex = spacing_string("", charge)?;
    let hex_len = markup.len().checked_mul(2).ok_or(TextSourceError::Budget)?;
    charge(hex_len as u64)?;
    hex.try_reserve_exact(hex_len).map_err(|_| TextSourceError::Allocation)?;
    if hex.capacity() > hex_len {
        return Err(TextSourceError::Budget);
    }
    const DIGITS: &[u8] = b"0123456789abcdef";
    for b in markup.bytes() {
        hex.push(char::from(*DIGITS.get(usize::from(b >> 4)).ok_or_else(invalid)?));
        hex.push(char::from(*DIGITS.get(usize::from(b & 15)).ok_or_else(invalid)?));
    }
    // Qualification proves unique slots. Resolve every mutable target before
    // publication; a cancellation/allocation/schema error cannot partially edit.
    let EditableSource::Text { markup: current, configuration, .. } = &mut item.source else { return Err(invalid()) };
    let ti = configuration.iter().position(|a| a.name == "gimp-text-configuration").ok_or_else(invalid)?;
    let ri = configuration.iter().position(|a| a.name == "runs").ok_or_else(invalid)?;
    let si = configuration.iter().position(|a| a.name == "styles").ok_or_else(invalid)?;
    let [tree, run_slot, style_slot] = configuration.get_disjoint_mut([ti, ri, si]).map_err(|_| invalid())?;
    let FilterValue::Config(tree) = &mut tree.value else { return Err(invalid()) };
    let quoted = config_mut(tree, 0)?.get_mut(1).ok_or_else(invalid)?;
    poll(ctl)?;
    quoted.value = FilterValue::String(hex);
    run_slot.value = FilterValue::Config(runs);
    style_slot.value = FilterValue::Config(styles);
    *current = Some(markup);
    Ok(true)
}

fn underline_indexed_arg(prefix: &str, index: usize, value: FilterValue, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<FilterArgument> {
    if index > 15 {
        return Err(invalid());
    }
    let len = prefix.len().checked_add(if index < 10 { 2 } else { 3 }).ok_or(TextSourceError::Budget)?;
    charge(len as u64)?;
    let mut name = String::new();
    name.try_reserve_exact(len).map_err(|_| TextSourceError::Allocation)?;
    if name.capacity() > len {
        return Err(TextSourceError::Budget);
    }
    name.push_str(prefix);
    name.push('-');
    if index >= 10 {
        name.push('1')
    }
    name.push(char::from(b'0' + u8::try_from(index % 10).map_err(|_| invalid())?));
    charge(std::mem::size_of::<FilterArgument>() as u64)?;
    Ok(FilterArgument { name, value })
}
