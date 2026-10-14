//! Exact GIMP wrapper storage candidate, not a GEGL numerical alias.
//! Working-stage brightness/contrast math and source admission remain unbound.
pub use super::configured_source_candidate::{
    CurvesMappingLimits as BrightnessContrastMappingLimits, CurvesMappingRequirements as BrightnessContrastMappingRequirements,
};
use super::configured_source_candidate::{Meter, raw_argument, word};
use crate::IoError;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::*,
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, BrightnessContrastConfiguration, ConfigValue, Effect, EffectConfiguration, RawProperty, TypedConfiguration};
use std::mem::size_of;
pub const BRIGHTNESS_CONTRAST_SCHEMA: &str = "27409cfe3d3be690580fb52450cf72d17e07c65aecc41c86d20685696f6c7ae1";
pub const BRIGHTNESS_CONTRAST_SCHEMA_308: &str = "f721196221bd240a6c539f67498831ecdfd5550c99315bc830be445427129fb5";
#[derive(Clone, Copy, Debug)]
pub struct BrightnessContrastSourceIdentity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
}
pub struct MappedBrightnessContrastSource<'a, P> {
    source: &'a [u8],
    effect: &'a Effect<P>,
    configuration: &'a EffectConfiguration,
    stored: &'a BrightnessContrastConfiguration,
    identity: BrightnessContrastSourceIdentity<'a>,
    node: FilterNode,
    requirements: BrightnessContrastMappingRequirements,
}
impl<'a, P> MappedBrightnessContrastSource<'a, P> {
    pub fn original_source(&self) -> &'a [u8] {
        self.source
    }
    pub fn original_effect(&self) -> &'a Effect<P> {
        self.effect
    }
    pub fn original_configuration(&self) -> &'a EffectConfiguration {
        self.configuration
    }
    pub fn stored(&self) -> &'a BrightnessContrastConfiguration {
        self.stored
    }
    pub fn identity(&self) -> BrightnessContrastSourceIdentity<'a> {
        self.identity
    }
    pub fn node(&self) -> &FilterNode {
        &self.node
    }
    /// Move the already-admitted node without another deep configuration copy.
    pub(super) fn into_node(self) -> FilterNode {
        self.node
    }
    pub fn requirements(&self) -> BrightnessContrastMappingRequirements {
        self.requirements
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("GIMP Brightness Contrast source: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
fn slots(config: &BrightnessContrastConfiguration) -> Result<[u64; 3], IoError> {
    let time = config.time_bits.ok_or_else(|| bad("explicit time absent"))?;
    if time != 0
        || [config.brightness_bits, config.contrast_bits].into_iter().any(|v| {
            let n = f64::from_bits(v);
            !n.is_finite() || !(-1.0..=1.0).contains(&n)
        })
    {
        return Err(bad("constant time0/finite public configuration range"));
    }
    Ok([time, config.brightness_bits, config.contrast_bits])
}
fn typed_ast(config: &EffectConfiguration, text: &str, stored: &BrightnessContrastConfiguration, m: &mut Meter<'_, '_>) -> Result<(), IoError> {
    if config.nodes.len() != 3 {
        return Err(bad("three scalar AST fields required"));
    }
    let expected = slots(stored)?;
    let mut seen = 0u8;
    for n in &config.nodes {
        m.step(1)?;
        let ConfigValue::List(fields) = &n.value else {
            return Err(bad("scalar AST list"));
        };
        let [name_node, value] = fields.as_slice() else {
            return Err(bad("scalar AST arity"));
        };
        let ConfigValue::Atom(name) = &name_node.value else {
            return Err(bad("scalar AST name"));
        };
        let (index, bit) = match name.as_str() {
            "time" => (0, 1),
            "brightness" => (1, 2),
            "contrast" => (2, 4),
            _ => return Err(bad("known AST field")),
        };
        if seen & bit != 0 {
            return Err(bad("duplicate AST field"));
        }
        seen |= bit;
        let ConfigValue::Number { literal, bits } = &value.value else {
            return Err(bad("exact scalar AST number"));
        };
        let field_start = usize::try_from(n.start).map_err(|_| bad("field span"))?;
        let field_end = usize::try_from(n.end).map_err(|_| bad("field span"))?;
        let field = text.get(field_start..field_end).ok_or_else(|| bad("field text span"))?;
        m.step(add(mul(field.len() as u64, 2)?, 1)?)?;
        let inside = field.strip_prefix('(').and_then(|s| s.strip_suffix(')')).ok_or_else(|| bad("scalar field delimiters"))?;
        let mut tokens = inside.split_ascii_whitespace();
        if tokens.next() != Some(name.as_str())
            || tokens.next() != Some(literal.as_str())
            || tokens.next().is_some()
            || name_node.start < n.start
            || name_node.end > value.start
            || value.end > n.end
        {
            return Err(bad("raw field name/value association"));
        }
        m.step(add(mul(literal.len() as u64, 3)?, 1)?)?;
        let start = usize::try_from(value.start).map_err(|_| bad("number span"))?;
        let end = usize::try_from(value.end).map_err(|_| bad("number span"))?;
        if text.get(start..end) != Some(literal.as_str())
            || literal.parse::<f64>().ok().map(f64::to_bits) != Some(*bits)
            || expected.get(index).copied() != Some(*bits)
        {
            return Err(bad("raw scalar spelling/AST/typed bits mismatch"));
        }
    }
    if seen != 7 {
        return Err(bad("complete scalar fields"));
    }
    Ok(())
}
/// Full lower source/copy/index/work debit precedes all traversal and copies.
/// Source and complete decoder/mask owners remain separately held by the caller.
/// The canonical node keeps its GIMP identity and exact type6 fields; no prepared
/// numerical consumer is exposed until the independent wrapper recipe is known.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_brightness_contrast_source<'a, P>(
    source: &'a [u8],
    effect: &'a Effect<P>,
    filter: FilterId,
    mask: Option<MaskId>,
    identity: BrightnessContrastSourceIdentity<'a>,
    limits: BrightnessContrastMappingLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedBrightnessContrastSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = BrightnessContrastMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(bad("lower-only limits"));
    }
    if identity.xcf_version != 22
        || !matches!(identity.schema_sha256, BRIGHTNESS_CONTRAST_SCHEMA | BRIGHTNESS_CONTRAST_SCHEMA_308)
        || identity.expected_epoch.is_empty()
        || identity.expected_epoch.len() > 128
        || identity.expected_epoch.contains('\0')
        || effect.operation != "gimp:brightness-contrast"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 1
        || effect.properties.len() > 32
        || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0)
        || effect.mask.is_some() != mask.is_some()
        || !photocraft_xcf::is_xcf(source)
    {
        return Err(bad("operation/schema/source/canonical ID binding"));
    }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m = Meter {
        limits,
        input: 0,
        copied: add(size_of::<FilterNode>() as u64, size_of::<MappedBrightnessContrastSource<'a, P>>() as u64)?,
        index: 0,
        work: 0,
        ctl,
    };
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
        let actual = source.get(start..end).ok_or_else(|| bad("source extent"))?;
        for (a, b) in actual.chunks(4096).zip(p.payload.chunks(4096)) {
            m.step(mul(a.len() as u64, 2)?)?;
            if a != b {
                return Err(bad("raw/source property mismatch"));
            }
        }
    }
    m.input(mul(effect.arguments.capacity() as u64, size_of::<Argument>() as u64)?)?;
    let a = effect.arguments.first().ok_or_else(|| bad("config absent"))?;
    if a.name != "config" || a.graph.is_some() {
        return Err(bad("single known config argument"));
    }
    m.input_text(&a.name)?;
    m.step(13)?;
    let ArgumentValue::Config(text) = &a.value else {
        return Err(bad("type6 config required"));
    };
    m.input_text(text)?;
    let (raw, end) = raw_argument(a, &effect.properties)?;
    if word(raw, end)? != 6 {
        return Err(bad("raw config type"));
    }
    let at = end.checked_add(4).ok_or_else(|| bad("config offset"))?;
    let n = usize::try_from(word(raw, at)?).map_err(|_| bad("config length"))?;
    let start = at.checked_add(4).ok_or_else(|| bad("config start"))?;
    let stop = start.checked_add(n).ok_or_else(|| bad("config extent"))?;
    m.step(add(mul(text.len() as u64, 2)?, 1)?)?;
    if stop != raw.len() || raw.get(start..stop).and_then(|v| v.strip_suffix(&[0])) != Some(text.as_bytes()) {
        return Err(bad("raw configuration text"));
    }
    let configuration = a.configuration.as_ref().ok_or_else(|| bad("parsed config absent"))?;
    m.input(size_of::<EffectConfiguration>() as u64)?;
    m.ast(&configuration.nodes, 0, &mut 0, text.len() as u64)?;
    let Some(TypedConfiguration::BrightnessContrast(stored)) = &configuration.typed else {
        return Err(bad("typed GIMP config absent"));
    };
    m.input(size_of::<BrightnessContrastConfiguration>() as u64)?;
    typed_ast(configuration, text, stored, &mut m)?;
    let bits = slots(stored)?;
    let mut fields = m.vector(3)?;
    for (name, value) in ["time", "brightness", "contrast"].into_iter().zip(bits) {
        m.step(1)?;
        fields.push(FilterArgument { name: m.text(name)?, value: FilterValue::Number(ExactNumber::Float64Bits(value)) });
    }
    let mut arguments = m.vector(1)?;
    arguments.push(FilterArgument { name: m.text("config")?, value: FilterValue::Config(fields) });
    let state = &effect.state;
    let opacity = match (state.opacity_bits, state.opacity_u8) {
        (Some(v), None) => f64::from(f32::from_bits(v)),
        _ => return Err(bad("Float32 opacity required")),
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
    let requirements = BrightnessContrastMappingRequirements {
        reserved_resident_bytes: limits.resident_bytes,
        reserved_index_bytes: limits.index_bytes,
        reserved_work_units: limits.work_units,
        selected_input_bytes: m.input,
        copied_bytes: m.copied,
        copied_index_bytes: m.index,
        work_units: m.work,
    };
    Ok(MappedBrightnessContrastSource { source, effect, configuration, stored, identity, node, requirements })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_fields_ast_and_typed_signed_zero_are_bound_before_copy() {
        use photocraft_xcf::ConfigNode;
        let text = "(time 0)\n(brightness -0)\n(contrast 0.125)\n";
        let mut nodes = Vec::new();
        for (name, literal) in [("time", "0"), ("brightness", "-0"), ("contrast", "0.125")] {
            let start = text.find(&format!("({name} ")).unwrap();
            let value = start + name.len() + 2;
            nodes.push(ConfigNode {
                start: start as u64,
                end: (value + literal.len() + 1) as u64,
                value: ConfigValue::List(vec![
                    ConfigNode { start: (start + 1) as u64, end: (start + name.len() + 1) as u64, value: ConfigValue::Atom(name.into()) },
                    ConfigNode {
                        start: value as u64,
                        end: (value + literal.len()) as u64,
                        value: ConfigValue::Number { literal: literal.into(), bits: literal.parse::<f64>().unwrap().to_bits() },
                    },
                ]),
            });
        }
        let stored = BrightnessContrastConfiguration { time_bits: Some(0), brightness_bits: (-0f64).to_bits(), contrast_bits: 0.125f64.to_bits() };
        let config = EffectConfiguration { nodes, typed: Some(TypedConfiguration::BrightnessContrast(stored)) };
        let mut m = Meter { limits: Default::default(), input: 0, copied: 0, index: 0, work: 0, ctl: &Interrupt::NONE };
        assert!(typed_ast(&config, text, &stored, &mut m).is_ok());
        let changed = BrightnessContrastConfiguration { brightness_bits: 0, ..stored };
        assert!(typed_ast(&config, text, &changed, &mut m).is_err());
        assert_eq!(m.copied, 0);
    }
    #[test]
    fn gimp_neutral_and_signed_zero_do_not_become_gegl_parameters() {
        let c = BrightnessContrastConfiguration { time_bits: Some(0), brightness_bits: (-0f64).to_bits(), contrast_bits: 0 };
        assert_eq!(slots(&c).unwrap(), [0, (-0f64).to_bits(), 0]);
        let bad = BrightnessContrastConfiguration { contrast_bits: 1.5f64.to_bits(), ..c };
        assert!(slots(&bad).is_err());
    }
    #[test]
    fn no_missing_or_nonconstant_time_default_is_invented() {
        let c = BrightnessContrastConfiguration { time_bits: None, brightness_bits: 0, contrast_bits: 0 };
        assert!(slots(&c).is_err());
        let c = BrightnessContrastConfiguration { time_bits: Some(1f64.to_bits()), ..c };
        assert!(slots(&c).is_err());
    }
}
