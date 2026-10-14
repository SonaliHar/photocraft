//! Exact retained Plain Stretch Contrast source storage, not HSV or math admission.
//! Both Boolean fields remain independent, including keep-colors=true/default.
//! The existing false/false kernel is a separate explicitly prepared consumer;
//! this mapper does not qualify configuration, profile, precision, mask or outer
//! rendering. Whole decoder/owner/mask backing must remain held/admitted by the
//! enclosing caller; this selected metadata ledger is not a generic P census.
pub use super::configured_source_candidate::{CurvesMappingLimits as StretchContrastMappingLimits, CurvesMappingRequirements as StretchContrastMappingRequirements};
use super::configured_source_candidate::{Meter, raw_argument, word};
use crate::IoError;
use photocraft_xcf::{Argument, ArgumentValue, Drawable, Effect, RawProperty};
use photocraft_doc::{import_compositing::GimpComposition, item_identity::{FilterId, ItemRef, MaskId}, xcf_filters::{FilterArgument, FilterNode, FilterValue, OperationIdentity}};
use photocraft_raster::Interrupt;
use std::mem::size_of;
use super::stretch_hsv_source::{header, raw_state_with_arguments};

pub const STRETCH_CONTRAST_SCHEMA: &str = "22652a0a27c020d4792df713e3acb546953954d91663a63fb0057fde53dec838";
pub const STRETCH_CONTRAST_EPOCH: &str = "gimp-3.2.6-windows-22652a0a27c020d4";
#[derive(Clone, Copy, Debug)]
pub struct StretchContrastSourceIdentity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
}
#[derive(Clone, Copy, Debug)]
pub struct StretchContrastStoredFields {
    pub keep_colors: bool,
    pub perceptual: bool,
}
fn bad(s: &str) -> IoError { IoError::Unsupported(format!("retained StretchContrast source: {s}")) }
fn add(a: u64, b: u64) -> Result<u64, IoError> { a.checked_add(b).ok_or_else(|| bad("size overflow")) }
fn mul(a: u64, b: u64) -> Result<u64, IoError> { a.checked_mul(b).ok_or_else(|| bad("size overflow")) }
fn extent(at: usize, n: usize) -> Result<usize, IoError> { at.checked_add(n).ok_or_else(|| bad("raw field extent")) }
/// Original two raw type2 words, with no defaults or kernel policy substitution.
fn stored_fields<P>(effect: &Effect<P>, m: &mut Meter<'_, '_>) -> Result<StretchContrastStoredFields, IoError> {
    if effect.arguments.len() != 2 { return Err(bad("two original arguments required")); }
    let mut keep_colors = None;
    let mut perceptual = None;
    let mut first_index = None;
    for a in &effect.arguments {
        m.step(32)?;
        if a.name.len() > 11 || a.configuration.is_some() || a.graph.is_some() { return Err(bad("closed Boolean argument shape")); }
        let destination = match a.name.as_str() { "keep-colors" => &mut keep_colors, "perceptual" => &mut perceptual, _ => return Err(bad("known original Boolean name")) };
        if destination.is_some() || first_index == Some(a.property_index) { return Err(bad("duplicate field/raw property association")); }
        first_index = Some(a.property_index);
        m.input_text(&a.name)?;
        // Prepay bounded name comparison before shared raw framing helper.
        m.step(add(mul(a.name.len() as u64, 2)?, 1)?)?;
        let (raw, end) = raw_argument(a, &effect.properties)?;
        let ArgumentValue::Boolean(value) = &a.value else { return Err(bad("original type2 Boolean")); };
        let at = extent(end, 4)?;
        if word(raw, end)? != 2 || extent(at, 4)? != raw.len() || word(raw, at)? != u32::from(*value) { return Err(bad("exact original Boolean raw/typed word")); }
        *destination = Some(*value);
    }
    Ok(StretchContrastStoredFields { keep_colors: keep_colors.ok_or_else(|| bad("keep-colors absent"))?, perceptual: perceptual.ok_or_else(|| bad("perceptual absent"))? })
}

/// Borrowed original owner including its entire mask; pixel backing and decoder
/// reservations stay in the caller's already-admitted enclosing lifetime.
pub struct MappedStretchContrastSource<'a, P> {
    source: &'a [u8],
    effect: &'a Effect<P>,
    identity: StretchContrastSourceIdentity<'a>,
    stored: StretchContrastStoredFields,
    node: FilterNode,
    requirements: StretchContrastMappingRequirements,
}
impl<'a, P> MappedStretchContrastSource<'a, P> {
    pub fn original_source(&self) -> &'a [u8] { self.source }
    pub fn original_effect(&self) -> &'a Effect<P> { self.effect }
    pub fn original_mask(&self) -> Option<&'a Drawable<P>> { self.effect.mask.as_deref() }
    pub fn identity(&self) -> StretchContrastSourceIdentity<'a> { self.identity }
    pub fn stored(&self) -> StretchContrastStoredFields { self.stored }
    pub fn node(&self) -> &FilterNode { &self.node }
    pub fn requirements(&self) -> StretchContrastMappingRequirements { self.requirements }
}
/// Caller-declared exact epoch is an association, never authentication. The
/// enclosing caller must retain/admit original decoder/owner/mask plane backing.
/// This selected metadata ledger includes source bytes, all selected backing and
/// actual canonical capacities. Both full lowered reservations/work debits occur
/// before raw/source traversal or copies, without refund/reset or cap increase.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_stretch_contrast_source<'a, P>(
    source: &'a [u8], effect: &'a Effect<P>, filter: FilterId, mask: Option<MaskId>,
    identity: StretchContrastSourceIdentity<'a>, limits: StretchContrastMappingLimits,
    ctl: &Interrupt<'_>, reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedStretchContrastSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = StretchContrastMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units { return Err(bad("lower-only limits")); }
    if identity.xcf_version != 22 || identity.expected_epoch != STRETCH_CONTRAST_EPOCH || identity.schema_sha256 != STRETCH_CONTRAST_SCHEMA
        || effect.operation != "gegl:stretch-contrast" || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 2 || effect.properties.len() > 32 || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0) || mask.is_some() != effect.mask.is_some()
        || source.get(..14) != Some(b"gimp xcf v022\0") {
        return Err(bad("exact operation/epoch/schema/IDs/plain source envelope"));
    }
    for text in [&effect.name, &effect.icon, &effect.operation] { if text.len() > 65536 { return Err(bad("source text size")); } }
    // Fixed bounded metadata only before the aggregate reservation.
    if effect.properties.iter().any(|v| v.payload.len() > 1 << 20) { return Err(bad("property size")); }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m = Meter { limits, input: 0, copied: size_of::<MappedStretchContrastSource<'a, P>>() as u64, index: 0, work: 0, ctl };
    m.check()?;
    m.input(source.len() as u64)?;
    m.input(size_of::<Effect<P>>() as u64)?;
    m.input(identity.expected_epoch.len() as u64)?;
    for text in [&effect.name, &effect.icon, &effect.operation] {
        m.input_text(text)?;
        for chunk in text.as_bytes().chunks(4096) { m.step(chunk.len() as u64)?; if chunk.contains(&0) { return Err(bad("source text NUL")); } }
    }
    if let Some(version) = &effect.version { m.input_text(version)?; }
    m.input(mul(effect.properties.capacity() as u64, size_of::<RawProperty>() as u64)?)?;
    m.input(mul(effect.arguments.capacity() as u64, size_of::<Argument>() as u64)?)?;
    let mut count = 0usize;
    for prop in &effect.properties { m.input(prop.payload.capacity() as u64)?; m.step(1)?; if prop.tag == 45 { count = count.checked_add(1).ok_or_else(|| bad("argument count"))?; } }
    if count != 2 { return Err(bad("exact two raw argument properties")); }
    header(source, effect, &mut m)?;
    raw_state_with_arguments(source, effect, &mut m)?;
    let stored = stored_fields(effect, &mut m)?;
    let mut arguments = m.vector::<FilterArgument>(2)?;
    for a in &effect.arguments {
        m.step(16)?;
        let ArgumentValue::Boolean(value) = &a.value else { return Err(bad("validated Boolean changed")); };
        let value = FilterValue::Bool(*value);
        arguments.push(FilterArgument { name: m.text(&a.name)?, value });
    }
    let state = &effect.state;
    let opacity = match (state.opacity_bits, state.opacity_u8) { (Some(v), None) => f64::from(f32::from_bits(v)), _ => return Err(bad("native Float32 opacity")) };
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) { return Err(bad("native opacity range")); }
    let node = FilterNode {
        id: filter,
        operation: OperationIdentity { name: m.text(&effect.operation)?, stored_version: m.text("0:0")?, schema_epoch: m.text(identity.expected_epoch)? },
        source_display_name: Some(m.text(&effect.name)?), source_icon: Some(m.text(&effect.icon)?), arguments,
        enabled: state.enabled.ok_or_else(|| bad("enabled absent"))?, opacity_f64_bits: opacity.to_bits(),
        source_clipping: state.clipping, source_region: state.region, mask: mask.map(ItemRef::Mask), auxiliary: Vec::new(),
        composition: Some(GimpComposition { compatibility_epoch: m.text(identity.expected_epoch)?, source_mode: state.mode.ok_or_else(|| bad("mode absent"))?, legacy: false,
            stored_blend_space: state.blend_space.ok_or_else(|| bad("blend space absent"))?, stored_composite_space: state.composite_space.ok_or_else(|| bad("composite space absent"))?,
            stored_operator: state.composite_mode.ok_or_else(|| bad("operator absent"))?, resolved: None, pass_through: false }),
    };
    m.check()?; m.step(1)?;
    let requirements = StretchContrastMappingRequirements { reserved_resident_bytes: limits.resident_bytes, reserved_index_bytes: limits.index_bytes, reserved_work_units: limits.work_units,
        selected_input_bytes: m.input, copied_bytes: m.copied, copied_index_bytes: m.index, work_units: m.work };
    Ok(MappedStretchContrastSource { source, effect, identity, stored, node, requirements })
}


#[cfg(test)]
#[path = "stretch_contrast_source_tests.rs"]
mod tests;
