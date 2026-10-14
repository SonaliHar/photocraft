//! Exact zero-argument retained Stretch Contrast HSV storage binding.
//! Fixed schema/explicit epoch associates a caller-declared typed source, not
//! independent provenance/stock authorization or numerical/render admission.
//! Original source/effect/raw opacity/opaque composition words stay borrowed;
//! no whole decoded scene, provider pixels or mask is cloned or interpreted.
//! The enclosing owner must hold/admit ALL original decoder/effect/mask/provider
//! backing beyond this selected metadata ledger (including spare containers).
pub use super::configured_source_candidate::{CurvesMappingLimits as StretchHsvMappingLimits, CurvesMappingRequirements as StretchHsvMappingRequirements};
use super::configured_source_candidate::{Meter, word};
use crate::IoError;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{FilterNode, OperationIdentity},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, Effect, EffectState, RawProperty};
use std::mem::size_of;
pub const STRETCH_HSV_SCHEMA: &str = "862de99c547cf3bbb25f77204bb93dcd1becf318ad65cce1d645bdafd7bd1a85";
#[derive(Clone, Copy, Debug)]
pub struct StretchHsvSourceIdentity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
}
pub struct MappedStretchHsvSource<'a, P> {
    source: &'a [u8],
    effect: &'a Effect<P>,
    identity: StretchHsvSourceIdentity<'a>,
    node: FilterNode,
    requirements: StretchHsvMappingRequirements,
}
impl<'a, P> MappedStretchHsvSource<'a, P> {
    pub fn original_source(&self) -> &'a [u8] {
        self.source
    }
    pub fn original_effect(&self) -> &'a Effect<P> {
        self.effect
    }
    pub fn identity(&self) -> StretchHsvSourceIdentity<'a> {
        self.identity
    }
    pub fn node(&self) -> &FilterNode {
        &self.node
    }
    pub fn requirements(&self) -> StretchHsvMappingRequirements {
        self.requirements
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("retained Stretch HSV source: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
fn offset(v: u64) -> Result<usize, IoError> {
    usize::try_from(v).map_err(|_| bad("source offset"))
}
/// Read all four length-framed strings against an already bounded candidate
/// header. Empty source strings may use either legitimate XCF0 or1 framing.
fn header_at<P>(source: &[u8], start: usize, end: usize, e: &Effect<P>, m: &mut Meter<'_, '_>) -> Result<bool, IoError> {
    let mut at = start;
    for expected in [e.name.as_str(), e.icon.as_str(), e.operation.as_str(), e.version.as_deref().ok_or_else(|| bad("version absent"))?] {
        m.step(16)?;
        let Some(prefix) = source.get(at..at.checked_add(4).ok_or_else(|| bad("header extent"))?) else { return Ok(false) };
        let n = u32::from_be_bytes(prefix.try_into().map_err(|_| bad("header width"))?) as usize;
        at = at.checked_add(4).ok_or_else(|| bad("header extent"))?;
        if n != expected.len().checked_add(1).ok_or_else(|| bad("header length"))? && !(expected.is_empty() && n == 0) {
            return Ok(false);
        }
        let stop = at.checked_add(n).ok_or_else(|| bad("header extent"))?;
        if stop > end {
            return Ok(false);
        }
        let Some(raw) = source.get(at..stop) else { return Ok(false) };
        if n != 0 {
            let Some(content) = raw.strip_suffix(&[0]) else { return Ok(false) };
            for (left, right) in content.chunks(4096).zip(expected.as_bytes().chunks(4096)) {
                m.step(mul(left.len() as u64, 2)?)?;
                if left != right {
                    return Ok(false);
                }
            }
        }
        at = stop;
    }
    Ok(at == end)
}
fn header<P>(source: &[u8], e: &Effect<P>, m: &mut Meter<'_, '_>) -> Result<(), IoError> {
    let first = e.properties.first().ok_or_else(|| bad("state properties absent"))?;
    let end = offset(first.offset)?.checked_sub(8).ok_or_else(|| bad("property header"))?;
    let mut minimum = 0usize;
    let mut empty = 0usize;
    for text in [e.name.as_str(), e.icon.as_str(), e.operation.as_str(), e.version.as_deref().ok_or_else(|| bad("version absent"))?] {
        minimum = minimum
            .checked_add(4)
            .and_then(|v| v.checked_add(text.len()))
            .and_then(|v| v.checked_add(usize::from(!text.is_empty())))
            .ok_or_else(|| bad("header length"))?;
        if text.is_empty() {
            empty = empty.checked_add(1).ok_or_else(|| bad("header length"))?;
        }
    }
    // At most five actual byte positions: bounded zero/NUL empty-string ambiguity.
    for padding in 0..=empty {
        m.step(1)?;
        let Some(start) = end.checked_sub(minimum.checked_add(padding).ok_or_else(|| bad("header length"))?) else { continue };
        if header_at(source, start, end, e, m)? {
            return Ok(());
        }
    }
    Err(bad("raw effect header/name/icon/operation/version mismatch"))
}
fn raw_state<P>(source: &[u8], e: &Effect<P>, m: &mut Meter<'_, '_>) -> Result<(), IoError> {
    let mut at = offset(e.properties.first().ok_or_else(|| bad("properties absent"))?.offset)?.checked_sub(8).ok_or_else(|| bad("property header"))?;
    let mut seen = 0u16;
    let EffectState { enabled, opacity_u8, opacity_bits, mode, composite_mode, composite_space, blend_space, clipping, region } = &e.state;
    if opacity_u8.is_some() {
        return Err(bad("unexpected byte opacity"));
    }
    for p in &e.properties {
        m.step(24)?;
        if p.tag == 0 || p.tag == 45 || p.tag == 6 {
            return Err(bad("zero-argument/native opacity properties required"));
        }
        let payload = at.checked_add(8).ok_or_else(|| bad("property extent"))?;
        if p.offset != payload as u64
            || word(source, at)? != p.tag
            || word(source, at.checked_add(4).ok_or_else(|| bad("property extent"))?)? != p.declared_bytes
            || p.declared_bytes as usize != p.payload.len()
        {
            return Err(bad("contiguous original property framing"));
        }
        let end = payload.checked_add(p.payload.len()).ok_or_else(|| bad("property extent"))?;
        let actual = source.get(payload..end).ok_or_else(|| bad("property source extent"))?;
        for (left, right) in actual.chunks(4096).zip(p.payload.chunks(4096)) {
            m.step(mul(left.len() as u64, 2)?)?;
            if left != right {
                return Err(bad("original raw property mismatch"));
            }
        }
        let known = match p.tag {
            8 => Some((1u16, enabled.map(u32::from))),
            33 => Some((2, *opacity_bits)),
            7 => Some((4, mode.map(|v| v as u32))),
            35 => Some((8, composite_mode.map(|v| v as u32))),
            36 => Some((16, composite_space.map(|v| v as u32))),
            37 => Some((32, blend_space.map(|v| v as u32))),
            46 => Some((64, *clipping)),
            44 => Some((128, *region)),
            _ => None,
        };
        if let Some((bit, value)) = known {
            if seen & bit != 0 || p.payload.len() != 4 || Some(word(&p.payload, 0)?) != value {
                return Err(bad("exact state/native-word association"));
            }
            seen |= bit;
        }
        at = end;
    }
    m.step(24)?;
    if seen != 255 || word(source, at)? != 0 || word(source, at.checked_add(4).ok_or_else(|| bad("END extent"))?)? != 0 {
        return Err(bad("complete singleton state and original END"));
    }
    let pointer = at.checked_add(8).ok_or_else(|| bad("mask pointer extent"))?;
    m.step(16)?;
    let bytes = source.get(pointer..pointer.checked_add(8).ok_or_else(|| bad("mask pointer extent"))?).ok_or_else(|| bad("mask pointer extent"))?;
    let actual = u64::from_be_bytes(bytes.try_into().map_err(|_| bad("mask pointer"))?);
    if actual != e.mask.as_ref().map_or(0, |mask| mask.record_offset) || e.mask.as_ref().is_some_and(|mask| mask.record_offset == 0) {
        return Err(bad("original mask pointer association"));
    }
    Ok(())
}
/// Full lowered callback reserve/work receipt precedes header/property scans and
/// any canonical text copy. The source schema/epoch remains explicitly asserted
/// by a separately retained caller; no hash whitelist, evaluator or grants exist.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_stretch_hsv_source<'a, P>(
    source: &'a [u8],
    effect: &'a Effect<P>,
    filter: FilterId,
    mask: Option<MaskId>,
    identity: StretchHsvSourceIdentity<'a>,
    limits: StretchHsvMappingLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedStretchHsvSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = StretchHsvMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(bad("lower-only limits"));
    }
    if identity.xcf_version != 22
        || identity.schema_sha256 != STRETCH_HSV_SCHEMA
        || identity.expected_epoch.is_empty()
        || identity.expected_epoch.len() > 128
        || identity.expected_epoch.contains('\0')
        || effect.operation != "gegl:stretch-contrast-hsv"
        || effect.version.as_deref() != Some("0:0")
        || !effect.arguments.is_empty()
        || effect.properties.len() > 32
        || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0)
        || mask.is_some() != effect.mask.is_some()
        || source.get(..14) != Some(b"gimp xcf v022\0")
    {
        return Err(bad("exact zero-argument source/identity/canonical owners"));
    }
    for text in [&effect.name, &effect.icon, &effect.operation] {
        if text.len() > 65536 {
            return Err(bad("bounded source text"));
        }
    }
    if effect.properties.iter().any(|p| p.payload.len() > 1 << 20) {
        return Err(bad("property size"));
    }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m = Meter { limits, input: 0, copied: 0, index: 0, work: 0, ctl };
    m.input(source.len() as u64)?;
    m.input(size_of::<Effect<P>>() as u64)?;
    m.input(identity.expected_epoch.len() as u64)?;
    for text in [&effect.name, &effect.icon, &effect.operation] {
        m.input_text(text)?;
        for chunk in text.as_bytes().chunks(4096) {
            m.step(chunk.len() as u64)?;
            if chunk.contains(&0) {
                return Err(bad("embedded source text NUL"));
            }
        }
    }
    if let Some(version) = &effect.version {
        m.input_text(version)?;
    }
    m.input(mul(effect.properties.capacity() as u64, size_of::<RawProperty>() as u64)?)?;
    m.input(mul(effect.arguments.capacity() as u64, size_of::<Argument>() as u64)?)?;
    for property in &effect.properties {
        m.input(property.payload.capacity() as u64)?;
    }
    header(source, effect, &mut m)?;
    raw_state(source, effect, &mut m)?;
    let state = &effect.state;
    let opacity = f64::from(f32::from_bits(state.opacity_bits.ok_or_else(|| bad("opacity absent"))?));
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(bad("native opacity range"));
    }
    m.copied = add(m.copied, size_of::<MappedStretchHsvSource<'a, P>>() as u64)?;
    m.check()?;
    let node = FilterNode {
        id: filter,
        operation: OperationIdentity { name: m.text(&effect.operation)?, stored_version: m.text("0:0")?, schema_epoch: m.text(identity.expected_epoch)? },
        source_display_name: Some(m.text(&effect.name)?),
        source_icon: Some(m.text(&effect.icon)?),
        arguments: Vec::new(),
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
    let requirements = StretchHsvMappingRequirements {
        reserved_resident_bytes: limits.resident_bytes,
        reserved_index_bytes: limits.index_bytes,
        reserved_work_units: limits.work_units,
        selected_input_bytes: m.input,
        copied_bytes: m.copied,
        copied_index_bytes: m.index,
        work_units: m.work,
    };
    Ok(MappedStretchHsvSource { source, effect, identity, node, requirements })
}
#[cfg(test)]
#[path = "stretch_hsv_source_tests.rs"]
mod tests;
