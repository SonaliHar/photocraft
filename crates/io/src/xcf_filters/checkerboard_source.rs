//! Exact retained six-argument Checkerboard storage binding, not rendering admission.
//! Chosen public Windows3.2.6 schema has six known stored properties plus an
//! unknown GParam output-format placeholder. No seventh/default value is invented.
//! Native color layout/sample/tail bytes remain independent uninterpreted storage;
//! source profiles, output-format authority, masks and opaque composition require
//! separate caller evidence. All original decoder/effect/mask/provider backing
//! (including unselected containers) stays held in the enclosing caller ledger.
pub use super::configured_source_candidate::{CurvesMappingLimits as CheckerboardMappingLimits, CurvesMappingRequirements as CheckerboardMappingRequirements};
use super::configured_source_candidate::{Meter, raw_argument, word};
use super::stretch_hsv_source::{header, raw_state_with_arguments};
use crate::IoError;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactColorStorage, ExactNumber, FilterArgument, FilterNode, FilterValue, OperationIdentity},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, Drawable, Effect, RawProperty};
use std::mem::size_of;
pub const CHECKERBOARD_SCHEMA: &str = "ebd4bd09a17546fdcb3142db0748e8a65ffe4a7d304d4c2850b59e4ec6792bdd";
pub const CHECKERBOARD_EPOCH: &str = "gimp-3.2.6-windows-ebd4bd09a17546fd";
#[derive(Clone, Copy, Debug)]
pub struct CheckerboardSourceIdentity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicSchemaGap {
    UnknownOutputFormatParameterNotStored,
}
#[derive(Clone, Copy, Debug)]
pub struct CheckerboardColorStorage<'a> {
    pub layout: &'a str,
    pub samples: &'a [u8],
    pub opaque_tail: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub struct CheckerboardStoredFields<'a> {
    pub integer_bits: [u32; 4],
    pub colors: [CheckerboardColorStorage<'a>; 2],
}
pub struct MappedCheckerboardSource<'a, P> {
    source: &'a [u8],
    effect: &'a Effect<P>,
    identity: CheckerboardSourceIdentity<'a>,
    stored: CheckerboardStoredFields<'a>,
    node: FilterNode,
    requirements: CheckerboardMappingRequirements,
}
impl<'a, P> MappedCheckerboardSource<'a, P> {
    pub fn original_source(&self) -> &'a [u8] {
        self.source
    }
    pub fn original_effect(&self) -> &'a Effect<P> {
        self.effect
    }
    pub fn original_mask(&self) -> Option<&'a Drawable<P>> {
        self.effect.mask.as_deref()
    }
    pub fn identity(&self) -> CheckerboardSourceIdentity<'a> {
        self.identity
    }
    pub fn stored(&self) -> CheckerboardStoredFields<'a> {
        self.stored
    }
    pub fn schema_gap(&self) -> PublicSchemaGap {
        PublicSchemaGap::UnknownOutputFormatParameterNotStored
    }
    pub fn node(&self) -> &FilterNode {
        &self.node
    }
    pub fn requirements(&self) -> CheckerboardMappingRequirements {
        self.requirements
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("retained Checkerboard source: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
fn extent(a: usize, b: usize) -> Result<usize, IoError> {
    a.checked_add(b).ok_or_else(|| bad("raw extent"))
}
fn same(left: &[u8], right: &[u8], m: &mut Meter<'_, '_>) -> Result<bool, IoError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (a, b) in left.chunks(4096).zip(right.chunks(4096)) {
        m.step(mul(a.len() as u64, 2)?)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn color<'a>(a: &'a Argument, raw: &[u8], at: usize, m: &mut Meter<'_, '_>) -> Result<CheckerboardColorStorage<'a>, IoError> {
    let ArgumentValue::Color { encoding, samples, icc } = &a.value else {
        return Err(bad("literal nonnull StoredColor required"));
    };
    if encoding.is_empty() || encoding.len() > 65536 || samples.len() > 1 << 20 || icc.len() > 1 << 20 {
        return Err(bad("bounded native color storage"));
    }
    m.input_text(encoding)?;
    m.input(samples.capacity() as u64)?;
    m.input(icc.capacity() as u64)?;
    m.step(encoding.len() as u64)?;
    if encoding.contains('\0') {
        return Err(bad("native layout NUL"));
    }
    let n = usize::try_from(word(raw, at)?).map_err(|_| bad("layout length"))?;
    let start = extent(at, 4)?;
    let end = extent(start, n)?;
    let layout = raw.get(start..end).and_then(|v| v.strip_suffix(&[0])).ok_or_else(|| bad("native layout framing"))?;
    if !same(layout, encoding.as_bytes(), m)? {
        return Err(bad("original layout association"));
    }
    let sample_len = usize::try_from(word(raw, end)?).map_err(|_| bad("sample length"))?;
    if sample_len != samples.len() {
        return Err(bad("native sample length"));
    }
    let start = extent(end, 4)?;
    let end = extent(start, sample_len)?;
    if !same(raw.get(start..end).ok_or_else(|| bad("native sample extent"))?, samples, m)? {
        return Err(bad("original sample association"));
    }
    let tail_len = usize::try_from(word(raw, end)?).map_err(|_| bad("opaque tail length"))?;
    if tail_len != icc.len() {
        return Err(bad("opaque tail length"));
    }
    let start = extent(end, 4)?;
    let end = extent(start, tail_len)?;
    if end != raw.len() || !same(raw.get(start..end).ok_or_else(|| bad("opaque tail extent"))?, icc, m)? {
        return Err(bad("original opaque tail association"));
    }
    Ok(CheckerboardColorStorage { layout: encoding, samples, opaque_tail: icc })
}
fn fields<'a, P>(effect: &'a Effect<P>, m: &mut Meter<'_, '_>) -> Result<CheckerboardStoredFields<'a>, IoError> {
    let mut integers = [0; 4];
    let mut colors = [None; 2];
    let mut seen = 0u8;
    let mut indices = [usize::MAX; 6];
    for a in &effect.arguments {
        m.step(64)?;
        if a.name.len() > 8 || a.configuration.is_some() || a.graph.is_some() {
            return Err(bad("closed literal argument"));
        }
        let slot = match a.name.as_str() {
            "x" => 0,
            "y" => 1,
            "x-offset" => 2,
            "y-offset" => 3,
            "color1" => 4,
            "color2" => 5,
            _ => return Err(bad("six known stored names; no unknown default")),
        };
        let bit = 1u8 << slot;
        if seen & bit != 0 || indices.contains(&a.property_index) {
            return Err(bad("duplicate name/raw property association"));
        }
        seen |= bit;
        *indices.get_mut(slot).ok_or_else(|| bad("fixed slot"))? = a.property_index;
        m.input_text(&a.name)?;
        m.step(add(mul(a.name.len() as u64, 2)?, 1)?)?;
        let (raw, end) = raw_argument(a, &effect.properties)?;
        let kind = word(raw, end)?;
        let at = extent(end, 4)?;
        if slot < 4 {
            let ArgumentValue::Integer(value) = &a.value else {
                return Err(bad("original type1 signed32 word"));
            };
            if kind != 1 || extent(at, 4)? != raw.len() || word(raw, at)? != *value as u32 {
                return Err(bad("original integer raw/typed association"));
            }
            if slot < 2 && *value <= 0 {
                return Err(bad("positive public cell dimensions"));
            }
            *integers.get_mut(slot).ok_or_else(|| bad("integer slot"))? = *value as u32;
        } else {
            if kind != 8 {
                return Err(bad("original type8 color word"));
            }
            *colors.get_mut(slot - 4).ok_or_else(|| bad("color slot"))? = Some(color(a, raw, at, m)?);
        }
    }
    if seen != 63 {
        return Err(bad("complete duplicate-free six fields"));
    }
    let [first, second] = colors;
    Ok(CheckerboardStoredFields {
        integer_bits: integers,
        colors: [first.ok_or_else(|| bad("first color missing"))?, second.ok_or_else(|| bad("second color missing"))?],
    })
}
/// Full lowered enclosing reserve and work prepayment precede source scans and
/// copies. Identity is an explicit source/schema association, not authentication.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_checkerboard_source<'a, P>(
    source: &'a [u8],
    effect: &'a Effect<P>,
    filter: FilterId,
    mask: Option<MaskId>,
    identity: CheckerboardSourceIdentity<'a>,
    limits: CheckerboardMappingLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedCheckerboardSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = CheckerboardMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(bad("lower-only limits"));
    }
    if identity.xcf_version != 22
        || identity.expected_epoch != CHECKERBOARD_EPOCH
        || identity.schema_sha256 != CHECKERBOARD_SCHEMA
        || effect.operation != "gegl:checkerboard"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 6
        || effect.properties.len() > 32
        || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0)
        || mask.is_some() != effect.mask.is_some()
        || source.get(..14) != Some(b"gimp xcf v022\0")
    {
        return Err(bad("exact six-argument source/schema/epoch/IDs"));
    }
    for text in [&effect.name, &effect.icon, &effect.operation] {
        if text.len() > 65536 {
            return Err(bad("source text size"));
        }
    }
    if effect.properties.iter().any(|v| v.payload.len() > 1 << 20) {
        return Err(bad("property size"));
    }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m = Meter { limits, input: 0, copied: size_of::<MappedCheckerboardSource<'a, P>>() as u64, index: 0, work: 0, ctl };
    m.check()?;
    m.input(source.len() as u64)?;
    m.input(size_of::<Effect<P>>() as u64)?;
    m.input(identity.expected_epoch.len() as u64)?;
    for text in [&effect.name, &effect.icon, &effect.operation] {
        m.input_text(text)?;
        for chunk in text.as_bytes().chunks(4096) {
            m.step(chunk.len() as u64)?;
            if chunk.contains(&0) {
                return Err(bad("source text NUL"));
            }
        }
    }
    if let Some(version) = &effect.version {
        m.input_text(version)?;
    }
    m.input(mul(effect.properties.capacity() as u64, size_of::<RawProperty>() as u64)?)?;
    m.input(mul(effect.arguments.capacity() as u64, size_of::<Argument>() as u64)?)?;
    let mut count = 0usize;
    for prop in &effect.properties {
        m.input(prop.payload.capacity() as u64)?;
        m.step(1)?;
        if prop.tag == 45 {
            count = count.checked_add(1).ok_or_else(|| bad("raw count"))?;
        }
    }
    if count != 6 {
        return Err(bad("exact six raw argument properties"));
    }
    header(source, effect, &mut m)?;
    raw_state_with_arguments(source, effect, &mut m)?;
    let stored = fields(effect, &mut m)?;
    let mut arguments = m.vector::<FilterArgument>(6)?;
    for a in &effect.arguments {
        m.step(16)?;
        let value = match &a.value {
            ArgumentValue::Integer(value) => FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(*value) }),
            ArgumentValue::Color { encoding, samples, icc } => {
                let mut bytes = m.vector::<u8>(samples.len())?;
                for chunk in samples.chunks(4096) {
                    m.step(chunk.len() as u64)?;
                    bytes.extend_from_slice(chunk);
                }
                let mut tail = m.vector::<u8>(icc.len())?;
                for chunk in icc.chunks(4096) {
                    m.step(chunk.len() as u64)?;
                    tail.extend_from_slice(chunk);
                }
                FilterValue::StoredColor(ExactColorStorage { layout: m.text(encoding)?, samples: bytes, opaque_tail: tail })
            }
            _ => return Err(bad("validated field changed")),
        };
        arguments.push(FilterArgument { name: m.text(&a.name)?, value });
    }
    let state = &effect.state;
    let opacity = match (state.opacity_bits, state.opacity_u8) {
        (Some(bits), None) => f64::from(f32::from_bits(bits)),
        _ => return Err(bad("native Float opacity")),
    };
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(bad("native opacity range"));
    }
    let node = FilterNode {
        id: filter,
        operation: OperationIdentity { name: m.text(&effect.operation)?, stored_version: m.text("0:0")?, schema_epoch: m.text(identity.expected_epoch)? },
        source_display_name: Some(m.text(&effect.name)?),
        source_icon: Some(m.text(&effect.icon)?),
        arguments,
        enabled: state.enabled.ok_or_else(|| bad("enabled absent"))?,
        opacity_f64_bits: opacity.to_bits(),
        source_clipping: state.clipping,
        source_region: state.region,
        mask: mask.map(ItemRef::Mask),
        auxiliary: Vec::new(),
        composition: Some(GimpComposition {
            compatibility_epoch: m.text(identity.expected_epoch)?,
            source_mode: state.mode.ok_or_else(|| bad("mode absent"))?,
            legacy: false,
            stored_blend_space: state.blend_space.ok_or_else(|| bad("blend space absent"))?,
            stored_composite_space: state.composite_space.ok_or_else(|| bad("composite space absent"))?,
            stored_operator: state.composite_mode.ok_or_else(|| bad("operator absent"))?,
            resolved: None,
            pass_through: false,
        }),
    };
    m.check()?;
    m.step(1)?;
    let requirements = CheckerboardMappingRequirements {
        reserved_resident_bytes: limits.resident_bytes,
        reserved_index_bytes: limits.index_bytes,
        reserved_work_units: limits.work_units,
        selected_input_bytes: m.input,
        copied_bytes: m.copied,
        copied_index_bytes: m.index,
        work_units: m.work,
    };
    Ok(MappedCheckerboardSource { source, effect, identity, stored, node, requirements })
}
#[cfg(test)]
#[path = "checkerboard_source_tests.rs"]
mod tests;
