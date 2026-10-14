//! Explicit retained Levels source mapping, not qualified_stack admission.
//! Complete source/decoder/mask owners stay admitted by the enclosing caller.
pub use super::configured_source_candidate::{CurvesMappingLimits as LevelsMappingLimits, CurvesMappingRequirements as LevelsMappingRequirements};
use super::configured_source_candidate::{Meter, raw_argument, word};
use crate::IoError;
use photocraft_compose::gimp_configured_candidate::{LEVELS_SCHEMA, RetainedSourceIdentity};
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::*,
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, Effect, EffectConfiguration, LevelsConfiguration, RawProperty, TypedConfiguration};
use std::mem::size_of;

pub struct MappedLevelsSource<'a, P> {
    original: &'a Effect<P>,
    configuration: &'a EffectConfiguration,
    source: &'a [u8],
    identity: RetainedSourceIdentity<'a>,
    node: FilterNode,
    requirements: LevelsMappingRequirements,
}
impl<'a, P> MappedLevelsSource<'a, P> {
    pub fn original_effect(&self) -> &'a Effect<P> {
        self.original
    }
    pub fn original_configuration(&self) -> &'a EffectConfiguration {
        self.configuration
    }
    pub fn original_source(&self) -> &'a [u8] {
        self.source
    }
    pub fn node(&self) -> &FilterNode {
        &self.node
    }
    pub fn identity(&self) -> RetainedSourceIdentity<'a> {
        self.identity
    }
    pub fn requirements(&self) -> LevelsMappingRequirements {
        self.requirements
    }
    /// Uses unchanged configured/CMS-domain validation with a distinct lower quota.
    pub fn prepare_retained_plan(
        &self,
        limits: photocraft_algo::gimp::configured::Limits,
        ctl: &Interrupt<'_>,
    ) -> Result<photocraft_compose::gimp_configured_candidate::RetainedSourcePlan<'_>, photocraft_raster::RasterInputError> {
        photocraft_compose::gimp_configured_candidate::prepare_retained_source_plan(&self.node, self.identity, limits, ctl)
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("retained Levels candidate: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
fn transfer(config: &LevelsConfiguration, tag: i32) -> Result<(), IoError> {
    let name = match tag {
        0 => "linear",
        1 => "non-linear",
        2 => "perceptual",
        _ => return Err(bad("TRC enum")),
    };
    if config.trc.as_deref() != Some(name) {
        return Err(bad("outer/config TRC mismatch"));
    }
    Ok(())
}
fn bits(c: &photocraft_xcf::LevelsChannel) -> [u64; 5] {
    [c.low_input_bits, c.high_input_bits, c.gamma_bits, c.low_output_bits, c.high_output_bits]
}
fn validate_numeric(config: &LevelsConfiguration, m: &mut Meter<'_, '_>) -> Result<(), IoError> {
    use photocraft_algo::gimp::levels::{LevelsChannel, LevelsConfiguration as Numeric};
    let mut channels = [LevelsChannel::default(); 5];
    for (c, dst) in config.channels.iter().zip(channels.iter_mut()) {
        m.step(1)?;
        let [low_input, high_input, gamma, low_output, high_output] = bits(c).map(f64::from_bits);
        *dst = LevelsChannel { low_input, high_input, gamma, low_output, high_output };
    }
    Numeric {
        channels,
        clamp_input: config.clamp_input.ok_or_else(|| bad("clamp-input absent"))?,
        clamp_output: config.clamp_output.ok_or_else(|| bad("clamp-output absent"))?,
    }
    .validate()
    .map_err(|e| bad(&e.to_string()))
}
/// Exactly two stored arguments become the existing consumer's four canonical
/// arguments. Clamps come from the retained typed config, never source defaults.
/// Raw property association does not prove a caller-mutated AST independently.
/// Both callbacks debit one enclosing ledger before traversal or private copies.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_levels_source<'a, P>(
    source: &'a [u8],
    effect: &'a Effect<P>,
    filter: FilterId,
    mask: Option<MaskId>,
    identity: RetainedSourceIdentity<'a>,
    limits: LevelsMappingLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedLevelsSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = LevelsMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(bad("lower-only limits"));
    }
    if identity.xcf_version != 22
        || identity.schema_sha256 != LEVELS_SCHEMA
        || identity.expected_epoch.is_empty()
        || identity.expected_epoch.len() > 128
        || identity.expected_epoch.contains('\0')
        || !matches!(identity.color_channels, 1 | 3)
        || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0)
        || effect.mask.is_some() != mask.is_some()
        || effect.operation != "gimp:levels"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 2
        || effect.properties.len() > 32
        || !photocraft_xcf::is_xcf(source)
    {
        return Err(bad("operation/source/schema/ID binding"));
    }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m = Meter { limits, input: 0, copied: add(size_of::<FilterNode>() as u64, size_of::<MappedLevelsSource<'a, P>>() as u64)?, index: 0, work: 0, ctl };
    m.check()?;
    m.input(size_of::<Effect<P>>() as u64)?;
    for v in [&effect.name, &effect.icon, &effect.operation] {
        m.input_text(v)?;
    }
    if let Some(v) = &effect.version {
        m.input_text(v)?;
    }
    m.input(mul(effect.properties.capacity() as u64, size_of::<RawProperty>() as u64)?)?;
    for p in &effect.properties {
        m.input(p.payload.capacity() as u64)?;
        if p.payload.len() > 1 << 20 || usize::try_from(p.declared_bytes).ok() != Some(p.payload.len()) {
            return Err(bad("raw property bound"));
        }
        let start = usize::try_from(p.offset).map_err(|_| bad("source offset"))?;
        let end = start.checked_add(p.payload.len()).ok_or_else(|| bad("source extent"))?;
        let actual = source.get(start..end).ok_or_else(|| bad("source property extent"))?;
        for (a, b) in actual.chunks(4096).zip(p.payload.chunks(4096)) {
            m.step(mul(a.len() as u64, 2)?)?;
            if a != b {
                return Err(bad("raw/source property mismatch"));
            }
        }
    }
    m.input(mul(effect.arguments.capacity() as u64, size_of::<Argument>() as u64)?)?;
    let mut trc = None;
    let mut original = None;
    for a in &effect.arguments {
        if !matches!(a.name.as_str(), "trc" | "config") {
            return Err(bad("known bounded argument name required"));
        }
        m.input_text(&a.name)?;
        if a.graph.is_some() {
            return Err(bad("nested graph"));
        }
        m.step(add(mul(a.name.len() as u64, 2)?, 1)?)?;
        let (raw, end) = raw_argument(a, &effect.properties)?;
        match (a.name.as_str(), &a.value) {
            ("trc", ArgumentValue::Enum(tag)) if a.configuration.is_none() && trc.is_none() => {
                if word(raw, end)? != 5
                    || word(raw, end.checked_add(4).ok_or_else(|| bad("enum offset"))?)? != *tag as u32
                    || raw.len() != end.checked_add(8).ok_or_else(|| bad("enum extent"))?
                {
                    return Err(bad("raw TRC enum"));
                }
                trc = Some(*tag);
            }
            ("config", ArgumentValue::Config(text)) if original.is_none() => {
                m.input_text(text)?;
                if word(raw, end)? != 6 {
                    return Err(bad("raw config kind"));
                }
                let length_at = end.checked_add(4).ok_or_else(|| bad("config offset"))?;
                let len = usize::try_from(word(raw, length_at)?).map_err(|_| bad("config width"))?;
                let start = length_at.checked_add(4).ok_or_else(|| bad("config start"))?;
                let stop = start.checked_add(len).ok_or_else(|| bad("config extent"))?;
                // This is a second comparison, distinct from source-vs-property
                // above. Debit its full text/terminator traversal before reading.
                m.step(add(mul(text.len() as u64, 2)?, 1)?)?;
                if stop != raw.len() || raw.get(start..stop).and_then(|v| v.strip_suffix(&[0])) != Some(text.as_bytes()) {
                    return Err(bad("raw configuration text"));
                }
                let config = a.configuration.as_ref().ok_or_else(|| bad("parsed configuration absent"))?;
                m.input(size_of::<EffectConfiguration>() as u64)?;
                m.ast(&config.nodes, 0, &mut 0, text.len() as u64)?;
                original = Some(config);
            }
            _ => return Err(bad("exact Levels argument envelope")),
        }
    }
    let tag = trc.ok_or_else(|| bad("TRC missing"))?;
    let original = original.ok_or_else(|| bad("config missing"))?;
    let Some(TypedConfiguration::Levels(config)) = &original.typed else {
        return Err(bad("typed Levels absent"));
    };
    if config.time_bits != Some(0) || config.channels.len() != 5 {
        return Err(bad("configuration time/channel shape"));
    }
    transfer(config, tag)?;
    m.input(size_of::<LevelsConfiguration>() as u64)?;
    if let Some(v) = &config.trc {
        m.input_text(v)?;
    }
    m.input(mul(config.channels.capacity() as u64, size_of::<photocraft_xcf::LevelsChannel>() as u64)?)?;
    for (c, name) in config.channels.iter().zip(["value", "red", "green", "blue", "alpha"]) {
        m.step(1)?;
        if c.channel != name {
            return Err(bad("channel order"));
        }
        m.input_text(&c.channel)?;
    }
    // Reuse the exact existing finite/range/gamma guard, not new unit clamping.
    validate_numeric(config, &mut m)?;
    let mut channels = m.vector(5)?;
    for c in &config.channels {
        let mut fields = m.vector(5)?;
        for (name, value) in ["low-input", "high-input", "gamma", "low-output", "high-output"].into_iter().zip(bits(c)) {
            m.step(1)?;
            fields.push(FilterArgument { name: m.text(name)?, value: FilterValue::Number(ExactNumber::Float64Bits(value)) });
        }
        channels.push(FilterArgument { name: m.text(&c.channel)?, value: FilterValue::Config(fields) });
    }
    let mut arguments = m.vector(4)?;
    arguments.push(FilterArgument { name: m.text("trc")?, value: FilterValue::Enum { domain: m.text("gimp:levels::trc")?, value: tag } });
    arguments.push(FilterArgument { name: m.text("clamp-input")?, value: FilterValue::Bool(config.clamp_input.ok_or_else(|| bad("clamp-input absent"))?) });
    arguments.push(FilterArgument { name: m.text("clamp-output")?, value: FilterValue::Bool(config.clamp_output.ok_or_else(|| bad("clamp-output absent"))?) });
    arguments.push(FilterArgument { name: m.text("config")?, value: FilterValue::Config(channels) });
    let state = &effect.state;
    let opacity = match (state.opacity_bits, state.opacity_u8) {
        (Some(v), None) => f64::from(f32::from_bits(v)),
        _ => return Err(bad("explicit retained Float32 opacity required")),
    };
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(bad("opacity"));
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
    let requirements = LevelsMappingRequirements {
        reserved_resident_bytes: limits.resident_bytes,
        reserved_index_bytes: limits.index_bytes,
        reserved_work_units: limits.work_units,
        selected_input_bytes: m.input,
        copied_bytes: m.copied,
        copied_index_bytes: m.index,
        work_units: m.work,
    };
    Ok(MappedLevelsSource { original: effect, configuration: original, source, identity, node, requirements })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn channel() -> photocraft_xcf::LevelsChannel {
        photocraft_xcf::LevelsChannel {
            channel: "value".into(),
            low_input_bits: (-0f64).to_bits(),
            high_input_bits: 2f64.to_bits(),
            gamma_bits: 1f64.to_bits(),
            low_output_bits: (-2f64).to_bits(),
            high_output_bits: 4f64.to_bits(),
        }
    }
    #[test]
    fn native_hdr_signed_zero_and_clamps_are_not_unit_normalized() {
        let c = channel();
        let exact = bits(&c);
        assert_eq!(exact.first(), Some(&(-0f64).to_bits()));
        let config = LevelsConfiguration {
            time_bits: Some(0),
            trc: Some("non-linear".into()),
            clamp_input: Some(false),
            clamp_output: Some(true),
            channels: vec![c; 5],
        };
        let mut m = Meter { limits: LevelsMappingLimits::default(), input: 0, copied: 0, index: 0, work: 0, ctl: &Interrupt::NONE };
        assert!(validate_numeric(&config, &mut m).is_ok());
        assert_eq!(bits(config.channels.first().unwrap()), exact);
        assert!(transfer(&config, 0).is_err());
    }
    #[test]
    fn gamma_missing_clamp_and_cancel_are_named_refusals() {
        let mut config =
            LevelsConfiguration { time_bits: Some(0), trc: Some("linear".into()), clamp_input: None, clamp_output: Some(false), channels: vec![channel(); 5] };
        let mut m = Meter { limits: LevelsMappingLimits::default(), input: 0, copied: 0, index: 0, work: 0, ctl: &Interrupt::NONE };
        assert!(validate_numeric(&config, &mut m).is_err());
        config.clamp_input = Some(false);
        config.channels.first_mut().unwrap().gamma_bits = 0.01f64.to_bits();
        assert!(validate_numeric(&config, &mut m).is_err());
        let stop = || true;
        let ctl = Interrupt::cancel_only(&stop);
        m.ctl = &ctl;
        assert!(matches!(validate_numeric(&config, &mut m), Err(IoError::Cancelled)));
    }
}
