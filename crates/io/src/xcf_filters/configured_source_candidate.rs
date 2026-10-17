//! Opt-in retained Curves storage mapping, outside qualified_stack/import admission.
//! The original effect/configuration and complete caller-held decoded backing stay
//! borrowed. This is not an independent decode/source or rendering proof.
use crate::IoError;
use photocraft_compose::gimp_configured_candidate::{CURVES_SCHEMA, RetainedSourceIdentity};
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::*,
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, ConfigNode, ConfigValue, CurvesConfiguration, Effect, EffectConfiguration, RawProperty, TypedConfiguration};
use std::mem::size_of;

#[derive(Clone, Copy, Debug)]
pub struct CurvesMappingLimits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for CurvesMappingLimits {
    fn default() -> Self {
        Self { resident_bytes: 16 << 20, index_bytes: 1 << 20, work_units: 16_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct CurvesMappingRequirements {
    pub reserved_resident_bytes: u64,
    pub reserved_index_bytes: u64,
    pub reserved_work_units: u64,
    pub selected_input_bytes: u64,
    pub copied_bytes: u64,
    pub copied_index_bytes: u64,
    pub work_units: u64,
}
pub struct MappedCurvesSource<'a, P> {
    original: &'a Effect<P>,
    configuration: &'a EffectConfiguration,
    source: &'a [u8],
    identity: RetainedSourceIdentity<'a>,
    node: FilterNode,
    serialized_samples: [u32; 5],
    requirements: CurvesMappingRequirements,
}
impl<'a, P> MappedCurvesSource<'a, P> {
    pub fn original_effect(&self) -> &'a Effect<P> {
        self.original
    }
    pub fn original_configuration(&self) -> &'a EffectConfiguration {
        self.configuration
    }
    /// Already-authorized uncompressed backing; no locator or read capability.
    pub fn original_source(&self) -> &'a [u8] {
        self.source
    }
    pub fn node(&self) -> &FilterNode {
        &self.node
    }
    /// Move the already-admitted node without another deep configuration copy.
    pub(super) fn into_node(self) -> FilterNode {
        self.node
    }
    pub fn identity(&self) -> RetainedSourceIdentity<'a> {
        self.identity
    }
    pub fn serialized_samples(&self) -> [u32; 5] {
        self.serialized_samples
    }
    pub fn requirements(&self) -> CurvesMappingRequirements {
        self.requirements
    }
    /// Uses the existing configured consumer, with its independent lower quota
    /// and explicit profile domain. Mapping does not qualify this evaluation.
    pub fn prepare_retained_plan(
        &self,
        limits: photocraft_algo::gimp::configured::Limits,
        ctl: &Interrupt<'_>,
    ) -> Result<photocraft_compose::gimp_configured_candidate::RetainedSourcePlan<'_>, photocraft_raster::RasterInputError> {
        photocraft_compose::gimp_configured_candidate::prepare_retained_source_plan(&self.node, self.identity, limits, ctl)
    }
}
fn bad(message: &str) -> IoError {
    IoError::Unsupported(format!("retained Curves candidate: {message}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
pub(super) struct Meter<'c, 'i> {
    pub(super) limits: CurvesMappingLimits,
    pub(super) input: u64,
    pub(super) copied: u64,
    pub(super) index: u64,
    pub(super) work: u64,
    pub(super) ctl: &'c Interrupt<'i>,
}
impl Meter<'_, '_> {
    pub(super) fn step(&mut self, n: u64) -> Result<(), IoError> {
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        self.work = add(self.work, n)?;
        if self.work > self.limits.work_units {
            return Err(bad("work limit"));
        }
        Ok(())
    }
    pub(super) fn input(&mut self, n: u64) -> Result<(), IoError> {
        self.step(1)?;
        self.input = add(self.input, n)?;
        self.check()
    }
    pub(super) fn check(&self) -> Result<(), IoError> {
        if add(self.input, self.copied)? > self.limits.resident_bytes || self.index > self.limits.index_bytes {
            return Err(bad("input/copy/index envelope"));
        }
        Ok(())
    }
    pub(super) fn vector<T>(&mut self, n: usize) -> Result<Vec<T>, IoError> {
        let bytes = mul(n as u64, size_of::<T>() as u64)?;
        self.step(1)?;
        self.copied = add(self.copied, bytes)?;
        self.index = add(self.index, bytes)?;
        self.check()?;
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| bad("allocation"))?;
        let excess = mul(out.capacity().saturating_sub(n) as u64, size_of::<T>() as u64)?;
        self.copied = add(self.copied, excess)?;
        self.index = add(self.index, excess)?;
        self.check()?;
        Ok(out)
    }
    pub(super) fn text(&mut self, s: &str) -> Result<String, IoError> {
        if s.len() > 65536 || s.contains('\0') {
            return Err(bad("bounded text"));
        }
        let mut v = self.vector::<u8>(s.len())?;
        for c in s.as_bytes().chunks(4096) {
            self.step(c.len() as u64)?;
            v.extend_from_slice(c);
        }
        String::from_utf8(v).map_err(|_| bad("UTF8"))
    }
    pub(super) fn input_text(&mut self, s: &String) -> Result<(), IoError> {
        if s.len() > 1 << 20 {
            return Err(bad("input text bound"));
        }
        self.input(s.capacity() as u64)
    }
    pub(super) fn ast(&mut self, nodes: &Vec<ConfigNode>, depth: usize, count: &mut usize, span: u64) -> Result<(), IoError> {
        if depth > 32 {
            return Err(bad("AST depth"));
        }
        self.input(mul(nodes.capacity() as u64, size_of::<ConfigNode>() as u64)?)?;
        for n in nodes {
            self.step(1)?;
            *count = count.checked_add(1).filter(|v| *v <= 4096).ok_or_else(|| bad("AST count"))?;
            if n.start > n.end || n.end > span {
                return Err(bad("AST span"));
            }
            match &n.value {
                ConfigValue::Atom(s) => self.input_text(s)?,
                ConfigValue::Number { literal, .. } => self.input_text(literal)?,
                ConfigValue::Quoted(v) => self.input(v.capacity() as u64)?,
                ConfigValue::List(v) => self.ast(v, depth + 1, count, span)?,
            }
        }
        Ok(())
    }
}
pub(super) fn word(bytes: &[u8], at: usize) -> Result<u32, IoError> {
    let end = at.checked_add(4).ok_or_else(|| bad("word overflow"))?;
    let value: [u8; 4] = bytes.get(at..end).ok_or_else(|| bad("short word"))?.try_into().map_err(|_| bad("word width"))?;
    Ok(u32::from_be_bytes(value))
}
pub(super) fn raw_argument<'a>(a: &Argument, properties: &'a [RawProperty]) -> Result<(&'a [u8], usize), IoError> {
    let p = properties.get(a.property_index).filter(|p| p.tag == 45).ok_or_else(|| bad("argument raw association"))?;
    let len = usize::try_from(word(&p.payload, 0)?).map_err(|_| bad("argument name length"))?;
    let end = 4usize.checked_add(len).ok_or_else(|| bad("name overflow"))?;
    if p.payload.get(4..end).and_then(|v| v.strip_suffix(&[0])) != Some(a.name.as_bytes()) {
        return Err(bad("raw argument name"));
    }
    Ok((&p.payload, end))
}
fn transfer(config: &CurvesConfiguration, tag: i32) -> Result<(), IoError> {
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
fn curve_shape(curve: &photocraft_xcf::StoredCurve, m: &mut Meter<'_, '_>) -> Result<bool, IoError> {
    let free = match curve.curve_type.as_str() {
        "free" => true,
        "smooth" => false,
        _ => return Err(bad("curve mode")),
    };
    if curve.n_samples != 256
        || curve.samples_bits.len() != curve.n_samples as usize
        || !curve.points_bits.len().is_multiple_of(2)
        || curve.points_bits.len() > 128
        || curve.point_types.len() != curve.points_bits.len() / 2
    {
        return Err(bad("exact serialized curve shape; no requested-length alias"));
    }
    let mut previous = None;
    for p in curve.points_bits.as_chunks::<2>().0 {
        m.step(1)?;
        let x = f64::from_bits(*p.first().ok_or_else(|| bad("point x"))?);
        let y = f64::from_bits(*p.get(1).ok_or_else(|| bad("point y"))?);
        if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) || previous.is_some_and(|v| x <= v) {
            return Err(bad("curve point geometry"));
        }
        previous = Some(x);
    }
    for tag in &curve.point_types {
        m.step(1)?;
        if !matches!(tag.as_str(), "smooth" | "corner") {
            return Err(bad("curve point type"));
        }
    }
    for sample in &curve.samples_bits {
        m.step(1)?;
        if !f64::from_bits(*sample).is_finite() {
            return Err(bad("finite curve sample"));
        }
    }
    Ok(free)
}
/// Both callbacks debit the same enclosing ledger before traversal or copies.
/// Complete source/decoder/mask backing remains separately held by the caller.
/// The source slice is the authorized plain-XCF backing used by its parser;
/// property association is checked, not promoted into an independent source proof.
#[allow(clippy::too_many_arguments)]
pub fn map_retained_curves_source<'a, P>(
    source: &'a [u8],
    effect: &'a Effect<P>,
    filter: FilterId,
    mask: Option<MaskId>,
    identity: RetainedSourceIdentity<'a>,
    limits: CurvesMappingLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedCurvesSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = CurvesMappingLimits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(bad("lower-only limits"));
    }
    if identity.xcf_version != 22
        || identity.schema_sha256 != CURVES_SCHEMA
        || identity.expected_epoch.is_empty()
        || identity.expected_epoch.len() > 128
        || identity.expected_epoch.contains('\0')
        || !matches!(identity.color_channels, 1 | 3)
        || filter.0 == 0
        || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0)
        || effect.mask.is_some() != mask.is_some()
        || effect.operation != "gimp:curves"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 2
        || effect.properties.len() > 32
        || !photocraft_xcf::is_xcf(source)
    {
        return Err(bad("operation/source/schema/ID binding"));
    }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut m = Meter { limits, input: 0, copied: 0, index: 0, work: 0, ctl };
    m.copied = add(size_of::<FilterNode>() as u64, size_of::<MappedCurvesSource<'a, P>>() as u64)?;
    m.check()?;
    m.input(size_of::<Effect<P>>() as u64)?;
    for s in [&effect.name, &effect.icon, &effect.operation] {
        m.input_text(s)?;
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
        m.input_text(&a.name)?;
        if a.graph.is_some() {
            return Err(bad("nested graph"));
        }
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
                if stop != raw.len() || raw.get(start..stop).and_then(|v| v.strip_suffix(&[0])) != Some(text.as_bytes()) {
                    return Err(bad("raw configuration text"));
                }
                let config = a.configuration.as_ref().ok_or_else(|| bad("parsed configuration absent"))?;
                m.input(size_of::<EffectConfiguration>() as u64)?;
                m.ast(&config.nodes, 0, &mut 0, text.len() as u64)?;
                original = Some(config);
            }
            _ => return Err(bad("exact Curves argument envelope")),
        }
    }
    let tag = trc.ok_or_else(|| bad("TRC missing"))?;
    let original = original.ok_or_else(|| bad("config missing"))?;
    let Some(TypedConfiguration::Curves(config)) = &original.typed else {
        return Err(bad("typed Curves absent"));
    };
    if config.time_bits != Some(0) || config.channels.len() != 5 {
        return Err(bad("configuration time/channel shape"));
    }
    transfer(config, tag)?;
    m.input(size_of::<CurvesConfiguration>() as u64)?;
    if let Some(v) = &config.trc {
        m.input_text(v)?;
    }
    m.input(mul(config.channels.capacity() as u64, size_of::<photocraft_xcf::CurveChannel>() as u64)?)?;
    let mut forms = [false; 5];
    let mut serialized = [0; 5];
    for ((c, name), (free, count)) in config.channels.iter().zip(["value", "red", "green", "blue", "alpha"]).zip(forms.iter_mut().zip(serialized.iter_mut())) {
        if c.channel != name {
            return Err(bad("channel order"));
        }
        m.input_text(&c.channel)?;
        m.input_text(&c.curve.curve_type)?;
        m.input(mul(c.curve.points_bits.capacity() as u64, 8)?)?;
        m.input(mul(c.curve.samples_bits.capacity() as u64, 8)?)?;
        m.input(mul(c.curve.point_types.capacity() as u64, size_of::<String>() as u64)?)?;
        for v in &c.curve.point_types {
            m.input_text(v)?;
        }
        *free = curve_shape(&c.curve, &mut m)?;
        *count = c.curve.n_samples;
    }
    let mut channels = m.vector(5)?;
    for (c, free) in config.channels.iter().zip(forms) {
        let mut points = m.vector(c.curve.point_types.len())?;
        for p in c.curve.points_bits.as_chunks::<2>().0 {
            m.step(1)?;
            points.push([*p.first().ok_or_else(|| bad("point x"))?, *p.get(1).ok_or_else(|| bad("point y"))?]);
        }
        let mut types = m.vector(c.curve.point_types.len())?;
        for p in &c.curve.point_types {
            m.step(1)?;
            types.push(match p.as_str() {
                "smooth" => 0,
                "corner" => 1,
                _ => return Err(bad("point type")),
            });
        }
        let mut samples = m.vector(c.curve.samples_bits.len())?;
        for sample in &c.curve.samples_bits {
            m.step(1)?;
            samples.push(*sample);
        }
        channels.push(FilterArgument {
            name: m.text(&c.channel)?,
            value: FilterValue::Curve { free, points_f64_bits: points, point_types: types, samples_f64_bits: samples },
        });
    }
    let mut arguments = m.vector(2)?;
    arguments.push(FilterArgument { name: m.text("trc")?, value: FilterValue::Enum { domain: m.text("gimp:curves::trc")?, value: tag } });
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
    let requirements = CurvesMappingRequirements {
        reserved_resident_bytes: limits.resident_bytes,
        reserved_index_bytes: limits.index_bytes,
        reserved_work_units: limits.work_units,
        selected_input_bytes: m.input,
        copied_bytes: m.copied,
        copied_index_bytes: m.index,
        work_units: m.work,
    };
    Ok(MappedCurvesSource { original: effect, configuration: original, source, identity, node, serialized_samples: serialized, requirements })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_argument_name_and_index_are_exact_not_an_operation_name_alias() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&4u32.to_be_bytes());
        payload.extend_from_slice(b"trc\0");
        payload.extend_from_slice(&5u32.to_be_bytes());
        payload.extend_from_slice(&1u32.to_be_bytes());
        let property = RawProperty { tag: 45, offset: 0, declared_bytes: payload.len() as u32, payload };
        let mut a = Argument { name: "trc".into(), property_index: 0, value: ArgumentValue::Enum(1), configuration: None, graph: None };
        assert_eq!(raw_argument(&a, std::slice::from_ref(&property)).unwrap().1, 8);
        a.name = "config".into();
        assert!(raw_argument(&a, std::slice::from_ref(&property)).is_err());
        a.name = "trc".into();
        a.property_index = 1;
        assert!(raw_argument(&a, &[property]).is_err());
        assert!(word(&[0, 0, 0], 0).is_err());
    }
    #[test]
    fn smooth_free_and_signed_zero_are_kept_distinct_without_requested_count_aliases() {
        let ctl = Interrupt::NONE;
        let mut m = Meter { limits: CurvesMappingLimits::default(), input: 0, copied: 0, index: 0, work: 0, ctl: &ctl };
        let mut curve = photocraft_xcf::StoredCurve {
            curve_type: "smooth".into(),
            points_bits: vec![(-0f64).to_bits(), 0, 1f64.to_bits(), 1f64.to_bits()],
            point_types: vec!["smooth".into(), "corner".into()],
            n_samples: 256,
            samples_bits: vec![0; 256],
        };
        assert!(!curve_shape(&curve, &mut m).unwrap());
        assert_eq!(curve.points_bits.first(), Some(&(-0f64).to_bits()));
        curve.curve_type = "free".into();
        assert!(curve_shape(&curve, &mut m).unwrap());
        curve.n_samples = 257;
        assert!(curve_shape(&curve, &mut m).is_err());
        curve.n_samples = 256;
        curve.point_types.pop();
        assert!(curve_shape(&curve, &mut m).is_err());
    }
    #[test]
    fn precharged_copy_and_cancel_limits_fail_without_output() {
        let stop = || true;
        let ctl = Interrupt::cancel_only(&stop);
        let mut m = Meter { limits: CurvesMappingLimits::default(), input: 0, copied: 0, index: 0, work: 0, ctl: &ctl };
        assert!(matches!(m.vector::<u64>(256), Err(IoError::Cancelled)));
        let mut m = Meter {
            limits: CurvesMappingLimits { resident_bytes: 7, index_bytes: 7, work_units: 100 },
            input: 0,
            copied: 0,
            index: 0,
            work: 0,
            ctl: &Interrupt::NONE,
        };
        assert!(m.vector::<u64>(1).is_err());
        assert!(add(u64::MAX, 1).is_err());
    }
}
