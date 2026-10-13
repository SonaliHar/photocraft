//! Candidate source-unit layout. No raster or independently qualified source epoch.
//! Font bytes are explicit runtime authority; source names never resolve a font.
use super::{ExactFont, TextSourceError, poll, prepare_source_unit_layout, units::TextUnitLayout};
use photocraft_doc::{
    PhysicalResolution,
    xcf_filters::{FilterArgument, FilterValue},
    xcf_items::{EditableSource, SourceItem},
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_raster::Interrupt;
use skrifa::MetadataProvider;

type Result<T> = std::result::Result<T, TextSourceError>;
const SHAPING_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TEXT: usize = 256;
/// Opaque borrowed font authority tied to one canonical resource and exact face.
/// Construction hashes only supplied bytes; it never reads a source locator or
/// installs a font in the global text engine. Caller owns the bytes' residency.
pub struct AuthorizedSourceFont<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) resource: photocraft_doc::item_identity::ResourceId,
    pub(super) sha256: [u8; 32],
    pub(super) index: u32,
}
impl<'a> AuthorizedSourceFont<'a> {
    pub fn authorize(
        bytes: &'a [u8],
        resource: photocraft_doc::item_identity::ResourceId,
        retained_sha256: [u8; 32],
        index: u32,
        control: &Interrupt<'_>,
    ) -> Result<Self> {
        poll(control)?;
        if resource.0 == 0 || index > 4095 {
            return Err(TextSourceError::FontIdentity);
        }
        crate::fonts::identify_authorized_font_bytes(bytes, &retained_sha256, control).map_err(|e| match e {
            crate::fonts::FontIdentityError::Cancelled => TextSourceError::Cancelled,
            crate::fonts::FontIdentityError::InputLimit => TextSourceError::Budget,
            crate::fonts::FontIdentityError::Allocation => TextSourceError::Allocation,
            _ => TextSourceError::FontIdentity,
        })?;
        let face = skrifa::FontRef::from_index(bytes, index).map_err(|_| TextSourceError::FontIdentity)?;
        if !face.axes().is_empty() {
            return Err(TextSourceError::Unsupported("candidate variable font axes require explicit instance"));
        }
        poll(control)?;
        Ok(Self { bytes, resource, sha256: retained_sha256, index })
    }
}
fn digest_matches(value: &FilterValue, digest: &[u8; 32]) -> bool {
    let FilterValue::String(hex) = value else {
        return false;
    };
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    hex.as_bytes().as_chunks::<2>().0.iter().zip(digest).all(|(pair, byte)| {
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        };
        match (pair.first().and_then(|b| digit(*b)), pair.get(1).and_then(|b| digit(*b))) {
            (Some(a), Some(b)) => a * 16 + b == *byte,
            _ => false,
        }
    })
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: 4 * 1024 * 1024, work_units: 1_000_000 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub glyph_id: u32,
    pub cluster: u32,
    /// Position relative to the explicitly supplied baseline, in canvas pixels.
    pub position: [f64; 2],
    pub advance: [f64; 2],
    pub(super) empty_outline_allowed: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum LtrScript {
    Latin,
    Greek,
}
#[derive(Clone, Copy, Debug)]
pub struct LinePlacement {
    pub baseline: [f64; 2],
    pub script: LtrScript,
}
pub struct PreparedUnitLayout {
    pub(super) font_resource: photocraft_doc::item_identity::ResourceId,
    pub(super) font_sha256: [u8; 32],
    pub(super) font_index: u32,
    pub glyphs: Vec<PositionedGlyph>,
    pub basis: TextUnitLayout,
    /// Independent em scales for outline preparation by a future caller.
    pub pixels_per_font_unit: [f64; 2],
    pub retained_bytes: u64,
}
fn unsupported() -> TextSourceError {
    TextSourceError::Unsupported("unsupported candidate source-unit text style or font dependency")
}
fn field<'a>(fields: &'a [FilterArgument], key: &str, ctl: &Interrupt<'_>) -> Result<&'a FilterValue> {
    if fields.len() > 64 {
        return Err(TextSourceError::Budget);
    }
    let mut found = None;
    for f in fields {
        poll(ctl)?;
        if f.name.len() > 128 {
            return Err(TextSourceError::Budget);
        }
        if f.name == key {
            if found.is_some() {
                return Err(unsupported());
            }
            found = Some(&f.value);
        }
    }
    found.ok_or_else(unsupported)
}
fn config(value: &FilterValue) -> Result<&[FilterArgument]> {
    match value {
        FilterValue::Config(fields) => Ok(fields),
        _ => Err(unsupported()),
    }
}
fn literal(value: &FilterValue, expected: &str) -> bool {
    matches!(value,FilterValue::String(s) if s==expected)
}

/// Prepare a bounded candidate plan from current source fields, without consulting
/// SourceFontCache, original raster data, locators or a fallback font database.
/// Unsupported styles/fonts refuse; this plan does not assert GIMP layout equivalence.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    item: &SourceItem,
    resource: &ResourceReference,
    font: &ExactFont<'_>,
    resolution: &PhysicalResolution,
    baseline: [f64; 2],
    limits: Limits,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<PreparedUnitLayout> {
    let authorized = AuthorizedSourceFont { bytes: font.bytes, resource: resource.id, sha256: super::INTER_SHA, index: 0 };
    prepare_external(item, resource, &authorized, resolution, baseline, limits, ctl, charge)
}

/// Caller-authorized external font variant; identical closed style/domain limits.
/// Exact byte digest and face index must match retained source selectors.
#[allow(clippy::too_many_arguments)]
pub fn prepare_external(
    item: &SourceItem,
    resource: &ResourceReference,
    font: &AuthorizedSourceFont<'_>,
    resolution: &PhysicalResolution,
    baseline: [f64; 2],
    limits: Limits,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<PreparedUnitLayout> {
    prepare_external_positioned(item, resource, font, resolution, &[LinePlacement { baseline, script: LtrScript::Latin }], limits, ctl, charge)
}

/// Explicit baselines and scripts for each retained newline-separated LTR line.
/// No inferred line height, bidi resolution, fallback or script guessing is performed.
#[allow(clippy::too_many_arguments)]
pub fn prepare_external_positioned(
    item: &SourceItem,
    resource: &ResourceReference,
    font: &AuthorizedSourceFont<'_>,
    resolution: &PhysicalResolution,
    lines: &[LinePlacement],
    limits: Limits,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<PreparedUnitLayout> {
    poll(ctl)?;
    let defaults = Limits::default();
    if limits.resident_bytes > defaults.resident_bytes || limits.work_units > defaults.work_units {
        return Err(TextSourceError::Budget);
    }
    let EditableSource::Text { content, markup, font: font_id, direction, configuration, .. } = &item.source else {
        return Err(unsupported());
    };
    if content.is_empty()
        || content.len() > MAX_TEXT
        || !content.chars().all(|c| c == '\n' || matches!(u32::from(c),32..=126|0x00a0..=0x024f|0x0300..=0x036f|0x0370..=0x03ff))
        || lines.is_empty()
        || lines.len() > 16
        || content.split('\n').count() != lines.len()
        || markup.is_some()
        || *direction != 0
        || item.transform.is_some()
        || lines.iter().flat_map(|line| line.baseline).any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29))
        || resource.id != *font_id
        || resource.kind != ResourceKind::Font
        || resource.id != font.resource
        || resource.collection.is_some()
        || resource.internal
        || resource.owned_bytes.is_some()
        || resource.content_hash.is_some()
    {
        return Err(unsupported());
    }
    let layout = config(field(configuration, "layout", ctl)?)?;
    for (key, value) in [("language", "C"), ("base-direction", "ltr"), ("justification", "left"), ("box-mode", "dynamic")] {
        if !literal(field(layout, key, ctl)?, value) {
            return Err(unsupported());
        }
    }
    // Serialized booleans are exact DTO data, not a choice of hinter/grid.
    // The current candidate's raster consumer implements only unhinted area coverage.
    super::unit_raster_policy::read_source(item, ctl)?.require_unhinted_area()?;
    // Closed candidate style lane: spacing, transforms, outlines and box layout
    // must have dedicated implementations, rather than being silently ignored.
    for (index, f) in layout.iter().enumerate() {
        poll(ctl)?;
        if layout.iter().take(index).any(|prior| prior.name == f.name) {
            return Err(unsupported());
        }
        if !matches!(
            f.name.as_str(),
            "font-size" | "font-size-unit" | "language" | "base-direction" | "justification" | "box-mode" | "box-unit" | "antialias" | "hinting"
        ) {
            return Err(unsupported());
        }
    }
    let FilterValue::String(box_unit) = field(layout, "box-unit", ctl)? else {
        return Err(unsupported());
    };
    super::units::resolve_size(0f64.to_bits(), box_unit, Some(resolution), ctl)?;
    for key in ["runs", "styles"] {
        if !config(field(configuration, key, ctl)?)?.is_empty() {
            return Err(unsupported());
        }
    }
    let selectors = config(field(configuration, "font-selectors", ctl)?)?;
    if !literal(field(selectors, "name", ctl)?, &resource.name) || !digest_matches(field(selectors, "requested-hash", ctl)?, &font.sha256) {
        return Err(TextSourceError::FontIdentity);
    }
    if !matches!(field(selectors, "index", ctl)?, FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value }) if *value == u64::from(font.index))
    {
        return Err(TextSourceError::FontIdentity);
    }
    let basis = prepare_source_unit_layout(item, Some(resolution), ctl)?;
    if basis.box_size.is_some() || [basis.font_size.x_pixels, basis.font_size.y_pixels].iter().any(|v| !v.is_finite() || *v <= 0.0 || *v > 4096.0) {
        return Err(unsupported());
    }
    let output_bytes = (MAX_TEXT as u64).checked_mul(std::mem::size_of::<PositionedGlyph>() as u64).ok_or(TextSourceError::Budget)?;
    let resident = SHAPING_BYTES.checked_add(output_bytes).ok_or(TextSourceError::Budget)?;
    let work = (content.len() as u64).checked_mul(4096).ok_or(TextSourceError::Budget)?;
    if resident > limits.resident_bytes || work > limits.work_units {
        return Err(TextSourceError::Budget);
    }
    charge(resident)?;
    poll(ctl)?;
    let shaping_font = harfrust::FontRef::from_index(font.bytes, font.index).map_err(|_| TextSourceError::FontIdentity)?;
    let data = harfrust::ShaperData::new(&shaping_font);
    let shaper = data.shaper(&shaping_font).build();
    let upem = f64::from(shaper.units_per_em());
    if upem <= 0.0 {
        return Err(TextSourceError::FontIdentity);
    }
    let scale = [basis.font_size.x_pixels / upem, basis.font_size.y_pixels / upem];
    if scale.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err(unsupported());
    }
    let mut glyphs = Vec::new();
    glyphs.try_reserve_exact(MAX_TEXT).map_err(|_| TextSourceError::Allocation)?;
    let actual = (glyphs.capacity() as u64).checked_mul(std::mem::size_of::<PositionedGlyph>() as u64).ok_or(TextSourceError::Budget)?;
    if actual > output_bytes {
        return Err(TextSourceError::Budget);
    }
    let mut byte_offset = 0usize;
    for (line, placement) in content.split('\n').zip(lines) {
        poll(ctl)?;
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(line);
        buffer.set_direction(harfrust::Direction::LeftToRight);
        buffer.set_script(match placement.script {
            LtrScript::Latin => harfrust::script::LATIN,
            LtrScript::Greek => harfrust::script::GREEK,
        });
        let shaped = shaper.shape(buffer, harfrust::ShapeOptions::default());
        poll(ctl)?;
        if shaped.glyph_infos().len() != shaped.glyph_positions().len() || glyphs.len().checked_add(shaped.glyph_infos().len()).is_none_or(|n| n > MAX_TEXT) {
            return Err(TextSourceError::Budget);
        }
        let mut pen = placement.baseline;
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            poll(ctl)?;
            if info.glyph_id == 0 || usize::try_from(info.cluster).ok().is_none_or(|n| n >= line.len() || !line.is_char_boundary(n)) {
                return Err(unsupported());
            }
            let position = [pen[0] + f64::from(pos.x_offset) * scale[0], pen[1] - f64::from(pos.y_offset) * scale[1]];
            let advance = [f64::from(pos.x_advance) * scale[0], -f64::from(pos.y_advance) * scale[1]];
            pen = [pen[0] + advance[0], pen[1] + advance[1]];
            if position.iter().chain(advance.iter()).chain(pen.iter()).any(|v| !v.is_finite() || v.abs() > f64::from(1 << 29)) {
                return Err(unsupported());
            }
            let cluster = u32::try_from(byte_offset).ok().and_then(|o| o.checked_add(info.cluster)).ok_or(TextSourceError::Budget)?;
            let empty_outline_allowed = line
                .get(usize::try_from(info.cluster).map_err(|_| TextSourceError::Budget)?..)
                .and_then(|s| s.chars().next())
                .is_some_and(|c| c == ' ' || c == '\u{00a0}');
            glyphs.push(PositionedGlyph { glyph_id: info.glyph_id, cluster, position, advance, empty_outline_allowed });
        }
        byte_offset = byte_offset.checked_add(line.len()).and_then(|n| n.checked_add(1)).ok_or(TextSourceError::Budget)?;
    }
    poll(ctl)?;
    Ok(PreparedUnitLayout {
        font_resource: font.resource,
        font_sha256: font.sha256,
        font_index: font.index,
        glyphs,
        basis,
        pixels_per_font_unit: scale,
        retained_bytes: actual,
    })
}
