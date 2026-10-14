//! Source-only literal Colorize storage binding; not rendering or source admission.
//! Scalar type3 words and native HSL color bytes are independent retained fields.
//! Whole decoder/owner/mask backing must remain held in the enclosing admitted
//! ledger; this selected mapper never fabricates a generic pixel-owner census.
pub use super::configured_source_candidate::{CurvesMappingLimits as ColorizeMappingLimits, CurvesMappingRequirements as ColorizeMappingRequirements};
use super::configured_source_candidate::{Meter, raw_argument, word};
use crate::IoError;
use photocraft_xcf::{Argument, ArgumentValue, Drawable, Effect, RawProperty};
use photocraft_doc::{import_compositing::GimpComposition, item_identity::{FilterId, ItemRef, MaskId}, xcf_filters::{ExactColorStorage, ExactNumber, FilterArgument, FilterNode, FilterValue, OperationIdentity}};
use photocraft_raster::Interrupt;
use std::mem::size_of;
use super::stretch_hsv_source::{header, raw_state_with_arguments};

pub const COLORIZE_SCHEMA: &str = "03f514419a64c129d201b99b2853628e70a07b8947568339526d440c62c6a98c";
pub const COLORIZE_EPOCH: &str = "gimp-3.2.6-windows-03f514419a64c129";
#[derive(Clone, Copy, Debug)]
pub struct ColorizeSourceIdentity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
}
#[derive(Clone, Copy, Debug)]
pub struct ColorizeStoredFields<'a> {
    pub scalar_f32_bits: [u32; 3],
    pub layout: &'a str,
    pub samples: &'a [u8],
    pub opaque_tail: &'a [u8],
}
fn bad(s: &str) -> IoError { IoError::Unsupported(format!("retained Colorize source: {s}")) }
fn add(a: u64, b: u64) -> Result<u64, IoError> { a.checked_add(b).ok_or_else(|| bad("size overflow")) }
fn mul(a: u64, b: u64) -> Result<u64, IoError> { a.checked_mul(b).ok_or_else(|| bad("size overflow")) }
fn extent(at: usize, n: usize) -> Result<usize, IoError> { at.checked_add(n).ok_or_else(|| bad("raw field extent")) }
/// No proxy-derived reconstruction: inspect each original typed/raw field once.
fn stored_fields<'a, P>(effect: &'a Effect<P>, m: &mut Meter<'_, '_>) -> Result<ColorizeStoredFields<'a>, IoError> {
    if effect.arguments.len() != 4 { return Err(bad("four original arguments required")); }
    let mut scalars = [0u32; 3];
    let mut color = None;
    let mut seen = 0u8;
    let mut indices = [usize::MAX; 4];
    for a in &effect.arguments {
        m.step(32)?;
        if a.name.len() > 10 || a.configuration.is_some() || a.graph.is_some() { return Err(bad("closed literal argument shape")); }
        let slot = match a.name.as_str() { "hue" => 0, "saturation" => 1, "lightness" => 2, "color" => 3, _ => return Err(bad("known original argument name")) };
        let bit = 1u8 << slot;
        if seen & bit != 0 || indices.iter().any(|v| *v == a.property_index) { return Err(bad("duplicate name/raw property association")); }
        seen |= bit;
        if let Some(target) = indices.get_mut(slot) { *target = a.property_index; } else { return Err(bad("argument slot")); }
        m.input_text(&a.name)?;
        // Name scan/equality is prepaid before the shared raw name helper.
        m.step(add(mul(a.name.len() as u64, 2)?, 1)?)?;
        let (raw, end) = raw_argument(a, &effect.properties)?;
        let kind = word(raw, end)?;
        let at = extent(end, 4)?;
        if slot < 3 {
            let ArgumentValue::Float32Bits(bits) = &a.value else { return Err(bad("original type3 Float32 bits")); };
            let bits = *bits;
            if kind != 3 || extent(at, 4)? != raw.len() || word(raw, at)? != bits { return Err(bad("scalar original raw/typed word")); }
            let value = f32::from_bits(bits);
            let valid = if slot == 2 { (-1.0..=1.0).contains(&value) } else { (0.0..=1.0).contains(&value) };
            if !value.is_finite() || !valid { return Err(bad("finite scalar public range")); }
            if let Some(target) = scalars.get_mut(slot) { *target = bits; } else { return Err(bad("scalar slot")); }
        } else {
            let ArgumentValue::Color { encoding, samples, icc } = &a.value else { return Err(bad("literal nonnull stored color")); };
            if kind != 8 || encoding != "HSL float" || samples.len() != 12 || !icc.is_empty() { return Err(bad("observed HSL float12B empty-tail domain")); }
            m.input_text(encoding)?;
            m.input(samples.capacity() as u64)?;
            m.input(icc.capacity() as u64)?;
            m.step(128)?;
            let n = usize::try_from(word(raw, at)?).map_err(|_| bad("color layout length"))?;
            let layout_start = extent(at, 4)?;
            let layout_end = extent(layout_start, n)?;
            if raw.get(layout_start..layout_end).and_then(|v| v.strip_suffix(&[0])) != Some(encoding.as_bytes()) { return Err(bad("original native layout bytes")); }
            let sample_start = extent(layout_end, 4)?;
            let sample_end = extent(sample_start, 12)?;
            if word(raw, layout_end)? != 12 || raw.get(sample_start..sample_end) != Some(samples.as_slice()) || word(raw, sample_end)? != 0 || extent(sample_end, 4)? != raw.len() { return Err(bad("original native samples/tail framing")); }
            // Only known finite unit component storage; no HSL conversion or
            // asserted equality with the independently stored scalar triplet.
            for chunk in samples.chunks_exact(4) {
                m.step(8)?;
                let bytes: [u8; 4] = chunk.try_into().map_err(|_| bad("native word extent"))?;
                let value = f32::from_bits(u32::from_le_bytes(bytes));
                if !value.is_finite() || !(0.0..=1.0).contains(&value) { return Err(bad("finite unit HSL native word")); }
            }
            color = Some((encoding.as_str(), samples.as_slice(), icc.as_slice()));
        }
    }
    if seen != 15 { return Err(bad("complete singleton four fields")); }
    let (layout, samples, opaque_tail) = color.ok_or_else(|| bad("native color absent"))?;
    Ok(ColorizeStoredFields { scalar_f32_bits: scalars, layout, samples, opaque_tail })
}

/// Borrowed original owner including its entire mask; pixel backing and decoder
/// reservations stay in the caller's already-admitted enclosing lifetime.
pub struct MappedColorizeSource<'a, P> {
    source: &'a [u8],
    effect: &'a Effect<P>,
    identity: ColorizeSourceIdentity<'a>,
    stored: ColorizeStoredFields<'a>,
    node: FilterNode,
    requirements: ColorizeMappingRequirements,
}
impl<'a, P> MappedColorizeSource<'a, P> {
    pub fn original_source(&self) -> &'a [u8] { self.source }
    pub fn original_effect(&self) -> &'a Effect<P> { self.effect }
    pub fn original_mask(&self) -> Option<&'a Drawable<P>> { self.effect.mask.as_deref() }
    pub fn identity(&self) -> ColorizeSourceIdentity<'a> { self.identity }
    pub fn stored(&self) -> ColorizeStoredFields<'a> { self.stored }
    pub fn node(&self) -> &FilterNode { &self.node }
    pub fn requirements(&self) -> ColorizeMappingRequirements { self.requirements }
}
/// Caller-declared exact epoch is an association, never authentication. The
/// enclosing caller must retain/admit original decoder/owner/mask plane backing.
/// This selected metadata ledger includes source bytes, all selected backing and
/// actual canonical capacities. Both full lowered reservations/work debits occur
/// before raw/source traversal or copies, without refund/reset or cap increase.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_colorize_source<'a, P>(
    source: &'a [u8], effect: &'a Effect<P>, filter: FilterId, mask: Option<MaskId>,
    identity: ColorizeSourceIdentity<'a>, limits: ColorizeMappingLimits,
    ctl: &Interrupt<'_>, reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedColorizeSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = ColorizeMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units { return Err(bad("lower-only limits")); }
    if identity.xcf_version != 22 || identity.expected_epoch != COLORIZE_EPOCH || identity.schema_sha256 != COLORIZE_SCHEMA
        || effect.operation != "gimp:colorize" || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 4 || effect.properties.len() > 32 || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0) || mask.is_some() != effect.mask.is_some()
        || source.get(..14) != Some(b"gimp xcf v022\0") {
        return Err(bad("exact operation/epoch/schema/IDs/plain source envelope"));
    }
    for text in [&effect.name, &effect.icon, &effect.operation] { if text.len() > 65536 { return Err(bad("source text size")); } }
    // Fixed bounded metadata only before the aggregate reservation.
    if effect.properties.iter().any(|v| v.payload.len() > 1 << 20) { return Err(bad("property size")); }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m = Meter { limits, input: 0, copied: size_of::<MappedColorizeSource<'a, P>>() as u64, index: 0, work: 0, ctl };
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
    if count != 4 { return Err(bad("exact four raw argument properties")); }
    header(source, effect, &mut m)?;
    raw_state_with_arguments(source, effect, &mut m)?;
    let stored = stored_fields(effect, &mut m)?;
    let mut arguments = m.vector::<FilterArgument>(4)?;
    for a in &effect.arguments {
        m.step(16)?;
        let value = match &a.value {
            ArgumentValue::Float32Bits(bits) => FilterValue::Number(ExactNumber::Float32Bits(*bits)),
            ArgumentValue::Color { encoding, samples, icc } => {
                // Literal storage only; no inferred HSL profile/transfer/alpha.
                let mut copied = m.vector::<u8>(samples.len())?;
                for chunk in samples.chunks(4096) { m.step(chunk.len() as u64)?; copied.extend_from_slice(chunk); }
                let mut tail = m.vector::<u8>(icc.len())?;
                for chunk in icc.chunks(4096) { m.step(chunk.len() as u64)?; tail.extend_from_slice(chunk); }
                FilterValue::StoredColor(ExactColorStorage { layout: m.text(encoding)?, samples: copied, opaque_tail: tail })
            }
            _ => return Err(bad("validated literal argument unexpectedly changed")),
        };
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
    let requirements = ColorizeMappingRequirements { reserved_resident_bytes: limits.resident_bytes, reserved_index_bytes: limits.index_bytes, reserved_work_units: limits.work_units,
        selected_input_bytes: m.input, copied_bytes: m.copied, copied_index_bytes: m.index, work_units: m.work };
    Ok(MappedColorizeSource { source, effect, identity, stored, node, requirements })
}

#[cfg(test)]
mod tests {
    // UNRUN source-only synthetic drafts, not a complete GIMP/XCF oracle.
    use super::*;
    use photocraft_xcf::{Argument, EffectState, RawProperty};
    fn fixture() -> (Vec<u8>, Effect<()>) {
        let mut source = b"gimp xcf v022\0".to_vec();
        for text in ["Colorize literal", "", "gimp:colorize", "0:0"] {
            source.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
            source.extend_from_slice(text.as_bytes()); source.push(0);
        }
        let mut properties = Vec::new();
        let mut property = |tag: u32, payload: Vec<u8>| {
            source.extend_from_slice(&tag.to_be_bytes()); source.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            let offset = source.len() as u64; source.extend_from_slice(&payload);
            properties.push(RawProperty { tag, offset, declared_bytes: payload.len() as u32, payload });
        };
        for (tag, value) in [(8, 1u32), (33, 1f32.to_bits()), (7, 62), (35, 0x6e2a8251), (36, 0x6e2a8251), (37, 0x6e2a8251), (46, 0), (44, 1)] { property(tag, value.to_be_bytes().to_vec()); }
        let mut arguments = Vec::new();
        for (i, (name, bits)) in [("hue", (-0.0f32).to_bits()), ("saturation", 0.75f32.to_bits()), ("lightness", 0.25f32.to_bits())].into_iter().enumerate() {
            let mut payload = ((name.len() + 1) as u32).to_be_bytes().to_vec(); payload.extend_from_slice(name.as_bytes()); payload.push(0); payload.extend_from_slice(&3u32.to_be_bytes()); payload.extend_from_slice(&bits.to_be_bytes());
            property(45, payload); arguments.push(Argument { name: name.into(), property_index: 8 + i, value: ArgumentValue::Float32Bits(bits), configuration: None, graph: None });
        }
        // Deliberately independent of scalar fields; native words are literal.
        let samples: Vec<u8> = [0.5f32, 0.5, 0.5].into_iter().flat_map(|v| v.to_bits().to_le_bytes()).collect();
        let mut payload = 6u32.to_be_bytes().to_vec(); payload.extend_from_slice(b"color\0"); payload.extend_from_slice(&8u32.to_be_bytes()); payload.extend_from_slice(&10u32.to_be_bytes()); payload.extend_from_slice(b"HSL float\0"); payload.extend_from_slice(&12u32.to_be_bytes()); payload.extend_from_slice(&samples); payload.extend_from_slice(&0u32.to_be_bytes());
        property(45, payload); arguments.push(Argument { name: "color".into(), property_index: 11, value: ArgumentValue::Color { encoding: "HSL float".into(), samples, icc: Vec::new() }, configuration: None, graph: None });
        source.extend_from_slice(&[0; 16]);
        let effect = Effect { name: "Colorize literal".into(), icon: "".into(), operation: "gimp:colorize".into(), version: Some("0:0".into()), properties, arguments, mask: None,
            state: EffectState { enabled: Some(true), opacity_u8: None, opacity_bits: Some(1f32.to_bits()), mode: Some(62), composite_mode: Some(0x6e2a8251), composite_space: Some(0x6e2a8251), blend_space: Some(0x6e2a8251), clipping: Some(0), region: Some(1) } };
        (source, effect)
    }
    fn map<'a>(s: &'a [u8], e: &'a Effect<()>) -> Result<MappedColorizeSource<'a, ()>, IoError> {
        map_retained_colorize_source(s, e, FilterId(7), None, ColorizeSourceIdentity { xcf_version: 22, expected_epoch: COLORIZE_EPOCH, schema_sha256: COLORIZE_SCHEMA }, ColorizeMappingLimits::default(), &Interrupt::NONE, &mut |_, _| Ok(()), &mut |_| Ok(()))
    }
    #[test]
    fn independent_scalar_words_and_native_hsl_storage_survive_without_rgba_alias() {
        let (source, effect) = fixture(); let mapped = map(&source, &effect).unwrap();
        assert!(std::ptr::eq(mapped.original_effect(), &effect)); assert_eq!(mapped.original_source(), source); assert!(mapped.original_mask().is_none());
        assert_eq!(mapped.stored().scalar_f32_bits[0], (-0.0f32).to_bits());
        assert_eq!(mapped.node().arguments[0].value, FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())));
        let FilterValue::StoredColor(ExactColorStorage { layout, samples, opaque_tail }) = &mapped.node().arguments[3].value else { panic!("raw stored color missing") };
        assert_eq!(layout, "HSL float"); assert_eq!(samples.as_slice(), mapped.stored().samples); assert!(opaque_tail.is_empty());
        assert!(mapped.node().composition.as_ref().unwrap().resolved.is_none());
    }
    #[test]
    fn typed_metadata_cannot_substitute_for_original_header_scalar_or_native_bytes() {
        let (source, effect) = fixture(); let mut altered = effect.clone(); altered.name.push('x'); assert!(map(&source, &altered).is_err());
        let mut altered = effect.clone(); altered.arguments[0].value = ArgumentValue::Float32Bits(0.25f32.to_bits()); assert!(map(&source, &altered).is_err());
        let mut altered = effect.clone(); altered.arguments[1].property_index = altered.arguments[0].property_index; assert!(map(&source, &altered).is_err());
        let mut altered = effect.clone(); let ArgumentValue::Color { encoding, .. } = &mut altered.arguments[3].value else { panic!("fixture") }; *encoding = "RGBA float".into(); assert!(map(&source, &altered).is_err());
        let mut bytes = source.clone(); bytes[effect.properties[0].offset as usize + 3] = 0; assert!(map(&bytes, &effect).is_err());
    }
    #[test]
    fn aggregate_reserve_and_charge_rejection_or_cancel_publish_no_candidate() {
        let (source, effect) = fixture();
        let identity = ColorizeSourceIdentity { xcf_version: 22, expected_epoch: COLORIZE_EPOCH, schema_sha256: COLORIZE_SCHEMA };
        let mut charged = false;
        assert!(map_retained_colorize_source(&source, &effect, FilterId(7), None, identity, ColorizeMappingLimits::default(), &Interrupt::NONE, &mut |_, _| Err(bad("outer reserve")), &mut |_| { charged = true; Ok(()) }).is_err()); assert!(!charged);
        let ctl = Interrupt::cancel_only(&|| true); assert!(matches!(map_retained_colorize_source(&source, &effect, FilterId(7), None, identity, ColorizeMappingLimits::default(), &ctl, &mut |_, _| Ok(()), &mut |_| Ok(())), Err(IoError::Cancelled)));
        let mut limits = ColorizeMappingLimits::default(); limits.work_units = 1;
        assert!(map_retained_colorize_source(&source, &effect, FilterId(7), None, identity, limits, &Interrupt::NONE, &mut |_, _| Ok(()), &mut |_| Ok(())).is_err());
    }
}
