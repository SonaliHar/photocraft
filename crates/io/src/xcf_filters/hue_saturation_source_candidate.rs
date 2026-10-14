//! Opt-in exact seven-selector Hue/Saturation configuration storage.
//! Original flat AST/text/rawwords and outer metadata remain borrowed. This is
//! not an HSV/HSL evaluator, authored-persistence proof or default source grant.
pub use super::configured_source_candidate::{
    CurvesMappingLimits as HueSaturationMappingLimits, CurvesMappingRequirements as HueSaturationMappingRequirements,
};
use super::configured_source_candidate::{Meter, raw_argument, word};
use crate::IoError;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::*,
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, ConfigValue, Effect, EffectConfiguration, HueSaturationConfiguration, RawProperty, TypedConfiguration};
use std::mem::size_of;
pub const HUE_SATURATION_SCHEMA: &str = "82c932da73a597a0a6e81da7aa286bb130dc377bd1a1ff3e56ef86b2d00ff68f";
const SELECTORS: [&str; 7] = ["all", "red", "yellow", "green", "cyan", "blue", "magenta"];
const FIELDS: [&str; 23] = [
    "time",
    "all.hue",
    "all.saturation",
    "all.lightness",
    "red.hue",
    "red.saturation",
    "red.lightness",
    "yellow.hue",
    "yellow.saturation",
    "yellow.lightness",
    "green.hue",
    "green.saturation",
    "green.lightness",
    "cyan.hue",
    "cyan.saturation",
    "cyan.lightness",
    "blue.hue",
    "blue.saturation",
    "blue.lightness",
    "magenta.hue",
    "magenta.saturation",
    "magenta.lightness",
    "overlap",
];
#[derive(Clone, Copy, Debug)]
pub struct HueSaturationSourceIdentity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
}
pub struct MappedHueSaturationSource<'a, P> {
    source: &'a [u8],
    effect: &'a Effect<P>,
    configuration: &'a EffectConfiguration,
    stored: &'a HueSaturationConfiguration,
    identity: HueSaturationSourceIdentity<'a>,
    node: FilterNode,
    requirements: HueSaturationMappingRequirements,
}
impl<'a, P> MappedHueSaturationSource<'a, P> {
    pub fn original_source(&self) -> &'a [u8] {
        self.source
    }
    pub fn original_effect(&self) -> &'a Effect<P> {
        self.effect
    }
    pub fn original_configuration(&self) -> &'a EffectConfiguration {
        self.configuration
    }
    pub fn stored(&self) -> &'a HueSaturationConfiguration {
        self.stored
    }
    pub fn identity(&self) -> HueSaturationSourceIdentity<'a> {
        self.identity
    }
    pub fn node(&self) -> &FilterNode {
        &self.node
    }
    pub fn requirements(&self) -> HueSaturationMappingRequirements {
        self.requirements
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("GIMP Hue Saturation source: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
fn slots(config: &HueSaturationConfiguration) -> Result<[u64; 23], IoError> {
    let time = config.time_bits.ok_or_else(|| bad("explicit time absent"))?;
    if time != 0 {
        return Err(bad("constant positive time0 required"));
    }
    let mut output = [0u64; 23];
    *output.get_mut(0).ok_or_else(|| bad("time slot"))? = time;
    for (ordinal, range) in config.ranges.iter().enumerate() {
        for (component, bits) in range.iter().enumerate() {
            let value = f64::from_bits(*bits);
            if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
                return Err(bad("finite Hue/Saturation/Lightness range"));
            }
            let index = ordinal.checked_mul(3).and_then(|i| i.checked_add(component)).and_then(|i| i.checked_add(1)).ok_or_else(|| bad("fixed slot index"))?;
            *output.get_mut(index).ok_or_else(|| bad("fixed slot"))? = *bits;
        }
    }
    let overlap = f64::from_bits(config.overlap_bits);
    if !overlap.is_finite() || !(0.0..=1.0).contains(&overlap) {
        return Err(bad("finite overlap range"));
    }
    *output.get_mut(22).ok_or_else(|| bad("overlap slot"))? = config.overlap_bits;
    Ok(output)
}
fn typed_ast(config: &EffectConfiguration, text: &str, stored: &HueSaturationConfiguration, m: &mut Meter<'_, '_>) -> Result<(), IoError> {
    if config.nodes.len() != 30 {
        return Err(bad("exact30 flat configuration fields required"));
    }
    let expected = slots(stored)?;
    let mut previous = 0usize;
    for (ordinal, n) in config.nodes.iter().enumerate() {
        m.step(1)?;
        let ConfigValue::List(fields) = &n.value else {
            return Err(bad("flat AST field list"));
        };
        let [name_node, value_node] = fields.as_slice() else {
            return Err(bad("field arity"));
        };
        let ConfigValue::Atom(name) = &name_node.value else {
            return Err(bad("field name atom"));
        };
        let (expected_name, selector, index) = if ordinal == 0 {
            ("time", None, Some(0))
        } else if ordinal == 29 {
            ("overlap", None, Some(22))
        } else {
            let block = (ordinal - 1) / 4;
            let component = (ordinal - 1) % 4;
            if component == 0 {
                ("range", Some(*SELECTORS.get(block).ok_or_else(|| bad("selector ordinal"))?), None)
            } else {
                let scalar = *["hue", "saturation", "lightness"].get(component - 1).ok_or_else(|| bad("scalar ordinal"))?;
                (scalar, None, Some(1 + block * 3 + component - 1))
            }
        };
        if name != expected_name {
            return Err(bad("ordered known raw field name"));
        }
        let start = usize::try_from(n.start).map_err(|_| bad("field start"))?;
        let end = usize::try_from(n.end).map_err(|_| bad("field end"))?;
        let gap = text.get(previous..start).ok_or_else(|| bad("ordered field span"))?;
        m.step(add(gap.len() as u64, 1)?)?;
        if !gap.bytes().all(|b| b.is_ascii_whitespace()) {
            return Err(bad("unrepresented raw configuration text"));
        }
        let field = text.get(start..end).ok_or_else(|| bad("raw field span"))?;
        m.step(add(mul(field.len() as u64, 2)?, 1)?)?;
        let inside = field.strip_prefix('(').and_then(|s| s.strip_suffix(')')).ok_or_else(|| bad("raw field delimiters"))?;
        let (raw_value, bits) = match (&value_node.value, selector) {
            (ConfigValue::Atom(value), Some(selector)) if value == selector => (value.as_str(), None),
            (ConfigValue::Number { literal, bits }, None) => {
                m.step(add(mul(literal.len() as u64, 3)?, 1)?)?;
                if literal.parse::<f64>().ok().map(f64::to_bits) != Some(*bits) || index.and_then(|i| expected.get(i).copied()) != Some(*bits) {
                    return Err(bad("raw scalar spelling/AST/typed bits mismatch"));
                }
                (literal.as_str(), Some(*bits))
            }
            _ => return Err(bad("range atom versus exact scalar type")),
        };
        let mut tokens = inside.split_ascii_whitespace();
        if tokens.next() != Some(expected_name)
            || tokens.next() != Some(raw_value)
            || tokens.next().is_some()
            || name_node.start < n.start
            || name_node.end > value_node.start
            || value_node.end > n.end
        {
            return Err(bad("raw field association"));
        }
        let name_start = usize::try_from(name_node.start).map_err(|_| bad("name span"))?;
        let name_end = usize::try_from(name_node.end).map_err(|_| bad("name span"))?;
        let value_start = usize::try_from(value_node.start).map_err(|_| bad("value span"))?;
        let value_end = usize::try_from(value_node.end).map_err(|_| bad("value span"))?;
        m.step(add(add(name.len() as u64, raw_value.len() as u64)?, 1)?)?;
        if text.get(name_start..name_end) != Some(expected_name) || text.get(value_start..value_end) != Some(raw_value) {
            return Err(bad("raw child spans"));
        }
        if bits.is_some_and(|v| !f64::from_bits(v).is_finite()) {
            return Err(bad("nonfinite raw scalar"));
        }
        previous = end;
    }
    let suffix = text.get(previous..).ok_or_else(|| bad("terminal field extent"))?;
    m.step(add(suffix.len() as u64, 1)?)?;
    if !suffix.bytes().all(|b| b.is_ascii_whitespace()) {
        return Err(bad("unrepresented terminal raw text"));
    }
    Ok(())
}
/// Full lower source/copy/index/work debit precedes all traversal and copies.
/// Source and complete decoder/mask owners remain separately held by the caller.
/// The canonical node keeps its GIMP identity and exact type6 fields; no prepared
/// numerical consumer is inferred from this source grammar or schema identity.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_hue_saturation_source<'a, P>(
    source: &'a [u8],
    effect: &'a Effect<P>,
    filter: FilterId,
    mask: Option<MaskId>,
    identity: HueSaturationSourceIdentity<'a>,
    limits: HueSaturationMappingLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedHueSaturationSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = HueSaturationMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(bad("lower-only limits"));
    }
    if identity.xcf_version != 22
        || identity.schema_sha256 != HUE_SATURATION_SCHEMA
        || identity.expected_epoch.is_empty()
        || identity.expected_epoch.len() > 128
        || identity.expected_epoch.contains('\0')
        || effect.operation != "gimp:hue-saturation"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 1
        || effect.properties.len() > 32
        || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0)
        || effect.mask.is_some() != mask.is_some()
        || source.get(..14) != Some(b"gimp xcf v022\0".as_slice())
    {
        return Err(bad("operation/schema/source/canonical ID binding"));
    }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m =
        Meter { limits, input: 0, copied: add(size_of::<FilterNode>() as u64, size_of::<MappedHueSaturationSource<'a, P>>() as u64)?, index: 0, work: 0, ctl };
    m.check()?;
    m.input(source.len() as u64)?; // complete borrowed source span, not backing capacity
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
    let Some(TypedConfiguration::HueSaturation(stored)) = &configuration.typed else {
        return Err(bad("typed GIMP config absent"));
    };
    m.input(size_of::<HueSaturationConfiguration>() as u64)?;
    typed_ast(configuration, text, stored, &mut m)?;
    let bits = slots(stored)?;
    let mut fields = m.vector(23)?;
    for (name, value) in FIELDS.into_iter().zip(bits) {
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
    let requirements = HueSaturationMappingRequirements {
        reserved_resident_bytes: limits.resident_bytes,
        reserved_index_bytes: limits.index_bytes,
        reserved_work_units: limits.work_units,
        selected_input_bytes: m.input,
        copied_bytes: m.copied,
        copied_index_bytes: m.index,
        work_units: m.work,
    };
    Ok(MappedHueSaturationSource { source, effect, configuration, stored, identity, node, requirements })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_xcf::{ConfigNode, EffectState};
    fn fixture() -> (Vec<u8>, Effect<()>, HueSaturationConfiguration) {
        let mut stored = HueSaturationConfiguration { time_bits: Some(0), ranges: [[0; 3]; 7], overlap_bits: 0.5f64.to_bits() };
        stored.ranges[0][0] = (-0f64).to_bits();
        stored.ranges[1][0] = 0.125f64.to_bits();
        stored.ranges[1][1] = (-0.25f64).to_bits();
        let mut text = String::new();
        let mut nodes = Vec::new();
        let mut field = |name: &str, literal: &str, bits: Option<u64>| {
            let start = text.len();
            text.push_str(&format!("({name} {literal})\n"));
            let value = start + name.len() + 2;
            nodes.push(ConfigNode {
                start: start as u64,
                end: (text.len() - 1) as u64,
                value: ConfigValue::List(vec![
                    ConfigNode { start: (start + 1) as u64, end: (start + 1 + name.len()) as u64, value: ConfigValue::Atom(name.into()) },
                    ConfigNode {
                        start: value as u64,
                        end: (value + literal.len()) as u64,
                        value: if let Some(bits) = bits { ConfigValue::Number { literal: literal.into(), bits } } else { ConfigValue::Atom(literal.into()) },
                    },
                ]),
            });
        };
        field("time", "0", Some(0));
        for (selector, values) in SELECTORS.into_iter().zip(stored.ranges) {
            field("range", selector, None);
            for (name, bits) in ["hue", "saturation", "lightness"].into_iter().zip(values) {
                field(name, &f64::from_bits(bits).to_string(), Some(bits));
            }
        }
        field("overlap", "0.5", Some(stored.overlap_bits));
        drop(field);
        let mut payload = 7u32.to_be_bytes().to_vec();
        payload.extend_from_slice(b"config\0");
        payload.extend_from_slice(&6u32.to_be_bytes());
        payload.extend_from_slice(&u32::try_from(text.len() + 1).unwrap().to_be_bytes());
        payload.extend_from_slice(text.as_bytes());
        payload.push(0);
        let mut source = b"gimp xcf v022\0".to_vec();
        let offset = source.len() as u64;
        source.extend_from_slice(&payload);
        let effect = Effect {
            name: "Own literal HS".into(),
            icon: "own".into(),
            operation: "gimp:hue-saturation".into(),
            version: Some("0:0".into()),
            properties: vec![RawProperty { tag: 45, offset, declared_bytes: payload.len() as u32, payload }],
            arguments: vec![Argument {
                name: "config".into(),
                property_index: 0,
                value: ArgumentValue::Config(text),
                configuration: Some(EffectConfiguration { nodes, typed: Some(TypedConfiguration::HueSaturation(stored)) }),
                graph: None,
            }],
            mask: None,
            state: EffectState {
                enabled: Some(true),
                opacity_bits: Some(0.5f32.to_bits()),
                mode: Some(62),
                blend_space: Some(-77),
                composite_space: Some(-88),
                composite_mode: Some(-99),
                clipping: Some(5),
                region: Some(6),
                ..Default::default()
            },
        };
        (source, effect, stored)
    }
    fn identity() -> HueSaturationSourceIdentity<'static> {
        HueSaturationSourceIdentity { xcf_version: 22, expected_epoch: "own-explicit-retained-storage", schema_sha256: HUE_SATURATION_SCHEMA }
    }
    #[test]
    fn exact23_words_names_outer_tuple_and_raw_originals_are_preserved() {
        let (source, effect, stored) = fixture();
        let original = source.clone();
        let raw = effect.properties[0].payload.clone();
        let reserved = std::cell::Cell::new((0, 0));
        let work = std::cell::Cell::new(0);
        let mapped = map_retained_hue_saturation_source(
            &source,
            &effect,
            FilterId(11),
            None,
            identity(),
            Default::default(),
            &Interrupt::NONE,
            &mut |resident, index| {
                reserved.set((resident, index));
                Ok(())
            },
            &mut |units| {
                work.set(units);
                Ok(())
            },
        )
        .unwrap();
        let FilterValue::Config(fields) = &mapped.node().arguments[0].value else {
            panic!("fixed config");
        };
        assert_eq!(fields.len(), 23);
        for ((field, name), bits) in fields.iter().zip(FIELDS).zip(slots(&stored).unwrap()) {
            assert_eq!(field.name, name);
            assert_eq!(field.value, FilterValue::Number(ExactNumber::Float64Bits(bits)));
        }
        let composition = mapped.node().composition.as_ref().unwrap();
        assert_eq!((composition.stored_blend_space, composition.stored_composite_space, composition.stored_operator), (-77, -88, -99));
        assert_eq!(mapped.node().opacity_f64_bits, 0.5f64.to_bits());
        assert!(composition.resolved.is_none());
        assert!(std::ptr::eq(mapped.original_effect(), &effect));
        assert_eq!(mapped.original_source(), original);
        assert_eq!(effect.properties[0].payload, raw);
        assert!(reserved.get().0 > 0);
        assert!(work.get() > 0);
    }
    #[test]
    fn raw_text_and_typed_mismatch_refuse_before_canonical_vector_allocation() {
        let (_, effect, stored) = fixture();
        let argument = &effect.arguments[0];
        let ArgumentValue::Config(text) = &argument.value else {
            panic!("config");
        };
        let configuration = argument.configuration.as_ref().unwrap();
        let mut changed = stored;
        changed.ranges[1][0] = 0.25f64.to_bits();
        let mut meter = Meter { limits: Default::default(), input: 0, copied: 0, index: 0, work: 0, ctl: &Interrupt::NONE };
        assert!(typed_ast(configuration, text, &changed, &mut meter).is_err());
        assert_eq!(meter.index, 0);
        let substituted = text.replacen("(range red)", "(range all)", 1);
        assert!(typed_ast(configuration, &substituted, &stored, &mut meter).is_err());
        assert!(typed_ast(configuration, &format!("{text}(unknown 0)"), &stored, &mut meter).is_err());
    }
    #[test]
    fn lower_source_and_work_bounds_cancellation_and_nonconstant_time_refuse() {
        let (source, effect, stored) = fixture();
        for limits in
            [HueSaturationMappingLimits { resident_bytes: 1, ..Default::default() }, HueSaturationMappingLimits { work_units: 1, ..Default::default() }]
        {
            assert!(
                map_retained_hue_saturation_source(
                    &source,
                    &effect,
                    FilterId(11),
                    None,
                    identity(),
                    limits,
                    &Interrupt::NONE,
                    &mut |_, _| Ok(()),
                    &mut |_| Ok(())
                )
                .is_err()
            );
        }
        let cancelled = || true;
        assert!(matches!(
            map_retained_hue_saturation_source(
                &source,
                &effect,
                FilterId(11),
                None,
                identity(),
                Default::default(),
                &Interrupt::cancel_only(&cancelled),
                &mut |_, _| Ok(()),
                &mut |_| Ok(())
            ),
            Err(IoError::Cancelled)
        ));
        assert!(slots(&HueSaturationConfiguration { time_bits: Some((-0f64).to_bits()), ..stored }).is_err());
        assert!(slots(&HueSaturationConfiguration { time_bits: None, ..stored }).is_err());
    }
}
